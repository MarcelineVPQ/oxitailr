//! Secret store for values that must be recoverable — SSH passwords are sent to
//! the server, so they are encrypted or kept in the OS credential store, never
//! hashed.
//!
//! Backends, chosen by [`open_default`]:
//!
//! * **OS keychain** ([`KeychainStore`]): Windows Credential Manager, macOS
//!   Keychain, or the Linux/BSD Secret Service (GNOME Keyring, KWallet) over a
//!   pure-Rust D-Bus client, so no libdbus is needed at build or run time.
//!   Keychains cannot list their entries, so the ids (never the values) are
//!   kept in `<config_dir>/oxitailr/secrets-index.json`.
//! * **Encrypted file** ([`FileStore`]) when no keychain answers (headless
//!   Linux, services): `<config_dir>/oxitailr/secrets.vault`, AES-256-GCM with a
//!   fresh random nonce per write, under a random 32-byte data key. The key is
//!   kept in a keychain when one is given, else in
//!   `<config_dir>/oxitailr/vault.key` (0600 on Unix; on Windows the file is
//!   protected by the user-profile ACL only).
//!
//! Secret values travel as [`SecretString`], which zeroes its memory on drop and
//! prints as `[REDACTED]`. No error message or log line in this module contains
//! a secret value.

use aes_gcm::aead::{Aead, Generate, Key, KeyInit, Payload};
use aes_gcm::Aes256Gcm;
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use zeroize::Zeroizing;

pub use secrecy::{ExposeSecret, SecretString};

/// Service name every keychain entry is filed under.
pub const SERVICE: &str = "oxitailr";

/// Set to `file` to skip the keychain and use the encrypted file with a key
/// file (tests, headless machines, services).
pub const BACKEND_ENV: &str = "OXITAILR_SECRET_BACKEND";

/// Id of the vault data key when it is kept in a keychain.
pub const VAULT_KEY_ID: &str = "vault-key";

const PROBE_ID: &str = "oxitailr-probe";
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
/// Vault header: magic + format version. Bound into GCM as associated data.
const VAULT_HEADER: &[u8] = b"OXTVAULT\x01";

type Nonce = aes_gcm::aead::Nonce<Aes256Gcm>;

fn cipher_for(key: &[u8; KEY_LEN]) -> Aes256Gcm {
    Aes256Gcm::new_from_slice(key).expect("a 32-byte key is valid for AES-256-GCM")
}

fn nonce_from(bytes: &[u8]) -> Result<Nonce> {
    Nonce::try_from(bytes).map_err(|_| anyhow!("bad nonce length"))
}

/// A store of named secrets. Ids are not secret; values are.
pub trait SecretStore: Send + Sync {
    /// The secret stored under `id`, or `None` if there is none.
    fn get(&self, id: &str) -> Result<Option<SecretString>>;
    /// Store (or replace) the secret under `id`.
    fn set(&self, id: &str, secret: &SecretString) -> Result<()>;
    /// Remove the secret under `id`. Removing a missing id is not an error.
    fn delete(&self, id: &str) -> Result<()>;
    /// Every stored id, sorted.
    fn ids(&self) -> Result<Vec<String>>;
    /// Short backend name for logs and UI: "keychain", "file" or "memory".
    fn backend(&self) -> &'static str;
}

/// Id of the saved password for an SSH login: `ssh:{user}@{host}:{port}`, with
/// the host lowercased (host names are case-insensitive).
pub fn ssh_password_id(user: &str, host: &str, port: u16) -> String {
    format!("ssh:{}@{}:{}", user, host.to_lowercase(), port)
}

/// Id under which [`migrate_legacy`] files a password from the old
/// `credentials.enc`, which was keyed by source name only.
pub fn legacy_id(source_name: &str) -> String {
    format!("legacy:{source_name}")
}

/// Where the secret files live. Injectable so tests never touch the real
/// config directory.
#[derive(Debug, Clone)]
pub struct SecretPaths {
    dir: PathBuf,
}

impl SecretPaths {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// `<config_dir>/oxitailr`.
    pub fn default_dir() -> Result<Self> {
        let base =
            dirs::config_dir().ok_or_else(|| anyhow!("no config directory on this system"))?;
        Ok(Self::new(base.join("oxitailr")))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn index(&self) -> PathBuf {
        self.dir.join("secrets-index.json")
    }
    pub fn vault(&self) -> PathBuf {
        self.dir.join("secrets.vault")
    }
    pub fn key_file(&self) -> PathBuf {
        self.dir.join("vault.key")
    }
    /// The pre-secret-store credentials file, read once by [`migrate_legacy`].
    pub fn legacy_credentials(&self) -> PathBuf {
        self.dir.join("credentials.enc")
    }
}

// ---------------------------------------------------------------------------
// Opening the default store
// ---------------------------------------------------------------------------

/// Which backend [`open_in`] may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendChoice {
    /// The OS keychain if a real round trip works, else the encrypted file.
    Auto,
    /// The encrypted file with a key file; the keychain is never touched.
    File,
}

