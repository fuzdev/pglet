//! Lazy decoding of the string-literal forms whose lexeme isn't their value.
//!
//! Each function takes the *content* between the delimiters (quotes excluded) and
//! returns the resolved value. They run off the hot path — only when a literal
//! actually carries a transform (`''` doubling, `E'…'` escapes, `U&'…'` escapes).
//!
//! Modern Postgres semantics (`standard_conforming_strings = on`): a plain `'…'`
//! treats backslash literally, so only `''` collapses there. Backslash escapes
//! live in `E'…'`; Unicode `\XXXX` / `\+XXXXXX` escapes live in `U&'…'`.

use super::LexError;

/// Collapse `''` → `'` for a plain single-quoted string body.
pub(super) fn decode_plain(content: &str) -> String {
    content.replace("''", "'")
}

/// Decode a `"…"` quoted-identifier body: `""` → `"`.
pub(super) fn decode_quoted_ident(content: &str) -> String {
    content.replace("\"\"", "\"")
}

/// Decode an `E'…'` escape-string body: C-style backslash escapes plus `''` → `'`.
///
/// **An `E'…'` literal is assembled in bytes.** A `\xHH` / `\ooo` escape contributes a
/// single **byte**, truncated to eight bits, and everything else contributes a
/// character's UTF-8 bytes; the finished literal is then validated once, left to right,
/// in the server encoding (`finish_literal`). That is 18.6's `<xe>{xeoctesc}` /
/// `<xe>{xehexesc}` — `unsigned char c = strtoul(…); addlitchar(c)` — followed by the
/// `pg_verifymbstr` at the closing quote, and it is what makes `E'\xC3\xA9'` one
/// character rather than two and a lone `E'\xE9'` a `22021`.
///
/// **An escape is decoded by the grammar 18.6's scanner matches.** A spelling no rule
/// matches is not an error but a literal character: `\x` with no hex digit fails
/// `{xehexesc}`, falls through to `{xeescape}`, and `unescape_single_char('x')` returns
/// the letter. Too few digits after `\u` / `\U` is `{xeunicodefail}`'s `22025` rather
/// than a syntax error.
///
/// Handles `\b \f \n \r \t \v`, octal `\o`/`\oo`/`\ooo`, hex `\xH`/`\xHH`, Unicode
/// `\uXXXX` / `\UXXXXXXXX` (paired when two adjacent escapes spell a UTF-16 surrogate
/// pair), and `\\` / `\'` / `\<other>` (backslash dropped).
pub(super) fn decode_escape(content: &str, position: usize) -> Result<String, LexError> {
    let mut out: Vec<u8> = Vec::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    let mut pending: Option<u32> = None;
    while let Some(ch) = chars.next() {
        // A first surrogate puts 18.6's scanner in the exclusive `xeu` state, where
        // `<xeu>.`, `<xeu>\n` and `<xeu><<EOF>>` refuse everything the two Unicode
        // rules leave. The `<xe>`-only escape rules do not run there, so a backslash
        // reaches those three too: an escape that is not `\u` / `\U` is refused alongside
        // a plain character, a `''` and the closing quote. That member is what the
        // lookahead is for — measured on 18.6,
        // `E'\uD83D\x41\uDCA9'`, `E'\uD83D\t\uDCA9'` and `E'\uD83D\\\uDCA9'` are all
        // the pairing error there, where a guard asking only for a backslash decodes
        // them to `A💩`, a tab then `💩`, and `\💩`.
        //
        // The exemption a `\u` / `\U` gets here is a *different* rule,
        // `<xe,xeu>{xeunicodefail}`, live in `xeu` too — and it is reproduced by
        // `push_unicode_escape`'s malformed arm rather than by this lookahead:
        // `E'\uD83D\u12'` is that rule's `22025` under a bare-backslash guard as well.
        if pending.is_some() && !(ch == '\\' && matches!(chars.peek(), Some('u' | 'U'))) {
            return Err(surrogate_pair(position));
        }
        match ch {
            '\'' => {
                // `''` is a literal quote inside an E-string too.
                if chars.peek() == Some(&'\'') {
                    chars.next();
                }
                out.push(b'\'');
            }
            '\\' => match chars.next() {
                Some('b') => out.push(0x08),
                Some('f') => out.push(0x0C),
                Some('n') => out.push(b'\n'),
                Some('r') => out.push(b'\r'),
                Some('t') => out.push(b'\t'),
                Some('v') => out.push(0x0B),
                Some('x') => match take_hex(&mut chars, 1, 2) {
                    // `\xHH` is a raw byte (0x00..=0xFF) — a NUL makes the finished
                    // literal `22021`, never a Unicode-escape error.
                    Some(code) => out.push((code & 0xFF) as u8),
                    // `{xehexesc}` needs a hex digit. Without one the shorter
                    // `{xeescape}` matches instead and the letter survives.
                    None => out.push(b'x'),
                },
                Some('u') => {
                    let code = take_hex(&mut chars, 4, 4);
                    push_unicode_escape(&mut out, &mut pending, code, position)?;
                }
                Some('U') => {
                    let code = take_hex(&mut chars, 8, 8);
                    push_unicode_escape(&mut out, &mut pending, code, position)?;
                }
                Some(d @ '0'..='7') => {
                    let mut code = d.to_digit(8).unwrap_or(0);
                    for _ in 0..2 {
                        match chars.peek() {
                            Some(&n @ '0'..='7') => {
                                chars.next();
                                code = code * 8 + n.to_digit(8).unwrap_or(0);
                            }
                            _ => break,
                        }
                    }
                    // `\ooo` is a raw byte too, and `strtoul` into an `unsigned char`
                    // truncates: `\777` is `0xFF` and `\400` is a NUL.
                    out.push((code & 0o377) as u8);
                }
                // `\<other>` keeps the following char literally (backslash dropped).
                // 18.6 reads one *byte* there and lets the rest of a multi-byte
                // character through as ordinary content, which is the same bytes.
                Some(other) => push_char(&mut out, other),
                // Defensive rather than a rule of 18.6's: `scan_string` consumes a
                // backslash and the byte after it together in an E-string, so the
                // closing quote it then finds is at least two past the backslash and
                // `content` cannot end in a lone one — `E'a\'` is
                // `unterminated string literal`, measured.
                None => out.push(b'\\'),
            },
            other => push_char(&mut out, other),
        }
    }
    if pending.is_some() {
        return Err(surrogate_pair(position));
    }
    finish_literal(out, position)
}

