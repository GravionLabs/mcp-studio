use mcp_studio_core::{
    client_import::{self, ConfigSource, ImportCandidate, ImportSummary},
    collections::{CollectionNode, CollectionTree, ImportReport, SavedRequest, SavedRequestInput},
    docs_gen::{self, DocsInput},
    environments::{Environment, EnvironmentInput},
    events::{LogEvent, MessageRecord},
    explorer::{self, PromptInfo, ResourceInfo, ResourceTemplateInfo, ServerDetails, ToolInfo},
    flow::{self, Flow, FlowValidation, ToolCatalog},
    flow_replay::ReplayTools,
    flow_run::{self, Decision, FlowEngine, RunRequest, ToolPolicy},
    flow_runs::{FlowRun, RunSummary},
    flow_yaml,
    flows::FlowRecord,
    history::{HistoryEntry, HistoryFilter},
    lint::{self, LintReport},
    llm::{
        CompletionRequest, Message, ProviderSettings, ProviderStatus, ProviderTestResult,
        OPENAI_KEY_NAME,
    },
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
    tokens::{self, CountingStatus},
    trace::{query_spans, Span, SpanFilter},
    update::UpdateInfo,
};
use std::collections::BTreeMap;
use std::sync::Arc;

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

async fn counting_model(state: &AppState) -> CommandResult<String> {
    Ok(state
        .settings
        .get(tokens::COUNTING_MODEL_SETTING)
        .await?
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| mcp_studio_llm::DEFAULT_MODEL.to_owned()))
}

#[tauri::command]
pub async fn token_counting_status(state: State<'_, AppState>) -> CommandResult<CountingStatus> {
    Ok(CountingStatus {
        model: counting_model(&state).await?,
        has_key: state.secrets.get(tokens::ANTHROPIC_KEY_NAME)?.is_some(),
    })
}

#[tauri::command]
pub async fn token_counting_set_model(
    state: State<'_, AppState>,
    model: String,
) -> CommandResult<()> {
    Ok(state
        .settings
        .set(tokens::COUNTING_MODEL_SETTING, model.trim())
        .await?)
}

/// Which providers are set up: their addresses and whether API keys are stored.
#[tauri::command]
pub async fn provider_status(state: State<'_, AppState>) -> CommandResult<ProviderStatus> {
    Ok(ProviderStatus {
        settings: mcp_studio_llm::load_settings(&state.settings).await?,
        anthropic_key: state.secrets.get(tokens::ANTHROPIC_KEY_NAME)?.is_some(),
        openai_key: state.secrets.get(OPENAI_KEY_NAME)?.is_some(),
    })
}

#[tauri::command]
pub async fn provider_set_settings(
    state: State<'_, AppState>,
    settings: ProviderSettings,
) -> CommandResult<ProviderSettings> {
    Ok(mcp_studio_llm::save_settings(&state.settings, settings).await?)
}

/// Stores the API key of a provider (`anthropic` or `openai`) in the OS keyring, or removes it
/// when `key` is empty or missing.
#[tauri::command]
pub fn provider_set_key(
    state: State<'_, AppState>,
    provider: String,
    key: Option<String>,
) -> CommandResult<()> {
    let name = match provider.as_str() {
        "anthropic" => tokens::ANTHROPIC_KEY_NAME,
        "openai" => OPENAI_KEY_NAME,
        other => return Err(CommandError(format!("{other} does not use an API key"))),
    };
    match key.map(|k| k.trim().to_owned()).filter(|k| !k.is_empty()) {
        Some(key) => state.secrets.set(name, &key)?,
        None => state.secrets.delete(name)?,
    }
    Ok(())
}

/// The models installed in the configured Ollama.
#[tauri::command]
pub async fn ollama_models(state: State<'_, AppState>) -> CommandResult<Vec<String>> {
    let settings = mcp_studio_llm::load_settings(&state.settings).await?;
    mcp_studio_llm::ollama_models(&settings.ollama_url)
        .await
        .map_err(|e| CommandError(e.message))
}

