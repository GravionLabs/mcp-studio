use mcp_studio_core::db::DbError;
use serde::{Serialize, Serializer};

/// Error type returned by Tauri commands; serializes as a plain message string.
#[derive(Debug)]
pub struct CommandError(pub String);

impl Serialize for CommandError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl From<DbError> for CommandError {
    fn from(error: DbError) -> Self {
        Self(match error {
            // Show only the message for user-facing validation errors.
            DbError::Invalid(message) => message,
            other => other.to_string(),
        })
    }
}

pub type CommandResult<T> = Result<T, CommandError>;
