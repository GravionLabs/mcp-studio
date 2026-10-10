//! What a server can ask of the client: sampling, elicitation and roots, against the test server.

use std::{collections::BTreeMap, sync::Arc, sync::Once, time::Duration};

use mcp_studio_core::{
    client_requests::{ClientAnswer, ClientRequestKind},
    db::Db,
    environments::Environments,
    events::CollectingSink,
    model::JsonValue,
    registry::{Registry, ServerDefinition, ServerInput, TransportKind},
    secrets::{MemoryStore, SecretStore},
    session::{SessionManager, ToolCallRequest},
};
use serde_json::json;

fn server_binary() -> String {
    static BUILD: Once = Once::new();
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
        .to_string_lossy()
        .into_owned()
}

struct Harness {
    db: Db,
    registry: Registry,
    sink: Arc<CollectingSink>,
    manager: Arc<SessionManager>,
}

async fn harness() -> Harness {
    let db = Db::open_in_memory().await.unwrap();
    let registry = Registry::new(db.clone());
    let sink = Arc::new(CollectingSink::default());
    let manager = SessionManager::new(
        db.clone(),
        registry.clone(),
        Environments::new(db.clone()),
        Arc::new(MemoryStore::default()) as Arc<dyn SecretStore>,
        sink.clone(),
    );
    Harness {
        db,
        registry,
        sink,
        manager,
    }
}

fn input(roots: Vec<String>) -> ServerInput {
    ServerInput {
        name: "test".into(),
        transport: TransportKind::Stdio,
        command: Some(server_binary()),
        args: vec![],
        env: BTreeMap::new(),
        cwd: None,
        url: None,
        headers: BTreeMap::new(),
        tags: vec![],
        oauth: false,
        oauth_client_id: None,
        oauth_scopes: None,
        oauth_callback_port: None,
        azure_credentials: false,
        roots,
    }
}

async fn connected(h: &Harness, roots: Vec<String>) -> ServerDefinition {
    let server = h.registry.create(input(roots)).await.unwrap();
    h.manager.connect(&server.id, None).await.unwrap();
    server
}

