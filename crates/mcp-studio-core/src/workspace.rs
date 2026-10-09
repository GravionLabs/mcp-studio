//! Backup and restore of the workspace: servers, environments, collections, flows, test suites and
//! prices in one JSON file.
//!
//! The file holds the rows of those tables as they are stored. Secrets are never stored in the
//! database, only `keyring:<name>` references, so the file carries the names of secrets and not
//! their values; after an import the names that are missing from the keyring are listed. History,
//! sessions, flow runs and the app's own settings stay on the machine.
//!
//! An import adds what is new and replaces rows with the same id (environments: the same name).
//! It never deletes. [`preview`] reports what an import would do without changing anything.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use specta::Type;
use sqlx::{Column, Row, SqliteConnection, TypeInfo, ValueRef};

use crate::{
    db::{now_ms, Db, DbError, DbResult},
    secrets::{reference_name, SecretStore},
};

const FORMAT: &str = "mcp-studio-workspace";
const VERSION: i64 = 1;
/// How many names a preview lists per table and kind.
const MAX_NAMES: usize = 50;

struct TableSpec {
    name: &'static str,
    /// What the table is called in the preview.
    label: &'static str,
    /// Primary key columns.
    key: &'static [&'static str],
    /// Columns that identify the same thing on another machine (defaults to `key`).
    same_as: &'static [&'static str],
    /// Column shown as the name of a row.
    title: &'static str,
}

/// In the order rows must exist in: parents before the rows that point at them.
const TABLES: &[TableSpec] = &[
    TableSpec {
        name: "servers",
        label: "Servers",
        key: &["id"],
        same_as: &["id"],
        title: "name",
    },
    TableSpec {
        name: "environments",
        label: "Environments",
        key: &["id"],
        same_as: &["name"],
        title: "name",
    },
    TableSpec {
        name: "collections",
        label: "Collections",
        key: &["id"],
        same_as: &["id"],
        title: "name",
    },
    TableSpec {
        name: "requests",
        label: "Saved requests",
        key: &["id"],
        same_as: &["id"],
        title: "name",
    },
    TableSpec {
        name: "flows",
        label: "Flows",
        key: &["id"],
        same_as: &["id"],
        title: "name",
    },
    TableSpec {
        name: "test_suites",
        label: "Test suites",
        key: &["id"],
        same_as: &["id"],
        title: "name",
    },
    TableSpec {
        name: "test_cases",
        label: "Test cases",
        key: &["id"],
        same_as: &["id"],
        title: "input",
    },
    TableSpec {
        name: "prices",
        label: "Prices",
        key: &["model"],
        same_as: &["model"],
        title: "model",
    },
];

type Row_ = Map<String, Value>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TableChanges {
    pub label: String,
    /// Names of rows the import would add (at most 50).
    pub added: Vec<String>,
    /// Names of rows the import would replace because they differ (at most 50).
    pub replaced: Vec<String>,
    pub added_count: u32,
    pub replaced_count: u32,
    pub unchanged_count: u32,
}

/// A secret the workspace refers to that is not in this computer's keyring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MissingSecret {
    pub name: String,
    /// Where it is used, for example "server GitHub" or "environment Staging".
    pub used_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceChanges {
    pub tables: Vec<TableChanges>,
    pub missing_secrets: Vec<MissingSecret>,
}

impl WorkspaceChanges {
    /// Whether an import would change anything.
    pub fn is_empty(&self) -> bool {
        self.tables
            .iter()
            .all(|t| t.added_count == 0 && t.replaced_count == 0)
    }
}

/// All rows of the workspace tables as a JSON document.
pub async fn export(db: &Db) -> DbResult<String> {
    let mut conn = db.pool().acquire().await?;
    let schema = schema_version(&mut conn).await?;
    let mut tables = Map::new();
    for spec in TABLES {
        let order = spec.key.join(", ");
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT * FROM {} ORDER BY {order}",
            spec.name
        )))
        .fetch_all(&mut *conn)
        .await?;
        let rows = rows
            .iter()
            .map(|row| row_to_json(row).map(Value::Object))
            .collect::<DbResult<Vec<_>>>()?;
        tables.insert(spec.name.to_owned(), Value::Array(rows));
    }
    let bundle = json!({
        "format": FORMAT,
        "version": VERSION,
        "schema": schema,
        "exportedAt": now_ms(),
        "tables": tables,
    });
    serde_json::to_string_pretty(&bundle).map_err(|e| DbError::Invalid(e.to_string()))
}

/// What importing `bundle` would do. Changes nothing.
pub async fn preview(
    db: &Db,
    secrets: &dyn SecretStore,
    bundle: &str,
) -> DbResult<WorkspaceChanges> {
    let bundle = parse(bundle)?;
    let mut conn = db.pool().acquire().await?;
    check_schema(&mut conn, &bundle).await?;
    let plan = plan(&mut conn, &bundle).await?;
    Ok(plan.changes(secrets, &bundle))
}

