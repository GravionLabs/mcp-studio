//! The prompt-flow graph and its validation (see `docs/specs/flows.md`).
//!
//! A flow is an ordered list of steps. Data dependencies are implicit: a step depends on every
//! `steps.<id>` its templates mention, and those dependencies must form a DAG. Validation never
//! stops at the first problem; it reports every issue so the editor can show them all at once.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::{explorer::ToolInfo, model::JsonValue};

/// The only flow format version this build understands.
pub const FLOW_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Flow {
    pub version: u32,
    pub name: String,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub id: String,
    #[serde(flatten)]
    pub kind: StepKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum StepKind {
    /// Declares the flow's input variables, available as `inputs.<name>`.
    Input {
        #[serde(default)]
        inputs: BTreeMap<String, InputDecl>,
    },
    /// Calls a model; may expose MCP tools to it (agent step).
    Llm {
        model: String,
        prompt: String,
        #[serde(default)]
        system: Option<String>,
        #[serde(default)]
        tools: Vec<ToolRef>,
    },
    /// Calls one MCP tool with templated arguments.
    Tool {
        server: String,
        tool: String,
        #[serde(default)]
        arguments: BTreeMap<String, JsonValue>,
    },
    /// Branches on an expression; `then` / `else` name the steps to continue with.
    Condition {
        expression: String,
        #[serde(default)]
        then: Option<String>,
        #[serde(default, rename = "else")]
        otherwise: Option<String>,
    },
    /// Maps or extracts values; each entry is a template.
    Transform {
        #[serde(default)]
        values: BTreeMap<String, String>,
    },
    /// Declares the flow's result; each entry is a template.
    Output {
        #[serde(default)]
        outputs: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InputDecl {
    /// JSON Schema primitive type name: `string`, `number`, `integer`, `boolean`, `array`, `object`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// A tool exposed to an `llm` step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolRef {
    pub server: String,
    pub tool: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum FlowIssueCode {
    UnsupportedVersion,
    EmptyName,
    InvalidStepId,
    DuplicateStepId,
    MultipleInputSteps,
    MissingOutputStep,
    EmptyField,
    UnknownServer,
    UnknownTool,
    MissingArgument,
    UnknownArgument,
    ArgumentType,
    UnknownInput,
    UnknownStep,
    Cycle,
    MalformedTemplate,
    /// A condition jumps to itself or to an earlier step; jumps only go forward.
    BackwardJump,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FlowIssue {
    /// The offending step; `None` for problems with the flow itself.
    pub step_id: Option<String>,
    pub code: FlowIssueCode,
    pub message: String,
}

/// What each server offers, keyed by the server name used in `tool` steps.
pub type ToolCatalog = BTreeMap<String, Vec<ToolInfo>>;

/// Checks a flow against the servers and tools that are available. An empty result means the flow
/// can run.
///
/// Argument schemas are checked at the top level: required properties are present, unknown ones are
/// rejected when the schema forbids additional properties, and literal values match the declared
/// type. Values containing `{{ ... }}` are only known at run time and are not type-checked.
pub fn validate(flow: &Flow, catalog: &ToolCatalog) -> Vec<FlowIssue> {
    let mut v = Validator {
        catalog,
        issues: Vec::new(),
    };
    v.flow(flow);
    v.issues
}

struct Validator<'a> {
    catalog: &'a ToolCatalog,
    issues: Vec<FlowIssue>,
}

impl Validator<'_> {
    fn issue(&mut self, step: Option<&str>, code: FlowIssueCode, message: impl Into<String>) {
        self.issues.push(FlowIssue {
            step_id: step.map(str::to_owned),
            code,
            message: message.into(),
        });
    }

    fn flow(&mut self, flow: &Flow) {
        if flow.version != FLOW_VERSION {
            self.issue(
                None,
                FlowIssueCode::UnsupportedVersion,
                format!(
                    "flow version {} is not supported (expected {FLOW_VERSION})",
                    flow.version
                ),
            );
        }
        if flow.name.trim().is_empty() {
            self.issue(None, FlowIssueCode::EmptyName, "the flow has no name");
        }

        let mut ids = BTreeSet::new();
        for step in &flow.steps {
            if !is_identifier(&step.id) {
                self.issue(
                    Some(&step.id),
                    FlowIssueCode::InvalidStepId,
                    format!(
                        "step id {:?} must start with a letter or underscore and contain only letters, digits, and underscores",
                        step.id
                    ),
                );
            }
            if !ids.insert(step.id.as_str()) {
                self.issue(
                    Some(&step.id),
                    FlowIssueCode::DuplicateStepId,
                    format!("step id {:?} is used more than once", step.id),
                );
            }
        }

        let input_steps: Vec<_> = flow
            .steps
            .iter()
            .filter(|s| matches!(s.kind, StepKind::Input { .. }))
            .collect();
        if input_steps.len() > 1 {
            self.issue(
                Some(&input_steps[1].id),
                FlowIssueCode::MultipleInputSteps,
                "a flow can have at most one input step",
            );
        }
        if !flow
            .steps
            .iter()
            .any(|s| matches!(s.kind, StepKind::Output { .. }))
        {
            self.issue(
                None,
                FlowIssueCode::MissingOutputStep,
                "the flow has no output step",
            );
        }
        let inputs: BTreeSet<&str> = input_steps
            .iter()
            .filter_map(|s| match &s.kind {
                StepKind::Input { inputs } => Some(inputs.keys().map(String::as_str)),
                _ => None,
            })
            .flatten()
            .collect();

        let mut deps: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for step in &flow.steps {
            self.step(step, &ids, &inputs, deps.entry(&step.id).or_default());
        }
        self.cycles(&deps);
        // A condition may only jump forward, so a flow cannot loop forever.
        let positions: BTreeMap<&str, usize> = flow
            .steps
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.as_str(), i))
            .collect();
        for (index, step) in flow.steps.iter().enumerate() {
            if let StepKind::Condition {
                then, otherwise, ..
            } = &step.kind
            {
                for (label, target) in [("then", then), ("else", otherwise)] {
                    let backward = target
                        .as_deref()
                        .and_then(|t| positions.get(t))
                        .is_some_and(|position| *position <= index);
                    if backward {
                        self.issue(
                            Some(&step.id),
                            FlowIssueCode::BackwardJump,
                            format!("`{label}` must point to a step after this one"),
                        );
                    }
                }
            }
        }
    }

    fn step<'f>(
        &mut self,
        step: &'f Step,
        ids: &BTreeSet<&'f str>,
        inputs: &BTreeSet<&str>,
        deps: &mut BTreeSet<&'f str>,
    ) {
        let id = step.id.as_str();
        let mut texts: Vec<Text<'_>> = Vec::new();
        match &step.kind {
            StepKind::Input { inputs } => {
                for (name, decl) in inputs {
                    if !is_identifier(name) {
                        self.issue(
                            Some(id),
                            FlowIssueCode::EmptyField,
                            format!("input name {name:?} is not a valid identifier"),
                        );
                    }
                    if !is_json_type(&decl.kind) {
                        self.issue(
                            Some(id),
                            FlowIssueCode::ArgumentType,
                            format!("input {name:?} has unknown type {:?}", decl.kind),
                        );
                    }
                }
            }
            StepKind::Llm {
                model,
                prompt,
                system,
                tools,
            } => {
                self.non_empty(id, "model", model);
                self.non_empty(id, "prompt", prompt);
                texts.push(Text::Template(prompt));
                if let Some(system) = system {
                    texts.push(Text::Template(system));
                }
                for tool in tools {
                    self.find_tool(id, &tool.server, &tool.tool);
                }
            }
            StepKind::Tool {
                server,
                tool,
                arguments,
            } => {
                if let Some(info) = self.find_tool(id, server, tool) {
                    self.arguments(id, &info, arguments);
                }
                for value in arguments.values() {
                    collect_strings(&value.0, &mut texts);
                }
            }
            StepKind::Condition {
                expression,
                then,
                otherwise,
            } => {
                self.non_empty(id, "expression", expression);
                texts.push(if expression.contains("{{") {
                    Text::Template(expression)
                } else {
                    Text::Expression(expression)
                });
                for (label, target) in [("then", then), ("else", otherwise)] {
                    if let Some(target) = target {
                        if !ids.contains(target.as_str()) {
                            self.issue(
                                Some(id),
                                FlowIssueCode::UnknownStep,
                                format!("`{label}` points to unknown step {target:?}"),
                            );
                        }
                    }
                }
            }
            StepKind::Transform { values } => {
                texts.extend(values.values().map(|t| Text::Template(t)));
            }
            StepKind::Output { outputs } => {
                texts.extend(outputs.values().map(|t| Text::Template(t)));
            }
        }

        for text in texts {
            match text.references() {
                Ok(refs) => {
                    for reference in refs {
                        match reference {
                            Reference::Input(name) if !inputs.contains(name.as_str()) => self
                                .issue(
                                    Some(id),
                                    FlowIssueCode::UnknownInput,
                                    format!("`inputs.{name}` is not declared by an input step"),
                                ),
                            Reference::Step(other) => match ids.get(other.as_str()) {
                                Some(&known) => {
                                    deps.insert(known);
                                }
                                None => self.issue(
                                    Some(id),
                                    FlowIssueCode::UnknownStep,
                                    format!("`steps.{other}` refers to a step that does not exist"),
                                ),
                            },
                            Reference::Input(_) => {}
                        }
                    }
                }
                Err(message) => self.issue(Some(id), FlowIssueCode::MalformedTemplate, message),
            }
        }
    }

    fn non_empty(&mut self, step: &str, field: &str, value: &str) {
        if value.trim().is_empty() {
            self.issue(
                Some(step),
                FlowIssueCode::EmptyField,
                format!("`{field}` must not be empty"),
            );
        }
    }

    /// Looks a tool up in the catalog, reporting an unknown server or tool.
    fn find_tool(&mut self, step: &str, server: &str, tool: &str) -> Option<ToolInfo> {
        let Some(tools) = self.catalog.get(server) else {
            self.issue(
                Some(step),
                FlowIssueCode::UnknownServer,
                format!("server {server:?} does not exist"),
            );
            return None;
        };
        let found = tools.iter().find(|t| t.name == tool).cloned();
        if found.is_none() {
            self.issue(
                Some(step),
                FlowIssueCode::UnknownTool,
                format!("server {server:?} has no tool {tool:?}"),
            );
        }
        found
    }

    fn arguments(&mut self, step: &str, tool: &ToolInfo, arguments: &BTreeMap<String, JsonValue>) {
        let schema = &tool.input_schema.0;
        let properties = schema.get("properties").and_then(Value::as_object);
        let required = schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str);
        for name in required {
            if !arguments.contains_key(name) {
                self.issue(
                    Some(step),
                    FlowIssueCode::MissingArgument,
                    format!("tool {:?} requires argument {name:?}", tool.name),
                );
            }
        }
        let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
        for (name, value) in arguments {
            match properties.and_then(|p| p.get(name)) {
                None if closed => self.issue(
                    Some(step),
                    FlowIssueCode::UnknownArgument,
                    format!("tool {:?} has no argument {name:?}", tool.name),
                ),
                None => {}
                Some(property) => {
                    if is_runtime_value(&value.0) {
                        continue;
                    }
                    if let Some(expected) = declared_types(property) {
                        if !expected.iter().any(|ty| matches_type(&value.0, ty)) {
                            self.issue(
                                Some(step),
                                FlowIssueCode::ArgumentType,
                                format!(
                                    "argument {name:?} must be {}, got {}",
                                    expected.join(" or "),
                                    type_name(&value.0)
                                ),
                            );
                        }
                    }
                }
            }
        }
    }

    fn cycles(&mut self, deps: &BTreeMap<&str, BTreeSet<&str>>) {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            Visiting,
            Done,
        }
        fn visit<'a>(
            node: &'a str,
            deps: &BTreeMap<&'a str, BTreeSet<&'a str>>,
            marks: &mut BTreeMap<&'a str, Mark>,
            cyclic: &mut Vec<&'a str>,
        ) {
            marks.insert(node, Mark::Visiting);
            for &next in deps.get(node).into_iter().flatten() {
                match marks.get(next) {
                    Some(Mark::Visiting) => cyclic.push(node),
                    Some(Mark::Done) => {}
                    None => visit(next, deps, marks, cyclic),
                }
            }
            marks.insert(node, Mark::Done);
        }

        let mut marks = BTreeMap::new();
        let mut cyclic = Vec::new();
        for &node in deps.keys() {
            if !marks.contains_key(node) {
                visit(node, deps, &mut marks, &mut cyclic);
            }
        }
        for node in cyclic {
            self.issue(
                Some(node),
                FlowIssueCode::Cycle,
                format!("step {node:?} is part of a dependency cycle"),
            );
        }
    }
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_json_type(s: &str) -> bool {
    matches!(
        s,
        "string" | "number" | "integer" | "boolean" | "array" | "object" | "null"
    )
}

