//! Replays a run with its recorded tool results.
//!
//! A replay runs the flow again, but tool calls are answered from what the original run recorded
//! instead of reaching the servers. Templates, conditions, and model calls run live, so a replay
//! shows what a changed prompt, model, or flow would have done with exactly the same tool data. No
//! tool is called, so nothing needs the user's consent.
//!
//! Recorded results are matched by server and tool, in the order they were recorded; the arguments
//! of the replayed call may differ from the original ones. A call without a recorded result left is
//! an error, and so is a call that was denied or failed to run in the original run.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::{
    explorer::ToolInfo,
    flow_run::{ToolOutcome, ToolRunner},
    flow_runs::RecordedCall,
    model::JsonValue,
};

pub struct ReplayTools {
    recorded: Mutex<HashMap<(String, String), VecDeque<RecordedCall>>>,
    /// Describes the tools of servers that are still available (for validation and for the tool
    /// definitions of agent steps). Optional: without it, or when a server cannot be reached, the
    /// tools are taken from the recorded calls.
    catalog: Option<Arc<dyn ToolRunner>>,
}

impl ReplayTools {
    pub fn new(calls: &[RecordedCall], catalog: Option<Arc<dyn ToolRunner>>) -> Self {
        let mut recorded: HashMap<(String, String), VecDeque<RecordedCall>> = HashMap::new();
        for call in calls {
            recorded
                .entry((call.server.clone(), call.tool.clone()))
                .or_default()
                .push_back(call.clone());
        }
        Self {
            recorded: Mutex::new(recorded),
            catalog,
        }
    }

    /// How many recorded results are still unused.
    pub fn remaining(&self) -> usize {
        self.recorded
            .lock()
            .unwrap()
            .values()
            .map(VecDeque::len)
            .sum()
    }

    fn recorded_tools(&self, server: &str) -> Vec<ToolInfo> {
        let recorded = self.recorded.lock().unwrap();
        let mut names: Vec<&String> = recorded
            .keys()
            .filter(|(s, _)| s == server)
            .map(|(_, tool)| tool)
            .collect();
        names.sort();
        names
            .into_iter()
            .map(|name| ToolInfo {
                name: name.clone(),
                title: None,
                description: Some("A tool of the original run (replayed from its record)".into()),
                input_schema: JsonValue(json!({ "type": "object" })),
                output_schema: None,
                annotations: None,
            })
            .collect()
    }
}

#[async_trait]
impl ToolRunner for ReplayTools {
    async fn list_tools(&self, server: &str) -> Result<Vec<ToolInfo>, String> {
        if let Some(catalog) = &self.catalog {
            if let Ok(tools) = catalog.list_tools(server).await {
                return Ok(tools);
            }
        }
        let tools = self.recorded_tools(server);
        if tools.is_empty() {
            Err(format!("the original run called no tool of {server:?}"))
        } else {
            Ok(tools)
        }
    }

    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        _arguments: Value,
        _cancel: &CancellationToken,
    ) -> Result<ToolOutcome, String> {
        let call = self
            .recorded
            .lock()
            .unwrap()
            .get_mut(&(server.to_owned(), tool.to_owned()))
            .and_then(VecDeque::pop_front)
            .ok_or_else(|| {
                format!("the original run has no recorded result left for {server}/{tool}")
            })?;
        if call.denied {
            return Err(format!(
                "the original call of {server}/{tool} was not allowed"
            ));
        }
        match call.result {
            Some(result) => Ok(ToolOutcome {
                result: result.0,
                is_error: call.is_error,
            }),
            None => Err(format!(
                "the original call of {server}/{tool} did not return a result"
            )),
        }
    }

    fn is_live(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(server: &str, tool: &str, text: &str) -> RecordedCall {
        RecordedCall {
            seq: 0,
            step_id: "s".into(),
            server: server.into(),
            tool: tool.into(),
            arguments: JsonValue(json!({})),
            result: Some(JsonValue(
                json!({ "content": [{ "type": "text", "text": text }] }),
            )),
            is_error: false,
            denied: false,
        }
    }

    #[tokio::test]
    async fn answers_from_the_record_in_order_per_tool() {
        let replay = ReplayTools::new(
            &[
                call("github", "list", "first"),
                call("github", "get", "other"),
                call("github", "list", "second"),
            ],
            None,
        );
        let cancel = CancellationToken::new();
        let text = |outcome: ToolOutcome| {
            outcome.result["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        assert_eq!(replay.remaining(), 3);
        assert_eq!(
            text(
                replay
                    .call_tool("github", "list", json!({"any": 1}), &cancel)
                    .await
                    .unwrap()
            ),
            "first"
        );
        assert_eq!(
            text(
                replay
                    .call_tool("github", "list", json!({}), &cancel)
                    .await
                    .unwrap()
            ),
            "second"
        );
        assert_eq!(
            text(
                replay
                    .call_tool("github", "get", json!({}), &cancel)
                    .await
                    .unwrap()
            ),
            "other"
        );
        assert_eq!(replay.remaining(), 0);
        let error = replay
            .call_tool("github", "list", json!({}), &cancel)
            .await
            .unwrap_err();
        assert!(
            error.contains("no recorded result left for github/list"),
            "{error}"
        );
        assert!(!replay.is_live());
    }

    #[tokio::test]
    async fn the_recorded_error_flag_is_kept() {
        let mut failed = call("s", "t", "boom");
        failed.is_error = true;
        let replay = ReplayTools::new(&[failed], None);
        let outcome = replay
            .call_tool("s", "t", json!({}), &CancellationToken::new())
            .await
            .unwrap();
        assert!(outcome.is_error);
    }

    #[tokio::test]
    async fn denied_and_failed_calls_cannot_be_replayed() {
        let mut denied = call("s", "a", "");
        denied.denied = true;
        denied.result = None;
        let mut lost = call("s", "b", "");
        lost.result = None;
        lost.is_error = true;
        let replay = ReplayTools::new(&[denied, lost], None);
        let cancel = CancellationToken::new();
        assert!(replay
            .call_tool("s", "a", json!({}), &cancel)
            .await
            .unwrap_err()
            .contains("not allowed"));
        assert!(replay
            .call_tool("s", "b", json!({}), &cancel)
            .await
            .unwrap_err()
            .contains("did not return"));
    }

    #[tokio::test]
    async fn tools_come_from_the_catalog_or_else_from_the_record() {
        let replay = ReplayTools::new(
            &[call("github", "list", "x"), call("github", "get", "y")],
            None,
        );
        let tools = replay.list_tools("github").await.unwrap();
        assert_eq!(
            tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["get", "list"]
        );
        assert!(replay
            .list_tools("gitlab")
            .await
            .unwrap_err()
            .contains("gitlab"));
    }
}
