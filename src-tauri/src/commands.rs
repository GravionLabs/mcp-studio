use mcp_studio_core::{
    client_import::{self, ConfigSource, ImportCandidate, ImportSummary},
    collections::{CollectionNode, CollectionTree, ImportReport, SavedRequest, SavedRequestInput},
    environments::{Environment, EnvironmentInput},
    events::{LogEvent, MessageRecord},
    explorer::{self, PromptInfo, ResourceInfo, ResourceTemplateInfo, ServerDetails, ToolInfo},
    history::{HistoryEntry, HistoryFilter},
    message_store::{query_messages, MessageFilter},
    metering::{self, ContextCost, SessionUsage},
    model::{AppInfo, JsonValue},
    oauth,
    otlp::{ExportConfig, ExportStatus},
    prices::Price,
    proxy::ProxyInfo,
    registry::{ServerDefinition, ServerInput},
    secrets::{self, references_in},
    session::{ToolCallRequest, ToolCallResult},
    trace::{query_spans, Span, SpanFilter},
    update::UpdateInfo,
};
use std::collections::BTreeMap;

use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::UpdaterExt;

use crate::{
    error::{CommandError, CommandResult},
    AppState,
};

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo::current()
}

/// Asks the update server whether a newer version exists. The update is kept for [`update_install`].
#[tauri::command]
pub async fn update_check(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<Option<UpdateInfo>> {
    let update = app
        .updater()
        .map_err(|e| CommandError(e.to_string()))?
        .check()
        .await
        .map_err(|e| CommandError(format!("Could not check for updates: {e}")))?;
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone(),
        date: u.date.map(|d| d.to_string()),
    });
    *state.pending_update.lock().unwrap() = update;
    Ok(info)
}

/// Downloads and installs the update found by the last check. The app restarts to finish it.
#[tauri::command]
pub async fn update_install(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    let update = state.pending_update.lock().unwrap().take();
    let Some(update) = update else {
        return Err(CommandError(
            "There is no update to install; check for updates first".into(),
        ));
    };
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| CommandError(format!("Could not install the update: {e}")))?;
    app.restart()
}

#[tauri::command]
pub async fn spans_query(
    state: State<'_, AppState>,
    filter: SpanFilter,
) -> CommandResult<Vec<Span>> {
    Ok(query_spans(&state.db, &filter).await?)
}

#[tauri::command]
pub async fn trace_export_config(state: State<'_, AppState>) -> CommandResult<ExportConfig> {
    Ok(state.trace_exporter.config().await?)
}

#[tauri::command]
pub async fn trace_export_set_config(
    state: State<'_, AppState>,
    config: ExportConfig,
) -> CommandResult<ExportConfig> {
    Ok(state.trace_exporter.set_config(config).await?)
}

#[tauri::command]
pub fn trace_export_status(state: State<'_, AppState>) -> ExportStatus {
    state.trace_exporter.status()
}

/// Sends the waiting spans now instead of at the next interval.
#[tauri::command]
pub async fn trace_export_now(state: State<'_, AppState>) -> CommandResult<ExportStatus> {
    Ok(state.trace_exporter.export_now().await)
}

#[tauri::command]
pub async fn price_list(state: State<'_, AppState>) -> CommandResult<Vec<Price>> {
    Ok(state.prices.list().await?)
}

#[tauri::command]
pub async fn price_set(state: State<'_, AppState>, price: Price) -> CommandResult<Price> {
    Ok(state.prices.set(price).await?)
}

#[tauri::command]
pub async fn price_remove(state: State<'_, AppState>, model: String) -> CommandResult<()> {
    Ok(state.prices.delete(&model).await?)
}

/// Tokens (and, with a price model, the cost of one request) of a server's tool definitions. Pure
/// computation on the given tools; nothing is sent to the server.
#[tauri::command]
pub async fn tools_context_cost(
    state: State<'_, AppState>,
    tools: Vec<ToolInfo>,
    model: Option<String>,
) -> CommandResult<ContextCost> {
    let price = match model {
        Some(model) => state.prices.get(&model).await.ok(),
        None => None,
    };
    Ok(metering::context_cost(&tools, price.as_ref()))
}

