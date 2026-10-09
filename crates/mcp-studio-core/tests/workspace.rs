//! Backup and restore of the workspace: round trip, preview, replacing, secrets, and bad files.

use mcp_studio_core::{
    db::Db,
    secrets::{MemoryStore, SecretStore},
    workspace,
};
use serde_json::Value;

async fn run(db: &Db, sql: &'static str) {
    sqlx::query(sql).execute(db.pool()).await.unwrap();
}

/// A workspace with one row of everything the backup carries.
async fn filled() -> Db {
    let db = Db::open_in_memory().await.unwrap();
    run(&db, r#"INSERT INTO servers (id, name, transport, url, headers, oauth, azure_credentials, created_at, updated_at)
        VALUES ('s1', 'GitHub', 'http', 'https://example.com/mcp', '{"Authorization":"keyring:github-token"}', 0, 1, 10, 20)"#).await;
    run(&db, r#"INSERT INTO servers (id, name, transport, command, env, created_at, updated_at)
        VALUES ('s2', 'Local', 'stdio', 'npx', '{"API_KEY":"keyring:local-key","MODE":"dev"}', 11, 21)"#).await;
    run(&db, r#"INSERT INTO environments (id, name, variables) VALUES ('e1', 'Staging', '{"token":"keyring:staging-token","host":"example.com"}')"#).await;
    run(
        &db,
        "INSERT INTO collections (id, parent_id, name, sort_order) VALUES ('c1', NULL, 'Root', 0)",
    )
    .await;
    run(
        &db,
        "INSERT INTO collections (id, parent_id, name, sort_order) VALUES ('c2', 'c1', 'Child', 1)",
    )
    .await;
    run(&db, r#"INSERT INTO requests (id, collection_id, server_id, method, name, tool_name, arguments, notes, sort_order, created_at, updated_at)
        VALUES ('r1', 'c2', 's1', 'tools/call', 'Search', 'search', '{"q":"mcp"}', 'a note', 0, 1, 2)"#).await;
    run(&db, r#"INSERT INTO flows (id, name, graph, version, updated_at) VALUES ('f1', 'Flow', '{"version":1,"name":"Flow","steps":[]}', 1, 5)"#).await;
    run(&db, "INSERT INTO test_suites (id, server_id, name, system_prompt, updated_at) VALUES ('t1', 's1', 'Suite', NULL, 7)").await;
    run(&db, r#"INSERT INTO test_cases (id, suite_id, position, input, expectation, notes) VALUES ('tc1', 't1', 0, 'find it', '{"kind":"tool","name":"search"}', NULL)"#).await;
    run(&db, "INSERT INTO prices (model, input_per_mtok, output_per_mtok, cache_read_per_mtok, cache_write_per_mtok, currency) VALUES ('m', 3.0, 15.5, NULL, 3.75, 'USD')").await;
    db
}

fn tables(bundle: &str) -> Value {
    serde_json::from_str::<Value>(bundle).unwrap()["tables"].clone()
}

fn total(changes: &workspace::WorkspaceChanges, pick: fn(&workspace::TableChanges) -> u32) -> u32 {
    changes.tables.iter().map(pick).sum()
}

#[tokio::test]
async fn exports_and_imports_everything_into_an_empty_workspace() {
    let source = filled().await;
    let bundle = workspace::export(&source).await.unwrap();

    let target = Db::open_in_memory().await.unwrap();
    let secrets = MemoryStore::default();
    let done = workspace::import(&target, &secrets, &bundle).await.unwrap();

    assert_eq!(total(&done, |t| t.added_count), 10);
    assert_eq!(total(&done, |t| t.replaced_count), 0);
    assert_eq!(
        tables(&workspace::export(&target).await.unwrap()),
        tables(&bundle)
    );
}

#[tokio::test]
async fn importing_the_same_file_again_changes_nothing() {
    let source = filled().await;
    let bundle = workspace::export(&source).await.unwrap();
    let secrets = MemoryStore::default();

    let changes = workspace::import(&source, &secrets, &bundle).await.unwrap();

    assert!(changes.is_empty());
    assert_eq!(total(&changes, |t| t.unchanged_count), 10);
}

#[tokio::test]
async fn a_preview_changes_nothing_and_names_what_would_change() {
    let source = filled().await;
    let bundle = workspace::export(&source).await.unwrap();
    let target = Db::open_in_memory().await.unwrap();
    run(&target, "INSERT INTO servers (id, name, transport, command, created_at, updated_at) VALUES ('s2', 'Old name', 'stdio', 'npx', 1, 1)").await;
    let secrets = MemoryStore::default();

    let preview = workspace::preview(&target, &secrets, &bundle)
        .await
        .unwrap();

    let servers = &preview.tables[0];
    assert_eq!(servers.label, "Servers");
    assert_eq!(servers.added, ["GitHub"]);
    assert_eq!(servers.replaced, ["Local"]);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM servers")
        .fetch_one(target.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
    let name: String = sqlx::query_scalar("SELECT name FROM servers WHERE id = 's2'")
        .fetch_one(target.pool())
        .await
        .unwrap();
    assert_eq!(name, "Old name");
}

#[tokio::test]
async fn an_import_replaces_rows_with_the_same_id_and_keeps_the_others() {
    let source = filled().await;
    let bundle = workspace::export(&source).await.unwrap();
    let target = Db::open_in_memory().await.unwrap();
    run(&target, "INSERT INTO servers (id, name, transport, command, created_at, updated_at) VALUES ('s2', 'Old name', 'stdio', 'old', 1, 1)").await;
    run(&target, "INSERT INTO servers (id, name, transport, command, created_at, updated_at) VALUES ('mine', 'Mine', 'stdio', 'x', 1, 1)").await;

    workspace::import(&target, &MemoryStore::default(), &bundle)
        .await
        .unwrap();

    let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, name FROM servers ORDER BY id")
        .fetch_all(target.pool())
        .await
        .unwrap();
    assert_eq!(
        rows,
        [
            ("mine".into(), "Mine".into()),
            ("s1".into(), "GitHub".into()),
            ("s2".into(), "Local".into())
        ]
    );
}

#[tokio::test]
async fn an_environment_with_the_same_name_is_replaced_under_its_local_id() {
    let source = filled().await;
    let bundle = workspace::export(&source).await.unwrap();
    let target = Db::open_in_memory().await.unwrap();
    run(&target, r#"INSERT INTO environments (id, name, variables) VALUES ('local-id', 'Staging', '{"host":"old"}')"#).await;

    let done = workspace::import(&target, &MemoryStore::default(), &bundle)
        .await
        .unwrap();

    let environments = done
        .tables
        .iter()
        .find(|t| t.label == "Environments")
        .unwrap();
    assert_eq!(environments.replaced, ["Staging"]);
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, variables FROM environments")
        .fetch_all(target.pool())
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "local-id");
    assert!(rows[0].1.contains("staging-token"));
}

#[tokio::test]
async fn the_file_holds_secret_names_but_no_secret_values() {
    let source = filled().await;
    let secrets = MemoryStore::default();
    secrets.set("github-token", "ghp_very_secret").unwrap();

    let bundle = workspace::export(&source).await.unwrap();

    assert!(bundle.contains("keyring:github-token"));
    assert!(!bundle.contains("ghp_very_secret"));
}

#[tokio::test]
async fn secrets_missing_from_the_keyring_are_listed() {
    let source = filled().await;
    let bundle = workspace::export(&source).await.unwrap();
    let target = Db::open_in_memory().await.unwrap();
    let secrets = MemoryStore::default();
    secrets.set("github-token", "kept").unwrap();

    let done = workspace::import(&target, &secrets, &bundle).await.unwrap();

    let missing: Vec<(&str, &[String])> = done
        .missing_secrets
        .iter()
        .map(|s| (s.name.as_str(), s.used_by.as_slice()))
        .collect();
    assert_eq!(
        missing,
        [
            ("local-key", &["server Local".to_owned()][..]),
            ("staging-token", &["environment Staging".to_owned()][..]),
        ]
    );
}

#[tokio::test]
async fn files_that_are_not_a_workspace_are_rejected() {
    let db = Db::open_in_memory().await.unwrap();
    let secrets = MemoryStore::default();
    for text in [
        "",
        "not json",
        "[]",
        r#"{"format":"other","version":1,"tables":{}}"#,
    ] {
        let error = workspace::preview(&db, &secrets, text).await.unwrap_err();
        assert!(error
            .to_string()
            .contains("not a MCP Studio workspace file"));
    }
    let newer_format = r#"{"format":"mcp-studio-workspace","version":2,"tables":{}}"#;
    assert!(workspace::preview(&db, &secrets, newer_format)
        .await
        .unwrap_err()
        .to_string()
        .contains("format version 2"));
}

#[tokio::test]
async fn a_file_from_a_newer_app_is_rejected() {
    let db = Db::open_in_memory().await.unwrap();
    let text = r#"{"format":"mcp-studio-workspace","version":1,"schema":9999,"tables":{}}"#;
    let error = workspace::import(&db, &MemoryStore::default(), text)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("newer version"));
}

#[tokio::test]
async fn a_failing_import_leaves_the_workspace_as_it_was() {
    let source = filled().await;
    let bundle = workspace::export(&source).await.unwrap();
    // A request that points at a server which is not in the file.
    let mut value: Value = serde_json::from_str(&bundle).unwrap();
    value["tables"]["requests"][0]["server_id"] = Value::String("missing".into());
    let broken = serde_json::to_string(&value).unwrap();
    let target = Db::open_in_memory().await.unwrap();

    let result = workspace::import(&target, &MemoryStore::default(), &broken).await;

    assert!(result.is_err());
    for table in ["servers", "collections", "requests", "prices"] {
        let count: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
                .fetch_one(target.pool())
                .await
                .unwrap();
        assert_eq!(count, 0, "{table}");
    }
}

#[tokio::test]
async fn columns_this_version_does_not_know_are_ignored() {
    let source = filled().await;
    let bundle = workspace::export(&source).await.unwrap();
    let mut value: Value = serde_json::from_str(&bundle).unwrap();
    value["tables"]["prices"][0]["from_the_future"] = Value::String("x".into());
    let target = Db::open_in_memory().await.unwrap();

    workspace::import(&target, &MemoryStore::default(), &value.to_string())
        .await
        .unwrap();

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM prices")
        .fetch_one(target.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
}
