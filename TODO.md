# Oxitailr Feature Implementation TODO

## Current Sprint: Features 1-5 - ALL COMPLETED

### Feature 1: Search Navigation (Small) - COMPLETED
- [x] Add search_matches: Vec<usize> and current_match: Option<usize> to TailLoggerApp
- [x] Add navigation UI with match count and buttons (▲/▼)
- [x] Implement F3/Shift+F3 keyboard shortcuts
- [x] Build search matches when search text changes
- [x] Scroll to match on navigation

### Feature 2: Copy/Export Log Lines (Small) - COMPLETED
- [x] Add context menu to wrap_lines mode
- [x] Add context menu to non-wrap mode
- [x] Implement Copy Line, Copy with Timestamp, Copy Raw

### Feature 3: Log Bookmarks (Small) - COMPLETED
- [x] Add bookmarks: HashSet<usize> to TailLoggerApp
- [x] Add bookmark toggle icon (★/☆) in line rendering
- [x] Create bookmarks dropdown for jumping between bookmarks
- [x] Persist bookmarks in session.json

### Feature 4: Glob Pattern Support (Medium) - COMPLETED
- [x] Add glob crate to Cargo.toml
- [x] Detect glob patterns in file paths (*, ?, [)
- [x] Expand glob to list of matching files
- [x] Create separate source for each matching file

### Feature 5: Vim Keybindings (Medium) - COMPLETED
- [x] Add vim_mode_enabled and vim_pending_key to TailLoggerApp
- [x] Implement j/k scroll, G/gg jump, Ctrl+d/u page scroll
- [x] Implement Ctrl+f/b page scroll (alternate)
- [x] Implement / search focus, n/N search navigation
- [x] Add vim toggle in settings dialog
- [x] Show vim mode indicator in status bar

---

## Verification Checklist

After implementing all 5 features:

1. **Search Navigation:**
   - [ ] Type search term, verify match count shows
   - [ ] Press F3, verify jumps to next match
   - [ ] Press Shift+F3, verify jumps to previous
   - [ ] Click ▲/▼ buttons

2. **Copy/Export:**
   - [ ] Right-click on log line
   - [ ] Select "Copy Line"
   - [ ] Paste in text editor, verify content

3. **Bookmarks:**
   - [ ] Click star icon on a line
   - [ ] Scroll away, use bookmark dropdown to jump back
   - [ ] Restart app, verify bookmarks persist

4. **Glob Patterns:**
   - [ ] Open file with pattern `/tmp/*.log`
   - [ ] Verify multiple matching files are tailed

5. **Vim Mode:**
   - [ ] Enable vim mode in settings
   - [ ] Press j/k, verify line-by-line scroll
   - [ ] Press G (shift+g), verify jump to end
   - [ ] Type gg, verify jump to start
   - [ ] Press n/N, verify search navigation
   - [ ] Verify [VIM] indicator in status bar

---

## Future Features (Backlog)

6. Log Statistics Panel
7. Custom Log Level Colors
8. Enhanced Sound Alerts
9. SSH Known Hosts Auto-Add
10. Filter Active Indicator
11. Time-based Filtering