impl BackendChoice {
    /// Read [`BACKEND_ENV`]: `file` forces [`BackendChoice::File`].
    pub fn from_env() -> Self {
        match std::env::var(BACKEND_ENV) {
            Ok(v) if v.eq_ignore_ascii_case("file") => Self::File,
            Ok(v)
                if v.is_empty()
                    || v.eq_ignore_ascii_case("auto")
                    || v.eq_ignore_ascii_case("keychain") =>
            {
                Self::Auto
            }
            Ok(v) => {
                tracing::warn!(
                    "{BACKEND_ENV}={v:?} is not recognised (use \"file\"); choosing automatically"
                );
                Self::Auto
            }
            Err(_) => Self::Auto,
        }
    }
}

/// Open the secret store for this user: the OS keychain when a set/get/delete
/// probe succeeds, else the encrypted file, else (no usable config directory)
/// an in-memory store that forgets on exit. Honors [`BACKEND_ENV`], and moves
/// any old `credentials.enc` into the store (see [`migrate_legacy`]).
pub fn open_default() -> Box<dyn SecretStore> {
    let paths = match SecretPaths::default_dir() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("secret store: {e:#}; secrets will not be saved this run");
            return Box::new(MemoryStore::new());
        }
    };
    let store = open_in(&paths, BackendChoice::from_env());
    match migrate_legacy_from(&paths.legacy_credentials(), store.as_ref()) {
        Ok(0) => {}
        Ok(n) => tracing::info!("secret store: moved {n} saved password(s) from credentials.enc"),
        Err(e) => tracing::warn!("secret store: could not migrate credentials.enc: {e:#}"),
    }
    store
}

/// [`open_default`] without the legacy migration, for an explicit directory.
pub fn open_in(paths: &SecretPaths, choice: BackendChoice) -> Box<dyn SecretStore> {
    if choice == BackendChoice::Auto {
        match KeychainStore::open(paths).and_then(|k| k.probe().map(|_| k)) {
            Ok(k) => {
                tracing::info!("secret store: using the OS keychain");
                return Box::new(k);
            }
            Err(e) => tracing::info!(
                "secret store: OS keychain unavailable ({e:#}); using the encrypted file"
            ),
        }
    }
    match FileStore::open(paths, None) {
        Ok(f) => {
            tracing::info!(
                "secret store: using the encrypted file {}",
                f.vault_path.display()
            );
            Box::new(f)
        }
        Err(e) => {
            tracing::warn!("secret store: encrypted file unavailable ({e:#}); secrets will not be saved this run");
            Box::new(MemoryStore::new())
        }
    }
}

/// One process-wide store from [`open_default`], opened on first use (the
/// keychain probe runs once). Front ends use this to save passwords into the
/// same store the engine reads.
pub fn shared_default() -> Arc<dyn SecretStore> {
    static SHARED: OnceLock<Arc<dyn SecretStore>> = OnceLock::new();
    SHARED.get_or_init(|| Arc::from(open_default())).clone()
}

/// How the engine finds saved passwords. Cheap to clone.
#[derive(Clone, Default)]
pub enum SecretLookup {
    /// Never look up saved passwords.
    #[default]
    Disabled,
    /// Use [`shared_default`], opened lazily the first time a password is needed.
    SharedDefault,
    /// Use this store.
    Store(Arc<dyn SecretStore>),
}

impl std::fmt::Debug for SecretLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("Disabled"),
            Self::SharedDefault => f.write_str("SharedDefault"),
            Self::Store(s) => write!(f, "Store({})", s.backend()),
        }
    }
}

impl SecretLookup {
    /// The store, opening the shared default if needed. May block (keychain
    /// probe, D-Bus): call from a blocking context.
    pub fn resolve(&self) -> Option<Arc<dyn SecretStore>> {
        match self {
            Self::Disabled => None,
            Self::SharedDefault => Some(shared_default()),
            Self::Store(s) => Some(Arc::clone(s)),
        }
    }
}

