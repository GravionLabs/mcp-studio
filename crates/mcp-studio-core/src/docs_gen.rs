//! Markdown documentation for the tools of a server.
//!
//! The text is built from three sources and needs no model: the tool definitions (purpose and
//! parameters), the recorded calls in the history (examples), and the failed calls in the history
//! (error cases). Results in the history are already masked and size-limited when they are stored.
//! The same input always gives the same text, so the file can be kept in Git and diffed.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::{
    explorer::ToolInfo,
    flow_run::result_text,
    history::HistoryEntry,
    lint::{lint_tools, Severity},
};

/// Examples shown per tool.
pub const MAX_EXAMPLES: usize = 3;
/// Different error messages shown per tool.
pub const MAX_ERRORS: usize = 5;
/// A result is cut after this many characters.
pub const EXCERPT_CHARS: usize = 400;

pub struct DocsInput<'a> {
    pub server_name: &'a str,
    /// The date shown as the generation date, `YYYY-MM-DD`.
    pub generated_on: &'a str,
    pub tools: &'a [ToolInfo],
    /// Recorded calls of the server (any order); only `tools/call` entries are used.
    pub history: &'a [HistoryEntry],
}

/// `YYYY-MM-DD` (UTC) of a Unix time in milliseconds.
pub fn civil_date(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}")
}

fn cell(text: &str) -> String {
    text.replace('|', "\\|")
        .replace(['\n', '\r'], " ")
        .trim()
        .to_owned()
}

/// A code fence that the text cannot close early.
fn fenced(language: &str, text: &str) -> String {
    let mut fence = "```".to_owned();
    while text.contains(&fence) {
        fence.push('`');
    }
    format!("{fence}{language}\n{}\n{fence}\n", text.trim_end())
}

fn excerpt(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= EXCERPT_CHARS {
        return text.to_owned();
    }
    let cut: String = text.chars().take(EXCERPT_CHARS).collect();
    format!("{}…", cut.trim_end())
}

fn anchor(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

// ---- parameters -------------------------------------------------------------------------------

struct ParamRow {
    name: String,
    kind: String,
    required: bool,
    description: String,
}

fn type_of(schema: &Value) -> String {
    if let Some(options) = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
    {
        let kinds: Vec<String> = options.iter().map(type_of).collect();
        return kinds.join(" | ");
    }
    match schema.get("type") {
        Some(Value::String(kind)) if kind == "array" => match schema.get("items") {
            Some(items) => format!("array<{}>", type_of(items)),
            None => "array".to_owned(),
        },
        Some(Value::String(kind)) => kind.clone(),
        Some(Value::Array(kinds)) => kinds
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" | "),
        _ if schema.get("enum").is_some() => "enum".to_owned(),
        _ => "any".to_owned(),
    }
}

fn details(schema: &Value) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        let list: Vec<String> = values.iter().map(|v| format!("`{}`", display(v))).collect();
        parts.push(format!("allowed: {}", list.join(", ")));
    }
    if let Some(default) = schema.get("default") {
        parts.push(format!("default: `{}`", display(default)));
    }
    let (min, max) = (schema.get("minimum"), schema.get("maximum"));
    match (min, max) {
        (Some(a), Some(b)) => parts.push(format!("range: {}–{}", display(a), display(b))),
        (Some(a), None) => parts.push(format!("at least {}", display(a))),
        (None, Some(b)) => parts.push(format!("at most {}", display(b))),
        _ => {}
    }
    if let Some(format) = schema.get("format").and_then(Value::as_str) {
        parts.push(format!("format: {format}"));
    }
    parts
}

fn display(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The parameters of an object schema; nested objects are listed as `parent.child` up to two levels.
fn parameter_rows(schema: &Value, prefix: &str, depth: usize, rows: &mut Vec<ParamRow>) {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    for (name, property) in properties {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        let mut description = property
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();
        let extra = details(property);
        if !extra.is_empty() {
            let joined = extra.join("; ");
            description = if description.is_empty() {
                joined
            } else {
                format!("{description} ({joined})")
            };
        }
        rows.push(ParamRow {
            name: path.clone(),
            kind: type_of(property),
            required: required.contains(&name.as_str()),
            description,
        });
        if depth < 2 && property.get("type").and_then(Value::as_str) == Some("object") {
            parameter_rows(property, &path, depth + 1, rows);
        }
    }
}

fn hints(tool: &ToolInfo) -> Vec<&'static str> {
    let Some(annotations) = tool.annotations.as_ref().map(|a| &a.0) else {
        return Vec::new();
    };
    let flag = |key: &str| annotations.get(key).and_then(Value::as_bool);
    let mut hints = Vec::new();
    if flag("readOnlyHint") == Some(true) {
        hints.push("read-only");
    }
    if flag("destructiveHint") == Some(true) {
        hints.push("may be destructive");
    }
    if flag("idempotentHint") == Some(true) {
        hints.push("repeating it has no further effect");
    }
    if flag("openWorldHint") == Some(true) {
        hints.push("talks to the outside world");
    }
    hints
}

