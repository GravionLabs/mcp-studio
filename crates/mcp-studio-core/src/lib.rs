//! MCP Studio core. Must never depend on Tauri.

pub mod bindings;
pub mod model;

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