/// Tokens and cost of the tool calls of one session.
#[tauri::command]
pub async fn session_usage(
    state: State<'_, AppState>,
    session_id: String,
    model: Option<String>,
) -> CommandResult<SessionUsage> {
    let price = match model {
        Some(model) => state.prices.get(&model).await.ok(),
        None => None,
    };
    Ok(metering::session_usage(&state.db, &session_id, price.as_ref()).await?)
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
    state.secrets.delete(&oauth::credential_key(&id))?;
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

#[tauri::command]
pub async fn messages_query(
    state: State<'_, AppState>,
    filter: MessageFilter,
) -> CommandResult<Vec<MessageRecord>> {
    Ok(query_messages(&state.db, &filter).await?)
}

#[tauri::command]
pub async fn server_connect(
    state: State<'_, AppState>,
    id: String,
    environment_id: Option<String>,
) -> CommandResult<()> {
    state
        .sessions
        .connect(&id, environment_id.as_deref())
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn server_disconnect(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    Ok(state.sessions.disconnect(&id).await?)
}

/// Buffered log lines (stderr and MCP log notifications) of a server.
#[tauri::command]
pub fn server_logs(state: State<'_, AppState>, id: String) -> Vec<LogEvent> {
    state.sessions.logs(&id)
}

#[tauri::command]
pub fn server_details(state: State<'_, AppState>, id: String) -> CommandResult<ServerDetails> {
    Ok(explorer::details(&state.sessions.peer(&id)?)?)
}

#[tauri::command]
pub async fn tools_list(state: State<'_, AppState>, id: String) -> CommandResult<Vec<ToolInfo>> {
    Ok(explorer::list_tools(&state.sessions.peer(&id)?).await?)
}

#[tauri::command]
pub async fn resources_list(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<Vec<ResourceInfo>> {
    Ok(explorer::list_resources(&state.sessions.peer(&id)?).await?)
}

#[tauri::command]
pub async fn resource_templates_list(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<Vec<ResourceTemplateInfo>> {
    Ok(explorer::list_resource_templates(&state.sessions.peer(&id)?).await?)
}

#[tauri::command]
pub async fn prompts_list(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<Vec<PromptInfo>> {
    Ok(explorer::list_prompts(&state.sessions.peer(&id)?).await?)
}

#[tauri::command]
pub async fn tool_call(
    state: State<'_, AppState>,
    request: ToolCallRequest,
) -> CommandResult<ToolCallResult> {
    Ok(state.sessions.call_tool(request).await?)
}

/// Cancels a running tool call; returns whether a call with this id was still running.
#[tauri::command]
pub fn request_cancel(state: State<'_, AppState>, call_id: String) -> bool {
    state.sessions.cancel_call(&call_id)
}

#[tauri::command]
pub async fn resource_read(
    state: State<'_, AppState>,
    id: String,
    uri: String,
) -> CommandResult<JsonValue> {
    Ok(state.sessions.read_resource(&id, &uri).await?)
}

#[tauri::command]
pub async fn prompt_get(
    state: State<'_, AppState>,
    id: String,
    name: String,
    arguments: BTreeMap<String, String>,
) -> CommandResult<JsonValue> {
    Ok(state.sessions.get_prompt(&id, &name, &arguments).await?)
}

#[tauri::command]
pub async fn collections_tree(state: State<'_, AppState>) -> CommandResult<CollectionTree> {
    Ok(state.collections.tree().await?)
}

#[tauri::command]
pub async fn collection_create(
    state: State<'_, AppState>,
    parent_id: Option<String>,
    name: String,
) -> CommandResult<CollectionNode> {
    Ok(state
        .collections
        .create_collection(parent_id.as_deref(), &name)
        .await?)
}

#[tauri::command]
pub async fn collection_rename(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> CommandResult<()> {
    Ok(state.collections.rename_collection(&id, &name).await?)
}

#[tauri::command]
pub async fn collection_move(
    state: State<'_, AppState>,
    id: String,
    parent_id: Option<String>,
) -> CommandResult<()> {
    Ok(state
        .collections
        .move_collection(&id, parent_id.as_deref())
        .await?)
}

#[tauri::command]
pub async fn collection_delete(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    Ok(state.collections.delete_collection(&id).await?)
}

#[tauri::command]
pub async fn request_save(
    state: State<'_, AppState>,
    input: SavedRequestInput,
) -> CommandResult<SavedRequest> {
    Ok(state.collections.save_request(input).await?)
}

#[tauri::command]
pub async fn request_update(
    state: State<'_, AppState>,
    id: String,
    input: SavedRequestInput,
) -> CommandResult<()> {
    Ok(state.collections.update_request(&id, input).await?)
}

#[tauri::command]
pub async fn request_delete(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    Ok(state.collections.delete_request(&id).await?)
}

/// Writes a collection (folder with subfolders and requests) to a JSON file.
#[tauri::command]
pub async fn collection_export(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> CommandResult<()> {
    let json = state.collections.export_collection(&id).await?;
    std::fs::write(&path, json).map_err(|e| CommandError(format!("could not write {path}: {e}")))
}

/// Imports a collection file under `parent_id` (or at the top level).
#[tauri::command]
pub async fn collection_import(
    state: State<'_, AppState>,
    path: String,
    parent_id: Option<String>,
) -> CommandResult<ImportReport> {
    let json = std::fs::read_to_string(&path)
        .map_err(|e| CommandError(format!("could not read {path}: {e}")))?;
    Ok(state
        .collections
        .import_collection(&json, parent_id.as_deref())
        .await?)
}

#[tauri::command]
pub async fn history_list(
    state: State<'_, AppState>,
    filter: HistoryFilter,
) -> CommandResult<Vec<HistoryEntry>> {
    Ok(state.sessions.history().list(&filter).await?)
}

/// Deletes the history of one server, or of all servers. Returns how many entries were removed.
#[tauri::command]
pub async fn history_clear(
    state: State<'_, AppState>,
    server_id: Option<String>,
) -> CommandResult<u64> {
    Ok(state.sessions.history().clear(server_id.as_deref()).await?)
}

#[tauri::command]
pub fn proxy_info(state: State<'_, AppState>) -> ProxyInfo {
    state
        .proxy
        .info(&state.discovery_file, state.http_proxy.port())
}

/// The environment whose variables apply to sessions started through the proxy.
#[tauri::command]
pub fn proxy_set_environment(state: State<'_, AppState>, id: Option<String>) {
    state.proxy.set_environment(id.clone());
    state.http_proxy.set_environment(id);
}

/// Signs in to an OAuth-protected server in the browser. Returns when the tokens are stored.
#[tauri::command]
pub async fn oauth_sign_in(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<()> {
    let opener = crate::sink::BrowserOpener { app };
    Ok(state.sessions.sign_in(&id, &opener).await?)
}

#[tauri::command]
pub async fn oauth_sign_out(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    Ok(state.sessions.sign_out(&id).await?)
}

#[tauri::command]
pub async fn oauth_status(state: State<'_, AppState>, id: String) -> CommandResult<bool> {
    Ok(state.sessions.is_signed_in(&id).await)
}

/// Well-known client configuration files of the current user.
#[tauri::command]
pub fn import_sources(app: AppHandle) -> Vec<ConfigSource> {
    let home = app.path().home_dir().ok();
    let app_data = app.path().data_dir().ok();
    client_import::detect_sources(home.as_deref(), app_data.as_deref())
}

/// Lists the servers defined in a client configuration file.
#[tauri::command]
pub async fn import_preview(
    state: State<'_, AppState>,
    path: String,
) -> CommandResult<Vec<ImportCandidate>> {
    let json = std::fs::read_to_string(&path)
        .map_err(|e| CommandError(format!("could not read {path}: {e}")))?;
    let existing: Vec<ServerInput> = state
        .registry
        .list()
        .await?
        .into_iter()
        .map(|s| s.input)
        .collect();
    Ok(client_import::parse_config(&json, &existing)?)
}

#[tauri::command]
pub async fn import_apply(
    state: State<'_, AppState>,
    servers: Vec<ServerInput>,
) -> CommandResult<ImportSummary> {
    Ok(client_import::import_servers(&state.registry, state.secrets.as_ref(), servers).await)
}
