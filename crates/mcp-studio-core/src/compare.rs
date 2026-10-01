//! Comparing variants of a prompt and of tool descriptions.
//!
//! A **variant** is a system prompt and/or replacement descriptions for some tools. To compare
//! variants, every case of a test suite is sent to a model together with the tool definitions of
//! the variant, and the answer is checked against what the case expects (which tool the model
//! calls first, no tool, or text in the answer). The tools are only *offered* to the model; they
//! are never called, so nothing here needs the user's consent and nothing touches a server.
//! A model can also propose variants, given the tools and the cases that fail.
//!
//! Every comparison costs model calls: one per case and variant (and one to propose variants).

use std::collections::BTreeMap;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;
use tokio_util::sync::CancellationToken;

use crate::{
    db::{DbError, DbResult},
    explorer::ToolInfo,
    llm::{CompletionRequest, ContentBlock, LlmProvider, Message, ToolDefinition},
    metering::context_cost,
    test_suites::{Expectation, TestCase, TestSuite},
};

/// Cases sent to the model at the same time.
const PARALLEL_CASES: usize = 4;
/// Longest description a proposed variant may have.
pub const MAX_PROPOSED_DESCRIPTION: usize = 1500;
/// A text is cut to this many characters when it is stored in a result.
const ANSWER_CHARS: usize = 300;

/// A way to describe the tools to the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Variant {
    pub id: String,
    pub label: String,
    /// Replaces the system prompt of the suite; `None` keeps the suite's own.
    pub system_prompt: Option<String>,
    /// Replacement descriptions by tool name; other tools keep theirs.
    pub tool_descriptions: BTreeMap<String, String>,
}

/// The outcome of one case for one variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CaseResult {
    pub case_id: String,
    pub input: String,
    pub expectation: Expectation,
    pub passed: bool,
    /// The first tool the model called, if any.
    pub called_tool: Option<String>,
    /// The start of the text the model wrote.
    pub answer: String,
    /// Exact tokens the provider reported for this call.
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Set when the call to the model failed; the case then counts as not passed.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VariantResult {
    pub variant: Variant,
    /// In the order of the cases of the suite.
    pub results: Vec<CaseResult>,
    pub passed: u32,
    pub total: u32,
    /// Exact tokens summed over all cases.
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Estimated tokens of the tool definitions that every request of this variant carries.
    pub definition_tokens: u32,
}

impl VariantResult {
    /// Share of passed cases, 0–1; 0 for an empty suite.
    pub fn accuracy(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            f64::from(self.passed) / f64::from(self.total)
        }
    }
}

/// The tools as the variant describes them.
fn tools_for(tools: &[ToolInfo], variant: &Variant) -> Vec<ToolInfo> {
    tools
        .iter()
        .map(|tool| {
            let mut tool = tool.clone();
            if let Some(description) = variant.tool_descriptions.get(&tool.name) {
                tool.description = Some(description.clone());
            }
            tool
        })
        .collect()
}

/// A name that every model API accepts, and the way back to the real name.
fn model_names(tools: &[ToolInfo]) -> BTreeMap<String, String> {
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    for tool in tools {
        let clean: String = tool
            .name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .take(64)
            .collect();
        let mut name = clean.clone();
        let mut n = 2;
        while names.contains_key(&name) {
            name = format!("{}_{n}", clean.chars().take(60).collect::<String>());
            n += 1;
        }
        names.insert(name, tool.name.clone());
    }
    names
}

fn excerpt(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= ANSWER_CHARS {
        text.to_owned()
    } else {
        format!(
            "{}…",
            text.chars()
                .take(ANSWER_CHARS)
                .collect::<String>()
                .trim_end()
        )
    }
}

fn check(expectation: &Expectation, called: Option<&str>, answer: &str) -> bool {
    match expectation {
        Expectation::Tool { name } => called == Some(name.as_str()),
        Expectation::NoTool => called.is_none(),
        Expectation::Answer { contains } => {
            called.is_none() && answer.to_lowercase().contains(&contains.to_lowercase())
        }
    }
}

