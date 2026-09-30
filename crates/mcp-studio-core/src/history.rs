//! History of everything the user asked a server to do (tool calls, resource reads, prompts).

use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::FromRow;

use crate::{
    db::{now_ms, Db, DbError, DbResult},
    model::JsonValue,
};

/// Results larger than this are not stored (only a marker is).
pub const MAX_STORED_RESULT_BYTES: usize = 256 * 1024;

/// A finished request, ready to be stored.
#[derive(Debug, Clone)]
pub struct NewEntry {
    pub server_id: String,
    /// MCP method: `tools/call`, `resources/read`, or `prompts/get`.
    pub method: String,
    /// Tool name, resource URI, or prompt name.
    pub target: String,
    /// Arguments as the user entered them (placeholders unresolved, so no secrets).
    pub arguments: serde_json::Value,
    pub is_error: bool,
    pub cancelled: bool,
    pub duration_ms: Option<i64>,
    /// Redacted result JSON.
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    #[specta(type = u32)]
    pub id: i64,
    pub server_id: String,
    pub method: String,
    pub target: String,
    pub arguments: JsonValue,
    pub is_error: bool,
    pub cancelled: bool,
    #[specta(type = Option<u32>)]
    pub duration_ms: Option<i64>,
    pub result: Option<JsonValue>,
    pub error: Option<String>,
    #[specta(type = u32)]
    pub ts: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct HistoryFilter {
    pub server_id: Option<String>,
    /// Case-insensitive match on the target, the arguments, and the error text.
    pub search: Option<String>,
    /// Page backwards: only entries older than this id.
    #[specta(type = u32)]
    pub before_id: Option<i64>,
    /// Default 100, maximum 1000. Newest first.
    pub limit: Option<u32>,
}

#[derive(FromRow)]
struct Row {
    id: i64,
    server_id: String,
    method: String,
    target: String,
    arguments: String,
    is_error: bool,
    cancelled: bool,
    duration_ms: Option<i64>,
    result: Option<String>,
    error: Option<String>,
    ts: i64,
}

impl TryFrom<Row> for HistoryEntry {
    type Error = DbError;

    fn try_from(row: Row) -> DbResult<Self> {
        let parse = |text: &str| {
            serde_json::from_str(text)
                .map_err(|e| DbError::Invalid(format!("corrupt history entry {}: {e}", row.id)))
        };
        Ok(Self {
            id: row.id,
            arguments: JsonValue(parse(&row.arguments)?),
            result: row.result.as_deref().map(parse).transpose()?.map(JsonValue),
            server_id: row.server_id,
            method: row.method,
            target: row.target,
            is_error: row.is_error,
            cancelled: row.cancelled,
            duration_ms: row.duration_ms,
            error: row.error,
            ts: row.ts,
        })
    }
}

#[derive(Clone, Debug)]
pub struct History {
    db: Db,
}

impl History {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn record(&self, entry: NewEntry) -> DbResult<i64> {
        let result = entry.result.map(|value| {
            let text = value.to_string();
            if text.len() > MAX_STORED_RESULT_BYTES {
                serde_json::json!({"truncated": true, "bytes": text.len()}).to_string()
            } else {
                text
            }
        });
        Ok(sqlx::query_scalar(
            "INSERT INTO history (server_id, method, target, arguments, is_error, cancelled, duration_ms, result, error, ts) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(entry.server_id)
        .bind(entry.method)
        .bind(entry.target)
        .bind(entry.arguments.to_string())
        .bind(entry.is_error)
        .bind(entry.cancelled)
        .bind(entry.duration_ms)
        .bind(result)
        .bind(entry.error)
        .bind(now_ms())
        .fetch_one(self.db.pool())
        .await?)
    }

