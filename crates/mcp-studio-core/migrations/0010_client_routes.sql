-- Entries of other clients' configuration files that were pointed at the MCP Studio proxy, so that
-- the original entry can be put back. `routed_entry` is the JSON that was written; the original is
-- read from `backup_path` (a copy of the whole file made before the change).
CREATE TABLE client_routes (
  id            TEXT PRIMARY KEY,
  client        TEXT NOT NULL,
  config_path   TEXT NOT NULL,
  -- JSON pointer to the object that holds the servers, e.g. /mcpServers
  location      TEXT NOT NULL,
  entry_name    TEXT NOT NULL,
  server_id     TEXT NOT NULL,
  routed_entry  TEXT NOT NULL,
  backup_path   TEXT NOT NULL,
  created_at    INTEGER NOT NULL,
  UNIQUE (config_path, location, entry_name)
);