// ---- recorded calls ---------------------------------------------------------------------------

struct Examples<'a> {
    good: Vec<&'a HistoryEntry>,
    /// Error message, how often it happened, and when it last happened.
    errors: Vec<(String, usize, i64)>,
}

fn error_message(entry: &HistoryEntry) -> String {
    let text = entry
        .error
        .clone()
        .or_else(|| entry.result.as_ref().map(|r| result_text(&r.0)))
        .unwrap_or_default();
    let first = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("The call failed");
    excerpt(first)
}

fn examples_of<'a>(tool: &str, history: &'a [HistoryEntry]) -> Examples<'a> {
    let mut calls: Vec<&HistoryEntry> = history
        .iter()
        .filter(|e| e.method == "tools/call" && e.target == tool && !e.cancelled)
        .collect();
    calls.sort_by(|a, b| b.ts.cmp(&a.ts).then(b.id.cmp(&a.id)));

    let mut good = Vec::new();
    let mut seen = Vec::new();
    let mut errors: BTreeMap<String, (usize, i64)> = BTreeMap::new();
    for entry in calls {
        if entry.is_error || entry.error.is_some() {
            let slot = errors.entry(error_message(entry)).or_insert((0, entry.ts));
            slot.0 += 1;
            slot.1 = slot.1.max(entry.ts);
        } else if entry.result.is_some() && good.len() < MAX_EXAMPLES {
            let key = entry.arguments.0.to_string();
            if !seen.contains(&key) {
                seen.push(key);
                good.push(entry);
            }
        }
    }
    let mut errors: Vec<(String, usize, i64)> =
        errors.into_iter().map(|(m, (n, ts))| (m, n, ts)).collect();
    errors.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(&b.0)));
    errors.truncate(MAX_ERRORS);
    Examples { good, errors }
}

// ---- the document -----------------------------------------------------------------------------

