//! A client reaches OAuth and Azure-login servers through the HTTP proxy: the proxy signs the
//! requests with the user's token, like a direct connection does.

use std::{
    collections::BTreeMap,
    process::Stdio,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use azure_core::{
    credentials::{AccessToken, TokenCredential, TokenRequestOptions},
    time::{Duration as TimeDuration, OffsetDateTime},
};
use mcp_studio_core::{
    db::Db,
    environments::Environments,
    events::CollectingSink,
    http_proxy::HttpProxy,
    registry::{Registry, ServerInput, TransportKind},
    secrets::{MemoryStore, SecretStore},
    session::SessionManager,
};
use rmcp::{
    model::{CallToolRequestParams, ClientConfig},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
};

fn server_binary() -> std::path::PathBuf {
    static BUILD: std::sync::Once = std::sync::Once::new();
    BUILD.call_once(|| {
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "-p", "mcp-studio-testserver"])
            .status()
            .unwrap();
        assert!(status.success());
    });
    let exe = if cfg!(windows) {
        "mcp-studio-testserver.exe"
    } else {
        "mcp-studio-testserver"
    };
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug")
        .join(exe)
        .canonicalize()
        .unwrap()
}

struct Harness {
    registry: Registry,
    manager: Arc<SessionManager>,
    proxy: HttpProxy,
    secrets: Arc<MemoryStore>,
    base: String,
    _server: Child,
}

/// The test server in OAuth mode, a session manager (to sign in) and the proxy, sharing one database.
async fn harness() -> Harness {
    let mut child = Command::new(server_binary())
        .args(["--oauth", "0"])
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let url = lines
        .next_line()
        .await
        .unwrap()
        .unwrap()
        .trim_start_matches("listening on ")
        .to_owned();
    let base = url.trim_end_matches("/mcp").to_owned();

    let db = Db::open_in_memory().await.unwrap();
    let registry = Registry::new(db.clone());
    let secrets = Arc::new(MemoryStore::default());
    let sink = Arc::new(CollectingSink::default());
    let manager = SessionManager::new(
        db.clone(),
        registry.clone(),
        Environments::new(db.clone()),
        secrets.clone() as Arc<dyn SecretStore>,
        sink.clone(),
    );
    manager.set_connect_timeout(Duration::from_secs(10));
    let proxy = HttpProxy::start(
        db.clone(),
        registry.clone(),
        Environments::new(db),
        secrets.clone() as Arc<dyn SecretStore>,
        sink,
        0,
    )
    .await
    .unwrap();
    Harness {
        registry,
        manager,
        proxy,
        secrets,
        base,
        _server: child,
    }
}

async fn add_server(h: &Harness, name: &str, oauth: bool, azure: bool) -> String {
    h.registry
        .create(ServerInput {
            name: name.into(),
            transport: TransportKind::Http,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            cwd: None,
            url: Some(format!("{}/mcp", h.base)),
            headers: BTreeMap::new(),
            tags: vec![],
            oauth,
            oauth_client_id: None,
            oauth_scopes: azure.then(|| "api://x/.default".to_owned()),
            oauth_callback_port: None,
            azure_credentials: azure,
            roots: vec![],
        })
        .await
        .unwrap()
        .id
}

/// Plays the browser: opens the authorization URL and follows the redirect back to the app.
fn browser() -> impl Fn(&str) -> Result<(), String> + Send + Sync {
    |url: &str| {
        let url = url.to_owned();
        tokio::spawn(async move {
            let response = reqwest::get(&url).await.expect("authorization page");
            assert!(response.status().is_success());
        });
        Ok(())
    }
}

