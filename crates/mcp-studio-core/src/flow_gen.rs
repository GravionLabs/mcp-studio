//! Generating a flow from a goal written in natural language.
//!
//! A model gets the goal, the format of a flow, and the tools of the servers the user chose, and
//! answers with a flow as YAML. The answer is never trusted: it is parsed, and validated against the
//! same tool catalog that is validated at run time. If it has problems, the model gets one chance to
//! repair them. The caller opens the flow in the editor only when `issues` is empty, and nothing is
//! run by generating: tools are only described to the model, never called.
//!
//! A generation costs one model call, or two when a repair is needed. The goal and the tool
//! definitions are sent to the provider of the chosen model.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use crate::{
    db::{DbError, DbResult},
    flow::{self, Flow, FlowIssue, ToolCatalog},
    flow_yaml,
    llm::{CompletionRequest, LlmProvider, Message},
};

/// Model calls for one generation: the first answer and one repair.
const ATTEMPTS: usize = 2;
/// Characters of a tool description that are sent.
const DESCRIPTION_CHARS: usize = 400;
/// Longest goal that is accepted.
pub const MAX_GOAL_CHARS: usize = 4000;

const SYSTEM: &str = "You design workflows (flows) that combine MCP tool calls and language model \
calls. You use only the servers and tools you are given and never invent tools or arguments. Answer \
with the flow as YAML only, in one code block.";

/// The outcome of a generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedFlow {
    /// The flow, when the answer could be read as one.
    pub flow: Option<Flow>,
    /// The YAML text the model wrote, as far as it could be extracted.
    pub yaml: String,
    /// What is wrong with it, from reading it or from validation. Empty means it is valid and may
    /// be opened.
    pub issues: Vec<FlowIssue>,
    /// Model calls this took (1, or 2 with a repair).
    pub attempts: u32,
}

impl GeneratedFlow {
    /// Whether the flow was read and passed validation.
    pub fn is_valid(&self) -> bool {
        self.flow.is_some() && self.issues.is_empty()
    }
}

