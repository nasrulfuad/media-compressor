use serde::Serialize;

use crate::error::AppResult;

pub fn to_pretty_json<T: Serialize>(value: &T) -> AppResult<String> {
    Ok(serde_json::to_string_pretty(value)?)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::report::{InfoReport, SizeStats};

    #[test]
    fn to_pretty_json_indents_nested_values() {
        let json = to_pretty_json(&SizeStats::new(100, 50)).unwrap();
        assert!(json.contains("\n  \"original_size\": 100"));
        assert!(json.contains("\"bytes_saved\": 50"));
    }

    #[test]
    fn to_pretty_json_round_trips_info_report() {
        let report = InfoReport {
            path: PathBuf::from("a.pdf").display().to_string(),
            file_size_bytes: 10,
            page_count: 2,
            pdf_version: "1.7".to_string(),
        };
        let value: serde_json::Value =
            serde_json::from_str(&to_pretty_json(&report).unwrap()).unwrap();
        assert_eq!(value["page_count"], 2);
        assert_eq!(value["pdf_version"], "1.7");
        assert_eq!(value["path"], "a.pdf");
    }
}
