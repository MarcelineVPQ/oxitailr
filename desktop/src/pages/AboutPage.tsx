// About: name, version (from the engine, so it is the build you're running),
// what it is, where it lives, and the keyboard shortcuts.
import { useEffect, useState } from "react";
import { api } from "../api";

const REPO = "https://github.com/MarcelineVPQ/oxitailr";

const SHORTCUTS: ReadonlyArray<[keys: string, what: string]> = [
  ["/", "Search"],
  ["f", "Filter"],
  ["1 – 6", "Show or hide a level (Trace … Fatal)"],
  ["G or End", "Follow live"],
  ["n / N", "Next / previous match"],
  ["b", "Bookmark the line"],
  ["] / [", "Next / previous bookmark"],
  ["y", "Copy the line"],
  ["o", "Open a file"],
  ["Ctrl+B", "Keep the rail open / collapse it"],
];

export default function AboutPage() {
  const [version, setVersion] = useState<string | null>(null);
  // lifetime: OK — one call; `alive` drops a late answer after unmount
  useEffect(() => {
    let alive = true;
    api
      .state()
      .then((s) => {
        if (alive) setVersion(s.version);
      })
      // error: NOT-WORTH — no version line; the rest of the page stands
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  return (
    <section className="page about" aria-labelledby="about-title">
      <header className="page-head">
        <h1 id="about-title">Oxitailr</h1>
        {version && <p className="page-sub about-version">Version {version}</p>}
      </header>
      <p className="about-lede">The oxitailr engine in a window: fast, low-resource log tailing.</p>
      <p>
        <a href={REPO} target="_blank" rel="noreferrer">
          github.com/MarcelineVPQ/oxitailr
        </a>
      </p>

      <div className="page-section">
        <h2>Keyboard shortcuts</h2>
        <table className="about-keys">
          <tbody>
            {/* loop: HOT-PATH-OK — N = 10 fixed rows */}
            {SHORTCUTS.map(([keys, what]) => (
              <tr key={keys}>
                <td>
                  <kbd>{keys}</kbd>
                </td>
                <td>{what}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
