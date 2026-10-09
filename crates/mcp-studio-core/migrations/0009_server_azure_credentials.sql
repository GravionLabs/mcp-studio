-- Servers behind Microsoft Entra ID can take their token from the user's Azure login (Azure CLI or
-- Azure Developer CLI) instead of an OAuth sign-in with a registered client.
ALTER TABLE servers ADD COLUMN azure_credentials INTEGER NOT NULL DEFAULT 0;
