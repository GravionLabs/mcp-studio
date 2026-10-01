mod commands;
mod error;
mod sink;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use mcp_studio_core::{
    collections::Collections,
    db::Db,
    environments::Environments,
    flows::Flows,
    http_proxy::{self, HttpProxy},
    message_store::{self, RetentionPolicy},
    otlp::TraceExporter,
    prices::Prices,
    proxy::{discovery_path, ProxyService},
    registry::Registry,
    secrets::{KeyringStore, SecretStore},
    session::SessionManager,
    settings::Settings,
};
use tauri::Manager;

/// Shared state managed by Tauri.
pub struct AppState {
    pub db: Db,
    pub registry: Registry,
    pub environments: Environments,
    pub collections: Collections,
    pub prices: Prices,
    pub flows: Flows,
    pub trace_exporter: Arc<TraceExporter>,
    pub settings: Settings,
    pub secrets: Arc<dyn SecretStore>,
    pub sessions: Arc<SessionManager>,
    pub proxy: ProxyService,
    pub http_proxy: HttpProxy,
    pub discovery_file: PathBuf,
    /// The update found by the last check, waiting to be installed.
    pub pending_update: Mutex<Option<tauri_plugin_updater::Update>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            let prices = Prices::new(db.clone());
            let flows = Flows::new(db.clone());
            let settings = Settings::new(db.clone());
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
            let trace_exporter = TraceExporter::new(db.clone(), secrets.clone());
            tauri::async_runtime::spawn(trace_exporter.clone().run());
            app.manage(AppState {
                db,
                registry,
                environments,
                collections,
                prices,
                flows,
                trace_exporter,
                settings,
                secrets,
                sessions,
                proxy,
                http_proxy,
                discovery_file,
                pending_update: Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::update_check,
            commands::update_install,
            commands::spans_query,
            commands::trace_export_config,
            commands::trace_export_set_config,
            commands::trace_export_status,
            commands::trace_export_now,
            commands::token_counting_status,
            commands::token_counting_set_model,
            commands::token_counting_set_key,
            commands::message_count_exact,
            commands::flow_list,
            commands::flow_get,
            commands::flow_save,
            commands::flow_delete,
            commands::flow_export,
            commands::flow_import,
            commands::flow_to_yaml,
            commands::flow_from_yaml,
            commands::provider_test_anthropic,
            commands::price_list,
            commands::price_set,
            commands::price_remove,
            commands::tools_context_cost,
            commands::session_usage,
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
