-- Every tool call, resource read, and prompt request made from MCP Studio, for the history view.
CREATE TABLE history (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  server_id   TEXT NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
  method      TEXT NOT NULL,
  target      TEXT NOT NULL,
  arguments   TEXT NOT NULL DEFAULT '{}',
  is_error    INTEGER NOT NULL DEFAULT 0,
  cancelled   INTEGER NOT NULL DEFAULT 0,
  duration_ms INTEGER,
  result      TEXT,
  error       TEXT,
  ts          INTEGER NOT NULL
);
CREATE INDEX history_server_ts ON history (server_id, ts DESC);
CREATE INDEX history_ts ON history (ts DESC);
