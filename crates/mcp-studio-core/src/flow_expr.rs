//! Expressions and templates of flows.
//!
//! Text in a flow may contain `{{ expression }}`; conditions are one expression. An expression is
//! evaluated against a root object with two members, `inputs` (the values of the run) and `steps`
//! (the output of every step that ran):
//!
//! ```text
//! inputs.repo
//! steps.issues.result
//! steps.fetch.items[0].title
//! steps.check.value && len(steps.issues.items) > 3
//! contains(steps.summary.text, "urgent") || !(inputs.limit <= 5)
//! ```
//!
//! Supported: paths with `.name`, `[index]` and `["key"]`; numbers; strings in `'` or `"`; `true`,
//! `false`, `null`; `==` `!=` `<` `<=` `>` `>=`; `&&` `||` `!`; parentheses; the functions `len(x)`
//! and `contains(haystack, needle)`. A path that does not exist is an error, not an empty value, so
//! a typo is found when the flow runs.

use serde_json::{Map, Value};

use crate::db::{DbError, DbResult};

fn error(message: impl Into<String>) -> DbError {
    DbError::Invalid(message.into())
}

// ---------------------------------------------------------------------------------------------
// Parsing

#[derive(Debug, Clone, PartialEq)]
enum Expr {
    Literal(Value),
    Path(Vec<Segment>),
    Not(Box<Expr>),
    Binary(Box<Expr>, Op, Box<Expr>),
    Call(String, Vec<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
enum Segment {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Text(String),
    Ident(String),
    Op(Op),
    Not,
    Dot,
    Comma,
    LParen,
    RParen,
    LBracket,
    RBracket,
}

fn tokenize(source: &str) -> DbResult<Vec<Token>> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            c if c.is_whitespace() => i += 1,
            '.' if next.is_some_and(|n| n.is_ascii_digit())
                && !matches!(tokens.last(), Some(Token::Ident(_) | Token::RBracket)) =>
            {
                // A number such as .5
                let start = i;
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                tokens.push(Token::Number(
                    text.parse()
                        .map_err(|_| error(format!("`{text}` is not a number")))?,
                ));
            }
            '.' => {
                tokens.push(Token::Dot);
                i += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            '[' => {
                tokens.push(Token::LBracket);
                i += 1;
            }
            ']' => {
                tokens.push(Token::RBracket);
                i += 1;
            }
            '&' if next == Some('&') => {
                tokens.push(Token::Op(Op::And));
                i += 2;
            }
            '|' if next == Some('|') => {
                tokens.push(Token::Op(Op::Or));
                i += 2;
            }
            '=' if next == Some('=') => {
                tokens.push(Token::Op(Op::Eq));
                i += 2;
            }
            '!' if next == Some('=') => {
                tokens.push(Token::Op(Op::Ne));
                i += 2;
            }
            '!' => {
                tokens.push(Token::Not);
                i += 1;
            }
            '<' | '>' => {
                let or_equal = next == Some('=');
                tokens.push(Token::Op(match (c, or_equal) {
                    ('<', false) => Op::Lt,
                    ('<', true) => Op::Le,
                    ('>', false) => Op::Gt,
                    _ => Op::Ge,
                }));
                i += if or_equal { 2 } else { 1 };
            }
            '\'' | '"' => {
                let quote = c;
                let mut text = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err(error(format!("a string is not closed in `{source}`"))),
                        Some('\\') => {
                            match chars.get(i + 1) {
                                Some('n') => text.push('\n'),
                                Some('t') => text.push('\t'),
                                Some(other) => text.push(*other),
                                None => {
                                    return Err(error(format!(
                                        "a string is not closed in `{source}`"
                                    )))
                                }
                            }
                            i += 2;
                        }
                        Some(ch) if *ch == quote => {
                            i += 1;
                            break;
                        }
                        Some(ch) => {
                            text.push(*ch);
                            i += 1;
                        }
                    }
                }
                tokens.push(Token::Text(text));
            }
            c if c.is_ascii_digit() => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    // `a.1` style access is not supported; a dot followed by a non-digit ends the number.
                    if chars[i] == '.' && !chars.get(i + 1).is_some_and(|n| n.is_ascii_digit()) {
                        break;
                    }
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                tokens.push(Token::Number(
                    text.parse()
                        .map_err(|_| error(format!("`{text}` is not a number")))?,
                ));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                tokens.push(Token::Ident(chars[start..i].iter().collect()));
            }
            other => return Err(error(format!("unexpected `{other}` in `{source}`"))),
        }
    }
    Ok(tokens)
}

