//! Data about an available application update. Checking and installing happens in the Tauri layer;
//! this type only crosses the IPC boundary.

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// Version that would be installed.
    pub version: String,
    /// Version that is running now.
    pub current_version: String,
    /// Release notes, if the release has any.
    pub notes: Option<String>,
    /// Publication date as an RFC 3339 string, if known.
    pub date: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_in_camel_case() {
        let info = UpdateInfo {
            version: "0.2.0".into(),
            current_version: "0.1.0".into(),
            notes: None,
            date: Some("2026-10-01T00:00:00Z".into()),
        };
        let json = serde_json::to_value(info).unwrap();
        assert_eq!(json["currentVersion"], "0.1.0");
        assert_eq!(json["notes"], serde_json::Value::Null);
    }
}
