//! The functions a FROM clause can call ([`FromFunction`]) and the argument verdicts of
//! `generate_series` — 18.6's overload resolution over its signatures, read off the
//! arguments as written.

use serde::Serialize;

use super::expr::{Expr, Literal, SetQuantifier};
use crate::types::{TypeOid, opt_type_oid_display_name};
use crate::{Error, Result};

/// The function a [`super::Plan::FunctionScan`] calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FromFunction {
    /// `unnest(arr, …)`: one column per array, zipped.
    Unnest,
    /// `generate_series(start, stop[, step])` over the integer overload it resolved to.
    GenerateSeries(SeriesType),
}

impl FromFunction {
    /// The function's name — its default relation and column name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Unnest => "unnest",
            Self::GenerateSeries(_) => "generate_series",
        }
    }
}

/// The `generate_series` overload pglet runs: `(int4, int4[, int4])` or
/// `(int8, int8[, int8])`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SeriesType {
    Int4,
    Int8,
}

impl SeriesType {
    /// The overload's argument and result type.
    #[must_use]
    pub const fn type_oid(self) -> TypeOid {
        match self {
            Self::Int4 => TypeOid::Int4,
            Self::Int8 => TypeOid::Int8,
        }
    }
}

/// The longest series a FROM-clause `generate_series` answers.
///
/// A FROM item's rows are materialized before anything above it runs — 18.6's
/// `FunctionScan` fills a tuplestore too, which spills to disk where pglet has only the
/// heap — so a longer series is refused `54000` from its evaluated arguments, before a
/// row is built.
pub const MAX_SERIES_ROWS: u64 = 1_000_000;

/// Resolve a FROM-clause `generate_series(args)` to its overload, returning the
/// arguments converted to the overload's type.
///
/// Each argument is read three ways, never two: a type it is **certain** of as written
/// (`super::certain_from_argument_type`); **unknown** — a bare string literal, a bare
/// `NULL` or a bare `$N` — which takes the overload's type, as 18.6's resolution gives
/// it; or neither, which is refused. A type guessed for that third kind would choose the
/// overload, and an `int4` declared over an `int8` series is silent. An argument naming
/// a column or holding a sub-plan is refused before it is read: the item is evaluated
/// once, with no row, so a column there is not the row 18.6 reads it from.
///
/// The resolution is 18.6's over `generate_series`'s signatures: two or three arguments
/// of `int2` / `int4` / `int8` / `numeric` or unknown resolve to the widest of
/// `numeric`, `int8`, `int4` written, and to no unique signature when none is (`42725`);
/// three of `date` / `timestamp` / `timestamptz` or unknown beside an `interval` or
/// unknown step — and four, with a zone name — resolve a temporal series; anything else
/// is `42883`. The `numeric` and temporal series are refused `0A000`.
///
/// # Errors
///
/// `Error::NotImplemented` for a column reference, a sub-plan, a set-returning or window
/// call, an argument of no certain type, or a `numeric` or temporal overload;
/// `Error::UndefinedFunction` / `Error::AmbiguousFunction` as 18.6 resolves;
/// `Error::NotAnAggregate` for a `DISTINCT`; and a string literal's conversion error
/// under the resolved type.
pub fn resolve_generate_series(
    quantifier: SetQuantifier,
    args: Vec<Expr>,
) -> Result<(SeriesType, Vec<Expr>)> {
    if args.iter().any(reads_beyond_constants) {
        return Err(Error::NotImplemented(
            "generate_series with a column reference or a subquery in an argument".into(),
        ));
    }
    let mut types = Vec::with_capacity(args.len());
    for arg in &args {
        types.push(match arg {
            Expr::Literal(Literal::String(_) | Literal::Null) | Expr::Parameter { .. } => None,
            _ => Some(super::certain_from_argument_type(arg).ok_or_else(|| {
                Error::NotImplemented(
                    "generate_series over an argument whose type is not known as written".into(),
                )
            })?),
        });
    }
    let series = match resolve_series_overload(&types) {
        Overload::Int4 => SeriesType::Int4,
        Overload::Int8 => SeriesType::Int8,
        Overload::Numeric => {
            return Err(Error::NotImplemented("generate_series over numeric".into()));
        }
        Overload::Temporal => {
            return Err(Error::NotImplemented(
                "generate_series over timestamps".into(),
            ));
        }
        Overload::Ambiguous => {
            return Err(Error::AmbiguousFunction(signature(&types)));
        }
        Overload::None => {
            return Err(Error::UndefinedFunction(format!(
                "function {} does not exist",
                signature(&types)
            )));
        }
    };
    // A `DISTINCT` is refused once the signature resolves, as `ParseFuncOrColumn` refuses
    // it — ahead of the argument conversion below.
    quantifier.refuse_distinct(FromFunction::GenerateSeries(series).name())?;
    let target_type = series.type_oid();
    args.into_iter()
        .zip(types)
        .map(|(arg, t)| {
            if t == Some(target_type) {
                return Ok(arg);
            }
            let cast = Expr::Cast {
                expr: Box::new(arg),
                target_type,
            };
            // An unknown literal is converted in parse analysis, as 18.6 converts it.
            super::validate_const_casts(&cast)?;
            Ok(cast)
        })
        .collect::<Result<_>>()
        .map(|args| (series, args))
}

