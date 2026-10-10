use mcp_studio_core::client_requests::{ClientRequest, ClientRequestDone};
use mcp_studio_core::events::{
    EventSink, ListChangedEvent, LogEvent, MessageRecord, ProgressEvent, StatusEvent,
};
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

    fn progress(&self, event: ProgressEvent) {
        let _ = self.app.emit("mcp://progress", event);
    }

    fn client_request(&self, event: ClientRequest) {
        let _ = self.app.emit("mcp://client-request", event);
    }

    fn client_request_done(&self, event: ClientRequestDone) {
        let _ = self.app.emit("mcp://client-request-done", event);
    }
}

/// Opens URLs (the OAuth authorization page) in the user's default browser.
pub struct BrowserOpener {
    pub app: AppHandle,
}

impl mcp_studio_core::oauth::UrlOpener for BrowserOpener {
    fn open(&self, url: &str) -> Result<(), String> {
        use tauri_plugin_opener::OpenerExt;
        self.app
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|e| e.to_string())
    }
}
