-- The folders MCP Studio offers a server as its roots (JSON array of paths or file:// URIs).
ALTER TABLE servers ADD COLUMN roots TEXT NOT NULL DEFAULT '[]';
