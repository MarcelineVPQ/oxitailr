use super::Parser;
use crate::models::{LogEntry, LogLevel};
use chrono::{DateTime, NaiveDateTime, Utc};
use regex::Regex;
use std::sync::LazyLock;

static TIMESTAMP_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        // ISO 8601: 2024-01-22T10:15:32.123Z
        (
            Regex::new(r"(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?)")
                .unwrap(),
            "%Y-%m-%dT%H:%M:%S%.fZ",
        ),
        // Common log format: 2024-01-22 10:15:32, with optional fractional
        // seconds after a dot or a comma (Python's logging writes
        // "2024-01-22 10:15:32,423"). Without the fraction in the pattern the
        // milliseconds were dropped and every line read as .000.
        (
            Regex::new(r"(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:[.,]\d{1,9})?)").unwrap(),
            "%Y-%m-%d %H:%M:%S%.f",
        ),
        // Syslog: Jan 22 10:15:32
        (
            Regex::new(r"([A-Z][a-z]{2} \d{1,2} \d{2}:\d{2}:\d{2})").unwrap(),
            "%b %d %H:%M:%S",
        ),
    ]
});

static LEVEL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(TRACE|DEBUG|INFO|WARN(?:ING)?|ERROR|ERR|FATAL|CRITICAL|CRIT)\b").unwrap()
});

pub struct PlainParser;

impl PlainParser {
    pub fn new() -> Self {
        Self
    }

    fn extract_timestamp(&self, line: &str) -> Option<DateTime<Utc>> {
        for (pattern, format) in TIMESTAMP_PATTERNS.iter() {
            if let Some(captures) = pattern.captures(line) {
                if let Some(m) = captures.get(1) {
                    // chrono's %.f wants a dot; a comma decimal separator is
                    // just as valid (ISO 8601, Python's logging).
                    let text = m.as_str().replace(',', ".");
                    if let Ok(dt) = NaiveDateTime::parse_from_str(&text, format) {
                        return Some(dt.and_utc());
                    }
                    // Try parsing as DateTime<Utc> directly for ISO format
                    if let Ok(dt) = DateTime::parse_from_rfc3339(&text) {
                        return Some(dt.with_timezone(&Utc));
                    }
                }
            }
        }
        None
    }

    fn extract_level(&self, line: &str) -> Option<LogLevel> {
        LEVEL_PATTERN
            .captures(line)
            .and_then(|c| c.get(1))
            .and_then(|m| LogLevel::parse(m.as_str()))
    }
}

impl Default for PlainParser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser for PlainParser {
    fn parse(&self, source: &str, line: &str) -> LogEntry {
        let mut entry = LogEntry::new(source.to_string(), line.to_string());

        if let Some(ts) = self.extract_timestamp(line) {
            entry = entry.with_timestamp(ts);
        }

        if let Some(level) = self.extract_level(line) {
            entry = entry.with_level(level);
        }

        entry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_iso_timestamp() {
        let parser = PlainParser::new();
        let entry = parser.parse("test", "2024-01-22T10:15:32.123Z INFO Starting server");
        assert!(entry.timestamp.is_some());
        assert_eq!(entry.level, Some(LogLevel::Info));
    }

    #[test]
    fn test_parse_common_timestamp() {
        let parser = PlainParser::new();
        let entry = parser.parse("test", "2024-01-22 10:15:32 ERROR Connection failed");
        assert!(entry.timestamp.is_some());
        assert_eq!(entry.level, Some(LogLevel::Error));
    }

    #[test]
    fn test_common_timestamp_keeps_milliseconds() {
        let parser = PlainParser::new();
        // Python logging: comma before the milliseconds.
        let entry = parser.parse(
            "test",
            "2026-10-23 10:16:25,423 - infrastructure_discovery - INFO - Upserting",
        );
        assert_eq!(
            entry.timestamp.unwrap().format("%H:%M:%S%.3f").to_string(),
            "10:16:25.423"
        );
        // A dot works the same, and no fraction still parses.
        let entry = parser.parse("test", "2026-10-23 10:16:25.007 INFO x");
        assert_eq!(
            entry.timestamp.unwrap().format("%H:%M:%S%.3f").to_string(),
            "10:16:25.007"
        );
        let entry = parser.parse("test", "2026-10-23 10:16:25 INFO x");
        assert_eq!(
            entry.timestamp.unwrap().format("%H:%M:%S%.3f").to_string(),
            "10:16:25.000"
        );
    }

    #[test]
    fn test_parse_no_level() {
        let parser = PlainParser::new();
        let entry = parser.parse("test", "Just a plain message");
        assert!(entry.level.is_none());
    }
}
