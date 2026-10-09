//! Import server definitions from other MCP clients' configuration files.
//!
//! Supported: Claude Desktop (`claude_desktop_config.json`), Claude Code (`~/.claude.json` and
//! project `.mcp.json`), and any file with the common `{"mcpServers": {...}}` shape.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::{
    db::{DbError, DbResult},
    registry::{Registry, ServerDefinition, ServerInput, TransportKind},
    secrets::{reference, SecretStore},
};

/// A configuration file that may contain servers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConfigSource {
    /// "Claude Desktop" or "Claude Code (user)".
    pub label: String,
    pub path: String,
    pub exists: bool,
}

/// One server found in a file, ready to be previewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportCandidate {
    pub input: ServerInput,
    /// Where it was defined, e.g. `Claude Code · project /home/me/app`.
    pub origin: String,
    /// The same name (or the same command/URL) already exists in MCP Studio.
    pub duplicate: bool,
    /// Why it cannot be imported (e.g. legacy SSE transport); such entries are not selectable.
    pub unsupported: Option<String>,
}

/// Result of an import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub created: u32,
    pub renamed: u32,
    /// Values (tokens, API keys) that were moved from the config into the OS keyring.
    pub secrets_moved: u32,
    pub failed: Vec<String>,
}

/// Locations of well-known config files for the current user.
pub fn detect_sources(home: Option<&Path>, app_data: Option<&Path>) -> Vec<ConfigSource> {
    let mut candidates: Vec<(String, PathBuf)> = Vec::new();
    if let Some(home) = home {
        candidates.push(("Claude Code (user)".into(), home.join(".claude.json")));
    }
    if cfg!(target_os = "macos") {
        if let Some(home) = home {
            candidates.push((
                "Claude Desktop".into(),
                home.join("Library/Application Support/Claude/claude_desktop_config.json"),
            ));
        }
    } else if cfg!(windows) {
        if let Some(app_data) = app_data {
            candidates.push((
                "Claude Desktop".into(),
                app_data.join("Claude/claude_desktop_config.json"),
            ));
        }
    } else if let Some(home) = home {
        candidates.push((
            "Claude Desktop".into(),
            home.join(".config/Claude/claude_desktop_config.json"),
        ));
    }
    candidates
        .into_iter()
        .map(|(label, path)| ConfigSource {
            exists: path.is_file(),
            path: path.to_string_lossy().into_owned(),
            label,
        })
        .collect()
}

/// Reads a config file and lists the servers in it. `existing` (name, command/url signature) marks
/// duplicates.
pub fn parse_config(json: &str, existing: &[ServerInput]) -> DbResult<Vec<ImportCandidate>> {
    let root: Value = serde_json::from_str(json)
        .map_err(|e| DbError::Invalid(format!("not a JSON file: {e}")))?;
    let mut found = Vec::new();

    collect(root.get("mcpServers"), "top level", &mut found);
    // Claude Code keeps project scoped servers under `projects.<path>.mcpServers`.
    if let Some(projects) = root.get("projects").and_then(Value::as_object) {
        for (path, project) in projects {
            collect(
                project.get("mcpServers"),
                &format!("project {path}"),
                &mut found,
            );
        }
    }
    if found.is_empty() && root.get("mcpServers").is_none() && root.get("projects").is_none() {
        return Err(DbError::Invalid(
            "no \"mcpServers\" found in this file".into(),
        ));
    }

    Ok(found
        .into_iter()
        .map(|(name, origin, entry)| {
            let (input, unsupported) = to_input(&name, &entry);
            let duplicate = existing.iter().any(|e| is_duplicate(e, &input));
            ImportCandidate {
                input,
                origin,
                duplicate,
                unsupported,
            }
        })
        .collect())
}

