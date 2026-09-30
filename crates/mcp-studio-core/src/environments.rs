//! Named sets of variables for `{{placeholder}}` resolution (like Postman environments).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::FromRow;

use crate::db::{new_id, Db, DbError, DbResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentInput {
    pub name: String,
    /// Values are plain text or `keyring:` references.
    pub variables: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub id: String,
    #[serde(flatten)]
    pub input: EnvironmentInput,
}

impl EnvironmentInput {
    pub fn normalized(mut self) -> DbResult<Self> {
        self.name = self.name.trim().to_owned();
        if self.name.is_empty() {
            return Err(DbError::Invalid("name is required".into()));
        }
        let mut cleaned = BTreeMap::new();
        for (key, value) in self.variables {
            let key = key.trim().to_owned();
            if key.is_empty() {
                return Err(DbError::Invalid("variable names must not be empty".into()));
            }
            if !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
            {
                return Err(DbError::Invalid(format!(
                    "invalid variable name \"{key}\" (use letters, digits, _ - .)"
                )));
            }
            cleaned.insert(key, value);
        }
        self.variables = cleaned;
        Ok(self)
    }
}

#[derive(FromRow)]
struct Row {
    id: String,
    name: String,
    variables: String,
}

impl TryFrom<Row> for Environment {
    type Error = DbError;

    fn try_from(row: Row) -> DbResult<Self> {
        let variables = serde_json::from_str(&row.variables)
            .map_err(|e| DbError::Invalid(format!("corrupt environment {}: {e}", row.id)))?;
        Ok(Self {
            id: row.id,
            input: EnvironmentInput {
                name: row.name,
                variables,
            },
        })
    }
}

#[derive(Clone, Debug)]
pub struct Environments {
    db: Db,
}

impl Environments {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn list(&self) -> DbResult<Vec<Environment>> {
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, name, variables FROM environments ORDER BY name COLLATE NOCASE",
        )
        .fetch_all(self.db.pool())
        .await?;
        rows.into_iter().map(Environment::try_from).collect()
    }

    pub async fn get(&self, id: &str) -> DbResult<Environment> {
        let row: Option<Row> =
            sqlx::query_as("SELECT id, name, variables FROM environments WHERE id = ?")
                .bind(id)
                .fetch_optional(self.db.pool())
                .await?;
        row.ok_or_else(|| DbError::NotFound(format!("environment {id}")))?
            .try_into()
    }

    pub async fn create(&self, input: EnvironmentInput) -> DbResult<Environment> {
        let input = input.normalized()?;
        let id = new_id();
        sqlx::query("INSERT INTO environments (id, name, variables) VALUES (?, ?, ?)")
            .bind(&id)
            .bind(&input.name)
            .bind(serde_json::to_string(&input.variables).unwrap_or_default())
            .execute(self.db.pool())
            .await
            .map_err(unique_name)?;
        self.get(&id).await
    }

    pub async fn update(&self, id: &str, input: EnvironmentInput) -> DbResult<Environment> {
        let input = input.normalized()?;
        let result = sqlx::query("UPDATE environments SET name = ?, variables = ? WHERE id = ?")
            .bind(&input.name)
            .bind(serde_json::to_string(&input.variables).unwrap_or_default())
            .bind(id)
            .execute(self.db.pool())
            .await
            .map_err(unique_name)?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("environment {id}")));
        }
        self.get(id).await
    }

    pub async fn delete(&self, id: &str) -> DbResult<()> {
        let result = sqlx::query("DELETE FROM environments WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("environment {id}")));
        }
        Ok(())
    }
}

fn unique_name(error: sqlx::Error) -> DbError {
    match &error {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            DbError::Invalid("an environment with this name already exists".into())
        }
        _ => DbError::Sql(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str) -> EnvironmentInput {
        EnvironmentInput {
            name: name.into(),
            variables: BTreeMap::from([("host".into(), "localhost".into())]),
        }
    }

    async fn repo() -> Environments {
        Environments::new(Db::open_in_memory().await.unwrap())
    }

    #[tokio::test]
    async fn crud_roundtrip() {
        let repo = repo().await;
        let created = repo.create(input("dev")).await.unwrap();
        assert_eq!(repo.get(&created.id).await.unwrap(), created);
        let mut changed = input("dev");
        changed.variables.insert("port".into(), "3000".into());
        let updated = repo.update(&created.id, changed).await.unwrap();
        assert_eq!(updated.input.variables.len(), 2);
        repo.delete(&created.id).await.unwrap();
        assert!(repo.list().await.unwrap().is_empty());
        assert!(matches!(
            repo.delete(&created.id).await,
            Err(DbError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn list_is_sorted_and_names_are_unique() {
        let repo = repo().await;
        repo.create(input("prod")).await.unwrap();
        repo.create(input("Dev")).await.unwrap();
        let names: Vec<_> = repo
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.input.name)
            .collect();
        assert_eq!(names, ["Dev", "prod"]);
        assert!(matches!(
            repo.create(input("prod")).await,
            Err(DbError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn rejects_bad_names() {
        let repo = repo().await;
        assert!(repo.create(input("  ")).await.is_err());
        let mut bad = input("x");
        bad.variables.insert("has space".into(), "v".into());
        assert!(repo.create(bad).await.is_err());
        let mut empty = input("y");
        empty.variables.insert(" ".into(), "v".into());
        assert!(repo.create(empty).await.is_err());
    }
}
