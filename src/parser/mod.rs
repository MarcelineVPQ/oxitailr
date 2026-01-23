mod json;
mod plain;

pub use json::JsonParser;
pub use plain::PlainParser;

use crate::models::LogEntry;

pub trait Parser: Send + Sync {
    fn parse(&self, source: &str, line: &str) -> LogEntry;
}

pub fn auto_detect_parser(line: &str) -> Box<dyn Parser> {
    if line.trim_start().starts_with('{') && line.trim_end().ends_with('}') {
        Box::new(JsonParser::new())
    } else {
        Box::new(PlainParser::new())
    }
}