/// Sends a tiny request to the provider of `model` (for example `ollama:llama3.1:8b`) to check
/// that the address, the key, and the model work.
#[tauri::command]
pub async fn provider_test(
    state: State<'_, AppState>,
    model: String,
) -> CommandResult<ProviderTestResult> {
    let settings = mcp_studio_llm::load_settings(&state.settings).await?;
    let resolved = mcp_studio_llm::resolve(&settings, state.secrets.as_ref(), &model)
        .map_err(|e| CommandError(e.message))?;
    let mut request = CompletionRequest::new(
        resolved.model,
        vec![Message::user("Reply with the single word OK.")],
    );
    request.max_tokens = 16;
    request.temperature = Some(0.0);
    let completion = resolved
        .provider
        .complete(&request)
        .await
        .map_err(|e| CommandError(e.message))?;
    Ok(ProviderTestResult {
        model: completion.model.clone(),
        reply: completion.text(),
        usage: completion.usage,
    })
}

/// Asks Anthropic for the exact token count of a stored message and saves it. Sends the message's
/// (secret-masked) content to Anthropic, so it only runs when the user asks for it.
#[tauri::command]
pub async fn message_count_exact(
    state: State<'_, AppState>,
    message_id: u32,
) -> CommandResult<u32> {
    let Some(key) = state.secrets.get(tokens::ANTHROPIC_KEY_NAME)? else {
        return Err(CommandError(
            "Add an Anthropic API key on the Providers page to count tokens exactly".into(),
        ));
    };
    let counter = mcp_studio_llm::AnthropicCounter::new(key, counting_model(&state).await?);
    Ok(tokens::count_message_exact(&state.db, &counter, i64::from(message_id)).await?)
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
pub async fn flow_list(state: State<'_, AppState>) -> CommandResult<Vec<FlowRecord>> {
    Ok(state.flows.list().await?)
}

#[tauri::command]
pub async fn flow_get(state: State<'_, AppState>, id: String) -> CommandResult<FlowRecord> {
    Ok(state.flows.get(&id).await?)
}

/// Creates a flow (`id` is null) or replaces an existing one.
#[tauri::command]
pub async fn flow_save(
    state: State<'_, AppState>,
    id: Option<String>,
    flow: Flow,
) -> CommandResult<FlowRecord> {
    Ok(state.flows.save(id.as_deref(), flow).await?)
}

#[tauri::command]
pub async fn flow_delete(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    Ok(state.flows.delete(&id).await?)
}

/// Writes a flow as a YAML file.
#[tauri::command]
pub async fn flow_export(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> CommandResult<()> {
    let yaml = state.flows.export_yaml(&id).await?;
    std::fs::write(&path, yaml).map_err(|e| CommandError(format!("could not write {path}: {e}")))
}

/// Adds the flow in a YAML file to the library.
#[tauri::command]
pub async fn flow_import(state: State<'_, AppState>, path: String) -> CommandResult<FlowRecord> {
    let yaml = std::fs::read_to_string(&path)
        .map_err(|e| CommandError(format!("could not read {path}: {e}")))?;
    Ok(state.flows.import_yaml(&yaml).await?)
}

/// Starts a run in the background and returns at once; progress arrives as `flow://event` events
/// and tool calls wait for the user in `flow://confirm`. The caller chooses `run_id` so it can
/// listen before the run begins. Pass `flow` to run an unsaved flow; otherwise `flow_id` is run.
#[tauri::command]
pub async fn flow_run_start(
    app: AppHandle,
    state: State<'_, AppState>,
    run_id: String,
    flow_id: Option<String>,
    flow: Option<Flow>,
    inputs: JsonValue,
    environment_id: Option<String>,
) -> CommandResult<()> {
    let (flow_id, flow) = match (flow, flow_id) {
        (Some(flow), id) => (id, flow),
        (None, Some(id)) => {
            let record = state.flows.get(&id).await?;
            (Some(id), record.flow)
        }
        (None, None) => return Err(CommandError("choose a flow to run".into())),
    };
    let inputs = match inputs.0 {
        serde_json::Value::Object(map) => map,
        serde_json::Value::Null => serde_json::Map::new(),
        _ => return Err(CommandError("the inputs must be an object".into())),
    };
    if state.running_flows.lock().unwrap().contains_key(&run_id) {
        return Err(CommandError("this run is already going".into()));
    }
    let tools = Arc::new(crate::flow_runtime::SessionTools {
        sessions: state.sessions.clone(),
        registry: state.registry.clone(),
        environment_id,
    });
    spawn_run(
        &app,
        &state,
        tools,
        RunRequest {
            run_id,
            flow_id,
            flow,
            inputs,
            replay_of: None,
        },
    )
    .await
}

/// Starts a run in the background with the given way of calling tools.
async fn spawn_run(
    app: &AppHandle,
    state: &State<'_, AppState>,
    tools: Arc<dyn flow_run::ToolRunner>,
    request: RunRequest,
) -> CommandResult<()> {
    if state
        .running_flows
        .lock()
        .unwrap()
        .contains_key(&request.run_id)
    {
        return Err(CommandError("this run is already going".into()));
    }
    let settings = mcp_studio_llm::load_settings(&state.settings).await?;
    let engine = FlowEngine::new(
        state.db.clone(),
        tools,
        Arc::new(crate::flow_runtime::ConfiguredModels {
            settings,
            secrets: state.secrets.clone(),
        }),
        Arc::new(crate::flow_runtime::AskInWebview {
            app: app.clone(),
            confirmations: state.confirmations.clone(),
        }),
        Arc::new(crate::flow_runtime::EmitProgress { app: app.clone() }),
    );
    let cancel = tokio_util::sync::CancellationToken::new();
    let run_id = request.run_id.clone();
    state
        .running_flows
        .lock()
        .unwrap()
        .insert(run_id.clone(), cancel.clone());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = engine.run(request, cancel).await;
        if let Some(state) = app.try_state::<AppState>() {
            state.running_flows.lock().unwrap().remove(&run_id);
        }
    });
    Ok(())
}

