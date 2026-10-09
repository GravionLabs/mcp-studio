//! Routes servers of other clients through MCP Studio, and puts them back.
//!
//! Recording a real client means pointing one of its server entries at the MCP Studio proxy. This
//! module does that edit in the client's own configuration file (Claude Desktop, Claude Code, VS
//! Code, the Copilot CLI and OpenCode; see `client_formats`): it registers the server in MCP Studio, copies the file to a backup, replaces only
//! that entry, and remembers what it did, so [`unroute`] can restore the original entry.
//!
//! Nothing here touches a file that is not one of the known configuration files, and a write is
//! refused when another program changed the file in the meantime (Claude Code rewrites
//! `~/.claude.json` often).

use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use sqlx::FromRow;

use crate::{
    client_formats::{ClientFormat, ClientSource},
    client_import::{import_one, looks_secret},
    db::{new_id, now_ms, Db, DbError, DbResult},
    registry::{Registry, ServerInput, TransportKind},
    secrets::SecretStore,
};

/// What the routing needs from the running app.
pub struct RouteContext<'a> {
    pub db: &'a Db,
    pub registry: &'a Registry,
    pub secrets: &'a dyn SecretStore,
    /// The `mcp-studio-proxy` program (stdio servers); `None` when it was not found.
    pub proxy_binary: Option<PathBuf>,
    /// Port of the local HTTP proxy (HTTP servers).
    pub http_proxy_port: u16,
    /// Where copies of the configuration files are kept.
    pub backup_dir: PathBuf,
}

/// One entry of a client's configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EntryRef {
    pub path: String,
    /// JSON pointer to the object that holds the servers, e.g. `/mcpServers`.
    pub pointer: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Stdio,
    Http,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClientEntry {
    #[serde(flatten)]
    pub entry: EntryRef,
    /// "Claude Desktop", "VS Code (user)", ...
    pub client: String,
    /// "top level" or "project /path".
    pub origin: String,
    pub kind: EntryKind,
    /// The command line or URL.
    pub summary: String,
    /// The entry already goes through MCP Studio.
    pub routed: bool,
    /// Set when MCP Studio did the routing and can undo it.
    pub route_id: Option<String>,
    /// Why the entry cannot be routed.
    pub unsupported: Option<String>,
}

/// A configuration file that could not be read, so none of its servers are listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UnreadableFile {
    pub client: String,
    pub path: String,
    pub reason: String,
}

/// The servers found in the known configuration files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClientEntries {
    pub entries: Vec<ClientEntry>,
    pub unreadable: Vec<UnreadableFile>,
}

