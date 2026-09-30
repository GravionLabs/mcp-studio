//! MCP Studio core. Must never depend on Tauri.

pub mod bindings;
pub mod db;
pub mod environments;
pub mod events;
pub mod message_store;
pub mod model;
pub mod placeholders;
pub mod recording;
pub mod registry;
pub mod secrets;

/// Returns the crate version, used by the app shell and smoke tests.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_semver_like() {
        assert_eq!(version().split('.').count(), 3);
    }
}
