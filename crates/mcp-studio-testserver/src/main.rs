//! `mcp-studio-testserver` (stdio) or `mcp-studio-testserver --http <port>` (Streamable HTTP at /mcp).

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
