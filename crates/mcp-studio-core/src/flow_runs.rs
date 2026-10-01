//! Stored runs of flows.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{
    db::{Db, DbError, DbResult},
    flow::Flow,
    model::JsonValue,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Succeeded => "succeeded",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "succeeded" => RunStatus::Succeeded,
            "failed" => RunStatus::Failed,
            "cancelled" => RunStatus::Cancelled,
            _ => RunStatus::Running,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    Running,
    Succeeded,
    Failed,
    /// Not run because a condition jumped over it.
    Skipped,
    Cancelled,
}

impl StepStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            StepStatus::Running => "running",
            StepStatus::Succeeded => "succeeded",
            StepStatus::Failed => "failed",
            StepStatus::Skipped => "skipped",
            StepStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "succeeded" => StepStatus::Succeeded,
            "failed" => StepStatus::Failed,
            "skipped" => StepStatus::Skipped,
            "cancelled" => StepStatus::Cancelled,
            _ => StepStatus::Running,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StepRun {
    #[specta(type = u32)]
    pub seq: i64,
    pub step_id: String,
    /// The step type: `input`, `llm`, `tool`, ...
    pub kind: String,
    pub status: StepStatus,
    #[specta(type = u32)]
    pub started_at: i64,
    #[specta(type = Option<u32>)]
    pub ended_at: Option<i64>,
    /// What the step was asked to do once templates were filled in.
    pub resolved: Option<JsonValue>,
    pub output: Option<JsonValue>,
    pub error: Option<String>,
    pub span_id: Option<String>,
}

/// A tool call made by a run, with its result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecordedCall {
    #[specta(type = u32)]
    pub seq: i64,
    pub step_id: String,
    pub server: String,
    pub tool: String,
    pub arguments: JsonValue,
    /// The MCP `CallToolResult`; `None` when the call was denied or failed before it ran.
    pub result: Option<JsonValue>,
    pub is_error: bool,
    /// The user did not allow the call.
    pub denied: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FlowRun {
    pub id: String,
    pub flow_id: Option<String>,
    /// The flow as it was when the run started.
    pub flow: Flow,
    pub inputs: JsonValue,
    pub status: RunStatus,
    #[specta(type = u32)]
    pub started_at: i64,
    #[specta(type = Option<u32>)]
    pub ended_at: Option<i64>,
    pub outputs: Option<JsonValue>,
    pub error: Option<String>,
    /// The run whose tool results this run replays.
    pub replay_of: Option<String>,
    pub steps: Vec<StepRun>,
    pub calls: Vec<RecordedCall>,
}

/// A run in a list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub id: String,
    pub flow_id: Option<String>,
    pub flow_name: String,
    pub status: RunStatus,
    #[specta(type = u32)]
    pub started_at: i64,
    #[specta(type = Option<u32>)]
    pub ended_at: Option<i64>,
    pub error: Option<String>,
    pub replay_of: Option<String>,
}

fn to_json(value: &serde_json::Value) -> String {
    value.to_string()
}

fn from_json(text: Option<String>) -> Option<JsonValue> {
    text.and_then(|t| serde_json::from_str(&t).ok())
        .map(JsonValue)
}

#[derive(Clone, Debug)]
pub struct FlowRuns {
    db: Db,
}

impl FlowRuns {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub async fn create(
        &self,
        id: &str,
        flow_id: Option<&str>,
        flow: &Flow,
        inputs: &serde_json::Value,
        replay_of: Option<&str>,
        started_at: i64,
    ) -> DbResult<()> {
        let snapshot = serde_json::to_string(flow)
            .map_err(|e| DbError::Invalid(format!("could not store the run: {e}")))?;
        sqlx::query(
            "INSERT INTO flow_runs (id, flow_id, flow_name, flow, inputs, status, started_at, replay_of) \
             VALUES (?, ?, ?, ?, ?, 'running', ?, ?)",
        )
        .bind(id)
        .bind(flow_id)
        .bind(&flow.name)
        .bind(snapshot)
        .bind(to_json(inputs))
        .bind(started_at)
        .bind(replay_of)
        .execute(self.db.pool())
        .await?;
        Ok(())
    }

