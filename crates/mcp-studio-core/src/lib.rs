//! MCP Studio core. Must never depend on Tauri.

pub mod azure_auth;
pub mod bindings;
pub mod client_import;
pub mod collections;
pub mod compare;
pub mod db;
pub mod docs_gen;
pub mod environments;
pub mod events;
pub mod explorer;
pub mod flow;
pub mod flow_expr;
pub mod flow_gen;
pub mod flow_replay;
pub mod flow_run;
pub mod flow_runs;
pub mod flow_yaml;
pub mod flows;
pub mod history;
pub mod http_proxy;
pub mod lint;
pub mod llm;
pub mod message_store;
pub mod metering;
pub mod model;
pub mod oauth;
pub mod otlp;
pub mod path_env;
pub mod placeholders;
pub mod prices;
pub mod proxy;
pub mod recording;
pub mod registry;
pub mod secrets;
pub mod session;
pub mod settings;
pub mod storage;
pub mod test_suites;
pub mod tokens;
pub mod trace;
pub mod update;

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
