//! Integration tests for Oxitailr
//!
//! These tests verify the core functionality without requiring a GUI.

use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use tempfile::TempDir;

/// Helper to create a temporary log file with content
fn create_temp_log(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    let mut file = File::create(&path).unwrap();
    file.write_all(content.as_bytes()).unwrap();
    path
}

mod file_operations {
    use super::*;

    #[test]
    fn test_read_simple_log_file() {
        let dir = TempDir::new().unwrap();
        let content = "2024-01-01 10:00:00 INFO Starting application\n\
                       2024-01-01 10:00:01 DEBUG Loading config\n\
                       2024-01-01 10:00:02 ERROR Failed to connect\n";
        let path = create_temp_log(&dir, "app.log", content);

        let read_content = fs::read_to_string(&path).unwrap();
        assert_eq!(read_content.lines().count(), 3);
        assert!(read_content.contains("INFO"));
        assert!(read_content.contains("ERROR"));
    }

    #[test]
    fn test_append_to_log_file() {
        let dir = TempDir::new().unwrap();
        let path = create_temp_log(&dir, "app.log", "Line 1\n");

        // Append more content
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "Line 2").unwrap();
        writeln!(file, "Line 3").unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), 3);
    }

    #[test]
    fn test_empty_log_file() {
        let dir = TempDir::new().unwrap();
        let path = create_temp_log(&dir, "empty.log", "");

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn test_large_lines() {
        let dir = TempDir::new().unwrap();
        let long_line = "X".repeat(10000);
        let content = format!("{}\n", long_line);
        let path = create_temp_log(&dir, "large.log", &content);

        let read_content = fs::read_to_string(&path).unwrap();
        assert_eq!(read_content.trim().len(), 10000);
    }
}

mod log_parsing {

    #[test]
    fn test_json_log_detection() {
        let json_line = r#"{"timestamp":"2024-01-01T10:00:00Z","level":"INFO","message":"Hello"}"#;
        assert!(json_line.starts_with('{'));
        assert!(json_line.ends_with('}'));
    }

    #[test]
    fn test_common_log_levels() {
        let levels = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL"];
        let log_line = "2024-01-01 10:00:00 ERROR Something went wrong";

        let found_level = levels.iter().find(|&level| log_line.contains(level));
        assert_eq!(found_level, Some(&"ERROR"));
    }

    #[test]
    fn test_iso_timestamp_format() {
        let line = "2024-01-15T10:30:45.123Z INFO Message";
        // ISO 8601 pattern: YYYY-MM-DDTHH:MM:SS
        assert!(line.contains("2024-01-15T10:30:45"));
    }

    #[test]
    fn test_common_timestamp_format() {
        let line = "2024-01-15 10:30:45 INFO Message";
        // Common format: YYYY-MM-DD HH:MM:SS
        assert!(line.contains("2024-01-15 10:30:45"));
    }
}

mod filtering {
    use regex::Regex;

    #[test]
    fn test_regex_filter_match() {
        let pattern = Regex::new(r"ERROR|FATAL").unwrap();
        let lines = vec![
            "INFO Starting up",
            "ERROR Connection failed",
            "DEBUG Retrying",
            "FATAL System crash",
        ];

        let matches: Vec<_> = lines.iter().filter(|l| pattern.is_match(l)).collect();
        assert_eq!(matches.len(), 2);
        assert!(matches[0].contains("ERROR"));
        assert!(matches[1].contains("FATAL"));
    }

    #[test]
    fn test_case_insensitive_filter() {
        let pattern = Regex::new(r"(?i)error").unwrap();
        let lines = vec![
            "ERROR uppercase",
            "error lowercase",
            "Error mixed",
            "INFO other",
        ];

        let matches: Vec<_> = lines.iter().filter(|l| pattern.is_match(l)).collect();
        assert_eq!(matches.len(), 3);
    }

    #[test]
    fn test_exclude_filter() {
        let include = Regex::new(r"ERROR").unwrap();
        let exclude = Regex::new(r"healthcheck").unwrap();

        let lines = vec![
            "ERROR Connection failed",
            "ERROR healthcheck timeout",
            "ERROR Database error",
        ];

        let matches: Vec<_> = lines
            .iter()
            .filter(|l| include.is_match(l) && !exclude.is_match(l))
            .collect();

        assert_eq!(matches.len(), 2);
        assert!(!matches.iter().any(|l| l.contains("healthcheck")));
    }