/// Adds and replaces the rows of `bundle` in one transaction. Returns what was done.
pub async fn import(
    db: &Db,
    secrets: &dyn SecretStore,
    bundle: &str,
) -> DbResult<WorkspaceChanges> {
    let bundle = parse(bundle)?;
    let mut tx = db.pool().begin().await?;
    check_schema(&mut tx, &bundle).await?;
    let plan = plan(&mut tx, &bundle).await?;
    sqlx::query("PRAGMA defer_foreign_keys = ON")
        .execute(&mut *tx)
        .await?;
    for table in &plan.tables {
        for row in table.added.iter().chain(&table.replaced) {
            upsert(&mut tx, table.spec, &table.columns, &row.row).await?;
        }
    }
    tx.commit().await?;
    Ok(plan.changes(secrets, &bundle))
}

struct Bundle {
    schema: i64,
    tables: Map<String, Value>,
}

fn parse(text: &str) -> DbResult<Bundle> {
    let invalid = || DbError::Invalid("this is not a MCP Studio workspace file".into());
    let value: Value = serde_json::from_str(text).map_err(|_| invalid())?;
    if value.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(invalid());
    }
    let version = value.get("version").and_then(Value::as_i64).unwrap_or(0);
    if version != VERSION {
        return Err(DbError::Invalid(format!(
            "this workspace file has format version {version}; this app reads version {VERSION}"
        )));
    }
    let Some(Value::Object(tables)) = value.get("tables").cloned() else {
        return Err(invalid());
    };
    Ok(Bundle {
        schema: value.get("schema").and_then(Value::as_i64).unwrap_or(0),
        tables,
    })
}

async fn schema_version(conn: &mut SqliteConnection) -> DbResult<i64> {
    Ok(
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(version) FROM _sqlx_migrations")
            .fetch_one(conn)
            .await?
            .unwrap_or(0),
    )
}

async fn check_schema(conn: &mut SqliteConnection, bundle: &Bundle) -> DbResult<()> {
    if bundle.schema > schema_version(conn).await? {
        return Err(DbError::Invalid(
            "this workspace file comes from a newer version of MCP Studio. Update the app first"
                .into(),
        ));
    }
    Ok(())
}

struct PlannedRow {
    title: String,
    row: Row_,
}

struct PlannedTable {
    spec: &'static TableSpec,
    /// Columns of the table that the bundle also has.
    columns: Vec<String>,
    added: Vec<PlannedRow>,
    replaced: Vec<PlannedRow>,
    unchanged: u32,
}

struct Plan {
    tables: Vec<PlannedTable>,
}

// Table names come from `TABLES` and column names from `PRAGMA table_info` (a column of the file
// that the table does not have is dropped first), so the SQL built below holds no text from the
// file; values are always bound.
async fn table_columns(conn: &mut SqliteConnection, table: &str) -> DbResult<Vec<String>> {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!("PRAGMA table_info({table})")))
        .fetch_all(conn)
        .await?;
    rows.iter()
        .map(|row| Ok(row.try_get::<String, _>("name")?))
        .collect()
}

async fn plan(conn: &mut SqliteConnection, bundle: &Bundle) -> DbResult<Plan> {
    let mut tables = Vec::new();
    for spec in TABLES {
        let known = table_columns(conn, spec.name).await?;
        let rows = match bundle.tables.get(spec.name) {
            None => &[][..],
            Some(Value::Array(rows)) => rows.as_slice(),
            Some(_) => {
                return Err(DbError::Invalid(format!(
                    "the workspace file has a malformed table \"{}\"",
                    spec.name
                )))
            }
        };
        let mut planned = PlannedTable {
            spec,
            columns: Vec::new(),
            added: Vec::new(),
            replaced: Vec::new(),
            unchanged: 0,
        };
        for value in rows {
            let Value::Object(mut row) = value.clone() else {
                return Err(DbError::Invalid(format!(
                    "the workspace file has a malformed row in \"{}\"",
                    spec.name
                )));
            };
            // Ignore columns this version does not know.
            row.retain(|column, _| known.contains(column));
            for column in spec.key.iter().chain(spec.same_as) {
                if !row.contains_key(*column) {
                    return Err(DbError::Invalid(format!(
                        "a row of \"{}\" has no \"{column}\"",
                        spec.name
                    )));
                }
            }
            if planned.columns.is_empty() {
                planned.columns = known
                    .iter()
                    .filter(|c| row.contains_key(*c))
                    .cloned()
                    .collect();
            }
            let title = display(row.get(spec.title));
            match find_existing(conn, spec, &row).await? {
                None => planned.added.push(PlannedRow { title, row }),
                Some(existing) => {
                    // The same thing under another id keeps the id it has here.
                    for column in spec.key {
                        if let Some(value) = existing.get(*column) {
                            row.insert((*column).to_owned(), value.clone());
                        }
                    }
                    if row
                        .iter()
                        .all(|(column, value)| existing.get(column) == Some(value))
                    {
                        planned.unchanged += 1;
                    } else {
                        planned.replaced.push(PlannedRow { title, row });
                    }
                }
            }
        }
        tables.push(planned);
    }
    Ok(Plan { tables })
}