/// What routing an entry would change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RoutePreview {
    pub client: String,
    pub path: String,
    pub before: String,
    pub after: String,
    /// Environment variables and headers whose values would move into the OS keyring.
    pub secrets: Vec<String>,
    /// The MCP Studio server that is used when the same server is already registered.
    pub existing_server: Option<String>,
    pub backup_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RouteResult {
    pub route_id: String,
    pub server_name: String,
    pub backup_path: String,
    pub secrets_moved: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UnrouteResult {
    pub restored: bool,
    /// Why nothing was restored: the entry changed (or is gone) since it was routed.
    pub conflict: Option<String>,
}

#[derive(FromRow)]
struct RouteRow {
    id: String,
    client: String,
    config_path: String,
    location: String,
    entry_name: String,
    routed_entry: String,
    backup_path: String,
}

struct LoadedFile {
    root: Value,
    modified: SystemTime,
    /// The whitespace that indents the file, so that a rewrite keeps its look.
    indent: String,
    trailing_newline: bool,
}

/// Every server entry in the known configuration files that exist.
pub async fn list_entries(
    ctx: &RouteContext<'_>,
    sources: &[ClientSource],
) -> DbResult<ClientEntries> {
    let rows = routes(ctx.db).await?;
    let mut entries = Vec::new();
    let mut unreadable = Vec::new();
    for source in sources.iter().filter(|s| s.exists) {
        let file = match load(Path::new(&source.path)) {
            Ok(file) => file,
            Err(error) => {
                unreadable.push(UnreadableFile {
                    client: source.label.clone(),
                    path: source.path.clone(),
                    reason: error.to_string(),
                });
                continue;
            }
        };
        for (pointer, origin) in source.format.locations(&file.root) {
            let Some(servers) = file.root.pointer(&pointer).and_then(Value::as_object) else {
                continue;
            };
            for (name, entry) in servers {
                let (input, unsupported) = source.format.parse(name, entry);
                let route = rows.iter().find(|r| {
                    r.config_path == source.path && r.location == pointer && r.entry_name == *name
                });
                let kind = match (&unsupported, input.transport) {
                    (Some(_), _) => EntryKind::Unsupported,
                    (None, TransportKind::Stdio) => EntryKind::Stdio,
                    (None, TransportKind::Http) => EntryKind::Http,
                };
                entries.push(ClientEntry {
                    entry: EntryRef {
                        path: source.path.clone(),
                        pointer: pointer.clone(),
                        name: name.clone(),
                    },
                    client: source.label.clone(),
                    origin: origin.clone(),
                    kind,
                    summary: summarize(&input),
                    routed: route.is_some() || is_routed(source.format, entry, ctx.http_proxy_port),
                    route_id: route.map(|r| r.id.clone()),
                    unsupported,
                });
            }
        }
    }
    Ok(ClientEntries {
        entries,
        unreadable,
    })
}

/// What routing `target` would do. Changes nothing.
pub async fn preview(
    ctx: &RouteContext<'_>,
    sources: &[ClientSource],
    target: &EntryRef,
) -> DbResult<RoutePreview> {
    let (source, _file, entry, input) = prepare(ctx, sources, target)?;
    let existing = find_same(ctx.registry, &input).await?;
    let server_id = existing
        .as_ref()
        .map_or("<id of the new server>", |s| s.id.as_str());
    let after = routed_entry(ctx, source.format, &entry, &input, server_id)?;
    let secrets = input
        .env
        .iter()
        .chain(input.headers.iter())
        .filter(|(key, value)| {
            looks_secret(key) && !value.is_empty() && !value.starts_with("keyring:")
        })
        .map(|(key, _)| key.clone())
        .collect();
    Ok(RoutePreview {
        client: source.label.clone(),
        path: source.path.clone(),
        before: pretty(&entry),
        after: pretty(&after),
        secrets,
        existing_server: existing.map(|s| s.input.name),
        backup_dir: ctx.backup_dir.to_string_lossy().into_owned(),
    })
}

/// Points the entry at the MCP Studio proxy. The original stays recoverable through [`unroute`].
pub async fn route(
    ctx: &RouteContext<'_>,
    sources: &[ClientSource],
    target: &EntryRef,
) -> DbResult<RouteResult> {
    let (source, mut file, entry, input) = prepare(ctx, sources, target)?;
    let path = PathBuf::from(&source.path);
    let route_id = new_id();
    let backup = backup_file(&path, &ctx.backup_dir, &route_id)?;

    let (server, secrets_moved) = match find_same(ctx.registry, &input).await? {
        Some(server) => (server, 0),
        None => {
            let imported = import_one(ctx.registry, ctx.secrets, input.clone()).await?;
            (imported.server, imported.secrets_moved)
        }
    };
    let routed = routed_entry(ctx, source.format, &entry, &input, &server.id)?;
    set_entry(&mut file.root, target, routed.clone())?;
    write_file(&path, &file)?;

    sqlx::query(
        "INSERT INTO client_routes \
         (id, client, config_path, location, entry_name, server_id, routed_entry, backup_path, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (config_path, location, entry_name) DO UPDATE SET \
         id = excluded.id, client = excluded.client, server_id = excluded.server_id, \
         routed_entry = excluded.routed_entry, backup_path = excluded.backup_path, \
         created_at = excluded.created_at",
    )
    .bind(&route_id)
    .bind(&source.label)
    .bind(&source.path)
    .bind(&target.pointer)
    .bind(&target.name)
    .bind(&server.id)
    .bind(routed.to_string())
    .bind(backup.to_string_lossy().as_ref())
    .bind(now_ms())
    .execute(ctx.db.pool())
    .await?;
    Ok(RouteResult {
        route_id,
        server_name: server.input.name,
        backup_path: backup.to_string_lossy().into_owned(),
        secrets_moved,
    })
}

/// Puts the original entry back. When the entry was changed (or removed) since it was routed,
/// nothing is overwritten unless `force` is set.
pub async fn unroute(db: &Db, route_id: &str, force: bool) -> DbResult<UnrouteResult> {
    let row: RouteRow = sqlx::query_as(
        "SELECT id, client, config_path, location, entry_name, routed_entry, backup_path \
         FROM client_routes WHERE id = ?",
    )
    .bind(route_id)
    .fetch_optional(db.pool())
    .await?
    .ok_or_else(|| DbError::NotFound(format!("route {route_id}")))?;
    let target = EntryRef {
        path: row.config_path.clone(),
        pointer: row.location.clone(),
        name: row.entry_name.clone(),
    };
    let path = PathBuf::from(&row.config_path);
    let mut file = load(&path)?;
    let routed: Value = serde_json::from_str(&row.routed_entry)
        .map_err(|e| DbError::Invalid(format!("corrupt route {}: {e}", row.id)))?;

    let current = get_entry(&file.root, &target);
    let conflict = match current {
        Some(current) if *current == routed => None,
        Some(_) => Some(format!(
            "The entry \"{}\" in {} was changed after it was routed.",
            row.entry_name, row.client
        )),
        None => Some(format!(
            "The entry \"{}\" is no longer in the {} file.",
            row.entry_name, row.client
        )),
    };
    if let (Some(conflict), false) = (&conflict, force) {
        return Ok(UnrouteResult {
            restored: false,
            conflict: Some(conflict.clone()),
        });
    }

    let backup = load(Path::new(&row.backup_path)).map_err(|e| {
        DbError::Invalid(format!(
            "the backup {} cannot be read ({e}), so the original entry cannot be restored",
            row.backup_path
        ))
    })?;
    let original = get_entry(&backup.root, &target).cloned().ok_or_else(|| {
        DbError::Invalid(format!(
            "the backup {} has no entry \"{}\"",
            row.backup_path, row.entry_name
        ))
    })?;
    set_entry(&mut file.root, &target, original)?;
    write_file(&path, &file)?;
    sqlx::query("DELETE FROM client_routes WHERE id = ?")
        .bind(&row.id)
        .execute(db.pool())
        .await?;
    let _ = std::fs::remove_file(&row.backup_path);
    Ok(UnrouteResult {
        restored: true,
        conflict: None,
    })
}

async fn routes(db: &Db) -> DbResult<Vec<RouteRow>> {
    Ok(sqlx::query_as(
        "SELECT id, client, config_path, location, entry_name, routed_entry, backup_path \
         FROM client_routes",
    )
    .fetch_all(db.pool())
    .await?)
}

/// Checks that `target` is an entry in a known file that can be routed.
fn prepare<'s>(
    ctx: &RouteContext<'_>,
    sources: &'s [ClientSource],
    target: &EntryRef,
) -> DbResult<(&'s ClientSource, LoadedFile, Value, ServerInput)> {
    let source = sources
        .iter()
        .find(|s| s.path == target.path && s.exists)
        .ok_or_else(|| DbError::Invalid("this is not a known client configuration file".into()))?;
    let file = load(Path::new(&source.path))?;
    let entry = get_entry(&file.root, target)
        .cloned()
        .ok_or_else(|| DbError::NotFound(format!("entry \"{}\"", target.name)))?;
    if is_routed(source.format, &entry, ctx.http_proxy_port) {
        return Err(DbError::Invalid(format!(
            "\"{}\" already goes through MCP Studio",
            target.name
        )));
    }
    let (input, unsupported) = source.format.parse(&target.name, &entry);
    if let Some(reason) = unsupported {
        return Err(DbError::Invalid(format!(
            "\"{}\" cannot be routed: {reason}",
            target.name
        )));
    }
    // Fail before anything is copied or registered when the replacement cannot be built.
    routed_entry(ctx, source.format, &entry, &input, "")?;
    Ok((source, file, entry, input))
}

