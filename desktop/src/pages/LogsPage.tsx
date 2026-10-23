// The log view: every action as a control, and as a key. Only the rows on screen are ever fetched (a virtualized
// list over the back end's filtered view), so 50,000 lines cost the same as
// 50. The back end says "changed" at most ten times a second; between those,
// nothing here runs.
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { ArrowDownToLine, Bell, Bookmark, Download, Eraser, FolderOpen, RotateCw, Search, Server, X } from "lucide-react";
import SshDialog from "../SshDialog";
import { api, LEVELS, onChanged, onWindowVisible, type Level, type Row, type Snapshot } from "../api";

const ROW_H = 20; // px, fixed: the list is positioned by arithmetic, not measured
const OVERSCAN = 30; // rows fetched above and below the visible ones

const formatBytes = (n: number): string =>
  n < 1024 ? `${n} B` : n < 1024 ** 2 ? `${(n / 1024).toFixed(1)} KB` : n < 1024 ** 3 ? `${(n / 1024 ** 2).toFixed(1)} MB` : `${(n / 1024 ** 3).toFixed(2)} GB`;

const rowText = (r: Row): string => r.segs.map((s) => s.t).join("");

const isTyping = (t: EventTarget | null): boolean => t instanceof HTMLElement && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable);

/** What the rail's status line shows. */
export interface RailStatus {
  sources: number;
  alerts: number;
  tailing: boolean;
}

