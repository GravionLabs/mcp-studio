use mcp_studio_core::{db::Db, model::AppInfo};
use tauri::Manager;

/// Shared state managed by Tauri.
pub struct AppState {
    pub db: Db,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo::current()
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
            app.manage(AppState { db });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![app_info])
        .run(tauri::generate_context!())
        .expect("error while running MCP Studio");
}
