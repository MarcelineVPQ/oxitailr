use super::Filter;
use crate::models::{LogEntry, LogLevel};
use chrono::Utc;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

/// Maximum size limit for regex patterns to prevent DoS
const MAX_REGEX_PATTERN_LEN: usize = 1000;

/// Timeout/size limit for regex compilation (in bytes of the regex program)
const MAX_REGEX_SIZE: usize = 10 * 1024 * 1024; // 10MB

/// Safely compile a regex with DoS protection
fn safe_regex_compile(pattern: &str) -> Result<Regex, String> {
    // Check pattern length
    if pattern.len() > MAX_REGEX_PATTERN_LEN {
        return Err(format!(
            "Regex pattern too long ({} > {} chars)",
            pattern.len(),
            MAX_REGEX_PATTERN_LEN
        ));
    }

    // Compile with size limit
    RegexBuilder::new(pattern)
        .size_limit(MAX_REGEX_SIZE)
        .build()
        .map_err(|e| format!("Invalid regex: {}", e))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FilterRule {
    Contains {
        pattern: String,
        case_sensitive: bool,
    },
    Regex {
        pattern: String,
    },
    Level {
        level: LogLevel,
        min: bool,
    },
    Field {
        name: String,
        pattern: String,
    },
    Source {
        name: String,
    },
    And {
        rules: Vec<FilterRule>,
    },
    Or {
        rules: Vec<FilterRule>,
    },
    Not {
        rule: Box<FilterRule>,
    },
    /// Filter to entries from the last N minutes
    TimeRange {
        minutes: u64,
    },
}

impl FilterRule {
    pub fn contains(pattern: &str) -> Self {
        FilterRule::Contains {
            pattern: pattern.to_string(),
            case_sensitive: false,
        }
    }

    pub fn regex(pattern: &str) -> Self {
        FilterRule::Regex {
            pattern: pattern.to_string(),
        }
    }

    pub fn min_level(level: LogLevel) -> Self {
        FilterRule::Level { level, min: true }
    }

    pub fn field(name: &str, pattern: &str) -> Self {
        FilterRule::Field {
            name: name.to_string(),
            pattern: pattern.to_string(),
        }
    }

    pub fn time_range(minutes: u64) -> Self {
        FilterRule::TimeRange { minutes }
    }
}

impl Filter for FilterRule {
    fn matches(&self, entry: &LogEntry) -> bool {
        match self {
            FilterRule::Contains {
                pattern,
                case_sensitive,
            } => {
                if *case_sensitive {
                    entry.raw.contains(pattern) || entry.message.contains(pattern)
                } else {
                    let pattern_lower = pattern.to_lowercase();
                    entry.raw.to_lowercase().contains(&pattern_lower)
                        || entry.message.to_lowercase().contains(&pattern_lower)
                }
            }
            FilterRule::Regex { pattern } => match safe_regex_compile(pattern) {
                Ok(re) => re.is_match(&entry.raw) || re.is_match(&entry.message),
                Err(_) => false,
            },
            FilterRule::Level { level, min } => {
                if let Some(entry_level) = &entry.level {
                    if *min {
                        entry_level >= level
                    } else {
                        entry_level == level
                    }
                } else {
                    false
                }
            }
            FilterRule::Field { name, pattern } => {
                if let Some(value) = entry.fields.get(name) {
                    let value_str = match value {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    match safe_regex_compile(pattern) {
                        Ok(re) => re.is_match(&value_str),
                        Err(_) => value_str.contains(pattern),
                    }
                } else {
                    false
                }
            }
            FilterRule::Source { name } => entry.source == *name || entry.source.contains(name),
            FilterRule::And { rules } => rules.iter().all(|r| r.matches(entry)),
            FilterRule::Or { rules } => rules.iter().any(|r| r.matches(entry)),
            FilterRule::Not { rule } => !rule.matches(entry),
            FilterRule::TimeRange { minutes } => entry.timestamp.is_some_and(|ts| {
                let cutoff = Utc::now() - chrono::Duration::minutes(*minutes as i64);
                ts > cutoff
            }),
        }
    }
}

pub struct FilterEngine {
    include_rules: Vec<FilterRule>,
    exclude_rules: Vec<FilterRule>,
    live_filter: Option<String>,
    compiled_live_filter: Option<Regex>,
}

impl FilterEngine {
    pub fn new() -> Self {
        Self {
            include_rules: Vec::new(),
            exclude_rules: Vec::new(),
            live_filter: None,
            compiled_live_filter: None,
        }
    }

    pub fn add_include_rule(&mut self, rule: FilterRule) {
        self.include_rules.push(rule);
    }

    pub fn add_exclude_rule(&mut self, rule: FilterRule) {
        self.exclude_rules.push(rule);
    }

    pub fn set_live_filter(&mut self, pattern: Option<String>) {
        self.live_filter = pattern.clone();
        self.compiled_live_filter = pattern.and_then(|p| safe_regex_compile(&p).ok());
    }

    pub fn clear_rules(&mut self) {
        self.include_rules.clear();
        self.exclude_rules.clear();
    }

    pub fn matches(&self, entry: &LogEntry) -> bool {
        // Check exclude rules first
        for rule in &self.exclude_rules {
            if rule.matches(entry) {
                return false;
            }
        }

        // Check live filter
        if let Some(re) = &self.compiled_live_filter {
            if !re.is_match(&entry.raw) && !re.is_match(&entry.message) {
                return false;
            }
        } else if let Some(pattern) = &self.live_filter {
            let pattern_lower = pattern.to_lowercase();
            if !entry.raw.to_lowercase().contains(&pattern_lower)
                && !entry.message.to_lowercase().contains(&pattern_lower)
            {
                return false;
            }
        }

        // If no include rules, accept everything not excluded
        if self.include_rules.is_empty() {
            return true;
        }

        // Check include rules (OR logic)
        self.include_rules.iter().any(|r| r.matches(entry))
    }
}

impl Default for FilterEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_entry(message: &str) -> LogEntry {
        LogEntry {
            id: 0,
            timestamp: None,
            source: "test".to_string(),
            level: Some(LogLevel::Info),
            message: message.to_string(),
            raw: message.to_string(),
            fields: HashMap::new(),
        }
    }

    #[test]
    fn test_contains_filter() {
        let rule = FilterRule::contains("error");
        assert!(rule.matches(&make_entry("An error occurred")));
        assert!(rule.matches(&make_entry("An ERROR occurred")));
        assert!(!rule.matches(&make_entry("All is well")));
    }

    #[test]
    fn test_regex_filter() {
        let rule = FilterRule::regex(r"\d{3}-\d{4}");
        assert!(rule.matches(&make_entry("Phone: 555-1234")));
        assert!(!rule.matches(&make_entry("No phone")));
    }

    #[test]
    fn test_and_filter() {
        let rule = FilterRule::And {
            rules: vec![
                FilterRule::contains("error"),
                FilterRule::contains("database"),
            ],
        };
        assert!(rule.matches(&make_entry("Database error occurred")));
        assert!(!rule.matches(&make_entry("Error occurred")));
    }

    #[test]
    fn test_filter_engine() {
        let mut engine = FilterEngine::new();
        engine.add_include_rule(FilterRule::contains("important"));
        engine.add_exclude_rule(FilterRule::contains("spam"));

        assert!(engine.matches(&make_entry("This is important")));
        assert!(!engine.matches(&make_entry("This is spam")));
        assert!(!engine.matches(&make_entry("This is important spam")));
    }
}
