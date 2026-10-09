//! Routing a client's servers through MCP Studio and putting them back.

use std::path::{Path, PathBuf};

use mcp_studio_core::{
    client_import::ConfigSource,
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
    sources: Vec<ConfigSource>,
}

impl Fixture {
    async fn new(content: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".claude.json");
        std::fs::write(&file, content).unwrap();
        let db = Db::open_in_memory().await.unwrap();
        Self {
            registry: Registry::new(db.clone()),
            db,
            secrets: MemoryStore::default(),
            sources: vec![ConfigSource {
                label: "Claude Code (user)".into(),
                path: file.to_string_lossy().into_owned(),
                exists: true,
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
        .unwrap();

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
        .unwrap();
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
        .unwrap();
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
