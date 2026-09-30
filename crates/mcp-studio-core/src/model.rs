//! Serializable domain types shared with the frontend.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Basic information about the running app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

impl AppInfo {
    pub fn current() -> Self {
        Self {
            name: "MCP Studio".to_owned(),
            version: crate::version().to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_serializes_camel_free_fields() {
        let json = serde_json::to_value(AppInfo::current()).unwrap();
        assert_eq!(json["name"], "MCP Studio");
        assert_eq!(json["version"], crate::version());
    }
}
