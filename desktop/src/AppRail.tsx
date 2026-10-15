// The app's navigation rail, on the left edge (carried over from the DS5
// configurator, mirrored): icons only by default, expanding over the page on
// hover or keyboard focus (closing right after a page is picked with the
// pointer); pinned open it takes its full width. Ctrl+B pins/unpins. The
// bottom shows what's being tailed (a colored dot plus a symbol, so it never
// relies on color alone), the pin, and in the bottom corner the lock: locked,
// the rail stays collapsed (icons only; hover and keyboard focus no longer
// expand it). Pin and lock exclude each other (keeping it open unlocks it;
// locking it unpins it). Both remembered.
// Idle cost: none (CSS states; no timers or listeners added beyond Ctrl+B).
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Info, Lock, LockOpen, PanelLeftClose, PanelLeftOpen, ScrollText, Settings } from "lucide-react";
import { useFlagPref } from "./storage";

export type RailItem = "logs" | "settings" | "about";
export const RAIL_ITEMS: readonly RailItem[] = ["logs", "settings", "about"];
const PIN_KEY = "oxi.railPinned";
export const RAIL_LOCK_KEY = "oxi.railLocked";

const LABEL: Record<RailItem, string> = {
  logs: "Logs",
  settings: "Settings",
  about: "About",
};

const TIP: Record<RailItem, string> = {
  logs: "The lines from every open file",
  settings: "Theme, and where the rest is configured",
  about: "Version and keyboard shortcuts",
};

interface Props {
  active: RailItem;
  onSelect: (item: RailItem) => void;
  /** Open sources. */
  sources: number;
  /** Alerts waiting (shown as a badge on Logs). */
  alerts: number;
  /** At least one source is connected. */
  tailing: boolean;
}

export default function AppRail({ active, onSelect, sources, alerts, tailing }: Props) {
  const [pinned, setPinned] = useFlagPref(PIN_KEY);
  const [locked, setLocked] = useFlagPref(RAIL_LOCK_KEY);
  // Kept open and locked collapsed exclude each other.
  const pin = useCallback(
    (on: boolean) => {
      setPinned(on);
      if (on) setLocked(false);
    },
    [setPinned, setLocked],
  );
  const lock = (on: boolean) => {
    setLocked(on);
    if (on) setPinned(false);
  };
  // Picking a page closes the rail right away, like a menu: hover doesn't
  // reopen it until the pointer leaves the rail and comes back. Keyboard focus
  // only opens it when it's visible focus (Tab), not after a click.
  const [picked, setPicked] = useState(false);
  // lifetime: OK — cleanup: removes the Ctrl+B keydown listener (unmount / pin change)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "b") {
        e.preventDefault();
        pin(!pinned);
      }
    };
    // lifetime: OK — removed in the effect cleanup
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [pinned, pin]);

  const link = (item: RailItem, icon: ReactNode, badge?: number) => {
    const label = LABEL[item];
    const badgeTip = badge ? `${badge} alert${badge === 1 ? "" : "s"}` : undefined;
    const tip = `${label} · ${TIP[item]}`;
    return (
      <button
        type="button"
        className={`rail-link${active === item ? " active" : ""}`}
        data-rail={item}
        aria-current={active === item ? "page" : undefined}
        title={badgeTip ? `${tip}\n${badgeTip}` : tip}
        aria-description={badgeTip}
        onClick={(e) => {
          onSelect(item);
          // A pointer click (detail > 0), not Enter/Space: close the rail now.
          if (e.detail > 0) setPicked(true);
        }}
      >
        <span className="rail-ic" aria-hidden="true">
          {icon}
          {badge ? <span className="rail-badge">{badge > 99 ? "99+" : badge}</span> : null}
        </span>
        <span className="rail-label">{label}</span>
      </button>
    );
  };

  const status = tailing ? `Tailing ${sources} file${sources === 1 ? "" : "s"}` : "No files open";
  const pinLabel = pinned ? "Collapse" : "Keep open";
  const lockLabel = locked ? "Unlock the rail" : "Lock the rail collapsed";

  return (
    <div className={`rail-slot${pinned ? " pinned" : ""}`}>
      <nav className={`app-rail${picked ? " picked" : ""}${locked ? " locked" : ""}`} aria-label="Pages" onPointerLeave={() => setPicked(false)}>
        <div className="rail-nav">
          {link("logs", <ScrollText size={20} />, alerts)}
          <div className="rail-divider" />
          {link("settings", <Settings size={20} />)}
          {link("about", <Info size={20} />)}
        </div>
        <div className="rail-footer">
          <div className={`rail-status ${tailing ? "ok" : "warn"}`} title={status} role="status">
            <span className="rail-ic" aria-hidden="true">
              <span className="rail-dot">{tailing ? "✓" : "…"}</span>
            </span>
            <span className="rail-label rail-status-text">
              <span>{status}</span>
            </span>
          </div>
          <button type="button" className="rail-link rail-pin" title={`${pinLabel} (Ctrl+B)`} aria-pressed={pinned} onClick={() => pin(!pinned)}>
            <span className="rail-ic" aria-hidden="true">
              {pinned ? <PanelLeftClose size={20} /> : <PanelLeftOpen size={20} />}
            </span>
            <span className="rail-label">{pinLabel}</span>
          </button>
          <button type="button" className="rail-lock" title={lockLabel} aria-label={lockLabel} aria-pressed={locked} onClick={() => lock(!locked)}>
            {locked ? <Lock size={13} aria-hidden="true" /> : <LockOpen size={13} aria-hidden="true" />}
          </button>
        </div>
      </nav>
    </div>
  );
}
