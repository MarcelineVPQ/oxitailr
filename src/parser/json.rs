use super::Parser;
use crate::models::{LogEntry, LogLevel};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;

pub struct JsonParser {
    timestamp_fields: Vec<&'static str>,
    level_fields: Vec<&'static str>,
    message_fields: Vec<&'static str>,
}

impl JsonParser {
    pub fn new() -> Self {
        Self {
            timestamp_fields: vec!["timestamp", "time", "ts", "@timestamp", "datetime", "date"],
            level_fields: vec!["level", "severity", "log_level", "loglevel", "lvl"],
            message_fields: vec!["message", "msg", "text", "log", "body"],
        }
    }

    fn extract_timestamp(&self, obj: &serde_json::Map<String, Value>) -> Option<DateTime<Utc>> {
        for field in &self.timestamp_fields {
            if let Some(value) = obj.get(*field) {
                match value {
                    Value::String(s) => {
                        // Try RFC3339
                        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                            return Some(dt.with_timezone(&Utc));
                        }
                        // Try common format
                        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                        {
                            return Some(dt.and_utc());
                        }
                    }
                    Value::Number(n) => {
                        // Unix timestamp (seconds or milliseconds)
                        if let Some(ts) = n.as_i64() {
                            let ts = if ts > 1_000_000_000_000 {
                                ts / 1000
                            } else {
                                ts
                            };
                            if let Some(dt) = DateTime::from_timestamp(ts, 0) {
                                return Some(dt);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }

    fn extract_level(&self, obj: &serde_json::Map<String, Value>) -> Option<LogLevel> {
        for field in &self.level_fields {
            if let Some(Value::String(s)) = obj.get(*field) {
                if let Some(level) = LogLevel::from_str(s) {
                    return Some(level);
                }
            }
        }
        None
    }

    fn extract_message(&self, obj: &serde_json::Map<String, Value>) -> Option<String> {
        for field in &self.message_fields {
            if let Some(Value::String(s)) = obj.get(*field) {
                return Some(s.clone());
            }
        }
        None
    }
}

impl Default for JsonParser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser for JsonParser {
    fn parse(&self, source: &str, line: &str) -> LogEntry {
        let mut entry = LogEntry::new(source.to_string(), line.to_string());

        if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(line) {
            if let Some(ts) = self.extract_timestamp(&obj) {
                entry = entry.with_timestamp(ts);
            }

            if let Some(level) = self.extract_level(&obj) {
                entry = entry.with_level(level);
            }

            if let Some(msg) = self.extract_message(&obj) {
                entry = entry.with_message(msg);
            }

            // Store all fields
            let fields: HashMap<String, Value> = obj.into_iter().collect();
            entry = entry.with_fields(fields);
        }

        entry
    }

    fn name(&self) -> &str {
        "json"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_json_log() {
        let parser = JsonParser::new();
        let line = r#"{"timestamp":"2024-01-22T10:15:32Z","level":"error","message":"Connection failed","host":"server1"}"#;
        let entry = parser.parse("test", line);

        assert!(entry.timestamp.is_some());
        assert_eq!(entry.level, Some(LogLevel::Error));
        assert_eq!(entry.message, "Connection failed");
        assert!(entry.fields.contains_key("host"));
    }

    #[test]
    fn test_parse_json_unix_timestamp() {
        let parser = JsonParser::new();
        let line = r#"{"ts":1705922132,"level":"info","msg":"Started"}"#;
        let entry = parser.parse("test", line);

        assert!(entry.timestamp.is_some());
        assert_eq!(entry.level, Some(LogLevel::Info));
        assert_eq!(entry.message, "Started");
    }

    #[test]
    fn test_parse_invalid_json() {
        let parser = JsonParser::new();
        let line = "Not JSON at all";
        let entry = parser.parse("test", line);

        assert!(entry.timestamp.is_none());
        assert!(entry.level.is_none());
        assert_eq!(entry.raw, line);
    }
}
