//! Quality checks for the tools of a server.
//!
//! Models choose tools by reading their names, descriptions, and parameters, so a vague or bloated
//! definition costs accuracy and context. These checks are heuristics: they point at definitions
//! worth a second look and never claim that something is wrong for certain. They run offline and
//! need no model.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use crate::{explorer::ToolInfo, tokens::estimate_value};

/// A description with fewer words than this says too little to choose by.
pub const MIN_DESCRIPTION_WORDS: usize = 4;
/// A single tool definition above this many (estimated) tokens is expensive to carry in every request.
pub const OVERSIZED_TOOL_TOKENS: u32 = 1200;
/// All tools of a server above this many (estimated) tokens crowd the context window.
pub const OVERSIZED_SERVER_TOKENS: u32 = 12_000;
/// Two descriptions with at least this share of their words in common are reported as overlapping.
pub const OVERLAP_SIMILARITY: f64 = 0.6;
/// Descriptions need this many distinctive words before they are compared for overlap.
pub const MIN_OVERLAP_WORDS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum LintRule {
    MissingDescription,
    VagueDescription,
    UndescribedParameter,
    MissingRequired,
    UnknownRequired,
    OverlappingTools,
    OversizedDefinition,
    OversizedServer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LintFinding {
    /// The tool the finding is about; `None` for the server as a whole.
    pub tool: Option<String>,
    pub rule: LintRule,
    pub severity: Severity,
    pub message: String,
    /// Other tools that are part of the finding (the overlapping ones).
    pub related: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LintReport {
    pub findings: Vec<LintFinding>,
    pub tools_checked: u32,
    /// Estimated tokens of all definitions.
    pub total_tokens: u32,
}

const STOPWORDS: &[&str] = &[
    "a",
    "an",
    "the",
    "of",
    "to",
    "for",
    "in",
    "on",
    "and",
    "or",
    "is",
    "are",
    "this",
    "that",
    "with",
    "by",
    "from",
    "as",
    "be",
    "it",
    "its",
    "which",
    "can",
    "will",
    "you",
    "your",
    "using",
    "use",
    "all",
    "any",
    "given",
    "specified",
    "tool",
];

/// Words that say nothing about what a tool does.
const GENERIC_WORDS: &[&str] = &[
    "stuff",
    "things",
    "something",
    "various",
    "misc",
    "miscellaneous",
    "etc",
    "helper",
    "utility",
    "generic",
    "general",
];

/// Lower-case words of a text; `listIssues` and `list_issues` both give `list`, `issues`.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for c in text.chars() {
        if c.is_alphanumeric() {
            if c.is_uppercase() && previous_lower && !current.is_empty() {
                out.push(std::mem::take(&mut current).to_lowercase());
            }
            previous_lower = c.is_lowercase() || c.is_numeric();
            current.push(c);
        } else {
            previous_lower = false;
            if !current.is_empty() {
                out.push(std::mem::take(&mut current).to_lowercase());
            }
        }
    }
    if !current.is_empty() {
        out.push(current.to_lowercase());
    }
    out
}

/// A crude stem, enough to see that `lists`, `listed` and `list` are one word.
fn stem(word: &str) -> String {
    let len = word.chars().count();
    if len > 4 && word.ends_with("ies") {
        format!("{}y", &word[..word.len() - 3])
    } else if len > 5 && word.ends_with("ing") {
        word[..word.len() - 3].to_owned()
    } else if len > 4 && word.ends_with("ed") {
        word[..word.len() - 2].to_owned()
    } else if len > 3 && word.ends_with('s') && !word.ends_with("ss") {
        word[..word.len() - 1].to_owned()
    } else {
        word.to_owned()
    }
}

fn content_words(text: &str) -> BTreeSet<String> {
    words(text)
        .into_iter()
        .filter(|w| !STOPWORDS.contains(&w.as_str()))
        .map(|w| stem(&w))
        .collect()
}

fn finding(
    tool: Option<&str>,
    rule: LintRule,
    severity: Severity,
    message: impl Into<String>,
) -> LintFinding {
    LintFinding {
        tool: tool.map(str::to_owned),
        rule,
        severity,
        message: message.into(),
        related: Vec::new(),
    }
}

fn definition_tokens(tool: &ToolInfo) -> u32 {
    estimate_value(&json!({
        "name": tool.name,
        "description": tool.description,
        "inputSchema": tool.input_schema.0,
    }))
}