struct Parser<'a> {
    tokens: Vec<Token>,
    position: usize,
    source: &'a str,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += 1;
        token
    }

    fn fail<T>(&self, message: &str) -> DbResult<T> {
        Err(error(format!("{message} in `{}`", self.source)))
    }

    fn expect(&mut self, token: Token, what: &str) -> DbResult<()> {
        if self.next().as_ref() == Some(&token) {
            Ok(())
        } else {
            self.fail(&format!("expected {what}"))
        }
    }

    fn binary(&mut self, ops: &[Op], operand: fn(&mut Self) -> DbResult<Expr>) -> DbResult<Expr> {
        let mut left = operand(self)?;
        while let Some(Token::Op(op)) = self.peek().cloned() {
            if !ops.contains(&op) {
                break;
            }
            self.position += 1;
            let right = operand(self)?;
            left = Expr::Binary(Box::new(left), op, Box::new(right));
        }
        Ok(left)
    }

    fn or(&mut self) -> DbResult<Expr> {
        self.binary(&[Op::Or], Self::and)
    }

    fn and(&mut self) -> DbResult<Expr> {
        self.binary(&[Op::And], Self::equality)
    }

    fn equality(&mut self) -> DbResult<Expr> {
        self.binary(&[Op::Eq, Op::Ne], Self::comparison)
    }

    fn comparison(&mut self) -> DbResult<Expr> {
        self.binary(&[Op::Lt, Op::Le, Op::Gt, Op::Ge], Self::unary)
    }

    fn unary(&mut self) -> DbResult<Expr> {
        if self.peek() == Some(&Token::Not) {
            self.position += 1;
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> DbResult<Expr> {
        match self.next() {
            Some(Token::Number(n)) => Ok(Expr::Literal(number(n))),
            Some(Token::Text(text)) => Ok(Expr::Literal(Value::String(text))),
            Some(Token::LParen) => {
                let inner = self.or()?;
                self.expect(Token::RParen, "`)`")?;
                Ok(inner)
            }
            Some(Token::Ident(name)) => match name.as_str() {
                "true" => Ok(Expr::Literal(Value::Bool(true))),
                "false" => Ok(Expr::Literal(Value::Bool(false))),
                "null" => Ok(Expr::Literal(Value::Null)),
                _ if self.peek() == Some(&Token::LParen) => {
                    self.position += 1;
                    let mut arguments = Vec::new();
                    if self.peek() != Some(&Token::RParen) {
                        loop {
                            arguments.push(self.or()?);
                            if self.peek() == Some(&Token::Comma) {
                                self.position += 1;
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Token::RParen, "`)`")?;
                    Ok(Expr::Call(name, arguments))
                }
                _ => self.path(name),
            },
            Some(_) => self.fail("unexpected symbol"),
            None => self.fail("the expression ends too early"),
        }
    }

    fn path(&mut self, first: String) -> DbResult<Expr> {
        let mut segments = vec![Segment::Key(first)];
        loop {
            match self.peek() {
                Some(Token::Dot) => {
                    self.position += 1;
                    match self.next() {
                        Some(Token::Ident(name)) => segments.push(Segment::Key(name)),
                        _ => return self.fail("expected a name after `.`"),
                    }
                }
                Some(Token::LBracket) => {
                    self.position += 1;
                    match self.next() {
                        Some(Token::Number(n)) if n >= 0.0 && n.fract() == 0.0 => {
                            segments.push(Segment::Index(n as usize));
                        }
                        Some(Token::Text(key)) => segments.push(Segment::Key(key)),
                        _ => {
                            return self
                                .fail("expected a whole number or a string between `[` and `]`")
                        }
                    }
                    self.expect(Token::RBracket, "`]`")?;
                }
                _ => break,
            }
        }
        Ok(Expr::Path(segments))
    }
}

fn number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        Value::from(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

fn parse(source: &str) -> DbResult<Expr> {
    let tokens = tokenize(source)?;
    if tokens.is_empty() {
        return Err(error("an expression is empty"));
    }
    let mut parser = Parser {
        tokens,
        position: 0,
        source,
    };
    let expr = parser.or()?;
    if parser.position < parser.tokens.len() {
        return parser.fail("unexpected text after the expression");
    }
    Ok(expr)
}

// ---------------------------------------------------------------------------------------------
// Evaluation

fn describe(segments: &[Segment]) -> String {
    let mut text = String::new();
    for segment in segments {
        match segment {
            Segment::Key(key) if text.is_empty() => text.push_str(key),
            Segment::Key(key) => {
                text.push('.');
                text.push_str(key);
            }
            Segment::Index(i) => text.push_str(&format!("[{i}]")),
        }
    }
    text
}

fn lookup(root: &Value, segments: &[Segment]) -> DbResult<Value> {
    let mut current = root;
    for (index, segment) in segments.iter().enumerate() {
        let next = match (segment, current) {
            (Segment::Key(key), Value::Object(map)) => map.get(key),
            (Segment::Index(i), Value::Array(list)) => list.get(*i),
            _ => None,
        };
        current = next.ok_or_else(|| {
            error(format!(
                "`{}` is not available (nothing at `{}`)",
                describe(segments),
                describe(&segments[..=index])
            ))
        })?;
    }
    Ok(current.clone())
}

/// Whether a value counts as true in a condition.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

fn compare(a: &Value, b: &Value, op: Op) -> DbResult<bool> {
    let ordering = match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64().partial_cmp(&y.as_f64()),
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        _ => None,
    }
    .ok_or_else(|| error("only two numbers or two strings can be compared with < <= > >="))?;
    Ok(match op {
        Op::Lt => ordering.is_lt(),
        Op::Le => ordering.is_le(),
        Op::Gt => ordering.is_gt(),
        _ => ordering.is_ge(),
    })
}

fn call(name: &str, arguments: &[Value]) -> DbResult<Value> {
    match (name, arguments) {
        ("len", [value]) => Ok(Value::from(match value {
            Value::String(s) => s.chars().count(),
            Value::Array(a) => a.len(),
            Value::Object(o) => o.len(),
            _ => return Err(error("len() needs text, a list, or an object")),
        })),
        ("contains", [haystack, needle]) => Ok(Value::Bool(match (haystack, needle) {
            (Value::String(h), Value::String(n)) => h.contains(n.as_str()),
            (Value::Array(list), needle) => list.iter().any(|item| equal(item, needle)),
            (Value::Object(map), Value::String(key)) => map.contains_key(key),
            _ => {
                return Err(error(
                    "contains() needs text, a list, or an object as its first argument",
                ))
            }
        })),
        ("len" | "contains", _) => Err(error(format!("wrong number of arguments for {name}()"))),
        _ => Err(error(format!("unknown function {name}()"))),
    }
}

fn eval(expr: &Expr, root: &Value) -> DbResult<Value> {
    Ok(match expr {
        Expr::Literal(value) => value.clone(),
        Expr::Path(segments) => lookup(root, segments)?,
        Expr::Not(inner) => Value::Bool(!truthy(&eval(inner, root)?)),
        Expr::Call(name, arguments) => {
            let values = arguments
                .iter()
                .map(|a| eval(a, root))
                .collect::<DbResult<Vec<_>>>()?;
            call(name, &values)?
        }
        Expr::Binary(left, op, right) => match op {
            // Short circuit: the right side is only evaluated when it matters.
            Op::And => {
                let l = eval(left, root)?;
                if !truthy(&l) {
                    Value::Bool(false)
                } else {
                    Value::Bool(truthy(&eval(right, root)?))
                }
            }
            Op::Or => {
                let l = eval(left, root)?;
                if truthy(&l) {
                    Value::Bool(true)
                } else {
                    Value::Bool(truthy(&eval(right, root)?))
                }
            }
            Op::Eq => Value::Bool(equal(&eval(left, root)?, &eval(right, root)?)),
            Op::Ne => Value::Bool(!equal(&eval(left, root)?, &eval(right, root)?)),
            op => Value::Bool(compare(&eval(left, root)?, &eval(right, root)?, *op)?),
        },
    })
}

/// Evaluates one expression.
pub fn evaluate(source: &str, root: &Value) -> DbResult<Value> {
    eval(&parse(source)?, root)
}

/// Evaluates a condition: any value is turned into true or false with [`truthy`]. Text in
/// `{{ }}` is accepted too, so a condition may be written either way.
pub fn evaluate_condition(source: &str, root: &Value) -> DbResult<bool> {
    let trimmed = source.trim();
    let inner = trimmed
        .strip_prefix("{{")
        .and_then(|s| s.strip_suffix("}}"))
        .map_or(trimmed, str::trim);
    Ok(truthy(&evaluate(inner, root)?))
}

// ---------------------------------------------------------------------------------------------
// Templates

/// Splits text into literal parts and `{{ expression }}` parts.
fn parts(text: &str) -> DbResult<Vec<(bool, &str)>> {
    let mut result = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        if start > 0 {
            result.push((false, &rest[..start]));
        }
        let after = &rest[start + 2..];
        let end = after
            .find("}}")
            .ok_or_else(|| error(format!("`{{{{` is not closed in {text:?}")))?;
        result.push((true, after[..end].trim()));
        rest = &after[end + 2..];
    }
    if !rest.is_empty() {
        result.push((false, rest));
    }
    Ok(result)
}

/// How a value appears inside text.
pub fn display(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Renders text with `{{ expression }}` parts.
pub fn render_text(text: &str, root: &Value) -> DbResult<String> {
    let mut out = String::new();
    for (is_expression, part) in parts(text)? {
        if is_expression {
            out.push_str(&display(&evaluate(part, root)?));
        } else {
            out.push_str(part);
        }
    }
    Ok(out)
}

/// Renders a JSON value: text is rendered, and text that is exactly one `{{ expression }}` becomes
/// the value of the expression with its type (a number stays a number, a list stays a list). Lists
/// and objects are rendered element by element.
pub fn render_value(value: &Value, root: &Value) -> DbResult<Value> {
    Ok(match value {
        Value::String(text) => {
            let found = parts(text)?;
            match found.as_slice() {
                [(true, expression)] => evaluate(expression, root)?,
                _ => Value::String(render_text(text, root)?),
            }
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| render_value(v, root))
                .collect::<DbResult<_>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), render_value(v, root)?)))
                .collect::<DbResult<Map<_, _>>>()?,
        ),
        other => other.clone(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn root() -> Value {
        json!({
            "inputs": { "repo": "a/b", "limit": 5, "ratio": 0.5, "debug": false, "tags": ["x", "y"] },
            "steps": {
                "issues": { "result": "3 open", "items": [{"title": "First", "n": 1}, {"title": "Second", "n": 2}] },
                "check": { "value": true },
                "empty": { "list": [], "text": "" },
            },
        })
    }

    fn eval_str(source: &str) -> Value {
        evaluate(source, &root()).unwrap_or_else(|e| panic!("{source}: {e}"))
    }

    fn fails(source: &str) -> String {
        evaluate(source, &root()).unwrap_err().to_string()
    }

    #[test]
    fn reads_paths_with_names_indexes_and_quoted_keys() {
        assert_eq!(eval_str("inputs.repo"), json!("a/b"));
        assert_eq!(eval_str("steps.issues.items[1].title"), json!("Second"));
        assert_eq!(eval_str("steps.issues['items'][0][\"n\"]"), json!(1));
        assert_eq!(eval_str("inputs.tags[0]"), json!("x"));
        assert_eq!(eval_str("steps.check.value"), json!(true));
    }

    #[test]
    fn missing_paths_are_errors_that_say_where_the_path_stops() {
        let e = fails("steps.issues.nothing");
        assert!(
            e.contains("steps.issues.nothing") && e.contains("is not available"),
            "{e}"
        );
        assert!(fails("steps.ghost.result").contains("steps.ghost"));
        assert!(fails("inputs.tags[9]").contains("inputs.tags[9]"));
        assert!(fails("inputs.repo.length").contains("is not available"));
    }

    #[test]
    fn literals() {
        assert_eq!(eval_str("42"), json!(42));
        assert_eq!(eval_str("2.5"), json!(2.5));
        assert_eq!(eval_str(".5"), json!(0.5));
        assert_eq!(eval_str("'a\\'b'"), json!("a'b"));
        assert_eq!(eval_str("\"line\\nbreak\""), json!("line\nbreak"));
        assert_eq!(eval_str("true"), json!(true));
        assert_eq!(eval_str("null"), Value::Null);
    }

    #[test]
    fn comparisons_and_equality() {
        assert_eq!(eval_str("inputs.limit == 5"), json!(true));
        assert_eq!(eval_str("inputs.limit == 5.0"), json!(true));
        assert_eq!(eval_str("inputs.limit != 5"), json!(false));
        assert_eq!(
            eval_str("inputs.limit > 3 && inputs.limit <= 5"),
            json!(true)
        );
        assert_eq!(eval_str("inputs.ratio < 1"), json!(true));
        assert_eq!(eval_str("inputs.repo == 'a/b'"), json!(true));
        assert_eq!(eval_str("'apple' < 'banana'"), json!(true));
        // Lists and objects cannot be written as literals.
        assert!(fails("inputs.tags == ['x']").contains("unexpected symbol"));
    }

    #[test]
    fn ordering_needs_two_numbers_or_two_strings() {
        assert!(fails("inputs.repo < 3").contains("only two numbers or two strings"));
        assert!(fails("inputs.tags > inputs.tags").contains("only two numbers"));
    }

    #[test]
    fn logic_precedence_and_parentheses() {
        assert_eq!(eval_str("true || false && false"), json!(true));
        assert_eq!(eval_str("(true || false) && false"), json!(false));
        assert_eq!(eval_str("!inputs.debug"), json!(true));
        assert_eq!(eval_str("!!inputs.repo"), json!(true));
        assert_eq!(eval_str("1 < 2 == true"), json!(true));
        assert_eq!(eval_str("!(inputs.limit > 9)"), json!(true));
    }

    #[test]
    fn and_or_do_not_evaluate_the_right_side_when_it_does_not_matter() {
        // The right side would fail because the path is missing.
        assert_eq!(eval_str("false && steps.ghost.x"), json!(false));
        assert_eq!(eval_str("true || steps.ghost.x"), json!(true));
        assert!(fails("true && steps.ghost.x").contains("steps.ghost"));
    }

    #[test]
    fn functions() {
        assert_eq!(eval_str("len(steps.issues.items)"), json!(2));
        assert_eq!(eval_str("len(inputs.repo)"), json!(3));
        assert_eq!(eval_str("len(inputs)"), json!(5));
        assert_eq!(
            eval_str("contains(steps.issues.result, 'open')"),
            json!(true)
        );
        assert_eq!(eval_str("contains(inputs.tags, 'y')"), json!(true));
        assert_eq!(eval_str("contains(inputs.tags, 'z')"), json!(false));
        assert_eq!(eval_str("contains(inputs, 'repo')"), json!(true));
        assert!(fails("len(1)").contains("len() needs"));
        assert!(fails("len()").contains("wrong number"));
        assert!(fails("sum(1)").contains("unknown function sum()"));
    }

    #[test]
    fn truthiness_decides_conditions() {
        let c = |s: &str| evaluate_condition(s, &root()).unwrap();
        assert!(c("steps.issues.result"));
        assert!(!c("steps.empty.text"));
        assert!(!c("steps.empty.list"));
        assert!(!c("inputs.debug"));
        assert!(c("inputs.limit"));
        assert!(c("{{ inputs.limit > 3 }}"));
        assert!(!c("  {{inputs.debug}}  "));
    }

    #[test]
    fn syntax_errors_are_explained() {
        for (source, expected) in [
            ("", "empty"),
            ("1 +", "unexpected `+`"),
            ("(1", "expected `)`"),
            ("a.", "after `.`"),
            ("a[", "between `[` and `]`"),
            ("a[x]", "between `[` and `]`"),
            ("'open", "not closed"),
            ("1 2", "unexpected text"),
            ("&& true", "unexpected symbol"),
            ("true &", "unexpected `&`"),
        ] {
            let e = evaluate(source, &root()).unwrap_err().to_string();
            assert!(e.contains(expected), "{source:?}: {e}");
        }
    }

    #[test]
    fn renders_text_with_expressions() {
        let r = |t: &str| render_text(t, &root()).unwrap();
        assert_eq!(
            r("Repo {{ inputs.repo }} has {{steps.issues.result}}."),
            "Repo a/b has 3 open."
        );
        assert_eq!(r("no expressions"), "no expressions");
        assert_eq!(
            r("{{ inputs.limit }}/{{ inputs.ratio }}/{{ inputs.debug }}"),
            "5/0.5/false"
        );
        assert_eq!(r("tags: {{ inputs.tags }}"), r#"tags: ["x","y"]"#);
        assert_eq!(r("[{{ steps.empty.text }}]"), "[]");
        assert!(render_text("{{ inputs.repo", &root())
            .unwrap_err()
            .to_string()
            .contains("not closed"));
        assert!(render_text("{{ steps.ghost.x }}", &root()).is_err());
    }

    #[test]
    fn a_value_that_is_exactly_one_expression_keeps_its_type() {
        let value = json!({
            "limit": "{{ inputs.limit }}",
            "tags": "{{ inputs.tags }}",
            "flag": "{{ inputs.debug }}",
            "text": "limit is {{ inputs.limit }}",
            "nested": [{ "repo": " {{ inputs.repo }} " }, 7, null],
            "plain": "unchanged",
        });
        assert_eq!(
            render_value(&value, &root()).unwrap(),
            json!({
                "limit": 5,
                "tags": ["x", "y"],
                "flag": false,
                // Surrounding text turns the value into text.
                "text": "limit is 5",
                "nested": [{ "repo": " a/b " }, 7, null],
                "plain": "unchanged",
            })
        );
    }

    #[test]
    fn rendering_reports_the_missing_value() {
        let e = render_value(&json!({"a": "{{ steps.ghost.x }}"}), &root())
            .unwrap_err()
            .to_string();
        assert!(e.contains("steps.ghost"), "{e}");
    }
}
