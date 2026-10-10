//! In-house arbitrary-precision decimal — the `NUMERIC`/`DECIMAL` value type.
//!
//! `PgNumeric` is a dependency-free exact decimal, matching Postgres `numeric`
//! semantics (exact base-10 arithmetic, a separate *display scale*, and a `NaN`
//! that sorts above every finite value and equals itself). It backs
//! [`Value::Numeric`](super::Value::Numeric).
//!
//! # Representation
//!
//! A finite value is a sign, a vector of **base-10** significant digits
//! (`0..=9`, most-significant first, normalized to strip leading and trailing
//! zeros), a `weight` (the power-of-ten place of the first digit), and a
//! `dscale` (how many fractional digits to *display*). So `1.50` is
//! `digits=[1,5]`, `weight=0`, `dscale=2` — the value `1.5` shown with two
//! fractional places. Zero is the empty digit vector.
//!
//! Base-10 (rather than Postgres's base-10000 `NBASE`) keeps parsing,
//! formatting, rounding, and long-hand arithmetic auditable; the base-10000
//! regrouping Postgres's binary wire format needs happens only at that boundary
//! ([`PgNumeric::to_wire_bytes`] / [`PgNumeric::recv`]).
//!
//! The scale rules follow `numeric.c`: add/sub display at `max(dscale)`, multiply
//! at `dscale1 + dscale2` (exact), and divide at a scale chosen to keep at least
//! `NUMERIC_MIN_SIG_DIGITS` significant digits (Postgres's `select_div_scale`).
//! Rounding is half-away-from-zero, as Postgres numeric rounds.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use super::BinaryReader;
use crate::error::{Error, Result};

/// Postgres `NUMERIC_MIN_SIG_DIGITS` — the significant-digit floor a division
/// result targets (so `numeric` division is no less accurate than `float8`).
const MIN_SIG_DIGITS: i32 = 16;
/// Postgres `NUMERIC_MAX_DISPLAY_SCALE` — the cap on a division result's scale.
const MAX_DISPLAY_SCALE: i32 = 1000;
/// Postgres `NUMERIC_WEIGHT_MAX` — the largest representable base-10000 weight,
/// bounding `power_var_int`'s crude overflow guard. (`DEC_DIGITS`, the decimal
/// digits per base-10000 limb, is defined near `select_div_scale`.)
const NUMERIC_WEIGHT_MAX: i32 = 0x7FFF;
/// The largest base-10 weight (power-of-ten position of the most significant
/// digit) a finite numeric result may carry. Postgres's `make_result` stores the
/// base-10000 weight in an `int16` capped at `NUMERIC_WEIGHT_MAX`, whose
/// most-significant decimal digit sits at position
/// `(NUMERIC_WEIGHT_MAX + 1) * DEC_DIGITS - 1`; a result past it raises "value
/// overflows numeric format" (`22003`).
const NUMERIC_MAX_WEIGHT: i32 = (NUMERIC_WEIGHT_MAX + 1) * DEC_DIGITS - 1;
/// The most-negative base-10 weight a finite numeric result may carry, mirroring
/// `make_result`'s lower `int16` bound on the base-10000 weight
/// (`PG_INT16_MIN = -(NUMERIC_WEIGHT_MAX + 1)`), whose least-significant base-10
/// position is `-(NUMERIC_WEIGHT_MAX + 1) * DEC_DIGITS`. In practice a literal hits
/// the tighter *display-scale* cap ([`NUMERIC_DSCALE_MAX`], enforced in
/// `parse_decimal`) first — a value whose weight is this negative needs a display
/// scale far past `NUMERIC_DSCALE_MAX` to render a non-zero digit — so this bound is
/// a defensive guard on arithmetic results, symmetric with `NUMERIC_MAX_WEIGHT`.
const NUMERIC_MIN_WEIGHT: i32 = -(NUMERIC_WEIGHT_MAX + 1) * DEC_DIGITS;
/// Postgres `NUMERIC_DSCALE_MAX` (`NUMERIC_DSCALE_MASK`, `0x3FFF`) — the largest
/// display scale a stored numeric can hold, and the upper clamp `numeric_round`
/// /`numeric_trunc` place on their scale argument before rounding.
const NUMERIC_DSCALE_MAX: i32 = 0x3FFF;

/// The largest exponent `set_var_from_str` reads before it gives up — `PG_INT32_MAX / 2`,
/// 18.6's own bound on the exponent digits it accumulates, there to keep the weight and
/// scale arithmetic below it inside an `int32`.
///
/// It is **not** a range check on the value, and reading it as one gets two cases
/// wrong. Past it 18.6 answers `22003 value overflows numeric format` however small the
/// value would be (`0e1073741824` is `22003`, `0e1073741823` is `0` — measured), and it
/// is raised *inside* the exponent's digit run, so a spelling that is also malformed
/// never reaches the syntax check: `'1e99999999999x'::numeric` is `22003` and
/// `'1e1073741823x'::numeric` — whose exponent stays inside the bound — is `22P02`.
const NUMERIC_EXPONENT_MAX: i64 = (i32::MAX / 2) as i64;

/// An arbitrary-precision exact decimal (`NUMERIC`/`DECIMAL`).
///
/// See the module docs for the representation and the scale rules. Value
/// equality (not structural): `1.5` equals `1.50`, and `NaN` equals `NaN`
/// (Postgres numeric semantics, unlike IEEE floats).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PgNumeric {
    /// Not-a-number. Equals itself and sorts above every finite value.
    NaN,
    /// A finite decimal.
    Num(NumericVar),
}

/// A finite decimal value: sign + normalized base-10 digits + weight + display scale.
///
/// Constructed only through [`PgNumeric`]; the fields stay private so the
/// normalization invariant (no leading/trailing zero digits) always holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumericVar {
    /// Sign. Always `false` for zero (no negative zero).
    negative: bool,
    /// Significant base-10 digits (`0..=9`), most-significant first. Normalized:
    /// no leading or trailing zeros. Empty ⇒ the value zero.
    digits: Vec<u8>,
    /// Power-of-ten place of `digits[0]` (e.g. `123.45` → weight `2`, `0.0045` →
    /// weight `-3`). `0` when the value is zero.
    weight: i32,
    /// Display scale: fractional digits to render (independent of the trailing
    /// zeros stripped from `digits`).
    dscale: u16,
}

impl PartialEq for PgNumeric {
    fn eq(&self, other: &Self) -> bool {
        self.cmp_value(other) == Ordering::Equal
    }
}

impl PgNumeric {
    /// The value zero with a display scale of zero.
    #[must_use]
    pub const fn zero() -> Self {
        Self::Num(NumericVar {
            negative: false,
            digits: Vec::new(),
            weight: 0,
            dscale: 0,
        })
    }

    /// Whether this is `NaN`.
    #[must_use]
    pub const fn is_nan(&self) -> bool {
        matches!(self, Self::NaN)
    }