/// Calls a tool of the test server and returns the first text of its result.
async fn call(
    h: &Harness,
    server: &ServerDefinition,
    tool: &str,
    arguments: serde_json::Value,
) -> String {
    let result = h
        .manager
        .call_tool(ToolCallRequest {
            server_id: server.id.clone(),
            tool_name: tool.into(),
            arguments: JsonValue(arguments),
            environment_id: None,
            call_id: format!("call-{tool}"),
        })
        .await
        .unwrap();
    result.result.0["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// Messages are written to the database in the background.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// Waits until the server asks the user something.
async fn question(h: &Harness) -> mcp_studio_core::client_requests::ClientRequest {
    for _ in 0..400 {
        if let Some(request) = h.manager.requests().pending(None).into_iter().next() {
            return request;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the server did not ask anything");
}

async fn ask_and_answer(
    h: &Arc<Harness>,
    server: &ServerDefinition,
    tool: &str,
    arguments: serde_json::Value,
    answer: ClientAnswer,
) -> (mcp_studio_core::client_requests::ClientRequest, String) {
    let call_task = {
        let (h, server, tool) = (h.clone(), server.clone(), tool.to_owned());
        tokio::spawn(async move { call(&h, &server, &tool, arguments).await })
    };
    let request = question(h).await;
    h.manager
        .requests()
        .answer(&request.id, answer)
        .expect("answer");
    (request, call_task.await.unwrap())
}

#[tokio::test]
async fn the_client_advertises_sampling_elicitation_and_roots() {
    let h = harness().await;
    connected(&h, vec![]).await;
    settle().await;
    let payload: String = sqlx::query_scalar(
        "SELECT payload FROM messages WHERE method = 'initialize' AND direction = 'out'",
    )
    .fetch_one(h.db.pool())
    .await
    .unwrap();
    let capabilities =
        &serde_json::from_str::<serde_json::Value>(&payload).unwrap()["params"]["capabilities"];
    assert!(capabilities["sampling"].is_object(), "{capabilities}");
    assert!(
        capabilities["elicitation"]["form"].is_object(),
        "{capabilities}"
    );
    assert_eq!(capabilities["roots"]["listChanged"], true, "{capabilities}");
}

#[tokio::test]
async fn roots_are_listed_and_changes_are_announced() {
    let h = harness().await;
    let uri = if cfg!(windows) {
        "file:///C:/work/app"
    } else {
        "file:///work/app"
    };
    let server = connected(&h, vec![uri.into()]).await;
    assert_eq!(call(&h, &server, "roots", json!({})).await, uri);

    let mut edited = input(vec![uri.into(), uri.replace("app", "lib")]);
    edited.name = server.input.name.clone();
    h.registry.update(&server.id, edited).await.unwrap();
    h.manager.roots_changed(&server.id).await.unwrap();
    let roots = call(&h, &server, "roots", json!({})).await;
    assert_eq!(roots.lines().count(), 2, "{roots}");

    settle().await;
    let sent: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE method = 'notifications/roots/list_changed' AND direction = 'out'",
    )
    .fetch_one(h.db.pool())
    .await
    .unwrap();
    assert_eq!(sent, 1);
}

#[tokio::test]
async fn a_server_without_roots_gets_an_empty_list() {
    let h = harness().await;
    let server = connected(&h, vec![]).await;
    assert_eq!(call(&h, &server, "roots", json!({})).await, "");
}

#[tokio::test]
async fn elicitation_returns_what_the_user_chose() {
    let h = Arc::new(harness().await);
    let server = connected(&h, vec![]).await;

    let (request, text) = ask_and_answer(
        &h,
        &server,
        "elicit",
        json!({"question": "Your name?"}),
        ClientAnswer::Accept {
            content: JsonValue(json!({"answer": "Ada"})),
        },
    )
    .await;
    assert_eq!(request.kind, ClientRequestKind::Elicitation);
    assert_eq!(request.params.0["message"], "Your name?");
    assert!(request.params.0["requestedSchema"]["properties"]["answer"].is_object());
    assert_eq!(text, r#"accept: {"answer":"Ada"}"#);

    let (_, text) = ask_and_answer(
        &h,
        &server,
        "elicit",
        json!({"question": "?"}),
        ClientAnswer::Decline,
    )
    .await;
    assert_eq!(text, "decline");
    let (_, text) = ask_and_answer(
        &h,
        &server,
        "elicit",
        json!({"question": "?"}),
        ClientAnswer::Cancel,
    )
    .await;
    assert_eq!(text, "cancel");
    assert!(h.manager.requests().pending(None).is_empty());
}

#[tokio::test]
async fn sampling_returns_the_answer_the_user_wrote_or_an_error() {
    let h = Arc::new(harness().await);
    let server = connected(&h, vec![]).await;

    let (request, text) = ask_and_answer(
        &h,
        &server,
        "sample",
        json!({"prompt": "Say hi"}),
        ClientAnswer::Respond {
            text: "hi there".into(),
            model: None,
        },
    )
    .await;
    assert_eq!(request.kind, ClientRequestKind::Sampling);
    assert_eq!(request.params.0["messages"][0]["content"]["text"], "Say hi");
    assert_eq!(request.params.0["systemPrompt"], "You answer briefly.");
    assert_eq!(text, "mcp-studio-manual said: hi there");

    let (_, text) = ask_and_answer(
        &h,
        &server,
        "sample",
        json!({"prompt": "x"}),
        ClientAnswer::Reject,
    )
    .await;
    assert!(text.contains("rejected"), "{text}");
}

#[tokio::test]
async fn questions_and_answers_are_recorded_in_the_inspector() {
    let h = Arc::new(harness().await);
    let server = connected(&h, vec![]).await;
    ask_and_answer(
        &h,
        &server,
        "elicit",
        json!({"question": "?"}),
        ClientAnswer::Decline,
    )
    .await;
    settle().await;
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT direction, method FROM messages WHERE method = 'elicitation/create' OR (direction = 'out' AND method IS NULL AND payload LIKE '%\"action\":\"decline\"%')",
    )
    .fetch_all(h.db.pool())
    .await
    .unwrap();
    assert!(
        rows.iter()
            .any(|(d, m)| d == "in" && m.as_deref() == Some("elicitation/create")),
        "{rows:?}"
    );
    assert!(rows.iter().any(|(d, _)| d == "out"), "{rows:?}");
    assert!(!h.sink.client_requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_disconnect_withdraws_open_questions() {
    let h = Arc::new(harness().await);
    let server = connected(&h, vec![]).await;
    let call_task = {
        let (h, server) = (h.clone(), server.clone());
        tokio::spawn(async move {
            h.manager
                .call_tool(ToolCallRequest {
                    server_id: server.id.clone(),
                    tool_name: "elicit".into(),
                    arguments: JsonValue(json!({"question": "?"})),
                    environment_id: None,
                    call_id: "c".into(),
                })
                .await
        })
    };
    question(&h).await;
    h.manager.disconnect(&server.id).await.unwrap();
    let _ = call_task.await;
    assert!(h.manager.requests().pending(None).is_empty());
    assert_eq!(h.sink.client_requests_done.lock().unwrap().len(), 1);
}
