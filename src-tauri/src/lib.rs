mod commands;
mod error;

use std::sync::Arc;

use mcp_studio_core::{
    db::Db,
    environments::Environments,
    message_store::{self, RetentionPolicy},
    registry::Registry,
    secrets::{KeyringStore, SecretStore},
};
use tauri::Manager;

/// Shared state managed by Tauri.
pub struct AppState {
    pub db: Db,
    pub registry: Registry,
    pub environments: Environments,
    pub secrets: Arc<dyn SecretStore>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let db = tauri::async_runtime::block_on(Db::open(&dir.join("mcp-studio.sqlite")))?;
            let cleanup_db = db.clone();
            tauri::async_runtime::spawn(async move {
                let _ = message_store::cleanup(&cleanup_db, RetentionPolicy::default()).await;
            });
            let registry = Registry::new(db.clone());
            let environments = Environments::new(db.clone());
            let secrets: Arc<dyn SecretStore> =
                Arc::new(KeyringStore::new("dev.gravionlabs.mcp-studio"));
            app.manage(AppState {
                db,
                registry,
                environments,
                secrets,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::server_list,
            commands::server_get,
            commands::server_add,
            commands::server_update,
            commands::server_remove,
            commands::environment_list,
            commands::environment_add,
            commands::environment_update,
            commands::environment_remove,
            commands::messages_query,
            commands::secret_set,
            commands::secret_delete,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MCP Studio");
}