    /// Whether this is a finite zero.
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        matches!(self, Self::Num(v) if v.digits.is_empty())
    }

    /// Exact conversion from a signed 128-bit integer (`display scale 0`).
    #[must_use]
    pub fn from_i128(n: i128) -> Self {
        if n == 0 {
            return Self::zero();
        }
        let negative = n < 0;
        // Format the magnitude; `i128::MIN` has no positive counterpart, so go
        // through the string form (unsigned_abs would need u128 formatting anyway).
        let mag = n.unsigned_abs().to_string();
        let digits: Vec<u8> = mag.bytes().map(|b| b - b'0').collect();
        let mut v = NumericVar {
            negative,
            weight: digits.len() as i32 - 1,
            digits,
            dscale: 0,
        };
        v.normalize();
        Self::Num(v)
    }

    /// Parse a decimal string (optional sign, digits, `.frac`, `e±exp`, or the
    /// literal `NaN`), matching Postgres `numeric` input.
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidInputSyntax`] (`22P02`) for any spelling Postgres rejects.
    /// - [`Error::NumericFormatOverflow`] (`22003`) for a literal past the numeric
    ///   weight or display-scale cap (Postgres `numeric_in` also runs `make_result`),
    ///   or past [`NUMERIC_EXPONENT_MAX`], which `set_var_from_str` refuses whatever
    ///   the value and whatever follows the exponent.
    pub fn parse(s: &str) -> Result<Self> {
        let t = s.trim();
        if t.eq_ignore_ascii_case("nan") {
            return Ok(Self::NaN);
        }
        parse_decimal(t)
            .map_err(|e| match e {
                DecimalError::Syntax => Error::InvalidInputSyntax {
                    type_name: "numeric",
                    value: s.to_string(),
                },
                DecimalError::Overflow => Error::NumericFormatOverflow,
            })?
            .check_weight()
    }

    /// Render as Postgres `numeric` text (`NaN`, or the decimal padded to its
    /// display scale, e.g. `1.50`).
    #[must_use]
    pub fn to_display_string(&self) -> String {
        match self {
            Self::NaN => "NaN".to_string(),
            Self::Num(v) => v.to_display_string(),
        }
    }

    /// Round to `scale` fractional digits (half-away-from-zero) and set the
    /// display scale, in place. `NaN` is unchanged.
    pub fn round_to_scale(&mut self, scale: i32) {
        if let Self::Num(v) = self {
            v.round_to_scale(scale);
        }
    }

    /// Enforce a `NUMERIC(precision, scale)` type modifier: round to `scale`, then
    /// verify the integer-part digit count fits `precision - scale`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NumericOutOfRange`] when the rounded value's integer part
    /// exceeds `precision - scale` digits (Postgres "numeric field overflow").
    pub fn apply_typmod(&mut self, precision: u16, scale: i16) -> Result<()> {
        let Self::Num(v) = self else {
            return Ok(()); // NaN carries no scale/precision
        };
        v.round_to_scale(i32::from(scale));
        // Integer-part digit budget. Postgres rejects when the value needs more
        // than `precision - scale` digits left of the point (weight + 1 digits for
        // a value with `weight >= 0`; zero needs none).
        if !v.digits.is_empty() {
            let int_digits = v.weight + 1;
            let max_int_digits = i32::from(precision) - i32::from(scale);
            if int_digits > max_int_digits {
                return Err(Error::NumericOutOfRange("numeric"));
            }
        }
        Ok(())
    }

    /// Total order matching Postgres: `NaN` sorts above every finite value and
    /// equals itself; finite values compare by signed magnitude. Backs both `=`
    /// and ordering.
    #[must_use]
    pub fn cmp_value(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::NaN, Self::NaN) => Ordering::Equal,
            (Self::NaN, Self::Num(_)) => Ordering::Greater,
            (Self::Num(_), Self::NaN) => Ordering::Less,
            (Self::Num(a), Self::Num(b)) => a.cmp(b),
        }
    }

    /// Negate (in place would need `&mut`; this returns a fresh value). `NaN` and
    /// zero are unchanged (no negative zero).
    #[must_use]
    pub fn negate(&self) -> Self {
        match self {
            Self::NaN => Self::NaN,
            Self::Num(v) => {
                let mut v = v.clone();
                if !v.digits.is_empty() {
                    v.negative = !v.negative;
                }
                Self::Num(v)
            }
        }
    }

    /// Absolute value — clears the sign, keeping the display scale
    /// (`abs(-1.50) = 1.50`). `NaN` is unchanged.
    #[must_use]
    pub fn abs(&self) -> Self {
        match self {
            Self::NaN => Self::NaN,
            Self::Num(v) => {
                let mut v = v.clone();
                v.negative = false;
                Self::Num(v)
            }
        }
    }

    /// Round to `scale` fractional digits (half-away-from-zero), returning a fresh
    /// value — the `round(numeric [, scale])` function. `NaN` is unchanged.
    #[must_use]
    pub fn round(&self, scale: i32) -> Self {
        let mut out = self.clone();
        out.round_to_scale(scale);
        out
    }

    /// Truncate toward zero to `scale` fractional digits (no rounding) — the
    /// `trunc(numeric [, scale])` function. `NaN` is unchanged.
    #[must_use]
    pub fn trunc(&self, scale: i32) -> Self {
        match self {
            Self::NaN => Self::NaN,
            Self::Num(v) => {
                let mut v = v.clone();
                v.trunc_to_scale(scale);
                Self::Num(v)
            }
        }
    }

    /// Round toward +∞ to an integer — `ceil`/`ceiling`. `NaN` is unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NumericOutOfRange`] if a round-up carry overflows the
    /// numeric weight cap (`make_result`, `22003`).
    pub fn ceil(&self) -> Result<Self> {
        self.round_toward_infinity(false)
    }

    /// Round toward −∞ to an integer — `floor`. `NaN` is unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NumericOutOfRange`] if a round-down carry overflows the
    /// numeric weight cap (`make_result`, `22003`).
    pub fn floor(&self) -> Result<Self> {
        self.round_toward_infinity(true)
    }

    /// Shared `ceil`/`floor` body: truncate toward zero, then step one past it when
    /// a fractional part was dropped in the rounding direction (`ceil` steps a
    /// positive up, `floor` steps a negative down).
    fn round_toward_infinity(&self, toward_neg: bool) -> Result<Self> {
        let Self::Num(v) = self else {
            return Ok(Self::NaN);
        };
        let has_fraction = !v.digits.is_empty() && v.low_pos() < 0;
        let truncated = self.trunc(0);
        if has_fraction && v.negative == toward_neg {
            if toward_neg {
                truncated.sub(&Self::from_i128(1))
            } else {
                truncated.add(&Self::from_i128(1))
            }
        } else {
            Ok(truncated)
        }
    }

    /// Fail loud when a finite value's base-10 weight falls outside what Postgres's
    /// numeric format can hold (`make_result`'s symmetric `int16` weight bounds,
    /// `22003` "value overflows numeric format"). `NaN`, zero, and in-range values
    /// pass through unchanged. Applied to every arithmetic result that can grow the
    /// magnitude (`add`/`sub`/`mul`/`div`) — which also caps the O(n²) schoolbook
    /// multiply so a chain of large powers can't build a multi-hundred-thousand-
    /// digit result — to over-cap literals in [`Self::parse`], and (by the caller) to
    /// a `round`/`trunc` whose round-up carry crosses the cap. The lower bound is
    /// symmetric with the upper (mirroring `make_result`); for a literal the
    /// display-scale cap in `parse_decimal` binds first, so the lower bound guards
    /// arithmetic results.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NumericFormatOverflow`] (`22003`, 18.6's own
    /// "value overflows numeric format") when the value's weight is outside
    /// `[NUMERIC_MIN_WEIGHT, NUMERIC_MAX_WEIGHT]`.
    pub fn check_weight(self) -> Result<Self> {
        if let Self::Num(v) = &self
            && !v.digits.is_empty()
            && (v.weight > NUMERIC_MAX_WEIGHT || v.weight < NUMERIC_MIN_WEIGHT)
        {
            return Err(Error::NumericFormatOverflow);
        }
        Ok(self)
    }

    /// Add. `NaN` propagates. Display scale = `max` of the operands'.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NumericOutOfRange`] (`22003`) when the sum overflows the
    /// numeric weight cap (`make_result`).
    pub fn add(&self, other: &Self) -> Result<Self> {
        match (self, other) {
            (Self::NaN, _) | (_, Self::NaN) => Ok(Self::NaN),
            (Self::Num(a), Self::Num(b)) => Self::Num(a.add(b)).check_weight(),
        }
    }

    /// Subtract. `NaN` propagates. Display scale = `max` of the operands'.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NumericOutOfRange`] (`22003`) when the difference overflows
    /// the numeric weight cap (`make_result`).
    pub fn sub(&self, other: &Self) -> Result<Self> {
        self.add(&other.negate())
    }

    /// Multiply — the Postgres `numeric_mul` operator. `NaN` propagates. The exact
    /// product's display scale is `dscale1 + dscale2`, but a stored numeric caps
    /// `dscale` at [`NUMERIC_DSCALE_MAX`] (16383); a product past it is rounded down
    /// to that scale (rounding the *exact* product keeps the result correctly
    /// rounded), then weight-checked — matching `numeric_mul`'s order. So repeatedly
    /// squaring `0.1` folds to exactly `0` once the scale would exceed the cap, as
    /// Postgres does.
    ///
    /// This is the SQL `*` operator; the exact primitive [`Self::mul_exact`] (no
    /// display-scale cap) backs the internal repeated-squaring / Newton loops where
    /// every guard digit must survive.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NumericOutOfRange`] (`22003`) when the product overflows the
    /// numeric weight cap (`make_result`).
    pub fn mul(&self, other: &Self) -> Result<Self> {
        match (self, other) {
            (Self::NaN, _) | (_, Self::NaN) => Ok(Self::NaN),
            (Self::Num(a), Self::Num(b)) => {
                let mut product = a.mul(b);
                if i32::from(product.dscale) > NUMERIC_DSCALE_MAX {
                    product.round_to_scale(NUMERIC_DSCALE_MAX);
                }
                Self::Num(product).check_weight()
            }
        }
    }

    /// Exact multiply with only the weight cap — no `numeric_mul` display-scale
    /// rounding. Postgres's `power_var`/`sqrt_var`/`mod_var` call `mul_var` directly
    /// (not the `numeric_mul` operator), so their guard digits aren't clipped at
    /// [`NUMERIC_DSCALE_MAX`]; this backs those internal paths. `NaN` propagates.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NumericOutOfRange`] (`22003`) when the product overflows the
    /// numeric weight cap (`make_result`).
    fn mul_exact(&self, other: &Self) -> Result<Self> {
        match (self, other) {
            (Self::NaN, _) | (_, Self::NaN) => Ok(Self::NaN),
            (Self::Num(a), Self::Num(b)) => Self::Num(a.mul(b)).check_weight(),
        }
    }

    /// Divide, choosing the result scale like Postgres `select_div_scale`. `NaN`
    /// propagates.
    ///
    /// # Errors
    ///
    /// - [`Error::DivisionByZero`] (`22012`) when the divisor is zero.
    /// - [`Error::NumericOutOfRange`] (`22003`) when the quotient overflows the
    ///   numeric weight cap (`make_result`).
    pub fn div(&self, other: &Self) -> Result<Self> {
        match (self, other) {
            (Self::NaN, _) | (_, Self::NaN) => Ok(Self::NaN),
            (Self::Num(a), Self::Num(b)) => {
                if b.digits.is_empty() {
                    return Err(Error::DivisionByZero);
                }
                Self::Num(a.div(b)).check_weight()
            }
        }
    }

    /// Modulo `self % other` (Postgres `numeric_mod`): `x − trunc(x/y)·y`, so the
    /// remainder takes the sign of the dividend (`(-7) % 3 = -1`, `7 % (-3) = 1`).
    /// `NaN` propagates; a zero divisor is [`Error::DivisionByZero`] (`22012`). The
    /// display scale follows `sub` (`max` of the dividend's and `y·trunc(x/y)`'s).
    ///
    /// # Errors
    ///
    /// Returns [`Error::DivisionByZero`] when `other` is zero.
    pub fn rem(&self, other: &Self) -> Result<Self> {
        match (self, other) {
            (Self::NaN, _) | (_, Self::NaN) => Ok(Self::NaN),
            (Self::Num(_), Self::Num(b)) => {
                if b.digits.is_empty() {
                    return Err(Error::DivisionByZero);
                }
                // mod(x, y) = x − trunc(x/y)·y (Postgres `mod_var`, which divides with
                // `round = false`). The truncated quotient is computed by EXACT
                // integer division — `div` rounds to `select_div_scale` (0 for large
                // operands), so `div().trunc(0)` would corrupt the truncation and give
                // a wrong remainder for operands ≳ 10^16.
                let q = self.trunc_div(other);
                self.sub(&q.mul_exact(other)?)
            }
        }
    }

    /// `trunc(self / other)` — the quotient truncated toward zero to an integer,
    /// computed by EXACT integer division (never `div`, whose `select_div_scale`
    /// rounding would corrupt a large-operand truncation). Scales the operands'
    /// integer coefficients to a common power of ten (`|a| = Da·10^la`,
    /// `|b| = Db·10^lb`, so `floor(|a|/|b|) = floor((Da·10^e)/Db)` for `e = la − lb ≥
    /// 0`, else `floor(Da / (Db·10^-e))`), floor-divides the magnitudes via
    /// [`bigint_div`], and applies the dividend/divisor sign. `other` is nonzero
    /// (the caller checks). Backs [`Self::rem`].
    fn trunc_div(&self, other: &Self) -> Self {
        let (Self::Num(a), Self::Num(b)) = (self, other) else {
            return Self::NaN; // NaN is screened by `rem`; defensive.
        };
        if a.digits.is_empty() {
            return Self::zero(); // 0 / y = 0
        }
        let e = a.low_pos() - b.low_pos();
        let mut num = a.digits.clone();
        let mut den = b.digits.clone();
        if e >= 0 {
            num.extend(std::iter::repeat_n(0u8, e as usize));
        } else {
            den.extend(std::iter::repeat_n(0u8, (-e) as usize));
        }
        let quotient = bigint_div(&num, &den);
        if quotient.is_empty() {
            return Self::zero();
        }
        let mut v = NumericVar {
            negative: a.negative != b.negative,
            weight: quotient.len() as i32 - 1,
            digits: quotient,
            dscale: 0,
        };
        v.normalize();
        Self::Num(v)
    }

    /// Raise `self` to the `exp` power (Postgres `numeric_power`), restricted to
    /// **integer** exponents. `NaN` follows POSIX `pow` (`NaN^0 = 1`, `1^NaN = 1`,
    /// else `NaN`). An integer exponent uses repeated squaring
    /// ([`Self::power_var_int`]); a **non-integer** exponent is handled only for the
    /// two Postgres error/edge cases — `0 ^ fractional = 0`, and a negative base
    /// raised to a non-integer power is [`Error::InvalidArgumentForPowerFunction`]
    /// (`2201F`) — while a positive base with a fractional exponent (the general
    /// `exp(exp · ln(base))` lift) is deferred and fails loud
    /// ([`Error::NotImplemented`], `0A000`).
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidArgumentForPowerFunction`] (`2201F`) — `0 ^ negative`, or a
    ///   negative base with a non-integer exponent.
    /// - [`Error::NumericOutOfRange`] (`22003`) — the result overflows numeric.
    /// - [`Error::NotImplemented`] (`0A000`) — a positive base with a fractional
    ///   exponent (the general ln/exp path, deferred).
    pub fn pow(&self, exp: &Self) -> Result<Self> {
        // NaN handling per POSIX pow(3), matching `numeric_power`.
        match (self, exp) {
            (Self::NaN, _) => {
                if !exp.is_nan() && exp.is_zero() {
                    return Ok(Self::from_i128(1));
                }
                return Ok(Self::NaN);
            }
            (_, Self::NaN) => {
                if *self == Self::from_i128(1) {
                    return Ok(Self::from_i128(1));
                }
                return Ok(Self::NaN);
            }
            (Self::Num(_), Self::Num(_)) => {}
        }
        // An exact integer exponent takes the repeated-squaring path.
        if let Some(iexp) = exp.to_exact_i64() {
            // `0 ^ negative` is a power-domain error (`2201F`), not a division-by-zero
            // — `numeric_power` raises it before reaching `power_var_int`'s internal
            // zero-base handling.
            if self.is_zero() && iexp < 0 {
                return Err(Error::InvalidArgumentForPowerFunction(
                    "zero raised to a negative power is undefined".into(),
                ));
            }
            let e32 = i32::try_from(iexp).map_err(|_| Error::NumericOutOfRange("numeric"))?;
            return self.power_var_int(e32, exp.display_scale());
        }
        // Fractional exponent: only the Postgres error/edge cases are modeled.
        if self.is_zero() {
            // `0 ^ (positive fractional)` → 0 at NUMERIC_MIN_SIG_DIGITS scale.
            let mut z = Self::zero();
            z.round_to_scale(MIN_SIG_DIGITS);
            return Ok(z);
        }
        if self.is_negative() {
            return Err(Error::InvalidArgumentForPowerFunction(
                "a negative number raised to a non-integer power yields a complex result".into(),
            ));
        }
        // Positive base, fractional exponent: the general `exp(exp·ln(base))` bignum
        // lift is deferred (fail loud rather than approximate).
        Err(Error::NotImplemented(
            "numeric ^ non-integer exponent (the general ln/exp path) is not yet supported".into(),
        ))
    }

    /// Raise `self` to an integer power (Postgres `power_var_int`): choose the result
    /// display scale to keep ≥ `NUMERIC_MIN_SIG_DIGITS` significant digits (never
    /// below either input's scale), then compute the exact value by repeated squaring
    /// (inverting for a negative exponent) and round to that scale. `self` is finite
    /// here (`NaN` is screened by [`Self::pow`]).
    ///
    /// # Errors
    ///
    /// - [`Error::DivisionByZero`] (`22012`) — `0 ^ negative`.
    /// - [`Error::NumericOutOfRange`] (`22003`) — the result overflows numeric.
    fn power_var_int(&self, exp: i32, exp_dscale: u16) -> Result<Self> {
        let Self::Num(base) = self else {
            return Ok(Self::NaN);
        };
        // Approximate the result's decimal weight: log10(base^exp) ≈ exp·log10(|base|).
        // `log10_estimate` is overflow-safe, so a base above `f64` range (which
        // `to_f64()` would map to `inf`) still yields a finite weight.
        let f: f64 = if base.digits.is_empty() {
            0.0
        } else {
            f64::from(exp) * self.log10_estimate()
        };
        // Crude overflow/underflow guards (Postgres `NUMERIC_WEIGHT_MAX`·DEC_DIGITS).
        if f > f64::from((NUMERIC_WEIGHT_MAX + 1) * DEC_DIGITS) {
            return Err(Error::NumericOutOfRange("numeric"));
        }
        if f + 1.0 < -f64::from(MAX_DISPLAY_SCALE) {
            let mut z = Self::zero();
            z.round_to_scale(MAX_DISPLAY_SCALE);
            return Ok(z);
        }
        // Result scale: same rule as `power_var` (≥ MIN_SIG_DIGITS sig digits, and
        // not below either input's display scale).
        #[allow(clippy::cast_possible_truncation)]
        let mut rscale = MIN_SIG_DIGITS - f as i32; // C `(int) f` truncates toward zero
        rscale = rscale.max(i32::from(base.dscale));
        rscale = rscale.max(i32::from(exp_dscale));
        rscale = rscale.max(0);
        rscale = rscale.min(MAX_DISPLAY_SCALE);
        // Special/corner cases.
        if exp == 0 {
            return Ok(Self::from_i128(1).round(rscale)); // x^0 = 1 (incl. 0^0)
        }
        if base.digits.is_empty() {
            // 0 ^ exp
            if exp < 0 {
                return Err(Error::DivisionByZero);
            }
            let mut z = Self::zero();
            z.round_to_scale(rscale);
            return Ok(z);
        }
        // Exact repeated squaring of |exp|; invert for a negative exponent.
        let mut acc = Self::from_i128(1);
        let mut squared = self.clone();
        let mut mask = exp.unsigned_abs();
        while mask > 0 {
            if mask & 1 == 1 {
                acc = acc.mul_exact(&squared)?;
            }
            mask >>= 1;
            if mask > 0 {
                squared = squared.mul_exact(&squared)?;
            }
        }
        // `acc` is nonzero (base ≠ 0), so the inverse division can't divide by zero.
        let result = if exp < 0 {
            Self::from_i128(1).div(&acc)?
        } else {
            acc
        };
        Ok(result.round(rscale))
    }

    /// Square root (Postgres `numeric_sqrt`) via Newton's method, with the final
    /// digit fixed EXACTLY. `NaN` → `NaN`; a negative argument is
    /// [`Error::InvalidArgumentForPowerFunction`] (`2201F`, `cannot take square root
    /// of a negative number`). The result display scale keeps ≥
    /// `NUMERIC_MIN_SIG_DIGITS` significant digits (never below the input's scale).
    ///
    /// Newton iterates `x_{n+1} = (x_n + a/x_n)/2` from a magnitude-aware seed (so a
    /// value beyond `f64` range still seeds correctly — the exponent comes from the
    /// numeric's own `weight`, not an `inf`-producing `to_f64()`), then a
    /// closed-form ½-ulp check adjusts the candidate ±1 ulp so the rounded digit is
    /// the true nearest — Newton alone with a fixed guard scale can round the last
    /// digit the wrong way near a boundary (the table-maker's dilemma).
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidArgumentForPowerFunction`] (`2201F`) — a negative argument.
    /// - [`Error::NumericOutOfRange`] (`22003`) — an out-of-range intermediate.
    pub fn sqrt(&self) -> Result<Self> {
        let Self::Num(v) = self else {
            return Ok(Self::NaN);
        };
        let rscale = self.sqrt_rscale();
        if v.digits.is_empty() {
            let mut z = Self::zero();
            z.round_to_scale(rscale);
            return Ok(z);
        }
        if v.negative {
            return Err(Error::InvalidArgumentForPowerFunction(
                "cannot take square root of a negative number".into(),
            ));
        }
        // Magnitude-aware seed: shift `self` by an even power of ten so its mantissa
        // is `f64`-safe (weight 0 or 1), take that mantissa's `f64` sqrt, then shift
        // the seed back by half the exponent — `sqrt(m·10^{2k}) = sqrt(m)·10^k`.
        let k = v.weight.div_euclid(2);
        let mantissa = self.shift_pow10(-2 * k).to_f64().sqrt();
        let mut x = Self::from_f64(mantissa)?.shift_pow10(k);
        if x.is_zero() {
            x = Self::from_i128(1); // guard against a degenerate seed
        }
        // Newton to a few guard digits past the target scale.
        let work_scale = rscale + 4;
        let two = Self::from_i128(2);
        for _ in 0..100 {
            let quotient = self.div(&x)?;
            let mut next = x.add(&quotient)?.div(&two)?;
            next.round_to_scale(work_scale);
            if next == x {
                break;
            }
            x = next;
        }
        // Exact final-digit fix: the correctly rounded root `c` (a multiple of one
        // ulp) satisfies `(c − ½ulp)² ≤ self < (c + ½ulp)²`. Newton lands within a
        // ulp, so at most a couple of ±ulp steps are needed — all via exact
        // `PgNumeric` ops, no `f64`.
        let ulp = Self::ulp_pow10(rscale);
        let half_ulp = Self::half_ulp_pow10(rscale);
        let mut c = x.round(rscale);
        for _ in 0..4 {
            let lo = c.sub(&half_ulp)?;
            let hi = c.add(&half_ulp)?;
            if self.cmp_value(&lo.mul_exact(&lo)?) == Ordering::Less {
                c = c.sub(&ulp)?; // candidate too large
            } else if self.cmp_value(&hi.mul_exact(&hi)?) != Ordering::Less {
                c = c.add(&ulp)?; // self ≥ hi² → candidate too small
            } else {
                break;
            }
        }
        Ok(c.round(rscale))
    }

    /// The result display scale of [`Self::sqrt`] (Postgres `numeric_sqrt`): derived
    /// from the argument's base-10000 weight (`floor(base10_weight / DEC_DIGITS)`) to
    /// keep ≥ `NUMERIC_MIN_SIG_DIGITS` significant digits, but never below the
    /// argument's own display scale.
    fn sqrt_rscale(&self) -> i32 {
        let (weight10, dscale) = match self {
            Self::NaN => return 0,
            Self::Num(v) => (v.weight, v.dscale),
        };
        // Postgres works in base-10000 (`DEC_DIGITS = 4`): its weight is the base-10
        // weight floored to a 4-digit group, and sweight ≈ that weight's digit count.
        let pg_weight = div_floor(weight10, DEC_DIGITS);
        let sweight = pg_weight * (DEC_DIGITS / 2) + 1;
        let mut rscale = MIN_SIG_DIGITS - sweight;
        rscale = rscale.max(i32::from(dscale));
        rscale = rscale.max(0);
        rscale = rscale.min(MAX_DISPLAY_SCALE);
        rscale
    }

    /// An **overflow-safe** `log10(|self|)` estimate: the leading digits form an
    /// f64 mantissa in `[1, 10)` and the base-10 `weight` supplies the exponent, so
    /// this never rounds through an out-of-`f64`-range magnitude (`to_f64()` on a
    /// value above ~`1.8e308` would be `inf`, poisoning the weight estimate). `NaN`
    /// → `NaN`; zero → `-inf` (`log10(0)`). Used to size `power_var_int`'s result
    /// scale and (halved) `sqrt`'s seed exponent.
    fn log10_estimate(&self) -> f64 {
        let Self::Num(v) = self else {
            return f64::NAN;
        };
        if v.digits.is_empty() {
            return f64::NEG_INFINITY;
        }
        // mantissa = d0.d1d2… ∈ [1, 10); |self| ≈ mantissa · 10^weight.
        let mut mantissa = 0f64;
        for (i, &d) in v.digits.iter().take(16).enumerate() {
            // `mul_add` fuses the rounding step and so changes the estimate's bits
            #[allow(clippy::suboptimal_flops)]
            {
                mantissa += f64::from(d) * 10f64.powi(-(i as i32));
            }
        }
        mantissa.log10() + f64::from(v.weight)
    }

    /// Multiply by `10^k` (a decimal-point shift, exact): the magnitude and digits
    /// are unchanged, only the `weight`/`dscale` move. Used to bring a huge/tiny
    /// value into `f64` range for `sqrt`'s Newton seed and to shift the seed back.
    fn shift_pow10(&self, k: i32) -> Self {
        match self {
            Self::NaN => Self::NaN,
            Self::Num(v) if v.digits.is_empty() => Self::zero(),
            Self::Num(v) => {
                let mut v = v.clone();
                v.weight += k;
                v.dscale = (i32::from(v.dscale) - k).clamp(0, i32::from(u16::MAX)) as u16;
                Self::Num(v)
            }
        }
    }

    /// `10^(-scale)` — one unit in the last place at display scale `scale` (`scale ≥
    /// 0` for `sqrt`). A single digit `1` at position `-scale`.
    fn ulp_pow10(scale: i32) -> Self {
        Self::Num(NumericVar {
            negative: false,
            digits: vec![1],
            weight: -scale,
            dscale: scale.max(0).clamp(0, i32::from(u16::MAX)) as u16,
        })
    }

    /// `½·10^(-scale)` = `5·10^(-(scale+1))` — half a ulp, for `sqrt`'s exact
    /// final-digit correction.
    fn half_ulp_pow10(scale: i32) -> Self {
        Self::Num(NumericVar {
            negative: false,
            digits: vec![5],
            weight: -(scale + 1),
            dscale: (scale + 1).max(0).clamp(0, i32::from(u16::MAX)) as u16,
        })
    }

    /// The display scale (fractional digits rendered). `NaN` reports `0`.
    #[must_use]
    pub const fn display_scale(&self) -> u16 {
        match self {
            Self::NaN => 0,
            Self::Num(v) => v.dscale,
        }
    }

    /// Whether this is a strictly negative finite value (`NaN` and zero are not).
    #[must_use]
    pub const fn is_negative(&self) -> bool {
        matches!(self, Self::Num(v) if v.negative && !v.digits.is_empty())
    }

    /// If this is an **exact** integer that fits `i64`, its value; `None` for `NaN`
    /// or any value with a fractional part (no rounding — a fractional value is not
    /// an integer). Used to detect an integer power exponent.
    #[must_use]
    pub fn to_exact_i64(&self) -> Option<i64> {
        let Self::Num(v) = self else {
            return None;
        };
        if v.digits.is_empty() {
            return Some(0);
        }
        if v.low_pos() < 0 {
            return None; // has fractional digits
        }
        // Integer value (low_pos ≥ 0): `to_i128` is exact here (rounding is a no-op).
        self.to_i128().ok().and_then(|n| i64::try_from(n).ok())
    }

    /// Convert to `f64` (lossy) — the `numeric::float8` cast. `NaN` maps to a
    /// float `NaN`.
    #[must_use]
    pub fn to_f64(&self) -> f64 {
        match self {
            Self::NaN => f64::NAN,
            Self::Num(_) => self.to_display_string().parse().unwrap_or(f64::NAN),
        }
    }

    /// Convert from `f64` (via its shortest round-trip decimal spelling, so a
    /// human-written literal round-trips exactly). Non-finite: `NaN` → `NaN`; an
    /// infinity has no `numeric` form.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NumericOutOfRange`] for `±inf`.
    pub fn from_f64(f: f64) -> Result<Self> {
        if f.is_nan() {
            return Ok(Self::NaN);
        }
        if f.is_infinite() {
            return Err(Error::NumericOutOfRange("numeric"));
        }
        // Rust's default float formatting is the shortest string that round-trips,
        // so `0.1_f64` becomes exactly "0.1" (not the full binary expansion).
        Self::parse(&format_shortest_f64(f))
    }

    /// Encode to an **order-preserving** byte key for a B-tree secondary index:
    /// the raw bytes sort in the same order as [`PgNumeric::cmp_value`], and equal
    /// values (`1.5` == `1.50`) encode identically (so a unique index collides
    /// them, as Postgres does).
    ///
    /// Layout — a class byte orders the four value classes (negative `0x00` <
    /// zero `0x01` < positive `0x02` < `NaN` `0x03`), then, for a finite nonzero
    /// value, a 4-byte sign-flipped big-endian `weight` (magnitude exponent),
    /// each significant digit as `digit + 1` (`1..=10`, reserving `0x00`), and a
    /// `0x00` terminator so a shorter mantissa sorts before a longer one that
    /// extends it. A negative value bitwise-inverts that whole body so larger
    /// magnitudes sort earlier (`-10` < `-1`).
    #[must_use]
    pub fn encode_order_key(&self) -> Vec<u8> {
        match self {
            Self::NaN => vec![0x03],
            Self::Num(v) if v.digits.is_empty() => vec![0x01],
            Self::Num(v) => {
                let mut body = Vec::with_capacity(5 + v.digits.len());
                body.extend_from_slice(&((v.weight as u32) ^ 0x8000_0000).to_be_bytes());
                body.extend(v.digits.iter().map(|&d| d + 1));
                body.push(0x00); // terminator (below every `digit + 1`)
                let mut key = Vec::with_capacity(1 + body.len());
                if v.negative {
                    key.push(0x00);
                    key.extend(body.iter().map(|b| !b));
                } else {
                    key.push(0x02);
                    key.extend(body);
                }
                key
            }
        }
    }

    /// Convert to `i128`, rounding to an integer (half-away-from-zero) — the
    /// `numeric::int8`/`int4` cast path. `NaN` has no integer form.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NumericOutOfRange`] for `NaN` or a value outside `i128`.
    pub fn to_i128(&self) -> Result<i128> {
        let Self::Num(v) = self else {
            return Err(Error::NumericOutOfRange("numeric"));
        };
        let mut r = v.clone();
        r.round_to_scale(0);
        if r.digits.is_empty() {
            return Ok(0);
        }
        // Rebuild the integer string from the rounded digits (weight ≥ 0 here).
        let mut s = String::with_capacity(r.digits.len() + 2);
        if r.negative {
            s.push('-');
        }
        for &d in &r.digits {
            s.push((b'0' + d) as char);
        }
        // Trailing zeros between the last digit and the units place.
        for _ in 0..(r.weight + 1 - r.digits.len() as i32).max(0) {
            s.push('0');
        }
        s.parse::<i128>()
            .map_err(|_| Error::NumericOutOfRange("numeric"))
    }
}