/// Renders the documentation as Markdown.
pub fn render_docs(input: &DocsInput) -> String {
    let calls = input
        .history
        .iter()
        .filter(|e| e.method == "tools/call")
        .count();
    let report = lint_tools(input.tools);
    let mut tools: Vec<&ToolInfo> = input.tools.iter().collect();
    tools.sort_by(|a, b| a.name.cmp(&b.name));

    let mut out = String::new();
    out.push_str(&format!("# {} tools\n\n", input.server_name));
    out.push_str(&format!(
        "_Generated by MCP Studio on {} from {} tool{} and {} recorded call{}._\n\n",
        input.generated_on,
        tools.len(),
        if tools.len() == 1 { "" } else { "s" },
        calls,
        if calls == 1 { "" } else { "s" },
    ));
    if tools.is_empty() {
        out.push_str("This server offers no tools.\n");
        return out;
    }

    out.push_str("## Overview\n\n");
    for tool in &tools {
        let summary = tool
            .description
            .as_deref()
            .and_then(|d| d.lines().map(str::trim).find(|l| !l.is_empty()))
            .map(|l| format!(" — {}", cell(l)))
            .unwrap_or_default();
        out.push_str(&format!(
            "- [`{}`](#{}){summary}\n",
            tool.name,
            anchor(&tool.name)
        ));
    }
    out.push('\n');

    for tool in tools {
        out.push_str(&format!("## `{}`\n\n", tool.name));
        if let Some(title) = tool.title.as_deref().filter(|t| !t.trim().is_empty()) {
            out.push_str(&format!("**{}**\n\n", title.trim()));
        }
        match tool
            .description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            Some(description) => out.push_str(&format!("{description}\n\n")),
            None => out.push_str("_No description is provided by the server._\n\n"),
        }
        let hint_list = hints(tool);
        if !hint_list.is_empty() {
            out.push_str(&format!(
                "Hints from the server: {}.\n\n",
                hint_list.join(", ")
            ));
        }

        out.push_str("### Parameters\n\n");
        let mut rows = Vec::new();
        parameter_rows(&tool.input_schema.0, "", 0, &mut rows);
        if rows.is_empty() {
            out.push_str("This tool takes no parameters.\n\n");
        } else {
            out.push_str(
                "| Parameter | Type | Required | Description |\n| --- | --- | --- | --- |\n",
            );
            for row in rows {
                out.push_str(&format!(
                    "| `{}` | {} | {} | {} |\n",
                    cell(&row.name),
                    cell(&row.kind),
                    if row.required { "yes" } else { "no" },
                    cell(&row.description),
                ));
            }
            out.push('\n');
        }

        if let Some(output) = tool
            .output_schema
            .as_ref()
            .map(|o| &o.0)
            .filter(|o| !o.is_null())
        {
            out.push_str("### Returns\n\n");
            out.push_str(&fenced(
                "json",
                &serde_json::to_string_pretty(output).unwrap_or_default(),
            ));
            out.push('\n');
        }

        let examples = examples_of(&tool.name, input.history);
        out.push_str("### Examples\n\n");
        if examples.good.is_empty() {
            out.push_str("_No successful call has been recorded yet._\n\n");
        }
        for entry in &examples.good {
            out.push_str(&format!("Call on {}:\n\n", civil_date(entry.ts)));
            let arguments = serde_json::to_string_pretty(&entry.arguments.0).unwrap_or_default();
            out.push_str(&fenced("json", &arguments));
            out.push('\n');
            if let Some(result) = &entry.result {
                let text = result_text(&result.0);
                let shown = if text.is_empty() {
                    result.0.to_string()
                } else {
                    text
                };
                out.push_str("Result:\n\n");
                out.push_str(&fenced("text", &excerpt(&shown)));
                out.push('\n');
            }
        }

        out.push_str("### Error cases\n\n");
        if examples.errors.is_empty() {
            out.push_str("_No failed call has been recorded._\n\n");
        } else {
            for (message, count, last) in &examples.errors {
                out.push_str(&format!(
                    "- {} — {} time{}, last on {}\n",
                    inline_code(message),
                    count,
                    if *count == 1 { "" } else { "s" },
                    civil_date(*last)
                ));
            }
            out.push('\n');
        }

        let gaps: Vec<&str> = report
            .findings
            .iter()
            .filter(|f| {
                f.tool.as_deref() == Some(tool.name.as_str()) && f.severity != Severity::Info
            })
            .map(|f| f.message.as_str())
            .collect();
        if !gaps.is_empty() {
            out.push_str("### Documentation gaps\n\n");
            for gap in gaps {
                out.push_str(&format!("- {gap}\n"));
            }
            out.push('\n');
        }
    }
    out.trim_end().to_owned() + "\n"
}