fn display(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

async fn find_existing(
    conn: &mut SqliteConnection,
    spec: &TableSpec,
    row: &Row_,
) -> DbResult<Option<Row_>> {
    let condition = spec
        .same_as
        .iter()
        .map(|column| format!("{column} = ?"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!("SELECT * FROM {} WHERE {condition}", spec.name);
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
    for column in spec.same_as {
        query = bind(query, &row[*column])?;
    }
    match query.fetch_optional(conn).await? {
        Some(found) => Ok(Some(row_to_json(&found)?)),
        None => Ok(None),
    }
}

async fn upsert(
    conn: &mut SqliteConnection,
    spec: &TableSpec,
    columns: &[String],
    row: &Row_,
) -> DbResult<()> {
    let columns: Vec<&String> = columns.iter().filter(|c| row.contains_key(*c)).collect();
    let names = columns
        .iter()
        .map(|c| c.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let marks = vec!["?"; columns.len()].join(", ");
    let updates = columns
        .iter()
        .filter(|c| !spec.key.contains(&c.as_str()))
        .map(|c| format!("{c} = excluded.{c}"))
        .collect::<Vec<_>>();
    let conflict = if updates.is_empty() {
        "DO NOTHING".to_owned()
    } else {
        format!("DO UPDATE SET {}", updates.join(", "))
    };
    let sql = format!(
        "INSERT INTO {} ({names}) VALUES ({marks}) ON CONFLICT ({}) {conflict}",
        spec.name,
        spec.key.join(", ")
    );
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
    for column in &columns {
        query = bind(query, &row[column.as_str()])?;
    }
    query.execute(conn).await?;
    Ok(())
}

type Query<'q> = sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments>;

fn bind<'q>(query: Query<'q>, value: &Value) -> DbResult<Query<'q>> {
    Ok(match value {
        Value::Null => query.bind(None::<String>),
        Value::String(text) => query.bind(text.clone()),
        Value::Number(number) => match number.as_i64() {
            Some(int) => query.bind(int),
            None => query.bind(number.as_f64().unwrap_or_default()),
        },
        other => {
            return Err(DbError::Invalid(format!(
                "the workspace file has a value that cannot be stored: {other}"
            )))
        }
    })
}

fn row_to_json(row: &sqlx::sqlite::SqliteRow) -> DbResult<Row_> {
    let mut out = Map::new();
    for (index, column) in row.columns().iter().enumerate() {
        let raw = row.try_get_raw(index)?;
        let value = if raw.is_null() {
            Value::Null
        } else {
            match raw.type_info().name() {
                "INTEGER" => json!(row.try_get::<i64, _>(index)?),
                "REAL" => json!(row.try_get::<f64, _>(index)?),
                "TEXT" => json!(row.try_get::<String, _>(index)?),
                other => {
                    return Err(DbError::Invalid(format!(
                        "column {} holds a {other} value, which a backup cannot carry",
                        column.name()
                    )))
                }
            }
        };
        out.insert(column.name().to_owned(), value);
    }
    Ok(out)
}

impl Plan {
    fn changes(&self, secrets: &dyn SecretStore, bundle: &Bundle) -> WorkspaceChanges {
        let tables = self
            .tables
            .iter()
            .map(|table| TableChanges {
                label: table.spec.label.to_owned(),
                added: names(&table.added),
                replaced: names(&table.replaced),
                added_count: table.added.len() as u32,
                replaced_count: table.replaced.len() as u32,
                unchanged_count: table.unchanged,
            })
            .collect();
        WorkspaceChanges {
            tables,
            missing_secrets: missing_secrets(secrets, bundle),
        }
    }
}

fn names(rows: &[PlannedRow]) -> Vec<String> {
    rows.iter()
        .take(MAX_NAMES)
        .map(|r| r.title.clone())
        .collect()
}

/// Secret names the bundle refers to (in server env and headers, and environment variables) that
/// the keyring does not have.
fn missing_secrets(secrets: &dyn SecretStore, bundle: &Bundle) -> Vec<MissingSecret> {
    let mut used: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let sources: [(&str, &str, &[&str]); 2] = [
        ("servers", "server", &["env", "headers"]),
        ("environments", "environment", &["variables"]),
    ];
    for (table, kind, columns) in sources {
        let Some(Value::Array(rows)) = bundle.tables.get(table) else {
            continue;
        };
        for row in rows {
            let owner = format!("{kind} {}", display(row.get("name")));
            for column in columns {
                let Some(Value::String(text)) = row.get(*column) else {
                    continue;
                };
                let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else {
                    continue;
                };
                for value in map.values() {
                    let Some(name) = value.as_str().and_then(reference_name) else {
                        continue;
                    };
                    let places = used.entry(name.to_owned()).or_default();
                    if !places.contains(&owner) {
                        places.push(owner.clone());
                    }
                }
            }
        }
    }
    used.into_iter()
        .filter(|(name, _)| !matches!(secrets.get(name), Ok(Some(_))))
        .map(|(name, used_by)| MissingSecret { name, used_by })
        .collect()
}
