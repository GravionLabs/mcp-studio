//! Requests a server sends to the client (sampling, elicitation) wait here until the user answers.
//!
//! The handler of a session calls [`ClientRequests::ask`], which tells the UI and waits. The UI
//! answers with [`ClientRequests::answer`]. Nothing is answered on its own.

use std::{collections::HashMap, sync::Arc, sync::Mutex};

use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::{
    db::{new_id, now_ms, DbError, DbResult},
    events::EventSink,
    llm::{CompletionRequest, ContentBlock, Message, Role},
    model::JsonValue,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ClientRequestKind {
    /// `sampling/createMessage`: the server asks for a model answer.
    Sampling,
    /// `elicitation/create`: the server asks the user for input.
    Elicitation,
}

/// A request that waits for the user. Sent to the UI as `mcp://client-request`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClientRequest {
    pub id: String,
    pub server_id: String,
    pub kind: ClientRequestKind,
    /// The params of the MCP request as the server sent them.
    pub params: JsonValue,
    #[specta(type = u32)]
    pub ts: i64,
}

/// A request is no longer waiting: it was answered, or the server withdrew it or disconnected.
/// Sent to the UI as `mcp://client-request-done`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClientRequestDone {
    pub id: String,
    pub server_id: String,
}

/// What the user decided.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum ClientAnswer {
    /// Sampling: answer with this text. `model` names who wrote it; empty means "manual".
    Respond { text: String, model: Option<String> },
    /// Sampling: refuse the request.
    Reject,
    /// Elicitation: the user filled in the form.
    Accept { content: JsonValue },
    /// Elicitation: the user refused to answer but the operation may go on.
    Decline,
    /// Elicitation: the user stops the operation.
    Cancel,
}

impl ClientAnswer {
    fn fits(&self, kind: ClientRequestKind) -> bool {
        match self {
            Self::Respond { .. } | Self::Reject => kind == ClientRequestKind::Sampling,
            Self::Accept { .. } | Self::Decline | Self::Cancel => {
                kind == ClientRequestKind::Elicitation
            }
        }
    }
}

/// What a provider wrote for a sampling request. The user still has to send it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SamplingSuggestion {
    pub text: String,
    pub model: String,
}

/// The model request that answers the params of a `sampling/createMessage` request.
///
/// Only text goes to the model; other content (images, audio, tool use) is replaced by a note so
/// that nothing the user did not see is sent on.
pub fn sampling_to_completion(
    params: &serde_json::Value,
    model: &str,
) -> DbResult<CompletionRequest> {
    let mut messages: Vec<Message> = Vec::new();
    for message in params["messages"].as_array().into_iter().flatten() {
        let role = match message["role"].as_str() {
            Some("assistant") => Role::Assistant,
            _ => Role::User,
        };
        let blocks = match &message["content"] {
            serde_json::Value::Array(blocks) => blocks.clone(),
            single => vec![single.clone()],
        };
        let text = blocks
            .iter()
            .map(|block| match block["type"].as_str() {
                Some("text") => block["text"].as_str().unwrap_or_default().to_owned(),
                other => format!("[{} content not shown]", other.unwrap_or("unknown")),
            })
            .collect::<Vec<_>>()
            .join("\n");
        messages.push(Message {
            role,
            content: vec![ContentBlock::text(text)],
        });
    }
    if messages.is_empty() {
        return Err(DbError::Invalid("the request has no messages".into()));
    }
    let mut request = CompletionRequest::new(model, messages);
    request.system = params["systemPrompt"].as_str().map(str::to_owned);
    if let Some(max) = params["maxTokens"].as_u64() {
        request.max_tokens = u32::try_from(max).unwrap_or(u32::MAX);
    }
    request.temperature = params["temperature"].as_f64();
    Ok(request)
}

struct Entry {
    request: ClientRequest,
    answer: oneshot::Sender<ClientAnswer>,
}

/// The requests that wait for an answer, across all servers.
pub struct ClientRequests {
    sink: Arc<dyn EventSink>,
    pending: Mutex<HashMap<String, Entry>>,
}

