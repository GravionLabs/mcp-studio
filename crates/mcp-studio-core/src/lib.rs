//! MCP Studio core. Must never depend on Tauri.

pub mod bindings;
pub mod client_import;
pub mod collections;
pub mod db;
pub mod environments;
pub mod events;
pub mod explorer;
pub mod flow;
pub mod history;
pub mod http_proxy;
pub mod message_store;
pub mod model;
pub mod oauth;
pub mod path_env;
pub mod placeholders;
pub mod proxy;
pub mod recording;
pub mod registry;
pub mod secrets;
pub mod session;

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
