//! Routing a client's servers through MCP Studio and putting them back.

use std::path::{Path, PathBuf};

use mcp_studio_core::{
    client_formats::{ClientFormat, ClientSource},
    client_routes::{self, EntryKind, EntryRef, RouteContext},
    db::Db,
    registry::{Registry, TransportKind},
    secrets::{MemoryStore, SecretStore},
};
use serde_json::{json, Value};

const PROXY: &str = "/opt/mcp-studio/mcp-studio-proxy";
const PORT: u16 = 38465;

const CLAUDE_CODE: &str = r#"{
  "numStartups": 42,
  "mcpServers": {
    "files": {
      "type": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
      "env": {}
    },
    "github": {
      "command": "docker",
      "args": ["run", "-i", "ghcr.io/github/github-mcp-server"],
      "env": {"GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_secret", "LOG_LEVEL": "info"}
    },
    "remote": {"type": "http", "url": "https://example.com/mcp", "headers": {"Authorization": "Bearer abc"}},
    "legacy": {"type": "sse", "url": "https://example.com/sse"}
  },
  "projects": {
    "/home/me/app": {"allowedTools": [], "mcpServers": {"db": {"command": "uvx", "args": ["db-server"]}}}
  }
}
"#;

struct Fixture {
    dir: tempfile::TempDir,
    db: Db,
    registry: Registry,
    secrets: MemoryStore,
    sources: Vec<ClientSource>,
}

impl Fixture {
    async fn new(content: &str) -> Self {
        Self::for_client(
            content,
            "Claude Code (user)",
            ".claude.json",
            ClientFormat::Claude,
        )
        .await
    }

    async fn for_client(content: &str, label: &str, file: &str, format: ClientFormat) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(file);
        std::fs::write(&file, content).unwrap();
        let db = Db::open_in_memory().await.unwrap();
        Self {
            registry: Registry::new(db.clone()),
            db,
            secrets: MemoryStore::default(),
            sources: vec![ClientSource {
                label: label.into(),
                path: file.to_string_lossy().into_owned(),
                exists: true,
                format,
            }],
            dir,
        }
    }

    fn ctx(&self, proxy: Option<&str>) -> RouteContext<'_> {
        RouteContext {
            db: &self.db,
            registry: &self.registry,
            secrets: &self.secrets,
            proxy_binary: proxy.map(PathBuf::from),
            http_proxy_port: PORT,
            backup_dir: self.dir.path().join("backups"),
        }
    }

    fn path(&self) -> &Path {
        Path::new(&self.sources[0].path)
    }

    fn read(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.path()).unwrap()).unwrap()
    }

    fn entry(&self, name: &str) -> EntryRef {
        EntryRef {
            path: self.sources[0].path.clone(),
            pointer: "/mcpServers".into(),
            name: name.into(),
        }
    }

    fn entry_at(&self, pointer: &str, name: &str) -> EntryRef {
        EntryRef {
            path: self.sources[0].path.clone(),
            pointer: pointer.into(),
            name: name.into(),
        }
    }

    fn rewrite(&self, f: impl FnOnce(&mut Value)) {
        let mut value = self.read();
        f(&mut value);
        // Make sure the change shows in the modification time even on coarse file systems.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(self.path(), serde_json::to_string_pretty(&value).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn lists_the_servers_of_every_scope_and_whether_they_are_routed() {
    let fx = Fixture::new(CLAUDE_CODE).await;

    let entries = client_routes::list_entries(&fx.ctx(Some(PROXY)), &fx.sources)
        .await
        .unwrap()
        .entries;

    let by_name = |name: &str| entries.iter().find(|e| e.entry.name == name).unwrap();
    assert_eq!(entries.len(), 5);
    assert_eq!(by_name("files").kind, EntryKind::Stdio);
    assert_eq!(
        by_name("files").summary,
        "npx -y @modelcontextprotocol/server-filesystem /tmp"
    );
    assert_eq!(by_name("remote").kind, EntryKind::Http);
    assert_eq!(by_name("legacy").kind, EntryKind::Unsupported);
    assert!(by_name("legacy").unsupported.is_some());
    assert_eq!(by_name("db").origin, "project /home/me/app");
    assert_eq!(
        by_name("db").entry.pointer,
        "/projects/~1home~1me~1app/mcpServers"
    );
    assert!(entries.iter().all(|e| !e.routed));
}

