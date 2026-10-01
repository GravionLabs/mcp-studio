//! Connects the flow engine to the app: the connected MCP servers, the model providers, the
//! questions to the user, and the progress events of the webview.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use mcp_studio_core::{
    db::new_id,
    explorer::{self, ToolInfo},
    flow_run::{
        ConfirmEvent, ConfirmRequest, Confirmer, Decision, ModelResolver, RunEvent, RunObserver,
        ToolOutcome, ToolRunner,
    },
    llm::{LlmError, LlmProvider, ProviderSettings},
    model::JsonValue,
    registry::Registry,
    secrets::SecretStore,
    session::{SessionManager, ToolCallRequest},
};
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

/// Calls the tools of the servers registered in MCP Studio, connecting them when needed.
pub struct SessionTools {
    pub sessions: Arc<SessionManager>,
    pub registry: Registry,
    /// The environment used when a server has to be connected.
    pub environment_id: Option<String>,
}

impl SessionTools {
    /// The id of the server with this name, connected.
    async fn connected(&self, name: &str) -> Result<String, String> {
        let servers = self.registry.list().await.map_err(|e| e.to_string())?;
        let server = servers
            .iter()
            .find(|s| s.input.name == name)
            .ok_or_else(|| format!("there is no server named {name:?}"))?;
        self.sessions
            .connect(&server.id, self.environment_id.as_deref())
            .await
            .map_err(|e| format!("could not connect to {name:?}: {e}"))?;
        Ok(server.id.clone())
    }
}

#[async_trait]
impl ToolRunner for SessionTools {
    async fn list_tools(&self, server: &str) -> Result<Vec<ToolInfo>, String> {
        let id = self.connected(server).await?;
        let peer = self.sessions.peer(&id).map_err(|e| e.to_string())?;
        explorer::list_tools(&peer).await.map_err(|e| e.to_string())
    }

    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
        cancel: &CancellationToken,
    ) -> Result<ToolOutcome, String> {
        let server_id = self.connected(server).await?;
        let call_id = new_id();
        // Tell the server to stop when the run is cancelled, even if the engine has already
        // stopped waiting for the answer.
        let canceller = {
            let sessions = self.sessions.clone();
            let call_id = call_id.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                cancel.cancelled().await;
                sessions.cancel_call(&call_id);
            })
        };
        let result = self
            .sessions
            .call_tool(ToolCallRequest {
                server_id,
                tool_name: tool.to_owned(),
                arguments: JsonValue(arguments),
                // Placeholders were filled in by the flow; no environment is applied on top.
                environment_id: None,
                call_id,
            })
            .await;
        canceller.abort();
        match result {
            Ok(done) if done.cancelled => Err("the call was cancelled".into()),
            Ok(done) => Ok(ToolOutcome {
                result: done.result.0,
                is_error: done.is_error,
            }),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// Picks the model provider with the settings and keys that were current when the run started.
pub struct ConfiguredModels {
    pub settings: ProviderSettings,
    pub secrets: Arc<dyn SecretStore>,
}

impl ModelResolver for ConfiguredModels {
    fn resolve(&self, model: &str) -> Result<(Arc<dyn LlmProvider>, String), LlmError> {
        let resolved = mcp_studio_llm::resolve(&self.settings, self.secrets.as_ref(), model)?;
        Ok((resolved.provider, resolved.model))
    }
}

/// Questions that wait for the user's answer from the webview.
#[derive(Default)]
pub struct Confirmations {
    waiting: Mutex<HashMap<String, oneshot::Sender<Decision>>>,
}

impl Confirmations {
    /// Delivers an answer. Returns whether the question was still waiting.
    pub fn answer(&self, id: &str, decision: Decision) -> bool {
        match self.waiting.lock().unwrap().remove(id) {
            Some(sender) => sender.send(decision).is_ok(),
            None => false,
        }
    }
}

/// Removes a question that was dropped without an answer (the run was cancelled).
struct Waiting<'a> {
    confirmations: &'a Confirmations,
    id: String,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.confirmations.waiting.lock().unwrap().remove(&self.id);
    }
}

pub struct AskInWebview {
    pub app: AppHandle,
    pub confirmations: Arc<Confirmations>,
}

#[async_trait]
impl Confirmer for AskInWebview {
    async fn confirm(&self, request: &ConfirmRequest) -> Decision {
        let id = new_id();
        let (sender, receiver) = oneshot::channel();
        self.confirmations
            .waiting
            .lock()
            .unwrap()
            .insert(id.clone(), sender);
        let _waiting = Waiting {
            confirmations: &self.confirmations,
            id: id.clone(),
        };
        let event = ConfirmEvent {
            id,
            request: request.clone(),
        };
        if self.app.emit("flow://confirm", event).is_err() {
            return Decision::Deny;
        }
        // No answer (the window closed) means no.
        receiver.await.unwrap_or(Decision::Deny)
    }
}

/// Sends the progress of a run to the webview.
pub struct EmitProgress {
    pub app: AppHandle,
}

impl RunObserver for EmitProgress {
    fn event(&self, event: RunEvent) {
        let _ = self.app.emit("flow://event", event);
    }
}
