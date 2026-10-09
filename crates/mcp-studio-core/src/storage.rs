//! The retention limits of recorded history, how much space the database takes, and deleting the
//! history by hand.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{
    db::{Db, DbError, DbResult},
    history::History,
    message_store::{self, RetentionPolicy},
    settings::Settings,
};

const SETTINGS_KEY: &str = "retention";
/// Entries of the tool call history that are kept besides the age limit.
pub const MAX_HISTORY_ENTRIES: u32 = 5000;
const MAX_DAYS: u32 = 3650;
const MAX_MESSAGES: u32 = 10_000_000;

impl RetentionPolicy {
    /// Rejects limits that would delete everything or that no database could hold.
    pub fn validated(self) -> DbResult<Self> {
        if !(1..=MAX_DAYS).contains(&self.max_age_days) {
            return Err(DbError::Invalid(format!(
                "keep history for 1 to {MAX_DAYS} days"
            )));
        }
        if !(1..=MAX_MESSAGES).contains(&self.max_messages) {
            return Err(DbError::Invalid(format!(
                "keep 1 to {MAX_MESSAGES} messages"
            )));
        }
        Ok(self)
    }
}

/// The stored policy, or the default when none was saved (or the saved one cannot be read).
pub async fn load_policy(settings: &Settings) -> DbResult<RetentionPolicy> {
    Ok(settings
        .get(SETTINGS_KEY)
        .await?
        .and_then(|raw| serde_json::from_str::<RetentionPolicy>(&raw).ok())
        .and_then(|policy| policy.validated().ok())
        .unwrap_or_default())
}

pub async fn save_policy(
    settings: &Settings,
    policy: RetentionPolicy,
) -> DbResult<RetentionPolicy> {
    let policy = policy.validated()?;
    let raw = serde_json::to_string(&policy).map_err(|e| DbError::Invalid(e.to_string()))?;
    settings.set(SETTINGS_KEY, &raw).await?;
    Ok(policy)
}

/// Applies the policy to everything that is recorded. Returns the number of deleted messages.
pub async fn apply_policy(db: &Db, policy: RetentionPolicy) -> DbResult<u64> {
    let deleted = message_store::cleanup(db, policy).await?;
    History::new(db.clone())
        .cleanup(MAX_HISTORY_ENTRIES, policy.max_age_days)
        .await?;
    Ok(deleted)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    /// Size of the database file, including free pages.
    pub database_bytes: f64,
    pub messages: u32,
    pub history_entries: u32,
}

pub async fn storage_info(db: &Db) -> DbResult<StorageInfo> {
    let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(db.pool())
        .await?;
    let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
        .fetch_one(db.pool())
        .await?;
    let messages: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(db.pool())
        .await?;
    let history_entries: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM history")
        .fetch_one(db.pool())
        .await?;
    Ok(StorageInfo {
        database_bytes: (pages * page_size) as f64,
        messages: messages as u32,
        history_entries: history_entries as u32,
    })
}

/// Deletes every recorded message and the tool call history. Servers, flows and the like stay.
pub async fn delete_history(db: &Db) -> DbResult<u64> {
    let deleted = message_store::cleanup(
        db,
        RetentionPolicy {
            max_age_days: 0,
            max_messages: 0,
        },
    )
    .await?;
    History::new(db.clone()).clear(None).await?;
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(db.pool())
        .await?;
    sqlx::query("VACUUM").execute(db.pool()).await?;
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::now_ms;

    #[tokio::test]
    async fn defaults_until_a_policy_is_saved() {
        let settings = Settings::new(Db::open_in_memory().await.unwrap());
        assert_eq!(
            load_policy(&settings).await.unwrap(),
            RetentionPolicy::default()
        );
    }

    #[tokio::test]
    async fn stored_policy_is_read_back() {
        let settings = Settings::new(Db::open_in_memory().await.unwrap());
        let policy = RetentionPolicy {
            max_age_days: 7,
            max_messages: 500,
        };
        save_policy(&settings, policy).await.unwrap();
        assert_eq!(load_policy(&settings).await.unwrap(), policy);
    }

    #[tokio::test]
    async fn rejects_limits_that_delete_everything() {
        let settings = Settings::new(Db::open_in_memory().await.unwrap());
        for (max_age_days, max_messages) in [(0, 10), (10, 0), (100_000, 10)] {
            let policy = RetentionPolicy {
                max_age_days,
                max_messages,
            };
            assert!(save_policy(&settings, policy).await.is_err());
        }
        assert_eq!(
            load_policy(&settings).await.unwrap(),
            RetentionPolicy::default()
        );
    }

    #[tokio::test]
    async fn unreadable_stored_policy_falls_back_to_the_default() {
        let settings = Settings::new(Db::open_in_memory().await.unwrap());
        settings.set(SETTINGS_KEY, "not json").await.unwrap();
        assert_eq!(
            load_policy(&settings).await.unwrap(),
            RetentionPolicy::default()
        );
        settings
            .set(SETTINGS_KEY, r#"{"maxAgeDays":0}"#)
            .await
            .unwrap();
        assert_eq!(
            load_policy(&settings).await.unwrap(),
            RetentionPolicy::default()
        );
    }

    #[tokio::test]
    async fn saved_limits_decide_what_cleanup_deletes() {
        let db = Db::open_in_memory().await.unwrap();
        let settings = Settings::new(db.clone());
        let now = now_ms();
        sqlx::query("INSERT INTO servers (id, name, transport, command, created_at, updated_at) VALUES ('srv', 'S', 'stdio', 'x', ?, ?)")
            .bind(now).bind(now).execute(db.pool()).await.unwrap();
        sqlx::query("INSERT INTO sessions (id, server_id, origin, started_at) VALUES ('sess', 'srv', 'studio', ?)")
            .bind(now)
            .execute(db.pool())
            .await
            .unwrap();
        for age_days in [10, 3, 0] {
            sqlx::query("INSERT INTO messages (session_id, direction, payload, bytes, ts) VALUES ('sess', 'out', 'm', 1, ?)")
                .bind(now - age_days * 24 * 60 * 60 * 1000)
                .execute(db.pool()).await.unwrap();
        }
        let policy = save_policy(
            &settings,
            RetentionPolicy {
                max_age_days: 5,
                max_messages: 100,
            },
        )
        .await
        .unwrap();
        assert_eq!(apply_policy(&db, policy).await.unwrap(), 1);
        assert_eq!(storage_info(&db).await.unwrap().messages, 2);
    }

    #[tokio::test]
    async fn delete_history_empties_messages_and_history() {
        let db = Db::open_in_memory().await.unwrap();
        let now = now_ms();
        sqlx::query("INSERT INTO servers (id, name, transport, command, created_at, updated_at) VALUES ('srv', 'S', 'stdio', 'x', ?, ?)")
            .bind(now).bind(now).execute(db.pool()).await.unwrap();
        sqlx::query("INSERT INTO sessions (id, server_id, origin, started_at) VALUES ('sess', 'srv', 'studio', ?)")
            .bind(now)
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO messages (session_id, direction, payload, bytes, ts) VALUES ('sess', 'out', 'm', 1, ?)")
            .bind(now).execute(db.pool()).await.unwrap();
        assert_eq!(delete_history(&db).await.unwrap(), 1);
        let info = storage_info(&db).await.unwrap();
        assert_eq!((info.messages, info.history_entries), (0, 0));
        assert!(info.database_bytes > 0.0);
        let servers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM servers")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(servers, 1);
    }
}
