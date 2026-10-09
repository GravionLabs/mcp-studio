//! The configuration files of the clients that can be routed through MCP Studio, and how each of
//! them writes a server entry: Claude Desktop and Claude Code, VS Code (GitHub Copilot), the
//! GitHub Copilot CLI, and OpenCode.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use specta::Type;

use crate::{
    client_import,
    db::{DbError, DbResult},
    registry::{ServerInput, TransportKind},
};

/// How a client's configuration file is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ClientFormat {
    /// `mcpServers`, plus one per project in Claude Code's `~/.claude.json`.
    Claude,
    /// `servers` in VS Code's `mcp.json`.
    VsCode,
    /// `mcpServers` in the Copilot CLI's `mcp-config.json`.
    CopilotCli,
    /// `mcp` in `opencode.json`, where a local server's command is one array.
    OpenCode,
}

/// A client configuration file that may contain servers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClientSource {
    /// "Claude Desktop", "Claude Code (user)", "VS Code (user)", "Copilot CLI" or "OpenCode".
    pub label: String,
    pub path: String,
    pub exists: bool,
    pub format: ClientFormat,
}

/// The well-known configuration files of the current user.
pub fn detect_sources(home: Option<&Path>, app_data: Option<&Path>) -> Vec<ClientSource> {
    let mut found: Vec<ClientSource> = client_import::detect_sources(home, app_data)
        .into_iter()
        .map(|s| ClientSource {
            label: s.label,
            path: s.path,
            exists: s.exists,
            format: ClientFormat::Claude,
        })
        .collect();
    let mut add = |label: &str, path: PathBuf, format| {
        found.push(ClientSource {
            label: label.to_owned(),
            exists: path.is_file(),
            path: path.to_string_lossy().into_owned(),
            format,
        });
    };
    let vscode_user = if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support/Code/User/mcp.json"))
    } else if cfg!(windows) {
        app_data.map(|d| d.join("Code/User/mcp.json"))
    } else {
        home.map(|h| h.join(".config/Code/User/mcp.json"))
    };
    if let Some(path) = vscode_user {
        add("VS Code (user)", path, ClientFormat::VsCode);
    }
    if let Some(home) = home {
        add(
            "Copilot CLI",
            home.join(".copilot/mcp-config.json"),
            ClientFormat::CopilotCli,
        );
        add(
            "OpenCode",
            home.join(".config/opencode/opencode.json"),
            ClientFormat::OpenCode,
        );
    }
    found
}

/// Keys that describe how a server is started or reached; everything else in an entry (a tool
/// allow list, an `enabled` flag, ...) is the client's own and stays when the entry is routed.
const DEFINITION_KEYS: [&str; 7] = ["command", "args", "env", "envFile", "cwd", "url", "headers"];
const OPENCODE_DEFINITION_KEYS: [&str; 5] = ["command", "environment", "url", "headers", "oauth"];

impl ClientFormat {
    /// Where servers are defined in a file: a JSON pointer to the object and what to call the scope.
    pub fn locations(self, root: &Value) -> Vec<(String, String)> {
        let top = match self {
            Self::Claude | Self::CopilotCli => "mcpServers",
            Self::VsCode => "servers",
            Self::OpenCode => "mcp",
        };
        let mut found = Vec::new();
        if root.get(top).is_some_and(Value::is_object) {
            found.push((format!("/{top}"), "top level".to_owned()));
        }
        if self == Self::Claude {
            // Claude Code keeps project scoped servers under `projects.<path>.mcpServers`.
            if let Some(projects) = root.get("projects").and_then(Value::as_object) {
                for (path, project) in projects {
                    if project.get("mcpServers").is_some_and(Value::is_object) {
                        found.push((
                            format!("/projects/{}/mcpServers", escape_pointer(path)),
                            format!("project {path}"),
                        ));
                    }
                }
            }
        }
        found
    }

    /// The server an entry describes, and why it cannot be routed if it cannot.
    pub fn parse(self, name: &str, entry: &Value) -> (ServerInput, Option<String>) {
        let (input, mut unsupported) = match self {
            Self::OpenCode => parse_opencode(name, entry),
            _ => client_import::to_input(name, entry),
        };
        // The clients expand `${VAR}` and similar in an entry; MCP Studio would pass them on as text.
        if unsupported.is_none() && entry.to_string().contains("${") {
            unsupported = Some(
                "it uses ${...} variables, which MCP Studio does not expand for the server".into(),
            );
        }
        if unsupported.is_none() && entry.get("envFile").is_some() {
            unsupported = Some("it uses an envFile, which MCP Studio does not read".into());
        }
        (input, unsupported)
    }

    /// The program an entry starts, as written.
    pub fn command_of(self, entry: &Value) -> Option<&str> {
        match self {
            Self::OpenCode => entry.get("command")?.as_array()?.first()?.as_str(),
            _ => entry.get("command")?.as_str(),
        }
    }

