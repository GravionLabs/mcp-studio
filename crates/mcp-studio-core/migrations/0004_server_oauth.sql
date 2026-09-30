-- Streamable HTTP servers can require OAuth 2.1 (browser sign-in). Tokens live in the OS keyring.
ALTER TABLE servers ADD COLUMN oauth INTEGER NOT NULL DEFAULT 0;