/// The entry that replaces `entry`; see [`ClientFormat::routed_entry`].
fn routed_entry(
    ctx: &RouteContext<'_>,
    format: ClientFormat,
    entry: &Value,
    input: &ServerInput,
    server_id: &str,
) -> DbResult<Value> {
    format.routed_entry(
        entry,
        input,
        ctx.proxy_binary.as_deref(),
        ctx.http_proxy_port,
        server_id,
    )
}

/// Whether `entry` already runs through the MCP Studio proxy.
fn is_routed(format: ClientFormat, entry: &Value, http_port: u16) -> bool {
    // The file name of the command, whichever separator the file was written with.
    let program = format
        .command_of(entry)
        .and_then(|command| command.rsplit(['/', '\\']).next());
    if program.is_some_and(|p| p.starts_with("mcp-studio-proxy")) {
        return true;
    }
    entry
        .get("url")
        .and_then(Value::as_str)
        .is_some_and(|url| url.starts_with(&format!("http://127.0.0.1:{http_port}/mcp/")))
}

/// A registered server that starts the same program or talks to the same URL.
async fn find_same(
    registry: &Registry,
    input: &ServerInput,
) -> DbResult<Option<crate::registry::ServerDefinition>> {
    Ok(registry.list().await?.into_iter().find(|s| {
        s.input.transport == input.transport
            && match input.transport {
                TransportKind::Stdio => {
                    s.input.command.is_some()
                        && s.input.command == input.command
                        && s.input.args == input.args
                }
                TransportKind::Http => s.input.url.is_some() && s.input.url == input.url,
            }
    }))
}

fn summarize(input: &ServerInput) -> String {
    match input.transport {
        TransportKind::Stdio => std::iter::once(input.command.clone().unwrap_or_default())
            .chain(input.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" "),
        TransportKind::Http => input.url.clone().unwrap_or_default(),
    }
}