fn check_description(tool: &ToolInfo, out: &mut Vec<LintFinding>) {
    let name = Some(tool.name.as_str());
    let description = tool.description.as_deref().unwrap_or("").trim();
    if description.is_empty() {
        out.push(finding(
            name,
            LintRule::MissingDescription,
            Severity::Error,
            "has no description, so a model can only guess from the name",
        ));
        return;
    }
    let all_words = words(description);
    if all_words.len() < MIN_DESCRIPTION_WORDS {
        out.push(finding(
            name,
            LintRule::VagueDescription,
            Severity::Warning,
            format!(
                "the description is only {} word{}; say what the tool does, when to use it, and what it returns",
                all_words.len(),
                if all_words.len() == 1 { "" } else { "s" }
            ),
        ));
        return;
    }
    let description_words = content_words(description);
    let name_words: BTreeSet<String> = words(&tool.name).iter().map(|w| stem(w)).collect();
    // "List issues" for a tool called list_issues adds nothing.
    if !description_words.is_empty() && description_words.is_subset(&name_words) {
        out.push(finding(
            name,
            LintRule::VagueDescription,
            Severity::Warning,
            "the description only repeats the name; add what it returns, its limits, or when to use it",
        ));
        return;
    }
    let generic: Vec<&String> = all_words
        .iter()
        .filter(|w| GENERIC_WORDS.contains(&w.as_str()))
        .collect();
    if let Some(word) = generic.first() {
        out.push(finding(
            name,
            LintRule::VagueDescription,
            Severity::Warning,
            format!(
                "the description uses the vague word \"{word}\"; name what the tool actually does"
            ),
        ));
    }
}

fn check_parameters(tool: &ToolInfo, out: &mut Vec<LintFinding>) {
    let name = Some(tool.name.as_str());
    let schema = &tool.input_schema.0;
    let properties = schema.get("properties").and_then(Value::as_object);
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    if let Some(properties) = properties.filter(|p| !p.is_empty()) {
        if required.is_empty() {
            out.push(finding(
                name,
                LintRule::MissingRequired,
                Severity::Warning,
                format!(
                    "declares {} parameter{} but marks none as required; mark the ones a call cannot do without",
                    properties.len(),
                    if properties.len() == 1 { "" } else { "s" }
                ),
            ));
        }
        let undescribed: Vec<&str> = properties
            .iter()
            .filter(|(_, p)| {
                p.get("description")
                    .and_then(Value::as_str)
                    .is_none_or(|d| d.trim().is_empty())
            })
            .map(|(k, _)| k.as_str())
            .collect();
        if !undescribed.is_empty() {
            out.push(finding(
                name,
                LintRule::UndescribedParameter,
                Severity::Info,
                format!(
                    "parameters without a description: {}",
                    undescribed.join(", ")
                ),
            ));
        }
    }
    let known = properties;
    let unknown: Vec<&str> = required
        .iter()
        .copied()
        .filter(|r| known.is_none_or(|p| !p.contains_key(*r)))
        .collect();
    if !unknown.is_empty() {
        out.push(finding(
            name,
            LintRule::UnknownRequired,
            Severity::Error,
            format!(
                "requires parameters that are not declared: {}",
                unknown.join(", ")
            ),
        ));
    }
}

fn similarity(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    let shared = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        shared / union
    }
}

fn check_overlap(tools: &[ToolInfo], out: &mut Vec<LintFinding>) {
    let described: Vec<(&ToolInfo, BTreeSet<String>)> = tools
        .iter()
        .map(|t| (t, content_words(t.description.as_deref().unwrap_or(""))))
        .filter(|(_, words)| words.len() >= MIN_OVERLAP_WORDS)
        .collect();
    for (i, (first, a)) in described.iter().enumerate() {
        for (second, b) in &described[i + 1..] {
            let score = similarity(a, b);
            if score >= OVERLAP_SIMILARITY {
                let mut f = finding(
                    Some(&first.name),
                    LintRule::OverlappingTools,
                    Severity::Warning,
                    format!(
                        "its description is {}% the same as that of {}; a model cannot tell when to use which",
                        (score * 100.0).round(),
                        second.name
                    ),
                );
                f.related.push(second.name.clone());
                out.push(f);
            }
        }
    }
}

