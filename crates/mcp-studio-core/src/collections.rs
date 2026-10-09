//! Collections: folders of saved requests, shareable as JSON files.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::FromRow;

use crate::{
    db::{new_id, now_ms, Db, DbError, DbResult},
    model::JsonValue,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CollectionNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    #[specta(type = u32)]
    pub sort_order: i64,
}

/// What the user edits about a saved request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedRequestInput {
    pub collection_id: String,
    pub server_id: String,
    /// MCP method, e.g. `tools/call`.
    pub method: String,
    pub name: String,
    pub tool_name: Option<String>,
    pub arguments: JsonValue,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedRequest {
    pub id: String,
    #[serde(flatten)]
    pub input: SavedRequestInput,
    #[specta(type = u32)]
    pub sort_order: i64,
    #[specta(type = u32)]
    pub updated_at: i64,
}

/// All folders and requests; the UI builds the tree from `parentId` / `collectionId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CollectionTree {
    pub collections: Vec<CollectionNode>,
    pub requests: Vec<SavedRequest>,
}

#[derive(FromRow)]
struct RequestRow {
    id: String,
    collection_id: String,
    server_id: String,
    method: String,
    name: String,
    tool_name: Option<String>,
    arguments: String,
    notes: String,
    sort_order: i64,
    updated_at: i64,
}

impl TryFrom<RequestRow> for SavedRequest {
    type Error = DbError;

    fn try_from(row: RequestRow) -> DbResult<Self> {
        let arguments = serde_json::from_str(&row.arguments)
            .map_err(|e| DbError::Invalid(format!("corrupt request {}: {e}", row.id)))?;
        Ok(Self {
            id: row.id,
            input: SavedRequestInput {
                collection_id: row.collection_id,
                server_id: row.server_id,
                method: row.method,
                name: row.name,
                tool_name: row.tool_name,
                arguments: JsonValue(arguments),
                notes: row.notes,
            },
            sort_order: row.sort_order,
            updated_at: row.updated_at,
        })
    }
}

// --- portable file format ---------------------------------------------------------------------

