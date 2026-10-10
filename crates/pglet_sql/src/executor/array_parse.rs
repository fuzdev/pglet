//! Postgres array-literal text parsing: turn a `{...}` array text literal into an
//! `ArrayValue`, honoring double-quoting, backslash escapes, and `NULL` elements.
//!
//! The grammar is 18.6's `array_in`: a token reader ([`read_array_token`], its
//! `ReadArrayToken`) and the walk that says where each token may stand
//! ([`parse_array_literal`], its `ReadArrayStr`). A literal that walk refuses as malformed
//! is `22P02` here, whatever the element type; the nesting it has no value for is `0A000`.

use std::iter::Peekable;
use std::str::Chars;

use crate::types::{ArrayValue, TypeOid, Value};
use crate::{Error, Result};

use super::coerce::coerce_value;

/// The nesting depth 18.6's array reader stops at (its `MAXDIM`).
const MAX_DEPTH: usize = 6;

/// A token read from an array text literal.
enum ArrayToken {
    /// An opening brace.
    LevelStart,
    /// A closing brace.
    LevelEnd,
    /// The comma between two elements.
    Delimiter,
    /// An element's de-escaped text, or `None` for an unquoted `NULL`.
    Element(Option<String>),
}

/// Whether `c` is whitespace to the array reader: 18.6's `scanner_isspace` set — space,
/// tab, newline, carriage return, vertical tab, form feed. Any other character,
/// a Unicode space included, is an element's own.
const fn is_array_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{b}' | '\u{c}')
}

/// Parse a Postgres array text literal (`{a,b,"c,d",NULL}`) into an `ArrayValue`,
/// coercing each element to `element_type`.
///
/// Each element is converted as it is read, so an element's own input error comes ahead
/// of a structural error later in the literal, and a literal refused at a token never
/// converts the element that token cut short.
///
/// A `Value::Array` has one dimension. A nested literal is walked as 18.6 walks it — its
/// elements converted, its sub-array lengths compared — and one 18.6 would accept as a
/// multidimensional array is then refused `0A000`; one holding no element at all
/// (`{{}}`) is the empty array, as there. Explicit bounds (`[1:3]={...}`) are refused
/// as malformed.
// TODO: 1-D only — lift the nested refusal and the bounds refusal together if
// multi-dim arrays are ever needed.
pub fn parse_array_literal(s: &str, element_type: TypeOid) -> Result<ArrayValue> {
    let malformed = || Error::InvalidInputSyntax {
        type_name: "array",
        value: s.to_string(),
    };
    let multidimensional =
        || Error::NotImplemented("multidimensional arrays are not supported".into());

    let mut chars = s.chars().peekable();
    while chars.next_if(|&c| is_array_space(c)).is_some() {}
    if chars.peek() != Some(&'{') {
        return Err(malformed());
    }

    let mut elements = Vec::new();
    // per open level, the elements read so far; per level, its first sub-array's length
    let mut counts = [0usize; MAX_DEPTH];
    let mut lengths = [None; MAX_DEPTH];
    let mut depth = 0;
    let mut dimensions = 0;
    let mut dimensions_frozen = false;
    let mut expect_delimiter = false;
    loop {
        match read_array_token(&mut chars).ok_or_else(malformed)? {
            ArrayToken::LevelStart => {
                if expect_delimiter {
                    return Err(malformed());
                }
                if depth == MAX_DEPTH {
                    return Err(multidimensional());
                }
                counts[depth] = 0;
                depth += 1;
                if depth > dimensions {
                    if dimensions_frozen {
                        return Err(malformed());
                    }
                    dimensions = depth;
                }
            }
            ArrayToken::LevelEnd => {
                // a brace may close an empty level, and otherwise stands where a
                // delimiter could
                if counts[depth - 1] > 0 && !expect_delimiter {
                    return Err(malformed());
                }
                depth -= 1;
                if depth == 0 {
                    break;
                }
                counts[depth - 1] += 1;
                let length = counts[depth];
                if *lengths[depth].get_or_insert(length) != length {
                    return Err(malformed());
                }
                expect_delimiter = true;
            }
            ArrayToken::Delimiter => {
                if !expect_delimiter {
                    return Err(malformed());
                }
                expect_delimiter = false;
            }
            ArrayToken::Element(text) => {
                if expect_delimiter {
                    return Err(malformed());
                }
                elements.push(match text {
                    Some(text) => coerce_value(Value::Text(text), element_type)?,
                    None => Value::Null,
                });
                dimensions_frozen = true;
                if depth != dimensions {
                    return Err(malformed());
                }
                counts[depth - 1] += 1;
                expect_delimiter = true;
            }
        }
    }
    if chars.any(|c| !is_array_space(c)) {
        return Err(malformed());
    }
    if dimensions > 1 && !elements.is_empty() {
        return Err(multidimensional());
    }
    Ok(ArrayValue::new(element_type, elements))
}

