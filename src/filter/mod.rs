mod rules;

pub use rules::{FilterEngine, FilterRule};

use crate::models::LogEntry;

pub trait Filter: Send + Sync {
    fn matches(&self, entry: &LogEntry) -> bool;
}
