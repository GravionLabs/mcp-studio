-- Small key-value settings that the Rust side needs (the webview keeps its own in local storage).
CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- Spans the OpenTelemetry exporter has already sent.
ALTER TABLE spans ADD COLUMN exported INTEGER NOT NULL DEFAULT 0;
CREATE INDEX spans_unexported ON spans (exported) WHERE exported = 0;
