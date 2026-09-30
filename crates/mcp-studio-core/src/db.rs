//! SQLite storage: connection pool, migrations, and small helpers.

use std::{path::Path, str::FromStr, time::Duration};

use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    SqlitePool,
};

/// Errors from the storage layer.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid input: {0}")]
    Invalid(String),
}

pub type DbResult<T> = Result<T, DbError>;

/// Handle to the app database. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    /// Opens (creating if needed) the database file and applies pending migrations.
    pub async fn open(path: &Path) -> DbResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        Self::migrate(pool).await
    }

    /// Opens a private in-memory database (used by tests).
    pub async fn open_in_memory() -> DbResult<Self> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        // One connection: every connection to `:memory:` would otherwise be its own database.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        Self::migrate(pool).await
    }

    async fn migrate(pool: SqlitePool) -> DbResult<Self> {
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// Current time in unix milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// A fresh random id.
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn table_names(db: &Db) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE '\\_%' ESCAPE '\\' \
             AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .fetch_all(db.pool())
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn migrations_create_all_tables() {
        let db = Db::open_in_memory().await.unwrap();
        let tables = table_names(&db).await;
        for expected in [
            "collections",
            "environments",
            "flows",
            "messages",
            "prices",
            "requests",
            "servers",
            "sessions",
            "spans",
        ] {
            assert!(
                tables.iter().any(|t| t == expected),
                "missing {expected}: {tables:?}"
            );
        }
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced() {
        let db = Db::open_in_memory().await.unwrap();
        let result = sqlx::query(
            "INSERT INTO sessions (id, server_id, origin, started_at) VALUES ('s', 'missing', 'studio', 0)",
        )
        .execute(db.pool())
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn check_constraints_reject_bad_values() {
        let db = Db::open_in_memory().await.unwrap();
        let result = sqlx::query(
            "INSERT INTO servers (id, name, transport, created_at, updated_at) VALUES ('a', 'n', 'carrier-pigeon', 0, 0)",
        )
        .execute(db.pool())
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn file_database_persists_and_migrations_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/studio.sqlite");
        {
            let db = Db::open(&path).await.unwrap();
            sqlx::query("INSERT INTO environments (id, name) VALUES ('e', 'dev')")
                .execute(db.pool())
                .await
                .unwrap();
        }
        let db = Db::open(&path).await.unwrap();
        let name: String = sqlx::query_scalar("SELECT name FROM environments WHERE id = 'e'")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(name, "dev");
    }

    #[test]
    fn ids_are_unique_and_clock_moves_forward() {
        assert_ne!(new_id(), new_id());
        assert!(now_ms() > 1_700_000_000_000);
    }
}
