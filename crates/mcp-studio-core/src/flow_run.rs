//! Runs a flow.
//!
//! Steps run in order. A condition may jump forward to the step named by `then` or `else`; the
//! steps it jumps over are recorded as skipped. Every step can use `inputs.*` and the output of
//! earlier steps as `steps.<id>.*` (see [`crate::flow_expr`]):
//!
//! | step        | output                                                                    |
//! | ----------- | ------------------------------------------------------------------------- |
//! | `input`     | the input values                                                          |
//! | `tool`      | `result` (text), `content`, `structured`, `isError`                       |
//! | `llm`       | `text`, `model`, `usage`, `calls` (tool calls the model made)             |
//! | `condition` | `value`                                                                   |
//! | `transform` | one entry per `values` key                                                |
//! | `output`    | one entry per `outputs` key; these also form the result of the run        |
//!
//! **Nothing runs without consent.** Before every tool call, whether a `tool` step makes it or a
//! model asks for it, the user is asked, unless the tool or its server was allowed before. A denied
//! call never runs: a `tool` step fails, and a model is told that the user said no.
//!
//! Every run is a trace: a `flow` span with one child span per step, and `tool` spans for the tool
//! calls under their step. Everything a run did is stored (see [`crate::flow_runs`]).

use std::{collections::BTreeMap, collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use specta::Type;
use tokio_util::sync::CancellationToken;

use crate::{
    db::{new_id, now_ms, Db, DbError, DbResult},
    explorer::ToolInfo,
    flow::{validate, Flow, FlowIssueCode, InputDecl, Step, StepKind, ToolCatalog},
    flow_expr::{evaluate_condition, render_text, render_value},
    flow_runs::{FlowRun, FlowRuns, RecordedCall, RunStatus, StepStatus},
    llm::{
        Completion, CompletionRequest, ContentBlock, LlmError, LlmProvider, Message, Role,
        StopReason, StreamEvent, ToolDefinition, Usage,
    },
    model::JsonValue,
    settings::Settings,
    trace::{self, NewSpan, SpanKind, SpanStatus},
};

/// The values a run gets for the declared inputs.
pub type RunInputs = Map<String, Value>;

/// How many times a model may call tools in one step before the step gives up.
pub const MAX_AGENT_TURNS: usize = 10;

const POLICY_KEY: &str = "flow_tool_policy";

// ---------------------------------------------------------------------------------------------
// What the engine needs from the outside

/// The result of a tool call that ran. `is_error` is the tool's own error flag (`isError`).
#[derive(Debug, Clone)]
pub struct ToolOutcome {
    /// The MCP `CallToolResult`.
    pub result: Value,
    pub is_error: bool,
}

/// Calls tools of MCP servers by server name.
#[async_trait]
pub trait ToolRunner: Send + Sync {
    /// The tools of a server. Fails when the server is unknown or cannot be reached.
    async fn list_tools(&self, server: &str) -> Result<Vec<ToolInfo>, String>;

    /// Calls a tool. `Err` means the call could not be made (protocol error, connection lost); a
    /// tool that ran and failed is `Ok` with `is_error`.
    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
        cancel: &CancellationToken,
    ) -> Result<ToolOutcome, String>;

    /// Whether calls reach real servers. A replay answers from recorded results, so nothing needs
    /// the user's consent.
    fn is_live(&self) -> bool {
        true
    }
}

/// Finds the provider for the `model` of an LLM step.
pub trait ModelResolver: Send + Sync {
    fn resolve(&self, model: &str) -> Result<(Arc<dyn LlmProvider>, String), LlmError>;
}

/// A tool call that waits for the user's decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmRequest {
    pub run_id: String,
    pub step_id: String,
    pub server: String,
    pub tool: String,
    pub arguments: JsonValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// This call only.
    Allow,
    /// This tool from now on, without asking.
    AllowTool,
    /// Every tool of this server from now on.
    AllowServer,
    Deny,
}

/// Asks the user whether a tool call may run.
#[async_trait]
pub trait Confirmer: Send + Sync {
    async fn confirm(&self, request: &ConfirmRequest) -> Decision;
}

/// A question for the user, with the id the answer must carry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmEvent {
    pub id: String,
    pub request: ConfirmRequest,
}

/// Progress of a run, for the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    #[serde(rename_all = "camelCase")]
    RunStarted { run_id: String, flow_name: String },
    #[serde(rename_all = "camelCase")]
    StepStarted {
        run_id: String,
        step_id: String,
        kind: String,
    },
    #[serde(rename_all = "camelCase")]
    StepFinished {
        run_id: String,
        step_id: String,
        status: StepStatus,
        error: Option<String>,
    },
    /// A piece of the answer of an LLM step, as the model writes it.
    #[serde(rename_all = "camelCase")]
    TextDelta {
        run_id: String,
        step_id: String,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    RunFinished {
        run_id: String,
        status: RunStatus,
        error: Option<String>,
    },
}

pub trait RunObserver: Send + Sync {
    fn event(&self, event: RunEvent);
}

// ---------------------------------------------------------------------------------------------
// Which tool calls need consent

/// The tools and servers that were allowed to run without asking. Entries are `server` or
/// `server/tool`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolPolicy {
    pub allow: Vec<String>,
}

impl ToolPolicy {
    pub fn allows(&self, server: &str, tool: &str) -> bool {
        let pair = format!("{server}/{tool}");
        self.allow
            .iter()
            .any(|entry| entry == server || *entry == pair)
    }

    pub fn allow_tool(&mut self, server: &str, tool: &str) {
        self.add(format!("{server}/{tool}"));
    }

    pub fn allow_server(&mut self, server: &str) {
        self.add(server.to_owned());
    }

    fn add(&mut self, entry: String) {
        if !self.allow.contains(&entry) {
            self.allow.push(entry);
            self.allow.sort();
        }
    }

    /// Removes blank and repeated entries.
    pub fn normalized(self) -> Self {
        let mut allow: Vec<String> = self
            .allow
            .into_iter()
            .map(|e| e.trim().to_owned())
            .filter(|e| !e.is_empty())
            .collect();
        allow.sort();
        allow.dedup();
        Self { allow }
    }
}

pub async fn load_policy(settings: &Settings) -> DbResult<ToolPolicy> {
    Ok(match settings.get(POLICY_KEY).await? {
        Some(text) => serde_json::from_str(&text).unwrap_or_default(),
        None => ToolPolicy::default(),
    })
}

pub async fn save_policy(settings: &Settings, policy: ToolPolicy) -> DbResult<ToolPolicy> {
    let policy = policy.normalized();
    let text = serde_json::to_string(&policy)
        .map_err(|e| DbError::Invalid(format!("could not save the policy: {e}")))?;
    settings.set(POLICY_KEY, &text).await?;
    Ok(policy)
}

// ---------------------------------------------------------------------------------------------
// The engine

/// What to run.
pub struct RunRequest {
    pub run_id: String,
    /// The library entry the run belongs to, if any.
    pub flow_id: Option<String>,
    pub flow: Flow,
    pub inputs: RunInputs,
    /// The run whose tool results are replayed.
    pub replay_of: Option<String>,
}

enum Stop {
    Failed(String),
    Cancelled,
}

impl From<DbError> for Stop {
    fn from(error: DbError) -> Self {
        Stop::Failed(error.to_string())
    }
}

type StepOutput = (Value, Option<usize>);

struct Ctx<'a> {
    run_id: &'a str,
    inputs: &'a RunInputs,
    steps: Map<String, Value>,
    outputs: Map<String, Value>,
    catalog: ToolCatalog,
    policy: ToolPolicy,
    cancel: &'a CancellationToken,
}

impl Ctx<'_> {
    /// The object expressions are evaluated against.
    fn root(&self) -> Value {
        json!({ "inputs": self.inputs, "steps": self.steps })
    }
}

enum ToolCall {
    Ran(ToolOutcome),
    Denied,
    Failed(String),
}

pub struct FlowEngine {
    db: Db,
    runs: FlowRuns,
    settings: Settings,
    tools: Arc<dyn ToolRunner>,
    models: Arc<dyn ModelResolver>,
    confirmer: Arc<dyn Confirmer>,
    observer: Arc<dyn RunObserver>,
}

fn span_kind(step: &Step) -> SpanKind {
    match step.kind {
        StepKind::Tool { .. } => SpanKind::Tool,
        StepKind::Llm { .. } => SpanKind::Llm,
        _ => SpanKind::Step,
    }
}

fn kind_name(step: &Step) -> &'static str {
    match step.kind {
        StepKind::Input { .. } => "input",
        StepKind::Llm { .. } => "llm",
        StepKind::Tool { .. } => "tool",
        StepKind::Condition { .. } => "condition",
        StepKind::Transform { .. } => "transform",
        StepKind::Output { .. } => "output",
    }
}

