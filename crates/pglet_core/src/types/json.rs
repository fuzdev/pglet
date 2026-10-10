//! Minimal JSON parsing into [`JsonbValue`].
//!
//! A small hand-rolled parser (no serde_json) shared by the `::jsonb` coercion
//! path, the `->`/`->>` JSON accessors, and the binary-format JSONB decoder
//! (`value_from_binary`). The inverse is `JsonbValue::to_json_string`.

use super::{JsonbValue, PgNumeric};
use crate::error::{Error, Result};

/// Maximum JSON nesting depth before failing loud.
///
/// `parse_json` recurses once per nested object/array, so a hostile deeply-nested
/// value (`[[[[…]]]]`, trivially millions deep within the 64 MiB wire-message cap)
/// would otherwise overflow the stack — a single malformed `::jsonb` literal or
/// binary JSONB param crashing the process. This cap bounds the recursion so it
/// fails loud (`InvalidJson`) instead, the JSON twin of the SQL parser's
/// `MAX_DEPTH`. Set to the same conservative 128: far beyond any real value's
/// nesting yet safe even on a small debug-build thread stack.
const MAX_JSON_DEPTH: usize = 128;

/// Parse JSON text into a `JsonbValue`, erroring on the malformed input the parser detects.
///
/// The fallible wrapper used by the `::jsonb` cast and the binary JSONB decoder. A
/// malformed fragment it detects — a bad object pair, a bad array element, an
/// unknown/truncated escape, a raw control character inside a string, a character 18.6's
/// lexer does not count as whitespace where a token starts — fails the *whole* parse
/// rather than being dropped, as in Postgres. Where the two still part is the *order* in
/// which a value's structure and its scalars are read: pglet checks a container's shape
/// from the outside in, 18.6 lexes left to right (`EDGES.md` row 131).
///
/// # Errors
///
/// Returns `Error::InvalidJson` (SQLSTATE `22P02`) where the parser refuses `s`,
/// `Error::NumericFormatOverflow` (`22003`) if a syntactically-valid JSON number
/// overflows the numeric format (matching Postgres's `numeric_in`, which `jsonb_in`
/// runs each number token through — e.g. `1e200000`), `Error::UnsupportedUnicodeEscape`
/// (`22P05`) for a `\u0000` escape, or `Error::InvalidEncoding` (`22021`) for a raw NUL
/// inside a string, which the wire doors answer at the encoding scan before the reader is
/// reached, leaving the in-process ones (`raw_control_character`).
///
/// The parsed value is returned in **canonical** form ([`JsonbValue::canonical`]) —
/// object keys deduplicated last-wins and sorted Postgres's way — mirroring
/// `jsonb_in`, which runs `uniqueifyJsonbObject` on every object it builds. The one
/// deep pass happens here at the entry point rather than per nested object, so a
/// document is canonicalized once rather than once per nesting level.
pub fn parse_jsonb(s: &str) -> Result<JsonbValue> {
    parse_json_at(s, 0).map(JsonbValue::canonical)
}

/// Parse JSON text into a `JsonbValue`, returning `None` where [`parse_jsonb`] errors.
///
/// The hand-rolled (no `serde_json`) recursive parser is behind [`parse_jsonb`]; this
/// `Option` view discards the specific error (both a syntax error and a numeric
/// overflow map to `None`). Canonical form applies here too — see [`parse_jsonb`].
#[must_use]
pub fn parse_json(s: &str) -> Option<JsonbValue> {
    parse_jsonb(s).ok()
}

/// The whitespace 18.6's JSON lexer skips: space, tab, LF and CR, and nothing else.
///
/// `json_lex` (`src/common/jsonapi.c`) advances over exactly those four bytes ahead of a
/// token, so any other character standing where a token starts is the start of a token
/// and the input is `22P02 invalid input syntax for type json`. Rust's `str::trim` strips
/// every character carrying the Unicode `White_Space` property, a strict superset that
/// takes in `0x0B`, `0x0C`, `U+0085`, `U+00A0`, `U+2028` and `U+3000` — each measured
/// accepted here and refused on 18.6 before this narrowing.
const fn is_json_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// `str::trim` narrowed to [`is_json_whitespace`].
fn json_trim(s: &str) -> &str {
    s.trim_matches(is_json_whitespace)
}