/// Decode a `U&'…'` body with the default `\` escape: `\XXXX` (4 hex) and
/// `\+XXXXXX` (6 hex) Unicode code points, `\\` → `\`, and `''` → `'`.
///
/// This decoder appends characters rather than bytes, so no encoding validation runs
/// here — 18.6's `{quotestop}` runs none for the `xus` state either. Two
/// adjacent escapes spelling a UTF-16 surrogate pair combine, in all four
/// `\XXXX`/`\+XXXXXX` combinations, and a first half the next escape does not complete
/// is `42601 invalid Unicode surrogate pair` — `str_udeescape`'s `pair_first`.
///
/// Too few hex digits is a plain `42601` here, where the *same* malformation inside an
/// `E'…'` is `22025`: 18.6's two decoders disagree and each side is reproduced. That
/// refusal is also the one arm `str_udeescape` takes **without** reading `pair_first` —
/// it `ereport`s before any pairing check — so `U&'\D83D\zz'` is
/// `invalid Unicode escape` on both engines and not the pairing error, which is why the
/// `take_hex` failure below is raised ahead of the `pending` consultation.
///
/// The optional trailing `UESCAPE 'c'` clause (custom escape char) is not
/// supported; such input falls through to the parser as separate tokens.
pub(super) fn decode_unicode(content: &str, position: usize) -> Result<String, LexError> {
    let mut out = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    let mut pending: Option<u32> = None;
    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                if pending.is_some() {
                    return Err(surrogate_pair(position));
                }
                if chars.peek() == Some(&'\'') {
                    chars.next();
                }
                out.push('\'');
            }
            '\\' => {
                let code = match chars.peek() {
                    Some('\\') => {
                        // A doubled escape char is refused while a first surrogate is
                        // pending, before it is emitted.
                        if pending.is_some() {
                            return Err(surrogate_pair(position));
                        }
                        chars.next();
                        out.push('\\');
                        continue;
                    }
                    Some('+') => {
                        chars.next();
                        take_hex(&mut chars, 6, 6)
                    }
                    _ => take_hex(&mut chars, 4, 4),
                };
                let code = code.ok_or_else(|| unicode_escape(position))?;
                // `str_udeescape` checks the value *before* the pairing, so
                // `U&'\D83D\0000'` is `invalid Unicode escape value` where the `E'…'`
                // decoder's `xeu` state answers `invalid Unicode surrogate pair`.
                check_unicode_value(code, position)?;
                if let Some(ch) = pair_surrogate(&mut pending, code, position)? {
                    out.push(ch);
                }
            }
            other => {
                if pending.is_some() {
                    return Err(surrogate_pair(position));
                }
                out.push(other);
            }
        }
    }
    if pending.is_some() {
        return Err(surrogate_pair(position));
    }
    Ok(out)
}

/// Consume between `min` and `max` hex digits, returning the parsed value, or
/// `None` if fewer than `min` digits are present.
fn take_hex<I: Iterator<Item = char>>(
    chars: &mut std::iter::Peekable<I>,
    min: usize,
    max: usize,
) -> Option<u32> {
    let mut value: u32 = 0;
    let mut count = 0;
    while count < max {
        match chars.peek() {
            Some(&c) if c.is_ascii_hexdigit() => {
                chars.next();
                value = value * 16 + c.to_digit(16).unwrap_or(0);
                count += 1;
            }
            _ => break,
        }
    }
    (count >= min).then_some(value)
}

