//! Tests for the window's filtered view (`view::View`): incremental refresh,
//! ring-buffer eviction, rebuilds, tabs, and `find`'s wrap-around search.

use crate::view::View;
use oxitailr_core::app::{AppCore, CoreChannels};
use oxitailr_core::config::AppConfig;
use oxitailr_core::source::SourceEvent;
use std::sync::Arc;
use tokio::runtime::Runtime;

/// Index of ERROR in `AppCore::show_levels` (Trace, Debug, Info, Warn, Error, Fatal).
const ERROR_IDX: usize = 4;

/// An `AppCore` plus the runtime and channels it needs to stay alive.
struct Harness {
    core: AppCore,
    _channels: CoreChannels,
    _rt: Arc<Runtime>,
}

fn harness(buffer_size: usize) -> Harness {
    let rt = Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime"),
    );
    let mut config = AppConfig::default();
    config.general.buffer_size = buffer_size;
    let (core, channels) = AppCore::new(Arc::clone(&rt), config, Vec::new());
    Harness {
        core,
        _channels: channels,
        _rt: rt,
    }
}

fn feed(core: &mut AppCore, source: &str, lines: &[&str]) {
    core.ingest(SourceEvent::Lines {
        source: source.to_string(),
        lines: lines.iter().map(|l| l.to_string()).collect(),
    });
}

/// Every row the view shows, as (line number, raw text).
fn rows(view: &View, core: &AppCore) -> Vec<(usize, String)> {
    (0..view.len())
        .map(|i| {
            let l = view.line(core, i).expect("row in range");
            (l.line_num, l.entry.raw.clone())
        })
        .collect()
}

/// A view built from scratch over the same core, for comparison.
fn fresh(core: &AppCore, tab: Option<&str>) -> Vec<(usize, String)> {
    let mut v = View::default();
    v.tab = tab.map(str::to_string);
    v.refresh(core);
    rows(&v, core)
}

fn n(i: usize) -> String {
    format!("2024-01-01 10:00:0{i} INFO line {i}")
}

#[test]
fn refresh_incremental_two_batches_matches_rebuild() {
    let mut h = harness(100);
    h.core.filter_text = "keep".into();
    h.core.update_filter();
    let mut view = View::default();

    feed(
        &mut h.core,
        "a",
        &[
            "2024-01-01 10:00:00 INFO keep 1",
            "2024-01-01 10:00:01 INFO drop 2",
            "2024-01-01 10:00:02 WARN keep 3",
        ],
    );
    view.refresh(&h.core);
    assert_eq!(view.len(), 2);
    let epoch = view.epoch;

    feed(
        &mut h.core,
        "a",
        &[
            "2024-01-01 10:00:03 ERROR keep 4",
            "2024-01-01 10:00:04 INFO drop 5",
            "2024-01-01 10:00:05 INFO keep 6",
        ],
    );
    view.refresh(&h.core);
    let got = rows(&view, &h.core);
    let nums: Vec<usize> = got.iter().map(|r| r.0).collect();
    assert_eq!(nums, [1, 3, 4, 6]);
    assert_eq!(got, fresh(&h.core, None));
    assert_eq!(view.epoch, epoch, "an incremental refresh is not a rebuild");

    // A refresh with nothing new changes nothing (no duplicate hits).
    view.refresh(&h.core);
    assert_eq!(rows(&view, &h.core), got);
}

#[test]
fn incremental_scan_skips_already_scanned_lines() {
    // Observable proof that old lines are not rescanned: change the filter
    // WITHOUT invalidating. Old hits must stay as they were, and only the new
    // lines are judged by the new filter.
    let mut h = harness(100);
    let mut view = View::default();
    feed(&mut h.core, "a", &["alpha 1", "beta 2"]);
    view.refresh(&h.core);
    assert_eq!(view.len(), 2);

    h.core.filter_text = "alpha".into();
    h.core.update_filter();
    feed(&mut h.core, "a", &["alpha 3", "beta 4"]);
    view.refresh(&h.core);
    let nums: Vec<usize> = rows(&view, &h.core).iter().map(|r| r.0).collect();
    assert_eq!(nums, [1, 2, 3]);
}

