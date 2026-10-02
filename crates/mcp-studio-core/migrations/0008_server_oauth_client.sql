-- Authorization servers without dynamic client registration (Microsoft Entra ID) need a client ID that
-- was registered by hand, optionally the scopes to request and a fixed port for the loopback redirect.
ALTER TABLE servers ADD COLUMN oauth_client_id TEXT;
ALTER TABLE servers ADD COLUMN oauth_scopes TEXT;
ALTER TABLE servers ADD COLUMN oauth_callback_port INTEGER;
