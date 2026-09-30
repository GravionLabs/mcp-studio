mod commands;
mod error;
mod sink;

use std::{path::PathBuf, sync::Arc};

use mcp_studio_core::{
    collections::Collections,
    db::Db,
    environments::Environments,
    http_proxy::{self, HttpProxy},
    message_store::{self, RetentionPolicy},
    proxy::{discovery_path, ProxyService},
    registry::Registry,
    secrets::{KeyringStore, SecretStore},
    session::SessionManager,
};
use tauri::Manager;

/// Shared state managed by Tauri.
pub struct AppState {
    pub db: Db,
    pub registry: Registry,
    pub environments: Environments,
    pub collections: Collections,
    pub secrets: Arc<dyn SecretStore>,
    pub sessions: Arc<SessionManager>,
    pub proxy: ProxyService,
    pub http_proxy: HttpProxy,
    pub discovery_file: PathBuf,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
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
                let _ = mcp_studio_core::history::History::new(cleanup_db)
                    .cleanup(5000, 30)
                    .await;
            });
            let registry = Registry::new(db.clone());
            let environments = Environments::new(db.clone());
            let collections = Collections::new(db.clone());
            let secrets: Arc<dyn SecretStore> =
                Arc::new(KeyringStore::new("dev.gravionlabs.mcp-studio"));
            let sink = Arc::new(sink::TauriSink {
                app: app.handle().clone(),
            });
            let sessions = SessionManager::new(
                db.clone(),
                registry.clone(),
                environments.clone(),
                secrets.clone(),
                sink.clone(),
            );
            let discovery_file = discovery_path(&dir);
            let proxy = tauri::async_runtime::block_on(ProxyService::start(
                db.clone(),
                registry.clone(),
                environments.clone(),
                secrets.clone(),
                sink.clone(),
                &discovery_file,
            ))?;
            let http_proxy = tauri::async_runtime::block_on(HttpProxy::start(
                db.clone(),
                registry.clone(),
                environments.clone(),
                secrets.clone(),
                sink,
                http_proxy::DEFAULT_PORT,
            ))?;
            app.manage(AppState {
                db,
                registry,
                environments,
                collections,
                secrets,
                sessions,
                proxy,
                http_proxy,
                discovery_file,
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
            commands::server_connect,
            commands::server_disconnect,
            commands::server_logs,
            commands::server_details,
            commands::tools_list,
            commands::resources_list,
            commands::resource_templates_list,
            commands::prompts_list,
            commands::resource_read,
            commands::prompt_get,
            commands::collections_tree,
            commands::collection_create,
            commands::collection_rename,
            commands::collection_move,
            commands::collection_delete,
            commands::request_save,
            commands::request_update,
            commands::request_delete,
            commands::collection_export,
            commands::collection_import,
            commands::history_list,
            commands::history_clear,
            commands::proxy_info,
            commands::proxy_set_environment,
            commands::oauth_sign_in,
            commands::oauth_sign_out,
            commands::oauth_status,
            commands::import_sources,
            commands::import_preview,
            commands::import_apply,
            commands::tool_call,
            commands::request_cancel,
            commands::messages_query,
            commands::secret_set,
            commands::secret_delete,
        ])
        .build(tauri::generate_context!())
        .expect("error while building MCP Studio")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<AppState>() {
                    tauri::async_runtime::block_on(async {
                        state.sessions.disconnect_all().await;
                        state.http_proxy.shutdown().await;
                    });
                }
            }
        });
}