    /// The entry that replaces `entry`: the proxy program for stdio servers, the local HTTP proxy
    /// for HTTP servers. The client's own keys stay.
    pub fn routed_entry(
        self,
        entry: &Value,
        input: &ServerInput,
        proxy_binary: Option<&Path>,
        http_proxy_port: u16,
        server_id: &str,
    ) -> DbResult<Value> {
        let mut routed: Map<String, Value> = entry.as_object().cloned().unwrap_or_default();
        let strip: &[&str] = if self == Self::OpenCode {
            &OPENCODE_DEFINITION_KEYS
        } else {
            &DEFINITION_KEYS
        };
        routed.retain(|key, _| !strip.contains(&key.as_str()));
        match input.transport {
            TransportKind::Stdio => {
                let proxy = proxy_binary.ok_or_else(|| {
                    DbError::Invalid(
                        "the mcp-studio-proxy program was not found next to the app, so stdio servers cannot be routed"
                            .into(),
                    )
                })?;
                let proxy = proxy.to_string_lossy();
                if self == Self::OpenCode {
                    routed.insert("type".into(), json!("local"));
                    routed.insert("command".into(), json!([proxy, "--server", server_id]));
                } else {
                    routed.insert("command".into(), json!(proxy));
                    routed.insert("args".into(), json!(["--server", server_id]));
                }
            }
            TransportKind::Http => {
                let remote = if self == Self::OpenCode {
                    "remote"
                } else {
                    "http"
                };
                if self == Self::OpenCode {
                    routed.insert("type".into(), json!(remote));
                } else {
                    routed.entry("type").or_insert_with(|| json!(remote));
                }
                routed.insert(
                    "url".into(),
                    json!(format!(
                        "http://127.0.0.1:{http_proxy_port}/mcp/{server_id}"
                    )),
                );
            }
        }
        Ok(Value::Object(routed))
    }
}

