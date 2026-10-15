// Settings: the window's own look. Everything about what is tailed and how it
// is shown comes from the same config file the terminal app reads, so this
// page says where, instead of duplicating it.
import "../dialog.css";
import { useCallback, useEffect, useRef, useState } from "react";
import { Eye, EyeOff } from "lucide-react";
import { Segmented, Row } from "../controls";
import { useTheme, type Theme } from "../theme";
import { api, type SavedSecret } from "../api";

const CONFIG = "~/.config/oxitailr/config.toml";

const THEME_OPTIONS: ReadonlyArray<{ value: Theme; label: string; title?: string }> = [
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
  { value: "system", label: "System", title: "Follow the system's light or dark setting" },
];

const BACKEND_LABEL: Record<string, string> = {
  keychain: "System password vault",
  file: "Encrypted file in your config folder",
  memory: "Memory only (not saved)",
};

const KIND_LABEL: Record<SavedSecret["kind"], string> = {
  ssh: "ssh",
  legacy: "older version",
  other: "other",
};

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
        <button
          type="button"
          className="dk-btn dk-icon-btn"
          aria-label={eyeLabel}
          title={eyeLabel}
          aria-pressed={shown}
          aria-busy={busy}
          disabled={busy}
          onClick={toggle}
        >
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

function SavedPasswords() {
  const [backend, setBackend] = useState<string | null>(null);
  const [secrets, setSecrets] = useState<SavedSecret[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [b, list] = await Promise.all([api.secretBackend(), api.savedSecrets()]);
      setBackend(b);
      setSecrets(list);
      setError(null);
    } catch {
      setError("The saved passwords could not be listed.");
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="page-section">
      <h2>Saved passwords</h2>
      <Row label="Kept in">
        <span>{backend ? (BACKEND_LABEL[backend] ?? backend) : "…"}</span>
      </Row>
      {error && (
        <div className="dk-dialog-error" role="alert">
          {error}
        </div>
      )}
      {secrets && secrets.length === 0 && (
        <p className="dk-dialog-note">No saved passwords. Tick “Remember password” when adding an SSH source.</p>
      )}
      {/* loop: HOT-PATH-OK — a handful of saved passwords */}
      {secrets?.map((s) => <SecretRow key={s.id} secret={s} onForgotten={load} />)}
    </div>
  );
}

export default function SettingsPage() {
  const [theme, setTheme] = useTheme();
  return (
    <section className="page" aria-labelledby="settings-title">
      <header className="page-head">
        <h1 id="settings-title">Settings</h1>
        <p className="page-sub">The window's look. The rest is in the config file.</p>
      </header>

      <div className="page-section">
        <h2>Appearance</h2>
        <Segmented label="Theme" value={theme} options={THEME_OPTIONS} onChange={setTheme} />
      </div>

      <SavedPasswords />

      <div className="page-section">
        <h2>From the config file</h2>
        <Row label="Config file" sub="Shared with the terminal app" title={`The desktop app reads the same file as the terminal app: ${CONFIG}. Edit it, then Reload on the Logs page.`}>
          <code className="page-code">{CONFIG}</code>
        </Row>
        <Row label="Sources" sub="Files and SSH sources to open at start" title="[[sources]] entries (with enabled / auto_open): local paths and SSH sources opened when the app starts." />
        <Row label="Filters and presets" sub="Named filters you can apply from Logs" title="[filters.<name>] tables: a named filter, applied from the preset menu on the Logs page." />
        <Row label="Highlight rules" sub="Patterns colored in every line" title="[[highlights]] entries: a pattern and a color; matching text is colored in the log view." />
        <Row label="Alerts" sub="Patterns that raise an alert" title="[[alerts]] entries: a pattern that raises an alert (the bell on the Logs page and a badge on the rail) when a new line matches." />
      </div>
    </section>
  );
}
