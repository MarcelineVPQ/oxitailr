// How much of the user's attention the window has, told to the back end so
// it can pace its updates (ported from the DS5 configurator's useWindowShown):
//   focused  — the user is working in it: full rate
//   visible  — on screen, another app in front (a tailer on a second monitor
//              still gets glanced at): a slower rate
//   hidden   — minimized, on another virtual desktop, or in the tray: nothing
//              is sent; tailing and alerts carry on in the back end.
// Wayland doesn't say when another window covers this one, so "covered"
// counts as visible. While hidden, :root[data-window-hidden] pauses every
// CSS animation (shell.css).
import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, onWindowVisible } from "./api";

type Level = "focused" | "visible" | "hidden";

export function useAttention(): void {
  // lifetime: OK — cleanup removes the DOM listeners and both Tauri listeners (unmount)
  useEffect(() => {
    let shown = true; // the tray hasn't hidden it
    let minimized = false;
    let last: Level | null = null;
    const send = () => {
      const level: Level = !shown || minimized || document.hidden ? "hidden" : document.hasFocus() ? "focused" : "visible";
      if (level === last) return;
      last = level;
      document.documentElement.toggleAttribute("data-window-hidden", level === "hidden");
      // error: LOGGED-OK — the back end keeps its last pace; the next change retries
      api.setAttention(level).catch((e) => console.warn("set_attention failed", e));
    };
    const win = getCurrentWindow();
    const offs: Array<() => void> = [];
    let gone = false;
    const keep = (p: Promise<() => void>) => void p.then((u) => (gone ? u() : offs.push(u)));
    keep(
      onWindowVisible((v) => {
        shown = v;
        send();
      }),
    );
    // Minimizing doesn't always change document.visibilityState; ask the window.
    keep(
      win.onResized(async () => {
        try {
          minimized = await win.isMinimized();
        } catch {
          // error: NOT-WORTH — unknown counts as not minimized
          minimized = false;
        }
        send();
      }),
    );
    window.addEventListener("focus", send);
    window.addEventListener("blur", send);
    document.addEventListener("visibilitychange", send);
    send();
    return () => {
      gone = true;
      offs.forEach((u) => u());
      window.removeEventListener("focus", send);
      window.removeEventListener("blur", send);
      document.removeEventListener("visibilitychange", send);
    };
  }, []);
}
