// App settings: the app's own settings, in the settings panel beside the rail
// (on the left), on the DS5 configurator's framework (SplitView + Sections). The main view shows the
// readouts (MainBlocks): the config file as the app read it, and where saved
// passwords are kept. The panel holds settings and actions only. Every value
// shown comes from the back end (config_summary, startup_settings, the secret
// store), never from a guess.
import "../dialog.css";
import { useCallback, useEffect, useRef, useState } from "react";
import { AppWindow, Eye, EyeOff, FileCog, FileText, FolderOpen, KeyRound, Palette } from "lucide-react";
import { Segmented, Toggle } from "../controls";
import { useTheme, type Theme } from "../theme";
import { api, onStartupSettings, type ConfigSummary, type SavedSecret, type StartupSettings, type StartupSwitch } from "../api";
import { Section, SplitView } from "../accordion/Section";
import { MainBlock, useMainBlock } from "../accordion/MainBlock";
import { pinScope, useAccordion } from "../accordion/useAccordion";

const THEME_OPTIONS: ReadonlyArray<{ value: Theme; label: string; title?: string }> = [
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
  { value: "system", label: "System", title: "Follow the system's light or dark setting" },
];

const BACKEND_LABEL: Record<string, string> = {
  keychain: "The system password vault",
  file: "An encrypted file in the config folder",
  memory: "Memory only (not saved)",
};

const KIND_LABEL: Record<SavedSecret["kind"], string> = {
  ssh: "ssh",
  legacy: "older version",
  other: "other",
};

// `tray`: needs a system tray (without one the window can't hide where it can come back).
const STARTUP: ReadonlyArray<{ key: StartupSwitch; label: string; info: string; tray?: boolean }> = [
  { key: "close_to_tray", label: "Close to tray", info: "Closing the window hides it to the tray; tailing and alerts carry on. Quit from the tray menu.", tray: true },
  { key: "start_minimized", label: "Start minimized", info: "The window starts hidden in the tray.", tray: true },
  { key: "launch_at_login", label: "Launch at login", info: "Oxitailr starts when you sign in, in the tray." },
  { key: "notify_alerts", label: "Notify me of alerts", info: "An alert raises a desktop notification while the window isn't focused (at most one every 10 seconds)." },
];