impl NumericVar {
    /// The base-10 digit at power-of-ten position `pos` (0 outside the stored
    /// range), so callers can iterate a common position window over two operands.
    fn digit_at(&self, pos: i32) -> u8 {
        let i = self.weight - pos;
        if i >= 0 && (i as usize) < self.digits.len() {
            self.digits[i as usize]
        } else {
            0
        }
    }

    /// Position of the least significant stored digit (the value's low bound).
    const fn low_pos(&self) -> i32 {
        if self.digits.is_empty() {
            self.weight
        } else {
            self.weight - (self.digits.len() as i32 - 1)
        }
    }

    /// Strip leading and trailing zero digits, keeping `weight` consistent; a
    /// wholly-zero value collapses to the canonical zero (empty digits, weight 0,
    /// sign cleared). `dscale` is untouched.
    ///
    /// The leading run is counted and dropped in **one** `drain`, not one
    /// `remove(0)` a digit: `remove(0)` shifts the whole tail down a place, so a
    /// spelling's leading zeros cost time quadratic in their own count, and the
    /// zeros carry no value at all. `the_leading_zero_strip_stays_within_a_wall_clock_bound`
    /// is the guard.
    fn normalize(&mut self) {
        // Leading zeros: dropping one lowers the first digit's place by one.
        let lead = self.digits.iter().take_while(|d| **d == 0).count();
        if lead > 0 {
            self.digits.drain(..lead);
            self.weight -= lead as i32;
        }
        while self.digits.last() == Some(&0) {
            self.digits.pop();
        }
        if self.digits.is_empty() {
            self.weight = 0;
            self.negative = false;
        }
    }

