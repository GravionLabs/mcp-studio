//! A tiny deterministic MCP server for tests: `echo`, `add`, and `fail` tools.

use std::future::Future;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        GetPromptRequestParams, GetPromptResponse, GetPromptResult, ListPromptsResult,
        ListResourcesResult, PaginatedRequestParams, Prompt, PromptArgument, PromptMessage,
        ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
        ResourceContents, Role, ServerCapabilities, ServerConfig,
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
