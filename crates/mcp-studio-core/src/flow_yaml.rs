//! The YAML representation of a flow (see `docs/specs/flows.md`).
//!
//! The YAML file and the graph are two views of the same flow, so converting in either direction
//! loses nothing: `from_yaml(to_yaml(flow)) == flow`. A typical flow reads like the draft in the
//! spec, with the declarations of the `input` step under a top-level `inputs:` and those of the
//! `output` step under `outputs:`:
//!
//! ```yaml
//! version: 1
//! name: summarize
//! inputs:
//!   repo: { type: string }
//! steps:
//!   - id: issues
//!     type: tool
//!     server: github
//!     tool: list_issues
//! outputs:
//!   summary: "{{ steps.summary.text }}"
//! ```
//!
//! Those two keys are shorthand. They stand for an `input` step with the id `inputs` at the start
//! and an `output` step with the id `outputs` at the end. A flow whose input and output steps look
//! different (other ids, other positions) is written with explicit `type: input` / `type: output`
//! steps instead. Unknown keys are rejected instead of ignored, so a typo cannot silently drop part
//! of a flow.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;

use crate::{
    db::{DbError, DbResult},
    flow::{Flow, InputDecl, Step, StepKind, FLOW_VERSION},
};

/// Id of the `input` step that the top-level `inputs:` shorthand stands for.
pub const INPUTS_STEP_ID: &str = "inputs";
/// Id of the `output` step that the top-level `outputs:` shorthand stands for.
pub const OUTPUTS_STEP_ID: &str = "outputs";

#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    inputs: Option<BTreeMap<String, InputDecl>>,
    #[serde(default)]
    steps: Vec<Step>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    outputs: Option<BTreeMap<String, String>>,
}

fn invalid(message: impl std::fmt::Display) -> DbError {
    DbError::Invalid(format!("invalid flow YAML: {message}"))
}

/// Writes a flow as YAML.
pub fn to_yaml(flow: &Flow) -> DbResult<String> {
    let mut steps = flow.steps.as_slice();
    let mut inputs = None;
    if let Some((first, rest)) = steps.split_first() {
        if let (INPUTS_STEP_ID, StepKind::Input { inputs: declared }) =
            (first.id.as_str(), &first.kind)
        {
            inputs = Some(declared.clone());
            steps = rest;
        }
    }
    let mut outputs = None;
    if let Some((last, rest)) = steps.split_last() {
        if let (OUTPUTS_STEP_ID, StepKind::Output { outputs: declared }) =
            (last.id.as_str(), &last.kind)
        {
            outputs = Some(declared.clone());
            steps = rest;
        }
    }
    let document = Document {
        version: flow.version,
        name: flow.name.clone(),
        inputs,
        steps: steps.to_vec(),
        outputs,
    };
    let write_error =
        |e: serde_yaml_ng::Error| DbError::Invalid(format!("could not write YAML: {e}"));
    let mut value = serde_yaml_ng::to_value(&document).map_err(write_error)?;
    prune_defaults(&mut value);
    serde_yaml_ng::to_string(&value).map_err(write_error)
}

/// Leaves out fields that are empty or `null`, so the YAML stays short. Reading fills them in
/// again with the same empty values, so nothing is lost. Only the fields of a step and of an input
/// declaration are pruned: an empty value inside `arguments` belongs to the flow and stays.
fn prune_defaults(document: &mut Value) {
    fn is_empty(value: &Value) -> bool {
        match value {
            Value::Null => true,
            Value::Mapping(map) => map.is_empty(),
            Value::Sequence(list) => list.is_empty(),
            _ => false,
        }
    }
    fn prune_keys(value: &mut Value) {
        if let Value::Mapping(map) = value {
            let empty: Vec<Value> = map
                .iter()
                .filter(|(_, v)| is_empty(v))
                .map(|(k, _)| k.clone())
                .collect();
            for key in empty {
                map.remove(&key);
            }
        }
    }
    fn prune_declarations(inputs: Option<&mut Value>) {
        if let Some(Value::Mapping(inputs)) = inputs {
            inputs.iter_mut().for_each(|(_, decl)| prune_keys(decl));
        }
    }
    if let Some(steps) = document.get_mut("steps").and_then(Value::as_sequence_mut) {
        for step in steps {
            prune_keys(step);
            prune_declarations(step.get_mut("inputs"));
        }
    }
    prune_declarations(document.get_mut("inputs"));
}