/// Checks the tools of one server.
pub fn lint_tools(tools: &[ToolInfo]) -> LintReport {
    let mut findings = Vec::new();
    let mut total = 0u32;
    for tool in tools {
        check_description(tool, &mut findings);
        check_parameters(tool, &mut findings);
        let tokens = definition_tokens(tool);
        total = total.saturating_add(tokens);
        if tokens > OVERSIZED_TOOL_TOKENS {
            findings.push(finding(
                Some(&tool.name),
                LintRule::OversizedDefinition,
                Severity::Warning,
                format!(
                    "the definition takes about {tokens} tokens (over {OVERSIZED_TOOL_TOKENS}); shorten the description or drop rarely used parameters"
                ),
            ));
        }
    }
    check_overlap(tools, &mut findings);
    if total > OVERSIZED_SERVER_TOKENS {
        findings.push(finding(
            None,
            LintRule::OversizedServer,
            Severity::Warning,
            format!(
                "the {} tools take about {total} tokens of context in every request; consider offering fewer tools",
                tools.len()
            ),
        ));
    }
    // The server first, then tool by tool, the most severe finding of a tool first.
    findings.sort_by(|a, b| {
        a.tool
            .cmp(&b.tool)
            .then(b.severity.cmp(&a.severity))
            .then(a.rule.cmp(&b.rule))
            .then(a.related.cmp(&b.related))
    });
    LintReport {
        findings,
        tools_checked: u32::try_from(tools.len()).unwrap_or(u32::MAX),
        total_tokens: total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::JsonValue;

    fn tool(name: &str, description: Option<&str>, schema: Value) -> ToolInfo {
        ToolInfo {
            name: name.into(),
            title: None,
            description: description.map(str::to_owned),
            input_schema: JsonValue(schema),
            output_schema: None,
            annotations: None,
        }
    }

    fn good_schema() -> Value {
        json!({
            "type": "object",
            "properties": { "repo": { "type": "string", "description": "owner/name of the repository" } },
            "required": ["repo"],
        })
    }

    fn good_tool(name: &str) -> ToolInfo {
        tool(
            name,
            Some("Lists the open issues of a repository, newest first, with their labels and authors"),
            good_schema(),
        )
    }

    fn rules(report: &LintReport) -> Vec<(Option<&str>, LintRule)> {
        report
            .findings
            .iter()
            .map(|f| (f.tool.as_deref(), f.rule))
            .collect()
    }

    #[test]
    fn a_well_described_tool_has_no_findings() {
        let report = lint_tools(&[good_tool("list_issues")]);
        assert_eq!(report.findings, vec![]);
        assert_eq!(report.tools_checked, 1);
        assert!(report.total_tokens > 0);
        assert_eq!(lint_tools(&[]).findings, vec![]);
    }

    #[test]
    fn reports_missing_and_blank_descriptions_as_errors() {
        let report = lint_tools(&[
            tool("a", None, good_schema()),
            tool("b", Some("   "), good_schema()),
        ]);
        assert_eq!(
            rules(&report),
            [
                (Some("a"), LintRule::MissingDescription),
                (Some("b"), LintRule::MissingDescription)
            ]
        );
        assert!(report
            .findings
            .iter()
            .all(|f| f.severity == Severity::Error));
    }

    #[test]
    fn reports_short_descriptions_and_descriptions_that_repeat_the_name() {
        let report = lint_tools(&[
            tool("get", Some("Gets it"), good_schema()),
            tool("list_issues", Some("List the issues"), good_schema()),
            tool(
                "listIssues2",
                Some("lists issues for issues"),
                good_schema(),
            ),
        ]);
        let by_tool = |name: &str| {
            report
                .findings
                .iter()
                .find(|f| f.tool.as_deref() == Some(name))
                .unwrap()
        };
        assert!(by_tool("get").message.contains("only 2 words"));
        assert!(
            by_tool("list_issues").message.contains("only 3 words"),
            "{:?}",
            by_tool("list_issues")
        );
        // Long enough, but only the name again.
        let repeats = lint_tools(&[tool(
            "list_issues",
            Some("Lists all the issues listed"),
            good_schema(),
        )]);
        assert!(
            repeats.findings[0].message.contains("repeats the name"),
            "{:?}",
            repeats.findings
        );
    }

    #[test]
    fn reports_vague_words() {
        let report = lint_tools(&[tool(
            "run",
            Some("Does various things with your repository when needed"),
            good_schema(),
        )]);
        assert_eq!(rules(&report), [(Some("run"), LintRule::VagueDescription)]);
        assert!(report.findings[0].message.contains("\"various\""));
    }

    #[test]
    fn stems_plurals_and_simple_endings() {
        assert_eq!(stem("issues"), "issue");
        assert_eq!(stem("listed"), "list");
        assert_eq!(stem("listing"), "list");
        assert_eq!(stem("entries"), "entry");
        assert_eq!(stem("class"), "class");
        assert_eq!(stem("is"), "is");
    }

    #[test]
    fn camel_case_and_snake_case_names_are_split_into_words() {
        assert_eq!(words("listOpenIssues"), ["list", "open", "issues"]);
        assert_eq!(words("list_open-issues2"), ["list", "open", "issues2"]);
        assert_eq!(words("HTTPServer"), ["httpserver"]);
        assert_eq!(words(""), Vec::<String>::new());
    }

    #[test]
    fn reports_parameters_that_are_not_marked_required() {
        let report = lint_tools(&[tool(
            "search",
            Some("Searches the issues of a repository by free text and returns the best matches"),
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "the text" },
                    "limit": { "type": "integer", "description": "how many" },
                },
            }),
        )]);
        assert_eq!(
            rules(&report),
            [(Some("search"), LintRule::MissingRequired)]
        );
        assert!(report.findings[0].message.contains("2 parameters"));
        // A tool without parameters has nothing to mark.
        let none = lint_tools(&[tool(
            "ping",
            Some("Checks that the server answers and returns its version number"),
            json!({ "type": "object", "properties": {} }),
        )]);
        assert_eq!(none.findings, vec![]);
    }

    #[test]
    fn reports_undescribed_parameters_and_unknown_required_ones() {
        let report = lint_tools(&[tool(
            "search",
            Some("Searches the issues of a repository by free text and returns the best matches"),
            json!({
                "type": "object",
                "properties": { "query": { "type": "string" }, "limit": { "type": "integer", "description": " " } },
                "required": ["query", "ghost"],
            }),
        )]);
        assert_eq!(
            rules(&report),
            [
                (Some("search"), LintRule::UnknownRequired),
                (Some("search"), LintRule::UndescribedParameter)
            ]
        );
        assert_eq!(report.findings[0].severity, Severity::Error);
        assert!(report.findings[0].message.contains("ghost"));
        assert!(report.findings[1].message.contains("query, limit"));
        assert_eq!(report.findings[1].severity, Severity::Info);
    }

    #[test]
    fn reports_overlapping_tools_once_per_pair() {
        let report = lint_tools(&[
            tool(
                "find_issue",
                Some("Finds an issue in a repository by its number and returns the title"),
                good_schema(),
            ),
            tool(
                "get_issue",
                Some("Gets an issue in a repository by its number and returns the title"),
                good_schema(),
            ),
            tool(
                "create_pr",
                Some("Creates a pull request from a branch with a title and a description"),
                good_schema(),
            ),
        ]);
        assert_eq!(
            rules(&report),
            [(Some("find_issue"), LintRule::OverlappingTools)]
        );
        assert_eq!(report.findings[0].related, ["get_issue"]);
        assert!(report.findings[0].message.contains("get_issue"));
    }

    #[test]
    fn short_descriptions_are_not_compared_for_overlap() {
        let report = lint_tools(&[
            tool("a", Some("Fetches data"), good_schema()),
            tool("b", Some("Fetches data"), good_schema()),
        ]);
        assert!(report
            .findings
            .iter()
            .all(|f| f.rule != LintRule::OverlappingTools));
    }

    #[test]
    fn reports_oversized_definitions_and_servers() {
        let long = "word ".repeat(1500);
        let report = lint_tools(&[tool(
            "big",
            Some(&format!("Explains many options: {long}")),
            good_schema(),
        )]);
        assert!(rules(&report).contains(&(Some("big"), LintRule::OversizedDefinition)));

        let many: Vec<ToolInfo> = (0..14)
            .map(|i| {
                tool(
                    &format!("tool{i}"),
                    Some(&format!("Does job number {i} {}", "with care ".repeat(600))),
                    good_schema(),
                )
            })
            .collect();
        let report = lint_tools(&many);
        assert!(report.total_tokens > OVERSIZED_SERVER_TOKENS);
        let server = report.findings.iter().find(|f| f.tool.is_none()).unwrap();
        assert_eq!(server.rule, LintRule::OversizedServer);
        // The finding about the whole server comes first.
        assert!(report.findings[0].tool.is_none());
    }

    #[test]
    fn findings_are_ordered_by_tool_and_then_by_severity() {
        let report = lint_tools(&[
            tool("z", None, good_schema()),
            tool(
                "a",
                Some("Gets it"),
                json!({ "type": "object", "properties": { "x": {} } }),
            ),
        ]);
        let order: Vec<_> = report
            .findings
            .iter()
            .map(|f| (f.tool.as_deref(), f.severity))
            .collect();
        assert_eq!(order[0].0, Some("a"));
        let a: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.tool.as_deref() == Some("a"))
            .map(|f| f.severity)
            .collect();
        assert!(a.windows(2).all(|w| w[0] >= w[1]), "{a:?}");
        assert_eq!(order.last().unwrap().0, Some("z"));
    }

    #[test]
    fn odd_schemas_do_not_crash() {
        let report = lint_tools(&[
            tool(
                "a",
                Some("Does a thing with a repository and returns text"),
                Value::Null,
            ),
            tool(
                "b",
                Some("Does a thing with a repository and returns text"),
                json!("not an object"),
            ),
            tool(
                "c",
                Some("Does a thing with a repository and returns text"),
                json!({ "properties": 5, "required": "x" }),
            ),
        ]);
        assert_eq!(report.tools_checked, 3);
    }
}
