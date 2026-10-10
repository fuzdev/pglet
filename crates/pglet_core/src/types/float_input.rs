//! A `float4` / `float8` read from text, as 18.6's `float4in` / `float8in` read it —
//! `float4in_internal` / `float8in_internal` in `src/backend/utils/adt/float.c`.
//!
//! Those run C's `strtof` / `strtod` over the input once its leading whitespace is
//! skipped, and three outcomes follow:
//!
//! - **The value is out of range** — a decimal whose magnitude overflows the type, or
//!   whose nonzero value underflows it to zero — and the input is refused `22003`, naming
//!   the number as the reader consumed it (no surrounding whitespace, no trailing text):
//!   `'1e39'::real` and `'1e39x'::real` are both `"1e39" is out of range for type real`.
//!   A value that underflows to a **subnormal** is not out of range (`'1e-40'::real` is
//!   `1e-40`), and an exact zero is not either (`'0e-999'::float8` is `0`).
//! - **Nothing the reader consumes is followed by anything but whitespace**, and the
//!   value is the answer — `Infinity`, `inf` and `NaN` in any case, each with an optional
//!   sign, among the spellings.
//! - Otherwise the input is `22P02`, naming it whole.
//!
//! Whitespace is C's `isspace` set, which is the lexer's: a no-break space is text, not
//! padding, as it is on 18.6.
//!
//! The decimal itself is converted by Rust's float parsing, which rounds correctly to
//! nearest-even as glibc's `strtod` / `strtof` do, so the value and the range verdict are
//! one rounding of the decimal straight to the target width. What glibc's readers take
//! beyond that grammar this one refuses `22P02` — the extensions `float.c` calls
//! "unportable" and leaves to the platform; the measured members are a hexadecimal float
//! (`'0x1A'::float8` is `26` on 18.6) and a NaN with a payload (`'nan(123)'` is `NaN`
//! there).
//!
//! @module

use crate::error::Error;

/// Read a `double precision` from text, as 18.6's `float8in` does.
///
/// # Errors
///
/// [`Error::FloatInputOutOfRange`] (`22003`) for a decimal outside the type's range,
/// [`Error::InvalidInputSyntax`] (`22P02`) for text that is not a float.
pub fn parse_float8_text(text: &str) -> Result<f64, Error> {
    read_float(text, "double precision", |s| s.parse::<f64>().ok())
}

/// Read a `real` from text, as 18.6's `float4in` does — one rounding of the decimal
/// straight to `f32`, never through `f64`.
///
/// # Errors
///
/// [`Error::FloatInputOutOfRange`] (`22003`) for a decimal outside the type's range,
/// [`Error::InvalidInputSyntax`] (`22P02`) for text that is not a float.
pub fn parse_float4_text(text: &str) -> Result<f32, Error> {
    read_float(text, "real", |s| s.parse::<f32>().ok())
}

/// The float trait surface the reader needs from `f32` and `f64`.
trait FloatValue: Copy {
    fn is_infinite(self) -> bool;
    fn is_zero(self) -> bool;
}

impl FloatValue for f32 {
    fn is_infinite(self) -> bool {
        Self::is_infinite(self)
    }
    fn is_zero(self) -> bool {
        self == 0.0
    }
}

impl FloatValue for f64 {
    fn is_infinite(self) -> bool {
        Self::is_infinite(self)
    }
    fn is_zero(self) -> bool {
        self == 0.0
    }
}

/// The shared reader. `parse` converts the number the scanner delimited; the scanner's
/// grammar is a subset of Rust's float grammar, so a `None` there is a reader
/// disagreement, refused `22P02` rather than answered.
fn read_float<T: FloatValue>(
    text: &str,
    type_name: &'static str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<T, Error> {
    let syntax = || Error::InvalidInputSyntax {
        type_name,
        value: text.to_string(),
    };
    let body = text.trim_start_matches(|c: char| u8::try_from(c).is_ok_and(is_c_space));
    let (len, decimal) = match scan_number(body.as_bytes()) {
        Scanned::Nothing => return Err(syntax()),
        Scanned::Special(len) => (len, None),
        Scanned::Decimal { len, nonzero } => (len, Some(nonzero)),
    };
    let number = &body[..len];
    let value = parse(number).ok_or_else(syntax)?;
    // The range verdict comes ahead of the trailing-text check, as `float8in_internal`'s
    // `ERANGE` arm does: the number alone decides it.
    if let Some(nonzero) = decimal
        && (value.is_infinite() || (nonzero && value.is_zero()))
    {
        return Err(Error::FloatInputOutOfRange {
            value: number.to_string(),
            type_name,
        });
    }
    if !body.as_bytes()[len..].iter().copied().all(is_c_space) {
        return Err(syntax());
    }
    Ok(value)
}