const TOP_LEVEL: &[&str] = &["version", "name", "inputs", "steps", "outputs"];
const COMMON: &[&str] = &["id", "type"];

fn step_keys(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "input" => &["inputs"],
        "llm" => &["model", "prompt", "system", "tools"],
        "tool" => &["server", "tool", "arguments"],
        "condition" => &["expression", "then", "else"],
        "transform" => &["values"],
        "output" => &["outputs"],
        _ => return None,
    })
}

fn check_keys(value: &Value, allowed: &[&[&str]], context: &str) -> DbResult<()> {
    let Some(map) = value.as_mapping() else {
        return Err(invalid(format!("{context} must be a mapping")));
    };
    for key in map.keys() {
        let Some(key) = key.as_str() else {
            return Err(invalid(format!("{context} has a key that is not text")));
        };
        if !allowed.iter().any(|list| list.contains(&key)) {
            return Err(invalid(format!("unknown key `{key}` in {context}")));
        }
    }
    Ok(())
}

/// Rejects keys the format does not know. The deserializer would skip them, which would make a
/// misspelled key (`argumnets:`) silently disappear.
fn check_unknown_keys(root: &Value) -> DbResult<()> {
    check_keys(root, &[TOP_LEVEL], "the flow")?;
    let declarations = |value: Option<&Value>, context: &str| -> DbResult<()> {
        if let Some(map) = value.and_then(Value::as_mapping) {
            for (name, decl) in map {
                let label = format!("{context} `{}`", name.as_str().unwrap_or("?"));
                check_keys(decl, &[&["type", "description"]], &label)?;
            }
        }
        Ok(())
    };
    declarations(root.get("inputs"), "input")?;
    let Some(steps) = root.get("steps").and_then(Value::as_sequence) else {
        return Ok(());
    };
    for (index, step) in steps.iter().enumerate() {
        if !step.is_mapping() {
            return Err(invalid(format!("step {} must be a mapping", index + 1)));
        }
        let id = step.get("id").and_then(Value::as_str).unwrap_or("?");
        let context = format!("step {} (`{id}`)", index + 1);
        let kind = step.get("type").and_then(Value::as_str).unwrap_or("");
        match step_keys(kind) {
            Some(keys) => check_keys(step, &[COMMON, keys], &context)?,
            // The deserializer reports a missing or unknown `type` with the list of valid ones.
            None => continue,
        }
        if kind == "input" {
            declarations(step.get("inputs"), "input")?;
        }
        if kind == "llm" {
            if let Some(tools) = step.get("tools").and_then(Value::as_sequence) {
                for tool in tools {
                    check_keys(
                        tool,
                        &[&["server", "tool"]],
                        &format!("a tool of {context}"),
                    )?;
                }
            }
        }
    }
    Ok(())
}

/// Reads a flow from YAML. The result is not validated against servers and tools; use
/// [`crate::flow::validate`] for that.
pub fn from_yaml(text: &str) -> DbResult<Flow> {
    let root: Value = serde_yaml_ng::from_str(text).map_err(invalid)?;
    check_unknown_keys(&root)?;
    let document: Document = serde_yaml_ng::from_value(root).map_err(invalid)?;
    if document.version != FLOW_VERSION {
        return Err(invalid(format!(
            "version {} is not supported (expected {FLOW_VERSION})",
            document.version
        )));
    }

    let mut steps = document.steps;
    if let Some(inputs) = document.inputs {
        reserve(&steps, INPUTS_STEP_ID, "inputs")?;
        steps.insert(
            0,
            Step {
                id: INPUTS_STEP_ID.into(),
                kind: StepKind::Input { inputs },
            },
        );
    }
    if let Some(outputs) = document.outputs {
        reserve(&steps, OUTPUTS_STEP_ID, "outputs")?;
        steps.push(Step {
            id: OUTPUTS_STEP_ID.into(),
            kind: StepKind::Output { outputs },
        });
    }
    Ok(Flow {
        version: document.version,
        name: document.name,
        steps,
    })
}

