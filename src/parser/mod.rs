mod json;
mod plain;

pub use json::JsonParser;
pub use plain::PlainParser;

use crate::models::LogEntry;

pub trait Parser: Send + Sync {
    fn parse(&self, source: &str, line: &str) -> LogEntry;
}