/// `str::trim_start` narrowed to [`is_json_whitespace`].
fn json_trim_start(s: &str) -> &str {
    s.trim_start_matches(is_json_whitespace)
}

/// `parse_jsonb` with an explicit nesting depth, so deeply-nested input fails loud
/// at [`MAX_JSON_DEPTH`] instead of overflowing the stack. Each object/array
/// recurses with `depth + 1`. Returns the crate `Error` so a numeric overflow deep
/// inside a value surfaces `22003` rather than the generic `22P02`.
fn parse_json_at(s: &str, depth: usize) -> Result<JsonbValue> {
    if depth > MAX_JSON_DEPTH {
        return Err(Error::InvalidJson); // too deep: fail loud, don't overflow the stack
    }
    let s = json_trim(s);
    if s.is_empty() {
        return Err(Error::InvalidJson);
    }

    // Parse based on first character
    match s.chars().next().ok_or(Error::InvalidJson)? {
        '{' => parse_json_object(s, depth),
        '[' => parse_json_array(s, depth),
        '"' => parse_json_string(s),
        't' if s == "true" => Ok(JsonbValue::Bool(true)),
        'f' if s == "false" => Ok(JsonbValue::Bool(false)),
        'n' if s == "null" => Ok(JsonbValue::Null),
        // A number token is stored as the exact `PgNumeric` (Postgres runs the JSON
        // number through `numeric_in`), so a big integer / high-scale decimal keeps
        // full precision rather than being `f64`-rounded. The token must first match
        // the strict JSON number grammar — `PgNumeric::parse` is looser (accepts
        // `007`, `1.`, a leading `+`), which Postgres's JSON lexer rejects (`22P02`).
        // A grammar-valid number that overflows the numeric format keeps its specific
        // `22003` (matching the direct `::numeric` cast); anything else → `22P02`.
        c if c.is_ascii_digit() || c == '-' => {
            if !is_json_number(s) {
                return Err(Error::InvalidJson);
            }
            match PgNumeric::parse(s) {
                Ok(n) => Ok(JsonbValue::Number(n)),
                Err(e @ (Error::NumericOutOfRange(_) | Error::NumericFormatOverflow)) => Err(e),
                Err(_) => Err(Error::InvalidJson),
            }
        }
        _ => Err(Error::InvalidJson),
    }
}

