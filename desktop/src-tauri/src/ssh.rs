//! "Add SSH source": the dialog's command. The form is checked here, in plain
//! English, before anything reaches the engine; connection and login failures
//! arrive later as the source's own error status, not from this call.
//!
//! The password is passed to the engine and never logged or echoed in an
//! error. With "Remember password" it is also saved in the secret store (the
//! OS credential vault, or the encrypted file); see `secrets_cmd`.

use oxitailr_core::secrets::SecretString;
use std::path::{Path, PathBuf};

/// A form that passed [`validate`], ready for `AppCore::add_ssh_source`.
#[derive(PartialEq, Eq)]
pub struct Validated {
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub user: String,
    pub path: String,
    pub key_path: Option<PathBuf>,
    pub password: Option<String>,
}

// Hand-written so a failing test (or anyone's `{:?}`) never prints the password.
impl std::fmt::Debug for Validated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Validated")
            .field("name", &self.name)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("path", &self.path)
            .field("key_path", &self.key_path)
            .field("password", &self.password.as_ref().map(|_| "<set>"))
            .finish()
    }
}

/// Expand a leading `~` (alone or followed by a separator) to `home`.
fn expand_tilde(raw: &str, home: Option<&Path>) -> PathBuf {
    if let Some(home) = home {
        if raw == "~" {
            return home.to_path_buf();
        }
        if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

/// The last component of a remote (always `/`-separated) path.
fn basename(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}

/// Check the form. `open_names` are the names of the sources already open;
/// `home` expands a leading `~` in the key path. Pure: touches the file system
/// only to check that the key file exists.
#[allow(clippy::too_many_arguments)]
pub fn validate<'a>(
    name: &str,
    host: &str,
    port: Option<u16>,
    user: &str,
    path: &str,
    key_path: Option<&str>,
    password: Option<String>,
    open_names: impl IntoIterator<Item = &'a String>,
    home: Option<&Path>,
) -> Result<Validated, String> {
    let host = host.trim();
    let user = user.trim();
    let path = path.trim();
    if host.is_empty() {
        return Err("Enter the host to connect to.".into());
    }
    if user.is_empty() {
        return Err("Enter the user to log in as.".into());
    }
    if path.is_empty() {
        return Err("Enter the path of the log file on the remote machine.".into());
    }
    if port == Some(0) {
        return Err("Port 0 is not a valid port; leave it blank for 22.".into());
    }

    let name = match name.trim() {
        "" => format!("{host}:{}", basename(path)),
        n => n.to_string(),
    };
    if open_names.into_iter().any(|n| *n == name) {
        return Err(format!(
            "A source named {name} is already open. Choose another name."
        ));
    }

    let key_path = match key_path.map(str::trim).filter(|k| !k.is_empty()) {
        None => None,
        Some(raw) => {
            let p = expand_tilde(raw, home);
            if !p.is_file() {
                return Err(format!("No key file found at {}.", p.display()));
            }
            Some(p)
        }
    };
    let password = password.filter(|p| !p.is_empty());

    Ok(Validated {
        name,
        host: host.to_string(),
        port,
        user: user.to_string(),
        path: path.to_string(),
        key_path,
        password,
    })
}

/// Open a file on another machine over SSH.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn add_ssh(
    shared: tauri::State<'_, crate::Shared>,
    name: String,
    host: String,
    port: Option<u16>,
    user: String,
    path: String,
    key_path: Option<String>,
    password: Option<String>,
    remember: bool,
) -> Result<(), String> {
    let home = dirs::home_dir();
    let v = {
        let host_state = shared.lock();
        validate(
            &name,
            &host,
            port,
            &user,
            &path,
            key_path.as_deref(),
            password,
            host_state.core.source_infos.keys(),
            home.as_deref(),
        )?
    };
    // Save first (off the window's thread; the engine lock isn't held), so a
    // vault that refuses is reported before anything connects.
    if remember {
        if let Some(pw) = &v.password {
            let store = crate::secrets_cmd::store(&shared)?;
            crate::secrets_cmd::save_ssh_password(
                store,
                &v.user,
                &v.host,
                v.port,
                SecretString::from(pw.clone()),
            )
            .await?;
        }
    }
    shared.lock().core.add_ssh_source(
        v.name, v.host, v.user, v.path, v.port, v.key_path, v.password,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(
        name: &str,
        host: &str,
        user: &str,
        path: &str,
        key: Option<&str>,
        pw: Option<&str>,
    ) -> Result<Validated, String> {
        validate(
            name,
            host,
            None,
            user,
            path,
            key,
            pw.map(String::from),
            &[] as &[String],
            None,
        )
    }

    #[test]
    fn required_fields() {
        assert!(ok("", " ", "u", "/p", None, None)
            .unwrap_err()
            .contains("host"));
        assert!(ok("", "h", "  ", "/p", None, None)
            .unwrap_err()
            .contains("user"));
        assert!(ok("", "h", "u", "", None, None)
            .unwrap_err()
            .contains("path"));
    }

    #[test]
    fn trims_and_defaults_name() {
        let v = ok(" ", " web1 ", " me ", " /var/log/syslog ", None, None).unwrap();
        assert_eq!(v.name, "web1:syslog");
        assert_eq!(v.host, "web1");
        assert_eq!(v.user, "me");
        assert_eq!(v.path, "/var/log/syslog");
        assert_eq!(v.port, None);
        let v = ok("", "h", "u", "/var/log/", None, None).unwrap();
        assert_eq!(v.name, "h:log");
        assert_eq!(
            ok(" mine ", "h", "u", "/p", None, None).unwrap().name,
            "mine"
        );
    }

    #[test]
    fn rejects_port_zero() {
        let e = validate(
            "",
            "h",
            Some(0),
            "u",
            "/p",
            None,
            None,
            &[] as &[String],
            None,
        );
        assert!(e.unwrap_err().contains("Port 0"));
    }

    #[test]
    fn duplicate_name_is_an_error() {
        let open = vec!["h:syslog".to_string()];
        let e = validate(
            "",
            "h",
            None,
            "u",
            "/var/log/syslog",
            None,
            None,
            &open,
            None,
        );
        assert_eq!(
            e.unwrap_err(),
            "A source named h:syslog is already open. Choose another name."
        );
        let v = validate(
            "other",
            "h",
            None,
            "u",
            "/var/log/syslog",
            None,
            None,
            &open,
            None,
        );
        assert!(v.is_ok());
    }

    #[test]
    fn key_path_must_exist_and_expands_tilde() {
        let dir = std::env::temp_dir().join(format!("oxitailr-ssh-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("id_test");
        std::fs::write(&key, "not a real key").unwrap();

        let v = validate(
            "",
            "h",
            None,
            "u",
            "/p",
            Some("~/id_test"),
            None,
            &[] as &[String],
            Some(&dir),
        )
        .unwrap();
        assert_eq!(v.key_path.as_deref(), Some(key.as_path()));

        let e = validate(
            "",
            "h",
            None,
            "u",
            "/p",
            Some("~/missing"),
            None,
            &[] as &[String],
            Some(&dir),
        )
        .unwrap_err();
        assert!(
            e.contains(&dir.join("missing").display().to_string()),
            "{e}"
        );

        // A blank key path means "no key".
        assert_eq!(
            ok("", "h", "u", "/p", Some("  "), None).unwrap().key_path,
            None
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_password_is_none_and_never_printed() {
        assert_eq!(
            ok("", "h", "u", "/p", None, Some("")).unwrap().password,
            None
        );
        let v = ok("", "h", "u", "/p", None, Some("hunter2")).unwrap();
        assert_eq!(v.password.as_deref(), Some("hunter2"));
        assert!(!format!("{v:?}").contains("hunter2"));
    }
}
