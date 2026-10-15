// "Add SSH source": follow a log file on another machine. The form is checked
// by the add_ssh command; a connection or login failure shows up later as the
// source's own error status, since connecting happens in the background.
// The password is held only while the dialog is open; it is saved (in the
// system's password vault, or an encrypted file) only when "Remember password"
// is ticked. A saved password shows as "saved"; leaving the field blank uses it.
import "./dialog.css";
import { useEffect, useId, useRef, useState } from "react";
import type { FormEvent, KeyboardEvent } from "react";
import { open as openFile } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import { Row, Toggle } from "./controls";
import RevealField from "./RevealField";

type Auth = "key" | "password";

/** The last part of a remote path ("/var/log/syslog" -> "syslog"). */
function basename(path: string): string {
  const parts = path.trim().replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || path.trim();
}

function message(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "The source could not be added.";
}

/** Where "Remember password" puts it, from api.secretBackend(). */
function rememberHint(backend: string | null): string {
  if (backend === "file") return "Saved in an encrypted file";
  if (backend === "memory") return "This system has no password vault: kept until the app closes";
  return "Saved in your system's password vault";
}

/** The port field as a number, null for blank (22), undefined when invalid. */
function parsePort(port: string): number | null | undefined {
  if (port.trim() === "") return null;
  const n = Number(port.trim());
  return Number.isInteger(n) && n >= 1 && n <= 65535 ? n : undefined;
}

