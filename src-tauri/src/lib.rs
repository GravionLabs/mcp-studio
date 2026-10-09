mod commands;
mod error;
mod flow_runtime;
mod sink;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use mcp_studio_core::{
    collections::Collections,
    db::Db,
    environments::Environments,
    flow_runs::FlowRuns,
    flows::Flows,
    http_proxy::{self, HttpProxy},
    otlp::TraceExporter,
    prices::Prices,
    proxy::{discovery_path, ProxyService},
    registry::Registry,
    secrets::{KeyringStore, SecretStore},
    session::SessionManager,
    settings::Settings,
    storage,
    test_suites::TestSuites,
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
    pub test_suites: TestSuites,
    pub flow_runs: FlowRuns,
    /// Runs in progress, so they can be cancelled.
    pub running_flows: Mutex<HashMap<String, tokio_util::sync::CancellationToken>>,
    pub confirmations: Arc<flow_runtime::Confirmations>,
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
                let settings = Settings::new(cleanup_db.clone());
                let policy = storage::load_policy(&settings).await.unwrap_or_default();
                let _ = storage::apply_policy(&cleanup_db, policy).await;
            });
            let registry = Registry::new(db.clone());
            let environments = Environments::new(db.clone());
            let collections = Collections::new(db.clone());
            let prices = Prices::new(db.clone());
            let flows = Flows::new(db.clone());
            let test_suites = TestSuites::new(db.clone());
            let flow_runs = FlowRuns::new(db.clone());
            {
                // Runs that were still going when the app stopped will never finish.
                let flow_runs = flow_runs.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = flow_runs
                        .fail_interrupted(mcp_studio_core::db::now_ms())
                        .await;
                });
            }
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
                test_suites,
                flow_runs,
                running_flows: Mutex::new(HashMap::new()),
                confirmations: Arc::new(flow_runtime::Confirmations::default()),
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
            commands::retention_get,
            commands::retention_set,
            commands::storage_info,
            commands::history_delete_all,
            commands::workspace_export,
            commands::workspace_preview,
            commands::workspace_import,
            commands::token_counting_status,
            commands::token_counting_set_model,
            commands::message_count_exact,
            commands::flow_list,
            commands::flow_get,
            commands::flow_save,
            commands::flow_delete,
            commands::flow_export,
            commands::flow_import,
            commands::flow_run_start,
            commands::flow_run_replay,
            commands::flow_run_cancel,
            commands::flow_run_get,
            commands::flow_run_list,
            commands::flow_run_delete,
            commands::flow_confirm,
            commands::tool_policy_get,
            commands::tool_policy_set,
            commands::flow_validate,
            commands::flow_generate,
            commands::flow_to_yaml,
            commands::flow_from_yaml,
            commands::provider_status,
            commands::provider_set_settings,
            commands::provider_set_key,
            commands::ollama_models,
            commands::provider_test,
            commands::server_docs,
            commands::server_docs_export,
            commands::test_suite_list,
            commands::test_suite_save,
            commands::test_suite_delete,
            commands::variants_propose,
            commands::variants_run,
            commands::tools_lint,
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
            commands::azure_login,
            commands::demo_server_path,
            commands::proxy_set_environment,
            commands::oauth_sign_in,
            commands::oauth_sign_out,
            commands::oauth_status,
            commands::import_sources,
            commands::client_entries,
            commands::client_route_preview,
            commands::client_route,
            commands::client_unroute,
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
