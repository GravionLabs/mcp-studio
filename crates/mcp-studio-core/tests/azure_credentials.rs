//! Sign-in with the Azure login end to end: a fake credential fetches its tokens from the test server
//! (`mcp-studio-testserver --oauth`, `/_issue`), so no Azure CLI is involved.

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
    error::ErrorKind,
    time::{Duration as TimeDuration, OffsetDateTime},
};

use mcp_studio_core::{
    db::Db,
    environments::Environments,
    events::CollectingSink,
    model::JsonValue,
    registry::{Registry, ServerInput, TransportKind},
    secrets::{MemoryStore, SecretStore},
    session::{SessionManager, ToolCallRequest},
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
    base: String,
    _server: Child,
}

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
    let manager = SessionManager::new(
        db.clone(),
        registry.clone(),
        Environments::new(db),
        secrets as Arc<dyn SecretStore>,
        Arc::new(CollectingSink::default()),
    );
    manager.set_connect_timeout(Duration::from_secs(10));
    Harness {
        registry,

        manager,
        base,
        _server: child,
    }
}

/// A credential that gets its tokens from the test server and counts how often it was asked.
#[derive(Debug)]
struct IssuingCredential {
    base: String,
    calls: AtomicUsize,
    scopes: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl TokenCredential for IssuingCredential {
    async fn get_token(
        &self,
        scopes: &[&str],
        _options: Option<TokenRequestOptions<'_>>,
    ) -> azure_core::Result<AccessToken> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.scopes.lock().unwrap() = scopes.iter().map(|s| (*s).to_string()).collect();
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

#[derive(Debug)]
struct FailingCredential;

#[async_trait::async_trait]
impl TokenCredential for FailingCredential {
    async fn get_token(
        &self,
        _scopes: &[&str],
        _options: Option<TokenRequestOptions<'_>>,
    ) -> azure_core::Result<AccessToken> {
        Err(azure_core::Error::with_message(
            ErrorKind::Credential,
            "Please run 'az login' to set up an account",
        ))
    }
}

async fn add_server(h: &Harness, scopes: Option<&str>, oauth: bool) -> String {
    h.registry
        .create(ServerInput {
            name: "Entra".into(),
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
            oauth_scopes: scopes.map(str::to_owned),
            oauth_callback_port: None,
            azure_credentials: !oauth,
            roots: vec![],
        })
        .await
        .unwrap()
        .id
}

fn credential(h: &Harness) -> Arc<IssuingCredential> {
    Arc::new(IssuingCredential {
        base: h.base.clone(),
        calls: AtomicUsize::new(0),
        scopes: std::sync::Mutex::new(Vec::new()),
    })
}

fn call(server_id: &str, tool: &str, arguments: serde_json::Value) -> ToolCallRequest {
    ToolCallRequest {
        server_id: server_id.to_owned(),
        tool_name: tool.to_owned(),
        arguments: JsonValue(arguments),
        environment_id: None,
        call_id: mcp_studio_core::db::new_id(),
    }
}

#[tokio::test]
async fn connects_with_a_token_from_the_azure_login() {
    let h = harness().await;
    let credential = credential(&h);
    h.manager.set_azure_credential(credential.clone());
    let id = add_server(&h, Some("api://x/.default"), false).await;

    h.manager.connect(&id, None).await.expect("connect");
    let result = h
        .manager
        .call_tool(call(&id, "echo", serde_json::json!({"message": "hi"})))
        .await
        .expect("call");
    assert!(!result.is_error, "{result:?}");
    assert_eq!(credential.calls.load(Ordering::SeqCst), 1);
    assert_eq!(*credential.scopes.lock().unwrap(), vec!["api://x/.default"]);
}

#[tokio::test]
async fn a_rejected_token_is_replaced_and_the_request_repeated() {
    let h = harness().await;
    let credential = credential(&h);
    h.manager.set_azure_credential(credential.clone());
    let id = add_server(&h, Some("api://x/.default"), false).await;
    h.manager.connect(&id, None).await.expect("connect");

    // The server now only accepts a newer token, as if the one in use was revoked.
    reqwest::Client::new()
        .post(format!("{}/_issue", h.base))
        .send()
        .await
        .unwrap();
    let result = h
        .manager
        .call_tool(call(&id, "echo", serde_json::json!({"message": "again"})))
        .await
        .expect("call after the token was replaced");
    assert!(!result.is_error, "{result:?}");
    assert_eq!(credential.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_missing_azure_login_is_reported_before_connecting() {
    let h = harness().await;
    h.manager.set_azure_credential(Arc::new(FailingCredential));
    let id = add_server(&h, Some("api://x/.default"), false).await;
    let error = h
        .manager
        .connect(&id, None)
        .await
        .map(|_| ())
        .expect_err("must fail");
    let message = error.to_string();
    assert!(message.contains("not signed in to Azure"), "{message}");
    assert!(message.contains("az login"), "{message}");
}

#[tokio::test]
async fn without_a_configured_scope_the_server_is_asked_which_one() {
    // The test server advertises no scopes, so the user is told to enter one.
    let h = harness().await;
    h.manager.set_azure_credential(credential(&h));
    let id = add_server(&h, None, false).await;
    let error = h
        .manager
        .connect(&id, None)
        .await
        .map(|_| ())
        .expect_err("must fail");
    assert!(error.to_string().contains("scope"), "{error}");
}

#[tokio::test]
async fn oauth_and_azure_credentials_cannot_be_combined() {
    let h = harness().await;
    let id = add_server(&h, None, false).await;
    let mut input = h.registry.get(&id).await.unwrap().input;
    input.oauth = true;
    let error = h.registry.update(&id, input).await.unwrap_err();
    assert!(error.to_string().contains("cannot be combined"), "{error}");
}

async fn add_plain_server(h: &Harness, path: &str, azure: bool) -> String {
    h.registry
        .create(ServerInput {
            name: format!("Plain {path}"),
            transport: TransportKind::Http,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            cwd: None,
            url: Some(format!("{}{path}", h.base)),
            headers: BTreeMap::new(),
            tags: vec![],
            oauth: false,
            oauth_client_id: None,
            oauth_scopes: None,
            oauth_callback_port: None,
            azure_credentials: azure,
            roots: vec![],
        })
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn a_rejected_entra_server_is_explained() {
    let h = harness().await;
    let id = add_plain_server(&h, "/entra", false).await;
    let message = h
        .manager
        .connect(&id, None)
        .await
        .map(|_| ())
        .expect_err("must fail")
        .to_string();
    assert!(message.contains("Microsoft Entra ID"), "{message}");
    assert!(message.contains("Uses the Azure login"), "{message}");
}

#[tokio::test]
async fn a_rejected_server_that_is_not_entra_gets_no_advice() {
    let h = harness().await;
    let id = add_plain_server(&h, "/mcp", false).await;
    let message = h
        .manager
        .connect(&id, None)
        .await
        .map(|_| ())
        .expect_err("must fail")
        .to_string();
    assert!(message.contains("Auth required"), "{message}");
    assert!(!message.contains("Entra"), "{message}");
}
