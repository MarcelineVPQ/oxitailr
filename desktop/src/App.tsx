// The window: the navigation rail on the left, the page beside it. The Logs
// page stays mounted the whole time (hidden when another page is shown) so its
// scroll position, filters and search survive a trip to Settings and back.
// The rail's status comes from the Logs page, which already fetches the
// source list when it changes, so nothing here polls.
import { useCallback, useState } from "react";
import AppRail, { RAIL_ITEMS, type RailItem } from "./AppRail";
import { usePref } from "./storage";
import LogsPage, { type RailStatus } from "./pages/LogsPage";
import SettingsPage from "./pages/SettingsPage";
import AboutPage from "./pages/AboutPage";
import { useAttention } from "./attention";

const VIEW_KEY = "oxi.view";

export default function App() {
  const [view, setView] = usePref<RailItem>(VIEW_KEY, "logs", RAIL_ITEMS);
  useAttention();
  const [status, setStatus] = useState<RailStatus>({ sources: 0, alerts: 0, tailing: false });
  // Only re-render the rail when its status actually changes.
  const onStatus = useCallback(
    (s: RailStatus) => setStatus((old) => (old.sources === s.sources && old.alerts === s.alerts && old.tailing === s.tailing ? old : s)),
    [],
  );

  return (
    <div className="dk dk-one">
      <AppRail active={view} onSelect={setView} sources={status.sources} alerts={status.alerts} tailing={status.tailing} />
      <main className="dk-main">
        <LogsPage active={view === "logs"} onStatus={onStatus} />
        {view === "settings" && <SettingsPage />}
        {view === "about" && <AboutPage />}
      </main>
    </div>
  );
}