/// Whether `s` matches the JSON number grammar (RFC 8259):
/// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
///
/// Stricter than `PgNumeric::parse`, which also accepts a leading zero (`007`,
/// `01`), a bare trailing/leading dot (`1.`, `.5`), and a leading `+` — all of which
/// Postgres's JSON lexer rejects (`22P02`). A grammar-valid number may still
/// overflow the numeric range (`1e200000`); that is caught afterward as `22003`.
fn is_json_number(s: &str) -> bool {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    if i < len && bytes[i] == b'-' {
        i += 1;
    }
    // Integer part: a lone `0`, or a nonzero leading digit then more digits — no
    // leading zero, and at least one digit (rejects `.5`, `-.5`, `+5`, empty).
    match bytes.get(i) {
        Some(&b'0') => i += 1,
        Some(&c) if c.is_ascii_digit() => {
            while i < len && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
        _ => return false,
    }
    // Optional fraction: a dot then at least one digit (rejects `1.`).
    if i < len && bytes[i] == b'.' {
        i += 1;
        let start = i;
        while i < len && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    // Optional exponent: `e`/`E`, optional sign, then at least one digit (rejects
    // `1e`, `1e+`).
    if i < len && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < len && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        let start = i;
        while i < len && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    // No trailing characters — `01` fails here: after the lone `0`, `1` remains.
    i == len
}

/// Parse a JSON object at nesting `depth`.
///
/// `s` arrives already trimmed of JSON whitespace: `parse_json_at` is the only caller and
/// it trims before it dispatches on the first character, so a second trim here would be a
/// predicate no input can exercise.
fn parse_json_object(s: &str, depth: usize) -> Result<JsonbValue> {
    if !s.starts_with('{') || !s.ends_with('}') {
        return Err(Error::InvalidJson);
    }
    let inner = json_trim(&s[1..s.len() - 1]);
    if inner.is_empty() {
        return Ok(JsonbValue::Object(Vec::new()));
    }

    // A malformed pair fails the whole parse (no silent drop); a trailing comma
    // or empty segment (`{"a":1,}`, `{"a":1,,}`) fails via `parse_json_pair`.
    let pairs = split_top_level(inner)
        .into_iter()
        .map(|seg| parse_json_pair(seg, depth + 1))
        .collect::<Result<Vec<_>>>()?;
    Ok(JsonbValue::Object(pairs))
}

/// Parse a JSON `key:value` pair, its value at nesting `depth`.
///
/// The key is parsed as a JSON string (escapes decoded), so a key containing a
/// colon (`{"a:b": 1}`) splits on the *value* colon, not the one inside the key.
fn parse_json_pair(s: &str, depth: usize) -> Result<(String, JsonbValue)> {
    let s = json_trim(s);
    // Key must be a quoted string; find its closing quote respecting escapes.
    if !s.starts_with('"') {
        return Err(Error::InvalidJson);
    }
    let key_end = json_string_end(s).ok_or(Error::InvalidJson)?;
    let key = unescape_json_string(&s[1..key_end])?;
    // The colon must follow the key (only whitespace between).
    let after_key = json_trim_start(&s[key_end + 1..]);
    let value_part = json_trim(after_key.strip_prefix(':').ok_or(Error::InvalidJson)?);
    let value = parse_json_at(value_part, depth)?;

    Ok((key, value))
}

/// Parse a JSON array at nesting `depth`.
///
/// `s` arrives already trimmed, for the reason `parse_json_object`'s doc gives.
fn parse_json_array(s: &str, depth: usize) -> Result<JsonbValue> {
    if !s.starts_with('[') || !s.ends_with(']') {
        return Err(Error::InvalidJson);
    }
    let inner = json_trim(&s[1..s.len() - 1]);
    if inner.is_empty() {
        return Ok(JsonbValue::Array(Vec::new()));
    }

    // A malformed element fails the whole parse (no silent drop); a trailing
    // comma or empty segment (`[1,2,]`, `[1,,3]`) fails via `parse_json_at`.
    split_top_level(inner)
        .into_iter()
        .map(|seg| parse_json_at(json_trim(seg), depth + 1))
        .collect::<Result<Vec<_>>>()
        .map(JsonbValue::Array)
}

/// Split an object/array body (braces already stripped) on **top-level** commas
/// — commas that are not inside a string or a nested `{}`/`[]`.
///
/// Always yields a final segment (empty when the body has a trailing comma), so
/// the caller rejects malformed input by failing to parse that segment rather
/// than silently dropping it. Shared by the object and array parsers so their
/// scan/propagation logic can't drift.
fn split_top_level(inner: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut start = 0;

    for (i, c) in inner.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '{' | '[' if !in_string => depth += 1,
            '}' | ']' if !in_string => depth -= 1,
            ',' if !in_string && depth == 0 => {
                segments.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    segments.push(&inner[start..]);
    segments
}

/// Parse a JSON string value (a leading `"`, its escaped body, a closing `"`,
/// and nothing after it).
///
/// **The body is decoded before the trailing junk is refused**, because 18.6 finishes
/// lexing the string token — raising a bad escape's error as it goes — before it reads
/// whatever stands after the closing quote. So a segment whose string is loud and whose
/// tail is junk answers the string's error: `["\u0000"<0C>]` is `22P05` on 18.6 and here,
/// where refusing the tail first would answer `22P02`. Narrowing the trim to JSON's own
/// whitespace is what put that tail in front of this check — `0x0C` used to be trimmed
/// away — so the two decisions move together. What stays after the decode is the
/// structure check itself: pglet reads a value's shape from the outside in where 18.6
/// lexes left to right, which is `EDGES.md` row 131 and is not closed here.
fn parse_json_string(s: &str) -> Result<JsonbValue> {
    if !s.starts_with('"') {
        return Err(Error::InvalidJson);
    }
    let end = json_string_end(s).ok_or(Error::InvalidJson)?;
    let decoded = unescape_json_string(&s[1..end])?;
    // The closing quote must be the final char — reject trailing junk (`"a"b`).
    if end != s.len() - 1 {
        return Err(Error::InvalidJson);
    }
    Ok(JsonbValue::String(decoded))
}

/// Find the byte index of the closing `"` of a JSON string that begins at byte
/// 0, honoring backslash escapes. Returns `None` if unterminated.
fn json_string_end(s: &str) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in s.char_indices().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '"' => return Some(i),
            _ => {}
        }
    }
    None
}