/// The saved password for an SSH source: `ssh:{user}@{host}:{port}` first,
/// then a migrated `legacy:{source_name}`. Lookup errors are logged (without
/// the secret) and treated as "none saved".
pub fn saved_ssh_password(
    store: &dyn SecretStore,
    source_name: &str,
    user: &str,
    host: &str,
    port: u16,
) -> Option<SecretString> {
    for id in [ssh_password_id(user, host, port), legacy_id(source_name)] {
        match store.get(&id) {
            Ok(Some(s)) => {
                tracing::debug!("using the saved password {id} ({})", store.backend());
                return Some(s);
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("could not read saved password {id}: {e:#}"),
        }
    }
    None
}

// ---------------------------------------------------------------------------
// OS keychain
// ---------------------------------------------------------------------------

/// Secrets in the OS credential store, under service [`SERVICE`] with the id as
/// the account name.
pub struct KeychainStore {
    store: Arc<keyring_core::CredentialStore>,
    index_path: PathBuf,
    index_lock: Mutex<()>,
}

impl std::fmt::Debug for KeychainStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeychainStore")
            .field("index_path", &self.index_path)
            .finish_non_exhaustive()
    }
}

#[cfg(target_os = "windows")]
fn platform_store() -> keyring_core::Result<Arc<keyring_core::CredentialStore>> {
    Ok(windows_native_keyring_store::Store::new()?)
}