#[tokio::test]
async fn routing_a_stdio_server_runs_it_through_the_proxy_and_registers_it() {
    let fx = Fixture::new(CLAUDE_CODE).await;

    let result = client_routes::route(&fx.ctx(Some(PROXY)), &fx.sources, &fx.entry("files"))
        .await
        .unwrap();

    let servers = fx.registry.list().await.unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].input.name, "files");
    assert_eq!(servers[0].input.transport, TransportKind::Stdio);
    assert_eq!(result.server_name, "files");
    let file = fx.read();
    assert_eq!(
        file["mcpServers"]["files"],
        json!({"type": "stdio", "command": PROXY, "args": ["--server", servers[0].id]})
    );
    assert!(Path::new(&result.backup_path).is_file());
    let entries = client_routes::list_entries(&fx.ctx(Some(PROXY)), &fx.sources)
        .await
        .unwrap()
        .entries;
    let routed = entries.iter().find(|e| e.entry.name == "files").unwrap();
    assert!(routed.routed);
    assert_eq!(routed.route_id.as_deref(), Some(result.route_id.as_str()));
}

#[tokio::test]
async fn routing_changes_only_that_entry() {
    let fx = Fixture::new(CLAUDE_CODE).await;
    let before = fx.read();

    client_routes::route(&fx.ctx(Some(PROXY)), &fx.sources, &fx.entry("files"))
        .await
        .unwrap();

    let mut after = fx.read();
    assert_eq!(after["numStartups"], 42);
    assert_eq!(
        after["mcpServers"]["github"],
        before["mcpServers"]["github"]
    );
    assert_eq!(after["projects"], before["projects"]);
    // Everything except the routed entry is as it was.
    after["mcpServers"]["files"] = before["mcpServers"]["files"].clone();
    assert_eq!(after, before);
    let text = std::fs::read_to_string(fx.path()).unwrap();
    assert!(text.ends_with('\n'));
    assert!(text.contains("\n  \"mcpServers\""), "indentation is kept");
}

#[tokio::test]
async fn routing_an_http_server_points_it_at_the_local_proxy() {
    let fx = Fixture::new(CLAUDE_CODE).await;

    client_routes::route(&fx.ctx(None), &fx.sources, &fx.entry("remote"))
        .await
        .unwrap();

    let servers = fx.registry.list().await.unwrap();
    assert_eq!(
        servers[0].input.url.as_deref(),
        Some("https://example.com/mcp")
    );
    assert_eq!(
        fx.read()["mcpServers"]["remote"],
        json!({"type": "http", "url": format!("http://127.0.0.1:{PORT}/mcp/{}", servers[0].id)})
    );
}

