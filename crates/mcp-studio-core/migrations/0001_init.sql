-- MCP Studio initial schema. Timestamps are unix milliseconds; JSON is stored as TEXT.

CREATE TABLE servers (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  transport   TEXT NOT NULL CHECK (transport IN ('stdio', 'http')),
  command     TEXT,
  args        TEXT NOT NULL DEFAULT '[]',
  env         TEXT NOT NULL DEFAULT '{}',
  cwd         TEXT,
  url         TEXT,
  headers     TEXT NOT NULL DEFAULT '{}',
  tags        TEXT NOT NULL DEFAULT '[]',
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);

CREATE TABLE environments (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL UNIQUE,
  variables  TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE collections (
  id          TEXT PRIMARY KEY,
  parent_id   TEXT REFERENCES collections (id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  sort_order  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE requests (
  id             TEXT PRIMARY KEY,
  collection_id  TEXT NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
  server_id      TEXT NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
  method         TEXT NOT NULL,
  name           TEXT NOT NULL,
  tool_name      TEXT,
  arguments      TEXT NOT NULL DEFAULT '{}',
  notes          TEXT NOT NULL DEFAULT '',
  sort_order     INTEGER NOT NULL DEFAULT 0,
  created_at     INTEGER NOT NULL,
  updated_at     INTEGER NOT NULL
);
CREATE INDEX requests_collection ON requests (collection_id);

CREATE TABLE sessions (
  id               TEXT PRIMARY KEY,
  server_id        TEXT NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
  origin           TEXT NOT NULL CHECK (origin IN ('studio', 'proxy')),
  started_at       INTEGER NOT NULL,
  ended_at         INTEGER,
  protocol_version TEXT,
  server_info      TEXT,
  capabilities     TEXT
);
CREATE INDEX sessions_server ON sessions (server_id, started_at);

CREATE TABLE spans (
  id          TEXT PRIMARY KEY,
  trace_id    TEXT NOT NULL,
  parent_id   TEXT REFERENCES spans (id) ON DELETE CASCADE,
  kind        TEXT NOT NULL CHECK (kind IN ('session', 'flow', 'step', 'llm', 'tool')),
  name        TEXT NOT NULL,
  started_at  INTEGER NOT NULL,
  ended_at    INTEGER,
  status      TEXT NOT NULL DEFAULT 'ok',
  attributes  TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX spans_trace ON spans (trace_id);

CREATE TABLE messages (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id    TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
  span_id       TEXT REFERENCES spans (id) ON DELETE SET NULL,
  direction     TEXT NOT NULL CHECK (direction IN ('out', 'in')),
  jsonrpc_id    TEXT,
  method        TEXT,
  payload       TEXT NOT NULL,
  bytes         INTEGER NOT NULL,
  tokens        INTEGER,
  token_source  TEXT CHECK (token_source IN ('estimate', 'exact')),
  ts            INTEGER NOT NULL
);
CREATE INDEX messages_session ON messages (session_id, id);
CREATE INDEX messages_method ON messages (method);

CREATE TABLE flows (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  graph       TEXT NOT NULL,
  version     INTEGER NOT NULL DEFAULT 1,
  updated_at  INTEGER NOT NULL
);

CREATE TABLE prices (
  model                 TEXT PRIMARY KEY,
  input_per_mtok        REAL NOT NULL,
  output_per_mtok       REAL NOT NULL,
  cache_read_per_mtok   REAL,
  cache_write_per_mtok  REAL,
  currency              TEXT NOT NULL DEFAULT 'USD'
);
