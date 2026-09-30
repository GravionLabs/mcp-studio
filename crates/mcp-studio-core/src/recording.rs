//! Message-level recording of everything that crosses an MCP transport.
//!
//! [`RecordingTransport`] wraps any `rmcp` transport and reports each JSON-RPC message (as
//! `serde_json::Value`) to a [`Recorder`] before it is sent or after it is received. It never
//! alters or delays the message. See ADR 0002 for why the tap sits at the `rmcp::Transport` layer
//! instead of the byte stream.

use std::{borrow::Cow, future::Future, marker::PhantomData, sync::Arc};

use rmcp::{
    service::{RxJsonRpcMessage, ServiceRole, TxJsonRpcMessage},
    transport::Transport,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::now_ms;

/// Direction of a message relative to the party that owns the transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Sent by us to the peer.
    Out,
    /// Received from the peer.
    In,
}

/// One recorded JSON-RPC message.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedMessage {
    pub direction: Direction,
    /// Unix milliseconds when the message passed the tap.
    pub ts: i64,
    pub payload: serde_json::Value,
    /// Size of the serialized message in bytes.
    pub bytes: usize,
}

/// Receives recorded messages. Implementations must not block or fail the connection.
pub trait Recorder: Send + Sync + 'static {
    fn record(&self, message: RecordedMessage);
}

impl<F> Recorder for F
where
    F: Fn(RecordedMessage) + Send + Sync + 'static,
{
    fn record(&self, message: RecordedMessage) {
        self(message)
    }
}

/// Transport wrapper that taps every message.
pub struct RecordingTransport<T, R> {
    inner: T,
    recorder: Arc<dyn Recorder>,
    _role: PhantomData<fn() -> R>,
}

impl<T, R> RecordingTransport<T, R> {
    pub fn new(inner: T, recorder: Arc<dyn Recorder>) -> Self {
        Self {
            inner,
            recorder,
            _role: PhantomData,
        }
    }
}

fn tap<M: Serialize>(recorder: &dyn Recorder, direction: Direction, message: &M) {
    // Serialization of a message we could already serialize for the wire cannot realistically fail;
    // if it does, dropping the record is better than breaking the connection.
    if let Ok(payload) = serde_json::to_value(message) {
        let bytes = payload.to_string().len();
        recorder.record(RecordedMessage {
            direction,
            ts: now_ms(),
            payload,
            bytes,
        });
    }
}

impl<T, R> Transport<R> for RecordingTransport<T, R>
where
    T: Transport<R>,
    R: ServiceRole,
    TxJsonRpcMessage<R>: Serialize,
    RxJsonRpcMessage<R>: Serialize,
{
    type Error = T::Error;

    fn name() -> Cow<'static, str> {
        format!("recording({})", T::name()).into()
    }

    fn send(
        &mut self,
        item: TxJsonRpcMessage<R>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        tap(self.recorder.as_ref(), Direction::Out, &item);
        self.inner.send(item)
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<R>> {
        let message = self.inner.receive().await;
        if let Some(message) = &message {
            tap(self.recorder.as_ref(), Direction::In, message);
        }
        message
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.inner.close()
    }
}