#[tokio::test]
async fn secrets_move_into_the_keyring_and_not_into_the_new_entry() {
    let fx = Fixture::new(CLAUDE_CODE).await;

    let result = client_routes::route(&fx.ctx(Some(PROXY)), &fx.sources, &fx.entry("github"))
        .await
        .unwrap();

    assert_eq!(result.secrets_moved, 1);
    let server = &fx.registry.list().await.unwrap()[0];
    let reference = &server.input.env["GITHUB_PERSONAL_ACCESS_TOKEN"];
    assert!(reference.starts_with("keyring:imported/"), "{reference}");
    assert_eq!(
        fx.secrets
            .get(reference.trim_start_matches("keyring:"))
            .unwrap()
            .as_deref(),
        Some("ghp_secret")
    );
    assert!(!std::fs::read_to_string(fx.path())
        .unwrap()
        .contains("ghp_secret"));
    // The backup keeps the original text, so it must only be readable by the user.
    let backup = std::fs::read_to_string(&result.backup_path).unwrap();
    assert!(backup.contains("ghp_secret"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&result.backup_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
    }
}

#[tokio::test]
async fn a_server_that_is_already_registered_is_reused() {
    let fx = Fixture::new(CLAUDE_CODE).await;
    let first = client_routes::route(&fx.ctx(Some(PROXY)), &fx.sources, &fx.entry("files"))
        .await
        .unwrap();
    client_routes::unroute(&fx.db, &first.route_id, false)
        .await
        .unwrap();

    let again = client_routes::route(&fx.ctx(Some(PROXY)), &fx.sources, &fx.entry("files"))
        .await
        .unwrap();

    assert_eq!(fx.registry.list().await.unwrap().len(), 1);
    assert_eq!(again.server_name, "files");
}

#[tokio::test]
async fn undoing_puts_the_original_entry_back() {
    let fx = Fixture::new(CLAUDE_CODE).await;
    let before = fx.read();
    let result = client_routes::route(&fx.ctx(Some(PROXY)), &fx.sources, &fx.entry("github"))
        .await
        .unwrap();

    let undone = client_routes::unroute(&fx.db, &result.route_id, false)
        .await
        .unwrap();

    assert!(undone.restored);
    assert_eq!(undone.conflict, None);
    assert_eq!(fx.read(), before);
    assert!(
        !Path::new(&result.backup_path).exists(),
        "the backup is removed"
    );
    let entries = client_routes::list_entries(&fx.ctx(Some(PROXY)), &fx.sources)
        .await
        .unwrap()
        .entries;
    assert!(entries.iter().all(|e| !e.routed && e.route_id.is_none()));
}

#[tokio::test]
async fn project_entries_are_routed_and_restored_in_place() {
    let fx = Fixture::new(CLAUDE_CODE).await;
    let before = fx.read();
    let target = EntryRef {
        path: fx.sources[0].path.clone(),
        pointer: "/projects/~1home~1me~1app/mcpServers".into(),
        name: "db".into(),
    };

    let result = client_routes::route(&fx.ctx(Some(PROXY)), &fx.sources, &target)
        .await
        .unwrap();
    assert_eq!(
        fx.read()["projects"]["/home/me/app"]["mcpServers"]["db"]["command"],
        PROXY
    );
    client_routes::unroute(&fx.db, &result.route_id, false)
        .await
        .unwrap();

    assert_eq!(fx.read(), before);
}

#[tokio::test]
async fn an_entry_that_was_edited_since_is_not_overwritten_unless_forced() {
    let fx = Fixture::new(CLAUDE_CODE).await;
    let result = client_routes::route(&fx.ctx(Some(PROXY)), &fx.sources, &fx.entry("files"))
        .await
        .unwrap();
    fx.rewrite(|v| v["mcpServers"]["files"]["args"] = json!(["--server", "other"]));
    let edited = fx.read();

    let refused = client_routes::unroute(&fx.db, &result.route_id, false)
        .await
        .unwrap();

    assert!(!refused.restored);
    assert!(refused.conflict.unwrap().contains("changed"));
    assert_eq!(fx.read(), edited, "nothing was written");

    let forced = client_routes::unroute(&fx.db, &result.route_id, true)
        .await
        .unwrap();
    assert!(forced.restored);
    assert_eq!(fx.read()["mcpServers"]["files"]["command"], "npx");
}

#[tokio::test]
async fn an_entry_that_was_removed_since_is_reported() {
    let fx = Fixture::new(CLAUDE_CODE).await;
    let result = client_routes::route(&fx.ctx(Some(PROXY)), &fx.sources, &fx.entry("files"))
        .await
        .unwrap();
    fx.rewrite(|v| {
        v["mcpServers"].as_object_mut().unwrap().remove("files");
    });

    let refused = client_routes::unroute(&fx.db, &result.route_id, false)
        .await
        .unwrap();

    assert!(refused.conflict.unwrap().contains("no longer"));
    assert!(
        client_routes::unroute(&fx.db, &result.route_id, true)
            .await
            .unwrap()
            .restored
    );
    assert_eq!(fx.read()["mcpServers"]["files"]["command"], "npx");
}

#[tokio::test]
async fn routing_needs_the_proxy_program_for_stdio_servers_and_changes_nothing_without_it() {
    let fx = Fixture::new(CLAUDE_CODE).await;
    let before = std::fs::read_to_string(fx.path()).unwrap();

    let error = client_routes::route(&fx.ctx(None), &fx.sources, &fx.entry("files"))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("mcp-studio-proxy"), "{error}");
    assert_eq!(std::fs::read_to_string(fx.path()).unwrap(), before);
    assert!(fx.registry.list().await.unwrap().is_empty());
    assert!(!fx.dir.path().join("backups").exists());
}

