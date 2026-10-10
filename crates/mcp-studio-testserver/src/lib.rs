//! A tiny deterministic MCP server for tests: `echo`, `add`, `sleep` and `fail` tools, and tools that
//! ask the client for something: `sample` (sampling), `elicit` (elicitation) and `roots` (roots).

// Sampling and roots are deprecated in the MCP spec (SEP-2577), but servers still use them.
#![allow(deprecated)]

pub mod oauth;

use std::future::Future;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CreateMessageRequestParams, ElicitRequestParams, ElicitationAction, ElicitationSchema,
        GetPromptRequestParams, GetPromptResponse, GetPromptResult, ListPromptsResult,
        ListResourcesResult, PaginatedRequestParams, Prompt, PromptArgument, PromptMessage,
        ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
        ResourceContents, Role, SamplingMessage, ServerCapabilities, ServerConfig,
    },
    schemars,
    service::{MaybeSendFuture, RequestContext, RoleServer},
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use serde::Deserialize;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EchoRequest {
    #[schemars(description = "Text to echo back")]
    pub message: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddRequest {
    pub a: i64,
    pub b: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SleepRequest {
    #[schemars(description = "Milliseconds to wait")]
    pub ms: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SampleRequest {
    #[schemars(description = "What to ask the client's model")]
    pub prompt: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ElicitToolRequest {
    #[schemars(description = "What to ask the user")]
    pub question: String,
}

#[derive(Debug, Clone)]
pub struct TestServer {
    tool_router: ToolRouter<Self>,
}

impl TestServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

impl Default for TestServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router]
impl TestServer {
    #[tool(description = "Echo the message back")]
    fn echo(&self, Parameters(EchoRequest { message }): Parameters<EchoRequest>) -> String {
        message
    }

    #[tool(description = "Add two integers")]
    fn add(&self, Parameters(AddRequest { a, b }): Parameters<AddRequest>) -> String {
        (a + b).to_string()
    }

    #[tool(description = "Wait for the given number of milliseconds, then answer \"done\"")]
    async fn sleep(&self, Parameters(SleepRequest { ms }): Parameters<SleepRequest>) -> String {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
        "done".to_owned()
    }

    #[tool(description = "Ask the client for a model answer (sampling) and return its text")]
    async fn sample(
        &self,
        Parameters(SampleRequest { prompt }): Parameters<SampleRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let request =
            CreateMessageRequestParams::new(vec![SamplingMessage::user_text(prompt)], 256)
                .with_system_prompt("You answer briefly.");
        let answer = context
            .peer
            .create_message(request)
            .await
            .map_err(|e| format!("sampling failed: {e}"))?;
        let text = serde_json::to_value(&answer.message.content)
            .ok()
            .and_then(|content| {
                content
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        Ok(format!("{} said: {text}", answer.model))
    }

    #[tool(
        description = "Ask the user a question (elicitation); answers accept, decline or cancel"
    )]
    async fn elicit(
        &self,
        Parameters(ElicitToolRequest { question }): Parameters<ElicitToolRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let schema = ElicitationSchema::builder()
            .required_string("answer")
            .build()
            .map_err(|e| e.to_string())?;
        let result = context
            .peer
            .create_elicitation(ElicitRequestParams::FormElicitationParams {
                meta: None,
                message: question,
                requested_schema: schema,
            })
            .await
            .map_err(|e| format!("elicitation failed: {e}"))?;
        Ok(match result.action {
            ElicitationAction::Accept => {
                format!("accept: {}", result.content.unwrap_or_default())
            }
            ElicitationAction::Decline => "decline".to_owned(),
            _ => "cancel".to_owned(),
        })
    }

    #[tool(description = "List the roots the client offers, one URI per line")]
    async fn roots(&self, context: RequestContext<RoleServer>) -> Result<String, String> {
        let roots = context
            .peer
            .list_roots()
            .await
            .map_err(|e| format!("roots failed: {e}"))?;
        Ok(roots
            .roots
            .iter()
            .map(|r| r.uri.clone())
            .collect::<Vec<_>>()
            .join("\n"))
    }

    #[tool(description = "Always fails with a tool error")]
    fn fail(&self) -> Result<String, String> {
        Err("intentional failure".to_owned())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for TestServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
        )
        .with_instructions("MCP Studio reference server")
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + MaybeSendFuture + '_ {
        let resources = vec![Resource::new("test://greeting", "greeting")
            .with_description("A friendly greeting")
            .with_mime_type("text/plain")];
        std::future::ready(Ok(ListResourcesResult::with_all_items(resources)))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, McpError>> + MaybeSendFuture + '_ {
        let result = match request.uri.as_str() {
            "test://greeting" => Ok(ReadResourceResult::new(vec![ResourceContents::text(
                "Hello from MCP Studio",
                request.uri.clone(),
            )
            .with_mime_type("text/plain")])
            .into()),
            other => Err(McpError::invalid_params(
                format!("unknown resource {other}"),
                None,
            )),
        };
        std::future::ready(result)
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, McpError>> + MaybeSendFuture + '_ {
        let prompt = Prompt::new(
            "greet",
            Some("Greets someone"),
            Some(vec![PromptArgument::new("name").with_required(true)]),
        );
        std::future::ready(Ok(ListPromptsResult::with_all_items(vec![prompt])))
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResponse, McpError>> + MaybeSendFuture + '_ {
        let result = if request.name == "greet" {
            let name = request
                .arguments
                .as_ref()
                .and_then(|a| a.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("world")
                .to_owned();
            Ok(GetPromptResult::new(vec![PromptMessage::new_text(
                Role::User,
                format!("Hello, {name}!"),
            )])
            .with_description("A greeting")
            .into())
        } else {
            Err(McpError::invalid_params(
                format!("unknown prompt {}", request.name),
                None,
            ))
        };
        std::future::ready(result)
    }
}
