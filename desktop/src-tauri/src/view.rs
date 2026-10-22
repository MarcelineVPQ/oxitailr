//! The window's view of the log: which lines pass the current tab, level mask
//! and filter, and how a visible row is drawn.
//!
//! The window only ever asks for the rows it is showing, so this keeps the
//! passing lines as a list of line numbers and extends it incrementally: a
//! change scans only the lines that arrived since the last look, never the
//! whole buffer. A filter, level or tab change rebuilds it once.

use oxitailr_core::app::{AppCore, DisplayLine, HighlightMatcher};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, OnceLock};

#[derive(Default)]
pub struct View {
    /// Only lines from this source, or all of them.
    pub tab: Option<String>,
    /// Lowercased search term, painted in the visible rows.
    pub search_lower: String,
    /// Line numbers (`DisplayLine::line_num`) that pass, ascending.
    hits: VecDeque<usize>,
    /// The highest line number already scanned.
    scanned: usize,
    /// Set when the rules change or the buffer is cleared: rebuild on next look.
    stale: bool,
    /// Bumped on every rebuild. Positions from an older epoch mean nothing now.
    pub epoch: u64,
    /// Hits dropped off the front (lines evicted from the ring buffer) in this
    /// epoch. Every position shifts down by one per drop, so the window uses
    /// this to keep a paused view and its cursor on the same lines.
    pub dropped: u64,
}

impl View {
    /// The rules changed (tab, filter, levels) or the buffer was cleared.
    pub fn invalidate(&mut self) {
        self.stale = true;
    }

    /// Bring the hit list up to date with the buffer.
    pub fn refresh(&mut self, core: &AppCore) {
        let lines = &core.log_state.lines;
        if self.stale {
            self.hits.clear();
            self.scanned = 0;
            self.stale = false;
            self.epoch += 1;
            self.dropped = 0;
        }
        let Some(first) = lines.front().map(|l| l.line_num) else {
            self.hits.clear();
            return;
        };
        let last = lines.back().map(|l| l.line_num).unwrap_or(0);
        // Lines that fell out of the ring buffer.
        while self.hits.front().is_some_and(|&n| n < first) {
            self.hits.pop_front();
            self.dropped += 1;
        }
        if last > self.scanned {
            let skip = (self.scanned + 1).saturating_sub(first);
            for line in lines.iter().skip(skip) {
                if self.passes(core, line) {
                    self.hits.push_back(line.line_num);
                }
            }
            self.scanned = last;
        }
    }

    fn passes(&self, core: &AppCore, line: &DisplayLine) -> bool {
        if let Some(tab) = &self.tab {
            if &line.entry.source != tab {
                return false;
            }
        }
        core.passes_filter(line)
    }

    pub fn len(&self) -> usize {
        self.hits.len()
    }

    /// The line at a position in the view.
    pub fn line<'a>(&self, core: &'a AppCore, index: usize) -> Option<&'a Arc<DisplayLine>> {
        let n = *self.hits.get(index)?;
        let first = core.log_state.lines.front()?.line_num;
        core.log_state.lines.get(n.checked_sub(first)?)
    }

    /// The first position whose line satisfies `pred`, searching from `from`
    /// in one direction and wrapping around (like the terminal app's n/N).
    /// `from: None` starts at the edge: the first row going forward, the last
    /// row going back, each of them included.
    pub fn find(
        &self,
        core: &AppCore,
        from: Option<usize>,
        forward: bool,
        pred: impl Fn(&DisplayLine) -> bool,
    ) -> Option<usize> {
        let n = self.len();
        if n == 0 {
            return None;
        }
        let mut order: Box<dyn Iterator<Item = usize>> = match (from.map(|f| f.min(n - 1)), forward)
        {
            (None, true) => Box::new(0..n),
            (None, false) => Box::new((0..n).rev()),
            (Some(f), true) => Box::new((f + 1..n).chain(0..=f)),
            (Some(f), false) => Box::new((0..f).rev().chain((f..n).rev())),
        };
        order.find(|&i| self.line(core, i).is_some_and(|l| pred(l)))
    }
}

/// One drawn piece of a row: text plus how to paint it.
#[derive(Serialize)]
pub struct Seg {
    pub t: String,
    /// A highlight rule's color.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub c: Option<[u8; 3]>,
    /// Part of a search match.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub s: bool,
}

#[derive(Serialize)]
pub struct Row {
    /// Position in the view.
    pub i: usize,
    /// Line number in the source buffer (stable while the line exists).
    pub n: usize,
    pub src: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lvl: Option<&'static str>,
    pub segs: Vec<Seg>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub mark: bool,
}