async fn run_case(
    provider: &dyn LlmProvider,
    model: &str,
    system: Option<&str>,
    definitions: &[ToolDefinition],
    names: &BTreeMap<String, String>,
    case: &TestCase,
) -> CaseResult {
    let mut request = CompletionRequest::new(model, vec![Message::user(case.input.clone())]);
    request.system = system.map(str::to_owned);
    request.tools = definitions.to_vec();
    request.max_tokens = 512;
    request.temperature = Some(0.0);
    let base = CaseResult {
        case_id: case.id.clone(),
        input: case.input.clone(),
        expectation: case.expectation.clone(),
        passed: false,
        called_tool: None,
        answer: String::new(),
        input_tokens: 0,
        output_tokens: 0,
        error: None,
    };
    match provider.complete(&request).await {
        Ok(completion) => {
            let called = completion.content.iter().find_map(|block| match block {
                ContentBlock::ToolUse { name, .. } => {
                    Some(names.get(name).cloned().unwrap_or_else(|| name.clone()))
                }
                _ => None,
            });
            let answer = completion.text();
            CaseResult {
                passed: check(&case.expectation, called.as_deref(), &answer),
                called_tool: called,
                answer: excerpt(&answer),
                input_tokens: completion.usage.input_tokens,
                output_tokens: completion.usage.output_tokens,
                ..base
            }
        }
        Err(error) => CaseResult {
            error: Some(error.message),
            ..base
        },
    }
}

/// Runs every case of a suite with one variant. `on_case` is told about each case as it finishes
/// (in any order); the returned results are in the order of the suite. A cancelled run marks the
/// cases that did not start as not passed with the error `cancelled`.
pub async fn evaluate_variant(
    provider: &dyn LlmProvider,
    model: &str,
    tools: &[ToolInfo],
    suite: &TestSuite,
    variant: &Variant,
    cancel: &CancellationToken,
    on_case: &(dyn Fn(&CaseResult) + Send + Sync),
) -> VariantResult {
    let described = tools_for(tools, variant);
    let names = model_names(&described);
    let by_original: BTreeMap<&String, &String> =
        names.iter().map(|(model, real)| (real, model)).collect();
    let definitions: Vec<ToolDefinition> = described
        .iter()
        .map(|tool| ToolDefinition {
            name: by_original
                .get(&tool.name)
                .map_or_else(|| tool.name.clone(), |n| (*n).clone()),
            description: tool.description.clone().unwrap_or_default(),
            input_schema: tool.input_schema.clone(),
        })
        .collect();
    let system = variant
        .system_prompt
        .as_deref()
        .or(suite.system_prompt.as_deref());

    let mut indexed: Vec<(usize, CaseResult)> =
        futures_util::stream::iter(suite.cases.clone().into_iter().enumerate())
            .map(|(index, case)| {
                let (definitions, names) = (&definitions, &names);
                async move {
                    let result = if cancel.is_cancelled() {
                        CaseResult {
                            case_id: case.id.clone(),
                            input: case.input.clone(),
                            expectation: case.expectation.clone(),
                            passed: false,
                            called_tool: None,
                            answer: String::new(),
                            input_tokens: 0,
                            output_tokens: 0,
                            error: Some("cancelled".into()),
                        }
                    } else {
                        run_case(provider, model, system, definitions, names, &case).await
                    };
                    on_case(&result);
                    (index, result)
                }
            })
            .buffer_unordered(PARALLEL_CASES)
            .collect()
            .await;
    indexed.sort_by_key(|(index, _)| *index);
    let results: Vec<CaseResult> = indexed.into_iter().map(|(_, r)| r).collect();

    VariantResult {
        passed: u32::try_from(results.iter().filter(|r| r.passed).count()).unwrap_or(u32::MAX),
        total: u32::try_from(results.len()).unwrap_or(u32::MAX),
        input_tokens: results.iter().map(|r| r.input_tokens).sum(),
        output_tokens: results.iter().map(|r| r.output_tokens).sum(),
        definition_tokens: context_cost(&described, None).tokens,
        variant: variant.clone(),
        results,
    }
}