fn reserve(steps: &[Step], id: &str, key: &str) -> DbResult<()> {
    if steps.iter().any(|s| s.id == id) {
        return Err(invalid(format!(
            "a step has the id `{id}`, which the top-level `{key}:` shorthand uses"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{flow::ToolRef, model::JsonValue};

    const SPEC_EXAMPLE: &str = r#"
version: 1
name: summarize-open-issues
inputs:
  repo: { type: string }
steps:
  - id: issues
    type: tool
    server: github
    tool: list_issues
    arguments:
      repo: "{{ inputs.repo }}"
      state: open
  - id: summary
    type: llm
    model: claude-sonnet-5-5
    prompt: |
      Summarize these issues in five bullets:
      {{ steps.issues.result }}
outputs:
  summary: "{{ steps.summary.text }}"
"#;

    fn step(id: &str, kind: StepKind) -> Step {
        Step {
            id: id.into(),
            kind,
        }
    }

    fn round_trip(flow: &Flow) -> String {
        let yaml = to_yaml(flow).unwrap();
        assert_eq!(&from_yaml(&yaml).unwrap(), flow, "YAML was:\n{yaml}");
        yaml
    }

    fn every_step_type() -> Flow {
        Flow {
            version: 1,
            name: "everything".into(),
            steps: vec![
                step(
                    "start",
                    StepKind::Input {
                        inputs: BTreeMap::from([
                            (
                                "repo".into(),
                                InputDecl {
                                    kind: "string".into(),
                                    description: Some("Repository: owner/name".into()),
                                },
                            ),
                            (
                                "limit".into(),
                                InputDecl {
                                    kind: "integer".into(),
                                    description: None,
                                },
                            ),
                        ]),
                    },
                ),
                step(
                    "issues",
                    StepKind::Tool {
                        server: "github".into(),
                        tool: "list_issues".into(),
                        arguments: BTreeMap::from([
                            ("repo".into(), JsonValue(json!("{{ inputs.repo }}"))),
                            ("limit".into(), JsonValue(json!(5))),
                            ("ratio".into(), JsonValue(json!(0.25))),
                            (
                                "big".into(),
                                JsonValue(json!(18_446_744_073_709_551_615u64)),
                            ),
                            ("negative".into(), JsonValue(json!(-3))),
                            ("flag".into(), JsonValue(json!(true))),
                            ("nothing".into(), JsonValue(serde_json::Value::Null)),
                            ("labels".into(), JsonValue(json!(["bug", "help wanted"]))),
                            (
                                "nested".into(),
                                JsonValue(json!({"a": {"b": [1, {"c": null}]}})),
                            ),
                        ]),
                    },
                ),
                step(
                    "summary",
                    StepKind::Llm {
                        model: "claude-sonnet-5-5".into(),
                        prompt:
                            "Line one\n  indented line\n\nSummarize {{ steps.issues.result }}\n"
                                .into(),
                        system: Some("You are terse: answer in 5 bullets # no more".into()),
                        tools: vec![ToolRef {
                            server: "github".into(),
                            tool: "get_issue".into(),
                        }],
                    },
                ),
                step(
                    "check",
                    StepKind::Condition {
                        expression: "steps.summary.text != ''".into(),
                        then: Some("pick".into()),
                        otherwise: Some("done".into()),
                    },
                ),
                step(
                    "pick",
                    StepKind::Transform {
                        values: BTreeMap::from([(
                            "text".into(),
                            "{{ steps.summary.text }}".into(),
                        )]),
                    },
                ),
                step(
                    "done",
                    StepKind::Output {
                        outputs: BTreeMap::from([(
                            "summary".into(),
                            "{{ steps.pick.text }}".into(),
                        )]),
                    },
                ),
            ],
        }
    }

    #[test]
    fn reads_the_example_of_the_spec() {
        let flow = from_yaml(SPEC_EXAMPLE).unwrap();
        assert_eq!(flow.name, "summarize-open-issues");
        let ids: Vec<_> = flow.steps.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["inputs", "issues", "summary", "outputs"]);
        assert!(
            matches!(&flow.steps[0].kind, StepKind::Input { inputs } if inputs["repo"].kind == "string")
        );
        let StepKind::Llm { prompt, .. } = &flow.steps[2].kind else {
            panic!()
        };
        assert_eq!(
            prompt,
            "Summarize these issues in five bullets:\n{{ steps.issues.result }}\n"
        );
        assert!(matches!(&flow.steps[3].kind, StepKind::Output { outputs } if outputs.len() == 1));
    }

    #[test]
    fn the_example_round_trips_in_the_shorthand_form() {
        let flow = from_yaml(SPEC_EXAMPLE).unwrap();
        let yaml = round_trip(&flow);
        assert!(yaml.contains("\ninputs:\n"), "{yaml}");
        assert!(yaml.contains("\noutputs:\n"), "{yaml}");
        assert!(!yaml.contains("type: input"), "{yaml}");
        assert!(!yaml.contains("type: output"), "{yaml}");
    }

    #[test]
    fn every_step_type_round_trips_with_explicit_input_and_output_steps() {
        let yaml = round_trip(&every_step_type());
        assert!(
            yaml.contains("type: input") && yaml.contains("type: output"),
            "{yaml}"
        );
        assert!(!yaml.contains("\ninputs:\n"), "{yaml}");
    }

    #[test]
    fn steps_keep_their_order() {
        let flow = every_step_type();
        let back = from_yaml(&to_yaml(&flow).unwrap()).unwrap();
        let ids = |f: &Flow| f.steps.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&back), ids(&flow));
    }

    #[test]
    fn text_that_looks_like_other_types_stays_text() {
        let tricky = [
            "123",
            "true",
            "null",
            "yes",
            "1.0",
            "0x1F",
            "2026-10-01",
            "~",
            "",
            " padded ",
            "a: b",
            "# c",
            "'q'",
            "\"d\"",
            "日本語 ✓",
        ];
        let flow = Flow {
            version: 1,
            name: "123".into(),
            steps: vec![step(
                "t",
                StepKind::Transform {
                    values: tricky
                        .iter()
                        .enumerate()
                        .map(|(i, t)| (format!("k{i}"), (*t).to_owned()))
                        .collect(),
                },
            )],
        };
        round_trip(&flow);
    }

    #[test]
    fn numbers_keep_their_kind() {
        let flow = every_step_type();
        let back = from_yaml(&to_yaml(&flow).unwrap()).unwrap();
        let StepKind::Tool { arguments, .. } = &back.steps[1].kind else {
            panic!()
        };
        assert!(arguments["limit"].0.is_u64());
        assert!(arguments["ratio"].0.is_f64());
        assert!(arguments["negative"].0.is_i64());
        assert_eq!(arguments["big"].0.as_u64(), Some(u64::MAX));
    }

    #[test]
    fn input_and_output_steps_that_do_not_fit_the_shorthand_stay_explicit() {
        // Wrong id.
        let mut flow = Flow {
            version: 1,
            name: "f".into(),
            steps: vec![],
        };
        flow.steps.push(step(
            "start",
            StepKind::Input {
                inputs: BTreeMap::new(),
            },
        ));
        flow.steps.push(step(
            "end",
            StepKind::Output {
                outputs: BTreeMap::new(),
            },
        ));
        round_trip(&flow);
        // Right ids, wrong positions.
        let flow = Flow {
            version: 1,
            name: "f".into(),
            steps: vec![
                step(
                    "a",
                    StepKind::Transform {
                        values: BTreeMap::new(),
                    },
                ),
                step(
                    INPUTS_STEP_ID,
                    StepKind::Input {
                        inputs: BTreeMap::new(),
                    },
                ),
                step(
                    OUTPUTS_STEP_ID,
                    StepKind::Output {
                        outputs: BTreeMap::new(),
                    },
                ),
                step(
                    "b",
                    StepKind::Transform {
                        values: BTreeMap::new(),
                    },
                ),
            ],
        };
        let yaml = round_trip(&flow);
        assert!(
            !yaml.contains("\ninputs:") && !yaml.contains("\noutputs:"),
            "{yaml}"
        );
    }

    #[test]
    fn shorthand_is_used_for_either_side_independently() {
        let only_inputs = Flow {
            version: 1,
            name: "f".into(),
            steps: vec![
                step(
                    INPUTS_STEP_ID,
                    StepKind::Input {
                        inputs: BTreeMap::new(),
                    },
                ),
                step(
                    "x",
                    StepKind::Transform {
                        values: BTreeMap::new(),
                    },
                ),
            ],
        };
        let yaml = round_trip(&only_inputs);
        assert!(yaml.contains("inputs: {}"), "{yaml}");
        let empty = Flow {
            version: 1,
            name: "empty".into(),
            steps: vec![],
        };
        round_trip(&empty);
    }

    #[test]
    fn omits_empty_optional_fields() {
        let yaml = to_yaml(&every_step_type()).unwrap();
        assert!(!yaml.contains("null") || yaml.contains("nothing"), "{yaml}");
        let bare = Flow {
            version: 1,
            name: "bare".into(),
            steps: vec![step(
                "l",
                StepKind::Llm {
                    model: "m".into(),
                    prompt: "p".into(),
                    system: None,
                    tools: vec![],
                },
            )],
        };
        let yaml = round_trip(&bare);
        assert!(
            !yaml.contains("system") && !yaml.contains("tools"),
            "{yaml}"
        );
    }

    fn error(yaml: &str) -> String {
        from_yaml(yaml).unwrap_err().to_string()
    }

    #[test]
    fn rejects_unknown_keys_instead_of_dropping_them() {
        let top = error("version: 1\nname: f\nsteps: []\nextra: 1\n");
        assert!(
            top.contains("unknown key `extra`") && top.contains("the flow"),
            "{top}"
        );
        let typo = error(
            "version: 1\nname: f\nsteps:\n  - id: t\n    type: tool\n    server: s\n    tool: x\n    argumnets: {}\n",
        );
        assert!(
            typo.contains("unknown key `argumnets`") && typo.contains("step 1 (`t`)"),
            "{typo}"
        );
        let decl = error(
            "version: 1\nname: f\ninputs:\n  repo: { type: string, requierd: true }\nsteps: []\n",
        );
        assert!(
            decl.contains("unknown key `requierd`") && decl.contains("repo"),
            "{decl}"
        );
        let tool = error(
            "version: 1\nname: f\nsteps:\n  - id: l\n    type: llm\n    model: m\n    prompt: p\n    tools:\n      - { server: s, tool: t, extra: 1 }\n",
        );
        assert!(tool.contains("unknown key `extra`"), "{tool}");
    }

    #[test]
    fn rejects_bad_documents_with_a_readable_message() {
        assert!(error("version: 2\nname: f\nsteps: []\n").contains("version 2 is not supported"));
        assert!(error("name: f\nsteps: []\n").contains("version"));
        assert!(error("version: 1\nsteps: []\n").contains("name"));
        assert!(error("version: 1\nname: f\nsteps:\n  - id: a\n    type: loop\n").contains("loop"));
        assert!(error("version: 1\nname: f\nsteps:\n  - id: a\n").contains("type"));
        assert!(error("version: 1\nname: f\nsteps:\n  - just text\n").contains("must be a mapping"));
        assert!(error("version: [1\n").contains("invalid flow YAML"));
        assert!(error("").contains("invalid flow YAML"));
        assert!(error("- a\n- b\n").contains("the flow must be a mapping"));
    }

    #[test]
    fn the_shorthand_cannot_collide_with_a_step_id() {
        let clash =
            error("version: 1\nname: f\ninputs: {}\nsteps:\n  - id: inputs\n    type: transform\n");
        assert!(
            clash.contains("`inputs`") && clash.contains("shorthand"),
            "{clash}"
        );
        let clash = error(
            "version: 1\nname: f\noutputs: {}\nsteps:\n  - id: outputs\n    type: transform\n",
        );
        assert!(clash.contains("`outputs`"), "{clash}");
    }
}