export default function SshDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const titleId = useId();
  const hostRef = useRef<HTMLInputElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);

  const [host, setHost] = useState("");
  const [port, setPort] = useState("");
  const [user, setUser] = useState("");
  const [path, setPath] = useState("");
  const [name, setName] = useState("");
  const [auth, setAuth] = useState<Auth>("key");
  const [keyPath, setKeyPath] = useState("");
  const [password, setPassword] = useState("");
  const [remember, setRemember] = useState(false);
  const [backend, setBackend] = useState<string | null>(null);
  // The saved password's id for this user@host:port, or null when none is saved.
  const [savedId, setSavedId] = useState<string | null>(null);
  const [checkRev, setCheckRev] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Opening: focus Host. Closing: forget the password and any old error.
  useEffect(() => {
    if (open) {
      hostRef.current?.focus();
      let live = true;
      api
        .secretBackend()
        .then((b) => live && setBackend(b))
        .catch(() => live && setBackend(null));
      return () => {
        live = false;
      };
    } else {
      setPassword("");
      setRemember(false);
      setSavedId(null);
      setError(null);
      setBusy(false);
    }
  }, [open]);

  // Is a password already saved for this user@host:port? Checked ~300 ms after typing stops.
  useEffect(() => {
    if (!open) return;
    const h = host.trim();
    const u = user.trim();
    const p = parsePort(port);
    if (!h || !u || p === undefined) {
      setSavedId(null);
      return;
    }
    let live = true;
    const timer = window.setTimeout(async () => {
      try {
        const saved = await api.sshPasswordSaved(u, h, p);
        const id = saved ? await api.sshSecretId(u, h, p) : null;
        if (live) setSavedId(id);
      } catch {
        if (live) {
          setSavedId(null);
          setError("Could not check for a saved password.");
        }
      }
    }, 300);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [open, host, user, port, checkRev]);

  if (!open) return null;

  const ready = host.trim() !== "" && user.trim() !== "" && path.trim() !== "";
  const derivedName = host.trim() && path.trim() ? `${host.trim()}:${basename(path)}` : "host:file";

  const forget = async () => {
    if (!savedId) return;
    setError(null);
    try {
      await api.forgetSecret(savedId);
    } catch {
      setError("The saved password could not be forgotten.");
    }
    setCheckRev((n) => n + 1);
  };

  const browse = async () => {
    try {
      const picked = await openFile({ multiple: false, directory: false, title: "Choose a private key" });
      if (typeof picked === "string") setKeyPath(picked);
    } catch (e) {
      // error: LOGGED-OK — the picker was closed or failed; the field is unchanged
      console.warn("key file picker failed", e);
    }
  };

  const submit = async (e?: FormEvent) => {
    e?.preventDefault();
    if (!ready || busy) return;
    const portNum = parsePort(port);
    if (portNum === undefined) {
      setError("The port must be a whole number from 1 to 65535, or blank for 22.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await api.addSsh({
        name: name.trim(),
        host: host.trim(),
        port: portNum,
        user: user.trim(),
        path: path.trim(),
        keyPath: auth === "key" && keyPath.trim() ? keyPath.trim() : null,
        // Blank with a saved password: null, and the engine uses the saved one.
        password: auth === "password" && password ? password : null,
        remember: auth === "password" && remember && password !== "",
      });
      setPassword("");
      onClose();
    } catch (err) {
      setError(message(err));
    } finally {
      setBusy(false);
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
      return;
    }
    // Keep Tab inside the dialog.
    if (e.key === "Tab" && dialogRef.current) {
      const items = dialogRef.current.querySelectorAll<HTMLElement>(
        "input:not(:disabled), button:not(:disabled)",
      );
      if (!items.length) return;
      const first = items[0];
      const last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  };

  return (
    <div
      className="dk-dialog-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && !busy) onClose();
      }}
    >
      <div
        ref={dialogRef}
        className="dk-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onKeyDown={onKeyDown}
      >
        <h2 id={titleId} className="dk-dialog-title">
          Add SSH source
        </h2>
        <form className="dk-dialog-body" onSubmit={submit}>
          <Row label="Host">
            <input
              ref={hostRef}
              className="dk-input"
              aria-label="Host"
              value={host}
              onChange={(e) => setHost(e.target.value)}
              placeholder="server.example.com"
              autoComplete="off"
              spellCheck={false}
            />
          </Row>
          <Row label="Port">
            <input
              className="dk-input short"
              aria-label="Port"
              inputMode="numeric"
              value={port}
              onChange={(e) => setPort(e.target.value.replace(/[^0-9]/g, ""))}
              placeholder="22"
              autoComplete="off"
            />
          </Row>
          <Row label="User">
            <input
              className="dk-input"
              aria-label="User"
              value={user}
              onChange={(e) => setUser(e.target.value)}
              autoComplete="off"
              spellCheck={false}
            />
          </Row>
          <Row label="Remote file path">
            <input
              className="dk-input"
              aria-label="Remote file path"
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="/var/log/syslog"
              autoComplete="off"
              spellCheck={false}
            />
          </Row>
          <Row label="Name" sub="Optional">
            <input
              className="dk-input"
              aria-label="Name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={derivedName}
              autoComplete="off"
              spellCheck={false}
            />
          </Row>
          <Row label="Authentication">
            <div className="dk-seg" role="group" aria-label="Authentication">
              <button type="button" className={auth === "key" ? "on" : ""} aria-pressed={auth === "key"} onClick={() => setAuth("key")}>
                Key file
              </button>
              <button
                type="button"
                className={auth === "password" ? "on" : ""}
                aria-pressed={auth === "password"}
                onClick={() => setAuth("password")}
              >
                Password
              </button>
            </div>
          </Row>
          {auth === "key" ? (
            <Row label="Key file" sub="Blank tries your default keys in ~/.ssh">
              <input
                className="dk-input"
                aria-label="Key file"
                value={keyPath}
                onChange={(e) => setKeyPath(e.target.value)}
                placeholder="~/.ssh/id_ed25519"
                autoComplete="off"
                spellCheck={false}
              />
              <button type="button" className="dk-btn" onClick={browse}>
                Browse…
              </button>
            </Row>
          ) : (
            <>
              <Row label="Password" sub={savedId ? "Leave blank to use the saved one" : undefined}>
                <RevealField
                  aria-label="Password"
                  value={password}
                  onChange={setPassword}
                  savedId={savedId}
                  placeholder={savedId ? "•••••••• saved" : undefined}
                  onError={setError}
                />
                {savedId && (
                  <button type="button" className="dk-link-btn" onClick={forget}>
                    Forget
                  </button>
                )}
              </Row>
              <Toggle label="Remember password" sub={rememberHint(backend)} value={remember} onChange={setRemember} />
            </>
          )}

          {error && (
            <div className="dk-dialog-error" role="alert">
              {error}
            </div>
          )}

          <div className="dk-dialog-actions">
            <button type="button" className="dk-btn" onClick={onClose}>
              Cancel
            </button>
            <button type="submit" className="dk-btn primary" disabled={!ready || busy} aria-busy={busy}>
              {busy ? "Adding…" : "Add source"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