fn ansi_re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").expect("valid ANSI pattern"))
}

/// The text a row shows: the parsed message, with any ANSI codes removed.
pub fn row_text(line: &DisplayLine) -> String {
    if line.has_ansi {
        ansi_re().replace_all(&line.entry.raw, "").into_owned()
    } else {
        line.entry.message.clone()
    }
}

/// Split `msg` into segments: config highlight rules in their color, the search
/// term marked (search wins on overlap). Ported from the terminal app's
/// `styled_message`: byte offsets, and non-ASCII substring matches are skipped
/// so slicing never lands inside a character.
pub fn segments(msg: &str, highlights: &[HighlightMatcher], search_lower: &str) -> Vec<Seg> {
    if highlights.is_empty() && search_lower.is_empty() {
        return vec![Seg {
            t: msg.to_string(),
            c: None,
            s: false,
        }];
    }
    #[derive(Clone, Copy, PartialEq)]
    struct Mark {
        c: Option<[u8; 3]>,
        s: bool,
    }
    let plain = Mark { c: None, s: false };
    let n = msg.len();
    let mut marks = vec![plain; n];
    let lower = msg.to_lowercase();
    let aligned = lower.len() == n;

    for h in highlights {
        let mut paint = |start: usize, end: usize| {
            for m in &mut marks[start.min(n)..end.min(n)] {
                if m.c.is_none() {
                    m.c = Some(h.color);
                }
            }
        };
        if let Some(re) = &h.regex {
            for m in re.find_iter(msg) {
                paint(m.start(), m.end());
            }
        } else if aligned && !h.needle_lower.is_empty() {
            let mut pos = 0;
            while let Some(rel) = lower[pos..].find(&h.needle_lower) {
                let start = pos + rel;
                paint(start, start + h.needle_lower.len());
                pos = start + h.needle_lower.len();
            }
        }
    }
    if aligned && !search_lower.is_empty() {
        let mut pos = 0;
        while let Some(rel) = lower[pos..].find(search_lower) {
            let start = pos + rel;
            for m in &mut marks[start..start + search_lower.len()] {
                m.s = true;
            }
            pos = start + search_lower.len();
        }
    }

    let mut out = Vec::new();
    let mut run_start = 0;
    let mut run = marks.first().copied().unwrap_or(plain);
    for (i, _) in msg.char_indices().skip(1) {
        if marks[i] != run {
            out.push(Seg {
                t: msg[run_start..i].to_string(),
                c: run.c,
                s: run.s,
            });
            run_start = i;
            run = marks[i];
        }
    }
    out.push(Seg {
        t: msg[run_start..].to_string(),
        c: run.c,
        s: run.s,
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hl(needle: &str, color: [u8; 3]) -> HighlightMatcher {
        HighlightMatcher {
            regex: None,
            needle_lower: needle.to_lowercase(),
            color,
        }
    }

    fn joined(segs: &[Seg]) -> String {
        segs.iter().map(|s| s.t.as_str()).collect()
    }

    #[test]
    fn no_rules_is_one_plain_segment() {
        let segs = segments("hello world", &[], "");
        assert_eq!(segs.len(), 1);
        assert!(segs[0].c.is_none() && !segs[0].s);
    }

    #[test]
    fn highlight_colors_only_the_match() {
        let segs = segments("an ERROR here", &[hl("error", [255, 0, 0])], "");
        assert_eq!(joined(&segs), "an ERROR here");
        let colored: Vec<&str> = segs
            .iter()
            .filter(|s| s.c.is_some())
            .map(|s| s.t.as_str())
            .collect();
        assert_eq!(colored, ["ERROR"]);
    }

    #[test]
    fn search_marks_every_match_case_insensitively() {
        let segs = segments("Foo foo FOO", &[], "foo");
        let marked = segs.iter().filter(|s| s.s).count();
        assert_eq!(marked, 3);
        assert_eq!(joined(&segs), "Foo foo FOO");
    }

    #[test]
    fn non_ascii_text_never_panics() {
        let segs = segments("ünïcödé ERROR ✓", &[hl("error", [1, 2, 3])], "rr");
        assert_eq!(joined(&segs), "ünïcödé ERROR ✓");
    }

    #[test]
    fn ansi_codes_are_stripped() {
        let entry =
            oxitailr_core::models::LogEntry::new("s".into(), "\x1b[31mred\x1b[0m plain".into());
        let line = DisplayLine::from_entry(entry, 1);
        assert_eq!(row_text(&line), "red plain");
    }
}
