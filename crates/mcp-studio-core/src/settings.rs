//! Small string settings stored in the database.

use crate::db::{Db, DbResult};

#[derive(Clone, Debug)]
pub struct Settings {
    db: Db,
}

impl Settings {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn get(&self, key: &str) -> DbResult<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
                .bind(key)
                .fetch_optional(self.db.pool())
                .await?,
        )
    }

    pub async fn set(&self, key: &str, value: &str) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?, ?) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(self.db.pool())
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stores_and_replaces_values() {
        let settings = Settings::new(Db::open_in_memory().await.unwrap());
        assert_eq!(settings.get("k").await.unwrap(), None);
        settings.set("k", "one").await.unwrap();
        settings.set("k", "two").await.unwrap();
        assert_eq!(settings.get("k").await.unwrap().as_deref(), Some("two"));
    }
}