    /// Render to Postgres text, padded to `dscale` fractional digits.
    fn to_display_string(&self) -> String {
        let mut out = String::new();
        if self.negative {
            out.push('-');
        }
        // Integer part: positions from max(weight, 0) down to 0 (at least "0").
        let hi = self.weight.max(0);
        for pos in (0..=hi).rev() {
            out.push((b'0' + self.digit_at(pos)) as char);
        }
        if self.dscale > 0 {
            out.push('.');
            for pos in 1..=i32::from(self.dscale) {
                out.push((b'0' + self.digit_at(-pos)) as char);
            }
        }
        out
    }

    /// Compare by signed magnitude (zero is neither positive nor negative).
    fn cmp(&self, other: &Self) -> Ordering {
        let sign = |v: &Self| -> i8 {
            if v.digits.is_empty() {
                0
            } else if v.negative {
                -1
            } else {
                1
            }
        };
        match sign(self).cmp(&sign(other)) {
            Ordering::Equal => {}
            ord => return ord,
        }
        // Same sign (or both zero → Equal). Compare magnitudes; flip if negative.
        let mag = cmp_abs(self, other);
        if self.negative { mag.reverse() } else { mag }
    }

    /// Round to `scale` fractional digits (half-away-from-zero) and set `dscale`.
    fn round_to_scale(&mut self, scale: i32) {
        // Clamp `scale` to Postgres `numeric_round`'s bounds before touching the
        // digits: a huge positive scale (e.g. `i32::MAX`) would otherwise overflow
        // `weight + scale + 1` and wrap negative, silently zeroing the value. The
        // low bound allows one extra digit past the smallest weight for a round-up
        // carry; the high bound is the max display scale.
        let scale = scale.clamp(
            -(NUMERIC_WEIGHT_MAX + 1) * DEC_DIGITS - 1,
            NUMERIC_DSCALE_MAX,
        );
        // Number of leading digits to keep: those at positions ≥ -scale. Saturating
        // arithmetic is belt-and-suspenders — the clamp already bounds the sum.
        let keep = self.weight.saturating_add(scale).saturating_add(1);
        let n = self.digits.len() as i32;
        if keep < n {
            let round_up = if keep < 0 {
                false
            } else {
                self.digits[keep as usize] >= 5
            };
            let kc = keep.max(0) as usize;
            self.digits.truncate(kc);
            if round_up {
                if self.digits.is_empty() {
                    // Everything dropped but the value rounds up to 1·10^-scale.
                    self.digits.push(1);
                    self.weight = -scale;
                } else {
                    increment_digits(&mut self.digits, &mut self.weight);
                }
            }
            self.normalize();
        }
        self.dscale = scale.max(0).min(i32::from(u16::MAX)) as u16;
    }

    /// Truncate toward zero to `scale` fractional digits (drop lower digits, no
    /// rounding) and set `dscale`.
    fn trunc_to_scale(&mut self, scale: i32) {
        // Clamp `scale` to Postgres `numeric_trunc`'s bounds (one less negative
        // headroom than `round` — truncation never carries a digit up) so
        // `weight + scale + 1` can't overflow i32 and wrap to a silent zero.
        let scale = scale.clamp(-(NUMERIC_WEIGHT_MAX + 1) * DEC_DIGITS, NUMERIC_DSCALE_MAX);
        let keep = self.weight.saturating_add(scale).saturating_add(1);
        if keep < self.digits.len() as i32 {
            self.digits.truncate(keep.max(0) as usize);
            self.normalize();
        }
        self.dscale = scale.max(0).min(i32::from(u16::MAX)) as u16;
    }

    /// Signed add (subtraction routes here via `PgNumeric::negate`). Result
    /// `dscale` is the max of the operands', which always covers the exact sum.
    fn add(&self, other: &Self) -> Self {
        let dscale = self.dscale.max(other.dscale);
        let (digits, weight, negative) = if self.negative == other.negative {
            let (d, w) = add_abs(self, other);
            (d, w, self.negative)
        } else {
            match cmp_abs(self, other) {
                Ordering::Equal => (Vec::new(), 0, false),
                Ordering::Greater => {
                    let (d, w) = sub_abs(self, other);
                    (d, w, self.negative)
                }
                Ordering::Less => {
                    let (d, w) = sub_abs(other, self);
                    (d, w, other.negative)
                }
            }
        };
        let mut v = Self {
            negative,
            digits,
            weight,
            dscale,
        };
        v.normalize();
        v
    }

    /// Exact multiply. Result `dscale` is the sum of the operands'.
    fn mul(&self, other: &Self) -> Self {
        let dscale = self.dscale.saturating_add(other.dscale);
        if self.digits.is_empty() || other.digits.is_empty() {
            let mut z = Self {
                negative: false,
                digits: Vec::new(),
                weight: 0,
                dscale,
            };
            z.normalize();
            return z;
        }
        // Multiply the integer coefficients (least-significant-first), then place
        // the point: the product's least-significant digit sits at the sum of the
        // operands' least-significant positions.
        let a: Vec<u8> = self.digits.iter().rev().copied().collect();
        let b: Vec<u8> = other.digits.iter().rev().copied().collect();
        let mut prod = vec![0u16; a.len() + b.len()];
        for (i, &da) in a.iter().enumerate() {
            let mut carry = 0u16;
            for (j, &db) in b.iter().enumerate() {
                let cur = prod[i + j] + u16::from(da) * u16::from(db) + carry;
                prod[i + j] = cur % 10;
                carry = cur / 10;
            }
            prod[i + b.len()] += carry;
        }
        // `prod` is least-significant-first; convert to MSB-first digits.
        let low_pos = self.low_pos() + other.low_pos();
        let mut digits: Vec<u8> = prod.iter().rev().map(|&d| d as u8).collect();
        let weight = low_pos + (digits.len() as i32 - 1);
        // trim internal representation via normalize (handles leading/trailing).
        let mut v = Self {
            negative: self.negative != other.negative,
            digits: std::mem::take(&mut digits),
            weight,
            dscale,
        };
        v.normalize();
        v
    }

    /// Divide, targeting `select_div_scale`'s result scale, rounded half-away.
    fn div(&self, other: &Self) -> Self {
        // Zero dividend → zero at the chosen scale.
        let rscale = select_div_scale(self, other);
        if self.digits.is_empty() {
            let mut z = Self {
                negative: false,
                digits: Vec::new(),
                weight: 0,
                dscale: 0,
            };
            z.round_to_scale(rscale);
            return z;
        }
        // Compute one guard digit past the target scale, then round.
        let guard = rscale + 1;
        // Align so the integer quotient's least-significant digit lands at
        // position -guard: numerator = |self| · 10^p, denominator = |other| · 10^q
        // where p - q shifts the quotient's LSB to -guard.
        let a_low = self.low_pos();
        let b_low = other.low_pos();
        // M = floor(|self|/|other| · 10^guard) = floor((A·10^p) / B), where A/B are
        // the operands' integer coefficients and p = (a_low - b_low) + guard places
        // the quotient's least-significant digit at position -guard. Scale whichever
        // side p calls for (numerator up when p ≥ 0, else denominator).
        let p = (a_low - b_low) + guard;
        let mut num: Vec<u8> = self.digits.clone();
        let mut den: Vec<u8> = other.digits.clone();
        if p >= 0 {
            num.extend(std::iter::repeat_n(0u8, p as usize));
        } else {
            den.extend(std::iter::repeat_n(0u8, (-p) as usize));
        }
        let quotient = bigint_div(&num, &den);
        // `quotient` is the integer floor(num/den); its LSB is at position -guard.
        let mut v = if quotient.is_empty() {
            Self {
                negative: false,
                digits: Vec::new(),
                weight: 0,
                dscale: 0,
            }
        } else {
            Self {
                negative: self.negative != other.negative,
                weight: (quotient.len() as i32 - 1) - guard,
                digits: quotient,
                dscale: (guard.max(0)) as u16,
            }
        };
        v.normalize();
        v.round_to_scale(rscale);
        v
    }
}

