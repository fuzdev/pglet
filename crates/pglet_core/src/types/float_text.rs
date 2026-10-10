//! A `float4` / `float8`'s text, as 18.6's `float4out` / `float8out` write it at the default
//! `extra_float_digits` — `src/common/f2s.c` / `d2s.c`, Ryu with `acceptBounds = false`.
//!
//! Three rules decide it, and each is reproduced here rather than borrowed from Rust's own
//! shortest formatting, which differs at each:
//!
//! - **The digits** are the fewest that name the value from **strictly inside** its rounding
//!   interval — a spelling exactly on the interval's boundary is never answered, even where it
//!   would read back as the same value — and among spellings of that length the one nearest the
//!   value, an exact decimal tie going to the even digit. Rust accepts the boundary on an even
//!   mantissa and rounds a tie up, so `-84197376::float4` is `-84197380` there and
//!   `-8.4197376e+07` here, and `155.390625::float4` `155.39063` there and `155.39062` here.
//! - **An integer below 2^24 (`float4`) or 2^53 (`float8`) is spelled by its own digits**,
//!   trailing zeros dropped (`f2d_small_int`), then laid out by the next rule: `100000` and
//!   `1.6777215e+07`.
//! - **Fixed point when the leading digit's power of ten is in `-4 ..= 5` (`float4`) or
//!   `-4 ..= 14` (`float8`), exponent form otherwise**, with at least two exponent digits:
//!   `1e+20`, `1e-10`, `1.5e+07`.
//!
//! Rust's own shortest spelling answers when it lies strictly inside the interval and the value
//! is not exactly halfway between it and a neighbour of the same length — then it is the nearest
//! spelling of the shortest length, which is 18.6's. Otherwise the digits are found by asking,
//! for each length from Rust's shortest one up, whether the correctly rounded spelling of that
//! length (Rust's exact formatter, whose ties go to even) — or, in a power of two's lopsided
//! interval, its neighbour on the value's other side — reads back as the value and is not a
//! boundary. The boundary and tie tests are exact integer arithmetic over the value's mantissa
//! and exponent. No table and no dependency. Measured equal to 18.6's text over a random and
//! edge-class corpus of each width (`EDGES.md` row 255); what 18.6's exact digits cost over
//! Rust's shortest spelling is `TRADEOFFS.md`'s.
//!
//! @module

/// The text of a `float4`, as 18.6's `float4out` writes it.
#[must_use]
pub fn format_float4_text(f: f32) -> String {
    let bits = f.to_bits();
    let exp_bits = (bits >> 23) & 0xff;
    let mant_bits = u64::from(bits & 0x7f_ffff);
    let parts = FloatParts {
        negative: bits >> 31 == 1,
        value: f64::from(f.abs()),
        mantissa: if exp_bits == 0 {
            mant_bits
        } else {
            mant_bits | (1 << 23)
        },
        exp2: if exp_bits == 0 {
            -149
        } else {
            i32::try_from(exp_bits).unwrap_or(0) - 150
        },
        lower_is_closer: mant_bits == 0 && exp_bits > 1,
        small_int_bits: 24,
        max_digits: 9,
        fixed_below: 6,
    };
    render(
        f.is_nan(),
        f.is_infinite(),
        f == 0.0,
        &parts,
        |n| {
            n.map_or_else(
                || format!("{:e}", f.abs()),
                |n| format!("{:.*e}", n - 1, f.abs()),
            )
        },
        |s| {
            s.parse::<f32>()
                .is_ok_and(|v| v.to_bits() == f.abs().to_bits())
        },
    )
}