const FORMAT: &str = "mcp-studio-collection";
const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct ExportFile {
    format: String,
    version: u32,
    collection: ExportFolder,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportFolder {
    name: String,
    #[serde(default)]
    requests: Vec<ExportRequest>,
    #[serde(default)]
    children: Vec<ExportFolder>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportRequest {
    name: String,
    /// Servers are referenced by name so files work across machines.
    server_name: String,
    method: String,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    arguments: serde_json::Value,
    #[serde(default)]
    notes: String,
}

/// Result of importing a collection file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    /// Id of the imported top-level folder.
    pub collection_id: String,
    pub collections_created: u32,
    pub requests_created: u32,
    /// Requests that could not be imported, with the reason (e.g. unknown server).
    pub skipped: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Collections {
    db: Db,
}

impl Collections {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn tree(&self) -> DbResult<CollectionTree> {
        let collections: Vec<CollectionNode> = sqlx::query_as(
            "SELECT id, parent_id, name, sort_order FROM collections ORDER BY sort_order, name COLLATE NOCASE",
        )
        .fetch_all(self.db.pool())
        .await?;
        let rows: Vec<RequestRow> = sqlx::query_as(
            "SELECT id, collection_id, server_id, method, name, tool_name, arguments, notes, sort_order, updated_at \
             FROM requests ORDER BY sort_order, name COLLATE NOCASE",
        )
        .fetch_all(self.db.pool())
        .await?;
        Ok(CollectionTree {
            collections,
            requests: rows
                .into_iter()
                .map(SavedRequest::try_from)
                .collect::<DbResult<_>>()?,
        })
    }

    pub async fn create_collection(
        &self,
        parent_id: Option<&str>,
        name: &str,
    ) -> DbResult<CollectionNode> {
        let name = clean_name(name)?;
        if let Some(parent) = parent_id {
            self.require_collection(parent).await?;
        }
        let id = new_id();
        let order = self.next_order(parent_id).await?;
        sqlx::query(
            "INSERT INTO collections (id, parent_id, name, sort_order) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(parent_id)
        .bind(&name)
        .bind(order)
        .execute(self.db.pool())
        .await?;
        Ok(CollectionNode {
            id,
            parent_id: parent_id.map(str::to_owned),
            name,
            sort_order: order,
        })
    }

    pub async fn rename_collection(&self, id: &str, name: &str) -> DbResult<()> {
        let name = clean_name(name)?;
        let result = sqlx::query("UPDATE collections SET name = ? WHERE id = ?")
            .bind(name)
            .bind(id)
            .execute(self.db.pool())
            .await?;
        not_found_if_none(result.rows_affected(), "collection", id)
    }

    /// Moves a folder under another one (or to the top level). Moving into itself or one of its
    /// descendants is rejected.
    pub async fn move_collection(&self, id: &str, new_parent: Option<&str>) -> DbResult<()> {
        self.require_collection(id).await?;
        if let Some(parent) = new_parent {
            self.require_collection(parent).await?;
            let mut cursor = Some(parent.to_owned());
            let mut seen = HashSet::new();
            while let Some(current) = cursor {
                if current == id {
                    return Err(DbError::Invalid(
                        "a folder cannot be moved into itself".into(),
                    ));
                }
                if !seen.insert(current.clone()) {
                    break;
                }
                cursor = sqlx::query_scalar("SELECT parent_id FROM collections WHERE id = ?")
                    .bind(&current)
                    .fetch_optional(self.db.pool())
                    .await?
                    .flatten();
            }
        }
        let order = self.next_order(new_parent).await?;
        sqlx::query("UPDATE collections SET parent_id = ?, sort_order = ? WHERE id = ?")
            .bind(new_parent)
            .bind(order)
            .bind(id)
            .execute(self.db.pool())
            .await?;
        Ok(())
    }

    /// Deletes a folder with everything inside it.
    pub async fn delete_collection(&self, id: &str) -> DbResult<()> {
        let result = sqlx::query("DELETE FROM collections WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await?;
        not_found_if_none(result.rows_affected(), "collection", id)
    }

    pub async fn save_request(&self, input: SavedRequestInput) -> DbResult<SavedRequest> {
        let input = validate_request(input)?;
        self.require_collection(&input.collection_id).await?;
        let id = new_id();
        let now = now_ms();
        let order: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sort_order) + 1, 0) FROM requests WHERE collection_id = ?",
        )
        .bind(&input.collection_id)
        .fetch_one(self.db.pool())
        .await?;
        sqlx::query(
            "INSERT INTO requests (id, collection_id, server_id, method, name, tool_name, arguments, notes, sort_order, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&input.collection_id)
        .bind(&input.server_id)
        .bind(&input.method)
        .bind(&input.name)
        .bind(&input.tool_name)
        .bind(input.arguments.0.to_string())
        .bind(&input.notes)
        .bind(order)
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(server_missing)?;
        Ok(SavedRequest {
            id,
            input,
            sort_order: order,
            updated_at: now,
        })
    }

    pub async fn update_request(&self, id: &str, input: SavedRequestInput) -> DbResult<()> {
        let input = validate_request(input)?;
        self.require_collection(&input.collection_id).await?;
        let result = sqlx::query(
            "UPDATE requests SET collection_id = ?, server_id = ?, method = ?, name = ?, tool_name = ?, \
             arguments = ?, notes = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&input.collection_id)
        .bind(&input.server_id)
        .bind(&input.method)
        .bind(&input.name)
        .bind(&input.tool_name)
        .bind(input.arguments.0.to_string())
        .bind(&input.notes)
        .bind(now_ms())
        .bind(id)
        .execute(self.db.pool())
        .await
        .map_err(server_missing)?;
        not_found_if_none(result.rows_affected(), "request", id)
    }

    pub async fn delete_request(&self, id: &str) -> DbResult<()> {
        let result = sqlx::query("DELETE FROM requests WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await?;
        not_found_if_none(result.rows_affected(), "request", id)
    }

    // --- export / import ---

    /// Serializes a folder (with subfolders and requests) to the portable JSON format.
    pub async fn export_collection(&self, id: &str) -> DbResult<String> {
        let tree = self.tree().await?;
        let servers: HashMap<String, String> =
            sqlx::query_as::<_, (String, String)>("SELECT id, name FROM servers")
                .fetch_all(self.db.pool())
                .await?
                .into_iter()
                .collect();
        let root = tree
            .collections
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| DbError::NotFound(format!("collection {id}")))?;
        let folder = build_export(root, &tree, &servers);
        let file = ExportFile {
            format: FORMAT.into(),
            version: FORMAT_VERSION,
            collection: folder,
        };
        serde_json::to_string_pretty(&file).map_err(|e| DbError::Invalid(e.to_string()))
    }

    /// Imports a file produced by [`Self::export_collection`] under `parent_id`.
    pub async fn import_collection(
        &self,
        json: &str,
        parent_id: Option<&str>,
    ) -> DbResult<ImportReport> {
        let file: ExportFile = serde_json::from_str(json)
            .map_err(|e| DbError::Invalid(format!("not a collection file: {e}")))?;
        if file.format != FORMAT {
            return Err(DbError::Invalid(format!(
                "unexpected file format \"{}\"",
                file.format
            )));
        }
        if file.version > FORMAT_VERSION {
            return Err(DbError::Invalid(format!(
                "the file uses format version {}, this app understands up to {FORMAT_VERSION}",
                file.version
            )));
        }
        let servers: HashMap<String, String> =
            sqlx::query_as::<_, (String, String)>("SELECT LOWER(name), id FROM servers")
                .fetch_all(self.db.pool())
                .await?
                .into_iter()
                .collect();
        let mut report = ImportReport {
            collection_id: String::new(),
            collections_created: 0,
            requests_created: 0,
            skipped: vec![],
        };
        report.collection_id = self
            .import_folder(&file.collection, parent_id, &servers, &mut report)
            .await?;
        Ok(report)
    }

    fn import_folder<'a>(
        &'a self,
        folder: &'a ExportFolder,
        parent_id: Option<&'a str>,
        servers: &'a HashMap<String, String>,
        report: &'a mut ImportReport,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = DbResult<String>> + Send + 'a>> {
        Box::pin(async move {
            let node = self.create_collection(parent_id, &folder.name).await?;
            report.collections_created += 1;
            for request in &folder.requests {
                let Some(server_id) = servers.get(&request.server_name.to_lowercase()) else {
                    report.skipped.push(format!(
                        "\"{}\": unknown server \"{}\"",
                        request.name, request.server_name
                    ));
                    continue;
                };
                self.save_request(SavedRequestInput {
                    collection_id: node.id.clone(),
                    server_id: server_id.clone(),
                    method: request.method.clone(),
                    name: request.name.clone(),
                    tool_name: request.tool_name.clone(),
                    arguments: JsonValue(request.arguments.clone()),
                    notes: request.notes.clone(),
                })
                .await?;
                report.requests_created += 1;
            }
            for child in &folder.children {
                self.import_folder(child, Some(&node.id), servers, report)
                    .await?;
            }
            Ok(node.id)
        })
    }

    async fn require_collection(&self, id: &str) -> DbResult<()> {
        let exists: Option<String> = sqlx::query_scalar("SELECT id FROM collections WHERE id = ?")
            .bind(id)
            .fetch_optional(self.db.pool())
            .await?;
        exists
            .map(|_| ())
            .ok_or_else(|| DbError::NotFound(format!("collection {id}")))
    }

    async fn next_order(&self, parent: Option<&str>) -> DbResult<i64> {
        Ok(sqlx::query_scalar(
            "SELECT COALESCE(MAX(sort_order) + 1, 0) FROM collections WHERE parent_id IS ?",
        )
        .bind(parent)
        .fetch_one(self.db.pool())
        .await?)
    }
}

fn build_export(
    node: &CollectionNode,
    tree: &CollectionTree,
    servers: &HashMap<String, String>,
) -> ExportFolder {
    ExportFolder {
        name: node.name.clone(),
        requests: tree
            .requests
            .iter()
            .filter(|r| r.input.collection_id == node.id)
            .map(|r| ExportRequest {
                name: r.input.name.clone(),
                server_name: servers.get(&r.input.server_id).cloned().unwrap_or_default(),
                method: r.input.method.clone(),
                tool_name: r.input.tool_name.clone(),
                arguments: r.input.arguments.0.clone(),
                notes: r.input.notes.clone(),
            })
            .collect(),
        children: tree
            .collections
            .iter()
            .filter(|c| c.parent_id.as_deref() == Some(node.id.as_str()))
            .map(|c| build_export(c, tree, servers))
            .collect(),
    }
}

fn clean_name(name: &str) -> DbResult<String> {
    let name = name.trim();
    if name.is_empty() {
        Err(DbError::Invalid("name is required".into()))
    } else {
        Ok(name.to_owned())
    }
}

fn validate_request(mut input: SavedRequestInput) -> DbResult<SavedRequestInput> {
    input.name = clean_name(&input.name)?;
    if input.method.trim().is_empty() {
        return Err(DbError::Invalid("method is required".into()));
    }
    if input.method == "tools/call" && input.tool_name.as_deref().unwrap_or("").is_empty() {
        return Err(DbError::Invalid(
            "a tool name is required for tool calls".into(),
        ));
    }
    Ok(input)
}

fn not_found_if_none(rows: u64, what: &str, id: &str) -> DbResult<()> {
    if rows == 0 {
        Err(DbError::NotFound(format!("{what} {id}")))
    } else {
        Ok(())
    }
}

fn server_missing(error: sqlx::Error) -> DbError {
    match &error {
        sqlx::Error::Database(db) if db.is_foreign_key_violation() => {
            DbError::NotFound("the request's server no longer exists".into())
        }
        _ => DbError::Sql(error),
    }
}

/// Unused helper kept private: groups requests by folder (used by tests).
#[allow(dead_code)]
fn group_by_collection(requests: &[SavedRequest]) -> BTreeMap<&str, Vec<&SavedRequest>> {
    let mut map: BTreeMap<&str, Vec<&SavedRequest>> = BTreeMap::new();
    for request in requests {
        map.entry(request.input.collection_id.as_str())
            .or_default()
            .push(request);
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Registry, ServerInput, TransportKind};
    use serde_json::json;

    async fn setup() -> (Collections, Registry, String) {
        let db = Db::open_in_memory().await.unwrap();
        let registry = Registry::new(db.clone());
        let server = registry
            .create(ServerInput {
                name: "Everything".into(),
                transport: TransportKind::Stdio,
                command: Some("x".into()),
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
            })
            .await
            .unwrap();
        (Collections::new(db), registry, server.id)
    }

    fn request(collection: &str, server: &str, name: &str) -> SavedRequestInput {
        SavedRequestInput {
            collection_id: collection.into(),
            server_id: server.into(),
            method: "tools/call".into(),
            name: name.into(),
            tool_name: Some("echo".into()),
            arguments: JsonValue(json!({"message": "hi"})),
            notes: "note".into(),
        }
    }

    #[tokio::test]
    async fn builds_a_tree_of_folders_and_requests() {
        let (repo, _, server) = setup().await;
        let root = repo.create_collection(None, "Smoke tests").await.unwrap();
        let child = repo
            .create_collection(Some(&root.id), "Echo")
            .await
            .unwrap();
        repo.save_request(request(&child.id, &server, "hello"))
            .await
            .unwrap();
        let tree = repo.tree().await.unwrap();
        assert_eq!(tree.collections.len(), 2);
        let child_node = tree.collections.iter().find(|c| c.id == child.id).unwrap();
        assert_eq!(child_node.parent_id.as_deref(), Some(root.id.as_str()));
        assert_eq!(tree.requests[0].input.arguments.0["message"], "hi");
        assert_eq!(
            group_by_collection(&tree.requests)[child.id.as_str()].len(),
            1
        );
    }

    #[tokio::test]
    async fn rename_move_and_delete() {
        let (repo, _, server) = setup().await;
        let a = repo.create_collection(None, "A").await.unwrap();
        let b = repo.create_collection(None, "B").await.unwrap();
        repo.rename_collection(&a.id, "  Renamed ").await.unwrap();
        repo.move_collection(&b.id, Some(&a.id)).await.unwrap();
        let req = repo
            .save_request(request(&b.id, &server, "r"))
            .await
            .unwrap();
        let tree = repo.tree().await.unwrap();
        assert_eq!(
            tree.collections.iter().find(|c| c.id == a.id).unwrap().name,
            "Renamed"
        );
        assert_eq!(
            tree.collections
                .iter()
                .find(|c| c.id == b.id)
                .unwrap()
                .parent_id
                .as_deref(),
            Some(a.id.as_str())
        );
        repo.delete_collection(&a.id).await.unwrap();
        let tree = repo.tree().await.unwrap();
        assert!(
            tree.collections.is_empty() && tree.requests.is_empty(),
            "delete cascades: {req:?}"
        );
    }

    #[tokio::test]
    async fn moving_a_folder_into_its_descendant_is_rejected() {
        let (repo, _, _) = setup().await;
        let a = repo.create_collection(None, "A").await.unwrap();
        let b = repo.create_collection(Some(&a.id), "B").await.unwrap();
        let c = repo.create_collection(Some(&b.id), "C").await.unwrap();
        assert!(matches!(
            repo.move_collection(&a.id, Some(&c.id)).await,
            Err(DbError::Invalid(_))
        ));
        assert!(matches!(
            repo.move_collection(&a.id, Some(&a.id)).await,
            Err(DbError::Invalid(_))
        ));
        repo.move_collection(&c.id, None).await.unwrap();
    }

    #[tokio::test]
    async fn validates_input() {
        let (repo, _, server) = setup().await;
        assert!(repo.create_collection(None, "  ").await.is_err());
        assert!(repo.create_collection(Some("missing"), "x").await.is_err());
        let folder = repo.create_collection(None, "F").await.unwrap();
        let mut no_tool = request(&folder.id, &server, "x");
        no_tool.tool_name = None;
        assert!(repo.save_request(no_tool).await.is_err());
        assert!(repo
            .save_request(request(&folder.id, "no-such-server", "x"))
            .await
            .is_err());
        assert!(repo
            .save_request(request("no-such-folder", &server, "x"))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn updates_and_deletes_requests() {
        let (repo, _, server) = setup().await;
        let folder = repo.create_collection(None, "F").await.unwrap();
        let saved = repo
            .save_request(request(&folder.id, &server, "one"))
            .await
            .unwrap();
        let mut changed = saved.input.clone();
        changed.name = "two".into();
        changed.arguments = JsonValue(json!({"message": "changed"}));
        repo.update_request(&saved.id, changed).await.unwrap();
        let tree = repo.tree().await.unwrap();
        assert_eq!(tree.requests[0].input.name, "two");
        assert_eq!(tree.requests[0].input.arguments.0["message"], "changed");
        repo.delete_request(&saved.id).await.unwrap();
        assert!(matches!(
            repo.delete_request(&saved.id).await,
            Err(DbError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn export_and_import_roundtrip_across_databases() {
        let (repo, _, server) = setup().await;
        let root = repo.create_collection(None, "Suite").await.unwrap();
        let child = repo
            .create_collection(Some(&root.id), "Nested")
            .await
            .unwrap();
        repo.save_request(request(&root.id, &server, "top"))
            .await
            .unwrap();
        repo.save_request(request(&child.id, &server, "deep"))
            .await
            .unwrap();
        let file = repo.export_collection(&root.id).await.unwrap();
        assert!(file.contains("\"serverName\": \"Everything\""));
        assert!(!file.contains(&server), "file must not contain local ids");

        // A second "machine" with a server of the same name (different id) and one request whose server is unknown.
        let (other, _, other_server) = setup().await;
        let mut edited: serde_json::Value = serde_json::from_str(&file).unwrap();
        edited["collection"]["requests"].as_array_mut().unwrap().push(json!({
            "name": "orphan", "serverName": "Missing", "method": "tools/call", "toolName": "x", "arguments": {}
        }));
        let report = other
            .import_collection(&edited.to_string(), None)
            .await
            .unwrap();
        assert_eq!(
            (report.collections_created, report.requests_created),
            (2, 2)
        );
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].contains("Missing"));
        let tree = other.tree().await.unwrap();
        assert!(tree
            .requests
            .iter()
            .all(|r| r.input.server_id == other_server));
        assert_eq!(tree.collections.len(), 2);
    }

    #[tokio::test]
    async fn import_rejects_foreign_or_newer_files() {
        let (repo, _, _) = setup().await;
        assert!(repo.import_collection("not json", None).await.is_err());
        assert!(repo
            .import_collection(
                r#"{"format":"other","version":1,"collection":{"name":"x"}}"#,
                None
            )
            .await
            .is_err());
        let error = repo
            .import_collection(
                r#"{"format":"mcp-studio-collection","version":99,"collection":{"name":"x"}}"#,
                None,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("version 99"), "{error}");
        assert!(repo.tree().await.unwrap().collections.is_empty());
    }
}