/** One saved password: the eye fetches the real value on click; hiding drops it. */
function SecretRow({ secret, onForgotten }: { secret: SavedSecret; onForgotten: () => void }) {
  const [revealed, setRevealed] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ticket = useRef(0);

  const toggle = async () => {
    setError(null);
    if (revealed !== null) {
      ticket.current += 1;
      setRevealed(null);
      return;
    }
    const mine = ++ticket.current;
    setBusy(true);
    try {
      const v = await api.revealSecret(secret.id);
      if (mine !== ticket.current) return;
      if (v === null) setError("This password is no longer saved.");
      else setRevealed(v);
    } catch {
      // error: SHOWN — under the row
      if (mine === ticket.current) setError("The password could not be read.");
    } finally {
      if (mine === ticket.current) setBusy(false);
    }
  };

  const forget = async () => {
    setError(null);
    ticket.current += 1;
    setRevealed(null);
    try {
      await api.forgetSecret(secret.id);
      onForgotten();
    } catch {
      // error: SHOWN — under the row
      setError("The password could not be forgotten.");
      setConfirming(false);
    }
  };

  const shown = revealed !== null;
  const eyeLabel = shown ? `Hide the password for ${secret.label}` : `Show the password for ${secret.label}`;

  return (
    <div className="dk-row dk-secret-row">
      <span className="dk-label">
        <span className="dk-secret-name">{secret.label}</span>
        <span className="dk-chip">{KIND_LABEL[secret.kind] ?? secret.kind}</span>
      </span>
      <div className="dk-ctl">
        {shown && <code className="page-code dk-secret-value">{revealed}</code>}
        <button type="button" className="dk-btn dk-icon-btn" aria-label={eyeLabel} title={eyeLabel} aria-pressed={shown} aria-busy={busy} disabled={busy} onClick={toggle}>
          {shown ? <EyeOff size={14} aria-hidden="true" /> : <Eye size={14} aria-hidden="true" />}
        </button>
        {confirming ? (
          <span className="dk-confirm" role="group" aria-label={`Forget the password for ${secret.label}?`}>
            <span>Forget?</span>
            <button type="button" className="dk-btn danger" onClick={forget}>
              Yes
            </button>
            <button type="button" className="dk-btn" onClick={() => setConfirming(false)}>
              No
            </button>
          </span>
        ) : (
          <button type="button" className="dk-btn" onClick={() => setConfirming(true)}>
            Forget
          </button>
        )}
      </div>
      {error && (
        <span className="dk-inline-error dk-secret-error" role="alert">
          {error}
        </span>
      )}
    </div>
  );
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

export default function SettingsPage() {
  const [theme, setTheme] = useTheme();
  const [startup, setStartup] = useState<StartupSettings | null>(null);
  const [startupError, setStartupError] = useState<string | null>(null);
  const [config, setConfig] = useState<ConfigSummary | null>(null);
  const [backend, setBackend] = useState<string | null>(null);
  const [secrets, setSecrets] = useState<SavedSecret[] | null>(null);
  const [secretsError, setSecretsError] = useState<string | null>(null);
  const [folderError, setFolderError] = useState<string | null>(null);
  const acc = useAccordion("app", pinScope("app"));
  const configBlock = useMainBlock("app", "configFile");
  const storeBlock = useMainBlock("app", "passwordStore");

  const loadSecrets = useCallback(async () => {
    try {
      const [b, list] = await Promise.all([api.secretBackend(), api.savedSecrets()]);
      setBackend(b);
      setSecrets(list);
      setSecretsError(null);
    } catch {
      // error: SHOWN — in the Saved passwords section
      setSecretsError("The saved passwords could not be listed.");
    }
  }, []);

  // Everything the page shows, once on open; the startup switches then follow
  // the tray menu live (the startup-settings event). Nothing polls.
  // lifetime: OK — cleanup removes the startup-settings listener; `alive` drops late answers (unmount)
  useEffect(() => {
    let alive = true;
    let off: (() => void) | undefined;
    void api
      .startupSettings()
      .then((s) => alive && setStartup(s))
      // error: SHOWN — in the Startup section
      .catch((e) => alive && setStartupError(`The startup settings could not be read: ${e}`));
    void api
      .configSummary()
      .then((c) => alive && setConfig(c))
      // error: LOGGED-OK — the Config file block says "Reading…" until a reopen
      .catch((e) => console.warn("config summary failed", e));
    void loadSecrets();
    void onStartupSettings((s) => setStartup(s)).then((u) => (alive ? (off = u) : u()));
    return () => {
      alive = false;
      off?.();
    };
  }, [loadSecrets]);

  const setSwitch = async (key: StartupSwitch, value: boolean) => {
    setStartupError(null);
    try {
      setStartup(await api.setStartupSetting(key, value));
    } catch (e) {
      // error: SHOWN — in the Startup section
      setStartupError(String(e));
    }
  };

  const openFolder = async () => {
    setFolderError(null);
    try {
      await api.openConfigFolder();
    } catch (e) {
      // error: SHOWN — in the Config file section
      setFolderError(String(e));
    }
  };

  // Status lines: from the state the page already holds.
  const themeStatus = THEME_OPTIONS.find((o) => o.value === theme)?.label ?? theme;
  // loop: NOT-WORTH — N = 4 switches; render
  const switches = startup ? STARTUP.filter((s) => startup.tray || !s.tray) : [];
  const startupStatus = startup
    ? // loop: NOT-WORTH — N ≤ 4 switches; render
      switches
        .filter((s) => startup[s.key])
        .map((s) => s.label)
        .join(" · ") || "All off"
    : null;
  const secretsStatus = secrets ? (secrets.length ? `${plural(secrets.length, "password")} saved` : "None saved") : null;
  const configStatus = !config ? "Reading…" : config.problem ? "Couldn't be read: on the defaults" : config.exists ? "Read at start" : "Not created yet: on the defaults";

  const main = (
    <div className="dk-about-text">
      <MainBlock state={configBlock} icon={FileText} title="Config file" status={configStatus} task={config?.problem ? "Couldn't be read" : null}>
        {config && (
          <>
            {config.problem && (
              <p className="dk-hint dk-inline-error" role="alert">
                {config.problem}. Fix the file (see config.example.toml), then restart Oxitailr.
              </p>
            )}
            <dl className="live-data-rows">
              <div>
                <dt>Where</dt>
                <dd className="dk-dd-path">{config.path}</dd>
              </div>
              <div>
                <dt>Sources</dt>
                <dd>{config.sources}</dd>
              </div>
              <div>
                <dt>Filter presets</dt>
                <dd>{config.presets}</dd>
              </div>
              <div>
                <dt>Highlight rules</dt>
                <dd>{config.highlights}</dd>
              </div>
              <div>
                <dt>Alert rules</dt>
                <dd>{config.alerts}</dd>
              </div>
              <div>
                <dt>Lines kept</dt>
                <dd>{config.buffer_size.toLocaleString()}</dd>
              </div>
            </dl>
            <p className="dk-hint">Edits to the file apply the next time Oxitailr starts.</p>
          </>
        )}
      </MainBlock>
      <MainBlock state={storeBlock} icon={KeyRound} title="Password store" status={backend ? (BACKEND_LABEL[backend] ?? backend) : "Reading…"}>
        <p className="dk-hint">
          {backend === "memory"
            ? "No password vault answered and no file could be used, so a password is kept only until Oxitailr quits."
            : "SSH passwords you choose to remember. They are never written to the config file."}
        </p>
      </MainBlock>
    </div>
  );

  return (
    <SplitView page="app" main={main} title="App settings" sub="This computer only · saved as you change them" state={acc}>
      <Section id="appearance" icon={Palette} title="Appearance" status={themeStatus}>
        {() => <Segmented label="Theme" value={theme} options={THEME_OPTIONS} onChange={setTheme} />}
      </Section>
      <Section id="startup" icon={AppWindow} title="Startup" status={startupStatus} idle={!startup && !startupError}>
        {() => (
          <>
            {startupError && (
              <p className="dk-hint dk-inline-error" role="alert">
                {startupError}
              </p>
            )}
            {/* loop: NOT-WORTH — N = 4 switches; only while Startup is open */}
            {startup && switches.map((s) => <Toggle key={s.key} label={s.label} info={s.info} value={startup[s.key]} onChange={(v) => void setSwitch(s.key, v)} />)}
            {startup && <p className="dk-hint">{startup.tray ? "The same switches as the tray menu." : "This desktop has no system tray, so the window always stays shown (no Close to tray or Start minimized)."}</p>}
          </>
        )}
      </Section>
      <Section id="passwords" icon={KeyRound} title="Saved passwords" status={secretsStatus} idle={!!secrets && secrets.length === 0 && !secretsError}>
        {() => (
          <>
            {secretsError && (
              <p className="dk-hint dk-inline-error" role="alert">
                {secretsError}
              </p>
            )}
            {/* loop: HOT-PATH-OK — a handful of saved passwords; only while the section is open */}
            {secrets?.map((s) => <SecretRow key={s.id} secret={s} onForgotten={loadSecrets} />)}
          </>
        )}
      </Section>
      <Section id="config" icon={FileCog} title="Config file" status="Sources, filters, highlights, alerts">
        {() => (
          <>
            <p className="dk-hint">Open its folder to edit config.toml (config.example.toml lists every setting).</p>
            <div className="dk-row-btns">
              <button type="button" className="dk-btn" onClick={() => void openFolder()}>
                <FolderOpen size={14} aria-hidden="true" /> Open the config folder
              </button>
            </div>
            {folderError && (
              <p className="dk-hint dk-inline-error" role="alert">
                {folderError}
              </p>
            )}
          </>
        )}
      </Section>
    </SplitView>
  );
}