#[tokio::test]
async fn unsupported_and_already_routed_entries_are_refused() {
    let fx = Fixture::new(CLAUDE_CODE).await;

    let legacy = client_routes::route(&fx.ctx(Some(PROXY)), &fx.sources, &fx.entry("legacy"))
        .await
        .unwrap_err();
    assert!(legacy.to_string().contains("SSE"), "{legacy}");

    client_routes::route(&fx.ctx(Some(PROXY)), &fx.sources, &fx.entry("files"))
        .await
        .unwrap();
    let twice = client_routes::route(&fx.ctx(Some(PROXY)), &fx.sources, &fx.entry("files"))
        .await
        .unwrap_err();
    assert!(twice.to_string().contains("already"), "{twice}");
}

#[tokio::test]
async fn only_known_configuration_files_can_be_changed() {
    let fx = Fixture::new(CLAUDE_CODE).await;
    let other = fx.dir.path().join("other.json");
    std::fs::write(&other, CLAUDE_CODE).unwrap();
    let target = EntryRef {
        path: other.to_string_lossy().into_owned(),
        pointer: "/mcpServers".into(),
        name: "files".into(),
    };

    let error = client_routes::route(&fx.ctx(Some(PROXY)), &fx.sources, &target)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("not a known"), "{error}");
    assert_eq!(std::fs::read_to_string(&other).unwrap(), CLAUDE_CODE);
}

#[tokio::test]
async fn a_preview_shows_before_and_after_and_changes_nothing() {
    let fx = Fixture::new(CLAUDE_CODE).await;
    let before = std::fs::read_to_string(fx.path()).unwrap();

    let preview = client_routes::preview(&fx.ctx(Some(PROXY)), &fx.sources, &fx.entry("github"))
        .await
        .unwrap();

    assert!(preview.before.contains("ghp_secret") || preview.before.contains("docker"));
    assert!(preview.after.contains(PROXY));
    assert!(preview.after.contains("--server"));
    assert_eq!(preview.secrets, ["GITHUB_PERSONAL_ACCESS_TOKEN"]);
    assert_eq!(preview.existing_server, None);
    assert_eq!(std::fs::read_to_string(fx.path()).unwrap(), before);
    assert!(fx.registry.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn an_edit_made_before_routing_is_kept() {
    let fx = Fixture::new(CLAUDE_CODE).await;
    fx.rewrite(|v| v["numStartups"] = json!(43));

    client_routes::route(&fx.ctx(Some(PROXY)), &fx.sources, &fx.entry("files"))
        .await
        .unwrap();

    assert_eq!(fx.read()["numStartups"], 43);
}

const VSCODE: &str = r#"{
  "servers": {
    "files": {"type": "stdio", "command": "npx", "args": ["-y", "server-files"]},
    "remote": {"type": "http", "url": "https://example.com/mcp", "headers": {"X-Api-Key": "k-123"}},
    "input": {"type": "stdio", "command": "npx", "args": ["${workspaceFolder}"]}
  },
  "inputs": [{"id": "x", "type": "promptString", "description": "x"}]
}
"#;

const COPILOT: &str = r#"{
  "mcpServers": {
    "files": {"type": "local", "command": "npx", "args": ["-y", "server-files"], "env": {"A": "1"}, "tools": ["*"]},
    "remote": {"type": "http", "url": "https://example.com/mcp", "tools": ["*"]}
  }
}
"#;

const OPENCODE: &str = r#"{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "files": {"type": "local", "command": ["npx", "-y", "server-files"], "environment": {"A": "1"}, "enabled": true},
    "remote": {"type": "remote", "url": "https://example.com/mcp", "headers": {"X": "y"}, "enabled": true}
  }
}
"#;

/// Routes and restores every routable entry of a file and checks the file is as it was.
async fn round_trip(fx: &Fixture, pointer: &str, names: &[&str]) -> Vec<Value> {
    let before = fx.read();
    let mut routed = Vec::new();
    let mut ids = Vec::new();
    for name in names {
        let result = client_routes::route(
            &fx.ctx(Some(PROXY)),
            &fx.sources,
            &fx.entry_at(pointer, name),
        )
        .await
        .unwrap();
        ids.push(result.route_id);
        routed.push(fx.read()[pointer.trim_start_matches('/')][*name].clone());
    }
    for id in ids {
        assert!(
            client_routes::unroute(&fx.db, &id, false)
                .await
                .unwrap()
                .restored
        );
    }
    assert_eq!(fx.read(), before, "the file is as it was");
    routed
}