    pub async fn list(&self, filter: &HistoryFilter) -> DbResult<Vec<HistoryEntry>> {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT id, server_id, method, target, arguments, is_error, cancelled, duration_ms, result, error, ts \
             FROM history WHERE 1 = 1",
        );
        if let Some(server) = &filter.server_id {
            query.push(" AND server_id = ").push_bind(server.clone());
        }
        if let Some(id) = filter.before_id {
            query.push(" AND id < ").push_bind(id);
        }
        if let Some(text) = filter.search.as_ref().filter(|t| !t.trim().is_empty()) {
            let escaped = text
                .trim()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let pattern = format!("%{escaped}%");
            query
                .push(" AND (target LIKE ")
                .push_bind(pattern.clone())
                .push(" ESCAPE '\\' OR arguments LIKE ")
                .push_bind(pattern.clone())
                .push(" ESCAPE '\\' OR COALESCE(error, '') LIKE ")
                .push_bind(pattern)
                .push(" ESCAPE '\\')");
        }
        query
            .push(" ORDER BY id DESC LIMIT ")
            .push_bind(i64::from(filter.limit.unwrap_or(100).clamp(1, 1000)));
        let rows: Vec<Row> = query.build_query_as().fetch_all(self.db.pool()).await?;
        rows.into_iter().map(HistoryEntry::try_from).collect()
    }

    pub async fn get(&self, id: i64) -> DbResult<HistoryEntry> {
        let row: Option<Row> = sqlx::query_as(
            "SELECT id, server_id, method, target, arguments, is_error, cancelled, duration_ms, result, error, ts \
             FROM history WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.db.pool())
        .await?;
        row.ok_or_else(|| DbError::NotFound(format!("history entry {id}")))?
            .try_into()
    }

    /// Deletes the history of one server, or everything. Returns the number of deleted entries.
    pub async fn clear(&self, server_id: Option<&str>) -> DbResult<u64> {
        let result = match server_id {
            Some(id) => {
                sqlx::query("DELETE FROM history WHERE server_id = ?")
                    .bind(id)
                    .execute(self.db.pool())
                    .await?
            }
            None => {
                sqlx::query("DELETE FROM history")
                    .execute(self.db.pool())
                    .await?
            }
        };
        Ok(result.rows_affected())
    }

    /// Keeps at most `max_entries` and nothing older than `max_age_days`.
    pub async fn cleanup(&self, max_entries: u32, max_age_days: u32) -> DbResult<u64> {
        let cutoff = now_ms() - i64::from(max_age_days) * 24 * 60 * 60 * 1000;
        let mut deleted = sqlx::query("DELETE FROM history WHERE ts < ?")
            .bind(cutoff)
            .execute(self.db.pool())
            .await?
            .rows_affected();
        deleted += sqlx::query("DELETE FROM history WHERE id <= (SELECT id FROM history ORDER BY id DESC LIMIT 1 OFFSET ?)")
            .bind(i64::from(max_entries))
            .execute(self.db.pool())
            .await?
            .rows_affected();
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn setup() -> (History, Db) {
        let db = Db::open_in_memory().await.unwrap();
        let now = now_ms();
        for id in ["s1", "s2"] {
            sqlx::query("INSERT INTO servers (id, name, transport, command, created_at, updated_at) VALUES (?, ?, 'stdio', 'x', ?, ?)")
                .bind(id).bind(id).bind(now).bind(now).execute(db.pool()).await.unwrap();
        }
        (History::new(db.clone()), db)
    }

    fn entry(server: &str, target: &str, args: serde_json::Value) -> NewEntry {
        NewEntry {
            server_id: server.into(),
            method: "tools/call".into(),
            target: target.into(),
            arguments: args,
            is_error: false,
            cancelled: false,
            duration_ms: Some(12),
            result: Some(json!({"content": [{"type": "text", "text": "ok"}]})),
            error: None,
        }
    }

    #[tokio::test]
    async fn records_and_lists_newest_first() {
        let (history, _) = setup().await;
        history
            .record(entry("s1", "echo", json!({"message": "one"})))
            .await
            .unwrap();
        history
            .record(entry("s1", "add", json!({"a": 1})))
            .await
            .unwrap();
        let entries = history.list(&HistoryFilter::default()).await.unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|e| e.target.as_str())
                .collect::<Vec<_>>(),
            ["add", "echo"]
        );
        assert_eq!(entries[1].arguments.0["message"], "one");
        assert_eq!(
            entries[0].result.as_ref().unwrap().0["content"][0]["text"],
            "ok"
        );
        assert_eq!(entries[0].duration_ms, Some(12));
    }