/// Compare two `NumericVar`s by absolute value.
fn cmp_abs(a: &NumericVar, b: &NumericVar) -> Ordering {
    match (a.digits.is_empty(), b.digits.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (false, false) => {}
    }
    match a.weight.cmp(&b.weight) {
        Ordering::Equal => {}
        ord => return ord,
    }
    let lo = a.low_pos().min(b.low_pos());
    for pos in (lo..=a.weight).rev() {
        match a.digit_at(pos).cmp(&b.digit_at(pos)) {
            Ordering::Equal => {}
            ord => return ord,
        }
    }
    Ordering::Equal
}

/// Add two magnitudes; returns unnormalized MSB-first digits and the weight of
/// the first digit.
fn add_abs(a: &NumericVar, b: &NumericVar) -> (Vec<u8>, i32) {
    let hi = a.weight.max(b.weight);
    let lo = a.low_pos().min(b.low_pos());
    let mut res = Vec::with_capacity((hi - lo + 2) as usize);
    let mut carry = 0u8;
    for pos in lo..=hi {
        let s = a.digit_at(pos) + b.digit_at(pos) + carry;
        res.push(s % 10);
        carry = s / 10;
    }
    let weight = if carry > 0 {
        res.push(carry);
        hi + 1
    } else {
        hi
    };
    res.reverse();
    (res, weight)
}

/// Subtract `b` from `a` in magnitude; requires `|a| >= |b|`. Returns
/// unnormalized MSB-first digits and the weight of the first digit.
fn sub_abs(a: &NumericVar, b: &NumericVar) -> (Vec<u8>, i32) {
    let hi = a.weight;
    let lo = a.low_pos().min(b.low_pos());
    let mut res = Vec::with_capacity((hi - lo + 1) as usize);
    let mut borrow = 0i8;
    for pos in lo..=hi {
        let d = i8::try_from(a.digit_at(pos)).unwrap_or(0)
            - i8::try_from(b.digit_at(pos)).unwrap_or(0)
            - borrow;
        if d < 0 {
            res.push((d + 10) as u8);
            borrow = 1;
        } else {
            res.push(d as u8);
            borrow = 0;
        }
    }
    res.reverse();
    (res, hi)
}

/// Add 1 to the least-significant digit of an MSB-first digit vector, propagating
/// the carry; a carry out of the front prepends a digit and bumps `weight`.
fn increment_digits(digits: &mut Vec<u8>, weight: &mut i32) {
    let mut i = digits.len();
    loop {
        if i == 0 {
            digits.insert(0, 1);
            *weight += 1;
            return;
        }
        i -= 1;
        if digits[i] == 9 {
            digits[i] = 0;
        } else {
            digits[i] += 1;
            return;
        }
    }
}

/// Postgres `DEC_DIGITS` (decimal digits per `NBASE` digit) — division's result
/// scale is chosen in `NBASE` units, so the significant-digit estimate steps by 4.
const DEC_DIGITS: i32 = 4;

/// Postgres `select_div_scale`: pick a division result scale giving at least
/// `MIN_SIG_DIGITS` significant digits, but no less than either input's display
/// scale (clamped to `[0, MAX_DISPLAY_SCALE]`).
///
/// The weight estimate must be in `NBASE` (base-10000) units, exactly as
/// `numeric.c` computes it — a base-10 estimate rounds the result scale
/// differently (`7/10` is scale 20 in Postgres, not 17). So each operand's first
/// digit and weight come from its `NBASE` grouping.
fn select_div_scale(a: &NumericVar, b: &NumericVar) -> i32 {
    let (w1, f1) = nbase_first_digit(a);
    let (w2, f2) = nbase_first_digit(b);
    let mut qweight = w1 - w2;
    if f1 <= f2 {
        qweight -= 1;
    }
    let mut rscale = MIN_SIG_DIGITS - qweight * DEC_DIGITS;
    rscale = rscale.max(i32::from(a.dscale));
    rscale = rscale.max(i32::from(b.dscale));
    rscale = rscale.max(0);
    rscale.min(MAX_DISPLAY_SCALE)
}

/// The `NBASE` weight and value of a `NumericVar`'s most significant `NBASE`
/// digit (`(0, 0)` for zero) — the inputs `select_div_scale` needs.
fn nbase_first_digit(v: &NumericVar) -> (i32, i16) {
    let (weight, groups) = v.to_nbase_groups();
    match groups.first() {
        Some(&g) => (i32::from(weight), g),
        None => (0, 0),
    }
}

/// Integer long division of two nonnegative base-10 big integers (MSB-first, no
/// decimal point). Returns the floor quotient MSB-first (empty for zero). `den`
/// must be nonzero.
fn bigint_div(num: &[u8], den: &[u8]) -> Vec<u8> {
    let den = strip_leading_zeros(den);
    let mut quotient = Vec::with_capacity(num.len());
    let mut rem: Vec<u8> = Vec::new(); // running remainder, MSB-first big int
    for &d in num {
        // Bring down the next digit: rem = rem*10 + d.
        rem.push(d);
        strip_leading_zeros_in_place(&mut rem);
        // Largest q in 0..=9 with den*q <= rem.
        let mut q = 0u8;
        while q < 9 && bigint_cmp(&bigint_mul_small(den, q + 1), &rem) != Ordering::Greater {
            q += 1;
        }
        quotient.push(q);
        if q > 0 {
            rem = bigint_sub(&rem, &bigint_mul_small(den, q));
        }
    }
    strip_leading_zeros(&quotient).to_vec()
}

/// Compare two nonnegative big integers (MSB-first, possibly with leading zeros).
fn bigint_cmp(a: &[u8], b: &[u8]) -> Ordering {
    let a = strip_leading_zeros(a);
    let b = strip_leading_zeros(b);
    match a.len().cmp(&b.len()) {
        Ordering::Equal => a.cmp(b),
        ord => ord,
    }
}

/// Multiply a big integer (MSB-first) by a single digit `0..=9`.
fn bigint_mul_small(a: &[u8], m: u8) -> Vec<u8> {
    if m == 0 {
        return Vec::new();
    }
    let mut res_rev = Vec::with_capacity(a.len() + 1);
    let mut carry = 0u16;
    for &d in a.iter().rev() {
        let cur = u16::from(d) * u16::from(m) + carry;
        res_rev.push((cur % 10) as u8);
        carry = cur / 10;
    }
    while carry > 0 {
        res_rev.push((carry % 10) as u8);
        carry /= 10;
    }
    res_rev.reverse();
    res_rev
}

/// Subtract `b` from `a` (both MSB-first, nonnegative); requires `a >= b`.
fn bigint_sub(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut res_rev = Vec::with_capacity(a.len());
    let mut borrow = 0i16;
    let bl = b.len();
    let al = a.len();
    for i in 0..al {
        let da = i16::from(a[al - 1 - i]);
        let db = if i < bl { i16::from(b[bl - 1 - i]) } else { 0 };
        let mut d = da - db - borrow;
        if d < 0 {
            d += 10;
            borrow = 1;
        } else {
            borrow = 0;
        }
        res_rev.push(d as u8);
    }
    res_rev.reverse();
    strip_leading_zeros(&res_rev).to_vec()
}

fn strip_leading_zeros(d: &[u8]) -> &[u8] {
    let start = d.iter().position(|&x| x != 0).unwrap_or(d.len());
    &d[start..]
}

fn strip_leading_zeros_in_place(d: &mut Vec<u8>) {
    let start = d.iter().position(|&x| x != 0).unwrap_or(d.len());
    d.drain(0..start);
}

/// Rust's shortest round-trip float spelling, but normalized so it always parses
/// as decimal (`inf`/`NaN` are handled by the caller).
fn format_shortest_f64(f: f64) -> String {
    // `{}` already yields the shortest decimal that round-trips (e.g. 0.1).
    format!("{f}")
}

/// Why a decimal literal failed to parse: a syntactically-malformed spelling
/// (Postgres `22P02`) versus a spelling the numeric format overflows on — its display
/// scale or its exponent (`22003`). The caller ([`PgNumeric::parse`]) maps each to its
/// SQLSTATE, so a valid-but-out-of-range literal isn't collapsed into a generic
/// syntax error. Which of the two a spelling gets is what decides whether the message
/// carries it: `22P02`'s names the whole literal, `22003`'s names none of it.
enum DecimalError {
    Syntax,
    Overflow,
}

/// Parse a finite decimal string into a `PgNumeric::Num`.
///
/// # Errors
///
/// [`DecimalError::Syntax`] for a malformed spelling; [`DecimalError::Overflow`] when
/// the value's display scale exceeds [`NUMERIC_DSCALE_MAX`], or its exponent
/// [`NUMERIC_EXPONENT_MAX`] (Postgres "value overflows numeric format" in both cases).
/// A weight past the format's is the caller's `check_weight`, not this function's.
fn parse_decimal(s: &str) -> std::result::Result<PgNumeric, DecimalError> {
    if s.is_empty() {
        return Err(DecimalError::Syntax);
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    let negative = match bytes[0] {
        b'+' => {
            i += 1;
            false
        }
        b'-' => {
            i += 1;
            true
        }
        _ => false,
    };
    // Mantissa digits, tracking the fractional count.
    let mut digits: Vec<u8> = Vec::new();
    let mut int_len: i32 = 0;
    let mut frac_len: i32 = 0;
    let mut seen_dot = false;
    let mut seen_digit = false;
    while i < bytes.len() {
        match bytes[i] {
            b'0'..=b'9' => {
                digits.push(bytes[i] - b'0');
                if seen_dot {
                    frac_len += 1;
                } else {
                    int_len += 1;
                }
                seen_digit = true;
                i += 1;
            }
            b'.' if !seen_dot => {
                seen_dot = true;
                i += 1;
            }
            b'e' | b'E' => break,
            _ => return Err(DecimalError::Syntax),
        }
    }
    if !seen_digit {
        return Err(DecimalError::Syntax);
    }
    // Optional exponent.
    let exp: i64 = if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        let esign = match bytes.get(i) {
            Some(b'+') => {
                i += 1;
                1
            }
            Some(b'-') => {
                i += 1;
                -1
            }
            _ => 1,
        };
        let start = i;
        let mut e: i64 = 0;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            e = e * 10 + i64::from(bytes[i] - b'0');
            if e > NUMERIC_EXPONENT_MAX {
                // An overflow, not a syntax error, and raised **here** — inside the
                // digit run, before the trailing bytes are read — which is where
                // `set_var_from_str` raises it, so a malformed spelling behind a
                // too-large exponent is the overflow too.
                return Err(DecimalError::Overflow);
            }
            i += 1;
        }
        if i == start {
            return Err(DecimalError::Syntax); // 'e' with no digits
        }
        esign * e
    } else {
        0
    };
    if i != bytes.len() {
        return Err(DecimalError::Syntax); // trailing garbage
    }
    // weight of the first mantissa digit, before applying the exponent. Both sums are
    // taken in `i64`: the exponent alone reaches `PG_INT32_MAX / 2`, so an `i32` would
    // have the mantissa's own length to wrap it.
    let weight = i64::from(int_len) - 1 + exp;
    let dscale = (i64::from(frac_len) - exp).max(0);
    // A display scale past `NUMERIC_DSCALE_MAX` (the width of Postgres's 14-bit
    // dscale field) can't be stored, so `numeric_in` raises "value overflows numeric
    // format" (`22003`). This replaces a silent clamp to `MAX_DISPLAY_SCALE` (1000,
    // the division-result cap — the wrong limit here) that truncated the significant
    // digit out of a tiny value, leaving an internally-inconsistent phantom that
    // rendered `0` yet compared non-zero (`1e-200000`); scales in `1001..=16383` now
    // round-trip exactly, matching Postgres.
    if dscale > i64::from(NUMERIC_DSCALE_MAX) {
        return Err(DecimalError::Overflow);
    }
    // The weight's own cap is `PgNumeric::check_weight`'s, which the caller runs and
    // which exempts a zero (`0e1073741823` is `0` on 18.6, weight and all). Saturating
    // rather than wrapping keeps that verdict the cap's: every value the narrowing
    // would have wrapped is one `check_weight` refuses.
    let weight = i32::try_from(weight).unwrap_or(if weight > 0 { i32::MAX } else { i32::MIN });
    let mut v = NumericVar {
        negative,
        digits,
        weight,
        dscale: dscale as u16,
    };
    v.normalize();
    Ok(PgNumeric::Num(v))
}