/// Append a `\uXXXX` / `\UXXXXXXXX` escape's character, or hold it as the pending first
/// half of a surrogate pair. `code` is `None` when the escape had too few hex digits,
/// which is `{xeunicodefail}`'s `22025` and not a syntax error.
fn push_unicode_escape(
    out: &mut Vec<u8>,
    pending: &mut Option<u32>,
    code: Option<u32>,
    position: usize,
) -> Result<(), LexError> {
    let code = code.ok_or_else(|| LexError::invalid_unicode_escape(position))?;
    if let Some(ch) = pair_surrogate(pending, code, position)? {
        push_char(out, ch);
    }
    Ok(())
}

/// Resolve one Unicode escape's value against a pending first surrogate half — the one
/// home for the pairing rule both decoders take.
///
/// Returns the character to emit, or `None` when `code` is itself a first surrogate and
/// becomes the pending half the *next* escape must complete. A first half followed by
/// anything but a second half, and a second half with no first, are both
/// `invalid Unicode surrogate pair` here; what counts as "followed by" is the caller's,
/// because 18.6 spells it as a scanner state in one decoder and a saved variable in the
/// other.
fn pair_surrogate(
    pending: &mut Option<u32>,
    code: u32,
    position: usize,
) -> Result<Option<char>, LexError> {
    if let Some(first) = pending.take() {
        if !is_surrogate_second(code) {
            return Err(surrogate_pair(position));
        }
        let combined = ((first & 0x3FF) << 10) + 0x10000 + (code & 0x3FF);
        return unicode_char(combined, position).map(Some);
    }
    if is_surrogate_second(code) {
        return Err(surrogate_pair(position));
    }
    if is_surrogate_first(code) {
        *pending = Some(code);
        return Ok(None);
    }
    unicode_char(code, position).map(Some)
}

/// 18.6's `is_utf16_surrogate_first`.
const fn is_surrogate_first(code: u32) -> bool {
    matches!(code, 0xD800..=0xDBFF)
}

/// 18.6's `is_utf16_surrogate_second`.
const fn is_surrogate_second(code: u32) -> bool {
    matches!(code, 0xDC00..=0xDFFF)
}

/// `is_valid_unicode_codepoint`: `0 < c <= 0x10FFFF`. It **accepts** a lone surrogate —
/// a surrogate half is refused by the pairing rule, never by this gate.
fn check_unicode_value(code: u32, position: usize) -> Result<(), LexError> {
    if code == 0 || code > 0x0010_FFFF {
        return Err(LexError::syntax(position, "invalid Unicode escape value"));
    }
    Ok(())
}

/// The character a validated escape value names. `pair_surrogate` takes both surrogate
/// halves before calling this, so the only value `char::from_u32` still refuses is one
/// `check_unicode_value` has already refused.
fn unicode_char(code: u32, position: usize) -> Result<char, LexError> {
    check_unicode_value(code, position)?;
    char::from_u32(code).ok_or_else(|| LexError::syntax(position, "invalid Unicode escape value"))
}

/// 18.6's `invalid Unicode surrogate pair` (`42601`), from both decoders.
fn surrogate_pair(position: usize) -> LexError {
    LexError::syntax(position, "invalid Unicode surrogate pair")
}

/// The `U&'…'` unescaper's `invalid Unicode escape` (`42601`) — a malformed escape
/// there is a plain syntax error, where the `E'…'` scanner's is `22025`.
fn unicode_escape(position: usize) -> LexError {
    LexError::syntax(position, "invalid Unicode escape")
}

/// Append a character's UTF-8 bytes.
fn push_char(out: &mut Vec<u8>, ch: char) {
    let mut buf = [0u8; 4];
    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
}

/// Validate the finished `E'…'` literal in the server encoding — 18.6's
/// `pg_verifymbstr` at the closing quote, through `pglet_core::encoding`'s one scan
/// (the first NUL or illegal byte, left to right, and the `report_invalid_encoding`
/// byte list), which COPY, bound parameters and the dump's COPY payload reach too
/// (`docs/encoding.md`).
///
/// 18.6 runs this only when a `saw_non_ascii` flag is set, where an escape produced a
/// NUL or a high byte. pglet validates unconditionally instead, which refuses nothing
/// extra: the arms above that are not byte escapes append a character's own UTF-8, so a
/// buffer this scan can refuse is one a byte escape wrote. Carrying the flag would add
/// a second fact that has to agree with the buffer rather than a semantic.
fn finish_literal(bytes: Vec<u8>, position: usize) -> Result<String, LexError> {
    if let Some(report) = pglet_core::encoding::utf8_failure_bytes(&bytes) {
        return Err(LexError::invalid_encoding(position, report));
    }
    // The scan refuses a strict superset of what `from_utf8` refuses, so this holds.
    String::from_utf8(bytes).map_err(|e| LexError::invalid_encoding(position, e.into_bytes()))
}
