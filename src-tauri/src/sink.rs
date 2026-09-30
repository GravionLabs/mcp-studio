use mcp_studio_core::events::{EventSink, ListChangedEvent, LogEvent, MessageRecord, StatusEvent};
use tauri::{AppHandle, Emitter};

/// Forwards core events to the webview.
pub struct TauriSink {
    pub app: AppHandle,
}

impl EventSink for TauriSink {
    fn status(&self, event: StatusEvent) {
        let _ = self.app.emit("mcp://status", event);
    }

    fn message(&self, event: MessageRecord) {
        let _ = self.app.emit("mcp://message", event);
    }

    fn log(&self, event: LogEvent) {
        let _ = self.app.emit("mcp://log", event);
    }

    fn list_changed(&self, event: ListChangedEvent) {
        let _ = self.app.emit("mcp://list-changed", event);
    }
}