pub fn escape_pointer(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
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

fn parse_opencode(name: &str, entry: &Value) -> (ServerInput, Option<String>) {
    let remote =
        entry.get("type").and_then(Value::as_str) == Some("remote") || entry.get("url").is_some();
    let command = strings(entry.get("command"));
    let mut unsupported = None;
    if remote {
        if entry.get("oauth").is_some_and(|o| o != &Value::Bool(false)) {
            unsupported = Some("OpenCode signs in to this server itself (oauth)".into());
        }
    } else if command.is_empty() {
        unsupported = Some("neither a command nor a URL is defined".into());
    }
    let input = ServerInput {
        name: name.trim().to_owned(),
        transport: if remote {
            TransportKind::Http
        } else {
            TransportKind::Stdio
        },
        command: command.first().cloned(),
        args: command.iter().skip(1).cloned().collect(),
        env: string_map(entry.get("environment")),
        cwd: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio_input(command: &str, args: &[&str]) -> ServerInput {
        ServerInput {
            name: "s".into(),
            transport: TransportKind::Stdio,
            command: Some(command.into()),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
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
        }
    }

    #[test]
    fn each_format_looks_for_servers_where_its_client_keeps_them() {
        let claude = json!({"mcpServers": {}, "projects": {"/a/b": {"mcpServers": {}}}});
        assert_eq!(
            ClientFormat::Claude.locations(&claude),
            [
                ("/mcpServers".to_owned(), "top level".to_owned()),
                (
                    "/projects/~1a~1b/mcpServers".to_owned(),
                    "project /a/b".to_owned()
                )
            ]
        );
        let at = |format: ClientFormat, key: &str| {
            format
                .locations(&json!({ key: {} }))
                .into_iter()
                .next()
                .map(|l| l.0)
        };
        assert_eq!(
            at(ClientFormat::VsCode, "servers").as_deref(),
            Some("/servers")
        );
        assert_eq!(
            at(ClientFormat::CopilotCli, "mcpServers").as_deref(),
            Some("/mcpServers")
        );
        assert_eq!(at(ClientFormat::OpenCode, "mcp").as_deref(), Some("/mcp"));
        assert_eq!(at(ClientFormat::VsCode, "mcpServers"), None);
    }

    #[test]
    fn opencode_commands_are_one_array() {
        let entry = json!({"type": "local", "command": ["npx", "-y", "srv"], "environment": {"A": "1"}, "enabled": true});
        let (input, unsupported) = ClientFormat::OpenCode.parse("x", &entry);
        assert_eq!(unsupported, None);
        assert_eq!(input.command.as_deref(), Some("npx"));
        assert_eq!(input.args, ["-y", "srv"]);
        assert_eq!(input.env["A"], "1");
        assert_eq!(ClientFormat::OpenCode.command_of(&entry), Some("npx"));
        let remote = json!({"type": "remote", "url": "https://e.com/mcp", "headers": {"X": "y"}});
        let (input, unsupported) = ClientFormat::OpenCode.parse("x", &remote);
        assert_eq!((input.transport, unsupported), (TransportKind::Http, None));
        let oauth = json!({"type": "remote", "url": "https://e.com/mcp", "oauth": {}});
        assert!(ClientFormat::OpenCode
            .parse("x", &oauth)
            .1
            .unwrap()
            .contains("OpenCode"));
    }

    #[test]
    fn entries_with_variables_or_env_files_are_not_routed() {
        for entry in [
            json!({"command": "npx", "args": ["${workspaceFolder}"]}),
            json!({"type": "http", "url": "https://e.com", "headers": {"A": "Bearer ${input:token}"}}),
            json!({"command": "npx", "envFile": ".env"}),
        ] {
            assert!(
                ClientFormat::VsCode.parse("x", &entry).1.is_some(),
                "{entry}"
            );
        }
        assert!(ClientFormat::Claude
            .parse("x", &json!({"command": "npx"}))
            .1
            .is_none());
    }

    #[test]
    fn routing_keeps_the_clients_own_keys() {
        let proxy = Path::new("/p/mcp-studio-proxy");
        let input = stdio_input("npx", &["srv"]);
        let copilot = json!({"type": "local", "command": "npx", "args": ["srv"], "env": {"A": "1"}, "tools": ["*"]});
        assert_eq!(
            ClientFormat::CopilotCli
                .routed_entry(&copilot, &input, Some(proxy), 1, "id1")
                .unwrap(),
            json!({"type": "local", "command": "/p/mcp-studio-proxy", "args": ["--server", "id1"], "tools": ["*"]})
        );
        let vscode = json!({"type": "stdio", "command": "npx", "args": ["srv"]});
        assert_eq!(
            ClientFormat::VsCode
                .routed_entry(&vscode, &input, Some(proxy), 1, "id1")
                .unwrap(),
            json!({"type": "stdio", "command": "/p/mcp-studio-proxy", "args": ["--server", "id1"]})
        );
        let opencode = json!({"type": "local", "command": ["npx", "srv"], "environment": {"A": "1"}, "enabled": true});
        assert_eq!(
            ClientFormat::OpenCode
                .routed_entry(&opencode, &input, Some(proxy), 1, "id1")
                .unwrap(),
            json!({"type": "local", "command": ["/p/mcp-studio-proxy", "--server", "id1"], "enabled": true})
        );
    }

    #[test]
    fn http_entries_point_at_the_local_proxy_with_their_clients_type() {
        let mut input = stdio_input("", &[]);
        input.transport = TransportKind::Http;
        input.command = None;
        input.url = Some("https://e.com/mcp".into());
        let url = "http://127.0.0.1:38465/mcp/id1";
        let cases = [
            (
                ClientFormat::Claude,
                json!({"url": "https://e.com/mcp"}),
                json!({"type": "http", "url": url}),
            ),
            (
                ClientFormat::CopilotCli,
                json!({"type": "http", "url": "https://e.com/mcp", "headers": {"A": "b"}, "tools": ["*"]}),
                json!({"type": "http", "url": url, "tools": ["*"]}),
            ),
            (
                ClientFormat::OpenCode,
                json!({"type": "remote", "url": "https://e.com/mcp", "headers": {"A": "b"}, "enabled": false}),
                json!({"type": "remote", "url": url, "enabled": false}),
            ),
        ];
        for (format, entry, expected) in cases {
            assert_eq!(
                format
                    .routed_entry(&entry, &input, None, 38465, "id1")
                    .unwrap(),
                expected,
                "{format:?}"
            );
        }
    }

    #[test]
    fn stdio_needs_the_proxy_program() {
        let error = ClientFormat::Claude
            .routed_entry(
                &json!({"command": "npx"}),
                &stdio_input("npx", &[]),
                None,
                1,
                "id",
            )
            .unwrap_err();
        assert!(error.to_string().contains("mcp-studio-proxy"));
    }

    #[test]
    fn well_known_files_of_every_client_are_found() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        std::fs::create_dir_all(home.join(".copilot")).unwrap();
        std::fs::write(home.join(".copilot/mcp-config.json"), "{}").unwrap();
        let sources = detect_sources(Some(home), Some(&home.join("AppData")));
        let by = |label: &str| sources.iter().find(|s| s.label == label).unwrap();
        assert!(by("Copilot CLI").exists);
        assert_eq!(by("Copilot CLI").format, ClientFormat::CopilotCli);
        assert!(!by("OpenCode").exists);
        assert_eq!(by("OpenCode").format, ClientFormat::OpenCode);
        assert_eq!(by("VS Code (user)").format, ClientFormat::VsCode);
        assert_eq!(by("Claude Code (user)").format, ClientFormat::Claude);
    }
}
