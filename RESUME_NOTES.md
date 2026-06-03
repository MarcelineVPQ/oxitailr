# Oxitailr — performance & correctness work (resume notes)

Goal: make it actually behave like a live tail logger and be much more performant, then
tidy the architecture toward standard egui-app layout.

## Progress
- **Task #1 (live updates): DONE + verified.** Added a 250ms polling fallback to the tail loop
  in `src/source/local.rs` (notify is now just an optimization), shared read helpers, and
  partial-line handling. Two `#[tokio::test]` regression tests in `local.rs`.
- **Task #2 (render perf): DONE + verified.** Cache the filtered view (rebuild only when the
  buffer/filter/level/source actually change, keyed on `LogState.version` +
  `FilterEngine.generation()`); `std::mem::take` to avoid borrow conflicts. Wrap mode now uses
  `show_viewport` variable-height virtualization with per-row measured heights
  (`wrap_row_heights`). Visually verified wrapped + short lines render without overlap.
- **Build env:** switched `reqwest` to `rustls-tls` (was native-tls/OpenSSL) so the build needs
  no system OpenSSL/pkg-config. Rust toolchain installed at `~/.cargo` — source
  `. "$HOME/.cargo/env"` before cargo commands.
- Verify gates used: `cargo clippy -- -D warnings` (CI gate, binary-only) and
  `cargo test --bin oxitailr`. NOTE: `cargo clippy --all-targets` flags 6 pre-existing
  `useless_vec` lints in `tests/integration_tests.rs` (not from our work, doesn't gate CI).

- **Task #3 (UI-thread blocking): DONE + verified.** Replaced the `Arc<Mutex<SourceManager>>`
  + `runtime.block_on(...)` pattern with a `SourceCommand` channel (`source/mod.rs`): the
  manager is moved onto the runtime via `SourceManager::run(rx)` (`source/manager.rs`) and the
  UI sends non-blocking `UnboundedSender` commands (`add/remove/reload/stopall`). Removed the
  `block_in_place`+`block_on` from `info()` in `local.rs`/`ssh.rs` (now `blocking_lock`; method
  is currently unused). Runtime-verified: a CLI file arg loads + tails live through the channel.

- **Task #4 (de-monolith, focused cut): DONE + verified.** Extracted the central log view
  (`render_central_log_view`) into `src/ui/panels/log_view.rs` and the source side panel
  (`render_source_panel`) into `src/ui/panels/source_panel.rs`, both as `impl crate::TailLoggerApp`
  blocks (descendant modules can reach the root-private fields/methods; the moved methods are
  `pub(crate)`). `update()` is now slim. `main.rs` ~2760 → ~1861 lines. UI verified via screenshot.
  Remaining `main.rs` size is mostly the app struct + constructor + dialogs/toolbar + helpers;
  a future pass could extract the toolbar/status-bar and dialogs similarly.

## Status: all four tasks complete.

---
## Original diagnosis (for reference)

## Environment note
- No Rust toolchain is installed on this machine. Install before building:
  `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh` then restart the shell.
- Recommended VSCodium extension: **rust-analyzer**.
- Baseline build command: `cargo build` (release: `cargo build --release`).

## Root-cause diagnosis

### 1. Doesn't live-update (the core bug)
`src/source/local.rs` → `run_local_tail()`:
- Reads existing file content once (lines ~173–206).
- Then relies **entirely** on the `notify` filesystem watcher for new lines (lines ~213–328).
- **No polling fallback.** When notify misses/coalesces append events (common on Linux
  inotify depending on flush behavior, network FS, editor rename-on-save), new lines are
  never read — the `select!` loop just sleeps.
- The **Reload** button (`reload_sources`, main.rs:788) stops+restarts the source, which
  re-reads the whole file from scratch — which is exactly why reload "fixes" it.

Fix: add a periodic poll (~250ms) that reads from `last_position` to EOF regardless of
notify, handling truncation/rotation in that path too. Keep notify as an optimization.

### 2. Performance (all in the render path, runs ~10x/sec)
main.rs:
- **No virtualization in wrap mode** (`scroll_area.show` + `for line in filtered_lines.iter()`
  at ~line 2223) — builds widgets for ALL up to 10,000 lines every frame. The non-wrap
  branch already uses `show_rows` (virtualized) at ~line 2420; wrap mode needs the same.
- **Re-filters + clones the entire buffer every frame** (lines ~1997–2012): locks log state,
  filters all entries, `.cloned()` every match into a fresh Vec, every frame even when idle.
  Cache it; recompute only when buffer/filter/search/selected_source changes.
- `highlight_rules.clone()` every frame (line ~1995) and per-line String clones in the loop.
- `request_repaint_after(100ms)` (line 1493) redraws 10x/sec even when idle; prefer
  event-driven repaint with a slower idle fallback.

### 3. Architecture / "hacked together"
- **main.rs is 2759 lines** — holds the app object, all state, event loop, AND all rendering.
  `src/ui/panels/` exists but is barely used (`log_view.rs` is 58 lines).
- **UI thread blocks on async**: `reload_sources` and add/remove-source helpers call
  `runtime.block_on(...)` directly on the egui thread (main.rs ~700, 711, 749, 769, 801) —
  freezes the window. Standard fix: UI sends commands over a channel to an async task that
  owns the `SourceManager`.
- `info()` uses `block_in_place` + `block_on` (local.rs:73) — anti-pattern.
- What's fine: real crates (tokio, eframe, notify), log-rotation detection, and clean
  source/parser/filter/alert module separation. Bones are salvageable.

## Plan (agreed: do all three, in this order)
1. **Live updates** — polling fallback in `run_local_tail` (Task #1). Highest priority.
2. **Render perf** — virtualize wrap mode + cache filtered list (Task #2).
3. **Non-blocking source ops** — command channel; remove block_on from UI thread; fix
   `info()` (Task #3).
4. **De-monolith** — move rendering into `src/ui/panels/*`, keep behavior identical (Task #4).

## How to verify the live-update fix
Build, run on a file, and in another terminal append to it:
`while true; do echo "test $(date)" >> /tmp/oxitailr-test.log; sleep 1; done`
New lines should appear within ~250ms WITHOUT clicking Reload.

## Key file:line references
- Tail loop: `src/source/local.rs:136` (`run_local_tail`), watcher setup ~213, tail select ~239.
- Event drain: `src/main.rs:971` (`process_events`).
- Update loop: `src/main.rs:1473` (`impl eframe::App::update`), repaint at 1493.
- Filtered-lines rebuild: `src/main.rs:1997`.
- Wrap render (no virtualization): `src/main.rs:2223`. Non-wrap (virtualized): `src/main.rs:2420`.
- Reload: `src/main.rs:788`. Blocking source ops: ~700/711/749/769/801.
- Runtime build: `src/main.rs:2689` (multi-thread tokio).
- LogState buffer: `src/main.rs:140`, default buffer_size 10000 (`src/config/settings.rs:103`).