/// Read one token, leaving `chars` after it — an element's closing delimiter or brace is
/// left for the next call. `None` is a literal malformed at this token: the input ended
/// inside it, a quote or an opening brace stands inside an unquoted element, or a quoted
/// element is followed by more of an element.
fn read_array_token(chars: &mut Peekable<Chars<'_>>) -> Option<ArrayToken> {
    while chars.next_if(|&c| is_array_space(c)).is_some() {}
    match *chars.peek()? {
        '{' => {
            chars.next();
            Some(ArrayToken::LevelStart)
        }
        '}' => {
            chars.next();
            Some(ArrayToken::LevelEnd)
        }
        ',' => {
            chars.next();
            Some(ArrayToken::Delimiter)
        }
        '"' => {
            chars.next();
            let mut text = String::new();
            loop {
                match chars.next()? {
                    '\\' => text.push(chars.next()?),
                    '"' => break,
                    c => text.push(c),
                }
            }
            while chars.next_if(|&c| is_array_space(c)).is_some() {}
            matches!(*chars.peek()?, ',' | '{' | '}').then_some(ArrayToken::Element(Some(text)))
        }
        _ => {
            // A backslash takes the next character as it is, and an escaped character is
            // neither trailing whitespace nor part of a `NULL`. `kept` is the length up
            // to the last character that stays.
            let mut text = String::new();
            let mut kept = 0;
            let mut has_escapes = false;
            loop {
                match *chars.peek()? {
                    '{' | '"' => return None,
                    ',' | '}' => break,
                    c => {
                        chars.next();
                        if c == '\\' {
                            text.push(chars.next()?);
                            has_escapes = true;
                            kept = text.len();
                        } else {
                            text.push(c);
                            if !is_array_space(c) {
                                kept = text.len();
                            }
                        }
                    }
                }
            }
            text.truncate(kept);
            let is_null = !has_escapes && text.eq_ignore_ascii_case("null");
            Some(ArrayToken::Element((!is_null).then_some(text)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sqlstate(literal: &str) -> Option<&'static str> {
        parse_array_literal(literal, TypeOid::Text)
            .err()
            .map(|e| e.sqlstate())
    }

    /// The depth bound is answered as an error at the brace that passes it, for a
    /// literal that closes its levels and for one that never does.
    #[test]
    fn a_literal_nested_past_the_depth_bound_is_refused() {
        let open = "{".repeat(MAX_DEPTH + 1);
        let close = "}".repeat(MAX_DEPTH + 1);
        assert_eq!(sqlstate(&open), Some("0A000"));
        assert_eq!(sqlstate(&format!("{open}a{close}")), Some("0A000"));
        assert_eq!(sqlstate(&format!("{open}{close}")), Some("0A000"));
        assert_eq!(sqlstate(&"{".repeat(64)), Some("0A000"));
    }

    /// At the bound a well-formed literal is walked to its end: one holding an element
    /// is a multidimensional array, one holding none is the empty array.
    #[test]
    fn a_literal_nested_to_the_depth_bound_is_walked() {
        let open = "{".repeat(MAX_DEPTH);
        let close = "}".repeat(MAX_DEPTH);
        assert_eq!(sqlstate(&format!("{open}a{close}")), Some("0A000"));
        assert_eq!(sqlstate(&format!("{open}a,{close}")), Some("22P02"));
        let empty = parse_array_literal(&format!("{open}{close}"), TypeOid::Text);
        assert!(empty.is_ok_and(|array| array.elements.is_empty()));
    }

    /// A literal with no opening brace is refused before any token is walked.
    #[test]
    fn a_literal_with_no_opening_brace_is_malformed() {
        for literal in ["", " ", "a", "}", ",", "\"a\"", "a}", "[1:1]={a}"] {
            assert_eq!(sqlstate(literal), Some("22P02"), "{literal:?}");
        }
    }
}