fn get_entry<'a>(root: &'a Value, target: &EntryRef) -> Option<&'a Value> {
    root.pointer(&target.pointer)?
        .as_object()?
        .get(&target.name)
}

fn set_entry(root: &mut Value, target: &EntryRef, entry: Value) -> DbResult<()> {
    let servers = root
        .pointer_mut(&target.pointer)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| DbError::Invalid("the servers section is no longer in the file".into()))?;
    servers.insert(target.name.clone(), entry);
    Ok(())
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn load(path: &Path) -> DbResult<LoadedFile> {
    let io = |e: std::io::Error| DbError::Invalid(format!("{}: {e}", path.display()));
    let text = std::fs::read_to_string(path).map_err(io)?;
    let modified = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map_err(io)?;
    let root: Value = serde_json::from_str(&text)
        .map_err(|e| DbError::Invalid(format!("{} is not valid JSON: {e}", path.display())))?;
    let indent = text
        .lines()
        .nth(1)
        .map(|line| {
            line.chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect::<String>()
        })
        .filter(|indent| !indent.is_empty())
        .unwrap_or_else(|| "  ".to_owned());
    Ok(LoadedFile {
        root,
        modified,
        indent,
        trailing_newline: text.ends_with('\n'),
    })
}

/// Writes the file through a temporary file in the same directory, unless it changed since it was
/// loaded.
fn write_file(path: &Path, file: &LoadedFile) -> DbResult<()> {
    use serde::Serialize as _;
    let io = |e: std::io::Error| DbError::Invalid(format!("{}: {e}", path.display()));
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(file.indent.as_bytes());
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    file.root
        .serialize(&mut serializer)
        .map_err(|e| DbError::Invalid(e.to_string()))?;
    if file.trailing_newline {
        out.push(b'\n');
    }

    let metadata = std::fs::metadata(path).map_err(io)?;
    if metadata.modified().map_err(io)? != file.modified {
        return Err(DbError::Invalid(format!(
            "{} was changed by another program while MCP Studio was working. Nothing was written; try again",
            path.display()
        )));
    }
    let temp = path.with_file_name(format!(
        ".{}.mcp-studio-tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("config")
    ));
    std::fs::write(&temp, &out).map_err(io)?;
    let finish = || -> std::io::Result<()> {
        std::fs::set_permissions(&temp, metadata.permissions())?;
        std::fs::rename(&temp, path)
    };
    finish().map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        io(e)
    })
}

/// Copies the file into the backup directory; only the user can read the copy.
fn backup_file(path: &Path, dir: &Path, route_id: &str) -> DbResult<PathBuf> {
    let io = |e: std::io::Error| DbError::Invalid(format!("backup in {}: {e}", dir.display()));
    std::fs::create_dir_all(dir).map_err(io)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config");
    let target = dir.join(format!("{route_id}-{name}"));
    std::fs::copy(path, &target).map_err(io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).map_err(io)?;
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_file_that_changed_since_it_was_loaded_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{\n  \"a\": 1\n}\n").unwrap();
        let file = load(&path).unwrap();

        // Another program writes the file after we read it.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "{\"a\": 2}").unwrap();
        let error = write_file(&path, &file).unwrap_err();

        assert!(
            error.to_string().contains("changed by another program"),
            "{error}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\": 2}");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "no temporary file is left behind");
    }

    #[test]
    fn a_rewrite_keeps_the_indentation_and_the_final_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{\n\t\"a\": [1, 2]\n}").unwrap();
        let mut file = load(&path).unwrap();
        file.root["b"] = json!(true);

        write_file(&path, &file).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n\t\"a\": [\n\t\t1,\n\t\t2\n\t],\n\t\"b\": true\n}"
        );
    }

    #[test]
    fn routed_entries_are_recognized() {
        let claude = ClientFormat::Claude;
        let routed = |entry: Value, port: u16| is_routed(claude, &entry, port);
        assert!(routed(
            json!({"command": "/opt/app/mcp-studio-proxy", "args": []}),
            1
        ));
        assert!(routed(
            json!({"command": "C:\\app\\mcp-studio-proxy.exe"}),
            1
        ));
        assert!(routed(
            json!({"url": "http://127.0.0.1:38465/mcp/abc"}),
            38465
        ));
        assert!(!routed(json!({"url": "http://127.0.0.1:38465/mcp/abc"}), 1));
        assert!(!routed(json!({"command": "npx"}), 1));
        assert!(!routed(json!({"url": "https://example.com/mcp"}), 38465));
        let opencode =
            json!({"type": "local", "command": ["/opt/app/mcp-studio-proxy", "--server", "x"]});
        assert!(is_routed(ClientFormat::OpenCode, &opencode, 1));
        assert!(!is_routed(ClientFormat::Claude, &opencode, 1));
    }
}
