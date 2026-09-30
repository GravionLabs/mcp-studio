use mcp_studio_core::{
    environments::{Environment, EnvironmentInput},
    model::AppInfo,
    registry::{ServerDefinition, ServerInput},
    secrets::{self, references_in},
};
use tauri::State;

use crate::{
    error::{CommandError, CommandResult},
    AppState,
};

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
    let previous = state.registry.get(&id).await?;
    let updated = state.registry.update(&id, input).await?;
    // Drop secrets the edited definition no longer references.
    let still_used = references_in(&updated.input);
    for name in references_in(&previous.input) {
        if !still_used.contains(&name) {
            state.secrets.delete(&name)?;
        }
    }
    Ok(updated)
}

#[tauri::command]
pub async fn server_remove(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    let existing = state.registry.get(&id).await?;
    state.registry.delete(&id).await?;
    for name in references_in(&existing.input) {
        state.secrets.delete(&name)?;
    }
    Ok(())
}

/// Stores a secret value in the OS keyring under `name` (the part after `keyring:`).
#[tauri::command]
pub fn secret_set(state: State<'_, AppState>, name: String, value: String) -> CommandResult<()> {
    if secrets::reference_name(&secrets::reference(&name)).is_none() {
        return Err(CommandError("secret name must not be empty".into()));
    }
    Ok(state.secrets.set(&name, &value)?)
}

#[tauri::command]
pub fn secret_delete(state: State<'_, AppState>, name: String) -> CommandResult<()> {
    Ok(state.secrets.delete(&name)?)
}

#[tauri::command]
pub async fn environment_list(state: State<'_, AppState>) -> CommandResult<Vec<Environment>> {
    Ok(state.environments.list().await?)
}

#[tauri::command]
pub async fn environment_add(
    state: State<'_, AppState>,
    input: EnvironmentInput,
) -> CommandResult<Environment> {
    Ok(state.environments.create(input).await?)
}

#[tauri::command]
pub async fn environment_update(
    state: State<'_, AppState>,
    id: String,
    input: EnvironmentInput,
) -> CommandResult<Environment> {
    let previous = state.environments.get(&id).await?;
    let updated = state.environments.update(&id, input).await?;
    let still_used = secret_names(&updated.input);
    for name in secret_names(&previous.input) {
        if !still_used.contains(&name) {
            state.secrets.delete(&name)?;
        }
    }
    Ok(updated)
}

#[tauri::command]
pub async fn environment_remove(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    let existing = state.environments.get(&id).await?;
    state.environments.delete(&id).await?;
    for name in secret_names(&existing.input) {
        state.secrets.delete(&name)?;
    }
    Ok(())
}

fn secret_names(input: &EnvironmentInput) -> Vec<String> {
    input
        .variables
        .values()
        .filter_map(|v| secrets::reference_name(v).map(str::to_owned))
        .collect()
}
