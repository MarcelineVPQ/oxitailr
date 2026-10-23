//! Export the lines the view shows (the source tab, the filter and the level
//! switches; search only highlights) to a CSV file.
//!
//! Only the lines in memory can be exported: the buffer keeps the newest
//! `general.buffer_size` lines, so the result says how many lines that is.
//! Cells follow the spreadsheet rule AVIEW's exports use: one starting with
//! = + - @, a tab or a carriage return gets a leading ' so a spreadsheet shows
//! it as text instead of running it as a formula (a log line is data from
//! anywhere). A lone "-" is left alone.

use crate::view::{row_text, View};
use crate::Shared;
use oxitailr_core::app::AppCore;
use serde::Serialize;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use tauri::State;

/// One exported line.
pub struct ExportRow {
    pub line: usize,
    pub source: String,
    /// As the log wrote it, without a time zone (the parsers don't know one).
    pub time: Option<String>,
    pub level: Option<&'static str>,
    pub message: String,
}

#[derive(Serialize)]
pub struct ExportResult {
    pub path: String,
    pub lines: usize,
    /// The buffer's size: older lines than these aren't in memory to export.
    pub buffer_size: usize,
}

const FORMULA_START: [char; 6] = ['=', '+', '-', '@', '\t', '\r'];

/// One CSV field: spreadsheet-safe, then quoted when it holds a comma, a quote
/// or a line break (RFC 4180; quotes doubled).
pub fn csv_field(value: &str) -> String {
    let safe = if value != "-" && value.starts_with(FORMULA_START) {
        format!("'{value}")
    } else {
        value.to_string()
    };
    if safe.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", safe.replace('"', "\"\""))
    } else {
        safe
    }
}

/// Write the header and the rows; returns how many rows were written. UTF-8
/// with a byte-order mark: without one, Excel reads a CSV in the Windows code
/// page and garbles any non-ASCII text in a line.
pub fn write_csv<W: Write>(out: W, rows: &[ExportRow]) -> io::Result<usize> {
    let mut w = BufWriter::new(out);
    write!(w, "\u{feff}")?;
    writeln!(w, "line,source,time,level,message")?;
    // loop: NOT-WORTH — one pass over the rows being exported, off the lock, on a click
    for r in rows {
        writeln!(
            w,
            "{},{},{},{},{}",
            r.line,
            csv_field(&r.source),
            csv_field(r.time.as_deref().unwrap_or("")),
            csv_field(r.level.unwrap_or("")),
            csv_field(&r.message),
        )?;
    }
    w.flush()?;
    Ok(rows.len())
}

/// The rows the view shows now, in order.
pub fn collect_rows(view: &mut View, core: &AppCore) -> Vec<ExportRow> {
    view.refresh(core);
    // loop: NOT-WORTH — N = the view's lines (≤ buffer_size), once per export click
    (0..view.len())
        .filter_map(|i| {
            let line = view.line(core, i)?;
            Some(ExportRow {
                line: line.line_num,
                source: line.entry.source.clone(),
                time: line
                    .entry
                    .timestamp
                    .map(|t| t.format("%Y-%m-%d %H:%M:%S%.3f").to_string()),
                level: line.entry.level.map(|l| l.as_str()),
                message: row_text(line),
            })
        })
        .collect()
}

/// Export the view to `path` (chosen in the save dialog).
#[tauri::command]
pub async fn export_view(shared: State<'_, Shared>, path: String) -> Result<ExportResult, String> {
    // Copy the rows out under the lock (quick), then write the file without it,
    // so a slow disk never holds up tailing.
    let (rows, buffer_size) = {
        let mut guard = shared.lock();
        let h = &mut *guard;
        let rows = collect_rows(&mut h.view, &h.core);
        (rows, h.core.config.general.buffer_size)
    };
    let path = PathBuf::from(path);
    let file = std::fs::File::create(&path)
        .map_err(|e| format!("Could not create {}: {e}", path.display()))?;
    let lines =
        write_csv(file, &rows).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    Ok(ExportResult {
        path: path.display().to_string(),
        lines,
        buffer_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_fields_stay_plain() {
        assert_eq!(csv_field("worker.log"), "worker.log");
        assert_eq!(csv_field(""), "");
        assert_eq!(csv_field("-"), "-");
    }

    #[test]
    fn commas_quotes_and_breaks_are_quoted() {
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn formulas_become_text() {
        assert_eq!(csv_field("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
        assert_eq!(csv_field("+1"), "'+1");
        assert_eq!(csv_field("- infrastructure poll"), "'- infrastructure poll");
        assert_eq!(csv_field("@SUM(A1)"), "'@SUM(A1)");
    }

    #[test]
    fn writes_header_and_rows() {
        let rows = [
            ExportRow {
                line: 7,
                source: "app.log".into(),
                time: Some("2026-10-23 10:16:25.423".into()),
                level: Some("ERROR"),
                message: "QRC: Connection error, refused".into(),
            },
            ExportRow {
                line: 8,
                source: "app.log".into(),
                time: None,
                level: None,
                message: "plain".into(),
            },
        ];
        let mut out = Vec::new();
        assert_eq!(write_csv(&mut out, &rows).unwrap(), 2);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\u{feff}line,source,time,level,message\n\
             7,app.log,2026-10-23 10:16:25.423,ERROR,\"QRC: Connection error, refused\"\n\
             8,app.log,,,plain\n"
        );
    }
}
