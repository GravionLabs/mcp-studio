//! `mcp-studio-testserver` (stdio), `--http <port>` (Streamable HTTP at /mcp), or `--oauth <port>`
//! (OAuth-protected HTTP server with its own tiny authorization server).

use mcp_studio_testserver::TestServer;
use rmcp::{
    transport::{
        stdio,
        streamable_http_server::{
            session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
        },
    },
    ServiceExt,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--oauth") {
        // OAuth-protected MCP server plus a minimal authorization server (see oauth.rs).
        let requested: u16 = args
            .get(pos + 1)
            .map(|p| p.parse())
            .transpose()?
            .unwrap_or(0);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", requested)).await?;
        let port = listener.local_addr()?.port();
        println!("listening on http://127.0.0.1:{port}/mcp");
        axum::serve(listener, mcp_studio_testserver::oauth::router(port)).await?;
        return Ok(());
    }
    if let Some(pos) = args.iter().position(|a| a == "--http") {
        let port: u16 = args
            .get(pos + 1)
            .map(|p| p.parse())
            .transpose()?
            .unwrap_or(0);
        let service: StreamableHttpService<TestServer, LocalSessionManager> =
            StreamableHttpService::new(
                || Ok(TestServer::new()),
                Default::default(),
                StreamableHttpServerConfig::default(),
            );
        let router = axum::Router::new().nest_service("/mcp", service);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
        // Tests read the bound address from the first stdout line.
        println!("listening on http://{}/mcp", listener.local_addr()?);
        axum::serve(listener, router).await?;
    } else {
        TestServer::new().serve(stdio()).await?.waiting().await?;
    }
    Ok(())
}
