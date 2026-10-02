//! OAuth 2.1 end to end against the mock authorization server in `mcp-studio-testserver --oauth`.

use std::{collections::BTreeMap, process::Stdio, sync::Arc, time::Duration};

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
    secrets: Arc<MemoryStore>,
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
        secrets.clone() as Arc<dyn SecretStore>,
        Arc::new(CollectingSink::default()),
    );
    manager.set_connect_timeout(Duration::from_secs(10));
    Harness {
        registry,
        secrets,
        manager,
        base,
        _server: child,
    }
}

async fn add_server(h: &Harness, oauth: bool) -> String {
    h.registry
        .create(ServerInput {
            name: "Protected".into(),
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
            oauth_scopes: None,
            oauth_callback_port: None,
        })
        .await
        .unwrap()
        .id
}

async fn stats(h: &Harness) -> serde_json::Value {
    reqwest::get(format!("{}/_stats", h.base))
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// Plays the browser: opens the authorization URL and follows the redirect back to the app.
fn browser() -> impl Fn(&str) -> Result<(), String> + Send + Sync {
    |url: &str| {
        let url = url.to_owned();
        tokio::spawn(async move {
            let response = reqwest::get(&url).await.expect("authorization page");
            assert!(
                response.status().is_success(),
                "callback page: {}",
                response.status()
            );
        });
        Ok(())
    }
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
async fn connecting_without_signing_in_says_so() {
    let h = harness().await;
    let id = add_server(&h, true).await;
    assert!(!h.manager.is_signed_in(&id).await);
    let error = h
        .manager
        .connect(&id, None)
        .await
        .map(|_| ())
        .expect_err("must fail");
    assert!(error.to_string().contains("sign-in required"), "{error}");
}

#[tokio::test]
async fn a_server_without_the_oauth_flag_is_rejected_by_sign_in() {
    let h = harness().await;
    let id = add_server(&h, false).await;
    let error = h.manager.sign_in(&id, &browser()).await.unwrap_err();
    assert!(
        error.to_string().contains("not configured for OAuth"),
        "{error}"
    );
}

#[tokio::test]
async fn a_registered_client_signs_in_without_dynamic_registration() {
    let h = harness().await;
    let id = add_server(&h, true).await;
    let mut input = h.registry.get(&id).await.unwrap().input;
    input.oauth_client_id = Some("my-entra-app".into());
    input.oauth_scopes = Some("api://x/.default offline_access".into());
    h.registry.update(&id, input).await.unwrap();

    h.manager.sign_in(&id, &browser()).await.expect("sign in");
    assert!(h.manager.is_signed_in(&id).await);

    let stats = stats(&h).await;
    assert_eq!(stats["registrations"], 0);
    assert_eq!(stats["lastClientId"], "my-entra-app");
    let scope = stats["lastScope"].as_str().unwrap();
    assert!(scope.contains("api://x/.default"), "{scope}");
    h.manager
        .connect(&id, None)
        .await
        .expect("connect with the stored token");
}

#[tokio::test]
async fn sign_in_connect_call_refresh_and_sign_out() {
    let h = harness().await;
    let id = add_server(&h, true).await;

    // Sign in through the "browser": discovery, dynamic registration, PKCE, code exchange.
    h.manager.sign_in(&id, &browser()).await.expect("sign in");
    assert!(h.manager.is_signed_in(&id).await);
    assert!(
        h.secrets
            .get(&format!("oauth/{id}"))
            .unwrap()
            .is_some_and(|json| json.contains("refresh-1")),
        "tokens are stored in the keyring"
    );
    assert_eq!(stats(&h).await["codesIssued"], 1);

    // The stored token opens the protected MCP endpoint.
    let session = h
        .manager
        .connect(&id, None)
        .await
        .expect("connect with the stored token");
    assert!(session
        .peer
        .list_all_tools()
        .await
        .unwrap()
        .iter()
        .any(|t| t.name == "echo"));
    let result = h
        .manager
        .call_tool(call(&id, "add", serde_json::json!({"a": 1, "b": 2})))
        .await
        .unwrap();
    assert_eq!(result.result.0["content"][0]["text"], "3");
    h.manager.disconnect(&id).await.unwrap();

    // Access tokens live two seconds. Reconnecting afterwards must refresh transparently.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let again = h
        .manager
        .connect(&id, None)
        .await
        .expect("connect after the token expired");
    assert!(again
        .peer
        .list_all_tools()
        .await
        .unwrap()
        .iter()
        .any(|t| t.name == "fail"));
    assert!(
        stats(&h).await["refreshes"].as_u64().unwrap() >= 1,
        "the token was refreshed"
    );
    h.manager.disconnect(&id).await.unwrap();

    // Signing out forgets everything.
    h.manager.sign_out(&id).await.unwrap();
    assert!(!h.manager.is_signed_in(&id).await);
    assert!(h.secrets.get(&format!("oauth/{id}")).unwrap().is_none());
    assert!(h.manager.connect(&id, None).await.is_err());
}
