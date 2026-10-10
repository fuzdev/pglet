//! Tests for the regex match operators.

use super::super::compare::eval_regex;
use crate::types::Value;

fn text(s: &str) -> Value {
    Value::Text(s.to_string())
}

/// Every ARE metacharacter after the anchor is refused `0A000`, one at a time — so dropping
/// any one from the refused set reddens here, including `[`, `(` and `)`, whose one-character
/// patterns 18.6 refuses too (`2201B`) and a fixture therefore cannot hold.
#[test]
fn each_metacharacter_after_the_anchor_is_refused() {
    for c in [
        '.', '[', ']', '(', ')', '*', '+', '?', '{', '}', '|', '^', '$', '\\',
    ] {
        let pattern = format!("^a{c}");
        let err = eval_regex(&text("a"), &text(&pattern), "~").expect_err(&pattern);
        assert_eq!(err.sqlstate(), "0A000", "{pattern}");
    }
}

/// The accepted shape answers the prefix test, and an unanchored pattern is refused.
#[test]
fn an_anchored_literal_prefix_answers() {
    assert_eq!(
        eval_regex(&text("abc"), &text("^ab"), "~").ok(),
        Some(Some(true))
    );
    assert_eq!(
        eval_regex(&text("abc"), &text("^b"), "~").ok(),
        Some(Some(false))
    );
    assert_eq!(
        eval_regex(&text("abc"), &text("^"), "~").ok(),
        Some(Some(true))
    );
    assert_eq!(eval_regex(&Value::Null, &text("^a"), "~").ok(), Some(None));
    assert_eq!(
        eval_regex(&text("abc"), &text("b"), "~")
            .map_err(|e| e.sqlstate())
            .err(),
        Some("0A000")
    );
}