/// A short description of the tools for the prompt: name, what it does, and its parameters.
fn catalog_lines(catalog: &ToolCatalog) -> Value {
    let servers: serde_json::Map<String, Value> = catalog
        .iter()
        .map(|(server, tools)| {
            let tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    let schema = &t.input_schema.0;
                    let required: Vec<&Value> = schema
                        .get("required")
                        .and_then(Value::as_array)
                        .map(|r| r.iter().collect())
                        .unwrap_or_default();
                    let parameters: serde_json::Map<String, Value> = schema
                        .get("properties")
                        .and_then(Value::as_object)
                        .map(|p| {
                            p.iter()
                                .map(|(name, spec)| {
                                    (
                                        name.clone(),
                                        spec.get("type").cloned().unwrap_or(Value::Null),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    json!({
                        "name": t.name,
                        "description": t.description.as_deref().map(|d| d.chars().take(DESCRIPTION_CHARS).collect::<String>()),
                        "parameters": parameters,
                        "required": required,
                    })
                })
                .collect();
            (server.clone(), Value::Array(tools))
        })
        .collect();
    Value::Object(servers)
}

/// The question that asks a model for a flow.
pub fn generation_prompt(goal: &str, catalog: &ToolCatalog) -> String {
    format!(
        "Goal: {goal}\n\nServers and their tools (use the server names exactly as written):\n{}\n\n\
Write a flow as YAML in this format:\n\n```yaml\nversion: 1\nname: short-kebab-case-name\ninputs:\n  \
repo: {{ type: string, description: what it is }}\nsteps:\n  - id: issues\n    type: tool\n    server: \
github\n    tool: list_issues\n    arguments:\n      repo: \"{{{{ inputs.repo }}}}\"\n  - id: summary\n    \
type: llm\n    model: claude-sonnet-5-5\n    prompt: |\n      Summarize: {{{{ steps.issues.result }}}}\n\
outputs:\n  summary: \"{{{{ steps.summary.text }}}}\"\n```\n\nRules:\n\
- Step types: `tool` (server, tool, arguments), `llm` (model, prompt, optional system, optional tools \
as a list of `{{ server, tool }}` that the model may call), `condition` (expression, then, else: ids of \
later steps), `transform` (values: templates).\n\
- Step ids are lowercase letters, digits, `_` or `-`. Use `inputs:` for what the user must provide \
and `outputs:` for the result.\n\
- Templates are `{{{{ ... }}}}` and may use `inputs.<name>` and `steps.<id>.<field>`. A tool step has \
`result` (text), `structured`, `isError`; an llm step has `text`. A text that is only one template keeps \
its type.\n\
- Use only the tools above, with their parameters. Give every required parameter a value.\n\
- Keep the flow as small as the goal allows. Use `claude-sonnet-5-5` for llm steps.\n\
Answer with the YAML in one code block and nothing else.",
        serde_json::to_string_pretty(&catalog_lines(catalog)).unwrap_or_default(),
    )
}

/// The question that asks for a repair, with what was wrong.
fn repair_prompt(yaml: &str, issues: &[FlowIssue]) -> String {
    let problems: Vec<String> = issues
        .iter()
        .map(|i| match &i.step_id {
            Some(step) => format!("- step {step}: {}", i.message),
            None => format!("- {}", i.message),
        })
        .collect();
    format!(
        "The flow you wrote has problems:\n{}\n\nYour flow:\n```yaml\n{yaml}\n```\n\nFix these problems \
and answer with the complete corrected flow as YAML in one code block, and nothing else.",
        problems.join("\n")
    )
}

/// The YAML in a model's answer: the first fenced block, or the whole text without a fence.
pub fn extract_yaml(text: &str) -> String {
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        // Skip the language tag on the opening line.
        let body = after.split_once('\n').map_or("", |(_, rest)| rest);
        let end = body.find("```").unwrap_or(body.len());
        return body[..end].trim().to_owned();
    }
    text.trim().to_owned()
}

/// Reads and validates an answer. A flow that cannot be read has one issue saying why.
pub fn check_answer(text: &str, catalog: &ToolCatalog, attempts: u32) -> GeneratedFlow {
    let yaml = extract_yaml(text);
    match flow_yaml::from_yaml(&yaml) {
        Ok(flow) => {
            let issues = flow::validate(&flow, catalog);
            GeneratedFlow {
                flow: Some(flow),
                yaml,
                issues,
                attempts,
            }
        }
        Err(error) => GeneratedFlow {
            flow: None,
            yaml,
            issues: vec![FlowIssue {
                step_id: None,
                code: flow::FlowIssueCode::EmptyField,
                message: match error {
                    DbError::Invalid(message) => message,
                    other => other.to_string(),
                },
            }],
            attempts,
        },
    }
}

/// Asks `provider` for a flow that reaches `goal` with the tools in `catalog`. Every server the flow
/// may use must be in the catalog, so nothing is left unchecked. Sends the goal and the tool
/// definitions to the provider.
pub async fn generate_flow(
    provider: &dyn LlmProvider,
    model: &str,
    goal: &str,
    catalog: &ToolCatalog,
) -> DbResult<GeneratedFlow> {
    let goal = goal.trim();
    if goal.is_empty() {
        return Err(DbError::Invalid("describe what the flow should do".into()));
    }
    if goal.chars().count() > MAX_GOAL_CHARS {
        return Err(DbError::Invalid(format!(
            "the goal is longer than {MAX_GOAL_CHARS} characters"
        )));
    }
    if catalog.values().all(Vec::is_empty) {
        return Err(DbError::Invalid(
            "choose at least one server that has tools".into(),
        ));
    }
    let mut messages = vec![Message::user(generation_prompt(goal, catalog))];
    let mut result = None;
    for attempt in 1..=ATTEMPTS {
        let mut request = CompletionRequest::new(model, messages.clone());
        request.system = Some(SYSTEM.to_owned());
        request.max_tokens = 4096;
        request.temperature = Some(0.2);
        let completion = provider.complete(&request).await?;
        let answer = check_answer(&completion.text(), catalog, attempt as u32);
        let done = answer.is_valid();
        if !done && attempt < ATTEMPTS {
            messages = vec![
                Message::user(generation_prompt(goal, catalog)),
                Message::user(repair_prompt(&answer.yaml, &answer.issues)),
            ];
        }
        result = Some(answer);
        if done {
            break;
        }
    }
    result.ok_or_else(|| DbError::Invalid("no answer".into()))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use crate::{
        explorer::ToolInfo,
        llm::{Completion, ContentBlock, LlmError, LlmErrorKind, StopReason, StreamEvent, Usage},
        model::JsonValue,
    };

    fn catalog() -> ToolCatalog {
        let mut catalog = ToolCatalog::new();
        catalog.insert(
            "github".into(),
            vec![ToolInfo {
                name: "list_issues".into(),
                title: None,
                description: Some("Lists issues".into()),
                input_schema: JsonValue(json!({
                    "type": "object",
                    "properties": {"repo": {"type": "string"}},
                    "required": ["repo"],
                })),
                output_schema: None,
                annotations: None,
            }],
        );
        catalog
    }

    const GOOD: &str = "version: 1\nname: issues\ninputs:\n  repo: { type: string }\nsteps:\n  - id: issues\n    type: tool\n    server: github\n    tool: list_issues\n    arguments:\n      repo: \"{{ inputs.repo }}\"\noutputs:\n  list: \"{{ steps.issues.result }}\"\n";

    /// Answers with the queued texts, one per call, and remembers the requests.
    struct Scripted {
        answers: Mutex<Vec<String>>,
        requests: Mutex<Vec<CompletionRequest>>,
    }

    impl Scripted {
        fn new(answers: &[&str]) -> Self {
            Self {
                answers: Mutex::new(answers.iter().rev().map(|s| (*s).to_owned()).collect()),
                requests: Mutex::default(),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for Scripted {
        fn name(&self) -> &str {
            "fake"
        }

        async fn complete(&self, request: &CompletionRequest) -> Result<Completion, LlmError> {
            self.requests.lock().unwrap().push(request.clone());
            let text = self
                .answers
                .lock()
                .unwrap()
                .pop()
                .ok_or_else(|| LlmError::new(LlmErrorKind::Other, "no more answers"))?;
            Ok(Completion {
                model: "fake".into(),
                content: vec![ContentBlock::text(text)],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
            })
        }

        async fn stream(
            &self,
            request: &CompletionRequest,
            _: &mut (dyn FnMut(StreamEvent) + Send),
        ) -> Result<Completion, LlmError> {
            self.complete(request).await
        }
    }

    fn fenced(yaml: &str) -> String {
        format!("Here you go:\n```yaml\n{yaml}```\nDone.")
    }

    #[test]
    fn extracts_yaml_from_a_fence_or_plain_text() {
        assert_eq!(extract_yaml(&fenced("a: 1\n")), "a: 1");
        assert_eq!(extract_yaml("```\nb: 2\n```"), "b: 2");
        assert_eq!(extract_yaml("  c: 3\n"), "c: 3");
        // An unclosed fence still yields the body.
        assert_eq!(extract_yaml("```yaml\nd: 4\n"), "d: 4");
    }

    #[test]
    fn the_prompt_lists_servers_tools_and_parameters() {
        let prompt = generation_prompt("close stale issues", &catalog());
        assert!(prompt.contains("Goal: close stale issues"));
        assert!(prompt.contains("\"github\""));
        assert!(prompt.contains("list_issues"));
        assert!(prompt.contains("\"repo\""));
        assert!(prompt.contains("{{ inputs.repo }}"));
    }

    #[test]
    fn a_valid_answer_is_valid() {
        let answer = check_answer(&fenced(GOOD), &catalog(), 1);
        assert!(answer.is_valid(), "{:?}", answer.issues);
        assert_eq!(answer.flow.unwrap().name, "issues");
    }

    #[test]
    fn a_made_up_tool_is_reported() {
        let yaml = GOOD.replace("list_issues", "close_everything");
        let answer = check_answer(&yaml, &catalog(), 1);
        assert!(!answer.is_valid());
        assert!(answer
            .issues
            .iter()
            .any(|i| i.code == flow::FlowIssueCode::UnknownTool));
        assert!(answer.flow.is_some());
    }

    #[test]
    fn unreadable_yaml_becomes_one_issue_and_no_flow() {
        let answer = check_answer("this is: [not a flow", &catalog(), 1);
        assert!(answer.flow.is_none());
        assert_eq!(answer.issues.len(), 1);
        assert!(!answer.is_valid());
        let unknown_key = check_answer("version: 1\nname: x\nsurprise: 1\n", &catalog(), 1);
        assert!(unknown_key.flow.is_none());
    }

    #[tokio::test]
    async fn generates_in_one_call_when_the_answer_is_valid() {
        let model = Scripted::new(&[&fenced(GOOD)]);
        let out = generate_flow(&model, "claude-x", "list issues", &catalog())
            .await
            .unwrap();
        assert!(out.is_valid());
        assert_eq!(out.attempts, 1);
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].model, "claude-x");
        // The model is only told about tools; it is not given any to call.
        assert!(requests[0].tools.is_empty());
    }

    #[tokio::test]
    async fn repairs_once_and_tells_the_model_what_was_wrong() {
        let bad = GOOD.replace("list_issues", "close_everything");
        let model = Scripted::new(&[&fenced(&bad), &fenced(GOOD)]);
        let out = generate_flow(&model, "m", "list issues", &catalog())
            .await
            .unwrap();
        assert!(out.is_valid());
        assert_eq!(out.attempts, 2);
        let requests = model.requests.lock().unwrap();
        let repair = format!("{:?}", requests[1].messages);
        assert!(repair.contains("close_everything"));
        assert!(repair.contains("problems"));
    }

    #[tokio::test]
    async fn gives_up_after_the_repair_and_returns_the_issues() {
        let bad = GOOD.replace("list_issues", "close_everything");
        let model = Scripted::new(&[&fenced(&bad), &fenced(&bad), &fenced(GOOD)]);
        let out = generate_flow(&model, "m", "list issues", &catalog())
            .await
            .unwrap();
        assert!(!out.is_valid());
        assert_eq!(out.attempts, 2);
        assert!(!out.issues.is_empty());
        // The third answer is never asked for.
        assert_eq!(model.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn rejects_empty_goals_and_catalogs_without_tools_before_calling_the_model() {
        let model = Scripted::new(&[]);
        assert!(generate_flow(&model, "m", "  ", &catalog()).await.is_err());
        let long = "x".repeat(MAX_GOAL_CHARS + 1);
        assert!(generate_flow(&model, "m", &long, &catalog()).await.is_err());
        let mut empty = ToolCatalog::new();
        empty.insert("github".into(), Vec::new());
        assert!(generate_flow(&model, "m", "go", &empty).await.is_err());
        assert!(model.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_provider_error_is_returned() {
        let model = Scripted::new(&[]);
        assert!(generate_flow(&model, "m", "go", &catalog()).await.is_err());
    }
}