/// Replays a run: the flow runs again, but tool calls are answered from what the original run
/// recorded, so no tool is called and nothing needs confirmation. Model calls run live. With
/// `use_current_flow` the flow in the library is replayed instead of the one the run started with.
#[tauri::command]
pub async fn flow_run_replay(
    app: AppHandle,
    state: State<'_, AppState>,
    run_id: String,
    source_run_id: String,
    use_current_flow: bool,
    environment_id: Option<String>,
) -> CommandResult<()> {
    let source = state.flow_runs.get(&source_run_id).await?;
    let flow = if use_current_flow {
        match &source.flow_id {
            Some(id) => state.flows.get(id).await?.flow,
            None => {
                return Err(CommandError(
                    "this run has no flow in the library to replay with".into(),
                ))
            }
        }
    } else {
        source.flow.clone()
    };
    // Servers that are still around describe their tools; otherwise the record does.
    let live = Arc::new(crate::flow_runtime::SessionTools {
        sessions: state.sessions.clone(),
        registry: state.registry.clone(),
        environment_id,
    });
    let tools = Arc::new(ReplayTools::new(&source.calls, Some(live)));
    let inputs = source.inputs.0.as_object().cloned().unwrap_or_default();
    spawn_run(
        &app,
        &state,
        tools,
        RunRequest {
            run_id,
            flow_id: source.flow_id.clone(),
            flow,
            inputs,
            replay_of: Some(source.id),
        },
    )
    .await
}

/// Cancels a run in progress; returns whether it was still running.
#[tauri::command]
pub fn flow_run_cancel(state: State<'_, AppState>, run_id: String) -> bool {
    match state.running_flows.lock().unwrap().get(&run_id) {
        Some(token) => {
            token.cancel();
            true
        }
        None => false,
    }
}

#[tauri::command]
pub async fn flow_run_get(state: State<'_, AppState>, run_id: String) -> CommandResult<FlowRun> {
    Ok(state.flow_runs.get(&run_id).await?)
}

/// The newest runs, optionally of one flow.
#[tauri::command]
pub async fn flow_run_list(
    state: State<'_, AppState>,
    flow_id: Option<String>,
    limit: Option<u32>,
) -> CommandResult<Vec<RunSummary>> {
    Ok(state
        .flow_runs
        .list(flow_id.as_deref(), limit.unwrap_or(50))
        .await?)
}