// --- Postgres binary wire format (numeric_send / numeric_recv) ---------------

/// Postgres numeric sign codes.
const PG_NUMERIC_POS: u16 = 0x0000;
const PG_NUMERIC_NEG: u16 = 0x4000;
const PG_NUMERIC_NAN: u16 = 0xC000;
/// `NUMERIC_PINF`: accepted by `numeric_recv`'s sign check, not representable here.
const PG_NUMERIC_PINF: u16 = 0xD000;
/// `NUMERIC_NINF`: accepted by `numeric_recv`'s sign check, not representable here.
const PG_NUMERIC_NINF: u16 = 0xF000;
/// `NUMERIC_DSCALE_MASK`: the bits a binary display scale may set.
const PG_NUMERIC_DSCALE_MASK: u16 = 0x3FFF;

impl PgNumeric {
    /// Encode to Postgres binary format (`numeric_send`): `int16` ndigits, weight,
    /// sign, dscale, then the base-10000 digits — regrouping the internal base-10
    /// digits into `NBASE` groups aligned on the decimal point.
    #[must_use]
    pub fn to_wire_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        let push_i16 = |buf: &mut Vec<u8>, n: i16| buf.extend_from_slice(&n.to_be_bytes());
        match self {
            Self::NaN => {
                push_i16(&mut buf, 0); // ndigits
                push_i16(&mut buf, 0); // weight
                buf.extend_from_slice(&PG_NUMERIC_NAN.to_be_bytes());
                push_i16(&mut buf, 0); // dscale
            }
            Self::Num(v) => {
                let (nbase_weight, groups) = v.to_nbase_groups();
                push_i16(&mut buf, groups.len() as i16);
                push_i16(&mut buf, nbase_weight);
                let sign = if v.negative {
                    PG_NUMERIC_NEG
                } else {
                    PG_NUMERIC_POS
                };
                buf.extend_from_slice(&sign.to_be_bytes());
                push_i16(&mut buf, i16::try_from(v.dscale).unwrap_or(i16::MAX));
                for g in groups {
                    push_i16(&mut buf, g);
                }
            }
        }
        buf
    }

    /// Read a binary `numeric` from `buf` the way 18.6's `numeric_recv` reads it,
    /// leaving the cursor after the last digit.
    ///
    /// The order is `numeric_recv`'s, and it decides which refusal a malformed body
    /// earns: the digit count as an **unsigned** 16-bit value (so `0xFFFF` asks for
    /// 65535 digits), the weight, the sign — refused before any digit is read — the
    /// display scale, then each digit. Every field read goes through the multi-byte
    /// message reader, so a body that runs out is `08P01 insufficient data left in
    /// message`. A `NaN` still reads, and validates, the digits its header declares. A
    /// number is then truncated to its display scale, as `numeric_recv`'s `trunc_var`
    /// truncates it.
    /// Bytes after the last digit are not this reader's to refuse.
    ///
    /// # Errors
    ///
    /// [`Error::InsufficientDataLeft`] when a field runs past the buffer;
    /// [`Error::InvalidNumericSign`], [`Error::InvalidNumericScale`] and
    /// [`Error::InvalidNumericDigit`] (`22P03`) for the three fields `numeric_recv`
    /// validates; and [`Error::InvalidInputSyntax`] (`22P02`) for a well-formed
    /// `Infinity` / `-Infinity` body, which `PgNumeric` cannot represent.
    pub fn recv(buf: &mut BinaryReader<'_>) -> Result<Self> {
        let ndigits = buf.u16()?;
        let weight = buf.i16()?;
        let sign = buf.u16()?;
        if !matches!(
            sign,
            PG_NUMERIC_POS | PG_NUMERIC_NEG | PG_NUMERIC_NAN | PG_NUMERIC_PINF | PG_NUMERIC_NINF
        ) {
            return Err(Error::InvalidNumericSign);
        }
        let dscale = buf.u16()?;
        if dscale & PG_NUMERIC_DSCALE_MASK != dscale {
            return Err(Error::InvalidNumericScale);
        }
        let mut groups = Vec::with_capacity(usize::from(ndigits).min(buf.remaining() / 2));
        for _ in 0..ndigits {
            let digit = buf.i16()?;
            let group = u16::try_from(digit)
                .ok()
                .filter(|&g| g < 10000)
                .ok_or(Error::InvalidNumericDigit)?;
            groups.push(group);
        }
        match sign {
            PG_NUMERIC_NAN => Ok(Self::NaN),
            PG_NUMERIC_POS | PG_NUMERIC_NEG => {
                let mut value = NumericVar::from_nbase_groups(
                    sign == PG_NUMERIC_NEG,
                    i32::from(weight),
                    dscale,
                    &groups,
                );
                // `numeric_recv`'s `trunc_var(&value, value.dscale)`: digits the display
                // scale hides are truncated toward zero rather than kept, and a value
                // that truncates to zero loses its sign (`make_result`'s zero rule,
                // which `normalize` applies).
                value.trunc_to_scale(i32::from(dscale));
                Ok(Self::Num(value))
            }
            // `PgNumeric` has no infinity: refused here, after every field 18.6 reads
            // has been read, so each refusal above still answers as 18.6's does.
            _ => Err(Error::InvalidInputSyntax {
                type_name: "numeric",
                value: "<binary>".to_string(),
            }),
        }
    }
}

impl NumericVar {
    /// Regroup the base-10 digits into Postgres `NBASE` (10000) groups, returning
    /// the `NBASE` weight of the first group and the group values (leading/trailing
    /// zero groups stripped, matching `numeric_send`).
    fn to_nbase_groups(&self) -> (i16, Vec<i16>) {
        if self.digits.is_empty() {
            return (0, Vec::new());
        }
        // The most significant decimal position, and the least we must cover
        // (include the display scale so a value like 1.50 keeps its group).
        let hi_pos = self.weight;
        let lo_pos = self.low_pos().min(-i32::from(self.dscale));
        // NBASE group index g covers decimal positions [4g, 4g+3]; group value =
        // Σ digit_at(4g+j)·10^j.
        let hi_g = div_floor(hi_pos, 4);
        let lo_g = div_floor(lo_pos, 4);
        let mut groups: Vec<i16> = Vec::with_capacity((hi_g - lo_g + 1) as usize);
        for g in (lo_g..=hi_g).rev() {
            let mut val = 0i16;
            for j in 0..4 {
                val += i16::from(self.digit_at(4 * g + j)) * 10i16.pow(j as u32);
            }
            groups.push(val);
        }
        // Strip leading/trailing zero groups; track how many leading drops lower
        // the reported weight.
        let mut weight = hi_g;
        while groups.first() == Some(&0) {
            groups.remove(0);
            weight -= 1;
        }
        while groups.last() == Some(&0) {
            groups.pop();
        }
        (i16::try_from(weight).unwrap_or(0), groups)
    }

    /// Rebuild from Postgres `NBASE` groups (the inverse of `to_nbase_groups`).
    fn from_nbase_groups(negative: bool, nbase_weight: i32, dscale: u16, groups: &[u16]) -> Self {
        // Expand each group into 4 base-10 digits at positions [4g, 4g+3].
        let mut digits: Vec<u8> = Vec::with_capacity(groups.len() * 4);
        for &g in groups {
            // Split the group into 4 base-10 digits: tmp[0] is the least
            // significant (position 4·group_weight), tmp[3] the most.
            let mut val = g;
            let mut tmp = [0u8; 4];
            for slot in &mut tmp {
                *slot = (val % 10) as u8;
                val /= 10;
            }
            for j in (0..4).rev() {
                digits.push(tmp[j]);
            }
        }
        // Weight of the first pushed digit = 4*nbase_weight + 3.
        let mut v = Self {
            negative,
            weight: 4 * nbase_weight + 3,
            digits,
            dscale,
        };
        v.normalize();
        if v.digits.is_empty() {
            v.negative = false;
        }
        v
    }
}

