-- Errors are common filter targets in the inspector, so keep them queryable without parsing JSON.
ALTER TABLE messages ADD COLUMN is_error INTEGER NOT NULL DEFAULT 0;
CREATE INDEX messages_errors ON messages (session_id, is_error) WHERE is_error = 1;
