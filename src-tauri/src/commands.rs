use mcp_studio_core::{
    model::AppInfo,
    registry::{ServerDefinition, ServerInput},
};
use tauri::State;

use crate::{error::CommandResult, AppState};

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo::current()
}

#[tauri::command]
pub async fn server_list(state: State<'_, AppState>) -> CommandResult<Vec<ServerDefinition>> {
    Ok(state.registry.list().await?)
}

#[tauri::command]
pub async fn server_get(state: State<'_, AppState>, id: String) -> CommandResult<ServerDefinition> {
    Ok(state.registry.get(&id).await?)
}

#[tauri::command]
pub async fn server_add(
    state: State<'_, AppState>,
    input: ServerInput,
) -> CommandResult<ServerDefinition> {
    Ok(state.registry.create(input).await?)
}

#[tauri::command]
pub async fn server_update(
    state: State<'_, AppState>,
    id: String,
    input: ServerInput,
) -> CommandResult<ServerDefinition> {
    Ok(state.registry.update(&id, input).await?)
}

#[tauri::command]
pub async fn server_remove(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    Ok(state.registry.delete(&id).await?)
}