/// Decode the body of a JSON string (the chars between the quotes) into its
/// represented text in a single pass.
///
/// Handles `\" \\ \/ \b \f \n \r \t` and `\uXXXX` (including UTF-16 surrogate
/// pairs). Fails loud (`InvalidJson`, `22P02`) on an unknown escape, a lone backslash,
/// a truncated/invalid `\u`, a lone surrogate, or a **raw** control character
/// ([`raw_control_character`]) — matching Postgres, which rejects
/// all of these. A `\u0000` is `UnsupportedUnicodeEscape` (`22P05`) instead: 18.6's
/// jsonb input refuses code point zero on its own error path, because the decoded
/// string becomes a `text` value and a NUL is not valid in the server encoding.
///
/// **The raw-character gate is interleaved with the escapes rather than a pass over the
/// body**, because which of the two errors a string earns is its position. 18.6's
/// `json_lex_string` walks the token once, so whichever of a raw control character and a
/// `\u0000` escape stands first is the one that raises: `"\u0000a<TAB>b"` is `22P05`
/// and `"a<TAB>b\u0000"` is `22P02`, measured on 18.6 and reproduced here. A gate run
/// over the whole body first would answer `22P02` for both.
fn unescape_json_string(inner: &str) -> Result<String> {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            if (c as u32) < 0x20 {
                return Err(raw_control_character(c));
            }
            out.push(c);
            continue;
        }
        match chars.next().ok_or(Error::InvalidJson)? {
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            '/' => out.push('/'),
            'b' => out.push('\u{08}'),
            'f' => out.push('\u{0C}'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'u' => {
                let cp = parse_hex4(&mut chars).ok_or(Error::InvalidJson)?;
                if (0xD800..=0xDBFF).contains(&cp) {
                    // High surrogate: must be followed by a `\uXXXX` low surrogate.
                    if chars.next() != Some('\\') || chars.next() != Some('u') {
                        return Err(Error::InvalidJson);
                    }
                    let low = parse_hex4(&mut chars).ok_or(Error::InvalidJson)?;
                    if !(0xDC00..=0xDFFF).contains(&low) {
                        return Err(Error::InvalidJson);
                    }
                    let combined = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                    out.push(char::from_u32(combined).ok_or(Error::InvalidJson)?);
                } else if (0xDC00..=0xDFFF).contains(&cp) {
                    // Lone low surrogate.
                    return Err(Error::InvalidJson);
                } else if cp == 0 {
                    return Err(Error::UnsupportedUnicodeEscape);
                } else {
                    out.push(char::from_u32(cp).ok_or(Error::InvalidJson)?);
                }
            }
            _ => return Err(Error::InvalidJson), // unknown escape → loud
        }
    }
    Ok(out)
}

/// The error a raw control character inside a JSON string earns, by the character.
///
/// 18.6's `json_lex_string` refuses a byte at or below 31 inside a string —
/// `(unsigned char) *p <= 31` in `src/common/jsonapi.c` — with `JSON_ESCAPING_REQUIRED`,
/// which reaches the client as `22P02 invalid input syntax for type json` and the detail
/// `Character with value 0x… must be escaped.` (`0x7F` is above that bound and is taken
/// by both engines.) That is this function's `InvalidJson` arm.
///
/// A NUL is the arm that is not 18.6's, because 18.6 never reaches its JSON lexer with
/// one: its encoding check refuses the text at the door, `22021`, and so does pglet's at
/// every door that runs the scan (`crate::encoding`). The doors measured not to run it are
/// the in-process ones, which take their SQL as a Rust `&str` — so a raw NUL is measured
/// to arrive here from those, and `22021` is the answer the two engines have both been
/// measured to give it. Answering `22P02` for it instead would invent a code where a measured pair
/// agrees.
fn raw_control_character(c: char) -> Error {
    if c == '\0' {
        // The reported bytes are the scan's, not a second spelling of the rule.
        return Error::InvalidEncoding {
            bytes: crate::encoding::invalid_encoding_bytes(&[0], 0),
        };
    }
    Error::InvalidJson
}