/// Inline code that cannot be broken by backticks in the text.
fn inline_code(text: &str) -> String {
    let single = text.replace(['\n', '\r'], " ");
    let mut ticks = "`".to_owned();
    while single.contains(&ticks) {
        ticks.push('`');
    }
    let pad = if single.starts_with('`') || single.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{ticks}{pad}{single}{pad}{ticks}")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::model::JsonValue;

    fn tool(name: &str, description: &str, schema: Value) -> ToolInfo {
        ToolInfo {
            name: name.into(),
            title: None,
            description: Some(description.into()),
            input_schema: JsonValue(schema),
            output_schema: None,
            annotations: None,
        }
    }

    fn call(
        id: i64,
        tool: &str,
        ts: i64,
        arguments: Value,
        result: Option<Value>,
        error: Option<&str>,
    ) -> HistoryEntry {
        HistoryEntry {
            id,
            server_id: "srv".into(),
            method: "tools/call".into(),
            target: tool.into(),
            arguments: JsonValue(arguments),
            is_error: error.is_some(),
            cancelled: false,
            duration_ms: Some(5),
            result: result.map(JsonValue),
            error: error.map(str::to_owned),
            ts,
        }
    }

    fn text_result(text: &str) -> Value {
        json!({ "content": [{ "type": "text", "text": text }] })
    }

    fn issues_tool() -> ToolInfo {
        tool(
            "list_issues",
            "Lists the open issues of a repository, newest first.\nReturns title and number.",
            json!({
                "type": "object",
                "properties": {
                    "repo": { "type": "string", "description": "owner/name | of the repository" },
                    "state": { "type": "string", "enum": ["open", "closed"], "default": "open" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "description": "How many" },
                    "labels": { "type": "array", "items": { "type": "string" } },
                    "filter": { "type": "object", "properties": { "author": { "type": "string", "description": "Login" } } },
                },
                "required": ["repo"],
            }),
        )
    }

    fn input<'a>(tools: &'a [ToolInfo], history: &'a [HistoryEntry]) -> DocsInput<'a> {
        DocsInput {
            server_name: "GitHub",
            generated_on: "2026-10-01",
            tools,
            history,
        }
    }

    #[test]
    fn dates_are_civil_dates_in_utc() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(1_700_000_000_000), "2023-11-14");
        assert_eq!(civil_date(951_782_400_000), "2000-02-29");
        assert_eq!(civil_date(1_790_812_800_000), "2026-10-01");
        assert_eq!(civil_date(-86_400_000), "1969-12-31");
    }

    #[test]
    fn documents_purpose_and_parameters() {
        let tools = [issues_tool()];
        let md = render_docs(&input(&tools, &[]));
        assert!(md.starts_with("# GitHub tools\n"), "{md}");
        assert!(md
            .contains("_Generated by MCP Studio on 2026-10-01 from 1 tool and 0 recorded calls._"));
        assert!(md.contains("- [`list_issues`](#list_issues) — Lists the open issues of a repository, newest first."));
        assert!(md.contains("## `list_issues`"));
        assert!(md.contains(
            "Lists the open issues of a repository, newest first.\nReturns title and number."
        ));
        assert!(
            md.contains("| `repo` | string | yes | owner/name \\| of the repository |"),
            "{md}"
        );
        assert!(
            md.contains("| `state` | string | no | allowed: `open`, `closed`; default: `open` |"),
            "{md}"
        );
        assert!(
            md.contains("| `limit` | integer | no | How many (range: 1–100) |"),
            "{md}"
        );
        assert!(md.contains("| `labels` | array<string> | no |  |"), "{md}");
        // Nested object parameters are listed with a dotted name.
        assert!(md.contains("| `filter` | object | no |  |"));
        assert!(
            md.contains("| `filter.author` | string | no | Login |"),
            "{md}"
        );
    }

    #[test]
    fn says_so_when_there_is_nothing_to_show() {
        let tools = [tool(
            "ping",
            "Checks that the server answers and returns its version",
            json!({ "type": "object" }),
        )];
        let md = render_docs(&input(&tools, &[]));
        assert!(md.contains("This tool takes no parameters."));
        assert!(md.contains("_No successful call has been recorded yet._"));
        assert!(md.contains("_No failed call has been recorded._"));
        let empty = render_docs(&input(&[], &[]));
        assert!(empty.contains("This server offers no tools."));
        let missing = [ToolInfo {
            description: None,
            ..tool("x", "", json!({}))
        }];
        assert!(render_docs(&input(&missing, &[]))
            .contains("_No description is provided by the server._"));
    }

    #[test]
    fn shows_recent_distinct_successful_calls_as_examples() {
        let tools = [issues_tool()];
        let history = [
            call(
                1,
                "list_issues",
                1_000,
                json!({ "repo": "a/b" }),
                Some(text_result("old")),
                None,
            ),
            call(
                2,
                "list_issues",
                2_000,
                json!({ "repo": "a/b" }),
                Some(text_result("newer same arguments")),
                None,
            ),
            call(
                3,
                "list_issues",
                3_000,
                json!({ "repo": "c/d", "limit": 5 }),
                Some(text_result("3 open")),
                None,
            ),
            call(
                4,
                "list_issues",
                4_000,
                json!({ "repo": "e/f" }),
                Some(text_result("x")),
                None,
            ),
            call(
                5,
                "list_issues",
                5_000,
                json!({ "repo": "g/h" }),
                Some(text_result("y")),
                None,
            ),
            call(
                6,
                "other_tool",
                6_000,
                json!({}),
                Some(text_result("not this tool")),
                None,
            ),
        ];
        let md = render_docs(&input(&tools, &history));
        // The three newest distinct calls; the same arguments are shown once, with the newest result.
        assert!(
            md.contains("\"repo\": \"g/h\"")
                && md.contains("\"repo\": \"e/f\"")
                && md.contains("\"repo\": \"c/d\""),
            "{md}"
        );
        assert!(
            !md.contains("\"repo\": \"a/b\""),
            "only three examples: {md}"
        );
        assert!(md.contains("3 open") && !md.contains("not this tool"));
        assert!(md.contains("from 1 tool and 6 recorded calls"));
        assert!(md.contains("Call on 1970-01-01:"));
    }

    #[test]
    fn long_results_are_cut_and_code_in_results_cannot_break_the_fences() {
        let tools = [issues_tool()];
        let long = "x".repeat(EXCERPT_CHARS * 2);
        let history = [
            call(
                1,
                "list_issues",
                1,
                json!({ "repo": "a" }),
                Some(text_result(&long)),
                None,
            ),
            call(
                2,
                "list_issues",
                2,
                json!({ "repo": "b" }),
                Some(text_result("see ```code``` here")),
                None,
            ),
        ];
        let md = render_docs(&input(&tools, &history));
        assert!(
            md.contains(&format!("{}…", "x".repeat(EXCERPT_CHARS))),
            "{md}"
        );
        assert!(!md.contains(&"x".repeat(EXCERPT_CHARS + 1)));
        assert!(md.contains("````text\nsee ```code``` here\n````"), "{md}");
    }

    #[test]
    fn groups_error_cases_by_message() {
        let tools = [issues_tool()];
        let history = [
            call(
                1,
                "list_issues",
                1_000,
                json!({}),
                None,
                Some("repository not found"),
            ),
            call(
                2,
                "list_issues",
                3_000,
                json!({}),
                None,
                Some("repository not found"),
            ),
            call(
                3,
                "list_issues",
                2_000,
                json!({}),
                Some(
                    json!({ "isError": true, "content": [{ "type": "text", "text": "rate limit hit\nretry later" }] }),
                ),
                None,
            ),
            call(
                4,
                "list_issues",
                4_000,
                json!({}),
                None,
                Some("uses `backticks`"),
            ),
        ];
        let mut history = history.to_vec();
        history[2].is_error = true;
        let md = render_docs(&input(&tools, &history));
        assert!(
            md.contains("- `repository not found` — 2 times, last on 1970-01-01"),
            "{md}"
        );
        assert!(md.contains("- `rate limit hit` — 1 time"), "{md}");
        assert!(
            md.contains("- ``uses `backticks` `` — 1 time")
                || md.contains("`` uses `backticks` ``"),
            "{md}"
        );
        // The most frequent error comes first.
        assert!(md.find("repository not found").unwrap() < md.find("rate limit hit").unwrap());
    }

    #[test]
    fn cancelled_calls_do_not_count() {
        let tools = [issues_tool()];
        let mut cancelled = call(
            1,
            "list_issues",
            1,
            json!({ "repo": "a" }),
            Some(text_result("x")),
            None,
        );
        cancelled.cancelled = true;
        let md = render_docs(&input(&tools, &[cancelled]));
        assert!(md.contains("_No successful call has been recorded yet._"));
    }

    #[test]
    fn adds_hints_the_return_schema_and_documentation_gaps() {
        let mut t = tool("delete_all", "Deletes", json!({ "type": "object" }));
        t.annotations = Some(JsonValue(
            json!({ "destructiveHint": true, "readOnlyHint": false, "idempotentHint": true }),
        ));
        t.output_schema = Some(JsonValue(
            json!({ "type": "object", "properties": { "deleted": { "type": "integer" } } }),
        ));
        let md = render_docs(&input(&[t], &[]));
        assert!(
            md.contains(
                "Hints from the server: may be destructive, repeating it has no further effect."
            ),
            "{md}"
        );
        assert!(md.contains("### Returns\n\n```json\n{"), "{md}");
        assert!(md.contains("### Documentation gaps"), "{md}");
        assert!(md.contains("only 1 word"), "{md}");
    }

    #[test]
    fn tools_are_sorted_and_the_output_is_deterministic() {
        let tools = [
            tool(
                "zeta",
                "Does a useful thing with the data you pass in",
                json!({}),
            ),
            tool(
                "alpha",
                "Does another useful thing with the data you give",
                json!({}),
            ),
        ];
        let md = render_docs(&input(&tools, &[]));
        assert!(md.find("## `alpha`").unwrap() < md.find("## `zeta`").unwrap());
        assert_eq!(md, render_docs(&input(&tools, &[])));
        assert!(md.ends_with('\n') && !md.ends_with("\n\n"));
    }
}