    #[tokio::test]
    async fn filters_by_server_search_and_paging() {
        let (history, _) = setup().await;
        history
            .record(entry("s1", "echo", json!({"message": "hello"})))
            .await
            .unwrap();
        history
            .record(entry("s2", "echo", json!({"message": "other"})))
            .await
            .unwrap();
        let mut failed = entry("s1", "add", json!({}));
        failed.is_error = true;
        failed.error = Some("Connection refused".into());
        history.record(failed).await.unwrap();

        let s1 = history
            .list(&HistoryFilter {
                server_id: Some("s1".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(s1.len(), 2);
        let by_args = history
            .list(&HistoryFilter {
                search: Some("HELLO".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(by_args.len(), 1);
        let by_error = history
            .list(&HistoryFilter {
                search: Some("refused".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(by_error[0].target, "add");
        assert!(history
            .list(&HistoryFilter {
                search: Some("100%".into()),
                ..Default::default()
            })
            .await
            .unwrap()
            .is_empty());

        let first_page = history
            .list(&HistoryFilter {
                limit: Some(2),
                ..Default::default()
            })
            .await
            .unwrap();
        let second = history
            .list(&HistoryFilter {
                before_id: first_page.last().map(|e| e.id),
                limit: Some(2),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!((first_page.len(), second.len()), (2, 1));
    }

    #[tokio::test]
    async fn get_clear_and_deleting_a_server_removes_its_history() {
        let (history, db) = setup().await;
        let id = history
            .record(entry("s1", "echo", json!({})))
            .await
            .unwrap();
        history
            .record(entry("s2", "echo", json!({})))
            .await
            .unwrap();
        assert_eq!(history.get(id).await.unwrap().target, "echo");
        assert!(matches!(history.get(9999).await, Err(DbError::NotFound(_))));

        sqlx::query("DELETE FROM servers WHERE id = 's2'")
            .execute(db.pool())
            .await
            .unwrap();
        assert_eq!(
            history.list(&HistoryFilter::default()).await.unwrap().len(),
            1
        );
        assert_eq!(history.clear(Some("s1")).await.unwrap(), 1);
        history.record(entry("s1", "a", json!({}))).await.unwrap();
        assert_eq!(history.clear(None).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn oversized_results_are_replaced_by_a_marker() {
        let (history, _) = setup().await;
        let mut big = entry("s1", "dump", json!({}));
        big.result = Some(json!({"blob": "x".repeat(MAX_STORED_RESULT_BYTES + 10)}));
        let id = history.record(big).await.unwrap();
        let stored = history.get(id).await.unwrap().result.unwrap().0;
        assert_eq!(stored["truncated"], true);
    }

    #[tokio::test]
    async fn cleanup_enforces_count_and_age() {
        let (history, db) = setup().await;
        for i in 0..5 {
            history
                .record(entry("s1", &format!("t{i}"), json!({})))
                .await
                .unwrap();
        }
        sqlx::query("UPDATE history SET ts = ts - ? WHERE target = 't0'")
            .bind(40_i64 * 24 * 60 * 60 * 1000)
            .execute(db.pool())
            .await
            .unwrap();
        assert_eq!(history.cleanup(100, 30).await.unwrap(), 1);
        assert_eq!(history.cleanup(2, 30).await.unwrap(), 2);
        let left: Vec<_> = history
            .list(&HistoryFilter::default())
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.target)
            .collect();
        assert_eq!(left, ["t4", "t3"]);
    }
}