/// Floor division for `i32` (Rust `/` truncates toward zero).
const fn div_floor(a: i32, b: i32) -> i32 {
    let q = a / b;
    let r = a % b;
    if (r != 0) && ((r < 0) != (b < 0)) {
        q - 1
    } else {
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> PgNumeric {
        PgNumeric::parse(s).unwrap()
    }

    #[test]
    fn parse_and_display_round_trip() {
        for s in [
            "0",
            "0.00",
            "1",
            "-1",
            "1.50",
            "123.45",
            "0.0045",
            "-0.5",
            "1000",
            "0.10",
            "100000000000000000000000000000",
            "999.999",
        ] {
            assert_eq!(n(s).to_display_string(), s, "round-trip {s}");
        }
    }

    #[test]
    fn parse_exponent_and_signs() {
        assert_eq!(n("1.5e2").to_display_string(), "150");
        assert_eq!(n("1.5e-2").to_display_string(), "0.015");
        assert_eq!(n("+42").to_display_string(), "42");
        assert_eq!(n("007").to_display_string(), "7");
        assert_eq!(n("1E3").to_display_string(), "1000");
        assert_eq!(PgNumeric::parse("nan").unwrap().to_display_string(), "NaN");
        assert_eq!(PgNumeric::parse("NaN").unwrap().to_display_string(), "NaN");
    }

    #[test]
    fn parse_rejects_malformed() {
        for bad in ["", "abc", "1.2.3", "1e", "--1", "1e2e3", "0x10", "1 2", " "] {
            assert!(PgNumeric::parse(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    /// The **cost** bound on a spelling's leading zeros, which carry no value and
    /// which nothing caps the count of. The two caps that do bind sit elsewhere,
    /// each measured against 18.6 at its own edge: `NUMERIC_DSCALE_MAX` bounds a
    /// *fractional* run (`0.<16 382 zeros>1`, display scale 16 383, is a value on
    /// both engines and one zero more is `22003` on both) and `check_weight`
    /// bounds the significant part (131 072 nines is a value on both, 131 073 is
    /// `22003` on both) — while an integer-position leading run was accepted at
    /// every width measured, to 3 200 000, both engines reading `<zeros>1` as `1`.
    ///
    /// `normalize` drops that run in one `drain`; the `remove(0)`-a-digit loop it
    /// replaced shifted the whole tail down a place per zero, so the run cost time
    /// quadratic in its own count while every value it produced stayed **exact** —
    /// so nothing comparing values can see it, only the clock. Measured over the
    /// release server, `SELECT <n zeros>999999999999999999999999999999` against
    /// 18.4: 0.93 s / 0.03 at 400 000, 3.84 / 0.04 at 800 000, 16.46 / 0.05 at
    /// 1 600 000, 67.89 / 0.06 at 3 200 000 — and 0.03 / 0.03 / 0.04 / 0.06 at the
    /// same four after the drain. The doors measured to pay it, each closing with
    /// it: the literal, a `::numeric` cast, a text-format `$N`, `COPY FROM`, a
    /// column `DEFAULT` and a `CHECK` cook — and an `INSERT` into an indexed
    /// `numeric` column, where the cost is that `INSERT`'s own coercion and index
    /// maintenance re-reads no spelling. The one door reached that did not pay is
    /// a `jsonb` number, whose leading run can only be fractional (`007` is not
    /// JSON) and so is capped at the display scale.
    ///
    /// The bound is absolute rather than a ratio against a zero-free control of the
    /// same width: the fixed cost of both is milliseconds, so a ratio between them
    /// is a ratio of noise. This shape is
    /// `the_widest_decode_stays_within_a_wall_clock_bound`'s: here a debug build
    /// takes 0.06 s, and 64.63 s with the `remove(0)` loop restored.
    #[test]
    fn the_leading_zero_strip_stays_within_a_wall_clock_bound() {
        const BOUND: std::time::Duration = std::time::Duration::from_secs(20);
        const ZEROS: usize = 3_200_000;
        let mut spelling = "0".repeat(ZEROS);
        spelling.push('1');
        let start = std::time::Instant::now();
        let parsed = PgNumeric::parse(&spelling).expect("a leading-zero run is accepted");
        let elapsed = start.elapsed();
        assert_eq!(parsed.to_display_string(), "1", "the zeros carry no value");
        assert!(elapsed < BOUND, "{ZEROS} leading zeros took {elapsed:?}");
    }

    #[test]
    fn comparison_orders_by_value_with_nan_high() {
        assert_eq!(n("1.5").cmp_value(&n("1.50")), Ordering::Equal);
        assert_eq!(n("-1").cmp_value(&n("1")), Ordering::Less);
        assert_eq!(n("2").cmp_value(&n("10")), Ordering::Less);
        assert_eq!(n("0.1").cmp_value(&n("0.09")), Ordering::Greater);
        assert_eq!(n("-5").cmp_value(&n("-10")), Ordering::Greater);
        // NaN sorts above every finite value and equals itself.
        assert_eq!(PgNumeric::NaN.cmp_value(&n("1e30")), Ordering::Greater);
        assert_eq!(PgNumeric::NaN.cmp_value(&PgNumeric::NaN), Ordering::Equal);
        // Value equality (PartialEq) matches SQL `=`.
        assert_eq!(n("1.5"), n("1.50"));
        assert_eq!(PgNumeric::NaN, PgNumeric::NaN);
    }

    #[test]
    fn add_sub_scale_is_max() {
        assert_eq!(
            n("1.5").add(&n("2.25")).unwrap().to_display_string(),
            "3.75"
        );
        assert_eq!(
            n("1.50").add(&n("2.2")).unwrap().to_display_string(),
            "3.70"
        );
        assert_eq!(n("0.1").add(&n("0.2")).unwrap().to_display_string(), "0.3");
        assert_eq!(n("10").sub(&n("0.01")).unwrap().to_display_string(), "9.99");
        assert_eq!(n("5").sub(&n("5")).unwrap().to_display_string(), "0");
        assert_eq!(n("1").sub(&n("3")).unwrap().to_display_string(), "-2");
        // carries across many digits
        assert_eq!(
            n("999.99").add(&n("0.01")).unwrap().to_display_string(),
            "1000.00"
        );
    }

    #[test]
    fn mul_scale_is_sum() {
        assert_eq!(
            n("1.10").mul(&n("1.10")).unwrap().to_display_string(),
            "1.2100"
        );
        assert_eq!(n("2").mul(&n("3")).unwrap().to_display_string(), "6");
        assert_eq!(n("-1.5").mul(&n("2")).unwrap().to_display_string(), "-3.0");
        // 0 (dscale 0) · 123.45 (dscale 2) → dscale 0+2 = 2 (verified vs Postgres).
        assert_eq!(
            n("0").mul(&n("123.45")).unwrap().to_display_string(),
            "0.00"
        );
        assert_eq!(n("1.5").mul(&n("0.2")).unwrap().to_display_string(), "0.30");
        assert_eq!(
            n("12345678901234567890")
                .mul(&n("2"))
                .unwrap()
                .to_display_string(),
            "24691357802469135780"
        );
    }

    #[test]
    fn arithmetic_weight_cap_is_loud() {
        // `10^131071` is the largest representable numeric (base-10000 weight =
        // NUMERIC_WEIGHT_MAX); powers of ten stay single-digit so this is cheap.
        let big = n("10").pow(&n("131071")).unwrap();
        assert_eq!(big.display_scale(), 0);
        // One more decimal place past the cap → 22003 (make_result overflow).
        assert_eq!(big.mul(&n("10")).unwrap_err().sqlstate(), "22003");
        // A carry past the cap is loud too: `9·10^131071 + 9·10^131071` carries a
        // new most-significant digit at position 131072.
        let nine = big.mul(&n("9")).unwrap(); // 9·10^131071, weight 131071 (in range)
        assert_eq!(nine.add(&nine).unwrap_err().sqlstate(), "22003");
        // `10^100000` squared (weight 200000) is loud on the multiply.
        let e5 = n("10").pow(&n("100000")).unwrap();
        assert_eq!(e5.mul(&e5).unwrap_err().sqlstate(), "22003");
        // A large but in-range product still succeeds and is exact.
        let a = n(&format!("1{}", "0".repeat(999))); // 10^999
        let prod = a.mul(&a).unwrap(); // 10^1998, weight 1998 < cap
        assert_eq!(prod, n(&format!("1{}", "0".repeat(1998))));
        // Ordinary arithmetic is unchanged.
        assert_eq!(n("2").mul(&n("3")).unwrap().to_display_string(), "6");
    }

    #[test]
    fn round_trunc_clamp_huge_scale_matches_pg() {
        // A pathological scale must clamp to Postgres's numeric_round/trunc bounds,
        // not wrap `weight + scale + 1` and silently zero the value.
        let r = n("1.23").round(i32::MAX);
        assert_eq!(r.display_scale(), 16383); // NUMERIC_DSCALE_MAX
        assert_eq!(r, n("1.23")); // value preserved (equality ignores dscale)
        let t = n("1.23").trunc(i32::MAX);
        assert_eq!(t.display_scale(), 16383);
        assert_eq!(t, n("1.23"));
        // A very negative scale rounds/truncates to zero (correctly, not via wrap).
        assert!(n("1.23").round(i32::MIN).is_zero());
        assert!(n("1.23").trunc(i32::MIN).is_zero());
        // Ordinary scales are unchanged.
        assert_eq!(n("2.567").round(2).to_display_string(), "2.57");
        assert_eq!(n("2.567").trunc(2).to_display_string(), "2.56");
    }

    #[test]
    fn mul_caps_display_scale_like_numeric_mul() {
        // Ordinary products keep the exact `dscale1 + dscale2` scale.
        assert_eq!(n("1.5").mul(&n("2.5")).unwrap().to_display_string(), "3.75");
        assert_eq!(
            n("1.10").mul(&n("1.10")).unwrap().to_display_string(),
            "1.2100"
        );
        // But `numeric_mul` caps the product's display scale at NUMERIC_DSCALE_MAX
        // (16383): squaring `0.1` doubles the scale each step, so once the exact
        // product would need > 16383 fractional digits it rounds — and a value below
        // that precision folds to exactly 0, matching Postgres.
        let mut x = n("0.1");
        for _ in 0..13 {
            x = x.mul(&x).unwrap(); // 0.1^(2^k); at k=13 the value is 10^-8192
        }
        assert!(!x.is_zero(), "level 13 (dscale 8192) is still nonzero");
        x = x.mul(&x).unwrap(); // level 14: exact dscale 16384 > cap → rounds to 0
        assert!(
            x.is_zero(),
            "level 14 (dscale would be 16384) folds to 0 like PG"
        );
    }

    #[test]
    fn parse_rejects_over_cap_literal() {
        // A well-formed literal past the weight cap fails loud (Postgres `numeric_in`
        // also runs make_result). `1e131071` is the largest representable.
        assert!(PgNumeric::parse("1e131071").is_ok());
        assert_eq!(
            PgNumeric::parse("1e131072").unwrap_err().sqlstate(),
            "22003"
        );
    }

    #[test]
    fn parse_rejects_over_scale_literal_symmetric() {
        // The lower end is bounded by the 14-bit display-scale field
        // (`NUMERIC_DSCALE_MAX` = 16383), not by weight — matching Postgres, which
        // rejects `1e-16384` (display scale 16384) but renders `1e-16383` exactly.
        // Before the fix `parse_decimal` silently clamped the scale to 1000, so a
        // tiny value dropped its only significant digit and became a phantom that
        // rendered `0` yet compared non-zero.
        assert_eq!(
            PgNumeric::parse("1e-16384").unwrap_err().sqlstate(),
            "22003",
            "display scale 16384 must overflow like Postgres"
        );
        assert_eq!(
            PgNumeric::parse("1e-200000").unwrap_err().sqlstate(),
            "22003",
            "the extreme phantom case must now fail loud"
        );
        // The largest representable fractional literal round-trips exactly (Postgres
        // renders it with 16383 fractional places → 16385 chars total).
        let edge = PgNumeric::parse("1e-16383").expect("1e-16383 is representable");
        assert_eq!(edge.to_display_string().len(), 16385);
        assert!(
            edge.cmp_value(&PgNumeric::zero()) == Ordering::Greater,
            "and it is genuinely non-zero (no phantom)"
        );
        // Legitimate small numbers in the 1001..=16383 band (which the old 1000-clamp
        // corrupted) now parse and render correctly — no over-rejection.
        for (src, want_len) in [
            ("0.00001", 7),    // "0.00001"
            ("1e-1000", 1002), // "0." + 1000 places
            ("1e-16000", 16002),
        ] {
            let v = PgNumeric::parse(src).unwrap_or_else(|_| panic!("{src} must parse"));
            assert_eq!(v.to_display_string().len(), want_len, "render {src}");
            assert_eq!(v.cmp_value(&PgNumeric::zero()), Ordering::Greater);
        }
    }

    #[test]
    fn round_trunc_carry_past_cap_is_loud() {
        // `<131072 nines>.5` has weight 131071 (in range) and parses. `round(_, 0)`
        // carries to `10^131072` (weight 131072) — the scalar round/trunc entry
        // weight-checks the result, matching Postgres's make_result (22003). `trunc`
        // drops the `.5` with no carry, so it stays in range.
        let nines = format!("{}.5", "9".repeat(131_072));
        let v = n(&nines);
        assert_eq!(v.round(0).check_weight().unwrap_err().sqlstate(), "22003");
        assert!(v.trunc(0).check_weight().is_ok());
    }

    #[test]
    fn div_selects_scale_like_pg() {
        // 10.0 / 3.0 → 16 fractional digits (MIN_SIG_DIGITS, dscale-bumped).
        assert_eq!(
            n("10.0").div(&n("3.0")).unwrap().to_display_string(),
            "3.3333333333333333"
        );
        // scales verified against real Postgres (NBASE-unit `select_div_scale`)
        assert_eq!(
            n("6").div(&n("2")).unwrap().to_display_string(),
            "3.0000000000000000"
        );
        assert_eq!(
            n("1").div(&n("4")).unwrap().to_display_string(),
            "0.25000000000000000000"
        );
        assert_eq!(
            n("7").div(&n("10")).unwrap().to_display_string(),
            "0.70000000000000000000"
        );
        // divide by zero is loud
        assert!(n("1").div(&n("0")).is_err());
        // NaN propagation
        assert!(PgNumeric::NaN.div(&n("2")).unwrap().is_nan());
    }

    // `a`..`f` are one case each of the same rule; naming them would obscure the table.
    #[allow(clippy::many_single_char_names)]
    #[test]
    fn round_half_away_from_zero() {
        let mut a = n("1.25");
        a.round_to_scale(1);
        assert_eq!(a.to_display_string(), "1.3");
        let mut b = n("1.24");
        b.round_to_scale(1);
        assert_eq!(b.to_display_string(), "1.2");
        let mut c = n("-1.25");
        c.round_to_scale(1);
        assert_eq!(c.to_display_string(), "-1.3");
        let mut d = n("9.6");
        d.round_to_scale(0);
        assert_eq!(d.to_display_string(), "10");
        let mut e = n("0.5");
        e.round_to_scale(0);
        assert_eq!(e.to_display_string(), "1");
        let mut f = n("0.4");
        f.round_to_scale(0);
        assert_eq!(f.to_display_string(), "0");
    }

    #[test]
    fn abs_round_trunc_ceil_floor_match_pg() {
        // Values verified against real Postgres.
        assert_eq!(n("-1.50").abs().to_display_string(), "1.50"); // keeps scale
        assert_eq!(n("2.567").round(2).to_display_string(), "2.57");
        assert_eq!(n("-2.5").round(0).to_display_string(), "-3"); // half-away
        assert_eq!(n("0.15").round(1).to_display_string(), "0.2");
        assert_eq!(n("-1.789").trunc(1).to_display_string(), "-1.7"); // toward zero
        assert_eq!(n("2.99").trunc(0).to_display_string(), "2");
        assert_eq!(n("1.5").ceil().unwrap().to_display_string(), "2");
        assert_eq!(n("-1.5").ceil().unwrap().to_display_string(), "-1");
        assert_eq!(n("2.0").ceil().unwrap().to_display_string(), "2");
        assert_eq!(n("1.5").floor().unwrap().to_display_string(), "1");
        assert_eq!(n("-1.5").floor().unwrap().to_display_string(), "-2");
        assert_eq!(n("-2.0").floor().unwrap().to_display_string(), "-2");
        // NaN passes through every operation.
        for out in [
            PgNumeric::NaN.abs(),
            PgNumeric::NaN.round(2),
            PgNumeric::NaN.trunc(0),
            PgNumeric::NaN.ceil().unwrap(),
            PgNumeric::NaN.floor().unwrap(),
        ] {
            assert!(out.is_nan());
        }
    }

    #[test]
    fn typmod_rounds_and_overflows() {
        let mut a = n("1.239");
        a.apply_typmod(10, 2).unwrap();
        assert_eq!(a.to_display_string(), "1.24");
        // 123.45 into numeric(4,2): integer part 3 digits > (4-2)=2 → overflow.
        let mut b = n("123.45");
        assert!(b.apply_typmod(4, 2).is_err());
        // exactly fits: 99.99 into numeric(4,2)
        let mut c = n("99.99");
        c.apply_typmod(4, 2).unwrap();
        assert_eq!(c.to_display_string(), "99.99");
    }

    #[test]
    fn int_and_float_conversions() {
        assert_eq!(PgNumeric::from_i128(0).to_display_string(), "0");
        assert_eq!(PgNumeric::from_i128(-12345).to_display_string(), "-12345");
        assert_eq!(
            PgNumeric::from_i128(i128::MAX).to_i128().unwrap(),
            i128::MAX
        );
        assert_eq!(
            PgNumeric::from_i128(i128::MIN).to_i128().unwrap(),
            i128::MIN
        );
        assert_eq!(n("2.5").to_i128().unwrap(), 3); // half away
        assert_eq!(n("-2.5").to_i128().unwrap(), -3);
        assert_eq!(PgNumeric::from_f64(0.1).unwrap().to_display_string(), "0.1");
        assert_eq!(PgNumeric::from_f64(1.5).unwrap().to_display_string(), "1.5");
        assert!((n("1.5").to_f64() - 1.5).abs() < 1e-12);
        assert!(PgNumeric::from_f64(f64::NAN).unwrap().is_nan());
        assert!(PgNumeric::from_f64(f64::INFINITY).is_err());
    }

    #[test]
    fn order_key_preserves_value_order() {
        // A spread across signs, magnitudes, scales, zero, and NaN. The byte-key
        // order must match `cmp_value` for every pair.
        let mut vals: Vec<PgNumeric> = [
            "-1000", "-10", "-1.5", "-1", "-0.5", "-0.01", "0", "0.01", "0.5", "1", "1.5", "1.50",
            "10", "100", "999.99", "1000", "1e20",
        ]
        .iter()
        .map(|s| n(s))
        .collect();
        vals.push(PgNumeric::NaN);
        for a in &vals {
            for b in &vals {
                let key_ord = a.encode_order_key().cmp(&b.encode_order_key());
                assert_eq!(
                    key_ord,
                    a.cmp_value(b),
                    "key order disagrees for {} vs {}",
                    a.to_display_string(),
                    b.to_display_string()
                );
            }
        }
        // Equal values (1.5 == 1.50) encode to the identical key.
        assert_eq!(n("1.5").encode_order_key(), n("1.50").encode_order_key());
    }

    #[test]
    fn wire_round_trips() {
        for s in [
            "0",
            "1",
            "-1",
            "1.50",
            "123.45",
            "0.0045",
            "10000",
            "9999.9999",
            "NaN",
            "1000000",
        ] {
            let v = PgNumeric::parse(s).unwrap();
            let bytes = v.to_wire_bytes();
            let back = PgNumeric::recv(&mut BinaryReader::new(&bytes)).unwrap();
            assert_eq!(v, back, "wire round-trip {s}");
            // dscale survives too (display equality)
            assert_eq!(
                v.to_display_string(),
                back.to_display_string(),
                "dscale {s}"
            );
        }
    }

    #[test]
    fn wire_matches_known_pg_bytes() {
        // Postgres numeric_send for 1.50 aligns NBASE groups to the decimal point:
        // ndigits=2, weight=0, sign=POS, dscale=2, digits [1, 5000] (1·NBASE^0 +
        // 5000·NBASE^-1 = 1 + 0.5). Verify the exact header + digits.
        let bytes = n("1.50").to_wire_bytes();
        assert_eq!(&bytes[0..2], &2i16.to_be_bytes()); // ndigits
        assert_eq!(&bytes[2..4], &0i16.to_be_bytes()); // weight
        assert_eq!(&bytes[4..6], &PG_NUMERIC_POS.to_be_bytes());
        assert_eq!(&bytes[6..8], &2i16.to_be_bytes()); // dscale
        assert_eq!(&bytes[8..10], &1i16.to_be_bytes()); // NBASE digit 1
        assert_eq!(&bytes[10..12], &5000i16.to_be_bytes()); // NBASE digit 5000
    }

    #[test]
    fn rem_sign_follows_dividend_like_pg() {
        // Sign of the dividend, truncated toward zero (values verified vs Postgres).
        assert_eq!(n("7").rem(&n("3")).unwrap().to_display_string(), "1");
        assert_eq!(n("-7").rem(&n("3")).unwrap().to_display_string(), "-1");
        assert_eq!(n("7").rem(&n("-3")).unwrap().to_display_string(), "1");
        assert_eq!(n("10.5").rem(&n("3")).unwrap().to_display_string(), "1.5");
        // Large operands (> f64 exact-integer range): `div().trunc(0)` would give the
        // wrong remainder here (verified against Postgres — the exact-truncation path).
        assert_eq!(
            n("100000000000000001")
                .rem(&n("3"))
                .unwrap()
                .to_display_string(),
            "2"
        );
        assert_eq!(
            n("100000000000000007")
                .rem(&n("7"))
                .unwrap()
                .to_display_string(),
            "5"
        );
        // Zero divisor is loud; NaN propagates.
        assert!(n("7").rem(&n("0")).is_err());
        assert!(PgNumeric::NaN.rem(&n("3")).unwrap().is_nan());
        assert!(n("3").rem(&PgNumeric::NaN).unwrap().is_nan());
    }

    #[test]
    fn pow_integer_exponent_matches_pg() {
        // Result scale + value verified against real Postgres `numeric ^ numeric`.
        assert_eq!(
            n("2.0").pow(&n("3")).unwrap().to_display_string(),
            "8.0000000000000000"
        );
        assert_eq!(
            n("2.5").pow(&n("2")).unwrap().to_display_string(),
            "6.2500000000000000"
        );
        assert_eq!(
            n("1.1").pow(&n("3")).unwrap().to_display_string(),
            "1.3310000000000000"
        );
        assert_eq!(
            n("10").pow(&n("3")).unwrap().to_display_string(),
            "1000.0000000000000"
        );
        assert_eq!(
            n("2").pow(&n("10")).unwrap().to_display_string(),
            "1024.0000000000000"
        );
        // Negative (terminating) exponents.
        assert_eq!(
            n("2").pow(&n("-1")).unwrap().to_display_string(),
            "0.5000000000000000"
        );
        assert_eq!(
            n("4").pow(&n("-2")).unwrap().to_display_string(),
            "0.06250000000000000"
        );
    }

    #[test]
    fn pow_domain_and_nan_rules() {
        // 0 ^ negative → 2201F (numeric_power raises the power-domain error before
        // reaching power_var_int's internal division-by-zero).
        assert_eq!(n("0").pow(&n("-1")).unwrap_err().sqlstate(), "2201F");
        // Negative base, non-integer exponent → 2201F.
        assert_eq!(n("-2").pow(&n("0.5")).unwrap_err().sqlstate(), "2201F");
        // Positive base, fractional exponent → deferred (0A000).
        assert_eq!(n("2").pow(&n("0.5")).unwrap_err().sqlstate(), "0A000");
        // POSIX NaN rules: NaN^0 = 1, 1^NaN = 1, else NaN.
        assert_eq!(
            PgNumeric::NaN.pow(&n("0")).unwrap().to_display_string(),
            "1"
        );
        assert_eq!(
            n("1").pow(&PgNumeric::NaN).unwrap().to_display_string(),
            "1"
        );
        assert!(PgNumeric::NaN.pow(&n("2")).unwrap().is_nan());
        assert!(n("2").pow(&PgNumeric::NaN).unwrap().is_nan());
    }

    #[test]
    fn sqrt_scale_and_value_match_pg() {
        // Scale + value verified against real Postgres `sqrt(numeric)`.
        assert_eq!(
            n("4").sqrt().unwrap().to_display_string(),
            "2.000000000000000"
        );
        assert_eq!(
            n("2.25").sqrt().unwrap().to_display_string(),
            "1.500000000000000"
        );
        assert_eq!(
            n("2").sqrt().unwrap().to_display_string(),
            "1.414213562373095"
        );
        assert_eq!(
            n("400").sqrt().unwrap().to_display_string(),
            "20.000000000000000"
        );
        assert_eq!(
            n("0.04").sqrt().unwrap().to_display_string(),
            "0.20000000000000000"
        );
        assert_eq!(
            n("10000").sqrt().unwrap().to_display_string(),
            "100.0000000000000"
        );
        assert_eq!(
            n("0").sqrt().unwrap().to_display_string(),
            "0.000000000000000"
        );
        assert_eq!(
            n("0.25").sqrt().unwrap().to_display_string(),
            "0.50000000000000000"
        );
        // Table-maker's-dilemma boundaries: Newton alone rounds the last digit the
        // wrong way; the exact ½-ulp correction fixes it (verified vs Postgres).
        assert_eq!(
            n("1.000000000000001").sqrt().unwrap().to_display_string(),
            "1.000000000000000"
        );
        assert_eq!(
            n("4.00000000000001").sqrt().unwrap().to_display_string(),
            "2.000000000000002"
        );
        // Magnitude beyond f64 range: the seed comes from the numeric's own weight,
        // so this succeeds where an `f64` round-trip would give `inf` → `22003`.
        assert_eq!(
            n("1e320").sqrt().unwrap().to_display_string(),
            "1".to_string() + &"0".repeat(160)
        );
        // Negative → 2201F; NaN → NaN.
        assert_eq!(n("-1").sqrt().unwrap_err().sqlstate(), "2201F");
        assert!(PgNumeric::NaN.sqrt().unwrap().is_nan());
    }

    #[test]
    fn helpers_exact_integer_and_sign() {
        assert_eq!(n("42").to_exact_i64(), Some(42));
        assert_eq!(n("-42").to_exact_i64(), Some(-42));
        assert_eq!(n("0").to_exact_i64(), Some(0));
        assert_eq!(n("100").to_exact_i64(), Some(100)); // trailing-zero integer
        assert_eq!(n("1.5").to_exact_i64(), None); // fractional
        assert_eq!(PgNumeric::NaN.to_exact_i64(), None);
        assert!(n("-3").is_negative());
        assert!(!n("0").is_negative());
        assert!(!n("3").is_negative());
        assert!(!PgNumeric::NaN.is_negative());
        assert_eq!(n("1.50").display_scale(), 2);
        assert_eq!(n("7").display_scale(), 0);
    }
}