/// The text of a `float8`, as 18.6's `float8out` writes it.
#[must_use]
pub fn format_float8_text(f: f64) -> String {
    let bits = f.to_bits();
    let exp_bits = (bits >> 52) & 0x7ff;
    let mant_bits = bits & 0xf_ffff_ffff_ffff;
    let parts = FloatParts {
        negative: bits >> 63 == 1,
        value: f.abs(),
        mantissa: if exp_bits == 0 {
            mant_bits
        } else {
            mant_bits | (1 << 52)
        },
        exp2: if exp_bits == 0 {
            -1074
        } else {
            i32::try_from(exp_bits).unwrap_or(0) - 1075
        },
        lower_is_closer: mant_bits == 0 && exp_bits > 1,
        small_int_bits: 53,
        max_digits: 17,
        fixed_below: 15,
    };
    render(
        f.is_nan(),
        f.is_infinite(),
        f == 0.0,
        &parts,
        |n| {
            n.map_or_else(
                || format!("{:e}", f.abs()),
                |n| format!("{:.*e}", n - 1, f.abs()),
            )
        },
        |s| {
            s.parse::<f64>()
                .is_ok_and(|v| v.to_bits() == f.abs().to_bits())
        },
    )
}

/// A finite nonzero value as `mantissa · 2^exp2`, and what its text needs to know about it.
struct FloatParts {
    negative: bool,
    /// The magnitude, exact as an `f64` for either width.
    value: f64,
    mantissa: u64,
    exp2: i32,
    /// The value's lower neighbour is nearer than its upper one — a power of two above the
    /// smallest normal exponent — so the interval's lower half is half as wide.
    lower_is_closer: bool,
    /// The mantissa width: an integer below `2^small_int_bits` is written whole.
    small_int_bits: u32,
    max_digits: usize,
    /// The first power of ten of the leading digit written in exponent form.
    fixed_below: i32,
}

fn render(
    nan: bool,
    infinite: bool,
    zero: bool,
    parts: &FloatParts,
    rounded: impl Fn(Option<usize>) -> String,
    reads_back: impl Fn(&str) -> bool,
) -> String {
    let sign = if parts.negative { "-" } else { "" };
    if nan {
        return "NaN".to_string();
    }
    if infinite {
        return format!("{sign}Infinity");
    }
    if zero {
        return format!("{sign}0");
    }
    let (digits, exp10) =
        small_int_digits(parts).unwrap_or_else(|| shortest_digits(parts, &rounded, &reads_back));
    let mut text = layout(&digits, exp10, parts.fixed_below);
    if parts.negative {
        text.insert(0, '-');
    }
    text
}

/// An integer below the mantissa width, written whole (`f2d_small_int` / `d2d_small_int`).
fn small_int_digits(parts: &FloatParts) -> Option<(String, i32)> {
    if parts.exp2 > 0 || parts.exp2 < -i32::try_from(parts.small_int_bits - 1).unwrap_or(0) {
        return None;
    }
    let shift = parts.exp2.unsigned_abs();
    if parts.mantissa & ((1u64 << shift) - 1) != 0 {
        return None;
    }
    let int = parts.mantissa >> shift;
    if int == 0 {
        return None;
    }
    let digits = int.to_string();
    let exp10 = i32::try_from(digits.len()).unwrap_or(0) - 1;
    Some((digits, exp10))
}