#[cfg(target_os = "macos")]
fn platform_store() -> keyring_core::Result<Arc<keyring_core::CredentialStore>> {
    Ok(apple_native_keyring_store::keychain::Store::new()?)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_store() -> keyring_core::Result<Arc<keyring_core::CredentialStore>> {
    Ok(zbus_secret_service_keyring_store::Store::new()?)
}

#[cfg(not(any(target_os = "windows", unix)))]
fn platform_store() -> keyring_core::Result<Arc<keyring_core::CredentialStore>> {
    Err(keyring_core::Error::NotSupportedByStore(
        "no OS keychain on this platform".to_string(),
    ))
}

/// Describe a keychain error without ever including secret bytes (some
/// variants carry the raw stored value).
fn keychain_error(action: &str, id: &str, e: keyring_core::Error) -> anyhow::Error {
    use keyring_core::Error as E;
    let why = match e {
        E::PlatformFailure(p) => format!("platform failure: {p}"),
        E::NoStorageAccess(p) => format!("keychain locked or access denied: {p}"),
        E::NoEntry => "no such entry".to_string(),
        E::BadEncoding(_) => "stored value is not UTF-8".to_string(),
        E::BadDataFormat(_, _) => "stored value is in an unexpected format".to_string(),
        E::BadStoreFormat(s) => format!("keychain is in an unexpected format: {s}"),
        E::TooLong(attr, limit) => format!("{attr} is longer than the keychain limit of {limit}"),
        E::Invalid(param, reason) => format!("invalid {param}: {reason}"),
        E::Ambiguous(_) => "more than one matching keychain entry".to_string(),
        E::NoDefaultStore => "no keychain configured".to_string(),
        E::NotSupportedByStore(s) => format!("not supported by this keychain: {s}"),
        _ => "unknown keychain error".to_string(),
    };
    anyhow!("keychain {action} of {id} failed: {why}")
}

impl KeychainStore {
    /// Connect to the platform keychain. Succeeding here does not prove the
    /// keychain works (a Secret Service can be present but locked); call
    /// [`KeychainStore::probe`].
    pub fn open(paths: &SecretPaths) -> Result<Self> {
        let store = platform_store().map_err(|e| keychain_error("open", SERVICE, e))?;
        Ok(Self {
            store,
            index_path: paths.index(),
            index_lock: Mutex::new(()),
        })
    }

    /// A real round trip: set, read back and delete a random probe value.
    pub fn probe(&self) -> Result<()> {
        let value = SecretString::from(hex(&Nonce::generate()));
        self.set_raw(PROBE_ID, &value)?;
        let back = self.get_raw(PROBE_ID);
        let _ = self.delete_raw(PROBE_ID);
        match back? {
            Some(b) if b.expose_secret() == value.expose_secret() => Ok(()),
            _ => bail!("keychain probe read back a different value"),
        }
    }

    fn entry(&self, id: &str) -> Result<keyring_core::Entry> {
        self.store
            .build(SERVICE, id, None)
            .map_err(|e| keychain_error("lookup", id, e))
    }

    fn get_raw(&self, id: &str) -> Result<Option<SecretString>> {
        match self.entry(id)?.get_password() {
            Ok(p) => Ok(Some(SecretString::from(p))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(keychain_error("read", id, e)),
        }
    }

    fn set_raw(&self, id: &str, secret: &SecretString) -> Result<()> {
        self.entry(id)?
            .set_password(secret.expose_secret())
            .map_err(|e| keychain_error("write", id, e))
    }

    fn delete_raw(&self, id: &str) -> Result<()> {
        match self.entry(id)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(keychain_error("delete", id, e)),
        }
    }

    fn update_index(&self, f: impl FnOnce(&mut BTreeSet<String>)) -> Result<()> {
        let _guard = self
            .index_lock
            .lock()
            .map_err(|_| anyhow!("index lock poisoned"))?;
        let mut ids = read_index(&self.index_path)?;
        f(&mut ids);
        let json = serde_json::to_vec_pretty(&ids)?;
        write_private_atomic(&self.index_path, &json)
    }
}

fn read_index(path: &Path) -> Result<BTreeSet<String>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("secret index {} is not valid JSON", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

impl SecretStore for KeychainStore {
    fn get(&self, id: &str) -> Result<Option<SecretString>> {
        self.get_raw(id)
    }

    fn set(&self, id: &str, secret: &SecretString) -> Result<()> {
        self.set_raw(id, secret)?;
        self.update_index(|ids| {
            ids.insert(id.to_string());
        })
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.delete_raw(id)?;
        self.update_index(|ids| {
            ids.remove(id);
        })
    }

    fn ids(&self) -> Result<Vec<String>> {
        Ok(read_index(&self.index_path)?.into_iter().collect())
    }

    fn backend(&self) -> &'static str {
        "keychain"
    }
}

// ---------------------------------------------------------------------------
// Encrypted file
// ---------------------------------------------------------------------------

/// Secrets in one AES-256-GCM encrypted file holding a JSON map id -> value.
/// Every write re-encrypts the whole map with a fresh random nonce and replaces
/// the file atomically.
pub struct FileStore {
    vault_path: PathBuf,
    key: Zeroizing<[u8; KEY_LEN]>,
    lock: Mutex<()>,
}

impl std::fmt::Debug for FileStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileStore")
            .field("vault_path", &self.vault_path)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

impl FileStore {
    /// Open (or prepare) the vault in `paths`. The data key is read from
    /// `key_holder` under [`VAULT_KEY_ID`] when given, else from the key file;
    /// a new random key is created only when no vault exists yet, so a lost key
    /// is reported instead of silently replacing the vault.
    pub fn open(paths: &SecretPaths, key_holder: Option<&dyn SecretStore>) -> Result<Self> {
        let vault_path = paths.vault();
        let vault_exists = vault_path.exists();
        let key = match key_holder {
            Some(holder) => match holder.get(VAULT_KEY_ID)? {
                Some(encoded) => decode_key(encoded.expose_secret())?,
                None => {
                    ensure_no_vault(vault_exists, &vault_path, "in the keychain")?;
                    let key = new_key();
                    let encoded = SecretString::from(BASE64.encode(key.as_ref()));
                    holder.set(VAULT_KEY_ID, &encoded)?;
                    key
                }
            },
            None => load_or_create_key_file(&paths.key_file(), vault_exists, &vault_path)?,
        };
        Ok(Self::with_key(vault_path, key))
    }

    /// A vault at `vault_path` under an explicit key.
    pub fn with_key(vault_path: PathBuf, key: Zeroizing<[u8; KEY_LEN]>) -> Self {
        Self {
            vault_path,
            key,
            lock: Mutex::new(()),
        }
    }

    pub fn vault_path(&self) -> &Path {
        &self.vault_path
    }

    fn cipher(&self) -> Aes256Gcm {
        cipher_for(&self.key)
    }

    fn load(&self) -> Result<BTreeMap<String, SecretString>> {
        let data = match std::fs::read(&self.vault_path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", self.vault_path.display()))
            }
        };
        let body = data.strip_prefix(VAULT_HEADER).ok_or_else(|| {
            anyhow!(
                "{} is not an oxitailr secret vault",
                self.vault_path.display()
            )
        })?;
        if body.len() < NONCE_LEN {
            bail!("{} is truncated", self.vault_path.display());
        }
        let (nonce, ciphertext) = body.split_at(NONCE_LEN);
        let plaintext = Zeroizing::new(
            self.cipher()
                .decrypt(
                    &nonce_from(nonce)?,
                    Payload {
                        msg: ciphertext,
                        aad: VAULT_HEADER,
                    },
                )
                .map_err(|_| {
                    anyhow!(
                        "secret vault {} could not be decrypted: wrong key, or the file was modified",
                        self.vault_path.display()
                    )
                })?,
        );
        let map: BTreeMap<String, String> = serde_json::from_slice(&plaintext).map_err(|_| {
            anyhow!(
                "secret vault {} decrypted to invalid data",
                self.vault_path.display()
            )
        })?;
        Ok(map
            .into_iter()
            .map(|(k, v)| (k, SecretString::from(v)))
            .collect())
    }

    fn save(&self, map: &BTreeMap<String, SecretString>) -> Result<()> {
        let exposed: BTreeMap<&str, &str> = map
            .iter()
            .map(|(k, v)| (k.as_str(), v.expose_secret()))
            .collect();
        // Size the buffer for the worst-case JSON escaping up front so it never
        // reallocates and leaves an unzeroed copy behind.
        let cap = 2 + exposed
            .iter()
            .map(|(k, v)| 6 * (k.len() + v.len()) + 6)
            .sum::<usize>();
        let mut plaintext = Zeroizing::new(Vec::with_capacity(cap));
        serde_json::to_writer(&mut *plaintext, &exposed)?;

        let nonce = Nonce::generate();
        let ciphertext = self
            .cipher()
            .encrypt(
                &nonce,
                Payload {
                    msg: &plaintext,
                    aad: VAULT_HEADER,
                },
            )
            .map_err(|_| anyhow!("encrypting the secret vault failed"))?;

        let mut out = Vec::with_capacity(VAULT_HEADER.len() + NONCE_LEN + ciphertext.len());
        out.extend_from_slice(VAULT_HEADER);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        write_private_atomic(&self.vault_path, &out)
    }

    fn locked(&self) -> Result<std::sync::MutexGuard<'_, ()>> {
        self.lock
            .lock()
            .map_err(|_| anyhow!("secret vault lock poisoned"))
    }
}