/// Read exactly four hexadecimal digits as a code unit (for `\uXXXX`).
fn parse_hex4(chars: &mut std::str::Chars) -> Option<u32> {
    let mut v = 0u32;
    for _ in 0..4 {
        v = v * 16 + chars.next()?.to_digit(16)?;
    }
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `JsonbValue::Number` from an integer, for terse test expectations.
    fn num(v: i64) -> JsonbValue {
        JsonbValue::Number(PgNumeric::from_i128(i128::from(v)))
    }

    /// Decode a JSON value that is a string, returning its decoded text.
    fn parse_str(s: &str) -> Option<String> {
        match parse_json(s)? {
            JsonbValue::String(s) => Some(s),
            _ => None,
        }
    }

    // 18.6's `json_lex_string` refuses a raw byte at or below 31 inside a string
    // (`(unsigned char) *p <= 31`), at a value and at a key alike; `0x7F` is above that
    // bound and is taken. The escaped spellings decode to the same characters and are
    // accepted, which is what separates a gate over the raw input from one over the
    // decoded output. Measured on 18.6 for each spelling here.
    #[test]
    fn a_raw_control_character_inside_a_string_is_refused() {
        for mark in ['\u{01}', '\t', '\n', '\u{1f}'] {
            for text in [
                format!("\"a{mark}b\""),
                format!("{{\"k\": \"a{mark}b\"}}"),
                format!("{{\"a{mark}b\": 1}}"),
                format!("[\"a{mark}b\"]"),
            ] {
                assert!(
                    matches!(parse_jsonb(&text), Err(Error::InvalidJson)),
                    "{text:?}"
                );
            }
        }
        // The accept side, at both ends of the bound and through the escapes.
        assert_eq!(parse_str("\"a\u{7f}b\"").as_deref(), Some("a\u{7f}b"));
        assert_eq!(parse_str("\"a b\"").as_deref(), Some("a b"));
        assert_eq!(parse_str(r#""a\tb""#).as_deref(), Some("a\tb"));
    }

    // A raw NUL is the one arm that is not 18.6's JSON lexer's: 18.6 answers `22021` from
    // its encoding check before the lexer, at every route measured to carry one, and this
    // reader answers the same code for the in-process doors that skip pglet's scan.
    #[test]
    fn a_raw_nul_inside_a_string_is_an_encoding_failure() {
        let err = parse_jsonb("\"a\0b\"").unwrap_err();
        assert_eq!(err.sqlstate(), "22021");
        assert_eq!(
            err.to_string(),
            "invalid byte sequence for encoding \"UTF8\": 0x00"
        );
        assert!(matches!(
            parse_jsonb("{\"a\0b\": 1}"),
            Err(Error::InvalidEncoding { .. })
        ));
        // The escaped spelling keeps jsonb's own code rather than taking this one.
        assert!(matches!(
            parse_jsonb(r#""a\u0000b""#),
            Err(Error::UnsupportedUnicodeEscape)
        ));
    }

    // 18.6's `json_lex` skips exactly space, tab, LF and CR ahead of a token; every other
    // character carrying Rust's `White_Space` property is the start of a token there, and
    // the input is invalid. Measured on 18.6 for each spelling here.
    #[test]
    fn json_whitespace_is_space_tab_lf_and_cr_and_nothing_else() {
        for ws in [' ', '\t', '\n', '\r'] {
            assert_eq!(
                parse_json(&format!(
                    "{ws}{{{ws}\"k\"{ws}:{ws}1{ws},{ws}\"j\"{ws}:{ws}[{ws}]{ws}}}{ws}"
                )),
                parse_json(r#"{"k": 1, "j": []}"#),
                "{ws:?}"
            );
        }
        for other in [
            '\u{0b}', '\u{0c}', '\u{85}', '\u{a0}', '\u{2028}', '\u{3000}',
        ] {
            for text in [
                format!("{other}1"),
                format!("1{other}"),
                format!("{{\"k\"{other}: 1}}"),
                format!("{{\"k\":{other}1}}"),
                format!("{{{other}}}"),
                format!("{{{other}\"k\": 1}}"),
                format!("[{other}]"),
                format!("[{other}1]"),
                format!("[1,{other}2]"),
            ] {
                assert_eq!(parse_json(&text), None, "{text:?}");
            }
        }
    }

    // `\u0000` is 18.6's `JSON_UNICODE_CODE_POINT_ZERO` -- `22P05`, not the `22P02` the
    // other malformed escapes take -- on a value and on a key alike, while the other
    // spellings here keep their answers. Measured on 18.6 for each spelling here.
    #[test]
    fn a_code_point_zero_escape_is_unsupported_rather_than_malformed() {
        assert!(matches!(
            parse_jsonb(r#""a\u0000b""#),
            Err(Error::UnsupportedUnicodeEscape)
        ));
        assert!(matches!(
            parse_jsonb(r#"{"k\u0000":1}"#),
            Err(Error::UnsupportedUnicodeEscape)
        ));
        // The accept side: a non-zero code point, and a surrogate pair, are characters.
        assert_eq!(parse_str(r#""a\u0041b""#).as_deref(), Some("aAb"));
        assert_eq!(parse_str(r#""\ud83d\udca9""#).as_deref(), Some("\u{1F4A9}"));
        // Left to right: whichever escape fails first decides the code.
        assert!(matches!(
            parse_jsonb(r#"["\u0000", "\ud800"]"#),
            Err(Error::UnsupportedUnicodeEscape)
        ));
        assert!(matches!(
            parse_jsonb(r#"["\ud800", "\u0000"]"#),
            Err(Error::InvalidJson)
        ));
    }

    // Bug 1: a malformed object pair used to be silently dropped, leaving the
    // surrounding object "valid". It must now fail the whole parse.
    #[test]
    fn malformed_object_pair_fails_loud() {
        assert_eq!(parse_json(r#"{"a":1,"b":@@bad}"#), None);
        assert!(parse_jsonb(r#"{"a":1,"b":@@bad}"#).is_err());
        // not the silently-truncated `{"a":1}`
        assert_ne!(parse_json(r#"{"a":1,"b":@@bad}"#), parse_json(r#"{"a":1}"#));
    }

    // Bug 2: a malformed array element used to be silently dropped.
    #[test]
    fn malformed_array_element_fails_loud() {
        assert_eq!(parse_json("[1,@@bad,3]"), None);
        assert!(parse_jsonb("[1,@@bad,3]").is_err());
        assert_ne!(parse_json("[1,@@bad,3]"), parse_json("[1,3]"));
    }

    // Bug 3: the chained-`replace` decoder double-processed — JSON `\\t`
    // (backslash + 't') wrongly became a literal TAB.
    #[test]
    fn escaped_backslash_t_is_not_a_tab() {
        // source string literal "\\t" → backslash followed by 't'
        assert_eq!(parse_str(r#""\\t""#).as_deref(), Some("\\t"));
        // a real tab escape "\t" → TAB
        assert_eq!(parse_str(r#""\t""#).as_deref(), Some("\t"));
    }

    #[test]
    fn standard_escapes_decode() {
        assert_eq!(
            parse_str(r#""a\tb\nc\r\"\\\/d""#).as_deref(),
            Some("a\tb\nc\r\"\\/d")
        );
        assert_eq!(parse_str(r#""\b\f""#).as_deref(), Some("\u{08}\u{0C}"));
    }

    #[test]
    fn unicode_escape_decodes() {
        assert_eq!(parse_str(r#""é""#).as_deref(), Some("é"));
        // surrogate pair → U+1F600 😀
        assert_eq!(parse_str(r#""😀""#).as_deref(), Some("😀"));
    }

    #[test]
    fn lone_and_bad_surrogates_fail_loud() {
        assert_eq!(parse_str(r#""\uD83D""#), None); // lone high surrogate
        assert_eq!(parse_str(r#""\uDE00""#), None); // lone low surrogate
        assert_eq!(parse_str(r#""\uD83Dx""#), None); // high not followed by \u
    }

    #[test]
    fn unknown_escape_and_truncated_unicode_fail_loud() {
        assert_eq!(parse_str(r#""\x""#), None); // unknown escape
        assert_eq!(parse_str(r#""\u12""#), None); // truncated \u
        assert_eq!(parse_str(r#""\uZZZZ""#), None); // non-hex \u
        assert_eq!(parse_str(r#""ab\""#), None); // lone trailing backslash (unterminated)
    }

    // Trailing comma / empty segment is malformed JSON (Postgres rejects it).
    #[test]
    fn trailing_comma_fails_loud() {
        assert_eq!(parse_json(r#"{"a":1,}"#), None);
        assert_eq!(parse_json("[1,2,]"), None);
        assert_eq!(parse_json(r#"{"a":1,,"b":2}"#), None);
        assert_eq!(parse_json("[1,,3]"), None);
    }

    // Deeply nested input (an untrusted `::jsonb` literal or binary JSONB param)
    // must hit the depth guard and fail loud, not recurse into a stack overflow —
    // the JSON twin of the SQL parser's `deep_nesting_fails_loud_not_overflow`.
    // 50_000 levels would blow any thread stack without the `MAX_JSON_DEPTH` cap.
    #[test]
    fn deeply_nested_fails_loud_not_overflow() {
        let deep_array = "[".repeat(50_000) + &"]".repeat(50_000);
        assert_eq!(parse_json(&deep_array), None);
        assert!(parse_jsonb(&deep_array).is_err());

        let deep_object = "{\"a\":".repeat(50_000) + "1" + &"}".repeat(50_000);
        assert_eq!(parse_json(&deep_object), None);
        assert!(parse_jsonb(&deep_object).is_err());

        // Nesting right up to the limit still parses (the guard fires *past* it, not
        // at legitimate depth) — guards against an off-by-one that rejects valid JSON.
        let at_limit = "[".repeat(MAX_JSON_DEPTH) + "1" + &"]".repeat(MAX_JSON_DEPTH);
        assert!(
            parse_json(&at_limit).is_some(),
            "nesting at the limit must still parse"
        );
    }

    // A key containing a colon must split on the value colon, not the key's.
    #[test]
    fn colon_in_key_resolves() {
        let parsed = parse_json(r#"{"a:b":1}"#);
        assert_eq!(
            parsed,
            Some(JsonbValue::Object(vec![("a:b".to_string(), num(1))]))
        );
    }

    // A duplicate object key keeps the LAST value (Postgres `jsonb_in`
    // last-write-wins), collapsed to a single entry at the first-occurrence
    // position — so the first-match extraction/equality paths read the right value.
    #[test]
    fn duplicate_object_keys_keep_last_value() {
        assert_eq!(
            parse_json(r#"{"a":1,"a":2}"#),
            Some(JsonbValue::Object(vec![("a".to_string(), num(2))]))
        );
        // First-occurrence position retained, last value; unrelated key unaffected.
        assert_eq!(
            parse_json(r#"{"a":1,"b":9,"a":3}"#),
            Some(JsonbValue::Object(vec![
                ("a".to_string(), num(3)),
                ("b".to_string(), num(9)),
            ]))
        );
        // More than two occurrences: the very last wins.
        assert_eq!(
            parse_json(r#"{"k":1,"k":2,"k":3}"#),
            Some(JsonbValue::Object(vec![("k".to_string(), num(3))]))
        );
        // Structural equality holds after dedup: `{"a":1,"a":2}` == `{"a":2}`.
        assert_eq!(parse_json(r#"{"a":1,"a":2}"#), parse_json(r#"{"a":2}"#));
    }

    // A jsonb number parses into an exact `PgNumeric` (not f64), so a big integer
    // and a high-scale decimal round-trip through `parse_json` → `to_json_string`
    // byte-for-byte, and scientific input renders in plain decimal (`numeric_out`).
    #[test]
    fn numbers_parse_exact_and_round_trip() {
        for src in [
            "123456789012345678901234567890", // 30-digit integer, past f64 exactness
            "0.1234567890123456789",          // 19 significant fractional digits
            "1.50",                           // display scale preserved
            "100",
            "-42.5",
            "0",
        ] {
            let parsed = parse_json(src).expect("valid json number");
            assert_eq!(parsed.to_json_string(), src, "round-trip of {src}");
        }
        // Scientific input renders in plain decimal (Postgres `numeric_out`), never
        // scientific — `1e2` is `100`, not `1e+2`.
        assert_eq!(parse_json("1e2").unwrap().to_json_string(), "100");
        assert_eq!(parse_json("1.5e-3").unwrap().to_json_string(), "0.0015");
        // The parsed payload is the exact `PgNumeric`, not an f64 approximation.
        let big = "123456789012345678901234567890";
        assert_eq!(
            parse_json(big),
            Some(JsonbValue::Number(PgNumeric::parse(big).unwrap()))
        );
    }

    // A syntactically-valid JSON number that overflows the numeric format surfaces
    // the specific `22003` (`jsonb_in` runs each number token through `numeric_in`),
    // propagated up from a nested position too — not the generic `22P02` the old
    // `.ok()`-swallowing collapsed it to. A genuinely-malformed number stays `22P02`.
    #[test]
    fn number_overflow_propagates_22003_not_22p02() {
        for over in [
            "1e200000",          // weight overflow (upper bound)
            "1e-200000",         // display-scale overflow (lower bound — the phantom case)
            r#"{"n":1e200000}"#, // nested in an object
            r#"{"n":1e-200000}"#,
            "[1, 1e-200000]", // nested in an array
        ] {
            assert_eq!(
                parse_jsonb(over).unwrap_err().sqlstate(),
                "22003",
                "a valid-but-overflowing jsonb number must report 22003: {over}"
            );
        }
        // A genuinely-malformed number (not a valid number at all) stays 22P02.
        for bad in [r#"{"n":1.2.3}"#, "@@bad", "1.2.3", "[1, 2,]"] {
            assert_eq!(
                parse_jsonb(bad).unwrap_err().sqlstate(),
                "22P02",
                "malformed json must stay 22P02: {bad}"
            );
        }
    }

    // The jsonb number arm enforces the strict JSON number grammar: Postgres rejects
    // leading zeros, a bare dot, and a leading `+` even though the permissive numeric
    // parser accepts them. Boundary verified against live Postgres 18.
    #[test]
    fn strict_json_number_grammar() {
        for ok in [
            "0", "-0", "0.0", "1.5", "1E2", "10e-1", "-42.5", "1.5e-3", "100", "0.5",
        ] {
            assert!(parse_json(ok).is_some(), "{ok} must parse");
        }
        for bad in [
            "007", "01", "00", "1.", "-.5", ".5", "1e", "1.e5", "+5", "1e+",
        ] {
            assert_eq!(parse_json(bad), None, "{bad} must be rejected");
            assert_eq!(
                parse_jsonb(bad).unwrap_err().sqlstate(),
                "22P02",
                "grammar-invalid number must be 22P02: {bad}"
            );
        }
    }

    // Object keys come back in Postgres's canonical order, not input order.
    #[test]
    fn object_keys_are_canonically_ordered() {
        // Equal-length keys sort bytewise...
        assert_eq!(
            parse_json(r#"{"b":1,"a":2,"c":3}"#),
            Some(JsonbValue::Object(vec![
                ("a".to_string(), num(2)),
                ("b".to_string(), num(1)),
                ("c".to_string(), num(3)),
            ]))
        );
        // ...but length comes first, so a short key precedes a bytewise-smaller
        // long one (`lengthCompareJsonbString`). This is the case a lexical sort
        // would get wrong, and the one `{"b":1,"aa":2,"c":3}` renders as
        // `{"b": 1, "c": 3, "aa": 2}` on both engines.
        assert_eq!(
            parse_json(r#"{"aa":2,"c":3,"b":1}"#),
            Some(JsonbValue::Object(vec![
                ("b".to_string(), num(1)),
                ("c".to_string(), num(3)),
                ("aa".to_string(), num(2)),
            ]))
        );
        // Nested objects are canonicalized too — the deep pass runs at the entry
        // point, so nesting depth doesn't skip it.
        assert_eq!(
            parse_json(r#"{"z":{"bb":1,"a":2}}"#),
            Some(JsonbValue::Object(vec![(
                "z".to_string(),
                JsonbValue::Object(vec![("a".to_string(), num(2)), ("bb".to_string(), num(1)),])
            )]))
        );
    }

    #[test]
    fn well_formed_values_still_parse() {
        assert_eq!(parse_json("null"), Some(JsonbValue::Null));
        assert_eq!(parse_json("true"), Some(JsonbValue::Bool(true)));
        assert_eq!(parse_json("{}"), Some(JsonbValue::Object(Vec::new())));
        assert_eq!(parse_json("[]"), Some(JsonbValue::Array(Vec::new())));
        assert_eq!(
            parse_json(r#"{"a": 1, "b": [2, 3], "c": {"d": "e,f"}}"#),
            Some(JsonbValue::Object(vec![
                ("a".to_string(), num(1)),
                ("b".to_string(), JsonbValue::Array(vec![num(2), num(3)])),
                (
                    "c".to_string(),
                    JsonbValue::Object(vec![(
                        "d".to_string(),
                        JsonbValue::String("e,f".to_string())
                    )])
                ),
            ]))
        );
    }
}