/// The fewest digits naming the value from strictly inside its rounding interval, and the
/// power of ten of the first.
fn shortest_digits(
    parts: &FloatParts,
    rounded: &impl Fn(Option<usize>) -> String,
    reads_back: &impl Fn(&str) -> bool,
) -> (String, i32) {
    // Every spelling this rule accepts, Rust's own shortest (`rounded(None)`) accepts too — it
    // only adds the even-mantissa boundary — so no length below Rust's can serve: start there.
    let shortest = rounded(None);
    let (mantissa, exp) = shortest.split_once('e').unwrap_or((&shortest, "0"));
    let mut rust_len = 0usize;
    let mut d = 0u64;
    for b in mantissa.bytes().filter(u8::is_ascii_digit) {
        rust_len += 1;
        d = d.saturating_mul(10).saturating_add(u64::from(b - b'0'));
    }
    let rust_len = rust_len.max(1);
    // The common case, answered from that one spelling: Rust's shortest is the nearest spelling
    // of its length inside the closed interval, so when it is strictly inside and the value is
    // not exactly halfway between it and a neighbour (where Rust rounds up and 18.6 to even),
    // it is what the search below would answer at this same length.
    if let Ok(exp10) = exp.parse::<i32>()
        && rust_len <= parts.max_digits
    {
        let k = exp10 - (i32::try_from(rust_len).unwrap_or(0) - 1);
        if !on_boundary(parts, d, k) && !is_tie(parts, d, k) {
            // A shortest spelling never ends in a zero: dropping it would be shorter still.
            return (d.to_string(), exp10);
        }
    }
    let mut last = (String::new(), 0);
    for n in rust_len..=parts.max_digits {
        let spelled = rounded(Some(n));
        let (mantissa, exp) = spelled.split_once('e').unwrap_or((&spelled, "0"));
        let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
        let exp10: i32 = exp.parse().unwrap_or(0);
        last = (digits.clone(), exp10);
        let Ok(d) = digits.parse::<u64>() else {
            continue;
        };
        let k = exp10 - (i32::try_from(n).unwrap_or(0) - 1);
        let inside = |d: u64, k: i32| reads_back(&format!("{d}e{k}")) && !on_boundary(parts, d, k);
        if inside(d, k) {
            return (strip_trailing_zeros(&digits), exp10);
        }
        // The nearest spelling of this length falls outside a lopsided interval (a power of two,
        // whose lower half is half as wide): the next one on the value's other side can still be
        // inside, and Ryu answers it at this length (`vr + 1`) rather than spending a digit.
        let below = spelled_below(&format!("{d}e{k}"), parts);
        let other = if below {
            d.checked_add(1)
        } else {
            d.checked_sub(1)
        };
        if let Some(o) = other
            && o.to_string().len() == digits.len()
            && inside(o, k)
        {
            return (strip_trailing_zeros(&o.to_string()), exp10);
        }
    }
    (strip_trailing_zeros(&last.0), last.1)
}

/// Whether the decimal `spelled` is below the value — read as an `f64`, where every `float4`
/// is exact; for a `float8`, a spelling that misses reads as a different `f64` whose side it
/// is, and one on a boundary reads as the even neighbour, which is the side it is on.
fn spelled_below(spelled: &str, parts: &FloatParts) -> bool {
    spelled.parse::<f64>().is_ok_and(|v| v < parts.value)
}