impl SecretStore for FileStore {
    fn get(&self, id: &str) -> Result<Option<SecretString>> {
        let _g = self.locked()?;
        Ok(self.load()?.remove(id))
    }

    fn set(&self, id: &str, secret: &SecretString) -> Result<()> {
        let _g = self.locked()?;
        let mut map = self.load()?;
        map.insert(id.to_string(), secret.clone());
        self.save(&map)
    }

    fn delete(&self, id: &str) -> Result<()> {
        let _g = self.locked()?;
        let mut map = self.load()?;
        if map.remove(id).is_some() {
            self.save(&map)?;
        }
        Ok(())
    }

    fn ids(&self) -> Result<Vec<String>> {
        let _g = self.locked()?;
        Ok(self.load()?.into_keys().collect())
    }

    fn backend(&self) -> &'static str {
        "file"
    }
}

fn new_key() -> Zeroizing<[u8; KEY_LEN]> {
    let generated = Key::<Aes256Gcm>::generate();
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    key.copy_from_slice(&generated);
    key
}

fn decode_key(encoded: &str) -> Result<Zeroizing<[u8; KEY_LEN]>> {
    let bytes = Zeroizing::new(
        BASE64
            .decode(encoded.trim())
            .map_err(|_| anyhow!("vault key is not valid base64"))?,
    );
    if bytes.len() != KEY_LEN {
        bail!("vault key has the wrong length");
    }
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    key.copy_from_slice(&bytes);
    Ok(key)
}

fn ensure_no_vault(vault_exists: bool, vault_path: &Path, place: &str) -> Result<()> {
    if vault_exists {
        bail!(
            "{} exists but its key is missing {place}; refusing to create a new key over it",
            vault_path.display()
        );
    }
    Ok(())
}

fn load_or_create_key_file(
    path: &Path,
    vault_exists: bool,
    vault_path: &Path,
) -> Result<Zeroizing<[u8; KEY_LEN]>> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let text = Zeroizing::new(text);
            restrict_permissions(path)?;
            decode_key(&text).with_context(|| format!("reading {}", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            ensure_no_vault(
                vault_exists,
                vault_path,
                &format!("({} not found)", path.display()),
            )?;
            let key = new_key();
            let encoded = Zeroizing::new(BASE64.encode(key.as_ref()));
            write_private_atomic(path, encoded.as_bytes())?;
            Ok(key)
        }
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Write `bytes` to a temp file (0600 on Unix) beside `path`, flush it to disk
/// and rename it over `path`, so a crash never leaves a half-written file.
fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(format!(".tmp-{}", std::process::id()));
    let tmp = dir.join(tmp_name);

    let result = (|| -> Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        restrict_permissions(&tmp)?; // a stale temp file may predate the mode
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.with_context(|| format!("writing {}", path.display()))
}

/// 0600 on Unix. On Windows files under the user profile are already limited
/// to that user (and administrators) by the profile ACL.
fn restrict_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("setting permissions on {}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// In memory
// ---------------------------------------------------------------------------

/// Secrets held only in this process; the last resort when nothing on disk is
/// usable, and handy in tests.
#[derive(Default)]
pub struct MemoryStore {
    map: Mutex<BTreeMap<String, SecretString>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn map(&self) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, SecretString>>> {
        self.map
            .lock()
            .map_err(|_| anyhow!("memory store lock poisoned"))
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, id: &str) -> Result<Option<SecretString>> {
        Ok(self.map()?.get(id).cloned())
    }
    fn set(&self, id: &str, secret: &SecretString) -> Result<()> {
        self.map()?.insert(id.to_string(), secret.clone());
        Ok(())
    }
    fn delete(&self, id: &str) -> Result<()> {
        self.map()?.remove(id);
        Ok(())
    }
    fn ids(&self) -> Result<Vec<String>> {
        Ok(self.map()?.keys().cloned().collect())
    }
    fn backend(&self) -> &'static str {
        "memory"
    }
}

// ---------------------------------------------------------------------------
// Migration from credentials.enc
// ---------------------------------------------------------------------------

/// Move the passwords in the old `<config_dir>/oxitailr/credentials.enc` into
/// `store` under [`legacy_id`]`(source name)`, then delete the old file.
/// Returns how many were moved (0 when there is no old file). On any failure
/// the old file is left in place.
pub fn migrate_legacy(store: &dyn SecretStore) -> Result<usize> {
    migrate_legacy_from(&SecretPaths::default_dir()?.legacy_credentials(), store)
}

/// [`migrate_legacy`] for an explicit `credentials.enc` path.
pub fn migrate_legacy_from(path: &Path, store: &dyn SecretStore) -> Result<usize> {
    migrate_legacy_with(path, store, &legacy::machine_key_material())
}

fn migrate_legacy_with(path: &Path, store: &dyn SecretStore, key_material: &str) -> Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let key = legacy::derive_key(key_material)?;
    let entries = legacy::decrypt_file(path, &key)?;
    let count = entries.len();
    for (name, password) in entries {
        store.set(&legacy_id(&name), &password)?;
    }
    std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    Ok(count)
}