/// Whether `expr` reads anything but constants and parameters: a column, a sub-plan, a
/// set-returning `unnest`, a window function, or an aggregate.
fn reads_beyond_constants(expr: &Expr) -> bool {
    expr.any_node(&|node| {
        matches!(
            node,
            Expr::Column { .. }
                | Expr::Subquery { .. }
                | Expr::InSubquery { .. }
                | Expr::Exists { .. }
                | Expr::Unnest(_)
                | Expr::Window { .. }
                | Expr::Aggregate { .. }
                | Expr::Default
        )
        .then_some(true)
    })
}

/// [`resolve_series_overload`]'s verdict.
enum Overload {
    Int4,
    Int8,
    Numeric,
    Temporal,
    Ambiguous,
    None,
}

/// 18.6's resolution of `generate_series` over argument types, `None` being unknown.
fn resolve_series_overload(types: &[Option<TypeOid>]) -> Overload {
    use TypeOid::{Date, Int2, Int4, Int8, Interval, Name, Numeric, Text, Timestamp, Timestamptz};
    let is = |t: Option<TypeOid>, set: &[TypeOid]| t.is_none_or(|t| set.contains(&t));
    let n = types.len();
    if (2..=3).contains(&n) && types.iter().all(|t| is(*t, &[Int2, Int4, Int8, Numeric])) {
        let any = |want| types.contains(&Some(want));
        return if any(Numeric) {
            Overload::Numeric
        } else if any(Int8) {
            Overload::Int8
        } else if any(Int4) {
            Overload::Int4
        } else {
            Overload::Ambiguous
        };
    }
    let temporal = [Date, Timestamp, Timestamptz];
    if (3..=4).contains(&n)
        && is(types[0], &temporal)
        && is(types[1], &temporal)
        && is(types[2], &[Interval])
        && types
            .get(3)
            .is_none_or(|t| is(*t, &[Text, TypeOid::Varchar, Name, TypeOid::Char]))
    {
        return Overload::Temporal;
    }
    Overload::None
}