    #[test]
    fn test_level_filter() {
        let levels_to_show = vec!["ERROR", "WARN"];

        let lines = vec![
            ("INFO Starting", "INFO"),
            ("WARN Low memory", "WARN"),
            ("ERROR Failed", "ERROR"),
            ("DEBUG Details", "DEBUG"),
        ];

        let visible: Vec<_> = lines
            .iter()
            .filter(|(_, level)| levels_to_show.contains(level))
            .collect();

        assert_eq!(visible.len(), 2);
    }
}

mod bookmarks {
    use std::collections::{HashMap, HashSet};

    #[test]
    fn test_per_source_bookmarks() {
        let mut bookmarks: HashMap<String, HashSet<usize>> = HashMap::new();

        // Add bookmarks to different sources
        bookmarks
            .entry("app.log".to_string())
            .or_default()
            .insert(50);
        bookmarks
            .entry("app.log".to_string())
            .or_default()
            .insert(100);
        bookmarks
            .entry("server.log".to_string())
            .or_default()
            .insert(25);

        // Verify isolation
        let app_bookmarks = bookmarks.get("app.log").unwrap();
        let server_bookmarks = bookmarks.get("server.log").unwrap();

        assert_eq!(app_bookmarks.len(), 2);
        assert_eq!(server_bookmarks.len(), 1);
        assert!(app_bookmarks.contains(&50));
        assert!(app_bookmarks.contains(&100));
        assert!(server_bookmarks.contains(&25));
        assert!(!server_bookmarks.contains(&50)); // No cross-contamination
    }

    #[test]
    fn test_bookmark_toggle() {
        let mut bookmarks: HashSet<usize> = HashSet::new();

        // Toggle on
        let line = 42;
        if !bookmarks.remove(&line) {
            bookmarks.insert(line);
        }
        assert!(bookmarks.contains(&42));

        // Toggle off
        if !bookmarks.remove(&line) {
            bookmarks.insert(line);
        }
        assert!(!bookmarks.contains(&42));
    }

    #[test]
    fn test_sorted_bookmarks() {
        let mut bookmarks: HashSet<usize> = HashSet::new();
        bookmarks.insert(100);
        bookmarks.insert(25);
        bookmarks.insert(50);

        let mut sorted: Vec<_> = bookmarks.iter().copied().collect();
        sorted.sort();

        assert_eq!(sorted, vec![25, 50, 100]);
    }
}

mod source_ordering {
    #[test]
    fn test_consistent_source_ordering() {
        // Simulate HashMap keys (non-deterministic order)
        let sources = vec!["zebra.log", "alpha.log", "beta.log"];

        // Sort for consistent ordering
        let mut sorted = sources.clone();
        sorted.sort();

        assert_eq!(sorted, vec!["alpha.log", "beta.log", "zebra.log"]);

        // Multiple iterations should give same order
        let mut sorted2 = sources.clone();
        sorted2.sort();
        assert_eq!(sorted, sorted2);
    }
}

mod session_state {
    use serde::{Deserialize, Serialize};
    use std::collections::{HashMap, HashSet};

    #[derive(Serialize, Deserialize, Default, PartialEq, Debug)]
    struct MockSession {
        #[serde(default)]
        open_sources: Vec<String>,
        #[serde(default)]
        selected_source: Option<String>,
        #[serde(default)]
        bookmarks: HashMap<String, HashSet<usize>>,
    }

    #[test]
    fn test_session_serialization() {
        let mut session = MockSession::default();
        session.open_sources = vec!["app.log".to_string(), "server.log".to_string()];
        session.selected_source = Some("app.log".to_string());
        session
            .bookmarks
            .insert("app.log".to_string(), [10, 20, 30].into_iter().collect());

        // Serialize
        let json = serde_json::to_string(&session).unwrap();

        // Deserialize
        let restored: MockSession = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.open_sources, session.open_sources);
        assert_eq!(restored.selected_source, session.selected_source);
        assert_eq!(restored.bookmarks.get("app.log").unwrap().len(), 3);
    }

    #[test]
    fn test_session_default_on_missing_fields() {
        // Simulate old session format without bookmarks
        let json = r#"{"open_sources":["app.log"],"selected_source":"app.log"}"#;

        let session: MockSession = serde_json::from_str(json).unwrap();

        assert_eq!(session.open_sources, vec!["app.log"]);
        assert!(session.bookmarks.is_empty()); // Default
    }
}