/// C's `isspace` in the `C` locale — the bytes `float8in` skips around a number.
const fn is_c_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// What the start of a float's text holds, as `strtod` would read it.
enum Scanned {
    /// No number: `strtod` consumes nothing.
    Nothing,
    /// An infinity or NaN spelling of this many bytes, sign included.
    Special(usize),
    /// A decimal of this many bytes, sign and exponent included, and whether any of its
    /// mantissa digits is nonzero.
    Decimal { len: usize, nonzero: bool },
}

/// The longest prefix of `s` that `strtod` reads: an optional sign, then `infinity` /
/// `inf` / `nan` in any case, or a decimal — digits with an optional `.` and fraction
/// (at least one digit on either side), then an exponent only where its digits follow.
fn scan_number(s: &[u8]) -> Scanned {
    let mut i = usize::from(matches!(s.first(), Some(b'+' | b'-')));
    let rest = &s[i..];
    let starts =
        |word: &[u8]| rest.len() >= word.len() && rest[..word.len()].eq_ignore_ascii_case(word);
    if starts(b"infinity") {
        return Scanned::Special(i + 8);
    }
    if starts(b"inf") || starts(b"nan") {
        return Scanned::Special(i + 3);
    }
    let mut digits = 0usize;
    let mut nonzero = false;
    let mut eat_digits = |i: &mut usize| {
        while let Some(&d) = s.get(*i).filter(|b| b.is_ascii_digit()) {
            nonzero |= d != b'0';
            digits += 1;
            *i += 1;
        }
    };
    eat_digits(&mut i);
    if s.get(i) == Some(&b'.') {
        i += 1;
        eat_digits(&mut i);
    }
    if digits == 0 {
        return Scanned::Nothing;
    }
    if matches!(s.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(s.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if s.get(j).is_some_and(u8::is_ascii_digit) {
            while s.get(j).is_some_and(u8::is_ascii_digit) {
                j += 1;
            }
            i = j;
        }
    }
    Scanned::Decimal { len: i, nonzero }
}

#[cfg(test)]
mod tests {
    use super::{parse_float4_text, parse_float8_text};

    fn code8(s: &str) -> String {
        parse_float8_text(s).map_or_else(|e| e.sqlstate().to_string(), |v| format!("{v:?}"))
    }

    fn code4(s: &str) -> String {
        parse_float4_text(s).map_or_else(|e| e.sqlstate().to_string(), |v| format!("{v:?}"))
    }

    /// 18.6's answers, measured with `SELECT '<text>'::float8` / `::real`.
    #[test]
    fn reads_as_18_6_does() {
        for (input, want) in [
            ("1e309", "22003"),
            ("-1e309", "22003"),
            ("1e-400", "22003"),
            ("2e-324", "22003"),
            ("4e-324", "5e-324"),
            ("0e-999", "0.0"),
            ("0.0e999", "0.0"),
            ("-0", "-0.0"),
            (" 1.5 ", "1.5"),
            (".5", "0.5"),
            ("5.", "5.0"),
            ("1e", "22P02"),
            ("1_000", "22P02"),
            ("", "22P02"),
            ("   ", "22P02"),
            ("infinit", "22P02"),
            ("infinityx", "22P02"),
            ("Infinity", "inf"),
            ("-inf", "-inf"),
            ("+INFINITY", "inf"),
            ("NaN", "NaN"),
            ("-nan", "NaN"),
            ("0x1A", "22P02"),
            ("\u{a0}1", "22P02"),
        ] {
            assert_eq!(code8(input), want, "float8 {input:?}");
        }
        for (input, want) in [
            ("1e39", "22003"),
            ("-1e39", "22003"),
            ("1e-50", "22003"),
            ("3.4028236e38", "22003"),
            ("1e-40", "1e-40"),
            ("1e39x", "22003"),
            ("1e39 x", "22003"),
            ("1.5x", "22P02"),
            ("inf", "inf"),
            ("+.5e+1", "5.0"),
        ] {
            assert_eq!(code4(input), want, "real {input:?}");
        }
    }

    /// The message names the number as the reader consumed it, not the input.
    #[test]
    fn the_range_refusal_names_the_number() {
        let e = parse_float4_text(" 1e39x").expect_err("out of range");
        assert_eq!(e.to_string(), "\"1e39\" is out of range for type real");
        let e = parse_float8_text("1e309").expect_err("out of range");
        assert_eq!(
            e.to_string(),
            "\"1e309\" is out of range for type double precision"
        );
        let e = parse_float8_text("1e309 x").expect_err("out of range");
        assert_eq!(e.sqlstate(), "22003");
    }
}