/// The text of an MCP tool result: its text blocks, or its structured content as JSON.
pub fn result_text(result: &Value) -> String {
    let text: Vec<&str> = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect();
    if !text.is_empty() {
        return text.join("\n");
    }
    match &result["structuredContent"] {
        Value::Null => String::new(),
        structured => structured.to_string(),
    }
}

/// A name for a tool that every model API accepts (letters, digits, `_` and `-`, at most 64).
fn model_tool_name(server: &str, tool: &str, taken: &HashMap<String, (String, String)>) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    let mut name: String = format!("{}__{}", clean(server), clean(tool))
        .chars()
        .take(64)
        .collect();
    let base = name.clone();
    let mut n = 2;
    while taken.contains_key(&name) {
        let suffix = format!("_{n}");
        name = format!(
            "{}{suffix}",
            base.chars().take(64 - suffix.len()).collect::<String>()
        );
        n += 1;
    }
    name
}

fn type_matches(value: &Value, kind: &str) -> bool {
    match kind {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => true,
    }
}

impl FlowEngine {
    pub fn new(
        db: Db,
        tools: Arc<dyn ToolRunner>,
        models: Arc<dyn ModelResolver>,
        confirmer: Arc<dyn Confirmer>,
        observer: Arc<dyn RunObserver>,
    ) -> Self {
        Self {
            runs: FlowRuns::new(db.clone()),
            settings: Settings::new(db.clone()),
            db,
            tools,
            models,
            confirmer,
            observer,
        }
    }

    /// Runs a flow to the end and returns the stored run. Failures of the flow are part of the
    /// result (`status`, `error`); `Err` is only returned when the run could not be stored.
    pub async fn run(&self, request: RunRequest, cancel: CancellationToken) -> DbResult<FlowRun> {
        let started = now_ms();
        let id = request.run_id.clone();
        self.runs
            .create(
                &id,
                request.flow_id.as_deref(),
                &request.flow,
                &Value::Object(request.inputs.clone()),
                request.replay_of.as_deref(),
                started,
            )
            .await?;
        let root_span = new_id();
        let mut root_attributes = json!({
            "runId": id,
            "flowId": request.flow_id,
            "replayOf": request.replay_of,
        });
        trace::insert_span(
            &self.db,
            &NewSpan {
                id: &root_span,
                trace_id: &id,
                parent_id: None,
                kind: SpanKind::Flow,
                name: &request.flow.name,
                started_at: started,
                attributes: &root_attributes,
            },
        )
        .await?;
        self.observer.event(RunEvent::RunStarted {
            run_id: id.clone(),
            flow_name: request.flow.name.clone(),
        });

        let outcome = self.execute(&request, &root_span, &cancel).await;
        let ended = now_ms();
        let (status, outputs, error) = match outcome {
            Ok(outputs) => (RunStatus::Succeeded, Some(outputs), None),
            Err(Stop::Cancelled) => (
                RunStatus::Cancelled,
                None,
                Some("the run was cancelled".to_owned()),
            ),
            Err(Stop::Failed(message)) => (RunStatus::Failed, None, Some(message)),
        };
        self.runs
            .finish(&id, status, ended, outputs.as_ref(), error.as_deref())
            .await?;
        root_attributes["status"] = json!(status.as_str());
        trace::end_span(
            &self.db,
            &root_span,
            ended,
            match status {
                RunStatus::Succeeded => SpanStatus::Ok,
                RunStatus::Cancelled => SpanStatus::Cancelled,
                _ => SpanStatus::Error,
            },
            &root_attributes,
        )
        .await?;
        self.observer.event(RunEvent::RunFinished {
            run_id: id.clone(),
            status,
            error,
        });
        self.runs.get(&id).await
    }

    async fn execute(
        &self,
        request: &RunRequest,
        root_span: &str,
        cancel: &CancellationToken,
    ) -> Result<Value, Stop> {
        let flow = &request.flow;
        // Look up the tools of every server the flow uses, then check the flow against them.
        let replaying = request.replay_of.is_some();
        let mut catalog = ToolCatalog::new();
        for server in servers_of(flow) {
            match self.tools.list_tools(&server).await {
                Ok(tools) => {
                    catalog.insert(server, tools);
                }
                // A replay answers from the record, so a server that is gone does not matter.
                Err(_) if replaying => {
                    catalog.insert(server, Vec::new());
                }
                Err(e) => {
                    return Err(Stop::Failed(format!(
                        "server {server:?} cannot be used: {e}"
                    )))
                }
            }
        }
        let issues: Vec<_> = validate(flow, &catalog)
            .into_iter()
            .filter(|issue| !(replaying && is_tool_issue(issue.code)))
            .collect();
        if !issues.is_empty() {
            let list: Vec<String> = issues
                .iter()
                .map(|i| match &i.step_id {
                    Some(step) => format!("step `{step}`: {}", i.message),
                    None => i.message.clone(),
                })
                .collect();
            return Err(Stop::Failed(format!(
                "the flow is not valid: {}",
                list.join("; ")
            )));
        }

        let mut ctx = Ctx {
            run_id: &request.run_id,
            inputs: &request.inputs,
            steps: Map::new(),
            outputs: Map::new(),
            catalog,
            policy: load_policy(&self.settings).await?,
            cancel,
        };
        let mut index = 0;
        while index < flow.steps.len() {
            if cancel.is_cancelled() {
                return Err(Stop::Cancelled);
            }
            let step = &flow.steps[index];
            let (output, jump) = self.run_step(&mut ctx, flow, step, root_span).await?;
            ctx.steps.insert(step.id.clone(), output);
            match jump {
                Some(target) if target > index + 1 => {
                    for skipped in &flow.steps[index + 1..target] {
                        self.runs
                            .step_skipped(ctx.run_id, &skipped.id, kind_name(skipped), now_ms())
                            .await?;
                        self.observer.event(RunEvent::StepFinished {
                            run_id: ctx.run_id.to_owned(),
                            step_id: skipped.id.clone(),
                            status: StepStatus::Skipped,
                            error: None,
                        });
                    }
                    index = target;
                }
                _ => index += 1,
            }
        }
        Ok(Value::Object(ctx.outputs))
    }

    /// Runs one step and records it, its span, and its events.
    async fn run_step(
        &self,
        ctx: &mut Ctx<'_>,
        flow: &Flow,
        step: &Step,
        root_span: &str,
    ) -> Result<StepOutput, Stop> {
        let kind = kind_name(step);
        let span_id = new_id();
        let started = now_ms();
        let mut attributes = json!({ "stepId": step.id, "stepType": kind });
        trace::insert_span(
            &self.db,
            &NewSpan {
                id: &span_id,
                trace_id: ctx.run_id,
                parent_id: Some(root_span),
                kind: span_kind(step),
                name: &step.id,
                started_at: started,
                attributes: &attributes,
            },
        )
        .await?;
        let seq = self
            .runs
            .step_started(ctx.run_id, &step.id, kind, started, Some(&span_id))
            .await?;
        self.observer.event(RunEvent::StepStarted {
            run_id: ctx.run_id.to_owned(),
            step_id: step.id.clone(),
            kind: kind.to_owned(),
        });

        let result = self
            .exec(ctx, flow, step, seq, &span_id, &mut attributes)
            .await;

        let ended = now_ms();
        let (status, span_status, output, error) = match &result {
            Ok((output, _)) => (StepStatus::Succeeded, SpanStatus::Ok, Some(output), None),
            Err(Stop::Cancelled) => (StepStatus::Cancelled, SpanStatus::Cancelled, None, None),
            Err(Stop::Failed(message)) => (
                StepStatus::Failed,
                SpanStatus::Error,
                None,
                Some(message.as_str()),
            ),
        };
        self.runs
            .step_finished(ctx.run_id, seq, status, ended, output, error)
            .await?;
        trace::end_span(&self.db, &span_id, ended, span_status, &attributes).await?;
        self.observer.event(RunEvent::StepFinished {
            run_id: ctx.run_id.to_owned(),
            step_id: step.id.clone(),
            status,
            error: error.map(str::to_owned),
        });
        result.map_err(|stop| match stop {
            Stop::Failed(message) => Stop::Failed(format!("step `{}` failed: {message}", step.id)),
            other => other,
        })
    }

    async fn resolved(&self, ctx: &Ctx<'_>, seq: i64, value: &Value) -> Result<(), Stop> {
        Ok(self.runs.step_resolved(ctx.run_id, seq, value).await?)
    }

