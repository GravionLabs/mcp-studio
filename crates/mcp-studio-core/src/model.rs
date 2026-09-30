//! Serializable domain types shared with the frontend.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Arbitrary JSON. Exported to TypeScript as `unknown` (specta rejects `serde_json::Value` because
/// its numbers include `i64`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JsonValue(pub serde_json::Value);

impl Type for JsonValue {
    fn definition(_types: &mut specta::Types) -> specta::datatype::DataType {
        specta::datatype::DataType::Reference(specta_typescript::define("unknown"))
    }
}

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