impl ClientRequests {
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self {
            sink,
            pending: Mutex::default(),
        }
    }

    /// Shows a request to the user and waits for the answer. `None` when the server withdrew the
    /// request (`cancelled`) or its session ended.
    pub async fn ask(
        &self,
        server_id: &str,
        kind: ClientRequestKind,
        params: serde_json::Value,
        cancelled: &CancellationToken,
    ) -> Option<ClientAnswer> {
        let request = ClientRequest {
            id: new_id(),
            server_id: server_id.to_owned(),
            kind,
            params: JsonValue(params),
            ts: now_ms(),
        };
        let (answer, receiver) = oneshot::channel();
        self.pending.lock().unwrap().insert(
            request.id.clone(),
            Entry {
                request: request.clone(),
                answer,
            },
        );
        self.sink.client_request(request.clone());
        let outcome = tokio::select! {
            answer = receiver => answer.ok(),
            () = cancelled.cancelled() => None,
        };
        self.pending.lock().unwrap().remove(&request.id);
        self.sink.client_request_done(ClientRequestDone {
            id: request.id,
            server_id: request.server_id,
        });
        outcome
    }

    /// Gives the answer to the request that waits for it.
    pub fn answer(&self, id: &str, answer: ClientAnswer) -> DbResult<()> {
        let mut pending = self.pending.lock().unwrap();
        let entry = pending
            .get(id)
            .ok_or_else(|| DbError::NotFound("the request is no longer waiting".into()))?;
        if !answer.fits(entry.request.kind) {
            return Err(DbError::Invalid(
                "this answer does not fit the kind of the request".into(),
            ));
        }
        let entry = pending.remove(id).expect("checked above");
        // The handler may have been cancelled in the meantime; then nobody listens.
        let _ = entry.answer.send(answer);
        Ok(())
    }

    /// The requests that wait, oldest first, optionally of one server.
    pub fn pending(&self, server_id: Option<&str>) -> Vec<ClientRequest> {
        let mut requests: Vec<ClientRequest> = self
            .pending
            .lock()
            .unwrap()
            .values()
            .map(|e| e.request.clone())
            .filter(|r| server_id.is_none_or(|id| r.server_id == id))
            .collect();
        requests.sort_by_key(|r| r.ts);
        requests
    }

    /// One request that waits.
    pub fn get(&self, id: &str) -> Option<ClientRequest> {
        self.pending
            .lock()
            .unwrap()
            .get(id)
            .map(|e| e.request.clone())
    }

    /// Drops everything a server still waits for (its session ended).
    pub fn drop_server(&self, server_id: &str) {
        self.pending
            .lock()
            .unwrap()
            .retain(|_, e| e.request.server_id != server_id);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::events::CollectingSink;

    fn requests() -> (Arc<ClientRequests>, Arc<CollectingSink>) {
        let sink = Arc::new(CollectingSink::default());
        (Arc::new(ClientRequests::new(sink.clone())), sink)
    }

    async fn waiting(requests: &ClientRequests) -> ClientRequest {
        for _ in 0..200 {
            if let Some(request) = requests.pending(None).into_iter().next() {
                return request;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("no request arrived");
    }

    #[test]
    fn a_sampling_request_becomes_a_model_request_with_text_only() {
        let request = sampling_to_completion(
            &json!({
                "messages": [
                    {"role": "user", "content": {"type": "text", "text": "Hi"}},
                    {"role": "assistant", "content": [{"type": "text", "text": "Hello"}, {"type": "image", "data": "AAAA", "mimeType": "image/png"}]},
                    {"role": "user", "content": {"type": "text", "text": "Again"}}
                ],
                "systemPrompt": "Be brief",
                "maxTokens": 50,
                "temperature": 0.5
            }),
            "claude-x",
        )
        .unwrap();
        assert_eq!(request.model, "claude-x");
        assert_eq!(request.system.as_deref(), Some("Be brief"));
        assert_eq!(request.max_tokens, 50);
        assert_eq!(request.temperature, Some(0.5));
        assert_eq!(request.messages.len(), 3);
        assert_eq!(request.messages[1].role, Role::Assistant);
        assert_eq!(
            request.messages[1].content,
            [ContentBlock::text("Hello\n[image content not shown]")]
        );
        assert!(sampling_to_completion(&json!({"messages": []}), "m").is_err());
    }

    #[tokio::test]
    async fn an_answer_reaches_the_asking_handler() {
        let (requests, sink) = requests();
        let asking = {
            let requests = requests.clone();
            tokio::spawn(async move {
                requests
                    .ask(
                        "s1",
                        ClientRequestKind::Elicitation,
                        json!({"message": "name?"}),
                        &CancellationToken::new(),
                    )
                    .await
            })
        };
        let request = waiting(&requests).await;
        assert_eq!(request.server_id, "s1");
        assert_eq!(request.params.0["message"], "name?");
        requests.answer(&request.id, ClientAnswer::Decline).unwrap();
        assert_eq!(asking.await.unwrap(), Some(ClientAnswer::Decline));
        assert!(requests.pending(None).is_empty());
        assert_eq!(sink.client_requests.lock().unwrap().len(), 1);
        assert_eq!(sink.client_requests_done.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_answer_of_the_wrong_kind_is_refused_and_the_request_keeps_waiting() {
        let (requests, _) = requests();
        let asking = {
            let requests = requests.clone();
            tokio::spawn(async move {
                requests
                    .ask(
                        "s1",
                        ClientRequestKind::Sampling,
                        json!({}),
                        &CancellationToken::new(),
                    )
                    .await
            })
        };
        let request = waiting(&requests).await;
        assert!(requests.answer(&request.id, ClientAnswer::Cancel).is_err());
        assert_eq!(requests.pending(None).len(), 1);
        requests.answer(&request.id, ClientAnswer::Reject).unwrap();
        assert_eq!(asking.await.unwrap(), Some(ClientAnswer::Reject));
    }

    #[tokio::test]
    async fn a_withdrawn_request_stops_waiting() {
        let (requests, sink) = requests();
        let token = CancellationToken::new();
        let asking = {
            let (requests, token) = (requests.clone(), token.clone());
            tokio::spawn(async move {
                requests
                    .ask("s1", ClientRequestKind::Sampling, json!({}), &token)
                    .await
            })
        };
        let request = waiting(&requests).await;
        token.cancel();
        assert_eq!(asking.await.unwrap(), None);
        assert!(requests.answer(&request.id, ClientAnswer::Reject).is_err());
        assert_eq!(sink.client_requests_done.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn dropping_a_server_ends_its_requests_only() {
        let (requests, _) = requests();
        let mut tasks = Vec::new();
        for server in ["a", "b"] {
            let requests = requests.clone();
            tasks.push(tokio::spawn(async move {
                requests
                    .ask(
                        server,
                        ClientRequestKind::Sampling,
                        json!({}),
                        &CancellationToken::new(),
                    )
                    .await
            }));
        }
        while requests.pending(None).len() < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        requests.drop_server("a");
        assert_eq!(tasks.remove(0).await.unwrap(), None);
        let left = requests.pending(None);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].server_id, "b");
        requests.answer(&left[0].id, ClientAnswer::Reject).unwrap();
        assert_eq!(tasks.remove(0).await.unwrap(), Some(ClientAnswer::Reject));
    }
}