#[tokio::test]
async fn vs_code_servers_are_routed_and_restored() {
    let fx = Fixture::for_client(VSCODE, "VS Code (user)", "mcp.json", ClientFormat::VsCode).await;

    let routed = round_trip(&fx, "/servers", &["files", "remote"]).await;

    assert_eq!(routed[0]["type"], "stdio");
    assert_eq!(routed[0]["command"], PROXY);
    assert_eq!(routed[0]["args"][0], "--server");
    assert_eq!(routed[1]["type"], "http");
    assert!(routed[1]["url"]
        .as_str()
        .unwrap()
        .starts_with("http://127.0.0.1:38465/mcp/"));
    assert!(
        routed[1].get("headers").is_none(),
        "headers live in MCP Studio now"
    );
}

#[tokio::test]
async fn vs_code_entries_with_variables_are_listed_but_not_routable() {
    let fx = Fixture::for_client(VSCODE, "VS Code (user)", "mcp.json", ClientFormat::VsCode).await;

    let entries = client_routes::list_entries(&fx.ctx(Some(PROXY)), &fx.sources)
        .await
        .unwrap()
        .entries;
    let input = entries.iter().find(|e| e.entry.name == "input").unwrap();
    assert_eq!(input.kind, EntryKind::Unsupported);
    assert!(input.unsupported.as_deref().unwrap().contains("${"));

    let error = client_routes::route(
        &fx.ctx(Some(PROXY)),
        &fx.sources,
        &fx.entry_at("/servers", "input"),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("variables"), "{error}");
}

#[tokio::test]
async fn copilot_cli_servers_keep_their_tool_lists() {
    let fx = Fixture::for_client(
        COPILOT,
        "Copilot CLI",
        "mcp-config.json",
        ClientFormat::CopilotCli,
    )
    .await;
    let before = fx.read();

    let result = client_routes::route(
        &fx.ctx(Some(PROXY)),
        &fx.sources,
        &fx.entry_at("/mcpServers", "files"),
    )
    .await
    .unwrap();

    let routed = fx.read()["mcpServers"]["files"].clone();
    assert_eq!(routed["type"], "local");
    assert_eq!(routed["tools"], json!(["*"]));
    assert_eq!(routed["command"], PROXY);
    assert!(routed.get("env").is_none());
    client_routes::unroute(&fx.db, &result.route_id, false)
        .await
        .unwrap();
    assert_eq!(fx.read(), before);
    round_trip(&fx, "/mcpServers", &["files", "remote"]).await;
}

#[tokio::test]
async fn opencode_servers_use_one_command_array() {
    let fx = Fixture::for_client(
        OPENCODE,
        "OpenCode",
        "opencode.json",
        ClientFormat::OpenCode,
    )
    .await;

    let routed = round_trip(&fx, "/mcp", &["files", "remote"]).await;

    assert_eq!(routed[0]["type"], "local");
    assert_eq!(routed[0]["command"][0], PROXY);
    assert_eq!(routed[0]["command"][1], "--server");
    assert_eq!(routed[0]["enabled"], true);
    assert!(routed[0].get("environment").is_none());
    assert_eq!(routed[1]["type"], "remote");
    assert_eq!(routed[1]["enabled"], true);
    let servers = fx.registry.list().await.unwrap();
    let files = servers.iter().find(|s| s.input.name == "files").unwrap();
    assert_eq!(files.input.command.as_deref(), Some("npx"));
    assert_eq!(files.input.args, ["-y", "server-files"]);
    assert_eq!(files.input.env["A"], "1");
}

#[tokio::test]
async fn a_file_that_cannot_be_read_is_reported_instead_of_hidden() {
    // VS Code allows comments in mcp.json; that is not JSON.
    let fx = Fixture::for_client(
        "{\n  // my servers\n  \"servers\": {}\n}\n",
        "VS Code (user)",
        "mcp.json",
        ClientFormat::VsCode,
    )
    .await;

    let listed = client_routes::list_entries(&fx.ctx(Some(PROXY)), &fx.sources)
        .await
        .unwrap();

    assert!(listed.entries.is_empty());
    assert_eq!(listed.unreadable.len(), 1);
    assert_eq!(listed.unreadable[0].client, "VS Code (user)");
    assert!(listed.unreadable[0].reason.contains("not valid JSON"));
}