/// LEGACY: the pre-0.4 credentials file. Its key was derived from the user name
/// and home directory — public, guessable values — so it was obfuscation, not
/// protection. Kept only to read old files once during migration.
mod legacy {
    use super::*;
    use argon2::{Algorithm, Argon2, Params, Version};

    const APP_SALT_PREFIX: &[u8] = b"oxitailr-credential-storage-v2";

    /// `$USER` (or `%USERNAME%`) followed by the home directory path.
    pub(super) fn machine_key_material() -> String {
        let mut material = String::new();
        if let Ok(user) = std::env::var("USER").or_else(|_| std::env::var("USERNAME")) {
            material.push_str(&user);
        }
        if let Some(home) = dirs::home_dir() {
            material.push_str(&home.to_string_lossy());
        }
        material
    }

    /// Argon2id v0x13, m=19456 KiB, t=2, p=1 (argon2 0.5's defaults, spelled
    /// out so a crate upgrade cannot change them).
    pub(super) fn derive_key(material: &str) -> Result<Zeroizing<[u8; KEY_LEN]>> {
        let mut salt = Vec::with_capacity(APP_SALT_PREFIX.len() + material.len());
        salt.extend_from_slice(APP_SALT_PREFIX);
        salt.extend_from_slice(material.as_bytes());
        let params = Params::new(19 * 1024, 2, 1, Some(KEY_LEN))
            .map_err(|e| anyhow!("legacy key parameters: {e}"))?;
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(material.as_bytes(), &salt, key.as_mut())
            .map_err(|e| anyhow!("legacy key derivation failed: {e}"))?;
        Ok(key)
    }

