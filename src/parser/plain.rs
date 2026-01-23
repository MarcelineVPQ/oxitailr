use super::Parser;
use crate::models::{LogEntry, LogLevel};
use chrono::{DateTime, NaiveDateTime, Utc};
use regex::Regex;
use std::sync::LazyLock;

static TIMESTAMP_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        // ISO 8601: 2024-01-22T10:15:32.123Z
        (
            Regex::new(r"(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?)").unwrap(),
            "%Y-%m-%dT%H:%M:%S%.fZ",
        ),
        // Common log format: 2024-01-22 10:15:32
        (
            Regex::new(r"(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})").unwrap(),
            "%Y-%m-%d %H:%M:%S",
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
                    if let Ok(dt) = NaiveDateTime::parse_from_str(m.as_str(), format) {
                        return Some(dt.and_utc());
                    }
                    // Try parsing as DateTime<Utc> directly for ISO format
                    if let Ok(dt) = DateTime::parse_from_rfc3339(m.as_str()) {
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
            .and_then(|m| LogLevel::from_str(m.as_str()))
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

    fn name(&self) -> &str {
        "plain"
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
    fn test_parse_no_level() {
        let parser = PlainParser::new();
        let entry = parser.parse("test", "Just a plain message");
        assert!(entry.level.is_none());
    }
}