/// A client program that is pointed at the proxy.
async fn client_through_proxy(h: &Harness, name: &str) -> Result<String, String> {
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(h.proxy.url_for(name)),
    );
    let client = ClientConfig::default()
        .serve(transport)
        .await
        .map_err(|e| e.to_string())?;
    let tools = client.list_tools(None).await.map_err(|e| e.to_string())?;
    assert!(tools.tools.iter().any(|t| t.name == "echo"));
    let result = client
        .call_tool(
            CallToolRequestParams::new("add").with_arguments(
                serde_json::json!({"a": 40, "b": 2})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .map_err(|e| e.to_string())?;
    let _ = client.cancel().await;
    Ok(serde_json::to_value(&result).unwrap()["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned())
}

/// What the proxy answers to an `initialize` request of a client that is not allowed in.
async fn initialize_text(h: &Harness, name: &str) -> String {
    let response = reqwest::Client::new()
        .post(h.proxy.url_for(name))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_client_error() || response.status().is_server_error());
    response.text().await.unwrap()
}

#[tokio::test]
async fn a_signed_in_oauth_server_works_through_the_proxy_and_the_token_is_refreshed() {
    let h = harness().await;
    let id = add_server(&h, "Protected", true, false).await;
    h.manager.sign_in(&id, &browser()).await.expect("sign in");

    assert_eq!(client_through_proxy(&h, "protected").await.unwrap(), "42");

    // Access tokens live two seconds; the next client must get a refreshed one.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(client_through_proxy(&h, "protected").await.unwrap(), "42");
    let stats: serde_json::Value = reqwest::get(format!("{}/_stats", h.base))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(stats["refreshes"].as_u64().unwrap() >= 1, "{stats}");
}

#[tokio::test]
async fn an_oauth_server_without_sign_in_tells_the_client_to_sign_in() {
    let h = harness().await;
    add_server(&h, "Protected", true, false).await;
    let text = initialize_text(&h, "protected").await;
    assert!(
        text.contains("Sign in to \"Protected\" in MCP Studio"),
        "{text}"
    );

    // Signing out is noticed on the next request, and the client is told what to do.
    let id = add_server(&h, "Other", true, false).await;
    h.manager.sign_in(&id, &browser()).await.unwrap();
    assert_eq!(client_through_proxy(&h, "other").await.unwrap(), "42");
    h.manager.sign_out(&id).await.unwrap();
    assert!(h.secrets.get(&format!("oauth/{id}")).unwrap().is_none());
    let text = initialize_text(&h, "other").await;
    assert!(
        text.contains("Sign in to \"Other\" in MCP Studio"),
        "{text}"
    );
}

/// A credential that gets its tokens from the test server and counts how often it was asked.
#[derive(Debug)]
struct IssuingCredential {
    base: String,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl TokenCredential for IssuingCredential {
    async fn get_token(
        &self,
        _scopes: &[&str],
        _options: Option<TokenRequestOptions<'_>>,
    ) -> azure_core::Result<AccessToken> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let body: serde_json::Value = reqwest::Client::new()
            .post(format!("{}/_issue", self.base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        Ok(AccessToken::new(
            body["token"].as_str().unwrap().to_owned(),
            OffsetDateTime::now_utc() + TimeDuration::hours(1),
        ))
    }
}

#[tokio::test]
async fn a_server_with_the_azure_login_works_through_the_proxy() {
    let h = harness().await;
    let credential = Arc::new(IssuingCredential {
        base: h.base.clone(),
        calls: AtomicUsize::new(0),
    });
    h.proxy.set_azure_credential(credential.clone());
    add_server(&h, "Entra", false, true).await;

    assert_eq!(client_through_proxy(&h, "entra").await.unwrap(), "42");
    // One token serves every request of the session.
    assert_eq!(credential.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_token_the_server_rejects_is_replaced_and_the_request_repeated() {
    let h = harness().await;
    let credential = Arc::new(IssuingCredential {
        base: h.base.clone(),
        calls: AtomicUsize::new(0),
    });
    h.proxy.set_azure_credential(credential.clone());
    add_server(&h, "Entra", false, true).await;
    assert_eq!(client_through_proxy(&h, "entra").await.unwrap(), "42");

    // The server now only accepts a newer token, as if the one in use was revoked.
    reqwest::Client::new()
        .post(format!("{}/_issue", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(client_through_proxy(&h, "entra").await.unwrap(), "42");
    assert!(credential.calls.load(Ordering::SeqCst) >= 2);
}