/// Progress of a comparison, for the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvalEvent {
    #[serde(rename_all = "camelCase")]
    CaseDone {
        run_id: String,
        variant_id: String,
        case_id: String,
        passed: bool,
    },
    #[serde(rename_all = "camelCase")]
    VariantDone {
        run_id: String,
        result: VariantResult,
    },
    /// The comparison is over; `error` is set when it could not run at all.
    #[serde(rename_all = "camelCase")]
    Finished {
        run_id: String,
        cancelled: bool,
        error: Option<String>,
    },
}

/// The variant that keeps everything as it is, to compare the others against.
pub fn baseline() -> Variant {
    Variant {
        id: "baseline".into(),
        label: "Current".into(),
        system_prompt: None,
        tool_descriptions: BTreeMap::new(),
    }
}

// ---- proposing variants -----------------------------------------------------------------------

const PROPOSAL_SYSTEM: &str = "You improve how tools are described to a language model so that it picks \
the right tool. You change descriptions and the system prompt only; you never change what a tool does \
and never invent capabilities. Answer with JSON only.";

/// The question that asks a model for variants.
pub fn proposal_prompt(
    tools: &[ToolInfo],
    suite: &TestSuite,
    failing: &[CaseResult],
    count: usize,
) -> String {
    let tool_lines: Vec<Value> = tools
        .iter()
        .map(|t| {
            let parameters: Vec<&String> = t
                .input_schema
                .0
                .get("properties")
                .and_then(Value::as_object)
                .map(|p| p.keys().collect())
                .unwrap_or_default();
            json!({ "name": t.name, "description": t.description, "parameters": parameters })
        })
        .collect();
    let failing_lines: Vec<Value> = failing
        .iter()
        .map(|f| {
            json!({
                "input": f.input,
                "expected": f.expectation.describe(),
                "got": f.called_tool.clone().map_or_else(|| format!("no tool, answer: {}", f.answer), |t| format!("tool {t}")),
            })
        })
        .collect();
    format!(
        "The current system prompt is: {}\n\nThe tools are:\n{}\n\n{}\n\nPropose {count} different variants \
that should make the model choose correctly. A variant may rewrite descriptions of some tools (keep \
them short and concrete: what the tool does, when to use it, and when not) and may change the system \
prompt.\n\nAnswer with JSON only, in this form:\n{{\"variants\": [{{\"label\": \"short name\", \
\"systemPrompt\": null, \"toolDescriptions\": {{\"tool_name\": \"new description\"}}}}]}}\nUse only \
tool names from the list. Use null for systemPrompt to keep the current one.",
        suite.system_prompt.as_deref().map_or("(none)".to_owned(), |p| format!("{p:?}")),
        serde_json::to_string_pretty(&tool_lines).unwrap_or_default(),
        if failing_lines.is_empty() {
            "The model currently handles all test cases; look for descriptions that are vague or overlap.".to_owned()
        } else {
            format!(
                "With the current descriptions the model gets these cases wrong:\n{}",
                serde_json::to_string_pretty(&failing_lines).unwrap_or_default()
            )
        },
    )
}

/// The JSON object in a model's answer, which may be wrapped in a code fence or in prose.
fn json_in(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(text.get(start..=end)?).ok()
}

