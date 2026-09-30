//! `{{variable}}` placeholders in strings and JSON values.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::db::{DbError, DbResult};

/// Replaces every `{{name}}` in `template` with the variable's value.
///
/// Names may contain letters, digits, `_`, `-`, and `.`; whitespace inside the braces is ignored.
/// Text that merely looks like a placeholder (`{{ not valid }}`) is left untouched. If any variable is
/// undefined, the error lists all of them so the user can fix everything at once.
pub fn resolve_str(template: &str, variables: &BTreeMap<String, String>) -> DbResult<String> {
    let mut missing = BTreeSet::new();
    let out = substitute(template, variables, &mut missing);
    finish(out, missing)
}

/// Resolves placeholders in every string (values and object keys stay untouched) inside a JSON value.
pub fn resolve_json(value: &Value, variables: &BTreeMap<String, String>) -> DbResult<Value> {
    let mut missing = BTreeSet::new();
    let out = walk(value, variables, &mut missing);
    finish(out, missing)
}

/// Names of all placeholders used in `template`, in order of first appearance.
pub fn placeholders_in(template: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = template;
    while let Some((name, tail)) = next_placeholder(rest) {
        if let Some(name) = name {
            if !names.iter().any(|n| n == name) {
                names.push(name.to_owned());
            }
        }
        rest = tail;
    }
    names
}

fn finish<T>(value: T, missing: BTreeSet<String>) -> DbResult<T> {
    if missing.is_empty() {
        Ok(value)
    } else {
        let names: Vec<_> = missing.into_iter().collect();
        Err(DbError::Invalid(format!(
            "undefined variable(s): {}",
            names.join(", ")
        )))
    }
}

fn is_name(candidate: &str) -> bool {
    !candidate.is_empty()
        && candidate
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Finds the next `{{...}}`. Returns the variable name if the inside is a valid name, and the text after it.
fn next_placeholder(text: &str) -> Option<(Option<&str>, &str)> {
    let start = text.find("{{")?;
    let after = &text[start + 2..];
    let end = after.find("}}")?;
    let name = after[..end].trim();
    Some((is_name(name).then_some(name), &after[end + 2..]))
}

fn substitute(
    template: &str,
    variables: &BTreeMap<String, String>,
    missing: &mut BTreeSet<String>,
) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let name = after[..end].trim();
        out.push_str(&rest[..start]);
        if is_name(name) {
            match variables.get(name) {
                Some(value) => out.push_str(value),
                None => {
                    missing.insert(name.to_owned());
                }
            }
        } else {
            out.push_str(&rest[start..start + 2 + end + 2]);
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

fn walk(
    value: &Value,
    variables: &BTreeMap<String, String>,
    missing: &mut BTreeSet<String>,
) -> Value {
    match value {
        Value::String(s) => Value::String(substitute(s, variables, missing)),
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| walk(v, variables, missing)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), walk(v, variables, missing)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vars() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("host".to_owned(), "example.com".to_owned()),
            ("api.key".to_owned(), "k-1".to_owned()),
        ])
    }

    #[test]
    fn replaces_known_variables() {
        assert_eq!(
            resolve_str("https://{{host}}/x?k={{ api.key }}", &vars()).unwrap(),
            "https://example.com/x?k=k-1"
        );
    }

    #[test]
    fn text_without_placeholders_is_unchanged() {
        assert_eq!(
            resolve_str("plain { text } {", &vars()).unwrap(),
            "plain { text } {"
        );
    }

    #[test]
    fn lists_all_missing_variables_once() {
        let error = resolve_str("{{a}} {{b}} {{a}} {{host}}", &vars())
            .unwrap_err()
            .to_string();
        assert!(error.contains("a, b"), "{error}");
    }

    #[test]
    fn invalid_names_are_left_alone() {
        assert_eq!(
            resolve_str("{{ not valid }} {{}}", &vars()).unwrap(),
            "{{ not valid }} {{}}"
        );
    }

    #[test]
    fn unterminated_placeholder_is_left_alone() {
        assert_eq!(resolve_str("a {{host", &vars()).unwrap(), "a {{host");
    }

    #[test]
    fn values_are_not_resolved_recursively() {
        let vars = BTreeMap::from([("a".to_owned(), "{{b}}".to_owned())]);
        assert_eq!(resolve_str("{{a}}", &vars).unwrap(), "{{b}}");
    }

    #[test]
    fn resolves_inside_json_strings_only() {
        let value = json!({"url": "https://{{host}}", "n": 1, "list": ["{{host}}", true, null], "{{host}}": "key"});
        let resolved = resolve_json(&value, &vars()).unwrap();
        assert_eq!(
            resolved,
            json!({"url": "https://example.com", "n": 1, "list": ["example.com", true, null], "{{host}}": "key"})
        );
    }

    #[test]
    fn json_errors_collect_missing_names() {
        let error = resolve_json(&json!({"a": "{{x}}", "b": ["{{y}}"]}), &vars())
            .unwrap_err()
            .to_string();
        assert!(error.contains("x, y"), "{error}");
    }

    #[test]
    fn lists_placeholders_in_order_without_duplicates() {
        assert_eq!(
            placeholders_in("{{b}} {{a}} {{b}} {{ bad name }}"),
            ["b", "a"]
        );
    }
}
