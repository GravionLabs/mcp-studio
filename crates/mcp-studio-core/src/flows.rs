//! Saved flows.

use serde::{Deserialize, Serialize};
use specta::Type;
use sqlx::FromRow;

use crate::{
    db::{new_id, now_ms, Db, DbError, DbResult},
    flow::{Flow, FLOW_VERSION},
    flow_yaml,
};

/// A flow in the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FlowRecord {
    pub id: String,
    pub flow: Flow,
    /// Unix milliseconds.
    #[specta(type = u32)]
    pub updated_at: i64,
}

#[derive(FromRow)]
struct Row {
    id: String,
    graph: String,
    updated_at: i64,
}

impl TryFrom<Row> for FlowRecord {
    type Error = DbError;

    fn try_from(row: Row) -> DbResult<Self> {
        let flow = serde_json::from_str(&row.graph)
            .map_err(|e| DbError::Invalid(format!("corrupt flow {}: {e}", row.id)))?;
        Ok(Self {
            id: row.id,
            flow,
            updated_at: row.updated_at,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Flows {
    db: Db,
}

impl Flows {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn list(&self) -> DbResult<Vec<FlowRecord>> {
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, graph, updated_at FROM flows ORDER BY name COLLATE NOCASE, id",
        )
        .fetch_all(self.db.pool())
        .await?;
        rows.into_iter().map(FlowRecord::try_from).collect()
    }

    pub async fn get(&self, id: &str) -> DbResult<FlowRecord> {
        let row: Option<Row> =
            sqlx::query_as("SELECT id, graph, updated_at FROM flows WHERE id = ?")
                .bind(id)
                .fetch_optional(self.db.pool())
                .await?;
        row.ok_or_else(|| DbError::NotFound(format!("flow {id}")))?
            .try_into()
    }

    /// Creates a flow (`id` is `None`) or replaces an existing one. The flow does not have to be
    /// valid yet, so work in progress can be saved.
    pub async fn save(&self, id: Option<&str>, mut flow: Flow) -> DbResult<FlowRecord> {
        flow.name = flow.name.trim().to_owned();
        if flow.name.is_empty() {
            return Err(DbError::Invalid("a flow needs a name".into()));
        }
        if flow.version != FLOW_VERSION {
            return Err(DbError::Invalid(format!(
                "flow version {} is not supported (expected {FLOW_VERSION})",
                flow.version
            )));
        }
        let graph = serde_json::to_string(&flow)
            .map_err(|e| DbError::Invalid(format!("could not save the flow: {e}")))?;
        let now = now_ms();
        match id {
            Some(id) => {
                let result = sqlx::query(
                    "UPDATE flows SET name = ?, graph = ?, version = ?, updated_at = ? WHERE id = ?",
                )
                .bind(&flow.name)
                .bind(&graph)
                .bind(i64::from(flow.version))
                .bind(now)
                .bind(id)
                .execute(self.db.pool())
                .await?;
                if result.rows_affected() == 0 {
                    return Err(DbError::NotFound(format!("flow {id}")));
                }
                self.get(id).await
            }
            None => {
                let id = new_id();
                sqlx::query(
                    "INSERT INTO flows (id, name, graph, version, updated_at) VALUES (?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(&flow.name)
                .bind(&graph)
                .bind(i64::from(flow.version))
                .bind(now)
                .execute(self.db.pool())
                .await?;
                self.get(&id).await
            }
        }
    }

    pub async fn delete(&self, id: &str) -> DbResult<()> {
        let result = sqlx::query("DELETE FROM flows WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("flow {id}")));
        }
        Ok(())
    }

    pub async fn export_yaml(&self, id: &str) -> DbResult<String> {
        flow_yaml::to_yaml(&self.get(id).await?.flow)
    }

    /// Adds the flow in `yaml` to the library. A name that is already taken gets a number, so an
    /// import never overwrites anything.
    pub async fn import_yaml(&self, yaml: &str) -> DbResult<FlowRecord> {
        let mut flow = flow_yaml::from_yaml(yaml)?;
        let taken: Vec<String> = self
            .list()
            .await?
            .into_iter()
            .map(|r| r.flow.name)
            .collect();
        let base = flow.name.trim().to_owned();
        let mut name = base.clone();
        let mut n = 2;
        while taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
            name = format!("{base} ({n})");
            n += 1;
        }
        flow.name = name;
        self.save(None, flow).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::flow::{Step, StepKind};

    fn flow(name: &str) -> Flow {
        Flow {
            version: 1,
            name: name.into(),
            steps: vec![Step {
                id: "out".into(),
                kind: StepKind::Output {
                    outputs: BTreeMap::from([("x".into(), "{{ inputs.x }}".into())]),
                },
            }],
        }
    }

    async fn repo() -> Flows {
        Flows::new(Db::open_in_memory().await.unwrap())
    }

    #[tokio::test]
    async fn saves_lists_updates_and_deletes_flows() {
        let flows = repo().await;
        assert!(flows.list().await.unwrap().is_empty());
        let a = flows.save(None, flow("  beta ")).await.unwrap();
        let b = flows.save(None, flow("Alpha")).await.unwrap();
        assert_eq!(a.flow.name, "beta");
        let names: Vec<_> = flows
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.flow.name)
            .collect();
        assert_eq!(names, ["Alpha", "beta"]);

        let mut changed = a.flow.clone();
        changed.name = "gamma".into();
        let updated = flows.save(Some(&a.id), changed).await.unwrap();
        assert_eq!(updated.id, a.id);
        assert_eq!(flows.get(&a.id).await.unwrap().flow.name, "gamma");
        assert!(updated.updated_at >= a.updated_at);

        flows.delete(&b.id).await.unwrap();
        assert_eq!(flows.list().await.unwrap().len(), 1);
        assert!(matches!(
            flows.delete(&b.id).await,
            Err(DbError::NotFound(_))
        ));
        assert!(matches!(flows.get(&b.id).await, Err(DbError::NotFound(_))));
        assert!(matches!(
            flows.save(Some("missing"), flow("x")).await,
            Err(DbError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn work_in_progress_can_be_saved_but_not_nameless_or_unversioned_flows() {
        let flows = repo().await;
        // Not valid (no input step, unknown server), but it can be saved.
        assert!(flows.save(None, flow("draft")).await.is_ok());
        assert!(flows.save(None, flow("  ")).await.is_err());
        let mut future = flow("v2");
        future.version = 2;
        assert!(flows.save(None, future).await.is_err());
    }

    #[tokio::test]
    async fn a_flow_survives_saving_and_the_yaml_round_trip() {
        let flows = repo().await;
        let saved = flows.save(None, flow("rt")).await.unwrap();
        let yaml = flows.export_yaml(&saved.id).await.unwrap();
        // Importing the export gives the same flow, under a new name because "rt" is taken.
        let imported = flows.import_yaml(&yaml).await.unwrap();
        assert_ne!(imported.id, saved.id);
        assert_eq!(imported.flow.name, "rt (2)");
        assert_eq!(imported.flow.steps, saved.flow.steps);
        let again = flows.import_yaml(&yaml).await.unwrap();
        assert_eq!(again.flow.name, "rt (3)");
    }

    #[tokio::test]
    async fn importing_never_overwrites_and_rejects_bad_yaml() {
        let flows = repo().await;
        flows.save(None, flow("Same")).await.unwrap();
        let yaml = flow_yaml::to_yaml(&flow("same")).unwrap();
        let imported = flows.import_yaml(&yaml).await.unwrap();
        assert_eq!(imported.flow.name, "same (2)");
        assert_eq!(flows.list().await.unwrap().len(), 2);
        assert!(flows
            .import_yaml("version: 9\nname: x\nsteps: []\n")
            .await
            .is_err());
        assert_eq!(flows.list().await.unwrap().len(), 2);
    }
}