/// Reads the variants out of a model's answer. Entries that cannot be used are dropped: unknown
/// tools, empty or very long descriptions, variants that change nothing.
pub fn parse_proposals(text: &str, tools: &[ToolInfo], max: usize) -> DbResult<Vec<Variant>> {
    let value = json_in(text)
        .ok_or_else(|| DbError::Invalid("the model did not answer with JSON".into()))?;
    let entries = value
        .get("variants")
        .and_then(Value::as_array)
        .ok_or_else(|| DbError::Invalid("the model's answer has no list of variants".into()))?;
    let mut variants = Vec::new();
    for entry in entries {
        let mut descriptions = BTreeMap::new();
        if let Some(map) = entry.get("toolDescriptions").and_then(Value::as_object) {
            for (name, description) in map {
                let description = description.as_str().map(str::trim).unwrap_or("");
                let known = tools.iter().any(|t| &t.name == name);
                if known
                    && !description.is_empty()
                    && description.chars().count() <= MAX_PROPOSED_DESCRIPTION
                {
                    descriptions.insert(name.clone(), description.to_owned());
                }
            }
        }
        let system_prompt = entry
            .get("systemPrompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned);
        if descriptions.is_empty() && system_prompt.is_none() {
            continue;
        }
        let label = entry
            .get("label")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map_or_else(|| format!("Variant {}", variants.len() + 1), str::to_owned);
        variants.push(Variant {
            id: format!("v{}", variants.len() + 1),
            label,
            system_prompt,
            tool_descriptions: descriptions,
        });
        if variants.len() == max {
            break;
        }
    }
    if variants.is_empty() {
        return Err(DbError::Invalid(
            "the model proposed no usable variant".into(),
        ));
    }
    Ok(variants)
}

/// Asks the model for variants.
pub async fn propose_variants(
    provider: &dyn LlmProvider,
    model: &str,
    tools: &[ToolInfo],
    suite: &TestSuite,
    failing: &[CaseResult],
    count: usize,
) -> DbResult<Vec<Variant>> {
    let mut request = CompletionRequest::new(
        model,
        vec![Message::user(proposal_prompt(tools, suite, failing, count))],
    );
    request.system = Some(PROPOSAL_SYSTEM.to_owned());
    request.max_tokens = 4096;
    request.temperature = Some(0.7);
    let completion = provider.complete(&request).await?;
    parse_proposals(&completion.text(), tools, count)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    use async_trait::async_trait;

    use super::*;
    use crate::{
        llm::{Completion, LlmError, LlmErrorKind, StopReason, StreamEvent, Usage},
        model::JsonValue,
        test_suites::TestCase,
    };

    fn tool(name: &str, description: &str) -> ToolInfo {
        ToolInfo {
            name: name.into(),
            title: None,
            description: Some(description.into()),
            input_schema: JsonValue(
                json!({ "type": "object", "properties": { "repo": { "type": "string" } } }),
            ),
            output_schema: None,
            annotations: None,
        }
    }

    fn case(id: &str, input: &str, expectation: Expectation) -> TestCase {
        TestCase {
            id: id.into(),
            input: input.into(),
            expectation,
            notes: None,
        }
    }

    fn suite() -> TestSuite {
        TestSuite {
            id: "s".into(),
            server_id: "srv".into(),
            name: "suite".into(),
            system_prompt: Some("Be brief.".into()),
            updated_at: 0,
            cases: vec![
                case(
                    "c1",
                    "open issues in a/b?",
                    Expectation::Tool {
                        name: "list_issues".into(),
                    },
                ),
                case("c2", "hello", Expectation::NoTool),
                case(
                    "c3",
                    "which repo is this?",
                    Expectation::Answer {
                        contains: "A/B".into(),
                    },
                ),
            ],
        }
    }

    fn tools() -> Vec<ToolInfo> {
        vec![
            tool("list_issues", "Lists issues"),
            tool("get.issue", "Gets one issue"),
        ]
    }

    /// Answers by looking at the input: tool calls for "issues", text otherwise.
    struct Model {
        requests: Mutex<Vec<CompletionRequest>>,
        fail_on: Option<&'static str>,
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
    }

    impl Model {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::default(),
                fail_on: None,
                in_flight: AtomicUsize::new(0),
                max_in_flight: AtomicUsize::new(0),
            })
        }
    }

    #[async_trait]
    impl LlmProvider for Model {
        fn name(&self) -> &str {
            "fake"
        }

        async fn complete(&self, request: &CompletionRequest) -> Result<Completion, LlmError> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            self.requests.lock().unwrap().push(request.clone());
            let input = request.messages[0]
                .content
                .iter()
                .find_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            if self.fail_on.is_some_and(|f| input.contains(f)) {
                return Err(LlmError::new(LlmErrorKind::Unavailable, "overloaded"));
            }
            let (content, stop) = if input.contains("issues") {
                (
                    vec![ContentBlock::ToolUse {
                        id: "t".into(),
                        name: "list_issues".into(),
                        input: JsonValue(json!({})),
                    }],
                    StopReason::ToolUse,
                )
            } else if input.contains("which repo") {
                (
                    vec![ContentBlock::text("This is a/b, a small project.")],
                    StopReason::EndTurn,
                )
            } else {
                (vec![ContentBlock::text("Hi!")], StopReason::EndTurn)
            };
            Ok(Completion {
                model: "m".into(),
                content,
                stop_reason: stop,
                usage: Usage {
                    input_tokens: 100,
                    output_tokens: 10,
                    ..Usage::default()
                },
            })
        }

        async fn stream(
            &self,
            request: &CompletionRequest,
            _on_event: &mut (dyn FnMut(StreamEvent) + Send),
        ) -> Result<Completion, LlmError> {
            self.complete(request).await
        }
    }

    async fn evaluate(
        model: &Model,
        variant: &Variant,
        cancel: &CancellationToken,
    ) -> (VariantResult, Vec<String>) {
        let seen = Mutex::new(Vec::new());
        let result = evaluate_variant(model, "m", &tools(), &suite(), variant, cancel, &|r| {
            seen.lock().unwrap().push(r.case_id.clone());
        })
        .await;
        (result, seen.into_inner().unwrap())
    }

    #[tokio::test]
    async fn checks_each_case_against_what_the_model_did() {
        let model = Model::new();
        let (result, seen) = evaluate(&model, &baseline(), &CancellationToken::new()).await;
        assert_eq!((result.passed, result.total), (3, 3));
        assert!((result.accuracy() - 1.0).abs() < f64::EPSILON);
        assert_eq!(
            result
                .results
                .iter()
                .map(|r| r.case_id.as_str())
                .collect::<Vec<_>>(),
            ["c1", "c2", "c3"]
        );
        assert_eq!(
            result.results[0].called_tool.as_deref(),
            Some("list_issues")
        );
        assert_eq!(result.results[2].answer, "This is a/b, a small project.");
        // The answer check ignores case; every case is reported once.
        assert_eq!(seen.len(), 3);
        // Tokens are the exact numbers the provider reported.
        assert_eq!((result.input_tokens, result.output_tokens), (300, 30));
    }

    #[tokio::test]
    async fn wrong_tools_unexpected_tools_and_missing_text_fail() {
        let model = Model::new();
        let mut s = suite();
        s.cases = vec![
            case(
                "wrong",
                "open issues?",
                Expectation::Tool {
                    name: "get.issue".into(),
                },
            ),
            case("none", "open issues?", Expectation::NoTool),
            case(
                "text",
                "open issues?",
                Expectation::Answer {
                    contains: "issues".into(),
                },
            ),
            case(
                "missing",
                "hello",
                Expectation::Answer {
                    contains: "goodbye".into(),
                },
            ),
        ];
        let result = evaluate_variant(
            &*model,
            "m",
            &tools(),
            &s,
            &baseline(),
            &CancellationToken::new(),
            &|_| {},
        )
        .await;
        assert_eq!(result.passed, 0);
        assert_eq!(
            result.results[0].called_tool.as_deref(),
            Some("list_issues")
        );
        assert!(result.results.iter().all(|r| r.error.is_none()));
    }

    #[tokio::test]
    async fn the_variant_changes_what_the_model_is_given() {
        let model = Model::new();
        let variant = Variant {
            id: "v1".into(),
            label: "clearer".into(),
            system_prompt: Some("Always use tools.".into()),
            tool_descriptions: BTreeMap::from([(
                "list_issues".into(),
                "Lists the open issues of a repository".into(),
            )]),
        };
        let (result, _) = evaluate(&model, &variant, &CancellationToken::new()).await;
        let base = evaluate(&Model::new(), &baseline(), &CancellationToken::new())
            .await
            .0;
        let requests = model.requests.lock().unwrap();
        assert!(requests
            .iter()
            .all(|r| r.system.as_deref() == Some("Always use tools.")));
        let tools_seen = &requests[0].tools;
        assert_eq!(
            tools_seen
                .iter()
                .find(|t| t.name == "list_issues")
                .unwrap()
                .description,
            "Lists the open issues of a repository"
        );
        // Other tools keep their description, and names the model API rejects are made valid.
        assert_eq!(
            tools_seen
                .iter()
                .find(|t| t.name == "get_issue")
                .unwrap()
                .description,
            "Gets one issue"
        );
        assert!(requests
            .iter()
            .all(|r| r.temperature == Some(0.0) && r.messages.len() == 1));
        // The estimate of the definitions follows the variant's descriptions.
        assert!(result.definition_tokens > base.definition_tokens);
    }

    #[tokio::test]
    async fn the_suites_prompt_is_used_when_the_variant_has_none() {
        let model = Model::new();
        evaluate(&model, &baseline(), &CancellationToken::new()).await;
        assert!(model
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.system.as_deref() == Some("Be brief.")));
    }

    #[tokio::test]
    async fn names_the_model_may_call_map_back_to_the_real_tool_names() {
        let names = model_names(&[tool("get.issue", "x"), tool("get_issue", "y")]);
        assert_eq!(names.len(), 2);
        assert_eq!(names.values().collect::<Vec<_>>().len(), 2);
        assert!(names.keys().all(|k| k
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')));
        assert!(names.values().any(|v| v == "get.issue"));
    }

    #[tokio::test]
    async fn a_failing_call_fails_only_its_case_and_keeps_the_error() {
        let model = Arc::new(Model {
            fail_on: Some("hello"),
            ..Arc::try_unwrap(Model::new()).ok().unwrap()
        });
        let (result, _) = evaluate(&model, &baseline(), &CancellationToken::new()).await;
        assert_eq!((result.passed, result.total), (2, 3));
        let failed = &result.results[1];
        assert!(!failed.passed);
        assert_eq!(failed.error.as_deref(), Some("overloaded"));
        assert_eq!(failed.input_tokens, 0);
    }

    #[tokio::test]
    async fn runs_several_cases_at_once_but_not_unboundedly() {
        let model = Model::new();
        let mut s = suite();
        s.cases = (0..12)
            .map(|i| case(&format!("c{i}"), "hello", Expectation::NoTool))
            .collect();
        evaluate_variant(
            &*model,
            "m",
            &tools(),
            &s,
            &baseline(),
            &CancellationToken::new(),
            &|_| {},
        )
        .await;
        let peak = model.max_in_flight.load(Ordering::SeqCst);
        assert!(peak > 1 && peak <= PARALLEL_CASES, "peak {peak}");
    }

    #[tokio::test]
    async fn cancelling_marks_the_cases_that_did_not_start() {
        let model = Model::new();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let (result, _) = evaluate(&model, &baseline(), &cancel).await;
        assert_eq!(result.passed, 0);
        assert!(result
            .results
            .iter()
            .all(|r| r.error.as_deref() == Some("cancelled")));
        assert!(model.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_empty_suite_has_no_accuracy() {
        let model = Model::new();
        let mut s = suite();
        s.cases.clear();
        let result = evaluate_variant(
            &*model,
            "m",
            &tools(),
            &s,
            &baseline(),
            &CancellationToken::new(),
            &|_| {},
        )
        .await;
        assert_eq!((result.total, result.accuracy()), (0, 0.0));
    }

    #[test]
    fn reads_variants_from_json_in_a_code_fence_or_prose() {
        let text = "Sure! Here you go:\n```json\n{\"variants\": [\n {\"label\": \"Clearer\", \"systemPrompt\": null, \"toolDescriptions\": {\"list_issues\": \"  Lists open issues of a repo  \"}},\n {\"label\": \"Prompt only\", \"systemPrompt\": \"Use tools.\", \"toolDescriptions\": {}}\n]}\n```\nHope this helps.";
        let variants = parse_proposals(text, &tools(), 5).unwrap();
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].id, "v1");
        assert_eq!(variants[0].label, "Clearer");
        assert_eq!(
            variants[0].tool_descriptions["list_issues"],
            "Lists open issues of a repo"
        );
        assert_eq!(variants[1].system_prompt.as_deref(), Some("Use tools."));
    }

    #[test]
    fn drops_what_cannot_be_used() {
        let long = "x".repeat(MAX_PROPOSED_DESCRIPTION + 1);
        let text = format!(
            "{{\"variants\": [\
              {{\"label\": \"unknown tool\", \"toolDescriptions\": {{\"ghost\": \"d\"}}}},\
              {{\"label\": \"empty\", \"toolDescriptions\": {{\"list_issues\": \"  \"}}}},\
              {{\"label\": \"too long\", \"toolDescriptions\": {{\"list_issues\": \"{long}\"}}}},\
              {{\"toolDescriptions\": {{\"list_issues\": \"Good one\", \"ghost\": \"dropped\"}}}},\
              {{\"label\": \"second\", \"systemPrompt\": \"p\"}}\
            ]}}"
        );
        let variants = parse_proposals(&text, &tools(), 5).unwrap();
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].label, "Variant 1");
        assert_eq!(variants[0].tool_descriptions.len(), 1);
        assert_eq!(variants[1].id, "v2");
        // At most `max` variants.
        let capped = parse_proposals(&text, &tools(), 1).unwrap();
        assert_eq!(capped.len(), 1);
    }

    #[test]
    fn explains_answers_that_are_not_usable() {
        for (text, expected) in [
            ("no json here", "did not answer with JSON"),
            ("{\"other\": 1}", "no list of variants"),
            ("{\"variants\": []}", "no usable variant"),
            ("{\"variants\": [{\"label\": \"x\"}]}", "no usable variant"),
        ] {
            let error = parse_proposals(text, &tools(), 3).unwrap_err().to_string();
            assert!(error.contains(expected), "{text}: {error}");
        }
    }

    #[tokio::test]
    async fn proposing_asks_the_model_with_the_tools_and_the_failing_cases() {
        struct Proposer(Mutex<Option<CompletionRequest>>);
        #[async_trait]
        impl LlmProvider for Proposer {
            fn name(&self) -> &str {
                "p"
            }
            async fn complete(&self, request: &CompletionRequest) -> Result<Completion, LlmError> {
                *self.0.lock().unwrap() = Some(request.clone());
                Ok(Completion {
                    model: "m".into(),
                    content: vec![ContentBlock::text("{\"variants\": [{\"label\": \"A\", \"toolDescriptions\": {\"list_issues\": \"Lists open issues\"}}]}")],
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                })
            }
            async fn stream(
                &self,
                r: &CompletionRequest,
                _: &mut (dyn FnMut(StreamEvent) + Send),
            ) -> Result<Completion, LlmError> {
                self.complete(r).await
            }
        }
        let provider = Proposer(Mutex::new(None));
        let failing = CaseResult {
            case_id: "c1".into(),
            input: "open issues in a/b?".into(),
            expectation: Expectation::Tool {
                name: "list_issues".into(),
            },
            passed: false,
            called_tool: Some("get.issue".into()),
            answer: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            error: None,
        };
        let variants = propose_variants(&provider, "m", &tools(), &suite(), &[failing], 3)
            .await
            .unwrap();
        assert_eq!(variants.len(), 1);
        let request = provider.0.lock().unwrap().clone().unwrap();
        let prompt = match &request.messages[0].content[0] {
            ContentBlock::Text { text } => text.clone(),
            _ => String::new(),
        };
        assert!(prompt.contains("Propose 3 different variants"), "{prompt}");
        assert!(prompt.contains("list_issues") && prompt.contains("get.issue"));
        assert!(
            prompt.contains("open issues in a/b?")
                && prompt.contains("the model calls the tool list_issues")
        );
        assert!(prompt.contains("\"Be brief.\""));
        assert!(request
            .system
            .unwrap()
            .contains("never change what a tool does"));
        assert!(
            request.tools.is_empty(),
            "tools are described in the text, not offered"
        );
        // Without failing cases the model is asked to look for vague descriptions.
        let none = proposal_prompt(&tools(), &suite(), &[], 2);
        assert!(none.contains("handles all test cases"));
    }
}