    pub async fn finish(
        &self,
        id: &str,
        status: RunStatus,
        ended_at: i64,
        outputs: Option<&serde_json::Value>,
        error: Option<&str>,
    ) -> DbResult<()> {
        sqlx::query(
            "UPDATE flow_runs SET status = ?, ended_at = ?, outputs = ?, error = ? WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(ended_at)
        .bind(outputs.map(to_json))
        .bind(error)
        .bind(id)
        .execute(self.db.pool())
        .await?;
        Ok(())
    }

    /// Records that a step started and returns its sequence number.
    pub async fn step_started(
        &self,
        run_id: &str,
        step_id: &str,
        kind: &str,
        started_at: i64,
        span_id: Option<&str>,
    ) -> DbResult<i64> {
        let seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM flow_run_steps WHERE run_id = ?",
        )
        .bind(run_id)
        .fetch_one(self.db.pool())
        .await?;
        sqlx::query(
            "INSERT INTO flow_run_steps (run_id, seq, step_id, kind, status, started_at, span_id) \
             VALUES (?, ?, ?, ?, 'running', ?, ?)",
        )
        .bind(run_id)
        .bind(seq)
        .bind(step_id)
        .bind(kind)
        .bind(started_at)
        .bind(span_id)
        .execute(self.db.pool())
        .await?;
        Ok(seq)
    }

    /// Stores what the step was asked to do once its templates were filled in.
    pub async fn step_resolved(
        &self,
        run_id: &str,
        seq: i64,
        resolved: &serde_json::Value,
    ) -> DbResult<()> {
        sqlx::query("UPDATE flow_run_steps SET resolved = ? WHERE run_id = ? AND seq = ?")
            .bind(to_json(resolved))
            .bind(run_id)
            .bind(seq)
            .execute(self.db.pool())
            .await?;
        Ok(())
    }

    pub async fn step_finished(
        &self,
        run_id: &str,
        seq: i64,
        status: StepStatus,
        ended_at: i64,
        output: Option<&serde_json::Value>,
        error: Option<&str>,
    ) -> DbResult<()> {
        sqlx::query(
            "UPDATE flow_run_steps SET status = ?, ended_at = ?, output = ?, error = ? \
             WHERE run_id = ? AND seq = ?",
        )
        .bind(status.as_str())
        .bind(ended_at)
        .bind(output.map(to_json))
        .bind(error)
        .bind(run_id)
        .bind(seq)
        .execute(self.db.pool())
        .await?;
        Ok(())
    }

    /// Records a step that was jumped over.
    pub async fn step_skipped(
        &self,
        run_id: &str,
        step_id: &str,
        kind: &str,
        at: i64,
    ) -> DbResult<()> {
        let seq = self.step_started(run_id, step_id, kind, at, None).await?;
        self.step_finished(run_id, seq, StepStatus::Skipped, at, None, None)
            .await
    }

    pub async fn add_call(&self, run_id: &str, call: &RecordedCall) -> DbResult<i64> {
        let seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM flow_run_calls WHERE run_id = ?",
        )
        .bind(run_id)
        .fetch_one(self.db.pool())
        .await?;
        sqlx::query(
            "INSERT INTO flow_run_calls (run_id, seq, step_id, server, tool, arguments, result, is_error, denied) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(run_id)
        .bind(seq)
        .bind(&call.step_id)
        .bind(&call.server)
        .bind(&call.tool)
        .bind(to_json(&call.arguments.0))
        .bind(call.result.as_ref().map(|r| to_json(&r.0)))
        .bind(call.is_error)
        .bind(call.denied)
        .execute(self.db.pool())
        .await?;
        Ok(seq)
    }