export default function LogsPage({ active, onStatus }: { active: boolean; onStatus?: (s: RailStatus) => void }) {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [tab, setTab] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [filterErr, setFilterErr] = useState<string | null>(null);
  const [levels, setLevels] = useState<boolean[]>(() => LEVELS.map(() => true));
  const [search, setSearch] = useState("");
  const [preset, setPreset] = useState<string | null>(null);
  const [follow, setFollow] = useState(true);
  const [total, setTotal] = useState(0);
  const [rows, setRows] = useState<Row[]>([]);
  const [cursor, setCursor] = useState<number | null>(null);
  const [alertsOpen, setAlertsOpen] = useState(false);
  const [sshOpen, setSshOpen] = useState(false);
  // What the last export wrote, or why it failed (shown in the status line until dismissed).
  const [exported, setExported] = useState<{ ok: boolean; text: string } | null>(null);
  const [viewH, setViewH] = useState(400);
  const [lines, setLines] = useState(0);
  const [bytes, setBytes] = useState(0);

  const listRef = useRef<HTMLDivElement>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const followRef = useRef(follow);
  followRef.current = follow;
  const totalRef = useRef(total);
  totalRef.current = total;
  // What the last rows reply said, to notice a rebuild, evictions, and a
  // changed source list without asking for the full state every time.
  const epochRef = useRef(-1);
  // A paused view is held still by LINE, not by position: once the buffer is
  // full every new line pushes the oldest out and all positions shift. The
  // top visible line (by number) and its pixel offset are remembered, and put
  // back after each redraw, in the same frame, before anything is painted.
  // The cursor is held the same way, by line number.
  const anchorRef = useRef<{ n: number; offset: number } | null>(null);
  const cursorNRef = useRef<number | null>(null);
  const srcRevRef = useRef(-1);
  const alertsRef = useRef(-1);
  // Set while this code moves the scrollbar, so onScroll can tell it from the user.
  const autoScroll = useRef(false);
  // The tray hid the window: no fetching until it's shown again.
  const hiddenRef = useRef(false);

  // ---- fetching: one request in flight, at most one queued behind it ----
  const inFlight = useRef(false);
  const queued = useRef(false);
  const fetchWindow = useCallback(async () => {
    if (inFlight.current) {
      queued.current = true;
      return;
    }
    inFlight.current = true;
    try {
      do {
        queued.current = false;
        const el = listRef.current;
        const h = el?.clientHeight ?? 400;
        const visible = Math.ceil(h / ROW_H);
        // Following: ask for the tail directly (the total may have grown).
        const top = followRef.current ? Number.MAX_SAFE_INTEGER : Math.floor((el?.scrollTop ?? 0) / ROW_H);
        let start = Math.max(0, Math.min(top, totalRef.current - visible) - OVERSCAN);
        let res = await api.rows(start, visible + OVERSCAN * 2);
        if (followRef.current && res.total > totalRef.current) {
          // The tail moved past what we asked for: ask once more at the new end.
          start = Math.max(0, res.total - visible - OVERSCAN);
          res = await api.rows(start, visible + OVERSCAN * 2);
        }
        if (res.epoch !== epochRef.current) {
          // Rebuilt (filter, levels, tab, clear): old positions mean nothing.
          epochRef.current = res.epoch;
          anchorRef.current = null;
          cursorNRef.current = null;
          setCursor(null);
        }
        if (res.src_rev !== srcRevRef.current || res.alerts !== alertsRef.current) {
          srcRevRef.current = res.src_rev;
          alertsRef.current = res.alerts;
          void refreshStateRef.current();
        }
        totalRef.current = res.total;
        setTotal(res.total);
        setRows(res.rows);
        setLines(res.lines);
        setBytes(res.bytes);
      } while (queued.current);
    } catch (e) {
      // error: LOGGED-OK — a failed fetch leaves the last rows on screen; the next change retries
      console.warn("rows fetch failed", e);
    } finally {
      inFlight.current = false;
    }
  }, []);

  const refreshState = useCallback(async () => {
    try {
      setSnap(await api.state());
    } catch (e) {
      // error: LOGGED-OK — the status line keeps its last values
      console.warn("state fetch failed", e);
    }
  }, []);

  const refreshStateRef = useRef(refreshState);
  refreshStateRef.current = refreshState;

  // The rail's status follows the source list.
  useEffect(() => {
    if (!snap) return;
    onStatus?.({ sources: snap.sources.length, alerts: snap.alerts.length, tailing: snap.sources.some((x) => x.status === "Connected") });
  }, [snap, onStatus]);

  // lifetime: OK — the "changed" listener is removed on unmount
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void onChanged(() => {
      if (!hiddenRef.current) void fetchWindow();
    }).then((u) => (gone ? u() : (off = u)));
    void fetchWindow();
    void refreshState();
    return () => {
      gone = true;
      off?.();
    };
  }, [fetchWindow, refreshState]);

  // Drag files onto the window to open them.
  // lifetime: OK — the drag-drop listener is removed on unmount
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void getCurrentWebview()
      .onDragDropEvent((e) => {
        if (e.payload.type === "drop" && e.payload.paths.length) void api.openPaths(e.payload.paths);
      })
      .then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  }, []);

  // Hidden in the tray: stop fetching; shown again: catch up once.
  // lifetime: OK — the visibility listener is removed on unmount
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void onWindowVisible((v) => {
      hiddenRef.current = !v;
      if (v) void fetchWindow();
    }).then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  }, [fetchWindow]);

  // The view's rules go to the back end; the filter text waits for a pause in typing.
  // lifetime: OK — the debounce timer is cleared when the rules change again or on unmount
  useEffect(() => {
    const t = window.setTimeout(async () => {
      try {
        setFilterErr(await api.setView({ tab, filter, levels, search }));
      } catch (e) {
        // error: LOGGED-OK — the previous rules stay in force
        console.warn("set_view failed", e);
      }
      void fetchWindow();
      void refreshState();
    }, 120);
    return () => window.clearTimeout(t);
  }, [tab, filter, levels, search, fetchWindow, refreshState]);

  // The list's height decides how many rows are fetched.
  // lifetime: OK — the observer disconnects on unmount
  useLayoutEffect(() => {
    const el = listRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      setViewH(el.clientHeight);
      void fetchWindow();
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [fetchWindow]);

  // After every redraw, before paint: following keeps the newest line in view;
  // paused keeps the anchored line exactly where it was, and the cursor on
  // its line.
  useLayoutEffect(() => {
    const el = listRef.current;
    if (!el) return;
    const moveTo = (top: number) => {
      if (Math.abs(el.scrollTop - top) >= 1) {
        autoScroll.current = true; // ours, not the user's: onScroll ignores it
        el.scrollTop = top;
      }
    };
    if (follow) {
      moveTo(el.scrollHeight - el.clientHeight);
    } else if (anchorRef.current) {
      const a = anchorRef.current;
      const r = rows.find((x) => x.n === a.n);
      if (r) moveTo(r.i * ROW_H + a.offset);
    }
    if (cursorNRef.current !== null) {
      const r = rows.find((x) => x.n === cursorNRef.current);
      if (r && r.i !== cursor) setCursor(r.i);
    }
  }, [follow, total, rows, cursor]);

  // Remember the line at the top of the window (paused view only).
  const captureAnchor = () => {
    const el = listRef.current;
    if (!el || followRef.current) {
      anchorRef.current = null;
      return;
    }
    const topIdx = Math.floor(el.scrollTop / ROW_H);
    const r = rows.find((x) => x.i === topIdx);
    anchorRef.current = r ? { n: r.n, offset: el.scrollTop - topIdx * ROW_H } : null;
  };

  // The user scrolling up stops following at once, before any fetch lands
  // and pulls the view back down.
  const stopFollowing = () => {
    if (followRef.current) {
      followRef.current = false;
      setFollow(false);
    }
  };

  const onScroll = () => {
    const el = listRef.current;
    if (!el) return;
    if (autoScroll.current) {
      autoScroll.current = false;
      return;
    }
    const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - ROW_H;
    if (follow && !atBottom) {
      followRef.current = false;
      setFollow(false);
    } else if (!follow && atBottom) {
      followRef.current = true;
      setFollow(true);
    }
    captureAnchor();
    void fetchWindow();
  };

  // ---- actions ----
  const scrollTo = useCallback(
    (index: number) => {
      const el = listRef.current;
      if (!el) return;
      const top = index * ROW_H;
      if (top < el.scrollTop || top + ROW_H > el.scrollTop + el.clientHeight) el.scrollTop = Math.max(0, top - el.clientHeight / 2);
      setFollow(false);
      void fetchWindow();
    },
    [fetchWindow],
  );

  const moveCursor = (to: number) => {
    if (totalRef.current === 0) return;
    const i = Math.max(0, Math.min(totalRef.current - 1, to));
    setCursor(i);
    cursorNRef.current = rows.find((x) => x.i === i)?.n ?? null;
    if (cursorNRef.current === null) {
      // Not on screen yet: learn its line number once it's fetched.
      void api.rows(i, 1).then((res) => {
        if (res.rows[0]) cursorNRef.current = res.rows[0].n;
      });
    }
    scrollTo(i);
  };

  const goLive = () => {
    setFollow(true);
    setCursor(null);
    cursorNRef.current = null;
    anchorRef.current = null;
    void fetchWindow();
  };

  // Export what the view shows (tab, filter, levels) to a CSV file.
  const exportView = async () => {
    const name = `oxitailr-${(tab ?? "all").replace(/[^\w.-]+/g, "_")}.csv`;
    let path: string | null;
    try {
      path = await save({ title: "Export the shown lines", defaultPath: name, filters: [{ name: "CSV", extensions: ["csv"] }] });
    } catch (e) {
      // error: SHOWN — in the status line
      setExported({ ok: false, text: `The save dialog failed: ${e}` });
      return;
    }
    if (!path) return; // cancelled
    try {
      const r = await api.exportView(path);
      const file = r.path.split(/[\\/]/).pop() ?? r.path;
      const capped = r.lines >= r.buffer_size ? ` · only the newest ${r.buffer_size.toLocaleString()} lines are kept in memory` : "";
      setExported({ ok: true, text: `Exported ${r.lines.toLocaleString()} line${r.lines === 1 ? "" : "s"} to ${file}${capped}` });
    } catch (e) {
      // error: SHOWN — in the status line
      setExported({ ok: false, text: String(e) });
    }
  };

  const openDialog = async () => {
    try {
      const picked = await open({ multiple: true, directory: false, title: "Open log files" });
      const paths = picked === null ? [] : Array.isArray(picked) ? picked : [picked];
      if (paths.length) await api.openPaths(paths);
    } catch (e) {
      // error: LOGGED-OK — the dialog was closed or failed; nothing opened
      console.warn("open dialog failed", e);
    }
  };

  const findNext = async (forward: boolean) => {
    if (!search) {
      searchRef.current?.focus();
      return;
    }
    try {
      const at = await api.find(search, cursor, forward);
      if (at !== null) moveCursor(at);
    } catch (e) {
      // error: LOGGED-OK — the search jump failed; the view stays where it is
      console.warn("find failed", e);
    }
  };

  const rowAt = async (i: number): Promise<Row | undefined> => rows.find((r) => r.i === i) ?? (await api.rows(i, 1)).rows[0];

  const toggleMark = async (r: Row) => {
    await api.toggleBookmark(r.src, r.n);
    void fetchWindow();
  };

  const jumpBookmark = async (forward: boolean) => {
    try {
      const at = await api.findBookmark(cursor, forward);
      if (at !== null) moveCursor(at);
    } catch (e) {
      // error: LOGGED-OK — the bookmark jump failed; the view stays where it is
      console.warn("bookmark jump failed", e);
    }
  };

  const copyRow = async (r: Row) => {
    try {
      await navigator.clipboard.writeText(rowText(r));
    } catch (e) {
      // error: LOGGED-OK — clipboard refused; nothing copied
      console.warn("copy failed", e);
    }
  };

  const toggleLevel = (k: number) => setLevels((ls) => ls.map((on, j) => (j === k ? !on : on)));

  const applyPreset = async (name: string | null) => {
    setPreset(name);
    await api.applyPreset(name);
    void fetchWindow();
    void refreshState();
  };

  // ---- the keys ----
  // lifetime: OK — the keydown listener is removed on unmount / when the handlers change
  useEffect(() => {
    if (!active) return;
    const onKey = async (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (isTyping(e.target)) {
        if (e.key === "Escape") (e.target as HTMLElement).blur();
        return;
      }
      const k = e.key;
      const handled = () => e.preventDefault();
      if (k === "/") return handled(), searchRef.current?.focus();
      if (k === "f") return handled(), filterRef.current?.focus();
      if (k >= "1" && k <= "6") return handled(), toggleLevel(Number(k) - 1);
      if (k === "G" || k === "End" || k === " ") return handled(), goLive();
      if (k === "g" || k === "Home") return handled(), moveCursor(0);
      if (k === "j" || k === "ArrowDown") return handled(), moveCursor((cursor ?? -1) + 1);
      if (k === "k" || k === "ArrowUp") return handled(), moveCursor((cursor ?? totalRef.current) - 1);
      if (k === "PageDown") return handled(), moveCursor((cursor ?? 0) + Math.floor(viewH / ROW_H));
      if (k === "PageUp") return handled(), moveCursor((cursor ?? 0) - Math.floor(viewH / ROW_H));
      if (k === "n") return handled(), void findNext(true);
      if (k === "N") return handled(), void findNext(false);
      if (k === "]") return handled(), void jumpBookmark(true);
      if (k === "[") return handled(), void jumpBookmark(false);
      if (k === "o" || k === "O") return handled(), void openDialog();
      if ((k === "b" || k === "y") && cursor !== null) {
        handled();
        const r = await rowAt(cursor);
        if (r) void (k === "b" ? toggleMark(r) : copyRow(r));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // ---- render ----
  const sources = snap?.sources ?? [];
  const showSource = tab === null && sources.length > 1;
  const alerts = snap?.alerts ?? [];

  return (
    <section className="logs" hidden={!active} aria-label="Logs">
      <div className="logs-toolbar" role="toolbar" aria-label="Log controls">
        <button type="button" className="dk-btn primary" onClick={() => void openDialog()} title="Open log files (o), or drop them on the window">
          <FolderOpen size={14} aria-hidden="true" /> Open
        </button>
        <button type="button" className="dk-btn" onClick={() => setSshOpen(true)} title="Tail a file on another machine over SSH">
          <Server size={14} aria-hidden="true" /> SSH
        </button>
        <div className="logs-levels" role="group" aria-label="Levels">
          {LEVELS.map((l: Level, k) => (
            <button
              key={l}
              type="button"
              className={`lvl-chip lvl-${l.toLowerCase()}${levels[k] ? " on" : ""}`}
              aria-pressed={levels[k]}
              title={`${levels[k] ? "Hide" : "Show"} ${l} lines (${k + 1})`}
              onClick={() => toggleLevel(k)}
            >
              {l}
            </button>
          ))}
        </div>
        <label className={`logs-field${filterErr ? " invalid" : ""}`} title={filterErr ? `Not a valid pattern, matching as text: ${filterErr}` : "Show only lines matching this (regex or text) — f"}>
          <span className="logs-field-label">Filter</span>
          <input ref={filterRef} value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="regex or text" spellCheck={false} />
          {filter && (
            <button type="button" className="logs-field-clear" aria-label="Clear filter" onClick={() => setFilter("")}>
              <X size={12} aria-hidden="true" />
            </button>
          )}
        </label>
        <label className="logs-field" title="Highlight matches; Enter or n / N to jump — /">
          <Search size={13} aria-hidden="true" />
          <input
            ref={searchRef}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void findNext(!e.shiftKey);
              }
            }}
            placeholder="search"
            spellCheck={false}
          />
        </label>
        {(snap?.presets.length ?? 0) > 0 && (
          <select className="logs-preset" value={preset ?? ""} onChange={(e) => void applyPreset(e.target.value || null)} title="Filter preset (from the config file)">
            <option value="">No preset</option>
            {snap?.presets.map((p) => (
              <option key={p} value={p}>
                {p}
              </option>
            ))}
          </select>
        )}
        <span className="logs-spacer" />
        {total > 0 && (
          <button type="button" className="dk-btn" onClick={() => void exportView()} title="Save the lines shown now (this tab, the filter and the levels) as a CSV file">
            <Download size={14} aria-hidden="true" /> Export
          </button>
        )}
        <button type="button" className={`dk-btn${follow ? " on" : ""}`} aria-pressed={follow} onClick={goLive} title="Follow live: jump to the newest line and stay there (G, End)">
          <ArrowDownToLine size={14} aria-hidden="true" /> Live
        </button>
        <button type="button" className="dk-btn icon" onClick={() => void jumpBookmark(true)} title="Next bookmark (] — [ for previous)" aria-label="Next bookmark">
          <Bookmark size={14} aria-hidden="true" />
        </button>
        <button type="button" className="dk-btn icon" onClick={() => void api.reload().then(fetchWindow)} title="Reload: re-read every file from the start" aria-label="Reload">
          <RotateCw size={14} aria-hidden="true" />
        </button>
        <button type="button" className="dk-btn icon" onClick={() => void api.clear().then(fetchWindow)} title="Clear the buffer (files keep tailing)" aria-label="Clear">
          <Eraser size={14} aria-hidden="true" />
        </button>
        <button
          type="button"
          className={`dk-btn icon${alerts.length ? " alerting" : ""}`}
          onClick={() => setAlertsOpen((o) => !o)}
          aria-expanded={alertsOpen}
          title={alerts.length ? `${alerts.length} recent alert(s)` : "No alerts (rules come from the config file)"}
          aria-label="Alerts"
        >
          <Bell size={14} aria-hidden="true" />
          {alerts.length > 0 && <span className="logs-count">{alerts.length}</span>}
        </button>
      </div>

      {sources.length > 0 && (
        <div className="logs-tabs" role="tablist" aria-label="Sources">
          {sources.length > 1 && (
            <button type="button" role="tab" aria-selected={tab === null} className={`logs-tab${tab === null ? " on" : ""}`} onClick={() => setTab(null)}>
              All
            </button>
          )}
          {sources.map((s) => (
            <div key={s.name} className={`logs-tab${tab === s.name ? " on" : ""}`}>
              <button type="button" role="tab" aria-selected={tab === s.name} className="logs-tab-name" title={s.path} onClick={() => setTab(s.name)}>
                <span className={`src-dot ${s.status.toLowerCase()}`} aria-hidden="true" />
                {s.name}
              </button>
              <button type="button" className="logs-tab-close" aria-label={`Close ${s.name}`} title={`Stop tailing ${s.name}`} onClick={() => {
                  if (tab === s.name) setTab(null);
                  void api.closeSource(s.name).then(() => {
                    void refreshState();
                    void fetchWindow();
                  });
                }}>
                <X size={12} aria-hidden="true" />
              </button>
            </div>
          ))}
        </div>
      )}

      {alertsOpen && (
        <div className="logs-alerts" role="region" aria-label="Recent alerts">
          <div className="logs-alerts-head">
            <b>Recent alerts</b>
            {alerts.length > 0 && (
              <button type="button" className="dk-btn" onClick={() => void api.dismissAlerts().then(refreshState)}>
                Dismiss all
              </button>
            )}
          </div>
          {alerts.length === 0 ? (
            <p className="logs-empty-note">None. Alert rules live in the config file.</p>
          ) : (
            <ul>
              {alerts
                .slice()
                .reverse()
                .map((a, k) => (
                  <li key={k}>
                    <b>{a.rule}</b> <span className="logs-alert-src">{a.source}</span> {a.text}
                  </li>
                ))}
            </ul>
          )}
        </div>
      )}

      <div
        className="logs-list"
        ref={listRef}
        onScroll={onScroll}
        onWheel={(e) => {
          if (e.deltaY < 0) stopFollowing();
        }}
        onPointerDown={(e) => {
          // On the scrollbar itself (the list, not a row).
          if (e.target === e.currentTarget) stopFollowing();
        }}
        tabIndex={-1}
      >
        {sources.length === 0 ? (
          <div className="logs-empty">
            <FolderOpen size={28} aria-hidden="true" />
            <p>Open a log file to start tailing.</p>
            <p className="logs-empty-note">Use Open, press o, or drop files on the window.</p>
          </div>
        ) : (
          <div className="logs-spacer-y" style={{ height: total * ROW_H }}>
            {rows.map((r) => (
              <div
                key={r.n}
                className={`log-row${r.i === cursor ? " cursor" : ""}${r.lvl ? ` lvl-${r.lvl.toLowerCase()}` : ""}`}
                style={{ top: r.i * ROW_H }}
                onClick={() => {
                  setCursor(r.i);
                  cursorNRef.current = r.n;
                }}
                onDoubleClick={() => void toggleMark(r)}
              >
                <button type="button" className={`log-mark${r.mark ? " on" : ""}`} aria-label={r.mark ? "Remove bookmark" : "Bookmark"} onClick={() => void toggleMark(r)}>
                  ★
                </button>
                <span className="log-n">{r.n}</span>
                {showSource && <span className="log-src">{r.src}</span>}
                {r.ts && <span className="log-ts">{r.ts}</span>}
                {r.lvl && <span className="log-lvl">{r.lvl}</span>}
                <span className="log-text">
                  {r.segs.map((s, k) =>
                    s.s ? (
                      <mark key={k}>{s.t}</mark>
                    ) : s.c ? (
                      // theme-exempt: highlight colors are defined by the user's config rules
                      <span key={k} style={{ color: `rgb(${s.c[0]} ${s.c[1]} ${s.c[2]})` }}>
                        {s.t}
                      </span>
                    ) : (
                      s.t
                    ),
                  )}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>

      <SshDialog open={sshOpen} onClose={() => setSshOpen(false)} />

      <footer className="logs-status" role="status">
        <span>
          {total.toLocaleString()} shown
          {` of ${lines.toLocaleString()}`}
        </span>
        <span>{sources.length} source{sources.length === 1 ? "" : "s"}</span>
        {bytes > 0 && <span>{formatBytes(bytes)}</span>}
        {filterErr && <span className="logs-status-warn">Filter matched as text</span>}
        {exported && (
          <span className={exported.ok ? "logs-exported" : "logs-status-warn"} title={exported.text}>
            {exported.text}
            <button type="button" className="logs-field-clear" aria-label="Dismiss" onClick={() => setExported(null)}>
              <X size={12} aria-hidden="true" />
            </button>
          </span>
        )}
        <span className="logs-spacer" />
        <span className={follow ? "logs-live on" : "logs-live"}>{follow ? "● Live" : "Paused · G to follow"}</span>
      </footer>
    </section>
  );
}
