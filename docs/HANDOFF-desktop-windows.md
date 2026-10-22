# Handoff: Windows build of the desktop app

Written 2026-10-19. Resume on or after Thu 2026-10-22, once the weekly usage limits have reset.

## Goal

Ship the Tauri desktop app (`oxitailr-desktop`) for Windows alongside the existing release artifacts.

## Where things stand

- **Latest release:** v0.3.4 (2026-06-04). Its assets are the Linux binary, the AppImage and `oxitailr-0.3.4-x86_64.exe`. That `.exe` is the **terminal app only**; the desktop app has never been released.
- **The desktop app is still in two unmerged PRs, stacked:**
  - #17 `feat/workspace-core-split` → `main`. This splits the engine into the `oxitailr-core` library. CI passed (2026-10-15).
  - #18 `feat/desktop-app` → `feat/workspace-core-split`. This is the Tauri 2 desktop app plus the secure secret store. **No CI has run on it**, because `ci.yml` only triggers on PRs into `main`.
- **Builds so far:** the desktop app has only been built locally on the Linux laptop (`target/release/oxitailr-desktop`, 2026-10-15). It has never been built for Windows, locally or in CI.

## Things already known

- **Workspace setup.** In the root `Cargo.toml`, `default-members` excludes `desktop/src-tauri`, so a plain `cargo build` (and therefore the current `build-windows` release job) does **not** build the desktop app. Build it with `cargo build -p oxitailr-desktop` or `npm run tauri build` in `desktop/`.
- **The current `build-windows` job cross-compiles** from `ubuntu-latest` with `x86_64-pc-windows-gnu` (MinGW). That approach is not a good fit for Tauri: WebView2 and the NSIS/MSI bundlers expect a Windows host. Plan on a separate `windows-latest` job using the MSVC target and `npm run tauri build`, or `tauri-apps/tauri-action`.
- **Bundler config.** `tauri.conf.json` has `bundle.targets: "all"`, so on Windows it produces both NSIS (`.exe` installer) and MSI. Decide whether to ship both. `icons/icon.ico` already exists.
- **Secret store on Windows.** `oxitailr-core/src/secrets.rs` uses `keyring-core` with `windows-native-keyring-store` (Credential Manager). It is untested on Windows.
- **The tray backend is Linux-only.** The `tray-icon` + `ksni` dependency in `desktop/src-tauri/Cargo.toml` is behind `cfg(target_os = "linux")`, so Windows uses Tauri's default tray. Check that a single click still shows and hides the window there.
- **Artifact naming.** Per CLAUDE.md, the existing names are `oxitailr-<ver>-x86_64[.exe]` and `Oxitailr-<ver>-x86_64.AppImage`. Choose a name for the desktop installer that doesn't collide with the terminal `.exe` (for example, `Oxitailr-Desktop-<ver>-x86_64-setup.exe`).

## Suggested order

1. Make CI run on #18 by retargeting it to `main` after #17 merges, or by widening `ci.yml`'s `pull_request.branches`. The branch's `ci.yml` already has a `desktop` job (Linux) and a `desktop-windows` job (`windows-latest`, `npx tauri build --no-bundle`), so the first CI run is the first real Windows compile. Expect to fix whatever it surfaces.
2. Merge #17, then #18.
3. Add a `build-desktop-windows` job (`runs-on: windows-latest`) to `release.yml`, upload the installer(s), and add the job to `release.needs`.
4. Optional: add a Linux desktop bundle (`.deb`/AppImage) job at the same time.
5. Smoke-test on a real Windows machine: start-up, opening/tailing a file, an SSH source with a saved secret, the tray, and uninstall.
6. Bump the workspace version and tag a release (the v0.4.0 desktop launch?).
