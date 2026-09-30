//! M0 spike: connect to the reference server over stdio and Streamable HTTP through the
//! `RecordingTransport` and check that every message is recorded.

use std::{
    process::Stdio,
    sync::{Arc, Mutex, Once},
};

use mcp_studio_core::recording::{Direction, RecordedMessage, RecordingTransport};
use rmcp::{
    model::{CallToolRequestParams, ClientConfig},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, ConfigureCommandExt,
        StreamableHttpClientTransport, TokioChildProcess,
    },
    RoleClient, ServiceExt,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

fn server_binary() -> std::path::PathBuf {
    static BUILD: Once = Once::new();
    BUILD.call_once(|| {
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "-p", "mcp-studio-testserver"])
            .status()
            .expect("cargo build");
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
}

fn collector() -> (
    Arc<Mutex<Vec<RecordedMessage>>>,
    Arc<dyn mcp_studio_core::recording::Recorder>,
) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let sink = messages.clone();
    let recorder: Arc<dyn mcp_studio_core::recording::Recorder> =
        Arc::new(move |m: RecordedMessage| sink.lock().unwrap().push(m));
    (messages, recorder)
}

fn methods(messages: &[RecordedMessage], direction: Direction) -> Vec<String> {
    messages
        .iter()
        .filter(|m| m.direction == direction)
        .filter_map(|m| m.payload["method"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn stdio_session_is_recorded() {
    let child = TokioChildProcess::new(Command::new(server_binary()).configure(|_| {})).unwrap();
    let (messages, recorder) = collector();
    let transport = RecordingTransport::<_, RoleClient>::new(child, recorder);

    let client = ClientConfig::default()
        .serve(transport)
        .await
        .expect("initialize");
    let tools = client.list_tools(None).await.expect("list tools");
    assert!(tools.tools.iter().any(|t| t.name == "echo"));
    let result = client
        .call_tool(
            CallToolRequestParams::new("add").with_arguments(
                serde_json::json!({"a": 2, "b": 3})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .expect("call tool");
    assert_ne!(result.is_error, Some(true));
    client.cancel().await.ok();

    let messages = messages.lock().unwrap().clone();
    let out = methods(&messages, Direction::Out);
    assert!(out.contains(&"initialize".to_owned()), "{out:?}");
    assert!(out.contains(&"tools/list".to_owned()), "{out:?}");
    assert!(out.contains(&"tools/call".to_owned()), "{out:?}");
    // Every request has a response recorded as incoming message with the same id.
    let responses = messages
        .iter()
        .filter(|m| m.direction == Direction::In && m.payload.get("result").is_some())
        .count();
    assert!(responses >= 3, "responses: {responses}");
    assert!(messages.iter().all(|m| m.bytes > 0 && m.ts > 0));
    let call_response = messages
        .iter()
        .find(|m| m.direction == Direction::In && m.payload["result"]["content"][0]["text"] == "5");
    assert!(
        call_response.is_some(),
        "add result not recorded: {messages:#?}"
    );
}

#[tokio::test]
async fn http_session_is_recorded() {
    let mut child = Command::new(server_binary())
        .args(["--http", "0"])
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let first = lines
        .next_line()
        .await
        .unwrap()
        .expect("server prints its address");
    let url = first.trim_start_matches("listening on ").to_owned();

    let (messages, recorder) = collector();
    let transport = RecordingTransport::<_, RoleClient>::new(
        StreamableHttpClientTransport::from_config(StreamableHttpClientTransportConfig::with_uri(
            url,
        )),
        recorder,
    );
    let client = ClientConfig::default()
        .serve(transport)
        .await
        .expect("initialize");
    let tools = client.list_tools(None).await.expect("list tools");
    assert!(tools.tools.iter().any(|t| t.name == "fail"));
    client.cancel().await.ok();

    let messages = messages.lock().unwrap().clone();
    let out = methods(&messages, Direction::Out);
    assert!(out.contains(&"initialize".to_owned()), "{out:?}");
    assert!(out.contains(&"tools/list".to_owned()), "{out:?}");
    assert!(messages
        .iter()
        .any(|m| m.direction == Direction::In && m.payload.get("result").is_some()));
}
