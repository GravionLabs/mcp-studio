//! Server registry: CRUD for MCP server definitions.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::FromRow;

use crate::db::{new_id, now_ms, Db, DbError, DbResult};

/// How a server is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum TransportKind {
    Stdio,
    Http,
}

impl TransportKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
        }
    }
}

/// What the user edits: everything except identity and timestamps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ServerInput {
    pub name: String,
    pub transport: TransportKind,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<String>,
    pub url: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub tags: Vec<String>,
}

/// A stored server definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ServerDefinition {
    pub id: String,
    #[serde(flatten)]
    pub input: ServerInput,
    /// Unix milliseconds. Exported to TypeScript as `number` (specta forbids `i64`).
    #[specta(type = u32)]
    pub created_at: i64,
    #[specta(type = u32)]
    pub updated_at: i64,
}

impl ServerInput {
    /// Trims text fields and checks the rules for the chosen transport.
    pub fn normalized(mut self) -> DbResult<Self> {
        self.name = self.name.trim().to_owned();
        if self.name.is_empty() {
            return Err(DbError::Invalid("name is required".into()));
        }
        self.command = clean(self.command);
        self.cwd = clean(self.cwd);
        self.url = clean(self.url);
        self.tags = self
            .tags
            .into_iter()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect();
        if self
            .env
            .keys()
            .chain(self.headers.keys())
            .any(|k| k.trim().is_empty())
        {
            return Err(DbError::Invalid(
                "environment and header names must not be empty".into(),
            ));
        }
        match self.transport {
            TransportKind::Stdio => {
                if self.command.is_none() {
                    return Err(DbError::Invalid(
                        "a command is required for stdio servers".into(),
                    ));
                }
                self.url = None;
                self.headers.clear();
            }
            TransportKind::Http => {
                let raw = self
                    .url
                    .as_deref()
                    .ok_or_else(|| DbError::Invalid("a URL is required for HTTP servers".into()))?;
                let parsed = url::Url::parse(raw)
                    .map_err(|e| DbError::Invalid(format!("invalid URL: {e}")))?;
                if !matches!(parsed.scheme(), "http" | "https") {
                    return Err(DbError::Invalid(
                        "URL must start with http:// or https://".into(),
                    ));
                }
                self.command = None;
                self.args.clear();
                self.env.clear();
                self.cwd = None;
            }
        }
        Ok(self)
    }
}