fn collect(servers: Option<&Value>, origin: &str, out: &mut Vec<(String, String, Value)>) {
    if let Some(map) = servers.and_then(Value::as_object) {
        for (name, entry) in map {
            out.push((name.clone(), origin.to_owned(), entry.clone()));
        }
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn string_map(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn to_input(name: &str, entry: &Value) -> (ServerInput, Option<String>) {
    let kind = entry.get("type").and_then(Value::as_str).unwrap_or("");
    let is_remote =
        entry.get("url").is_some() || matches!(kind, "http" | "sse" | "streamable-http");
    let mut unsupported = None;
    if kind == "sse" {
        unsupported =
            Some("legacy SSE transport is not supported; use a Streamable HTTP URL".to_owned());
    } else if !is_remote && entry.get("command").and_then(Value::as_str).is_none() {
        unsupported = Some("neither a command nor a URL is defined".to_owned());
    }
    let input = ServerInput {
        name: name.trim().to_owned(),
        transport: if is_remote {
            TransportKind::Http
        } else {
            TransportKind::Stdio
        },
        command: entry
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_owned),
        args: strings(entry.get("args")),
        env: string_map(entry.get("env")),
        cwd: entry.get("cwd").and_then(Value::as_str).map(str::to_owned),
        url: entry.get("url").and_then(Value::as_str).map(str::to_owned),
        headers: string_map(entry.get("headers")),
        tags: vec!["imported".into()],
        oauth: false,
        oauth_client_id: None,
        oauth_scopes: None,
        oauth_callback_port: None,
        azure_credentials: false,
    };
    (input, unsupported)
}

fn is_duplicate(existing: &ServerInput, candidate: &ServerInput) -> bool {
    if existing.name.eq_ignore_ascii_case(&candidate.name) {
        return true;
    }
    match (existing.transport, candidate.transport) {
        (TransportKind::Stdio, TransportKind::Stdio) => {
            existing.command.is_some()
                && existing.command == candidate.command
                && existing.args == candidate.args
        }
        (TransportKind::Http, TransportKind::Http) => {
            existing.url.is_some() && existing.url == candidate.url
        }
        _ => false,
    }
}

/// Environment variables and headers with these words in their name are treated as secrets.
pub fn looks_secret(name: &str) -> bool {
    let upper = name.to_uppercase();
    [
        "KEY",
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "AUTH",
        "CREDENTIAL",
        "COOKIE",
    ]
    .iter()
    .any(|word| upper.contains(word))
}

/// What importing one server did.
pub struct ImportedServer {
    pub server: ServerDefinition,
    /// The name was already taken, so a numeric suffix was added.
    pub renamed: bool,
    /// Values (tokens, API keys) that were moved from the config into the OS keyring.
    pub secrets_moved: u32,
}

/// Creates one server. Secret-looking values move into the keyring. A name that is already taken
/// gets a numeric suffix.
pub async fn import_one(
    registry: &Registry,
    secrets: &dyn SecretStore,
    mut input: ServerInput,
) -> DbResult<ImportedServer> {
    let taken: Vec<String> = registry
        .list()
        .await?
        .into_iter()
        .map(|s| s.input.name.to_lowercase())
        .collect();
    let original = input.name.clone();
    let mut suffix = 1;
    while taken.contains(&input.name.to_lowercase()) {
        suffix += 1;
        input.name = format!("{original} ({suffix})");
    }
    let renamed = input.name != original;

    let mut stored = Vec::new();
    for map in [&mut input.env, &mut input.headers] {
        for (key, value) in map.iter_mut() {
            if looks_secret(key) && !value.is_empty() && !value.starts_with("keyring:") {
                let name = format!("imported/{}", crate::db::new_id());
                if secrets.set(&name, value).is_ok() {
                    *value = reference(&name);
                    stored.push(name);
                }
            }
        }
    }
    match registry.create(input).await {
        Ok(server) => Ok(ImportedServer {
            server,
            renamed,
            secrets_moved: stored.len() as u32,
        }),
        Err(error) => {
            // Do not leave orphaned secrets behind.
            for name in stored {
                let _ = secrets.delete(&name);
            }
            Err(error)
        }
    }
}

/// Creates the selected servers; see [`import_one`].
pub async fn import_servers(
    registry: &Registry,
    secrets: &dyn SecretStore,
    candidates: Vec<ServerInput>,
) -> ImportSummary {
    let mut summary = ImportSummary {
        created: 0,
        renamed: 0,
        secrets_moved: 0,
        failed: vec![],
    };
    for input in candidates {
        let original = input.name.clone();
        match import_one(registry, secrets, input).await {
            Ok(imported) => {
                summary.created += 1;
                summary.renamed += u32::from(imported.renamed);
                summary.secrets_moved += imported.secrets_moved;
            }
            Err(error) => summary.failed.push(format!("{original}: {error}")),
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::Db, secrets::MemoryStore};

    const DESKTOP: &str = r#"{
      "mcpServers": {
        "filesystem": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]},
        "github": {"command": "docker", "args": ["run", "-i", "ghcr.io/github/github-mcp-server"],
                   "env": {"GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_secret", "LOG_LEVEL": "info"}},
        "remote": {"type": "http", "url": "https://example.com/mcp", "headers": {"Authorization": "Bearer abc"}},
        "legacy": {"type": "sse", "url": "https://example.com/sse"},
        "broken": {"foo": "bar"}
      }
    }"#;

    #[test]
    fn parses_claude_desktop_style_files() {
        let found = parse_config(DESKTOP, &[]).unwrap();
        assert_eq!(found.len(), 5);
        let github = found.iter().find(|c| c.input.name == "github").unwrap();
        assert_eq!(github.input.transport, TransportKind::Stdio);
        assert_eq!(github.input.args[0], "run");
        assert_eq!(github.input.env["LOG_LEVEL"], "info");
        let remote = found.iter().find(|c| c.input.name == "remote").unwrap();
        assert_eq!(remote.input.transport, TransportKind::Http);
        assert_eq!(remote.input.headers["Authorization"], "Bearer abc");
        assert!(found
            .iter()
            .find(|c| c.input.name == "legacy")
            .unwrap()
            .unsupported
            .as_ref()
            .unwrap()
            .contains("SSE"));
        assert!(found
            .iter()
            .find(|c| c.input.name == "broken")
            .unwrap()
            .unsupported
            .is_some());
        assert!(found.iter().all(|c| c.input.tags == ["imported"]));
    }

    #[test]
    fn parses_claude_code_user_and_project_scopes() {
        let json = r#"{
          "mcpServers": {"user-tool": {"command": "uvx", "args": ["tool"]}},
          "projects": {
            "/home/me/app": {"mcpServers": {"proj-tool": {"command": "node", "args": ["server.js"]}}},
            "/home/me/empty": {}
          },
          "numStartups": 12
        }"#;
        let found = parse_config(json, &[]).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].origin, "top level");
        assert_eq!(found[1].origin, "project /home/me/app");
    }

    #[test]
    fn rejects_files_without_servers() {
        assert!(parse_config("not json", &[]).is_err());
        assert!(parse_config(r#"{"theme": "dark"}"#, &[]).is_err());
        assert_eq!(parse_config(r#"{"mcpServers": {}}"#, &[]).unwrap(), vec![]);
    }

    #[test]
    fn flags_duplicates_by_name_and_by_command() {
        let existing = parse_config(DESKTOP, &[]).unwrap();
        let existing_inputs: Vec<ServerInput> = existing.iter().map(|c| c.input.clone()).collect();
        let renamed = r#"{"mcpServers": {
            "FILESYSTEM": {"command": "other"},
            "fs2": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]},
            "fresh": {"command": "brand-new"},
            "r2": {"url": "https://example.com/mcp"}
        }}"#;
        let found = parse_config(renamed, &existing_inputs).unwrap();
        let flags: Vec<_> = found
            .iter()
            .map(|c| (c.input.name.as_str(), c.duplicate))
            .collect();
        assert_eq!(
            flags,
            [
                ("FILESYSTEM", true),
                ("fs2", true),
                ("fresh", false),
                ("r2", true)
            ]
        );
    }

    #[test]
    fn secret_names_are_recognized() {
        for name in [
            "API_KEY",
            "github_token",
            "Authorization",
            "DB_PASSWORD",
            "client_secret",
        ] {
            assert!(looks_secret(name), "{name}");
        }
        for name in ["LOG_LEVEL", "PATH", "HOME", "Content-Type"] {
            assert!(!looks_secret(name), "{name}");
        }
    }

    #[tokio::test]
    async fn imports_move_secrets_into_the_keyring_and_rename_clashes() {
        let db = Db::open_in_memory().await.unwrap();
        let registry = Registry::new(db);
        let secrets = MemoryStore::default();
        let found = parse_config(DESKTOP, &[]).unwrap();
        let usable: Vec<ServerInput> = found
            .into_iter()
            .filter(|c| c.unsupported.is_none())
            .map(|c| c.input)
            .collect();

        let first = import_servers(&registry, &secrets, usable.clone()).await;
        assert_eq!(
            (first.created, first.renamed, first.secrets_moved),
            (3, 0, 2)
        );
        assert!(first.failed.is_empty());

        let github = registry
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.input.name == "github")
            .unwrap();
        let token_ref = github.input.env["GITHUB_PERSONAL_ACCESS_TOKEN"].clone();
        assert!(token_ref.starts_with("keyring:imported/"), "{token_ref}");
        assert_eq!(github.input.env["LOG_LEVEL"], "info");
        let name = token_ref.trim_start_matches("keyring:");
        assert_eq!(secrets.get(name).unwrap().as_deref(), Some("ghp_secret"));

        // Importing again renames instead of failing.
        let second = import_servers(&registry, &secrets, usable).await;
        assert_eq!((second.created, second.renamed), (3, 3));
        let names: Vec<_> = registry
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.input.name)
            .collect();
        assert!(names.contains(&"github (2)".to_owned()));
    }

    #[tokio::test]
    async fn invalid_entries_fail_without_leaving_secrets_behind() {
        let db = Db::open_in_memory().await.unwrap();
        let registry = Registry::new(db);
        let secrets = MemoryStore::default();
        let (mut bad, _) = to_input(
            "bad",
            &serde_json::json!({"command": "x", "env": {"API_KEY": "s3cret"}}),
        );
        bad.command = None; // stdio without a command is invalid
        let summary = import_servers(&registry, &secrets, vec![bad]).await;
        assert_eq!(summary.created, 0);
        assert_eq!(summary.failed.len(), 1);
        assert_eq!(summary.secrets_moved, 0);
    }

    #[test]
    fn detects_well_known_locations() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        std::fs::write(home.join(".claude.json"), "{}").unwrap();
        let sources = detect_sources(Some(home), Some(&home.join("AppData")));
        let claude_code = sources
            .iter()
            .find(|s| s.label.starts_with("Claude Code"))
            .unwrap();
        assert!(claude_code.exists);
        let desktop = sources
            .iter()
            .find(|s| s.label == "Claude Desktop")
            .unwrap();
        assert!(!desktop.exists);
        assert!(desktop.path.contains("Claude"));
    }
}