    pub async fn get(&self, id: &str) -> DbResult<FlowRun> {
        type Header = (
            String,
            Option<String>,
            String,
            String,
            String,
            i64,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<String>,
        );
        let header: Option<Header> = sqlx::query_as(
            "SELECT id, flow_id, flow, inputs, status, started_at, ended_at, outputs, error, replay_of \
             FROM flow_runs WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.db.pool())
        .await?;
        let (id, flow_id, flow, inputs, status, started_at, ended_at, outputs, error, replay_of) =
            header.ok_or_else(|| DbError::NotFound(format!("run {id}")))?;
        let flow: Flow = serde_json::from_str(&flow)
            .map_err(|e| DbError::Invalid(format!("corrupt run {id}: {e}")))?;

        type StepRow = (
            i64,
            String,
            String,
            String,
            i64,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        );
        let step_rows: Vec<StepRow> = sqlx::query_as(
            "SELECT seq, step_id, kind, status, started_at, ended_at, resolved, output, error, span_id \
             FROM flow_run_steps WHERE run_id = ? ORDER BY seq",
        )
        .bind(&id)
        .fetch_all(self.db.pool())
        .await?;
        let steps = step_rows
            .into_iter()
            .map(
                |(
                    seq,
                    step_id,
                    kind,
                    status,
                    started_at,
                    ended_at,
                    resolved,
                    output,
                    error,
                    span_id,
                )| StepRun {
                    seq,
                    step_id,
                    kind,
                    status: StepStatus::parse(&status),
                    started_at,
                    ended_at,
                    resolved: from_json(resolved),
                    output: from_json(output),
                    error,
                    span_id,
                },
            )
            .collect();

        type CallRow = (
            i64,
            String,
            String,
            String,
            String,
            Option<String>,
            bool,
            bool,
        );
        let call_rows: Vec<CallRow> = sqlx::query_as(
            "SELECT seq, step_id, server, tool, arguments, result, is_error, denied \
             FROM flow_run_calls WHERE run_id = ? ORDER BY seq",
        )
        .bind(&id)
        .fetch_all(self.db.pool())
        .await?;
        let calls = call_rows
            .into_iter()
            .map(
                |(seq, step_id, server, tool, arguments, result, is_error, denied)| RecordedCall {
                    seq,
                    step_id,
                    server,
                    tool,
                    arguments: from_json(Some(arguments)).unwrap_or_default(),
                    result: from_json(result),
                    is_error,
                    denied,
                },
            )
            .collect();

        Ok(FlowRun {
            id,
            flow_id,
            flow,
            inputs: from_json(Some(inputs)).unwrap_or_default(),
            status: RunStatus::parse(&status),
            started_at,
            ended_at,
            outputs: from_json(outputs),
            error,
            replay_of,
            steps,
            calls,
        })
    }

    /// The newest runs first; `flow_id` limits the list to the runs of one flow.
    pub async fn list(&self, flow_id: Option<&str>, limit: u32) -> DbResult<Vec<RunSummary>> {
        type Row = (
            String,
            Option<String>,
            String,
            String,
            i64,
            Option<i64>,
            Option<String>,
            Option<String>,
        );
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, flow_id, flow_name, status, started_at, ended_at, error, replay_of FROM flow_runs \
             WHERE (? IS NULL OR flow_id = ?) ORDER BY started_at DESC, rowid DESC LIMIT ?",
        )
        .bind(flow_id)
        .bind(flow_id)
        .bind(i64::from(limit.clamp(1, 500)))
        .fetch_all(self.db.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(id, flow_id, flow_name, status, started_at, ended_at, error, replay_of)| {
                    RunSummary {
                        id,
                        flow_id,
                        flow_name,
                        status: RunStatus::parse(&status),
                        started_at,
                        ended_at,
                        error,
                        replay_of,
                    }
                },
            )
            .collect())
    }

    pub async fn delete(&self, id: &str) -> DbResult<()> {
        let result = sqlx::query("DELETE FROM flow_runs WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("run {id}")));
        }
        Ok(())
    }

    /// Marks runs that were still running when the app stopped as failed.
    pub async fn fail_interrupted(&self, at: i64) -> DbResult<u64> {
        let steps = sqlx::query(
            "UPDATE flow_run_steps SET status = 'failed', ended_at = ?, error = 'the app was closed during this step' \
             WHERE status = 'running'",
        )
        .bind(at)
        .execute(self.db.pool())
        .await?;
        let _ = steps;
        Ok(sqlx::query(
            "UPDATE flow_runs SET status = 'failed', ended_at = ?, error = 'the app was closed during this run' \
             WHERE status = 'running'",
        )
        .bind(at)
        .execute(self.db.pool())
        .await?
        .rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::flow::{Step, StepKind};

    fn flow() -> Flow {
        Flow {
            version: 1,
            name: "demo".into(),
            steps: vec![Step {
                id: "out".into(),
                kind: StepKind::Output {
                    outputs: BTreeMap::new(),
                },
            }],
        }
    }

    async fn runs() -> FlowRuns {
        FlowRuns::new(Db::open_in_memory().await.unwrap())
    }

    #[tokio::test]
    async fn stores_a_run_with_steps_and_calls_and_reads_it_back() {
        let runs = runs().await;
        runs.create(
            "r1",
            Some("f1"),
            &flow(),
            &json!({"repo": "a/b"}),
            None,
            100,
        )
        .await
        .unwrap();
        let seq = runs
            .step_started("r1", "issues", "tool", 110, Some("span-1"))
            .await
            .unwrap();
        assert_eq!(seq, 1);
        runs.step_resolved("r1", seq, &json!({"arguments": {"repo": "a/b"}}))
            .await
            .unwrap();
        runs.add_call(
            "r1",
            &RecordedCall {
                seq: 0,
                step_id: "issues".into(),
                server: "github".into(),
                tool: "list_issues".into(),
                arguments: JsonValue(json!({"repo": "a/b"})),
                result: Some(JsonValue(
                    json!({"content": [{"type": "text", "text": "3"}]}),
                )),
                is_error: false,
                denied: false,
            },
        )
        .await
        .unwrap();
        runs.step_finished(
            "r1",
            seq,
            StepStatus::Succeeded,
            150,
            Some(&json!({"result": "3"})),
            None,
        )
        .await
        .unwrap();
        runs.step_skipped("r1", "other", "transform", 151)
            .await
            .unwrap();
        runs.finish(
            "r1",
            RunStatus::Succeeded,
            200,
            Some(&json!({"summary": "ok"})),
            None,
        )
        .await
        .unwrap();

        let run = runs.get("r1").await.unwrap();
        assert_eq!(
            (run.status, run.ended_at),
            (RunStatus::Succeeded, Some(200))
        );
        assert_eq!(run.flow, flow());
        assert_eq!(run.inputs.0, json!({"repo": "a/b"}));
        assert_eq!(run.outputs.unwrap().0, json!({"summary": "ok"}));
        assert_eq!(run.flow_id.as_deref(), Some("f1"));
        assert_eq!(run.steps.len(), 2);
        let first = &run.steps[0];
        assert_eq!(
            (first.step_id.as_str(), first.status, first.ended_at),
            ("issues", StepStatus::Succeeded, Some(150))
        );
        assert_eq!(first.span_id.as_deref(), Some("span-1"));
        assert_eq!(
            first.resolved.as_ref().unwrap().0["arguments"]["repo"],
            "a/b"
        );
        assert_eq!(first.output.as_ref().unwrap().0["result"], "3");
        assert_eq!(run.steps[1].status, StepStatus::Skipped);
        assert_eq!(run.calls.len(), 1);
        assert_eq!(run.calls[0].seq, 1);
        assert_eq!(run.calls[0].tool, "list_issues");
        assert_eq!(
            run.calls[0].result.as_ref().unwrap().0["content"][0]["text"],
            "3"
        );
    }

    #[tokio::test]
    async fn lists_newest_first_and_filters_by_flow() {
        let runs = runs().await;
        runs.create("a", Some("f1"), &flow(), &json!({}), None, 1)
            .await
            .unwrap();
        runs.create("b", Some("f2"), &flow(), &json!({}), None, 2)
            .await
            .unwrap();
        runs.create("c", Some("f1"), &flow(), &json!({}), Some("a"), 3)
            .await
            .unwrap();
        let all: Vec<_> = runs
            .list(None, 10)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(all, ["c", "b", "a"]);
        let f1 = runs.list(Some("f1"), 10).await.unwrap();
        assert_eq!(
            f1.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["c", "a"]
        );
        assert_eq!(f1[0].replay_of.as_deref(), Some("a"));
        assert_eq!(runs.list(None, 1).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn deleting_a_run_removes_its_steps_and_calls() {
        let runs = runs().await;
        runs.create("r", None, &flow(), &json!({}), None, 1)
            .await
            .unwrap();
        runs.step_started("r", "s", "tool", 1, None).await.unwrap();
        runs.delete("r").await.unwrap();
        assert!(matches!(runs.get("r").await, Err(DbError::NotFound(_))));
        assert!(matches!(runs.delete("r").await, Err(DbError::NotFound(_))));
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM flow_run_steps")
            .fetch_one(runs.db.pool())
            .await
            .unwrap();
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn runs_that_were_cut_short_by_closing_the_app_are_marked_failed() {
        let runs = runs().await;
        runs.create("r", None, &flow(), &json!({}), None, 1)
            .await
            .unwrap();
        runs.step_started("r", "s", "tool", 1, None).await.unwrap();
        runs.create("done", None, &flow(), &json!({}), None, 1)
            .await
            .unwrap();
        runs.finish("done", RunStatus::Succeeded, 2, None, None)
            .await
            .unwrap();
        assert_eq!(runs.fail_interrupted(50).await.unwrap(), 1);
        let run = runs.get("r").await.unwrap();
        assert_eq!(run.status, RunStatus::Failed);
        assert!(run.error.unwrap().contains("closed"));
        assert_eq!(run.steps[0].status, StepStatus::Failed);
        assert_eq!(runs.get("done").await.unwrap().status, RunStatus::Succeeded);
    }
}