#[tauri::command]
pub async fn flow_run_delete(state: State<'_, AppState>, run_id: String) -> CommandResult<()> {
    Ok(state.flow_runs.delete(&run_id).await?)
}

/// Answers a question of a run (`flow://confirm`). Returns whether it was still waiting.
#[tauri::command]
pub fn flow_confirm(state: State<'_, AppState>, id: String, decision: Decision) -> bool {
    state.confirmations.answer(&id, decision)
}

/// The tools and servers that may run without asking.
#[tauri::command]
pub async fn tool_policy_get(state: State<'_, AppState>) -> CommandResult<ToolPolicy> {
    Ok(flow_run::load_policy(&state.settings).await?)
}

#[tauri::command]
pub async fn tool_policy_set(
    state: State<'_, AppState>,
    policy: ToolPolicy,
) -> CommandResult<ToolPolicy> {
    Ok(flow_run::save_policy(&state.settings, policy).await?)
}

/// Checks a flow (it does not have to be saved) against the servers that are connected right now.
/// Servers that are not connected are not connected for this: they are reported as unchecked.
#[tauri::command]
pub async fn flow_validate(
    state: State<'_, AppState>,
    flow: Flow,
) -> CommandResult<FlowValidation> {
    let servers = state.registry.list().await?;
    let mut catalog = ToolCatalog::new();
    for name in flow::referenced_servers(&flow) {
        let Some(server) = servers.iter().find(|s| s.input.name == name) else {
            // A server that does not exist is reported by validation itself.
            catalog.insert(name, Vec::new());
            continue;
        };
        if let Ok(peer) = state.sessions.peer(&server.id) {
            if let Ok(tools) = explorer::list_tools(&peer).await {
                catalog.insert(name, tools);
            }
        }
    }
    Ok(flow::validate_available(&flow, &catalog))
}

/// The YAML text of a flow, for the editor's YAML view.
#[tauri::command]
pub fn flow_to_yaml(flow: Flow) -> CommandResult<String> {
    Ok(flow_yaml::to_yaml(&flow)?)
}

/// Parses YAML text into a flow, for the editor's YAML view.
#[tauri::command]
pub fn flow_from_yaml(yaml: String) -> CommandResult<Flow> {
    Ok(flow_yaml::from_yaml(&yaml)?)
}

/// Markdown documentation of a server's tools: purpose and parameters from the definitions you pass,
/// examples and error cases from the recorded history of that server. Nothing is sent anywhere.
async fn render_server_docs(
    state: &AppState,
    server_id: &str,
    tools: &[ToolInfo],
) -> CommandResult<String> {
    let server = state.registry.get(server_id).await?;
    let history = state
        .sessions
        .history()
        .list(&HistoryFilter {
            server_id: Some(server_id.to_owned()),
            limit: Some(1000),
            ..HistoryFilter::default()
        })
        .await?;
    let generated_on = docs_gen::civil_date(mcp_studio_core::db::now_ms());
    Ok(docs_gen::render_docs(&DocsInput {
        server_name: &server.input.name,
        generated_on: &generated_on,
        tools,
        history: &history,
    }))
}

#[tauri::command]
pub async fn server_docs(
    state: State<'_, AppState>,
    server_id: String,
    tools: Vec<ToolInfo>,
) -> CommandResult<String> {
    render_server_docs(&state, &server_id, &tools).await
}

/// Writes the documentation of a server's tools to a Markdown file.
#[tauri::command]
pub async fn server_docs_export(
    state: State<'_, AppState>,
    server_id: String,
    tools: Vec<ToolInfo>,
    path: String,
) -> CommandResult<()> {
    let markdown = render_server_docs(&state, &server_id, &tools).await?;
    std::fs::write(&path, markdown)
        .map_err(|e| CommandError(format!("could not write {path}: {e}")))
}

/// Checks tool definitions for vague descriptions, missing `required` fields, overlapping tools and
/// oversized definitions. Pure computation on the given tools; nothing is sent anywhere.
#[tauri::command]
pub fn tools_lint(tools: Vec<ToolInfo>) -> LintReport {
    lint::lint_tools(&tools)
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