/// `generate_series(<types>)` as 18.6's resolution errors spell it.
fn signature(types: &[Option<TypeOid>]) -> String {
    let names: Vec<&str> = types
        .iter()
        .map(|t| opt_type_oid_display_name(*t))
        .collect();
    format!("generate_series({})", names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::{Overload, resolve_series_overload};
    use crate::types::TypeOid;

    /// Every argument-type combination 18.6 resolves to a signature or refuses as not
    /// unique, over the types below at arities zero to four, measured with `PREPARE` over
    /// `NULL::<type>` arguments (a bare `NULL` for `unknown`) and read from
    /// `pg_prepared_statements.result_types`. Every combination not listed is `42883`.
    const ANSWERING: &str = "\
int2 int2 -> 42725
int2 int4 -> int4
int2 int8 -> int8
int2 numeric -> numeric
int2 unknown -> 42725
int4 int2 -> int4
int4 int4 -> int4
int4 int8 -> int8
int4 numeric -> numeric
int4 unknown -> int4
int8 int2 -> int8
int8 int4 -> int8
int8 int8 -> int8
int8 numeric -> numeric
int8 unknown -> int8
numeric int2 -> numeric
numeric int4 -> numeric
numeric int8 -> numeric
numeric numeric -> numeric
numeric unknown -> numeric
unknown int2 -> 42725
unknown int4 -> int4
unknown int8 -> int8
unknown numeric -> numeric
unknown unknown -> 42725
int2 int2 int2 -> 42725
int2 int2 int4 -> int4
int2 int2 int8 -> int8
int2 int2 numeric -> numeric
int2 int2 unknown -> 42725
int2 int4 int2 -> int4
int2 int4 int4 -> int4
int2 int4 int8 -> int8
int2 int4 numeric -> numeric
int2 int4 unknown -> int4
int2 int8 int2 -> int8
int2 int8 int4 -> int8
int2 int8 int8 -> int8
int2 int8 numeric -> numeric
int2 int8 unknown -> int8
int2 numeric int2 -> numeric
int2 numeric int4 -> numeric
int2 numeric int8 -> numeric
int2 numeric numeric -> numeric
int2 numeric unknown -> numeric
int2 unknown int2 -> 42725
int2 unknown int4 -> int4
int2 unknown int8 -> int8
int2 unknown numeric -> numeric
int2 unknown unknown -> 42725
int4 int2 int2 -> int4
int4 int2 int4 -> int4
int4 int2 int8 -> int8
int4 int2 numeric -> numeric
int4 int2 unknown -> int4
int4 int4 int2 -> int4
int4 int4 int4 -> int4
int4 int4 int8 -> int8
int4 int4 numeric -> numeric
int4 int4 unknown -> int4
int4 int8 int2 -> int8
int4 int8 int4 -> int8
int4 int8 int8 -> int8
int4 int8 numeric -> numeric
int4 int8 unknown -> int8
int4 numeric int2 -> numeric
int4 numeric int4 -> numeric
int4 numeric int8 -> numeric
int4 numeric numeric -> numeric
int4 numeric unknown -> numeric
int4 unknown int2 -> int4
int4 unknown int4 -> int4
int4 unknown int8 -> int8
int4 unknown numeric -> numeric
int4 unknown unknown -> int4
int8 int2 int2 -> int8
int8 int2 int4 -> int8
int8 int2 int8 -> int8
int8 int2 numeric -> numeric
int8 int2 unknown -> int8
int8 int4 int2 -> int8
int8 int4 int4 -> int8
int8 int4 int8 -> int8
int8 int4 numeric -> numeric
int8 int4 unknown -> int8
int8 int8 int2 -> int8
int8 int8 int4 -> int8
int8 int8 int8 -> int8
int8 int8 numeric -> numeric
int8 int8 unknown -> int8
int8 numeric int2 -> numeric
int8 numeric int4 -> numeric
int8 numeric int8 -> numeric
int8 numeric numeric -> numeric
int8 numeric unknown -> numeric
int8 unknown int2 -> int8
int8 unknown int4 -> int8
int8 unknown int8 -> int8
int8 unknown numeric -> numeric
int8 unknown unknown -> int8
numeric int2 int2 -> numeric
numeric int2 int4 -> numeric
numeric int2 int8 -> numeric
numeric int2 numeric -> numeric
numeric int2 unknown -> numeric
numeric int4 int2 -> numeric
numeric int4 int4 -> numeric
numeric int4 int8 -> numeric
numeric int4 numeric -> numeric
numeric int4 unknown -> numeric
numeric int8 int2 -> numeric
numeric int8 int4 -> numeric
numeric int8 int8 -> numeric
numeric int8 numeric -> numeric
numeric int8 unknown -> numeric
numeric numeric int2 -> numeric
numeric numeric int4 -> numeric
numeric numeric int8 -> numeric
numeric numeric numeric -> numeric
numeric numeric unknown -> numeric
numeric unknown int2 -> numeric
numeric unknown int4 -> numeric
numeric unknown int8 -> numeric
numeric unknown numeric -> numeric
numeric unknown unknown -> numeric
unknown int2 int2 -> 42725
unknown int2 int4 -> int4
unknown int2 int8 -> int8
unknown int2 numeric -> numeric
unknown int2 unknown -> 42725
unknown int4 int2 -> int4
unknown int4 int4 -> int4
unknown int4 int8 -> int8
unknown int4 numeric -> numeric
unknown int4 unknown -> int4
unknown int8 int2 -> int8
unknown int8 int4 -> int8
unknown int8 int8 -> int8
unknown int8 numeric -> numeric
unknown int8 unknown -> int8
unknown numeric int2 -> numeric
unknown numeric int4 -> numeric
unknown numeric int8 -> numeric
unknown numeric numeric -> numeric
unknown numeric unknown -> numeric
unknown unknown int2 -> 42725
unknown unknown int4 -> int4
unknown unknown int8 -> int8
unknown unknown numeric -> numeric
unknown unknown unknown -> 42725
unknown unknown interval -> timestamptz
unknown date unknown -> timestamptz
unknown date interval -> timestamptz
unknown timestamp unknown -> timestamp
unknown timestamp interval -> timestamp
unknown timestamptz unknown -> timestamptz
unknown timestamptz interval -> timestamptz
date unknown unknown -> timestamptz
date unknown interval -> timestamptz
date date unknown -> timestamptz
date date interval -> timestamptz
date timestamp unknown -> timestamp
date timestamp interval -> timestamp
date timestamptz unknown -> timestamptz
date timestamptz interval -> timestamptz
timestamp unknown unknown -> timestamp
timestamp unknown interval -> timestamp
timestamp date unknown -> timestamp
timestamp date interval -> timestamp
timestamp timestamp unknown -> timestamp
timestamp timestamp interval -> timestamp
timestamp timestamptz unknown -> timestamptz
timestamp timestamptz interval -> timestamptz
timestamptz unknown unknown -> timestamptz
timestamptz unknown interval -> timestamptz
timestamptz date unknown -> timestamptz
timestamptz date interval -> timestamptz
timestamptz timestamp unknown -> timestamptz
timestamptz timestamp interval -> timestamptz
timestamptz timestamptz unknown -> timestamptz
timestamptz timestamptz interval -> timestamptz
unknown unknown unknown unknown -> timestamptz
unknown unknown unknown text -> timestamptz
unknown unknown unknown varchar -> timestamptz
unknown unknown interval unknown -> timestamptz
unknown unknown interval text -> timestamptz
unknown unknown interval varchar -> timestamptz
unknown date unknown unknown -> timestamptz
unknown date unknown text -> timestamptz
unknown date unknown varchar -> timestamptz
unknown date interval unknown -> timestamptz
unknown date interval text -> timestamptz
unknown date interval varchar -> timestamptz
unknown timestamp unknown unknown -> timestamptz
unknown timestamp unknown text -> timestamptz
unknown timestamp unknown varchar -> timestamptz
unknown timestamp interval unknown -> timestamptz
unknown timestamp interval text -> timestamptz
unknown timestamp interval varchar -> timestamptz
unknown timestamptz unknown unknown -> timestamptz
unknown timestamptz unknown text -> timestamptz
unknown timestamptz unknown varchar -> timestamptz
unknown timestamptz interval unknown -> timestamptz
unknown timestamptz interval text -> timestamptz
unknown timestamptz interval varchar -> timestamptz
date unknown unknown unknown -> timestamptz
date unknown unknown text -> timestamptz
date unknown unknown varchar -> timestamptz
date unknown interval unknown -> timestamptz
date unknown interval text -> timestamptz
date unknown interval varchar -> timestamptz
date date unknown unknown -> timestamptz
date date unknown text -> timestamptz
date date unknown varchar -> timestamptz
date date interval unknown -> timestamptz
date date interval text -> timestamptz
date date interval varchar -> timestamptz
date timestamp unknown unknown -> timestamptz
date timestamp unknown text -> timestamptz
date timestamp unknown varchar -> timestamptz
date timestamp interval unknown -> timestamptz
date timestamp interval text -> timestamptz
date timestamp interval varchar -> timestamptz
date timestamptz unknown unknown -> timestamptz
date timestamptz unknown text -> timestamptz
date timestamptz unknown varchar -> timestamptz
date timestamptz interval unknown -> timestamptz
date timestamptz interval text -> timestamptz
date timestamptz interval varchar -> timestamptz
timestamp unknown unknown unknown -> timestamptz
timestamp unknown unknown text -> timestamptz
timestamp unknown unknown varchar -> timestamptz
timestamp unknown interval unknown -> timestamptz
timestamp unknown interval text -> timestamptz
timestamp unknown interval varchar -> timestamptz
timestamp date unknown unknown -> timestamptz
timestamp date unknown text -> timestamptz
timestamp date unknown varchar -> timestamptz
timestamp date interval unknown -> timestamptz
timestamp date interval text -> timestamptz
timestamp date interval varchar -> timestamptz
timestamp timestamp unknown unknown -> timestamptz
timestamp timestamp unknown text -> timestamptz
timestamp timestamp unknown varchar -> timestamptz
timestamp timestamp interval unknown -> timestamptz
timestamp timestamp interval text -> timestamptz
timestamp timestamp interval varchar -> timestamptz
timestamp timestamptz unknown unknown -> timestamptz
timestamp timestamptz unknown text -> timestamptz
timestamp timestamptz unknown varchar -> timestamptz
timestamp timestamptz interval unknown -> timestamptz
timestamp timestamptz interval text -> timestamptz
timestamp timestamptz interval varchar -> timestamptz
timestamptz unknown unknown unknown -> timestamptz
timestamptz unknown unknown text -> timestamptz
timestamptz unknown unknown varchar -> timestamptz
timestamptz unknown interval unknown -> timestamptz
timestamptz unknown interval text -> timestamptz
timestamptz unknown interval varchar -> timestamptz
timestamptz date unknown unknown -> timestamptz
timestamptz date unknown text -> timestamptz
timestamptz date unknown varchar -> timestamptz
timestamptz date interval unknown -> timestamptz
timestamptz date interval text -> timestamptz
timestamptz date interval varchar -> timestamptz
timestamptz timestamp unknown unknown -> timestamptz
timestamptz timestamp unknown text -> timestamptz
timestamptz timestamp unknown varchar -> timestamptz
timestamptz timestamp interval unknown -> timestamptz
timestamptz timestamp interval text -> timestamptz
timestamptz timestamp interval varchar -> timestamptz
timestamptz timestamptz unknown unknown -> timestamptz
timestamptz timestamptz unknown text -> timestamptz
timestamptz timestamptz unknown varchar -> timestamptz
timestamptz timestamptz interval unknown -> timestamptz
timestamptz timestamptz interval text -> timestamptz
timestamptz timestamptz interval varchar -> timestamptz
";

    const TYPES: [(&str, Option<TypeOid>); 14] = [
        ("int2", Some(TypeOid::Int2)),
        ("int4", Some(TypeOid::Int4)),
        ("int8", Some(TypeOid::Int8)),
        ("numeric", Some(TypeOid::Numeric)),
        ("float4", Some(TypeOid::Float4)),
        ("float8", Some(TypeOid::Float8)),
        ("unknown", None),
        ("text", Some(TypeOid::Text)),
        ("varchar", Some(TypeOid::Varchar)),
        ("date", Some(TypeOid::Date)),
        ("timestamp", Some(TypeOid::Timestamp)),
        ("timestamptz", Some(TypeOid::Timestamptz)),
        ("interval", Some(TypeOid::Interval)),
        ("bool", Some(TypeOid::Bool)),
    ];

    /// The overload rule reproduces 18.6's verdict over the whole measured matrix.
    #[test]
    fn the_overload_rule_is_the_measured_matrix() {
        let expected: std::collections::HashMap<&str, &str> = ANSWERING
            .lines()
            .filter_map(|line| line.split_once(" -> "))
            .collect();
        let mut combos: Vec<Vec<usize>> = vec![Vec::new()];
        let mut checked = 0;
        for _arity in 0..=4 {
            for combo in &combos {
                let names: Vec<&str> = combo.iter().map(|&i| TYPES[i].0).collect();
                let types: Vec<Option<TypeOid>> = combo.iter().map(|&i| TYPES[i].1).collect();
                let got = match resolve_series_overload(&types) {
                    Overload::Int4 => "int4",
                    Overload::Int8 => "int8",
                    Overload::Numeric => "numeric",
                    // The two temporal results are one verdict here: refused alike.
                    Overload::Temporal => "temporal",
                    Overload::Ambiguous => "42725",
                    Overload::None => "42883",
                };
                let want = match expected.get(names.join(" ").as_str()).copied() {
                    Some("timestamp" | "timestamptz") => "temporal",
                    Some(verdict) => verdict,
                    None => "42883",
                };
                assert_eq!(got, want, "generate_series({})", names.join(", "));
                checked += 1;
            }
            combos = combos
                .iter()
                .flat_map(|c| {
                    (0..TYPES.len()).map(move |i| {
                        let mut next = c.clone();
                        next.push(i);
                        next
                    })
                })
                .collect();
        }
        assert_eq!(checked, 1 + 14 + 14 * 14 + 14 * 14 * 14 + 14 * 14 * 14 * 14);
    }
}