fn strip_trailing_zeros(digits: &str) -> String {
    let trimmed = digits.trim_end_matches('0');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Whether `d · 10^k` is exactly one of the two ends of the value's rounding interval.
fn on_boundary(parts: &FloatParts, d: u64, k: i32) -> bool {
    let m = u128::from(parts.mantissa);
    let e = parts.exp2;
    let upper = (2 * m + 1, e - 1);
    let lower = if parts.lower_is_closer {
        (4 * m - 1, e - 2)
    } else {
        (2 * m - 1, e - 1)
    };
    [upper, lower]
        .into_iter()
        .any(|(odd, q)| decimal_equals_binary(d, k, odd, q))
}

/// Whether the value lies exactly halfway between `d · 10^k` and a neighbour `(d ± 1) · 10^k`:
/// `(2d ± 1) · 10^k == mantissa · 2^(exp2 + 1)`.
fn is_tie(parts: &FloatParts, d: u64, k: i32) -> bool {
    let tz = parts.mantissa.trailing_zeros();
    let odd = u128::from(parts.mantissa >> tz);
    let q = parts.exp2 + 1 + i32::try_from(tz).unwrap_or(0);
    [
        d.checked_mul(2).and_then(|x| x.checked_add(1)),
        d.checked_mul(2).and_then(|x| x.checked_sub(1)),
    ]
    .into_iter()
    .flatten()
    .any(|twice| decimal_equals_binary(twice, k, odd, q))
}

/// Whether `d · 10^k == odd · 2^q`, `odd` odd — exactly, in integers.
fn decimal_equals_binary(d: u64, k: i32, odd: u128, q: i32) -> bool {
    if d == 0 {
        return false;
    }
    let tz = i32::try_from(d.trailing_zeros()).unwrap_or(0);
    let d_odd = u128::from(d >> d.trailing_zeros());
    let five_pow = |p: u32| 5u128.checked_pow(p);
    if k >= 0 {
        // d · 10^k = (d_odd · 5^k) · 2^(k + tz)
        tz + k == q
            && five_pow(k.unsigned_abs())
                .and_then(|p| d_odd.checked_mul(p))
                .is_some_and(|lhs| lhs == odd)
    } else {
        // d · 10^k = (d_odd / 5^-k) · 2^(tz + k), which must be an odd integer
        tz + k == q
            && five_pow(k.unsigned_abs()).is_some_and(|p| d_odd % p == 0 && d_odd / p == odd)
    }
}

/// `digits` with the first at `10^exp10`, laid out as `to_chars` lays it out.
fn layout(digits: &str, exp10: i32, fixed_below: i32) -> String {
    let len = i32::try_from(digits.len()).unwrap_or(1);
    if (-4..fixed_below).contains(&exp10) {
        if exp10 < 0 {
            let zeros = "0".repeat(usize::try_from(-exp10 - 1).unwrap_or(0));
            return format!("0.{zeros}{digits}");
        }
        if len <= exp10 + 1 {
            let zeros = "0".repeat(usize::try_from(exp10 + 1 - len).unwrap_or(0));
            return format!("{digits}{zeros}");
        }
        let point = usize::try_from(exp10 + 1).unwrap_or(1);
        return format!("{}.{}", &digits[..point], &digits[point..]);
    }
    let digits = strip_trailing_zeros(digits);
    let (first, rest) = digits.split_at(1);
    let mantissa = if rest.is_empty() {
        first.to_string()
    } else {
        format!("{first}.{rest}")
    };
    let sign = if exp10 < 0 { '-' } else { '+' };
    format!("{mantissa}e{sign}{:02}", exp10.unsigned_abs())
}

#[cfg(test)]
mod tests {
    use super::{format_float4_text, format_float8_text};

    /// 18.6's answers, measured with `SELECT <v>::float4` / `::float8`.
    // Each literal is the exact value measured, not a rounding of it.
    #[allow(clippy::excessive_precision)]
    #[test]
    fn answers_as_18_6_does() {
        for (v, want) in [
            (155.390_625_f32, "155.39062"),
            (-84_197_376.0, "-8.4197376e+07"),
            (1_636_696_064.0, "1.6366961e+09"),
            (1.1, "1.1"),
            (3e38, "3e+38"),
            (0.0001, "0.0001"),
            (1e-5, "1e-05"),
            (100_000.0, "100000"),
            (1_000_000.0, "1e+06"),
            (16_777_215.0, "1.6777215e+07"),
            (-0.0, "-0"),
            // a power of two, whose interval is lopsided
            (1.262_177_448_353_619e-29, "1.2621775e-29"),
            (1.547_425_049_106_725_3e26, "1.5474251e+26"),
            // Rust's shortest on the interval's boundary
            (89_317_900.0, "8.9317904e+07"),
        ] {
            assert_eq!(format_float4_text(v), want, "{v:?}");
        }
        for (v, want) in [
            (1e20_f64, "1e+20"),
            (1e-10, "1e-10"),
            (0.1, "0.1"),
            (1e15, "1e+15"),
            (123_456_789_012_345.0, "123456789012345"),
            (5e-324, "5e-324"),
            (f64::MAX, "1.7976931348623157e+308"),
            (f64::NAN, "NaN"),
            (f64::NEG_INFINITY, "-Infinity"),
            // 2^-24: a tie at Rust's length, whose nearest even spelling falls outside the
            // lopsided interval — the one the search's neighbour answers
            (5.960_464_477_539_063e-8, "5.960464477539063e-08"),
            (7.120_236_347_223_045e-307, "7.120236347223045e-307"),
            (-2.596_472_244_501_881e16, "-2.5964722445018808e+16"),
        ] {
            assert_eq!(format_float8_text(v), want, "{v:?}");
        }
    }
}
