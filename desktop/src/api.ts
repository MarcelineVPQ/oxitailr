// The window's side of the Rust commands (desktop/src-tauri/src/main.rs).
// Every call to the back end goes through here; nothing else imports invoke.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Level = "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR" | "FATAL";
export const LEVELS: readonly Level[] = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL"];

export interface SourceInfo {
  name: string;
  source_type: "Local" | "Ssh";
  path: string;
  status: "Connecting" | "Connected" | "Disconnected" | "Error";
  line_count: number;
  enabled: boolean;
  file_size: number | null;
}

export interface AlertOut {
  rule: string;
  source: string;
  text: string;
}

export interface Snapshot {
  version: string;
  sources: SourceInfo[];
  presets: string[];
  show_timestamps: boolean;
  total_lines: number;
  shown: number;
  bytes: number;
  alerts: AlertOut[];
  src_rev: number;
}

/** A piece of a row: text, a highlight rule's color, or part of a search match. */
export interface Seg {
  t: string;
  c?: [number, number, number];
  s?: boolean;
}

export interface Row {
  /** Position in the view. */
  i: number;
  /** Line number in the source buffer. */
  n: number;
  src: string;
  ts?: string;
  lvl?: Level;
  segs: Seg[];
  mark?: boolean;
}

export interface Rows {
  total: number;
  rows: Row[];
  /** The view's rebuild count: positions from another epoch mean nothing now. */
  epoch: number;
  /** Rows dropped off the front in this epoch (buffer eviction); every position shifts down by one per drop. */
  dropped: number;
  /** Lines in the buffer, bytes across sources: the status line. */
  lines: number;
  bytes: number;
  /** Changes when the source list does: refetch state() only then. */
  src_rev: number;
  alerts: number;
}

/** An SSH source to tail (the SSH dialog's form). */
export interface SshForm {
  name: string;
  host: string;
  port: number | null;
  user: string;
  path: string;
  keyPath: string | null;
  password: string | null;
  /** Save the password in the OS credential vault (or the encrypted file). */
  remember: boolean;
}

/** A saved password, without its value. */
export interface SavedSecret {
  id: string;
  label: string;
  kind: "ssh" | "legacy" | "other";
}

export const api = {
  state: () => invoke<Snapshot>("state"),
  openPaths: (paths: string[]) => invoke<void>("open_paths", { paths }),
  closeSource: (name: string) => invoke<void>("close_source", { name }),
  /** Returns the filter's error text when it isn't a valid pattern. */
  setView: (v: { tab: string | null; filter: string; levels: boolean[]; search: string }) => invoke<string | null>("set_view", v),
  rows: (start: number, count: number) => invoke<Rows>("rows", { start, count }),
  find: (term: string, from: number | null, forward: boolean) => invoke<number | null>("find", { term, from, forward }),
  toggleBookmark: (source: string, line: number) => invoke<boolean>("toggle_bookmark", { source, line }),
  findBookmark: (from: number | null, forward: boolean) => invoke<number | null>("find_bookmark", { from, forward }),
  applyPreset: (name: string | null) => invoke<boolean>("apply_preset", { name }),
  clear: () => invoke<void>("clear"),
  reload: () => invoke<void>("reload"),
  dismissAlerts: () => invoke<void>("dismiss_alerts"),
  /** How much attention the window has: paces the back end's updates. */
  setAttention: (level: "focused" | "visible" | "hidden") => invoke<void>("set_attention", { level }),
  /** Rejects with a plain-English reason when the form is incomplete. */
  addSsh: (f: SshForm) => invoke<void>("add_ssh", { name: f.name, host: f.host, port: f.port, user: f.user, path: f.path, keyPath: f.keyPath, password: f.password, remember: f.remember }),
  /** Where passwords are kept: "keychain", "file" or "memory". */
  secretBackend: () => invoke<string>("secret_backend"),
  savedSecrets: () => invoke<SavedSecret[]>("saved_secrets"),
  sshPasswordSaved: (user: string, host: string, port: number | null) => invoke<boolean>("ssh_password_saved", { user, host, port }),
  sshSecretId: (user: string, host: string, port: number | null) => invoke<string>("ssh_secret_id", { user, host, port }),
  /** The saved value: call only from an explicit reveal (eye) click. */
  revealSecret: (id: string) => invoke<string | null>("reveal_secret", { id }),
  forgetSecret: (id: string) => invoke<void>("forget_secret", { id }),
};

/** Fires (coalesced, at most ~10/s) when lines or sources changed. */
export const onChanged = (fn: () => void): Promise<UnlistenFn> => listen("changed", () => fn());
/** The tray showed or hid the window: stop fetching while hidden. */
export const onWindowVisible = (fn: (visible: boolean) => void): Promise<UnlistenFn> => listen<boolean>("window-visible", (e) => fn(e.payload));
export const onAlert = (fn: (a: AlertOut) => void): Promise<UnlistenFn> => listen<AlertOut>("alert", (e) => fn(e.payload));
