//! Read-only views of what a connected server offers: tools, resources, prompts, and server info.
//!
//! The DTOs are deliberately flat and permissive: they are built by re-reading the JSON that `rmcp`
//! serializes for each MCP type, so new optional fields in the spec never break the explorer.

use std::collections::BTreeMap;

// Server logging is deprecated in the MCP spec (SEP-2577) but servers still offer it.
#[allow(deprecated)]
use rmcp::model::{LoggingLevel, SetLevelRequestParams};
use rmcp::{
    model::{
        ArgumentInfo, CompleteRequestParams, CompletionContext, GetPromptRequestParams,
        ReadResourceRequestParams, Reference, SubscribeRequestParams, UnsubscribeRequestParams,
    },
    service::Peer,
    RoleClient,
};
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
    /// `resources.subscribe`: resources can be watched.
    pub can_subscribe: bool,
    /// `completions`: prompt arguments and template variables can be completed.
    pub has_completions: bool,
    /// `logging`: the server accepts a log level.
    pub has_logging: bool,
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
        can_subscribe: capabilities["resources"]["subscribe"] == Value::Bool(true),
        has_completions: has("completions"),
        has_logging: has("logging"),
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

/// Reads a resource. Returns the MCP `ReadResourceResult` (`contents: [{uri, mimeType, text|blob}]`).
pub async fn read_resource(peer: &Peer<RoleClient>, uri: &str) -> DbResult<JsonValue> {
    let result = peer
        .read_resource(ReadResourceRequestParams::new(uri))
        .await
        .map_err(service_error)?;
    Ok(JsonValue(
        serde_json::to_value(result).map_err(|e| DbError::Connection(e.to_string()))?,
    ))
}

/// Gets a prompt with the given arguments. Returns the MCP `GetPromptResult`
/// (`description`, `messages: [{role, content}]`).
pub async fn get_prompt(
    peer: &Peer<RoleClient>,
    name: &str,
    arguments: &BTreeMap<String, String>,
) -> DbResult<JsonValue> {
    let mut params = GetPromptRequestParams::new(name);
    if !arguments.is_empty() {
        params = params.with_arguments(
            arguments
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect(),
        );
    }
    let result = peer.get_prompt(params).await.map_err(service_error)?;
    Ok(JsonValue(
        serde_json::to_value(result).map_err(|e| DbError::Connection(e.to_string()))?,
    ))
}

/// What is completed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum CompletionTarget {
    /// An argument of the prompt with this name.
    Prompt { name: String },
    /// A variable of the resource template with this URI template.
    Resource {
        #[serde(rename = "uriTemplate")]
        uri_template: String,
    },
}

/// The suggestions of a server for a partly typed value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Completions {
    pub values: Vec<String>,
    /// The server has more than it sent.
    pub has_more: bool,
}

/// Asks the server to complete `value` for `argument`. `context` holds the arguments that are
/// already filled in. A server without the `completions` capability gives no suggestions.
pub async fn complete(
    peer: &Peer<RoleClient>,
    target: &CompletionTarget,
    argument: &str,
    value: &str,
    context: &BTreeMap<String, String>,
) -> DbResult<Completions> {
    if !details(peer)?.has_completions {
        return Ok(Completions {
            values: vec![],
            has_more: false,
        });
    }
    let reference = match target {
        CompletionTarget::Prompt { name } => Reference::for_prompt(name),
        CompletionTarget::Resource { uri_template } => Reference::for_resource(uri_template),
    };
    let mut params = CompleteRequestParams::new(reference, ArgumentInfo::new(argument, value));
    if !context.is_empty() {
        params = params.with_context(CompletionContext::with_arguments(
            context
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ));
    }
    let info = peer
        .complete(params)
        .await
        .map_err(service_error)?
        .completion;
    let has_more = info.has_more.unwrap_or(false)
        || info
            .total
            .is_some_and(|total| total as usize > info.values.len());
    Ok(Completions {
        values: info.values,
        has_more,
    })
}

/// Starts watching a resource. Needs `resources.subscribe`.
///
/// `resources/subscribe` is legacy in the newest protocol version, but it is what servers of the
/// versions we negotiate offer.
#[allow(deprecated)]
pub async fn subscribe(peer: &Peer<RoleClient>, uri: &str) -> DbResult<()> {
    if !details(peer)?.can_subscribe {
        return Err(DbError::Invalid(
            "the server does not support resource subscriptions".into(),
        ));
    }
    peer.subscribe(SubscribeRequestParams::new(uri))
        .await
        .map_err(service_error)
}

/// Stops watching a resource.
#[allow(deprecated)]
pub async fn unsubscribe(peer: &Peer<RoleClient>, uri: &str) -> DbResult<()> {
    peer.unsubscribe(UnsubscribeRequestParams::new(uri))
        .await
        .map_err(service_error)
}

/// The log levels of MCP, least to most severe.
pub const LOG_LEVELS: [&str; 8] = [
    "debug",
    "info",
    "notice",
    "warning",
    "error",
    "critical",
    "alert",
    "emergency",
];

#[allow(deprecated)]
fn log_level(name: &str) -> Option<LoggingLevel> {
    Some(match name {
        "debug" => LoggingLevel::Debug,
        "info" => LoggingLevel::Info,
        "notice" => LoggingLevel::Notice,
        "warning" => LoggingLevel::Warning,
        "error" => LoggingLevel::Error,
        "critical" => LoggingLevel::Critical,
        "alert" => LoggingLevel::Alert,
        "emergency" => LoggingLevel::Emergency,
        _ => return None,
    })
}

/// Sends `logging/setLevel`. Needs the `logging` capability.
#[allow(deprecated)]
pub async fn set_log_level(peer: &Peer<RoleClient>, level: &str) -> DbResult<()> {
    let parsed = log_level(level)
        .ok_or_else(|| DbError::Invalid(format!("unknown log level \"{level}\"")))?;
    if !details(peer)?.has_logging {
        return Err(DbError::Invalid(
            "the server does not support setting the log level".into(),
        ));
    }
    peer.set_level(SetLevelRequestParams::new(parsed))
        .await
        .map_err(service_error)
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
    fn completion_targets_use_the_wire_names() {
        let prompt: CompletionTarget =
            serde_json::from_value(json!({"type": "prompt", "name": "greet"})).unwrap();
        assert_eq!(
            prompt,
            CompletionTarget::Prompt {
                name: "greet".into()
            }
        );
        let resource: CompletionTarget =
            serde_json::from_value(json!({"type": "resource", "uriTemplate": "test://u/{n}"}))
                .unwrap();
        assert_eq!(
            resource,
            CompletionTarget::Resource {
                uri_template: "test://u/{n}".into()
            }
        );
    }

    #[test]
    fn every_log_level_has_a_name_and_back() {
        for name in LOG_LEVELS {
            assert!(log_level(name).is_some(), "{name}");
        }
        assert!(log_level("loud").is_none());
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
