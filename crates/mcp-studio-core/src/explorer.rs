//! Read-only views of what a connected server offers: tools, resources, prompts, and server info.
//!
//! The DTOs are deliberately flat and permissive: they are built by re-reading the JSON that `rmcp`
//! serializes for each MCP type, so new optional fields in the spec never break the explorer.

use rmcp::{service::Peer, RoleClient};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::{
    db::{DbError, DbResult},
    model::JsonValue,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// JSON Schema of the arguments.
    #[serde(default)]
    pub input_schema: JsonValue,
    #[serde(default)]
    pub output_schema: Option<JsonValue>,
    #[serde(default)]
    pub annotations: Option<JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ResourceInfo {
    pub uri: String,
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ResourceTemplateInfo {
    pub uri_template: String,
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptArgumentInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PromptInfo {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub arguments: Vec<PromptArgumentInfo>,
}

/// What the server told us during `initialize`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ServerDetails {
    pub protocol_version: String,
    pub name: String,
    pub version: String,
    pub instructions: Option<String>,
    pub capabilities: JsonValue,
    pub has_tools: bool,
    pub has_resources: bool,
    pub has_prompts: bool,
}

fn convert<T: DeserializeOwned>(value: &impl Serialize) -> DbResult<T> {
    let json = serde_json::to_value(value).map_err(|e| DbError::Connection(e.to_string()))?;
    serde_json::from_value(json)
        .map_err(|e| DbError::Connection(format!("unexpected server data: {e}")))
}

fn convert_all<T: DeserializeOwned>(items: &[impl Serialize]) -> DbResult<Vec<T>> {
    items.iter().map(convert).collect()
}

fn service_error(error: rmcp::ServiceError) -> DbError {
    DbError::Connection(error.to_string())
}

pub fn details(peer: &Peer<RoleClient>) -> DbResult<ServerDetails> {
    let info = peer
        .peer_info()
        .ok_or_else(|| DbError::Connection("the server has not finished initializing".into()))?;
    let capabilities = serde_json::to_value(&info.capabilities).unwrap_or(Value::Null);
    let has = |key: &str| capabilities.get(key).is_some_and(|v| !v.is_null());
    let protocol = serde_json::to_value(&info.protocol_version)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    Ok(ServerDetails {
        protocol_version: protocol,
        name: info
            .server_info
            .as_ref()
            .map(|i| i.name.clone())
            .unwrap_or_default(),
        version: info
            .server_info
            .as_ref()
            .map(|i| i.version.clone())
            .unwrap_or_default(),
        instructions: info.instructions.clone(),
        has_tools: has("tools"),
        has_resources: has("resources"),
        has_prompts: has("prompts"),
        capabilities: JsonValue(capabilities),
    })
}

/// Lists are only requested when the server advertises the capability; asking anyway makes many
/// servers answer with "method not found".
pub async fn list_tools(peer: &Peer<RoleClient>) -> DbResult<Vec<ToolInfo>> {
    if !details(peer)?.has_tools {
        return Ok(vec![]);
    }
    convert_all(&peer.list_all_tools().await.map_err(service_error)?)
}

pub async fn list_resources(peer: &Peer<RoleClient>) -> DbResult<Vec<ResourceInfo>> {
    if !details(peer)?.has_resources {
        return Ok(vec![]);
    }
    convert_all(&peer.list_all_resources().await.map_err(service_error)?)
}

pub async fn list_resource_templates(
    peer: &Peer<RoleClient>,
) -> DbResult<Vec<ResourceTemplateInfo>> {
    if !details(peer)?.has_resources {
        return Ok(vec![]);
    }
    convert_all(
        &peer
            .list_all_resource_templates()
            .await
            .map_err(service_error)?,
    )
}

pub async fn list_prompts(peer: &Peer<RoleClient>) -> DbResult<Vec<PromptInfo>> {
    if !details(peer)?.has_prompts {
        return Ok(vec![]);
    }
    convert_all(&peer.list_all_prompts().await.map_err(service_error)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_info_reads_the_mcp_wire_shape() {
        let tool: ToolInfo = serde_json::from_value(json!({
            "name": "echo",
            "description": "Echo",
            "inputSchema": {"type": "object"},
            "annotations": {"readOnlyHint": true},
            "extra": 1
        }))
        .unwrap();
        assert_eq!(tool.name, "echo");
        assert_eq!(tool.title, None);
        assert_eq!(tool.input_schema.0["type"], "object");
        assert!(tool.annotations.is_some());
    }

    #[test]
    fn prompt_arguments_default_to_optional() {
        let prompt: PromptInfo = serde_json::from_value(json!({
            "name": "greet",
            "arguments": [{"name": "who"}, {"name": "tone", "required": true}]
        }))
        .unwrap();
        assert!(!prompt.arguments[0].required);
        assert!(prompt.arguments[1].required);
    }

    #[test]
    fn resources_use_camel_case_mime_type() {
        let resource: ResourceInfo = serde_json::from_value(
            json!({"uri": "file:///a", "name": "a", "mimeType": "text/plain"}),
        )
        .unwrap();
        assert_eq!(resource.mime_type.as_deref(), Some("text/plain"));
    }
}
