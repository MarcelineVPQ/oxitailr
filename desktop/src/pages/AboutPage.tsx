// About: the open text in the main view (the guide, the changes, the keyboard
// shortcuts, what's coming, or the credits) and its sections in the panel, as
// in the DS5 configurator. The open section lists its text's chapters; a click
// scrolls the main view there. Nothing here polls: the texts are bundled.
import { useEffect, useMemo, useState } from "react";
import { BookOpen, Bug, Code, Flag, FolderGit2, Globe, Heart, Keyboard, ScrollText, type LucideIcon } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import readme from "../../../README.md?raw";
import changelog from "../../../CHANGELOG.md?raw";
import todo from "../../../TODO.md?raw";
import Markdown, { markdownHeadings, plainHeading, scrollToHeading } from "../Markdown";
import diceIcon from "../assets/42ps-dice.png";
import { api } from "../api";
import { Section, SplitView } from "../accordion/Section";
import { pinScope, useAccordion } from "../accordion/useAccordion";
import type { SectionId } from "../accordion/sections";

type AboutTab = SectionId<"about">;

const REPO = "https://github.com/MarcelineVPQ/oxitailr";

// The log view's keys (pages/LogsPage.tsx) and the rail's (AppRail.tsx).
const KEYS = `
## Moving

| Key | Action |
|---|---|
| \`j\` / \`k\`, \`↓\` / \`↑\` | Move the cursor one line |
| \`PgDn\` / \`PgUp\` | Page down / up |
| \`g\` / \`Home\` | Jump to the top |
| \`G\` / \`End\` / \`Space\` | Jump to the newest line (follow live) |

## Finding

| Key | Action |
|---|---|
| \`/\`, then \`Enter\` | Search, and jump to the next match |
| \`n\` / \`N\` | Next / previous match |
| \`f\` | Filter (regex or text) |
| \`1\` – \`6\` | Show or hide a level (Trace … Fatal) |

## Lines and files

| Key | Action |
|---|---|
| \`b\` | Bookmark the cursor line |
| \`]\` / \`[\` | Next / previous bookmark |
| \`y\` | Copy the cursor line |
| \`o\` | Open a file |

## The window

| Key | Action |
|---|---|
| \`Ctrl+B\` | Keep the navigation rail open / collapse it |
`;

const CREDITS = `
### Built on

The window's shell (the navigation rail, the settings panel and its sections, About) comes from the [DS5 Dongle OLED Edition Configurator](https://github.com/MarcelineVPQ/DS5Dongle-OLED-Config-Web) (MIT), also a 42 Pixel Studios project.

### Third-party code and assets

| What | Credit |
|---|---|
| App framework | [Tauri](https://tauri.app/) (MIT or Apache-2.0) |
| Interface | [React](https://react.dev/) (MIT), [react-markdown](https://github.com/remarkjs/react-markdown) and remark-gfm (MIT) |
| Async runtime | [tokio](https://tokio.rs/) (MIT) |
| SSH | [russh](https://github.com/warp-tech/russh) (Apache-2.0) |
| Password vaults | [keyring-core](https://crates.io/crates/keyring-core) (MIT or Apache-2.0) |
| Icons | [Lucide](https://lucide.dev/) (ISC) |
| Fonts | DM Sans, Space Grotesk, JetBrains Mono (SIL Open Font License) |
`;

const STUDIO_LINKS = [
  { href: "https://42pixelstudios.com/", icon: Globe, label: "42pixelstudios.com" },
  { href: "https://github.com/MarcelineVPQ", icon: FolderGit2, label: "github.com/MarcelineVPQ" },
  { href: REPO, icon: Code, label: "Source code (MIT)" },
  { href: `${REPO}/issues`, icon: Bug, label: "Report an issue" },
] as const;

function StudioCard({ version }: { version: string | null }) {
  return (
    <section className="dk-card dk-studio">
      <img className="dk-studio-logo" src={diceIcon} alt="" width={112} height={82} />
      <div className="dk-studio-text">
        <span className="dk-studio-eyebrow">A 42 Pixel Studios production</span>
        <h2 className="dk-studio-title">Oxitailr</h2>
        <p className="dk-studio-by">
          Developed by <strong>Marcy Kuhn</strong>
          {version && <span className="dk-studio-version"> · v{version}</span>}
        </p>
        <div className="dk-studio-links">
          {/* loop: HOT-PATH-OK — N = 4 studio links; About page render */}
          {STUDIO_LINKS.map((l) => (
            <a
              key={l.href}
              className="dk-btn"
              href={l.href}
              onClick={(e) => {
                e.preventDefault();
                // error: LOGGED-OK — the link didn't open; nothing else depends on it
                openUrl(l.href).catch((err) => console.warn("open link failed", err));
              }}
            >
              <l.icon size={14} aria-hidden="true" />
              {l.label}
            </a>
          ))}
        </div>
        <small className="dk-studio-copy">© {new Date().getFullYear()} 42 Pixel Studios — a Marcy Kuhn production.</small>
      </div>
    </section>
  );
}