    async fn exec(
        &self,
        ctx: &mut Ctx<'_>,
        flow: &Flow,
        step: &Step,
        seq: i64,
        span_id: &str,
        attributes: &mut Value,
    ) -> Result<StepOutput, Stop> {
        let fail = |e: DbError| Stop::Failed(e.to_string());
        match &step.kind {
            StepKind::Input { inputs } => {
                check_inputs(inputs, ctx.inputs)?;
                Ok((Value::Object(ctx.inputs.clone()), None))
            }
            StepKind::Transform { values } => {
                let root = ctx.root();
                let mut output = Map::new();
                for (key, template) in values {
                    output.insert(
                        key.clone(),
                        render_value(&Value::String(template.clone()), &root).map_err(fail)?,
                    );
                }
                self.resolved(ctx, seq, &json!({ "values": values }))
                    .await?;
                Ok((Value::Object(output), None))
            }
            StepKind::Output { outputs } => {
                let root = ctx.root();
                let mut output = Map::new();
                for (key, template) in outputs {
                    output.insert(
                        key.clone(),
                        render_value(&Value::String(template.clone()), &root).map_err(fail)?,
                    );
                }
                for (key, value) in &output {
                    ctx.outputs.insert(key.clone(), value.clone());
                }
                Ok((Value::Object(output), None))
            }
            StepKind::Condition {
                expression,
                then,
                otherwise,
            } => {
                let value = evaluate_condition(expression, &ctx.root()).map_err(fail)?;
                self.resolved(ctx, seq, &json!({ "expression": expression }))
                    .await?;
                let target = if value { then } else { otherwise };
                let jump = target
                    .as_deref()
                    .and_then(|id| flow.steps.iter().position(|s| s.id == id));
                Ok((json!({ "value": value }), jump))
            }
            StepKind::Tool {
                server,
                tool,
                arguments,
            } => {
                let raw = Value::Object(
                    arguments
                        .iter()
                        .map(|(k, v)| (k.clone(), v.0.clone()))
                        .collect(),
                );
                let rendered = render_value(&raw, &ctx.root()).map_err(fail)?;
                self.resolved(
                    ctx,
                    seq,
                    &json!({ "server": server, "tool": tool, "arguments": rendered }),
                )
                .await?;
                attributes["server"] = json!(server);
                attributes["tool"] = json!(tool);
                match self
                    .invoke_tool(ctx, &step.id, span_id, server, tool, rendered)
                    .await?
                {
                    ToolCall::Denied => Err(Stop::Failed("the tool call was not allowed".into())),
                    ToolCall::Failed(message) => Err(Stop::Failed(message)),
                    ToolCall::Ran(outcome) if outcome.is_error => {
                        let text = result_text(&outcome.result);
                        Err(Stop::Failed(if text.is_empty() {
                            "the tool reported an error".into()
                        } else {
                            text
                        }))
                    }
                    ToolCall::Ran(outcome) => Ok((
                        json!({
                            "result": result_text(&outcome.result),
                            "content": outcome.result["content"],
                            "structured": outcome.result["structuredContent"],
                            "isError": false,
                        }),
                        None,
                    )),
                }
            }
            StepKind::Llm {
                model,
                prompt,
                system,
                tools,
            } => {
                let root = ctx.root();
                let prompt = render_text(prompt, &root).map_err(fail)?;
                let system = system
                    .as_ref()
                    .map(|s| render_text(s, &root))
                    .transpose()
                    .map_err(fail)?;
                self.resolved(
                    ctx,
                    seq,
                    &json!({
                        "model": model,
                        "system": system,
                        "prompt": prompt,
                        "tools": tools.iter().map(|t| format!("{}/{}", t.server, t.tool)).collect::<Vec<_>>(),
                    }),
                )
                .await?;
                attributes["model"] = json!(model);
                self.exec_llm(ctx, step, span_id, attributes, model, system, prompt, tools)
                    .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn exec_llm(
        &self,
        ctx: &mut Ctx<'_>,
        step: &Step,
        span_id: &str,
        attributes: &mut Value,
        model: &str,
        system: Option<String>,
        prompt: String,
        tool_refs: &[crate::flow::ToolRef],
    ) -> Result<StepOutput, Stop> {
        let (provider, model_name) = self
            .models
            .resolve(model)
            .map_err(|e| Stop::Failed(e.message))?;

        // The tools the model may call, under names every provider accepts.
        let mut names: HashMap<String, (String, String)> = HashMap::new();
        let mut definitions = Vec::new();
        for reference in tool_refs {
            let info = ctx
                .catalog
                .get(&reference.server)
                .and_then(|tools| tools.iter().find(|t| t.name == reference.tool))
                .ok_or_else(|| {
                    Stop::Failed(format!(
                        "tool {}/{} is not available",
                        reference.server, reference.tool
                    ))
                })?;
            let name = model_tool_name(&reference.server, &reference.tool, &names);
            names.insert(
                name.clone(),
                (reference.server.clone(), reference.tool.clone()),
            );
            definitions.push(ToolDefinition {
                name,
                description: info.description.clone().unwrap_or_default(),
                input_schema: info.input_schema.clone(),
            });
        }

        let mut request = CompletionRequest::new(model_name, vec![Message::user(prompt)]);
        request.system = system;
        request.tools = definitions;
        request.max_tokens = 4096;

        let mut usage = Usage::default();
        let mut calls: Vec<Value> = Vec::new();
        let (text, final_model) = 'turns: {
            for _ in 0..MAX_AGENT_TURNS {
                if ctx.cancel.is_cancelled() {
                    return Err(Stop::Cancelled);
                }
                let run_id = ctx.run_id.to_owned();
                let step_id = step.id.clone();
                let observer = self.observer.clone();
                let mut on_event = move |event: StreamEvent| {
                    if let StreamEvent::TextDelta { text } = event {
                        observer.event(RunEvent::TextDelta {
                            run_id: run_id.clone(),
                            step_id: step_id.clone(),
                            text,
                        });
                    }
                };
                let completion: Completion = tokio::select! {
                    done = provider.stream(&request, &mut on_event) => {
                        done.map_err(|e| Stop::Failed(e.message))?
                    }
                    () = ctx.cancel.cancelled() => return Err(Stop::Cancelled),
                };
                usage.input_tokens += completion.usage.input_tokens;
                usage.output_tokens += completion.usage.output_tokens;
                usage.cache_read_tokens =
                    sum_options(usage.cache_read_tokens, completion.usage.cache_read_tokens);
                usage.cache_write_tokens = sum_options(
                    usage.cache_write_tokens,
                    completion.usage.cache_write_tokens,
                );

                let tool_uses: Vec<(String, String, Value)> = completion
                    .tool_uses()
                    .into_iter()
                    .map(|(id, name, input)| (id.to_owned(), name.to_owned(), input.clone()))
                    .collect();
                if completion.stop_reason != StopReason::ToolUse || tool_uses.is_empty() {
                    break 'turns (completion.text(), completion.model.clone());
                }

                // The model asked for tools: run them (with consent) and send the results back.
                request.messages.push(Message {
                    role: Role::Assistant,
                    content: completion.content.clone(),
                });
                let mut results = Vec::new();
                for (call_id, name, input) in tool_uses {
                    let Some((server, tool)) = names.get(&name).cloned() else {
                        results.push(ContentBlock::ToolResult {
                            tool_use_id: call_id,
                            content: format!("There is no tool named {name}."),
                            is_error: true,
                        });
                        continue;
                    };
                    let outcome = self
                        .invoke_tool(ctx, &step.id, span_id, &server, &tool, input.clone())
                        .await?;
                    let (content, is_error, denied) = match &outcome {
                        ToolCall::Ran(done) => (result_text(&done.result), done.is_error, false),
                        ToolCall::Denied => (
                            "The user did not allow this tool call.".to_owned(),
                            true,
                            true,
                        ),
                        ToolCall::Failed(message) => (message.clone(), true, false),
                    };
                    calls.push(json!({
                        "server": server, "tool": tool, "arguments": input,
                        "result": content, "isError": is_error, "denied": denied,
                    }));
                    results.push(ContentBlock::ToolResult {
                        tool_use_id: call_id,
                        content,
                        is_error,
                    });
                }
                request.messages.push(Message {
                    role: Role::User,
                    content: results,
                });
            }
            return Err(Stop::Failed(format!(
                "the model kept calling tools after {MAX_AGENT_TURNS} turns"
            )));
        };

        attributes["tokens"] =
            json!(u64::from(usage.input_tokens) + u64::from(usage.output_tokens));
        attributes["usage"] =
            json!({ "inputTokens": usage.input_tokens, "outputTokens": usage.output_tokens });
        Ok((
            json!({
                "text": text,
                "model": final_model,
                "usage": {
                    "inputTokens": usage.input_tokens,
                    "outputTokens": usage.output_tokens,
                    "cacheReadTokens": usage.cache_read_tokens,
                    "cacheWriteTokens": usage.cache_write_tokens,
                },
                "calls": calls,
            }),
            None,
        ))
    }

    /// Makes one tool call with the user's consent, and records it.
    async fn invoke_tool(
        &self,
        ctx: &mut Ctx<'_>,
        step_id: &str,
        parent_span: &str,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<ToolCall, Stop> {
        if ctx.cancel.is_cancelled() {
            return Err(Stop::Cancelled);
        }
        let record = |result: Option<Value>, is_error: bool, denied: bool| RecordedCall {
            seq: 0,
            step_id: step_id.to_owned(),
            server: server.to_owned(),
            tool: tool.to_owned(),
            arguments: JsonValue(arguments.clone()),
            result: result.map(JsonValue),
            is_error,
            denied,
        };

        if self.tools.is_live() && !ctx.policy.allows(server, tool) {
            let request = ConfirmRequest {
                run_id: ctx.run_id.to_owned(),
                step_id: step_id.to_owned(),
                server: server.to_owned(),
                tool: tool.to_owned(),
                arguments: JsonValue(arguments.clone()),
            };
            let decision = tokio::select! {
                decision = self.confirmer.confirm(&request) => decision,
                () = ctx.cancel.cancelled() => return Err(Stop::Cancelled),
            };
            match decision {
                Decision::Allow => {}
                Decision::AllowTool => {
                    ctx.policy.allow_tool(server, tool);
                    save_policy(&self.settings, ctx.policy.clone()).await?;
                }
                Decision::AllowServer => {
                    ctx.policy.allow_server(server);
                    save_policy(&self.settings, ctx.policy.clone()).await?;
                }
                Decision::Deny => {
                    self.runs
                        .add_call(ctx.run_id, &record(None, false, true))
                        .await?;
                    return Ok(ToolCall::Denied);
                }
            }
        }

        let span = new_id();
        let mut attributes = json!({ "server": server, "tool": tool });
        trace::insert_span(
            &self.db,
            &NewSpan {
                id: &span,
                trace_id: ctx.run_id,
                parent_id: Some(parent_span),
                kind: SpanKind::Tool,
                name: tool,
                started_at: now_ms(),
                attributes: &attributes,
            },
        )
        .await?;
        let outcome = tokio::select! {
            outcome = self.tools.call_tool(server, tool, arguments.clone(), ctx.cancel) => Some(outcome),
            () = ctx.cancel.cancelled() => None,
        };
        let ended = now_ms();
        match outcome {
            None => {
                trace::end_span(&self.db, &span, ended, SpanStatus::Cancelled, &attributes).await?;
                Err(Stop::Cancelled)
            }
            Some(Ok(done)) => {
                self.runs
                    .add_call(
                        ctx.run_id,
                        &record(Some(done.result.clone()), done.is_error, false),
                    )
                    .await?;
                attributes["isError"] = json!(done.is_error);
                let status = if done.is_error {
                    SpanStatus::Error
                } else {
                    SpanStatus::Ok
                };
                trace::end_span(&self.db, &span, ended, status, &attributes).await?;
                Ok(ToolCall::Ran(done))
            }
            Some(Err(message)) => {
                self.runs
                    .add_call(ctx.run_id, &record(None, true, false))
                    .await?;
                attributes["error"] = json!(message);
                trace::end_span(&self.db, &span, ended, SpanStatus::Error, &attributes).await?;
                Ok(ToolCall::Failed(message))
            }
        }
    }
}

/// Problems with servers and tools: a replay does not call them, so it does not check them.
fn is_tool_issue(code: FlowIssueCode) -> bool {
    matches!(
        code,
        FlowIssueCode::UnknownServer
            | FlowIssueCode::UnknownTool
            | FlowIssueCode::MissingArgument
            | FlowIssueCode::UnknownArgument
            | FlowIssueCode::ArgumentType
    )
}

fn sum_options(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
    }
}

/// The servers a flow's tool steps and LLM tool lists refer to.
fn servers_of(flow: &Flow) -> Vec<String> {
    let mut servers: Vec<String> = Vec::new();
    for step in &flow.steps {
        let names: Vec<&str> = match &step.kind {
            StepKind::Tool { server, .. } => vec![server.as_str()],
            StepKind::Llm { tools, .. } => tools.iter().map(|t| t.server.as_str()).collect(),
            _ => vec![],
        };
        for name in names {
            if !servers.iter().any(|s| s == name) {
                servers.push(name.to_owned());
            }
        }
    }
    servers
}

/// Every declared input needs a value of the declared type.
fn check_inputs(declared: &BTreeMap<String, InputDecl>, given: &RunInputs) -> Result<(), Stop> {
    for (name, decl) in declared {
        match given.get(name) {
            None => return Err(Stop::Failed(format!("the input `{name}` has no value"))),
            Some(value) if !type_matches(value, &decl.kind) => {
                return Err(Stop::Failed(format!(
                    "the input `{name}` must be of type {}",
                    decl.kind
                )));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex, time::Duration};

    use super::*;
    use crate::{
        flow_replay::ReplayTools,
        trace::{query_spans, SpanFilter},
    };

    // ---- fakes ----------------------------------------------------------------------------

    fn tool_info(name: &str) -> ToolInfo {
        ToolInfo {
            name: name.into(),
            title: None,
            description: Some(format!("The {name} tool")),
            input_schema: JsonValue(json!({
                "type": "object",
                "properties": { "repo": { "type": "string" }, "limit": { "type": "integer" } },
                "required": ["repo"]
            })),
            output_schema: None,
            annotations: None,
        }
    }

    fn text_result(text: &str) -> ToolOutcome {
        ToolOutcome {
            result: json!({ "content": [{ "type": "text", "text": text }] }),
            is_error: false,
        }
    }

    struct FakeTools {
        servers: HashMap<String, Vec<ToolInfo>>,
        calls: Mutex<Vec<(String, String, Value)>>,
        results: Mutex<VecDeque<Result<ToolOutcome, String>>>,
        delay: Duration,
        live: bool,
    }

    impl FakeTools {
        fn new(results: Vec<Result<ToolOutcome, String>>) -> Self {
            Self {
                servers: HashMap::from([(
                    "github".to_owned(),
                    vec![tool_info("list_issues"), tool_info("get_issue")],
                )]),
                calls: Mutex::default(),
                results: Mutex::new(results.into()),
                delay: Duration::ZERO,
                live: true,
            }
        }

        fn called(&self) -> Vec<(String, String, Value)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ToolRunner for FakeTools {
        async fn list_tools(&self, server: &str) -> Result<Vec<ToolInfo>, String> {
            self.servers
                .get(server)
                .cloned()
                .ok_or_else(|| format!("{server} is not connected"))
        }

        async fn call_tool(
            &self,
            server: &str,
            tool: &str,
            arguments: Value,
            _cancel: &CancellationToken,
        ) -> Result<ToolOutcome, String> {
            self.calls
                .lock()
                .unwrap()
                .push((server.to_owned(), tool.to_owned(), arguments));
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(text_result("3 open")))
        }

        fn is_live(&self) -> bool {
            self.live
        }
    }

    struct ScriptedProvider {
        replies: Mutex<VecDeque<Completion>>,
        /// Keep answering with the last reply when the script runs out.
        repeat_last: bool,
        last: Mutex<Option<Completion>>,
        requests: Mutex<Vec<CompletionRequest>>,
    }

    impl ScriptedProvider {
        fn new(replies: Vec<Completion>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies.into()),
                repeat_last: false,
                last: Mutex::default(),
                requests: Mutex::default(),
            })
        }

        fn forever(reply: Completion) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::default(),
                repeat_last: true,
                last: Mutex::new(Some(reply)),
                requests: Mutex::default(),
            })
        }

        fn next(&self, request: &CompletionRequest) -> Completion {
            self.requests.lock().unwrap().push(request.clone());
            let popped = self.replies.lock().unwrap().pop_front();
            match popped {
                Some(reply) => reply,
                None if self.repeat_last => self.last.lock().unwrap().clone().unwrap(),
                None => panic!("the scripted model has no reply left"),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for ScriptedProvider {
        fn name(&self) -> &str {
            "scripted"
        }

        async fn complete(&self, request: &CompletionRequest) -> Result<Completion, LlmError> {
            Ok(self.next(request))
        }

        async fn stream(
            &self,
            request: &CompletionRequest,
            on_event: &mut (dyn FnMut(StreamEvent) + Send),
        ) -> Result<Completion, LlmError> {
            let completion = self.next(request);
            let text = completion.text();
            if !text.is_empty() {
                on_event(StreamEvent::TextDelta { text });
            }
            Ok(completion)
        }
    }

    struct FakeModels(Arc<ScriptedProvider>);

    impl ModelResolver for FakeModels {
        fn resolve(&self, model: &str) -> Result<(Arc<dyn LlmProvider>, String), LlmError> {
            Ok((self.0.clone(), model.to_owned()))
        }
    }

    #[derive(Default)]
    struct ScriptedConfirmer {
        decisions: Mutex<VecDeque<Decision>>,
        asked: Mutex<Vec<ConfirmRequest>>,
    }

    impl ScriptedConfirmer {
        fn answering(decisions: Vec<Decision>) -> Arc<Self> {
            Arc::new(Self {
                decisions: Mutex::new(decisions.into()),
                asked: Mutex::default(),
            })
        }

        fn asked(&self) -> Vec<ConfirmRequest> {
            self.asked.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Confirmer for ScriptedConfirmer {
        async fn confirm(&self, request: &ConfirmRequest) -> Decision {
            self.asked.lock().unwrap().push(request.clone());
            self.decisions
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Decision::Allow)
        }
    }

    #[derive(Default)]
    struct Events(Mutex<Vec<RunEvent>>);

    impl RunObserver for Events {
        fn event(&self, event: RunEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    struct Harness {
        engine: FlowEngine,
        db: Db,
        tools: Arc<FakeTools>,
        confirmer: Arc<ScriptedConfirmer>,
        events: Arc<Events>,
    }

    async fn harness(
        tools: FakeTools,
        provider: Arc<ScriptedProvider>,
        confirmer: Arc<ScriptedConfirmer>,
    ) -> Harness {
        let db = Db::open_in_memory().await.unwrap();
        let tools = Arc::new(tools);
        let events = Arc::new(Events::default());
        let engine = FlowEngine::new(
            db.clone(),
            tools.clone(),
            Arc::new(FakeModels(provider.clone())),
            confirmer.clone(),
            events.clone(),
        );
        Harness {
            engine,
            db,
            tools,
            confirmer,
            events,
        }
    }

    fn flow(value: Value) -> Flow {
        serde_json::from_value(value).expect("test flow must deserialize")
    }

    fn inputs(value: Value) -> RunInputs {
        value.as_object().cloned().unwrap_or_default()
    }

    async fn run(h: &Harness, flow: Flow, inputs: RunInputs) -> FlowRun {
        h.engine
            .run(
                RunRequest {
                    run_id: new_id(),
                    flow_id: None,
                    flow,
                    inputs,
                    replay_of: None,
                },
                CancellationToken::new(),
            )
            .await
            .unwrap()
    }

    fn answer(text: &str, input: u32, output: u32) -> Completion {
        Completion {
            model: "scripted-model".into(),
            content: vec![ContentBlock::text(text)],
            stop_reason: StopReason::EndTurn,
            usage: Usage {
                input_tokens: input,
                output_tokens: output,
                ..Usage::default()
            },
        }
    }

    fn tool_request(
        call_id: &str,
        name: &str,
        input: Value,
        usage_in: u32,
        usage_out: u32,
    ) -> Completion {
        Completion {
            model: "scripted-model".into(),
            content: vec![ContentBlock::ToolUse {
                id: call_id.into(),
                name: name.into(),
                input: JsonValue(input),
            }],
            stop_reason: StopReason::ToolUse,
            usage: Usage {
                input_tokens: usage_in,
                output_tokens: usage_out,
                ..Usage::default()
            },
        }
    }

    fn tool_flow() -> Flow {
        flow(json!({
            "version": 1, "name": "summarize",
            "steps": [
                { "id": "inputs", "type": "input", "inputs": { "repo": { "type": "string" } } },
                { "id": "issues", "type": "tool", "server": "github", "tool": "list_issues",
                  "arguments": { "repo": "{{ inputs.repo }}", "limit": 5 } },
                { "id": "t", "type": "transform", "values": { "text": "Found {{ steps.issues.result }}" } },
                { "id": "outputs", "type": "output", "outputs": { "summary": "{{ steps.t.text }}" } }
            ]
        }))
    }

    fn statuses(run: &FlowRun) -> Vec<(String, StepStatus)> {
        run.steps
            .iter()
            .map(|s| (s.step_id.clone(), s.status))
            .collect()
    }

    // ---- tests ----------------------------------------------------------------------------

    #[tokio::test]
    async fn runs_a_tool_flow_and_stores_everything() {
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![Decision::Allow]),
        )
        .await;
        let run = run(&h, tool_flow(), inputs(json!({ "repo": "a/b" }))).await;

        assert_eq!(run.status, RunStatus::Succeeded, "{:?}", run.error);
        assert_eq!(
            run.outputs.clone().unwrap().0,
            json!({ "summary": "Found 3 open" })
        );
        // Typed values keep their type: the limit is a number, the repo comes from the input.
        assert_eq!(
            h.tools.called(),
            vec![(
                "github".into(),
                "list_issues".into(),
                json!({ "repo": "a/b", "limit": 5 })
            )]
        );
        assert!(statuses(&run)
            .iter()
            .all(|(_, s)| *s == StepStatus::Succeeded));
        assert_eq!(run.steps.len(), 4);
        let issues = &run.steps[1];
        assert_eq!(issues.output.as_ref().unwrap().0["result"], "3 open");
        assert_eq!(
            issues.resolved.as_ref().unwrap().0["arguments"],
            json!({ "repo": "a/b", "limit": 5 })
        );
        // The call was recorded with its result, and the user was asked once.
        assert_eq!(run.calls.len(), 1);
        assert_eq!(
            run.calls[0].result.as_ref().unwrap().0["content"][0]["text"],
            "3 open"
        );
        let asked = h.confirmer.asked();
        assert_eq!(asked.len(), 1);
        assert_eq!(
            (
                asked[0].server.as_str(),
                asked[0].tool.as_str(),
                asked[0].step_id.as_str()
            ),
            ("github", "list_issues", "issues")
        );
        assert_eq!(asked[0].arguments.0, json!({ "repo": "a/b", "limit": 5 }));
    }

    #[tokio::test]
    async fn a_run_is_a_trace_with_a_span_per_step_and_tool_call() {
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let run = run(&h, tool_flow(), inputs(json!({ "repo": "a/b" }))).await;
        let spans = query_spans(
            &h.db,
            &SpanFilter {
                trace_id: Some(run.id.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let root = spans.iter().find(|s| s.parent_id.is_none()).unwrap();
        assert_eq!(
            (root.kind, root.name.as_str()),
            (SpanKind::Flow, "summarize")
        );
        assert_eq!(root.status, SpanStatus::Ok);
        assert!(root.ended_at.is_some());
        // The root, four steps, and the tool call under its step.
        assert_eq!(spans.len(), 6);
        let by_name = |name: &str| spans.iter().filter(|s| s.name == name).collect::<Vec<_>>();
        let step_span = by_name("issues")[0];
        assert_eq!(
            (step_span.kind, step_span.parent_id.as_deref()),
            (SpanKind::Tool, Some(root.id.as_str()))
        );
        let call_span = by_name("list_issues")[0];
        assert_eq!(call_span.parent_id.as_deref(), Some(step_span.id.as_str()));
        assert_eq!(by_name("t")[0].kind, SpanKind::Step);
        assert!(spans.iter().all(|s| s.ended_at.is_some()));
        // Each step knows its span.
        assert_eq!(run.steps[1].span_id.as_deref(), Some(step_span.id.as_str()));
        // Flow runs are listed with the sessions.
        let roots = query_spans(
            &h.db,
            &SpanFilter {
                roots_only: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(roots.len(), 1);
    }

    #[tokio::test]
    async fn progress_events_follow_the_run() {
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let run = run(&h, tool_flow(), inputs(json!({ "repo": "a/b" }))).await;
        let events = h.events.0.lock().unwrap().clone();
        assert!(
            matches!(&events[0], RunEvent::RunStarted { flow_name, .. } if flow_name == "summarize")
        );
        let started: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                RunEvent::StepStarted { step_id, .. } => Some(step_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(started, ["inputs", "issues", "t", "outputs"]);
        assert!(matches!(
            events.last().unwrap(),
            RunEvent::RunFinished { run_id, status: RunStatus::Succeeded, error: None } if *run_id == run.id
        ));
    }

    #[tokio::test]
    async fn an_invalid_flow_stops_before_any_step_runs() {
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let bad = flow(json!({
            "version": 1, "name": "bad",
            "steps": [
                { "id": "t", "type": "tool", "server": "github", "tool": "delete_everything", "arguments": { "repo": "x" } },
                { "id": "o", "type": "output" }
            ]
        }));
        let run = run(&h, bad, RunInputs::new()).await;
        assert_eq!(run.status, RunStatus::Failed);
        let error = run.error.clone().unwrap();
        assert!(
            error.contains("not valid") && error.contains("delete_everything"),
            "{error}"
        );
        assert!(run.steps.is_empty() && run.calls.is_empty());
        assert!(h.confirmer.asked().is_empty());
        assert!(h.tools.called().is_empty());
    }

    #[tokio::test]
    async fn a_server_that_cannot_be_reached_fails_the_run() {
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let f = flow(json!({
            "version": 1, "name": "x",
            "steps": [
                { "id": "t", "type": "tool", "server": "gitlab", "tool": "x" },
                { "id": "o", "type": "output" }
            ]
        }));
        let run = run(&h, f, RunInputs::new()).await;
        assert_eq!(run.status, RunStatus::Failed);
        assert!(run.error.clone().unwrap().contains("gitlab") && run.steps.is_empty());
    }

    #[tokio::test]
    async fn the_input_step_checks_values_and_types() {
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let missing = run(&h, tool_flow(), RunInputs::new()).await;
        assert_eq!(missing.status, RunStatus::Failed);
        assert_eq!(missing.steps[0].status, StepStatus::Failed);
        assert!(missing
            .error
            .clone()
            .unwrap()
            .contains("`repo` has no value"));

        let wrong = run(&h, tool_flow(), inputs(json!({ "repo": 5 }))).await;
        assert!(wrong
            .error
            .unwrap()
            .contains("`repo` must be of type string"));
        assert!(h.tools.called().is_empty());
    }

    #[tokio::test]
    async fn a_missing_value_in_a_template_fails_the_step_and_names_the_path() {
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let f = flow(json!({
            "version": 1, "name": "x",
            "steps": [
                { "id": "issues", "type": "tool", "server": "github", "tool": "list_issues", "arguments": { "repo": "r" } },
                { "id": "t", "type": "transform", "values": { "x": "{{ steps.issues.nothing }}" } },
                { "id": "o", "type": "output" }
            ]
        }));
        let run = run(&h, f, RunInputs::new()).await;
        assert_eq!(run.status, RunStatus::Failed);
        let error = run.error.clone().unwrap();
        assert!(
            error.contains("step `t` failed") && error.contains("steps.issues.nothing"),
            "{error}"
        );
        assert_eq!(statuses(&run).last().unwrap().1, StepStatus::Failed);
    }

    #[tokio::test]
    async fn denying_a_tool_call_fails_the_step_and_the_tool_never_runs() {
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![Decision::Deny]),
        )
        .await;
        let run = run(&h, tool_flow(), inputs(json!({ "repo": "a/b" }))).await;
        assert_eq!(run.status, RunStatus::Failed);
        assert!(run.error.clone().unwrap().contains("not allowed"));
        assert!(h.tools.called().is_empty());
        assert_eq!(run.steps.len(), 2, "later steps do not run");
        assert_eq!(run.calls.len(), 1);
        assert!(run.calls[0].denied && run.calls[0].result.is_none());
    }

    #[tokio::test]
    async fn allow_tool_and_allow_server_are_remembered() {
        let two_calls = flow(json!({
            "version": 1, "name": "x",
            "steps": [
                { "id": "a", "type": "tool", "server": "github", "tool": "list_issues", "arguments": { "repo": "r" } },
                { "id": "b", "type": "tool", "server": "github", "tool": "list_issues", "arguments": { "repo": "r" } },
                { "id": "c", "type": "tool", "server": "github", "tool": "get_issue", "arguments": { "repo": "r" } },
                { "id": "o", "type": "output" }
            ]
        }));
        // Allowing the tool once covers the second call of the same tool, not another tool.
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![Decision::AllowTool, Decision::Allow]),
        )
        .await;
        let first = run(&h, two_calls.clone(), RunInputs::new()).await;
        assert_eq!(first.status, RunStatus::Succeeded, "{:?}", first.error);
        assert_eq!(
            h.confirmer
                .asked()
                .iter()
                .map(|r| r.tool.as_str())
                .collect::<Vec<_>>(),
            ["list_issues", "get_issue"]
        );
        let policy = load_policy(&Settings::new(h.db.clone())).await.unwrap();
        assert_eq!(policy.allow, ["github/list_issues"]);

        // Allowing the server covers everything on it, in later runs too.
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![Decision::AllowServer]),
        )
        .await;
        run(&h, two_calls.clone(), RunInputs::new()).await;
        assert_eq!(h.confirmer.asked().len(), 1);
        let second = run(&h, two_calls, RunInputs::new()).await;
        assert_eq!(second.status, RunStatus::Succeeded);
        assert_eq!(h.confirmer.asked().len(), 1, "no new question");
        assert_eq!(
            load_policy(&Settings::new(h.db.clone()))
                .await
                .unwrap()
                .allow,
            ["github"]
        );
    }

    #[tokio::test]
    async fn a_relaxed_policy_skips_the_question_per_server_and_per_tool() {
        for allowed in ["github", "github/list_issues"] {
            let h = harness(
                FakeTools::new(vec![]),
                ScriptedProvider::new(vec![]),
                ScriptedConfirmer::answering(vec![]),
            )
            .await;
            save_policy(
                &Settings::new(h.db.clone()),
                ToolPolicy {
                    allow: vec![allowed.into()],
                },
            )
            .await
            .unwrap();
            let run = run(&h, tool_flow(), inputs(json!({ "repo": "a/b" }))).await;
            assert_eq!(run.status, RunStatus::Succeeded);
            assert!(h.confirmer.asked().is_empty(), "{allowed}");
        }
        // Another tool of the same server is still asked about.
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        save_policy(
            &Settings::new(h.db.clone()),
            ToolPolicy {
                allow: vec!["github/get_issue".into()],
            },
        )
        .await
        .unwrap();
        run(&h, tool_flow(), inputs(json!({ "repo": "a/b" }))).await;
        assert_eq!(h.confirmer.asked().len(), 1);
    }

    #[tokio::test]
    async fn runners_that_do_not_touch_servers_need_no_consent() {
        let mut tools = FakeTools::new(vec![]);
        tools.live = false;
        let h = harness(
            tools,
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![Decision::Deny]),
        )
        .await;
        let run = run(&h, tool_flow(), inputs(json!({ "repo": "a/b" }))).await;
        assert_eq!(run.status, RunStatus::Succeeded);
        assert!(h.confirmer.asked().is_empty());
    }

    #[tokio::test]
    async fn a_tool_that_reports_an_error_fails_the_step_and_stops_the_run() {
        let failing = ToolOutcome {
            result: json!({ "isError": true, "content": [{ "type": "text", "text": "boom" }] }),
            is_error: true,
        };
        let h = harness(
            FakeTools::new(vec![Ok(failing)]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let run = run(&h, tool_flow(), inputs(json!({ "repo": "a/b" }))).await;
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(run.error.as_deref(), Some("step `issues` failed: boom"));
        assert_eq!(run.steps.len(), 2);
        assert!(run.calls[0].is_error && run.calls[0].result.is_some());
        let spans = query_spans(
            &h.db,
            &SpanFilter {
                trace_id: Some(run.id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            spans.iter().find(|s| s.name == "issues").unwrap().status,
            SpanStatus::Error
        );
        assert_eq!(
            spans.iter().find(|s| s.parent_id.is_none()).unwrap().status,
            SpanStatus::Error
        );
    }

    #[tokio::test]
    async fn a_connection_error_is_recorded_as_a_failed_call() {
        let h = harness(
            FakeTools::new(vec![Err("connection lost".into())]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let run = run(&h, tool_flow(), inputs(json!({ "repo": "a/b" }))).await;
        assert_eq!(
            run.error.as_deref(),
            Some("step `issues` failed: connection lost")
        );
        assert!(run.calls[0].is_error && run.calls[0].result.is_none() && !run.calls[0].denied);
    }

    fn condition_flow() -> Flow {
        flow(json!({
            "version": 1, "name": "branch",
            "steps": [
                { "id": "inputs", "type": "input", "inputs": { "n": { "type": "integer" } } },
                { "id": "c", "type": "condition", "expression": "inputs.n > 3", "then": "big", "else": "small" },
                { "id": "small", "type": "transform", "values": { "size": "small" } },
                { "id": "big", "type": "transform", "values": { "size": "big" } },
                { "id": "outputs", "type": "output", "outputs": { "size": "{{ steps.big.size }}" } }
            ]
        }))
    }

    #[tokio::test]
    async fn a_condition_jumps_forward_and_records_the_skipped_steps() {
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let run = run(&h, condition_flow(), inputs(json!({ "n": 5 }))).await;
        assert_eq!(run.status, RunStatus::Succeeded, "{:?}", run.error);
        assert_eq!(
            run.steps[1].output.as_ref().unwrap().0,
            json!({ "value": true })
        );
        assert_eq!(
            statuses(&run)
                .iter()
                .map(|(id, s)| (id.as_str(), *s))
                .collect::<Vec<_>>(),
            [
                ("inputs", StepStatus::Succeeded),
                ("c", StepStatus::Succeeded),
                ("small", StepStatus::Skipped),
                ("big", StepStatus::Succeeded),
                ("outputs", StepStatus::Succeeded),
            ]
        );
        assert_eq!(run.outputs.clone().unwrap().0, json!({ "size": "big" }));
        let finished: Vec<_> = h
            .events
            .0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                RunEvent::StepFinished {
                    step_id,
                    status: StepStatus::Skipped,
                    ..
                } => Some(step_id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(finished, ["small"]);
    }

    #[tokio::test]
    async fn without_a_jump_the_next_step_runs_in_order() {
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        // `else` points at the next step, so nothing is skipped and the flow falls through.
        let run = run(&h, condition_flow(), inputs(json!({ "n": 1 }))).await;
        assert_eq!(run.status, RunStatus::Succeeded, "{:?}", run.error);
        assert!(statuses(&run)
            .iter()
            .all(|(_, s)| *s == StepStatus::Succeeded));
        assert_eq!(
            run.steps[1].output.as_ref().unwrap().0,
            json!({ "value": false })
        );
    }

    fn llm_flow(tools: Value) -> Flow {
        flow(json!({
            "version": 1, "name": "ask-flow",
            "steps": [
                { "id": "inputs", "type": "input", "inputs": { "topic": { "type": "string" } } },
                { "id": "ask", "type": "llm", "model": "ollama:llama3.1", "system": "Be brief about {{ inputs.topic }}.",
                  "prompt": "Summarize {{ inputs.topic }}", "tools": tools },
                { "id": "outputs", "type": "output", "outputs": { "answer": "{{ steps.ask.text }}" } }
            ]
        }))
    }

    #[tokio::test]
    async fn an_llm_step_renders_its_prompt_reports_usage_and_streams_text() {
        let provider = ScriptedProvider::new(vec![answer("Short summary", 10, 5)]);
        let h = harness(
            FakeTools::new(vec![]),
            provider.clone(),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let run = run(&h, llm_flow(json!([])), inputs(json!({ "topic": "rust" }))).await;

        assert_eq!(run.status, RunStatus::Succeeded, "{:?}", run.error);
        assert_eq!(
            run.outputs.clone().unwrap().0,
            json!({ "answer": "Short summary" })
        );
        let request = provider.requests.lock().unwrap()[0].clone();
        assert_eq!(request.model, "ollama:llama3.1");
        assert_eq!(request.system.as_deref(), Some("Be brief about rust."));
        assert_eq!(request.messages[0], Message::user("Summarize rust"));
        assert!(request.tools.is_empty());
        let step = &run.steps[1];
        assert_eq!(
            step.resolved.as_ref().unwrap().0["prompt"],
            "Summarize rust"
        );
        let output = &step.output.as_ref().unwrap().0;
        assert_eq!(output["model"], "scripted-model");
        assert_eq!(output["usage"]["inputTokens"], 10);
        assert_eq!(output["usage"]["outputTokens"], 5);
        // The text arrived as it was written, and the span carries the exact usage.
        assert!(h.events.0.lock().unwrap().iter().any(|e| matches!(
            e,
            RunEvent::TextDelta { step_id, text, .. } if step_id == "ask" && text == "Short summary"
        )));
        let spans = query_spans(
            &h.db,
            &SpanFilter {
                trace_id: Some(run.id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let llm = spans.iter().find(|s| s.name == "ask").unwrap();
        assert_eq!((llm.kind, llm.tokens), (SpanKind::Llm, Some(15)));
    }

    #[tokio::test]
    async fn a_model_that_calls_tools_gets_the_results_after_the_user_agrees() {
        let provider = ScriptedProvider::new(vec![
            tool_request(
                "call_1",
                "github__list_issues",
                json!({ "repo": "a/b" }),
                20,
                10,
            ),
            answer("Two issues are open.", 30, 5),
        ]);
        let h = harness(
            FakeTools::new(vec![Ok(text_result("3 open"))]),
            provider.clone(),
            ScriptedConfirmer::answering(vec![Decision::Allow]),
        )
        .await;
        let tools = json!([{ "server": "github", "tool": "list_issues" }]);
        let run = run(&h, llm_flow(tools), inputs(json!({ "topic": "issues" }))).await;

        assert_eq!(run.status, RunStatus::Succeeded, "{:?}", run.error);
        // The user was asked, and the tool ran with the model's arguments.
        assert_eq!(h.confirmer.asked().len(), 1);
        assert_eq!(
            h.tools.called(),
            vec![(
                "github".into(),
                "list_issues".into(),
                json!({ "repo": "a/b" })
            )]
        );
        // The model saw the tool and, in the second request, its call and the result.
        let requests = provider.requests.lock().unwrap().clone();
        assert_eq!(requests[0].tools[0].name, "github__list_issues");
        assert_eq!(requests[0].tools[0].description, "The list_issues tool");
        let second = &requests[1].messages;
        assert_eq!(second.len(), 3);
        assert!(
            matches!(&second[1].content[0], ContentBlock::ToolUse { name, .. } if name == "github__list_issues")
        );
        assert_eq!(
            second[2].content[0],
            ContentBlock::ToolResult {
                tool_use_id: "call_1".into(),
                content: "3 open".into(),
                is_error: false
            }
        );
        let output = &run.steps[1].output.as_ref().unwrap().0;
        assert_eq!(output["text"], "Two issues are open.");
        assert_eq!(output["usage"]["inputTokens"], 50);
        assert_eq!(output["usage"]["outputTokens"], 15);
        assert_eq!(output["calls"][0]["tool"], "list_issues");
        assert_eq!(output["calls"][0]["result"], "3 open");
        // The call is a span under the LLM step and is recorded for replay.
        let spans = query_spans(
            &h.db,
            &SpanFilter {
                trace_id: Some(run.id.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let step_span = spans.iter().find(|s| s.name == "ask").unwrap();
        let call_span = spans.iter().find(|s| s.name == "list_issues").unwrap();
        assert_eq!(call_span.parent_id.as_deref(), Some(step_span.id.as_str()));
        assert_eq!(run.calls.len(), 1);
        assert_eq!(run.calls[0].step_id, "ask");
    }

    #[tokio::test]
    async fn a_denied_call_of_a_model_is_reported_to_it_and_the_step_goes_on() {
        let provider = ScriptedProvider::new(vec![
            tool_request(
                "call_1",
                "github__list_issues",
                json!({ "repo": "a/b" }),
                1,
                1,
            ),
            answer("I could not look.", 1, 1),
        ]);
        let h = harness(
            FakeTools::new(vec![]),
            provider.clone(),
            ScriptedConfirmer::answering(vec![Decision::Deny]),
        )
        .await;
        let tools = json!([{ "server": "github", "tool": "list_issues" }]);
        let run = run(&h, llm_flow(tools), inputs(json!({ "topic": "x" }))).await;
        assert_eq!(run.status, RunStatus::Succeeded, "{:?}", run.error);
        assert!(h.tools.called().is_empty());
        let result = provider.requests.lock().unwrap()[1].messages[2].content[0].clone();
        assert!(
            matches!(&result, ContentBlock::ToolResult { content, is_error: true, .. } if content.contains("did not allow"))
        );
        assert!(run.calls[0].denied);
        assert_eq!(
            run.steps[1].output.as_ref().unwrap().0["calls"][0]["denied"],
            true
        );
    }

    #[tokio::test]
    async fn a_tool_the_model_invents_gets_an_error_result() {
        let provider = ScriptedProvider::new(vec![
            tool_request("call_1", "nope__tool", json!({}), 1, 1),
            answer("Sorry.", 1, 1),
        ]);
        let h = harness(
            FakeTools::new(vec![]),
            provider.clone(),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let run = run(&h, llm_flow(json!([])), inputs(json!({ "topic": "x" }))).await;
        assert_eq!(run.status, RunStatus::Succeeded, "{:?}", run.error);
        let result = provider.requests.lock().unwrap()[1].messages[2].content[0].clone();
        assert!(
            matches!(&result, ContentBlock::ToolResult { content, is_error: true, .. } if content.contains("no tool named nope__tool"))
        );
        assert!(h.confirmer.asked().is_empty() && h.tools.called().is_empty());
    }

    #[tokio::test]
    async fn a_model_that_never_stops_calling_tools_is_stopped() {
        let provider = ScriptedProvider::forever(tool_request(
            "c",
            "github__list_issues",
            json!({ "repo": "r" }),
            1,
            1,
        ));
        let h = harness(
            FakeTools::new(vec![]),
            provider.clone(),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let tools = json!([{ "server": "github", "tool": "list_issues" }]);
        save_policy(
            &Settings::new(h.db.clone()),
            ToolPolicy {
                allow: vec!["github".into()],
            },
        )
        .await
        .unwrap();
        let run = run(&h, llm_flow(tools), inputs(json!({ "topic": "x" }))).await;
        assert_eq!(run.status, RunStatus::Failed);
        assert!(run.error.clone().unwrap().contains("kept calling tools"));
        assert_eq!(provider.requests.lock().unwrap().len(), MAX_AGENT_TURNS);
    }

    #[tokio::test]
    async fn cancelling_stops_a_running_tool_call_and_marks_everything_cancelled() {
        let mut tools = FakeTools::new(vec![]);
        tools.delay = Duration::from_secs(30);
        let h = Arc::new(
            harness(
                tools,
                ScriptedProvider::new(vec![]),
                ScriptedConfirmer::answering(vec![]),
            )
            .await,
        );
        let cancel = CancellationToken::new();
        let task = {
            let h = h.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                h.engine
                    .run(
                        RunRequest {
                            run_id: "cancel-me".into(),
                            flow_id: None,
                            flow: tool_flow(),
                            inputs: inputs(json!({ "repo": "a/b" })),
                            replay_of: None,
                        },
                        cancel,
                    )
                    .await
                    .unwrap()
            })
        };
        // Wait until the tool call has started, then cancel.
        for _ in 0..200 {
            if !h.tools.called().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cancel.cancel();
        let run = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(run.status, RunStatus::Cancelled);
        assert_eq!(run.steps.last().unwrap().status, StepStatus::Cancelled);
        let spans = query_spans(
            &h.db,
            &SpanFilter {
                trace_id: Some(run.id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(spans.iter().all(|s| s.ended_at.is_some()));
        assert_eq!(
            spans.iter().find(|s| s.parent_id.is_none()).unwrap().status,
            SpanStatus::Cancelled
        );
        assert_eq!(
            spans
                .iter()
                .find(|s| s.name == "list_issues")
                .unwrap()
                .status,
            SpanStatus::Cancelled
        );
    }

    async fn replay(h: &Harness, source: &FlowRun, flow: Flow, tools: Arc<ReplayTools>) -> FlowRun {
        let engine = FlowEngine::new(
            h.db.clone(),
            tools,
            Arc::new(FakeModels(ScriptedProvider::new(vec![]))),
            h.confirmer.clone(),
            h.events.clone(),
        );
        engine
            .run(
                RunRequest {
                    run_id: new_id(),
                    flow_id: source.flow_id.clone(),
                    flow,
                    inputs: source.inputs.0.as_object().cloned().unwrap_or_default(),
                    replay_of: Some(source.id.clone()),
                },
                CancellationToken::new(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_replay_answers_tool_calls_from_the_record_without_asking_or_calling() {
        let h = harness(
            FakeTools::new(vec![Ok(text_result("3 open"))]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![Decision::Allow]),
        )
        .await;
        let original = run(&h, tool_flow(), inputs(json!({ "repo": "a/b" }))).await;
        assert_eq!(original.status, RunStatus::Succeeded);
        assert_eq!(h.tools.called().len(), 1);

        let tools = Arc::new(ReplayTools::new(&original.calls, None));
        let again = replay(&h, &original, original.flow.clone(), tools.clone()).await;

        assert_eq!(again.status, RunStatus::Succeeded, "{:?}", again.error);
        assert_eq!(again.replay_of.as_deref(), Some(original.id.as_str()));
        assert_eq!(again.outputs, original.outputs);
        // Nothing live happened: no question, no call on the real runner.
        assert_eq!(h.confirmer.asked().len(), 1, "only the original run asked");
        assert_eq!(h.tools.called().len(), 1);
        assert_eq!(tools.remaining(), 0);
        // The replay records its own calls, so it can be replayed again.
        assert_eq!(again.calls.len(), 1);
        assert_eq!(again.calls[0].result, original.calls[0].result);
        // And it is a trace of its own.
        let spans = query_spans(
            &h.db,
            &SpanFilter {
                trace_id: Some(again.id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(spans.iter().any(|s| s.kind == SpanKind::Flow));
    }

    #[tokio::test]
    async fn a_changed_flow_replays_with_the_same_tool_data() {
        let h = harness(
            FakeTools::new(vec![Ok(text_result("3 open"))]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let original = run(&h, tool_flow(), inputs(json!({ "repo": "a/b" }))).await;
        let mut changed = original.flow.clone();
        if let StepKind::Transform { values } = &mut changed.steps[2].kind {
            values.insert("text".into(), "Now {{ steps.issues.result }}!".into());
        }
        let again = replay(
            &h,
            &original,
            changed,
            Arc::new(ReplayTools::new(&original.calls, None)),
        )
        .await;
        assert_eq!(again.status, RunStatus::Succeeded, "{:?}", again.error);
        assert_eq!(
            again.outputs.unwrap().0,
            json!({ "summary": "Now 3 open!" })
        );
    }

    #[tokio::test]
    async fn a_replay_fails_when_the_flow_calls_more_than_was_recorded() {
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let original = run(&h, tool_flow(), inputs(json!({ "repo": "a/b" }))).await;
        let twice = flow(json!({
            "version": 1, "name": "twice",
            "steps": [
                { "id": "a", "type": "tool", "server": "github", "tool": "list_issues", "arguments": { "repo": "r" } },
                { "id": "b", "type": "tool", "server": "github", "tool": "list_issues", "arguments": { "repo": "r" } },
                { "id": "o", "type": "output" }
            ]
        }));
        let again = replay(
            &h,
            &original,
            twice,
            Arc::new(ReplayTools::new(&original.calls, None)),
        )
        .await;
        assert_eq!(again.status, RunStatus::Failed);
        assert!(again
            .error
            .unwrap()
            .contains("no recorded result left for github/list_issues"));
        assert_eq!(again.steps[0].status, StepStatus::Succeeded);
    }

    #[tokio::test]
    async fn a_replay_needs_no_connected_servers() {
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let original = run(&h, tool_flow(), inputs(json!({ "repo": "a/b" }))).await;
        // Tools come from the record, and tool checks are skipped, so a gone server is fine.
        let other_server = serde_json::from_value::<Flow>(
            serde_json::to_value(&original.flow)
                .unwrap()
                .to_string()
                .replace("github", "gone")
                .parse::<Value>()
                .unwrap(),
        )
        .unwrap();
        let calls: Vec<RecordedCall> = original
            .calls
            .iter()
            .cloned()
            .map(|mut c| {
                c.server = "gone".into();
                c
            })
            .collect();
        let again = replay(
            &h,
            &original,
            other_server,
            Arc::new(ReplayTools::new(&calls, None)),
        )
        .await;
        assert_eq!(again.status, RunStatus::Succeeded, "{:?}", again.error);
    }

    #[tokio::test]
    async fn the_flow_is_stored_with_the_run() {
        let h = harness(
            FakeTools::new(vec![]),
            ScriptedProvider::new(vec![]),
            ScriptedConfirmer::answering(vec![]),
        )
        .await;
        let f = tool_flow();
        let run = run(&h, f.clone(), inputs(json!({ "repo": "a/b" }))).await;
        assert_eq!(run.flow, f);
        assert_eq!(run.inputs.0, json!({ "repo": "a/b" }));
    }

    #[test]
    fn tool_names_for_models_are_valid_and_unique() {
        let mut taken = HashMap::new();
        let first = model_tool_name("my server", "list.issues", &taken);
        assert_eq!(first, "my_server__list_issues");
        taken.insert(first.clone(), ("a".into(), "b".into()));
        let second = model_tool_name("my_server", "list_issues", &taken);
        assert_ne!(second, first);
        assert!(second
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
        let long = model_tool_name(&"s".repeat(40), &"t".repeat(40), &HashMap::new());
        assert_eq!(long.len(), 64);
    }

    #[test]
    fn tool_results_become_text() {
        assert_eq!(
            result_text(
                &json!({ "content": [{ "type": "text", "text": "a" }, { "type": "image", "data": "x" }, { "type": "text", "text": "b" }] })
            ),
            "a\nb"
        );
        assert_eq!(
            result_text(&json!({ "structuredContent": { "n": 1 } })),
            r#"{"n":1}"#
        );
        assert_eq!(result_text(&json!({})), "");
    }

    #[test]
    fn the_policy_matches_servers_and_tools_and_cleans_up() {
        let mut policy = ToolPolicy::default();
        assert!(!policy.allows("github", "x"));
        policy.allow_tool("github", "list_issues");
        policy.allow_tool("github", "list_issues");
        assert!(policy.allows("github", "list_issues") && !policy.allows("github", "other"));
        policy.allow_server("slack");
        assert!(policy.allows("slack", "anything") && !policy.allows("slack2", "anything"));
        assert_eq!(policy.allow.len(), 2);
        let cleaned = ToolPolicy {
            allow: vec![" b ".into(), "".into(), "a".into(), "b".into()],
        }
        .normalized();
        assert_eq!(cleaned.allow, ["a", "b"]);
    }
}