fn clean(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

#[derive(FromRow)]
struct Row {
    id: String,
    name: String,
    transport: String,
    command: Option<String>,
    args: String,
    env: String,
    cwd: Option<String>,
    url: Option<String>,
    headers: String,
    tags: String,
    created_at: i64,
    updated_at: i64,
}

impl TryFrom<Row> for ServerDefinition {
    type Error = DbError;

    fn try_from(row: Row) -> DbResult<Self> {
        let id = row.id.clone();
        let json = |e: serde_json::Error| DbError::Invalid(format!("corrupt server row {id}: {e}"));
        Ok(Self {
            id: row.id.clone(),
            input: ServerInput {
                name: row.name.clone(),
                transport: if row.transport == "http" {
                    TransportKind::Http
                } else {
                    TransportKind::Stdio
                },
                command: row.command.clone(),
                args: serde_json::from_str(&row.args).map_err(json)?,
                env: serde_json::from_str(&row.env).map_err(json)?,
                cwd: row.cwd.clone(),
                url: row.url.clone(),
                headers: serde_json::from_str(&row.headers).map_err(json)?,
                tags: serde_json::from_str(&row.tags).map_err(json)?,
            },
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

const SELECT_ALL: &str =
    "SELECT id, name, transport, command, args, env, cwd, url, headers, tags, \
     created_at, updated_at FROM servers ORDER BY name COLLATE NOCASE";
const SELECT_ONE: &str =
    "SELECT id, name, transport, command, args, env, cwd, url, headers, tags, \
     created_at, updated_at FROM servers WHERE id = ?";

/// Access to stored server definitions.
#[derive(Clone, Debug)]
pub struct Registry {
    db: Db,
}

impl Registry {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn list(&self) -> DbResult<Vec<ServerDefinition>> {
        let rows: Vec<Row> = sqlx::query_as(SELECT_ALL).fetch_all(self.db.pool()).await?;
        rows.into_iter().map(ServerDefinition::try_from).collect()
    }

    pub async fn get(&self, id: &str) -> DbResult<ServerDefinition> {
        let row: Option<Row> = sqlx::query_as(SELECT_ONE)
            .bind(id)
            .fetch_optional(self.db.pool())
            .await?;
        row.ok_or_else(|| DbError::NotFound(format!("server {id}")))?
            .try_into()
    }

    pub async fn create(&self, input: ServerInput) -> DbResult<ServerDefinition> {
        let input = input.normalized()?;
        self.ensure_unique_name(&input.name, None).await?;
        let id = new_id();
        let now = now_ms();
        sqlx::query(
            "INSERT INTO servers (id, name, transport, command, args, env, cwd, url, headers, tags, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&input.name)
        .bind(input.transport.as_str())
        .bind(&input.command)
        .bind(serde_json::to_string(&input.args).unwrap_or_default())
        .bind(serde_json::to_string(&input.env).unwrap_or_default())
        .bind(&input.cwd)
        .bind(&input.url)
        .bind(serde_json::to_string(&input.headers).unwrap_or_default())
        .bind(serde_json::to_string(&input.tags).unwrap_or_default())
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await?;
        self.get(&id).await
    }

    pub async fn update(&self, id: &str, input: ServerInput) -> DbResult<ServerDefinition> {
        let input = input.normalized()?;
        self.ensure_unique_name(&input.name, Some(id)).await?;
        let result = sqlx::query(
            "UPDATE servers SET name = ?, transport = ?, command = ?, args = ?, env = ?, cwd = ?, url = ?, \
             headers = ?, tags = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&input.name)
        .bind(input.transport.as_str())
        .bind(&input.command)
        .bind(serde_json::to_string(&input.args).unwrap_or_default())
        .bind(serde_json::to_string(&input.env).unwrap_or_default())
        .bind(&input.cwd)
        .bind(&input.url)
        .bind(serde_json::to_string(&input.headers).unwrap_or_default())
        .bind(serde_json::to_string(&input.tags).unwrap_or_default())
        .bind(now_ms())
        .bind(id)
        .execute(self.db.pool())
        .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("server {id}")));
        }
        self.get(id).await
    }

    pub async fn delete(&self, id: &str) -> DbResult<()> {
        let result = sqlx::query("DELETE FROM servers WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("server {id}")));
        }
        Ok(())
    }

    async fn ensure_unique_name(&self, name: &str, except: Option<&str>) -> DbResult<()> {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM servers WHERE name = ? COLLATE NOCASE AND id != ?")
                .bind(name)
                .bind(except.unwrap_or(""))
                .fetch_optional(self.db.pool())
                .await?;
        match existing {
            Some(_) => Err(DbError::Invalid(format!(
                "a server named \"{name}\" already exists"
            ))),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn stdio(name: &str) -> ServerInput {
        ServerInput {
            name: name.into(),
            transport: TransportKind::Stdio,
            command: Some("npx".into()),
            args: vec!["-y".into(), "server-everything".into()],
            env: BTreeMap::from([("DEBUG".into(), "1".into())]),
            cwd: None,
            url: None,
            headers: BTreeMap::new(),
            tags: vec!["dev".into()],
        }
    }

    fn http(name: &str, url: &str) -> ServerInput {
        ServerInput {
            name: name.into(),
            transport: TransportKind::Http,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            cwd: None,
            url: Some(url.into()),
            headers: BTreeMap::from([("Authorization".into(), "keyring:x".into())]),
            tags: vec![],
        }
    }

    async fn registry() -> Registry {
        Registry::new(Db::open_in_memory().await.unwrap())
    }

    #[tokio::test]
    async fn create_and_get_roundtrip() {
        let registry = registry().await;
        let created = registry.create(stdio("Everything")).await.unwrap();
        assert_eq!(registry.get(&created.id).await.unwrap(), created);
        assert_eq!(created.input.args, vec!["-y", "server-everything"]);
        assert_eq!(created.input.env["DEBUG"], "1");
        assert!(created.created_at > 0);
    }

    #[tokio::test]
    async fn list_is_sorted_case_insensitively() {
        let registry = registry().await;
        for name in ["beta", "Alpha", "charlie"] {
            registry.create(stdio(name)).await.unwrap();
        }
        let names: Vec<_> = registry
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.input.name)
            .collect();
        assert_eq!(names, ["Alpha", "beta", "charlie"]);
    }

    #[tokio::test]
    async fn update_changes_fields_and_bumps_timestamp() {
        let registry = registry().await;
        let created = registry.create(stdio("One")).await.unwrap();
        let updated = registry
            .update(&created.id, http("One", "https://example.com/mcp"))
            .await
            .unwrap();
        assert_eq!(updated.input.transport, TransportKind::Http);
        assert_eq!(updated.input.command, None);
        assert!(updated.input.args.is_empty());
        assert!(updated.updated_at >= created.updated_at);
        assert_eq!(updated.created_at, created.created_at);
    }

    #[tokio::test]
    async fn delete_removes_and_reports_missing() {
        let registry = registry().await;
        let created = registry.create(stdio("Gone")).await.unwrap();
        registry.delete(&created.id).await.unwrap();
        assert!(matches!(
            registry.get(&created.id).await,
            Err(DbError::NotFound(_))
        ));
        assert!(matches!(
            registry.delete(&created.id).await,
            Err(DbError::NotFound(_))
        ));
        assert!(matches!(
            registry.update("nope", stdio("x")).await,
            Err(DbError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn names_must_be_unique_ignoring_case() {
        let registry = registry().await;
        let first = registry.create(stdio("Server")).await.unwrap();
        assert!(matches!(
            registry.create(stdio("server")).await,
            Err(DbError::Invalid(_))
        ));
        // Renaming a server to its own name (or case variant) is fine.
        registry.update(&first.id, stdio("SERVER")).await.unwrap();
    }

    #[tokio::test]
    async fn validation_rejects_bad_input() {
        let registry = registry().await;
        assert!(registry.create(stdio("   ")).await.is_err());
        let mut no_command = stdio("a");
        no_command.command = Some("  ".into());
        assert!(registry.create(no_command).await.is_err());
        assert!(registry.create(http("b", "not a url")).await.is_err());
        assert!(registry
            .create(http("c", "ftp://example.com"))
            .await
            .is_err());
        let mut bad_header = http("d", "http://localhost:3000/mcp");
        bad_header.headers.insert(" ".into(), "v".into());
        assert!(registry.create(bad_header).await.is_err());
    }

    #[test]
    fn normalization_trims_and_drops_irrelevant_fields() {
        let mut input = stdio("  spaced  ");
        input.tags = vec![" a ".into(), "".into()];
        input.url = Some("http://ignored".into());
        input.headers.insert("X".into(), "y".into());
        let normalized = input.normalized().unwrap();
        assert_eq!(normalized.name, "spaced");
        assert_eq!(normalized.tags, vec!["a"]);
        assert_eq!(normalized.url, None);
        assert!(normalized.headers.is_empty());
    }
}