fn is_runtime_value(value: &Value) -> bool {
    value.as_str().is_some_and(|s| s.contains("{{"))
}

/// The `type` of a JSON Schema property as a list; `None` when the schema does not constrain it.
fn declared_types(property: &Value) -> Option<Vec<&str>> {
    match property.get("type")? {
        Value::String(ty) => Some(vec![ty.as_str()]),
        Value::Array(types) => Some(types.iter().filter_map(Value::as_str).collect()),
        _ => None,
    }
}

fn matches_type(value: &Value, ty: &str) -> bool {
    match ty {
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        _ => true,
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn collect_strings<'a>(value: &'a Value, out: &mut Vec<Text<'a>>) {
    match value {
        Value::String(s) => out.push(Text::Template(s)),
        Value::Array(items) => items.iter().for_each(|v| collect_strings(v, out)),
        Value::Object(map) => map.values().for_each(|v| collect_strings(v, out)),
        _ => {}
    }
}

/// A piece of text that may refer to inputs and earlier steps.
enum Text<'a> {
    /// Free text where only `{{ ... }}` parts are expressions.
    Template(&'a str),
    /// A bare expression, such as a condition.
    Expression(&'a str),
}

#[derive(Debug, PartialEq, Eq)]
enum Reference {
    Input(String),
    Step(String),
}

impl Text<'_> {
    fn references(&self) -> Result<Vec<Reference>, String> {
        match self {
            Text::Expression(expr) => Ok(scan_references(expr)),
            Text::Template(text) => {
                let mut refs = Vec::new();
                let mut rest = *text;
                while let Some(start) = rest.find("{{") {
                    let after = &rest[start + 2..];
                    let end = after
                        .find("}}")
                        .ok_or_else(|| format!("unclosed `{{{{` in {text:?}"))?;
                    refs.extend(scan_references(&after[..end]));
                    rest = &after[end + 2..];
                }
                Ok(refs)
            }
        }
    }
}

/// Finds `inputs.<name>` and `steps.<id>` paths in an expression, ignoring quoted strings and
/// paths that are a member of something else (`foo.steps.x`).
fn scan_references(expr: &str) -> Vec<Reference> {
    let chars: Vec<char> = expr.chars().collect();
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut refs = Vec::new();
    let mut i = 0;
    let mut previous_dot = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' || c == '\'' {
            i += 1;
            while i < chars.len() && chars[i] != c {
                i += 1;
            }
            i += 1;
            previous_dot = false;
        } else if is_word(c) {
            let start = i;
            while i < chars.len() && is_word(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if !previous_dot && (word == "inputs" || word == "steps") && chars.get(i) == Some(&'.')
            {
                let name_start = i + 1;
                let mut j = name_start;
                while j < chars.len() && is_word(chars[j]) {
                    j += 1;
                }
                if j > name_start {
                    let name: String = chars[name_start..j].iter().collect();
                    refs.push(if word == "inputs" {
                        Reference::Input(name)
                    } else {
                        Reference::Step(name)
                    });
                }
            }
            previous_dot = false;
        } else {
            previous_dot = c == '.';
            i += 1;
        }
    }
    refs
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn tool(name: &str, schema: Value) -> ToolInfo {
        ToolInfo {
            name: name.to_owned(),
            title: None,
            description: None,
            input_schema: JsonValue(schema),
            output_schema: None,
            annotations: None,
        }
    }

    fn catalog() -> ToolCatalog {
        BTreeMap::from([(
            "github".to_owned(),
            vec![tool(
                "list_issues",
                json!({
                    "type": "object",
                    "properties": {
                        "repo": { "type": "string" },
                        "limit": { "type": "integer" },
                        "state": { "type": ["string", "null"] }
                    },
                    "required": ["repo"],
                    "additionalProperties": false
                }),
            )],
        )])
    }

    fn flow(value: Value) -> Flow {
        serde_json::from_value(value).expect("test flow must deserialize")
    }

    fn valid_flow() -> Value {
        json!({
            "version": 1,
            "name": "summarize",
            "steps": [
                { "id": "start", "type": "input", "inputs": { "repo": { "type": "string" } } },
                { "id": "issues", "type": "tool", "server": "github", "tool": "list_issues",
                  "arguments": { "repo": "{{ inputs.repo }}", "limit": 5, "state": null } },
                { "id": "summary", "type": "llm", "model": "claude-sonnet-5-5",
                  "prompt": "Summarize {{ steps.issues.result }}",
                  "tools": [{ "server": "github", "tool": "list_issues" }] },
                { "id": "check", "type": "condition", "expression": "steps.summary.text != ''",
                  "then": "pick", "else": "done" },
                { "id": "pick", "type": "transform", "values": { "text": "{{ steps.summary.text }}" } },
                { "id": "done", "type": "output", "outputs": { "summary": "{{ steps.pick.text }}" } }
            ]
        })
    }

    fn codes(flow: &Flow) -> Vec<FlowIssueCode> {
        validate(flow, &catalog())
            .into_iter()
            .map(|i| i.code)
            .collect()
    }

    #[test]
    fn a_valid_flow_with_every_step_type_has_no_issues() {
        assert_eq!(validate(&flow(valid_flow()), &catalog()), vec![]);
    }

    #[test]
    fn steps_round_trip_through_json() {
        let original = flow(valid_flow());
        let json = serde_json::to_value(&original).unwrap();
        assert_eq!(json["steps"][1]["type"], "tool");
        assert_eq!(json["steps"][3]["else"], "done");
        assert_eq!(serde_json::from_value::<Flow>(json).unwrap(), original);
    }

    #[test]
    fn unknown_step_type_is_rejected_when_parsing() {
        let result = serde_json::from_value::<Flow>(json!({
            "version": 1, "name": "x", "steps": [{ "id": "a", "type": "loop" }]
        }));
        assert!(result.is_err());
    }

    #[test]
    fn rejects_unsupported_version_and_empty_name() {
        let mut f = flow(valid_flow());
        f.version = 2;
        f.name = "  ".into();
        assert_eq!(
            codes(&f),
            [FlowIssueCode::UnsupportedVersion, FlowIssueCode::EmptyName]
        );
    }

    #[test]
    fn rejects_duplicate_and_invalid_step_ids() {
        let mut f = flow(valid_flow());
        f.steps[4].id = "pick up".into();
        f.steps[5].id = "issues".into();
        let found = codes(&f);
        assert!(found.contains(&FlowIssueCode::InvalidStepId));
        assert!(found.contains(&FlowIssueCode::DuplicateStepId));
    }

    #[test]
    fn requires_an_output_step_and_at_most_one_input_step() {
        let f = flow(json!({
            "version": 1, "name": "x",
            "steps": [
                { "id": "a", "type": "input" },
                { "id": "b", "type": "input" }
            ]
        }));
        assert_eq!(
            codes(&f),
            [
                FlowIssueCode::MultipleInputSteps,
                FlowIssueCode::MissingOutputStep
            ]
        );
    }

    #[test]
    fn reports_unknown_server_and_tool() {
        let mut f = flow(valid_flow());
        let StepKind::Tool { server, .. } = &mut f.steps[1].kind else {
            unreachable!()
        };
        *server = "gitlab".into();
        let StepKind::Llm { tools, .. } = &mut f.steps[2].kind else {
            unreachable!()
        };
        tools[0].tool = "delete_everything".into();
        assert_eq!(
            codes(&f),
            [FlowIssueCode::UnknownServer, FlowIssueCode::UnknownTool]
        );
    }

    #[test]
    fn reports_missing_unknown_and_mistyped_arguments() {
        let f = flow(json!({
            "version": 1, "name": "x",
            "steps": [
                { "id": "t", "type": "tool", "server": "github", "tool": "list_issues",
                  "arguments": { "limit": "five", "bogus": true, "state": 3 } },
                { "id": "o", "type": "output" }
            ]
        }));
        let issues = validate(&f, &catalog());
        let found: Vec<_> = issues.iter().map(|i| i.code).collect();
        assert_eq!(
            found,
            [
                FlowIssueCode::MissingArgument,
                FlowIssueCode::UnknownArgument,
                FlowIssueCode::ArgumentType,
                FlowIssueCode::ArgumentType,
            ]
        );
        assert!(issues[2].message.contains("integer"), "{:?}", issues[2]);
        assert!(
            issues[3].message.contains("string or null"),
            "{:?}",
            issues[3]
        );
    }

    #[test]
    fn templated_argument_values_are_not_type_checked() {
        let f = flow(json!({
            "version": 1, "name": "x",
            "steps": [
                { "id": "t", "type": "tool", "server": "github", "tool": "list_issues",
                  "arguments": { "repo": "r", "limit": "{{ 1 + 1 }}" } },
                { "id": "o", "type": "output" }
            ]
        }));
        assert_eq!(codes(&f), []);
    }

    #[test]
    fn reports_undeclared_inputs_and_unknown_steps() {
        let f = flow(json!({
            "version": 1, "name": "x",
            "steps": [
                { "id": "start", "type": "input", "inputs": { "repo": { "type": "string" } } },
                { "id": "a", "type": "transform",
                  "values": { "x": "{{ inputs.branch }}", "y": "{{ steps.ghost.result }}" } },
                { "id": "c", "type": "condition", "expression": "true", "then": "nowhere" },
                { "id": "o", "type": "output" }
            ]
        }));
        assert_eq!(
            codes(&f),
            [
                FlowIssueCode::UnknownInput,
                FlowIssueCode::UnknownStep,
                FlowIssueCode::UnknownStep
            ]
        );
    }

    #[test]
    fn detects_dependency_cycles_including_self_references() {
        let f = flow(json!({
            "version": 1, "name": "x",
            "steps": [
                { "id": "a", "type": "transform", "values": { "v": "{{ steps.b.v }}" } },
                { "id": "b", "type": "transform", "values": { "v": "{{ steps.a.v }}" } },
                { "id": "c", "type": "transform", "values": { "v": "{{ steps.c.v }}" } },
                { "id": "o", "type": "output" }
            ]
        }));
        let cyclic: Vec<_> = validate(&f, &catalog())
            .into_iter()
            .filter(|i| i.code == FlowIssueCode::Cycle)
            .filter_map(|i| i.step_id)
            .collect();
        assert_eq!(cyclic, ["b", "c"]);
    }

    #[test]
    fn reports_malformed_templates_and_empty_fields() {
        let f = flow(json!({
            "version": 1, "name": "x",
            "steps": [
                { "id": "l", "type": "llm", "model": "", "prompt": "hi {{ steps.l" },
                { "id": "o", "type": "output" }
            ]
        }));
        assert_eq!(
            codes(&f),
            [FlowIssueCode::EmptyField, FlowIssueCode::MalformedTemplate]
        );
    }

    #[test]
    fn reference_scanner_ignores_quotes_and_member_access() {
        assert_eq!(
            scan_references("steps.a.x == 'steps.b' and foo.steps.c and inputs.n > 1"),
            [Reference::Step("a".into()), Reference::Input("n".into())]
        );
    }

    #[test]
    fn issues_serialize_in_camel_case() {
        let issue = FlowIssue {
            step_id: Some("a".into()),
            code: FlowIssueCode::UnknownTool,
            message: "m".into(),
        };
        let json = serde_json::to_value(issue).unwrap();
        assert_eq!(json["stepId"], "a");
        assert_eq!(json["code"], "unknownTool");
    }

    #[test]
    fn conditions_may_only_jump_forward() {
        let f = flow(json!({
            "version": 1, "name": "x",
            "steps": [
                { "id": "a", "type": "transform" },
                { "id": "c", "type": "condition", "expression": "true", "then": "a", "else": "c" },
                { "id": "ok", "type": "condition", "expression": "true", "then": "o" },
                { "id": "o", "type": "output" }
            ]
        }));
        let issues = validate(&f, &catalog());
        let backward: Vec<_> = issues
            .iter()
            .filter(|i| i.code == FlowIssueCode::BackwardJump)
            .collect();
        assert_eq!(backward.len(), 2);
        assert!(backward.iter().all(|i| i.step_id.as_deref() == Some("c")));
        assert!(backward[0].message.contains("`then`") && backward[1].message.contains("`else`"));
    }
}