#[test]
fn eviction_drops_front_hits_within_epoch() {
    let mut h = harness(5);
    let mut view = View::default();
    feed(&mut h.core, "a", &[&n(1), &n(2), &n(3)]);
    view.refresh(&h.core);
    let epoch = view.epoch;
    assert_eq!(view.len(), 3);
    assert_eq!(view.dropped, 0);

    let more: Vec<String> = (4..=8).map(n).collect();
    let more: Vec<&str> = more.iter().map(String::as_str).collect();
    feed(&mut h.core, "a", &more);
    view.refresh(&h.core);

    assert_eq!(h.core.log_state.lines.len(), 5);
    assert_eq!(view.len(), 5);
    assert_eq!(view.dropped, 3);
    assert_eq!(view.epoch, epoch, "eviction must not start a new epoch");
    let first = view.line(&h.core, 0).expect("first row");
    assert_eq!(first.line_num, 4);
    assert_eq!(first.entry.raw, n(4));
    assert_eq!(rows(&view, &h.core), fresh(&h.core, None));
}

#[test]
fn eviction_of_whole_buffer_between_refreshes() {
    let mut h = harness(5);
    let mut view = View::default();
    feed(&mut h.core, "a", &[&n(1), &n(2)]);
    view.refresh(&h.core);
    let lines: Vec<String> = (3..=9).map(n).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    feed(&mut h.core, "a", &lines);
    view.refresh(&h.core);
    let nums: Vec<usize> = rows(&view, &h.core).iter().map(|r| r.0).collect();
    assert_eq!(nums, [5, 6, 7, 8, 9]);
    assert_eq!(view.dropped, 2, "only hits the view held count as dropped");
}

#[test]
fn invalidate_rebuilds_with_new_level_mask() {
    let mut h = harness(100);
    let mut view = View::default();
    feed(
        &mut h.core,
        "a",
        &[
            "2024-01-01 10:00:00 INFO one",
            "2024-01-01 10:00:01 ERROR two",
            "2024-01-01 10:00:02 INFO three",
            "2024-01-01 10:00:03 ERROR four",
        ],
    );
    view.refresh(&h.core);
    assert_eq!(view.len(), 4);
    view.dropped = 7; // pretend some evictions happened in this epoch
    let epoch = view.epoch;

    h.core.show_levels[ERROR_IDX] = false;
    view.invalidate();
    view.refresh(&h.core);

    assert_eq!(view.epoch, epoch + 1);
    assert_eq!(view.dropped, 0);
    let nums: Vec<usize> = rows(&view, &h.core).iter().map(|r| r.0).collect();
    assert_eq!(nums, [1, 3]);
}

#[test]
fn invalidate_rebuilds_with_new_filter_text() {
    let mut h = harness(100);
    let mut view = View::default();
    feed(&mut h.core, "a", &["apple", "banana", "apricot"]);
    view.refresh(&h.core);
    h.core.filter_text = "^ap".into();
    h.core.update_filter();
    view.invalidate();
    view.refresh(&h.core);
    let texts: Vec<String> = rows(&view, &h.core).into_iter().map(|r| r.1).collect();
    assert_eq!(texts, ["apple", "apricot"]);
}

#[test]
fn clear_then_invalidate_restarts_line_numbers() {
    let mut h = harness(100);
    let mut view = View::default();
    feed(
        &mut h.core,
        "a",
        &["old 1", "old 2", "old 3", "old 4", "old 5"],
    );
    view.refresh(&h.core);
    assert_eq!(view.len(), 5);
    let epoch = view.epoch;

    h.core.log_state.clear();
    view.invalidate();
    feed(&mut h.core, "a", &["new 1", "new 2"]);
    view.refresh(&h.core);

    assert_eq!(view.epoch, epoch + 1);
    let got = rows(&view, &h.core);
    assert_eq!(
        got,
        [(1, "new 1".to_string()), (2, "new 2".to_string())],
        "no stale hits from before the clear"
    );
    assert!(view.line(&h.core, 2).is_none());
}

#[test]
fn clear_with_refresh_before_new_lines_still_correct_after_invalidate() {
    let mut h = harness(100);
    let mut view = View::default();
    feed(&mut h.core, "a", &["old 1", "old 2", "old 3"]);
    view.refresh(&h.core);

    h.core.log_state.clear();
    view.invalidate();
    view.refresh(&h.core); // empty buffer
    assert_eq!(view.len(), 0);
    feed(&mut h.core, "a", &["new 1"]);
    view.refresh(&h.core);
    assert_eq!(rows(&view, &h.core), [(1, "new 1".to_string())]);
}

