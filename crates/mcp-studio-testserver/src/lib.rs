//! A tiny deterministic MCP server for tests: `echo`, `add`, and `fail` tools.

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router, ServerHandler,
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
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("MCP Studio reference server")
    }
}
