# Handoff: Windows build of the desktop app

Written 2026-10-19. Updated 2026-10-22, after v0.4.0 shipped, and 2026-10-23 after the first
real Windows run.

## Goal

Ship the Tauri desktop app (`oxitailr-desktop`) for Windows alongside the existing release artifacts.

## Where things stand (2026-10-23)

- **Released in v0.4.0.** `release.yml` has a `build-desktop-windows` job (`windows-latest`,
  `npx tauri build --bundles nsis`). It publishes `Oxitailr-Desktop-<ver>-x86_64-setup.exe` next to
  the terminal app's three files.
- **Merged:** #17 (the `oxitailr-core` split), #18 (the desktop app and secret store) and #19 (the 0.4.0 bump).
- **CI:** `ci.yml` now runs on PRs into any branch. Its `desktop-windows` job builds the same NSIS
  installer and uploads it as the `oxitailr-desktop-windows-setup` artifact.
- **Runs on Windows.** The v0.4.1 installer was installed on a real Windows Server on 2026-10-23,
  and the app tails a busy production log there: level colors, the rail and live follow all work.
  That run found one bug, now fixed: timestamps lost their milliseconds (`10:16:25,423` showed as
  `.000`; PR #23). Still unchecked: the tray, saved SSH passwords and uninstall (step 5 below).

## Things already known

- **Workspace setup.** In the root `Cargo.toml`, `default-members` excludes `desktop/src-tauri`, so a plain `cargo build` (and therefore the current `build-windows` release job) does **not** build the desktop app. Build it with `cargo build -p oxitailr-desktop` or `npm run tauri build` in `desktop/`.
- **The current `build-windows` job cross-compiles** from `ubuntu-latest` with `x86_64-pc-windows-gnu` (MinGW). That approach is not a good fit for Tauri: WebView2 and the NSIS/MSI bundlers expect a Windows host. Plan on a separate `windows-latest` job using the MSVC target and `npm run tauri build`, or `tauri-apps/tauri-action`. *Resolved: the separate `build-desktop-windows` job.*
- **Bundler config.** `tauri.conf.json` has `bundle.targets: "all"`, so on Windows it produces both NSIS (`.exe` installer) and MSI. Decide whether to ship both. `icons/icon.ico` already exists. *Resolved: NSIS only (`--bundles nsis`).*
- **Secret store on Windows.** `oxitailr-core/src/secrets.rs` uses `keyring-core` with `windows-native-keyring-store` (Credential Manager). It is untested on Windows.
- **The tray backend is Linux-only.** The `tray-icon` + `ksni` dependency in `desktop/src-tauri/Cargo.toml` is behind `cfg(target_os = "linux")`, so Windows uses Tauri's default tray. Check that a single click still shows and hides the window there.
- **Artifact naming.** Per CLAUDE.md, the existing names are `oxitailr-<ver>-x86_64[.exe]` and `Oxitailr-<ver>-x86_64.AppImage`. Choose a name for the desktop installer that doesn't collide with the terminal `.exe` (for example, `Oxitailr-Desktop-<ver>-x86_64-setup.exe`).

## Suggested order

1. ~~Make CI run on #18.~~ Done: `ci.yml` triggers on PRs into any branch.
2. ~~Merge #17, then #18.~~ Done 2026-10-22.
3. ~~Add a `build-desktop-windows` job to `release.yml`.~~ Done: NSIS only. MSI was dropped because it refuses pre-release versions.
4. Optional: add a Linux desktop bundle (`.deb`/AppImage) job. **Open.**
5. Smoke-test on a real Windows machine. **In progress:**
   - ~~start-up, opening and tailing a file~~ (2026-10-23, Windows Server);
   - an SSH source with a saved secret (Credential Manager);
   - the tray: does a single click show and hide the window?
   - uninstall.
6. ~~Bump the version and tag a release.~~ Done: v0.4.0.