#[test]
fn tab_shows_only_its_source() {
    let mut h = harness(100);
    feed(&mut h.core, "a", &["a one"]);
    feed(&mut h.core, "b", &["b one", "b two"]);
    feed(&mut h.core, "a", &["a two"]);

    let mut view = View::default();
    view.tab = Some("a".into());
    view.refresh(&h.core);
    let got = rows(&view, &h.core);
    assert_eq!(got, [(1, "a one".to_string()), (4, "a two".to_string())]);
    for i in 0..view.len() {
        assert_eq!(view.line(&h.core, i).unwrap().entry.source, "a");
    }

    // Switching the tab back to "all" after an invalidate shows everything.
    view.tab = None;
    view.invalidate();
    view.refresh(&h.core);
    assert_eq!(view.len(), 4);
}

/// A view over rows: 0 "x", 1 "hit", 2 "x", 3 "hit", 4 "x".
fn find_fixture() -> (Harness, View) {
    let mut h = harness(100);
    feed(&mut h.core, "a", &["x 0", "hit 1", "x 2", "hit 3", "x 4"]);
    let mut view = View::default();
    view.refresh(&h.core);
    (h, view)
}

fn is_hit(l: &oxitailr_core::app::DisplayLine) -> bool {
    l.entry.raw.starts_with("hit")
}

#[test]
fn find_from_none_forward_includes_first_row() {
    let (h, view) = find_fixture();
    assert_eq!(view.find(&h.core, None, true, is_hit), Some(1));
    assert_eq!(view.find(&h.core, None, true, |_| true), Some(0));
}

#[test]
fn find_from_none_backward_includes_last_row() {
    let (h, view) = find_fixture();
    assert_eq!(view.find(&h.core, None, false, is_hit), Some(3));
    assert_eq!(view.find(&h.core, None, false, |_| true), Some(4));
}

#[test]
fn find_forward_from_position_skips_it_and_wraps() {
    let (h, view) = find_fixture();
    assert_eq!(view.find(&h.core, Some(1), true, is_hit), Some(3));
    assert_eq!(view.find(&h.core, Some(3), true, is_hit), Some(1), "wraps");
    // The only match is the start itself: found last, after a full lap.
    let only_one = |l: &oxitailr_core::app::DisplayLine| l.entry.raw == "hit 1";
    assert_eq!(view.find(&h.core, Some(1), true, only_one), Some(1));
    // A start past the end is clamped to the last row.
    assert_eq!(view.find(&h.core, Some(99), true, is_hit), Some(1));
}

#[test]
fn find_backward_from_position_skips_it_and_wraps() {
    let (h, view) = find_fixture();
    assert_eq!(view.find(&h.core, Some(3), false, is_hit), Some(1));
    assert_eq!(view.find(&h.core, Some(1), false, is_hit), Some(3), "wraps");
    let only_three = |l: &oxitailr_core::app::DisplayLine| l.entry.raw == "hit 3";
    assert_eq!(view.find(&h.core, Some(3), false, only_three), Some(3));
}

#[test]
fn find_no_match_is_none() {
    let (h, view) = find_fixture();
    for (from, fwd) in [
        (None, true),
        (None, false),
        (Some(2), true),
        (Some(2), false),
    ] {
        assert_eq!(view.find(&h.core, from, fwd, |_| false), None);
    }
}

#[test]
fn find_on_empty_view_is_none() {
    let h = harness(100);
    let mut view = View::default();
    view.refresh(&h.core);
    assert_eq!(view.len(), 0);
    assert_eq!(view.find(&h.core, None, true, |_| true), None);
    assert_eq!(view.find(&h.core, Some(0), false, |_| true), None);
}

#[test]
fn line_out_of_range_is_none() {
    let (h, view) = find_fixture();
    assert!(view.line(&h.core, 4).is_some());
    assert!(view.line(&h.core, 5).is_none());
    assert!(view.line(&h.core, usize::MAX).is_none());

    let empty = View::default();
    assert!(empty.line(&h.core, 0).is_none());
}