    /// Base64 text of nonce(12) || AES-256-GCM ciphertext of a JSON map
    /// source name -> password, no associated data.
    pub(super) fn decrypt_file(
        path: &Path,
        key: &[u8; KEY_LEN],
    ) -> Result<BTreeMap<String, SecretString>> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let decoded = BASE64
            .decode(text.trim())
            .map_err(|_| anyhow!("{} is not valid base64", path.display()))?;
        if decoded.len() < NONCE_LEN {
            // The old reader treated this as "no credentials".
            return Ok(BTreeMap::new());
        }
        let (nonce, ciphertext) = decoded.split_at(NONCE_LEN);
        let plaintext = Zeroizing::new(
            cipher_for(key)
                .decrypt(&nonce_from(nonce)?, ciphertext)
                .map_err(|_| {
                    anyhow!("{} could not be decrypted on this account", path.display())
                })?,
        );
        let map: BTreeMap<String, String> = serde_json::from_slice(&plaintext)
            .map_err(|_| anyhow!("{} decrypted to invalid data", path.display()))?;
        Ok(map
            .into_iter()
            .map(|(k, v)| (k, SecretString::from(v)))
            .collect())
    }

    /// Write a file the way the old code did (tests only).
    #[cfg(test)]
    pub(super) fn encrypt_file(path: &Path, key: &[u8; KEY_LEN], creds: &BTreeMap<String, String>) {
        let nonce = Nonce::generate();
        let ct = cipher_for(key)
            .encrypt(&nonce, serde_json::to_vec(creds).unwrap().as_slice())
            .unwrap();
        let mut data = nonce.to_vec();
        data.extend(ct);
        std::fs::write(path, BASE64.encode(&data)).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(s: &str) -> SecretString {
        SecretString::from(s.to_string())
    }

    fn file_store(dir: &Path) -> FileStore {
        FileStore::open(&SecretPaths::new(dir), None).unwrap()
    }

    fn get(store: &dyn SecretStore, id: &str) -> Option<String> {
        store
            .get(id)
            .unwrap()
            .map(|s| s.expose_secret().to_string())
    }

    #[test]
    fn ssh_id_lowercases_host_only() {
        assert_eq!(
            ssh_password_id("Admin", "Web01.Example.COM", 2222),
            "ssh:Admin@web01.example.com:2222"
        );
    }

    #[test]
    fn file_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = file_store(dir.path());
        assert_eq!(store.backend(), "file");
        assert!(store.ids().unwrap().is_empty());
        assert_eq!(get(&store, "a"), None);

        store.set("b", &secret("two")).unwrap();
        store.set("a", &secret("one")).unwrap();
        assert_eq!(get(&store, "a").as_deref(), Some("one"));
        assert_eq!(store.ids().unwrap(), vec!["a", "b"]);

        store.delete("a").unwrap();
        store.delete("missing").unwrap();
        assert_eq!(get(&store, "a"), None);
        assert_eq!(store.ids().unwrap(), vec!["b"]);

        // A second instance (same key file) reads the same vault.
        assert_eq!(get(&file_store(dir.path()), "b").as_deref(), Some("two"));
    }

    #[test]
    fn overwrite_replaces_value() {
        let dir = tempfile::tempdir().unwrap();
        let store = file_store(dir.path());
        store.set("x", &secret("old")).unwrap();
        store.set("x", &secret("new")).unwrap();
        assert_eq!(get(&store, "x").as_deref(), Some("new"));
        assert_eq!(store.ids().unwrap(), vec!["x"]);
    }

    #[test]
    fn wrong_key_fails_without_revealing_secret() {
        let dir = tempfile::tempdir().unwrap();
        let store = file_store(dir.path());
        store
            .set("ssh:u@h:22", &secret("hunter2-wrong-key"))
            .unwrap();

        let other = FileStore::with_key(store.vault_path().to_path_buf(), new_key());
        let err = format!("{:#}", other.get("ssh:u@h:22").unwrap_err());
        assert!(err.contains("could not be decrypted"), "{err}");
        assert!(!err.contains("hunter2"), "{err}");
    }

    #[test]
    fn tampered_vault_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let store = file_store(dir.path());
        store.set("id", &secret("tamper-me")).unwrap();

        let mut bytes = std::fs::read(store.vault_path()).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        std::fs::write(store.vault_path(), &bytes).unwrap();
        let err = format!("{:#}", store.get("id").unwrap_err());
        assert!(err.contains("could not be decrypted"), "{err}");

        // The header is authenticated too.
        bytes[last] ^= 0x01;
        bytes[VAULT_HEADER.len() - 1] = 0x02;
        std::fs::write(store.vault_path(), &bytes).unwrap();
        assert!(store.get("id").is_err());
    }

    #[test]
    fn nonce_differs_per_write() {
        let dir = tempfile::tempdir().unwrap();
        let store = file_store(dir.path());
        let nonce = || {
            std::fs::read(store.vault_path()).unwrap()[VAULT_HEADER.len()..][..NONCE_LEN].to_vec()
        };
        store.set("id", &secret("same")).unwrap();
        let first = nonce();
        store.set("id", &secret("same")).unwrap();
        assert_ne!(first, nonce());
    }

    #[test]
    fn vault_plaintext_is_not_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = file_store(dir.path());
        store
            .set("ssh:u@h:22", &secret("plain-text-canary"))
            .unwrap();
        let raw = std::fs::read(store.vault_path()).unwrap();
        assert!(!raw.windows(17).any(|w| w == b"plain-text-canary"));
    }

    #[test]
    fn secrets_never_print() {
        let s = secret("hunter2-debug");
        assert!(!format!("{s:?}").contains("hunter2"));
        let dir = tempfile::tempdir().unwrap();
        let store = file_store(dir.path());
        let debug = format!("{store:?}");
        assert!(debug.contains("[REDACTED]"), "{debug}");
    }

    #[cfg(unix)]
    #[test]
    fn files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let paths = SecretPaths::new(dir.path());
        let store = FileStore::open(&paths, None).unwrap();
        store.set("id", &secret("v")).unwrap();
        for p in [paths.key_file(), paths.vault()] {
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{}", p.display());
        }
    }

    #[test]
    fn missing_key_does_not_replace_existing_vault() {
        let dir = tempfile::tempdir().unwrap();
        let paths = SecretPaths::new(dir.path());
        file_store(dir.path()).set("id", &secret("v")).unwrap();
        std::fs::remove_file(paths.key_file()).unwrap();
        let err = format!("{:#}", FileStore::open(&paths, None).unwrap_err());
        assert!(err.contains("refusing"), "{err}");
        assert!(paths.vault().exists());
    }

    #[test]
    fn vault_key_can_live_in_another_store() {
        let dir = tempfile::tempdir().unwrap();
        let paths = SecretPaths::new(dir.path());
        let holder = MemoryStore::new();
        FileStore::open(&paths, Some(&holder))
            .unwrap()
            .set("id", &secret("v"))
            .unwrap();
        assert_eq!(holder.ids().unwrap(), vec![VAULT_KEY_ID]);
        assert!(!paths.key_file().exists());
        let reopened = FileStore::open(&paths, Some(&holder)).unwrap();
        assert_eq!(get(&reopened, "id").as_deref(), Some("v"));
    }

    #[test]
    fn open_in_file_choice_uses_file_backend() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_in(&SecretPaths::new(dir.path()), BackendChoice::File);
        assert_eq!(store.backend(), "file");
    }

    #[test]
    fn legacy_key_matches_old_derivation() {
        // Produced by the removed credentials.rs (argon2 0.5.3 defaults) for
        // USER=alice, home=/home/alice.
        let key = legacy::derive_key("alice/home/alice").unwrap();
        assert_eq!(
            hex(key.as_ref()),
            "e28bdebd391944e1116db3055a9253adcd81880bb51b7757e8fcb1386d41b146"
        );
    }

    #[test]
    fn legacy_migration_moves_entries_and_deletes_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = SecretPaths::new(dir.path());
        let material = "alice/home/alice";
        let key = legacy::derive_key(material).unwrap();
        let old: BTreeMap<String, String> = [("prod-web", "pw1"), ("db box", "pw2")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        legacy::encrypt_file(&paths.legacy_credentials(), &key, &old);

        let store = file_store(dir.path());
        assert_eq!(
            migrate_legacy_with(&paths.legacy_credentials(), &store, material).unwrap(),
            2
        );
        assert!(!paths.legacy_credentials().exists());
        assert_eq!(
            store.ids().unwrap(),
            vec!["legacy:db box", "legacy:prod-web"]
        );
        assert_eq!(get(&store, "legacy:prod-web").as_deref(), Some("pw1"));

        // Nothing left to do the second time.
        assert_eq!(
            migrate_legacy_with(&paths.legacy_credentials(), &store, material).unwrap(),
            0
        );
    }

    #[test]
    fn legacy_migration_with_wrong_account_keeps_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = SecretPaths::new(dir.path());
        let key = legacy::derive_key("alice/home/alice").unwrap();
        let old = BTreeMap::from([("s".to_string(), "pw".to_string())]);
        legacy::encrypt_file(&paths.legacy_credentials(), &key, &old);

        let store = MemoryStore::new();
        assert!(migrate_legacy_with(&paths.legacy_credentials(), &store, "bob/home/bob").is_err());
        assert!(paths.legacy_credentials().exists());
        assert!(store.ids().unwrap().is_empty());
    }

    #[test]
    fn saved_password_prefers_ssh_id_then_legacy() {
        let store = MemoryStore::new();
        assert!(saved_ssh_password(&store, "web", "u", "Host", 22).is_none());
        store.set(&legacy_id("web"), &secret("old")).unwrap();
        let got = saved_ssh_password(&store, "web", "u", "Host", 22).unwrap();
        assert_eq!(got.expose_secret(), "old");
        store
            .set(&ssh_password_id("u", "host", 22), &secret("new"))
            .unwrap();
        let got = saved_ssh_password(&store, "web", "u", "HOST", 22).unwrap();
        assert_eq!(got.expose_secret(), "new");
    }

    /// Needs a desktop session with an unlocked keychain:
    /// `cargo test -p oxitailr-core keychain -- --ignored`.
    #[test]
    #[ignore]
    fn keychain_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeychainStore::open(&SecretPaths::new(dir.path())).unwrap();
        store.probe().unwrap();
        let id = "test:oxitailr-keychain-round-trip";
        store.set(id, &secret("one")).unwrap();
        store.set(id, &secret("two")).unwrap();
        assert_eq!(get(&store, id).as_deref(), Some("two"));
        assert_eq!(store.ids().unwrap(), vec![id]);
        store.delete(id).unwrap();
        assert_eq!(get(&store, id), None);
        assert!(store.ids().unwrap().is_empty());
    }
}