// Each section: its icon, title and text.
const TEXTS: Record<AboutTab, { icon: LucideIcon; title: string; text: string }> = {
  guide: { icon: BookOpen, title: "Guide", text: readme },
  changes: { icon: ScrollText, title: "Changes", text: changelog },
  keys: { icon: Keyboard, title: "Keyboard shortcuts", text: KEYS },
  coming: { icon: Flag, title: "Coming soon", text: todo },
  credits: { icon: Heart, title: "Credits", text: CREDITS },
};
const IDS = Object.keys(TEXTS) as AboutTab[];

// The newest version a changelog lists ("0.4.1" from "[0.4.1] - 2026-10-22").
function newest(headings: string[]): string | null {
  // loop: NOT-WORTH — stops at the first released heading
  const released = headings.find((h) => !/^unreleased$/i.test(h));
  return released ? released.split(" ")[0] : null;
}

export default function AboutPage() {
  const [version, setVersion] = useState<string | null>(null);
  // The running build's version, from the engine.
  // lifetime: OK — one call; `alive` drops a late answer after unmount
  useEffect(() => {
    let alive = true;
    api
      .state()
      .then((s) => alive && setVersion(s.version))
      // error: NOT-WORTH — no version on the studio card; the rest of the page stands
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  const acc = useAccordion("about", pinScope("about"));
  // cache: OK — key=the bundled texts (fixed per build); invalidated=never (bundled); bound=5 lists
  const headings = useMemo(
    // loop: NOT-WORTH — N = 5 texts, once per mount
    () => Object.fromEntries(IDS.map((id) => [id, markdownHeadings(TEXTS[id].text)])) as Record<AboutTab, string[]>,
    [],
  );
  // The main view shows the open section's text (an unpinned one first), and
  // keeps the last one shown while all are folded.
  // loop: NOT-WORTH — N = 5 sections; About's render
  const openNow = IDS.find((id) => acc.isOpen(id) && !acc.isPinned(id)) ?? IDS.find((id) => acc.isOpen(id));
  const [doc, setDoc] = useState<AboutTab>(openNow ?? "guide");
  const [at, setAt] = useState<string | null>(null); // the chapter last clicked
  // A section opened: its text shows (set while rendering, React's pattern for state that follows other state).
  const [lastOpen, setLastOpen] = useState(openNow);
  if (openNow !== lastOpen) {
    setLastOpen(openNow);
    if (openNow) {
      setDoc(openNow);
      setAt(null);
    }
  }

  const chapters = (n: number) => `${n} chapter${n === 1 ? "" : "s"}`;
  const statuses: Record<AboutTab, string | null> = {
    guide: chapters(headings.guide.length),
    changes: newest(headings.changes),
    keys: headings.keys.join(" · "),
    coming: headings.coming.join(" · "),
    credits: headings.credits.join(" · "),
  };
  const go = (id: AboutTab, chapter: string) => {
    setDoc(id);
    setAt(chapter);
    // After the text for that section is shown.
    // lifetime: OK — fires once (scrolls after the next paint)
    requestAnimationFrame(() => scrollToHeading(document.querySelector(".dk-about-text"), (h) => plainHeading(h).startsWith(chapter)));
  };

  const main = (
    <article className="dk-about-text">
      {doc === "credits" && <StudioCard version={version} />}
      <div className="dk-card dk-doc">
        <Markdown text={TEXTS[doc].text} />
      </div>
    </article>
  );

  return (
    <SplitView page="about" main={main} mainKey={doc} title="About" sub="Guide, changes and credits" state={acc}>
      {/* loop: NOT-WORTH — N = 5 sections; About's render */}
      {IDS.map((id) => (
        <Section key={id} id={id} icon={TEXTS[id].icon} title={TEXTS[id].title} status={statuses[id]}>
          {() => (
            <nav className="dk-chapters" aria-label={TEXTS[id].title}>
              {/* loop: NOT-WORTH — N = one text's chapters (≤ ~60); only while its section is open */}
              {headings[id].map((c) => (
                <button type="button" key={c} className={`dk-chapter${doc === id && at === c ? " on" : ""}`} aria-current={doc === id && at === c ? "location" : undefined} onClick={() => go(id, c)}>
                  {c}
                </button>
              ))}
            </nav>
          )}
        </Section>
      ))}
    </SplitView>
  );
}
