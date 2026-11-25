//! String, catalog-information and variadic scalar functions.

use super::arithmetic::value_as_i64;
use super::compare::{values_compare_ordered, values_equal};
use super::temporal_fn::eval_date_trunc;
use crate::catalog::Schema;
use crate::executor::coerce::coerce_value;
use crate::planner::{
    ResolvedType, StringFunc, VariadicFunc, comparison_legality, comparison_operand_type,
    nullif_result_type,
};
use crate::types::{
    ArrayValue, PgInterval, TypeOid, Value, opt_type_oid_display_name, value_to_text_with_oid,
};
use crate::{CatalogError, Error, Result};

// ============================================================================
// String functions (`Expr::StringFn` / `Expr::VariadicFn`)
// ============================================================================

/// Evaluate a strict-NULL string function ([`StringFunc`]).
///
/// Any NULL argument short-circuits to NULL (the hoisted check, mirroring
/// [`eval_math_fn`](super::math_fn::eval_math_fn)); otherwise each argument is validated to its expected type — a
/// wrong type fails loud with `42883` (Postgres's function resolution: there is no
/// `length(integer)`) — and the result is computed **1-based** and **character**-based
/// (never byte-based), matching Postgres under a multibyte encoding.
pub(super) fn eval_string_fn(
    func: StringFunc,
    args: &[Value],
    schema: Option<&Schema>,
) -> Result<Value> {
    // `set_config` is not strict on 18.6 (`proisstrict = f`): it is answered ahead of the
    // family's NULL rule ([`StringFunc::SetConfigLockTimeout`]).
    if func == StringFunc::SetConfigLockTimeout {
        return eval_set_config_lock_timeout(args);
    }
    if args.iter().any(Value::is_null) {
        return Ok(Value::Null);
    }
    match func {
        StringFunc::Upper => Ok(Value::Text(str_arg(&args[0], "upper")?.to_uppercase())),
        StringFunc::Length => string_length(&args[0]),
        StringFunc::Replace => Ok(Value::Text(string_replace(
            str_arg(&args[0], "replace")?,
            str_arg(&args[1], "replace")?,
            str_arg(&args[2], "replace")?,
        ))),
        StringFunc::Substring => {
            let start = int_arg(&args[1], "substring")?;
            let length = args.get(2).map(|v| int_arg(v, "substring")).transpose()?;
            match &args[0] {
                Value::Bytea(b) => bytea_substring(b, start, length),
                other => string_substring(str_arg(other, "substring")?, start, length),
            }
        }
        // `substr` takes its overloads' `int4`s strictly, as 18.6 resolves them; the
        // `substring` arm above keeps its older acceptance of an `int8`.
        StringFunc::Substr => {
            let start = int4_arg(args, 1, "substr")?;
            let length = args
                .get(2)
                .map(|_| int4_arg(args, 2, "substr"))
                .transpose()?;
            match &args[0] {
                Value::Text(s) => string_substring(s, start.into(), length.map(i64::from)),
                Value::Bytea(b) => bytea_substring(b, start.into(), length.map(i64::from)),
                _ => Err(no_such_overload("substr", args)),
            }
        }
        StringFunc::Left | StringFunc::Right => {
            let name = if func == StringFunc::Left {
                "left"
            } else {
                "right"
            };
            let n = int4_arg(args, 1, name)?;
            match &args[0] {
                Value::Text(s) if func == StringFunc::Left => Ok(string_left(s, n)),
                Value::Text(s) => Ok(string_right(s, n)),
                _ => Err(no_such_overload(name, args)),
            }
        }
        StringFunc::OctetLength => {
            let n = match &args[0] {
                Value::Text(s) => s.len(),
                Value::Bytea(b) => b.len(),
                _ => return Err(no_such_overload("octet_length", args)),
            };
            i32::try_from(n)
                .map(Value::Int4)
                .map_err(|_| Error::NumericOutOfRange("integer"))
        }
        StringFunc::Ltrim => Ok(Value::Text(string_trim(
            str_arg(&args[0], "ltrim")?,
            text_or_space(args.get(1), "ltrim")?,
            true,
            false,
        ))),
        StringFunc::Rtrim => Ok(Value::Text(string_trim(
            str_arg(&args[0], "rtrim")?,
            text_or_space(args.get(1), "rtrim")?,
            false,
            true,
        ))),
        StringFunc::Btrim => Ok(Value::Text(string_trim(
            str_arg(&args[0], "btrim")?,
            text_or_space(args.get(1), "btrim")?,
            true,
            true,
        ))),
        // `strpos(str, sub)` and the `POSITION(sub IN str)` form (same [str, sub]
        // arg order the parser builds) — 1-based character position.
        StringFunc::Strpos | StringFunc::Position => {
            let n = i32::try_from(string_strpos(
                str_arg(&args[0], "strpos")?,
                str_arg(&args[1], "strpos")?,
            ))
            .map_err(|_| Error::NumericOutOfRange("integer"))?;
            Ok(Value::Int4(n))
        }
        StringFunc::Lpad => Ok(Value::Text(string_pad(
            str_arg(&args[0], "lpad")?,
            int_arg(&args[1], "lpad")?,
            text_or_space(args.get(2), "lpad")?,
            true,
        )?)),
        StringFunc::Rpad => Ok(Value::Text(string_pad(
            str_arg(&args[0], "rpad")?,
            int_arg(&args[1], "rpad")?,
            text_or_space(args.get(2), "rpad")?,
            false,
        )?)),
        StringFunc::SplitPart => string_split_part(
            str_arg(&args[0], "split_part")?,
            str_arg(&args[1], "split_part")?,
            int_arg(&args[2], "split_part")?,
        ),
        // `date_trunc(field, source)` — field/source non-NULL here (hoisted check).
        StringFunc::DateTrunc => eval_date_trunc(&args[0], &args[1]),
        // `current_setting(name [, missing_ok])`. Both arguments are non-NULL here
        // (the hoisted check *is* 18.6's strictness, in both arguments — measured).
        // The lookup is `SHOW`'s own table, so an unknown name raises the identical
        // `42704`; `missing_ok` swallows it into NULL, which is the only thing the
        // second argument does.
        StringFunc::CurrentSetting => {
            let name = str_arg(&args[0], "current_setting")?;
            let missing_ok = match args.get(1) {
                None => false,
                Some(v) => bool_arg(v, "current_setting")?,
            };
            let settings = crate::catalog::SessionSettings::ambient();
            match crate::catalog::setting_value(name, &settings) {
                Some(value) => Ok(Value::Text(value)),
                None if missing_ok => Ok(Value::Null),
                None => Err(Error::UnrecognizedParameter(name.to_string())),
            }
        }
        // `current_schemas(include_implicit)` — the one array-returning member here.
        StringFunc::CurrentSchemas => Ok(Value::Array(ArrayValue::new(
            TypeOid::Name,
            crate::catalog::current_schemas(bool_arg(&args[0], "current_schemas")?)
                .into_iter()
                .map(|s| Value::Text(s.to_string()))
                .collect(),
        ))),
        // The two catalog-reading members. `schema` is `None` in a context that has no
        // catalog to lend (a bare `CHECK` probe, a DDL-time evaluation) — loud there,
        // because 18.6's own NULL answer for an unresolvable OID would otherwise be
        // indistinguishable from "this engine could not look".
        StringFunc::PgGetConstraintdef => {
            let schema = catalog_or_err(schema, "pg_get_constraintdef")?;
            let oid = int_arg(&args[0], "pg_get_constraintdef")?;
            let oid = i32::try_from(oid).unwrap_or(0);
            // 18.6 answers **NULL** for an OID no constraint holds (measured), not an
            // error, so an unresolvable OID is NULL here too.
            Ok(crate::catalog::constraint_def_by_oid(schema, oid).map_or(Value::Null, Value::Text))
        }
        StringFunc::Regclass => {
            let schema = catalog_or_err(schema, "regclass")?;
            eval_regclass(schema, &args[0])
        }
        StringFunc::IsFinite => match &args[0] {
            Value::Date(d) => Ok(Value::Bool(d.finite().is_some())),
            Value::Timestamp(ts) => Ok(Value::Bool(ts.finite().is_some())),
            _ => Err(Error::UndefinedFunction(
                "function isfinite does not exist for this argument type".into(),
            )),
        },
        StringFunc::ColDescription => eval_col_description(args),
        StringFunc::QuoteIdent => match &args[0] {
            Value::Text(name) => Ok(Value::Text(crate::lexer::pg_quote::quote_identifier(name))),
            _ => Err(no_such_overload("quote_ident", args)),
        },
        StringFunc::PgGetSerialSequence => {
            let schema = catalog_or_err(schema, "pg_get_serial_sequence")?;
            eval_pg_get_serial_sequence(schema, args)
        }
        StringFunc::HasSchemaPrivilege => eval_has_schema_privilege(args),
        StringFunc::ToRegclass => {
            let schema = catalog_or_err(schema, "to_regclass")?;
            eval_to_regclass(schema, args)
        }
        // Reached only through an argument list the parser never builds — the node's
        // own arm in `eval_expr_full` answers every one it does — and answered by the
        // same rule over the evaluated labels.
        StringFunc::EnumIn => match args {
            [value, Value::Text(name), labels @ ..] => eval_enum_in(value.clone(), name, |l| {
                labels.iter().any(|m| matches!(m, Value::Text(m) if m == l))
            }),
            _ => Err(Error::NotImplemented(
                "a cast to an enum with no enum named".into(),
            )),
        },
        StringFunc::Md5 => match &args[0] {
            Value::Text(s) => Ok(Value::Text(super::super::md5::md5_hex(s.as_bytes()))),
            Value::Bytea(b) => Ok(Value::Text(super::super::md5::md5_hex(b))),
            _ => Err(no_such_overload("md5", args)),
        },
        // Reached here, a non-NULL call is not a top-level item of the statement's own
        // projection, so no host took the writer for it and nothing here holds the
        // session's locks: answering would report a lock nothing took. The one place it
        // answers is `crate::executor::advisory`, from the projection path.
        StringFunc::AdvisoryXactLock
        | StringFunc::AdvisoryXactLockShared
        | StringFunc::AdvisoryLock
        | StringFunc::AdvisoryUnlock
        | StringFunc::AdvisoryUnlockAll => {
            let Some(func) = crate::planner::AdvisoryFunc::of(func) else {
                unreachable!("an advisory member")
            };
            crate::executor::advisory::advisory_key(func, args)?;
            Err(crate::executor::advisory::off_projection(func))
        }
        StringFunc::SetConfigLockTimeout => eval_set_config_lock_timeout(args),
        // The node's own arm in `eval_expr_full` answers every call it builds, with the
        // argument's declared type; reached here, only the value's own type is known.
        StringFunc::MakeInterval => eval_make_interval(args),
        StringFunc::Decode => Ok(Value::Bytea(crate::executor::coerce::decode_bytes(
            str_arg(&args[0], "decode")?,
            str_arg(&args[1], "decode")?,
        )?)),
        StringFunc::ToJsonb => Ok(Value::Jsonb(
            super::eval::value_to_jsonb(&args[0], None).canonical(),
        )),
    }
}

/// `make_interval(years, months, weeks, days, hours, mins, secs)` over its non-NULL
/// arguments, each absent one zero — 18.6's `make_interval` (`timestamp.c`), field by
/// field: an `integer` (or a `smallint`, which widens to it) in the first six places and
/// any number in `secs`, which the cast converts to `double precision`; anything else is `42883`,
/// as no overload matches. A field overflow, an infinite or `NaN` `secs` and an infinite
/// result are `22008`; `secs` overflowing on its way to microseconds is `float8_mul`'s
/// `22003`.
fn eval_make_interval(args: &[Value]) -> Result<Value> {
    let out_of_range = || Error::TemporalOutOfRange("interval");
    let mut fields = [0_i32; 6];
    for (i, field) in fields.iter_mut().enumerate() {
        *field = match args.get(i) {
            None => 0,
            Some(Value::Int4(n)) => *n,
            Some(Value::Int2(n)) => i32::from(*n),
            Some(_) => return Err(no_such_overload("make_interval", args)),
        };
    }
    // Any number converts to `double precision` through the cast itself, which a
    // `numeric` too small or too large for one is `22003` at (`numeric_float8`).
    let secs = match args.get(6) {
        None => 0.0,
        Some(
            v @ (Value::Float8(_)
            | Value::Float4(_)
            | Value::Int2(_)
            | Value::Int4(_)
            | Value::Int8(_)
            | Value::Numeric(_)),
        ) => match coerce_value(v.clone(), TypeOid::Float8)? {
            Value::Float8(f) => f,
            _ => return Err(no_such_overload("make_interval", args)),
        },
        Some(_) => return Err(no_such_overload("make_interval", args)),
    };
    let [years, months, weeks, days, hours, mins] = fields;
    if secs.is_nan() || secs.is_infinite() {
        return Err(out_of_range());
    }
    let months = years
        .checked_mul(12)
        .and_then(|m| m.checked_add(months))
        .ok_or_else(out_of_range)?;
    let days = weeks
        .checked_mul(7)
        .and_then(|d| d.checked_add(days))
        .ok_or_else(out_of_range)?;
    // An `int4` times either factor stays well inside an `i64`.
    let time = i64::from(hours) * 3_600_000_000 + i64::from(mins) * 60_000_000;
    let micros = secs * 1_000_000.0;
    if micros.is_infinite() {
        return Err(Error::NumericOutOfRange("double precision"));
    }
    let micros = micros.round_ties_even();
    // `FLOAT8_FITS_IN_INT64`: at least `i64::MIN` and below `2^63`.
    #[expect(
        clippy::cast_precision_loss,
        reason = "the bounds are exact powers of two, as `FLOAT8_FITS_IN_INT64`'s are"
    )]
    let fits = micros >= i64::MIN as f64 && micros < -(i64::MIN as f64);
    if !fits {
        return Err(out_of_range());
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "`fits` bounds the value to `i64` and it is already integral"
    )]
    let time = time.checked_add(micros as i64).ok_or_else(out_of_range)?;
    let interval = PgInterval::new(months, days, time);
    let infinite = (months == i32::MAX && days == i32::MAX && time == i64::MAX)
        || (months == i32::MIN && days == i32::MIN && time == i64::MIN);
    if infinite {
        return Err(out_of_range());
    }
    Ok(Value::Interval(interval))
}

/// `set_config('lock_timeout', value, is_local)` over its evaluated arguments: the
/// assignment, through the ambient session
/// ([`crate::catalog::SessionSettings::set_config_lock_timeout`]), and its answer.
///
/// A NULL `value` resets the setting and a NULL `is_local` is `false`, as on 18.6.
///
/// # Errors
///
/// `22023` for a value `lock_timeout` refuses; `42883` for a value or an `is_local` of a
/// type no `set_config` takes.
fn eval_set_config_lock_timeout(args: &[Value]) -> Result<Value> {
    let [_, value, is_local] = args else {
        return Err(no_such_overload("set_config", args));
    };
    let value = match value {
        Value::Null => None,
        Value::Text(v) => Some(v.as_str()),
        _ => return Err(no_such_overload("set_config", args)),
    };
    let is_local = match is_local {
        Value::Null => false,
        Value::Bool(b) => *b,
        _ => return Err(no_such_overload("set_config", args)),
    };
    crate::catalog::SessionSettings::set_config_lock_timeout(value, is_local).map(Value::Text)
}

/// `<value>::<enum>` — 18.6's conversion to an enum, over a value already evaluated.
///
/// A string is tested for membership by `holds` (exact bytes: a label is never folded)
/// and passes through as the label it is, or is
/// `22P02 invalid input value for enum <name>: "<label>"`. NULL is NULL. A value of any
/// other type is `42846 cannot cast type <T> to <name>`: 18.6 converts to an enum only
/// through the text of a string-category source (`find_coercion_pathway`'s I/O branch),
/// and raises this in parse analysis where pglet raises it here, on the value — so over
/// no row it is not raised at all, and an enum value is a `Value::Text` this cannot tell
/// from a `text` one (`EDGES.md` row 104's erasure), so a second enum's label is tested
/// for membership rather than refused.
pub(super) fn eval_enum_in(
    value: Value,
    name: &str,
    holds: impl Fn(&str) -> bool,
) -> Result<Value> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::Text(label) if holds(&label) => Ok(Value::Text(label)),
        Value::Text(label) => Err(Error::InvalidEnumValue {
            enum_type: name.to_string(),
            value: label,
        }),
        other => Err(Error::TypeCoercion {
            from: opt_type_oid_display_name(other.type_oid()).to_string(),
            to: name.to_string(),
        }),
    }
}

/// The `function <name>(<arg types>) does not exist` refusal 18.6's `ParseFuncOrColumn`
/// raises when no overload takes the arguments (`42883`), spelled with each argument's
/// display type name.
pub fn no_such_overload(func: &str, args: &[Value]) -> Error {
    let types: Vec<&str> = args
        .iter()
        .map(|v| opt_type_oid_display_name(v.type_oid()))
        .collect();
    Error::UndefinedFunction(format!(
        "function {func}({}) does not exist",
        types.join(", ")
    ))
}

/// `col_description(oid, int4)`, over non-NULL arguments: the overload check 18.6 makes
/// before the call, the `oid` conversion of the first argument, then NULL — no column
/// carries a description here ([`StringFunc::ColDescription`]).
///
/// The first argument reaches `oid` from an `int2` / `int4` through 18.6's implicit casts
/// (`int4 → oid` is binary-coercible, so a negative one wraps rather than failing) and from
/// an `int8` through `i8tooid`, which refuses a value outside `0 ..= 4294967295`
/// (`22003 OID out of range`). The second is an `int4` or an `int2`; an `int8` there is
/// `42883`, `int8 → int4` being an assignment cast. The overload is decided before the
/// conversion runs, as on 18.6 (measured: `col_description(4294967296, true)` is `42883`,
/// `col_description(4294967296, 1)` is `22003`).
///
/// A `text` argument is refused. 18.6 takes an **unknown** literal (`'1'` resolves, `'abc'`
/// is `22P02 invalid input syntax for type oid`) and refuses a `text` one (`42883`); pglet
/// types a string literal `text` (`EDGES.md` row 54), so the literal is refused too —
/// pglet-stricter, and loud. The checks are made on the values, as this family's are, so a
/// NULL argument answers NULL before them, and a call over no rows answers nothing, where
/// 18.6 refuses a mistyped call at parse analysis (`EDGES.md` row 252).
///
/// # Errors
///
/// `42883` for an argument no overload takes, `22003` for an `int8` outside the `oid`
/// range.
fn eval_col_description(args: &[Value]) -> Result<Value> {
    let first = matches!(args[0], Value::Int2(_) | Value::Int4(_) | Value::Int8(_));
    let second = matches!(args[1], Value::Int2(_) | Value::Int4(_));
    if !(first && second) {
        return Err(no_such_overload("col_description", args));
    }
    if let Value::Int8(n) = args[0]
        && u32::try_from(n).is_err()
    {
        return Err(Error::NumericOutOfRange("OID"));
    }
    Ok(Value::Null)
}

/// The schemas pglet answers for: `public`, the one user schema, and the two whose
/// relations it serves (`pg_catalog`, `information_schema`). 18.6 has `pg_toast` too, and a
/// `pg_temp_N` per session that has made a temporary object; pglet has neither
/// (`EDGES.md` row 252).
const KNOWN_SCHEMAS: [&str; 3] = ["public", "pg_catalog", "information_schema"];

/// `pg_get_serial_sequence(table, column)` over non-NULL arguments: the sequence that
/// column owns, rendered `public.<name>` by 18.6's `quote_identifier`, or NULL.
///
/// The table is read as 18.6's `textToQualifiedNameList` reads it — the same grammar as a
/// `nextval` name literal (`crate::parser::split_name_literal`: whitespace around a part
/// skipped, an unquoted part downcased, a quoted one verbatim, `42602` for a malformed
/// one) — and resolved as `makeRangeVarFromNameList` + `RangeVarGetRelid` do: a third
/// part must be the current database (`0A000 cross-database references are not
/// implemented` otherwise), a fourth is `42601`, a schema pglet does not have is
/// `3F000`, and a relation it does not have is `42P01`. The **column** is taken verbatim
/// — not folded, not trimmed — as `get_attnum` takes it, and a column the relation does
/// not have is `42703`; the heap's system column names count as columns. All measured on
/// 18.6, in that order.
///
/// Refused `0A000` where pglet cannot answer as 18.6 does: a relation in `pg_catalog` or
/// `information_schema` (a bare name that is one of pglet's catalog relations included,
/// since 18.6's search path puts `pg_catalog` first), and an index.
///
/// # Errors
///
/// As above, and `42883` for a non-text argument.
fn eval_pg_get_serial_sequence(schema: &Schema, args: &[Value]) -> Result<Value> {
    let (Value::Text(table_arg), Value::Text(column)) = (&args[0], &args[1]) else {
        return Err(no_such_overload("pg_get_serial_sequence", args));
    };
    let parts = crate::parser::split_name_literal(table_arg)?;
    let catalog_relation = || {
        Error::NotImplemented(format!(
            "pg_get_serial_sequence over the catalog relation {table_arg:?} is not supported"
        ))
    };
    let qualified = |space: &str, name: &'_ String| -> Result<String> {
        match space {
            "public" => Ok(name.clone()),
            "pg_catalog" | "information_schema" => Err(catalog_relation()),
            other => Err(Error::SchemaNotFound(other.to_string())),
        }
    };
    let name = match parts.as_slice() {
        [name] => name.clone(),
        [space, name] => qualified(space, name)?,
        [db, space, name] if db == crate::catalog::SessionSettings::ambient().database() => {
            qualified(space, name)?
        }
        [_, _, _] => {
            return Err(Error::NotImplemented(format!(
                "cross-database references are not implemented: \"{}\"",
                parts.join(".")
            )));
        }
        _ => {
            return Err(Error::syntax(format!(
                "improper relation name (too many dotted names): {}",
                parts.join(".")
            )));
        }
    };
    if parts.len() == 1 && crate::catalog::CatalogTable::from_name(&name).is_some() {
        return Err(catalog_relation());
    }
    let has_column = match schema.relation_kind(&name) {
        Some(crate::RelationKind::Table) => {
            const SYSTEM_COLUMNS: [&str; 6] = ["ctid", "xmin", "cmin", "xmax", "cmax", "tableoid"];
            schema
                .get_table(&name)
                .is_some_and(|t| t.columns.iter().any(|c| &c.name == column))
                || SYSTEM_COLUMNS.contains(&column.as_str())
        }
        Some(crate::RelationKind::Sequence) => {
            ["last_value", "log_cnt", "is_called"].contains(&column.as_str())
        }
        Some(crate::RelationKind::Index) => {
            return Err(Error::NotImplemented(format!(
                "pg_get_serial_sequence over the index {name:?} is not supported"
            )));
        }
        None => {
            return Err(CatalogError::TableNotFound {
                table: parts.join("."),
            }
            .into());
        }
    };
    if !has_column {
        return Err(CatalogError::ColumnNotFound {
            table: name,
            column: column.clone(),
        }
        .into());
    }
    let mut owned = schema.sequences.iter().filter(|(_, seq)| {
        seq.owner
            .as_ref()
            .is_some_and(|o| o.table == name && &o.column == column)
    });
    let first = owned.next();
    // 18.6 answers the first AUTO / INTERNAL `pg_depend` entry it scans, which is creation
    // order in every case measured (two `OWNED BY` sequences answer the one created first; a
    // serial's or an identity's own sequence beats one attached later). pglet keeps no
    // creation order for ownership — `Schema.sequences` is keyed by name — so a column owning
    // more than one sequence is refused rather than answered by name order (`EDGES.md` row 252).
    if owned.next().is_some() {
        return Err(Error::NotImplemented(format!(
            "pg_get_serial_sequence over column \"{column}\" of relation \"{name}\", which owns more than one sequence"
        )));
    }
    Ok(first.map_or(Value::Null, |(seq_name, _)| {
        Value::Text(format!(
            "public.{}",
            crate::lexer::pg_quote::quote_identifier(seq_name)
        ))
    }))
}

/// `has_schema_privilege(schema, privilege)` over non-NULL arguments.
///
/// 18.6's order: the schema first, taken verbatim (`'PUBLIC'` and `' public'` are
/// `3F000`), then the privilege string as `convert_any_priv_string` parses it — split on
/// `,`, each item trimmed of C `isspace` characters, compared case-insensitively against
/// `CREATE`, `USAGE` and each `WITH GRANT OPTION` form spelled with single spaces, an
/// item that is none of them (the empty one included) `22023 unrecognized privilege
/// type`. pglet's one role holds every privilege, so a string that parses answers
/// `true`. An integer first argument is 18.6's `oid` overload, refused `0A000`.
///
/// # Errors
///
/// `3F000`, `22023`, `0A000` as above, and `42883` for another argument type.
fn eval_has_schema_privilege(args: &[Value]) -> Result<Value> {
    let (space, privilege) = match (&args[0], &args[1]) {
        (Value::Text(space), Value::Text(privilege)) => (space, privilege),
        (Value::Int2(_) | Value::Int4(_) | Value::Int8(_), Value::Text(_)) => {
            return Err(Error::NotImplemented(
                "has_schema_privilege(oid, text) is not supported".into(),
            ));
        }
        _ => return Err(no_such_overload("has_schema_privilege", args)),
    };
    if !KNOWN_SCHEMAS.contains(&space.as_str()) {
        return Err(Error::SchemaNotFound(space.clone()));
    }
    for item in privilege.split(',') {
        let item =
            item.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c'));
        let known = [
            "CREATE",
            "CREATE WITH GRANT OPTION",
            "USAGE",
            "USAGE WITH GRANT OPTION",
        ]
        .iter()
        .any(|p| p.eq_ignore_ascii_case(item));
        if !known {
            return Err(Error::InvalidParameterValue {
                message: format!("unrecognized privilege type: \"{item}\""),
                hint: None,
            });
        }
    }
    Ok(Value::Bool(true))
}

/// The catalog an env lends, or `0A000` naming the expression that needed it.
fn catalog_or_err<'a>(schema: Option<&'a Schema>, what: &str) -> Result<&'a Schema> {
    schema.ok_or_else(|| Error::NotImplemented(format!("{what} is not available in this context")))
}

/// `<value>::regclass` — resolve a relation reference to its **name**, as `text`.
///
/// Two input shapes: an **OID** resolves to the relation's name
/// and, for an OID no relation holds, renders as the OID's own digits (`SELECT
/// 99999::regclass` answers `99999` there, not an error); a **name** resolves against
/// the catalog and raises `42P01` when nothing carries it.
///
/// **The answer is the relation's name rendered for an identifier position, not its
/// stored bytes.** 18.6's `regclassout` (`utils/adt/regproc.c`) ends in
/// `quote_qualified_identifier`, so a relation created as `"Odd"` prints as `"Odd"` —
/// quotes included — and the value re-parses to the relation it names. Comparing the
/// stored bytes answered a bare `Odd`, which is the *other* relation's spelling: 18.6
/// folds an unquoted `Odd` to `odd`, so feeding pglet's answer back to
/// `'…'::regclass` resolved to nothing (or, given both spellings, to the wrong one).
/// [`crate::lexer::quote_ident`] is the predicate — the one pg_dump-style `fmtId` this
/// engine has — and it decides
/// keyword-ness by asking pglet's own lexer where 18.6 asks whether the keyword's
/// category is `UNRESERVED_KEYWORD`; the two sets are not the same, and `EDGES.md`
/// row 85 records the measured members. The numeric fallback is **not** quoted, because
/// digits are not a bare identifier and `quote_ident` would wrap them: `SELECT
/// 99999::regclass::text` is `99999` on both engines. It is not the whole of 18.6's
/// unresolvable-OID rendering — `regclassout` short-circuits `InvalidOid` to `-` before
/// the catalog lookup, so `SELECT 0::regclass::text` is `-` there against `0` here
/// (measured; `EDGES.md` row 85's (f)).
///
/// **A name is a relation of any kind pglet holds** ([`Schema::relation_kind`]) — a
/// table, an index (`parent_pkey::regclass` names the index) or a sequence (`'s1'::regclass`
/// answers `s1`), as `to_regclass` finds it. **An OID is a table's or an index's only**:
/// a sequence carries no OID here, so `<its oid>::regclass` has nothing to find
/// (`EDGES.md` row 85's (e)).
fn eval_regclass(schema: &Schema, value: &Value) -> Result<Value> {
    match value {
        Value::Int2(_) | Value::Int4(_) | Value::Int8(_) => {
            let oid = i32::try_from(int_arg(value, "regclass")?).unwrap_or(0);
            let name = schema
                .tables
                .values()
                .find(|t| t.oid == oid)
                .map(|t| t.name.clone())
                .or_else(|| {
                    schema
                        .indexes
                        .values()
                        .find(|i| i.oid == oid)
                        .map(|i| i.name.clone())
                });
            Ok(Value::Text(name.as_deref().map_or_else(
                || oid.to_string(),
                crate::lexer::quote_ident,
            )))
        }
        Value::Text(name) => {
            // A **string literal is not an identifier**, so nothing has folded it: the
            // lexer folds identifiers and leaves a literal's bytes alone. 18.6 reads the
            // string through `stringToQualifiedNameList`, which is why
            // `'PARENT'::regclass`, `'"parent"'::regclass` and `'  parent  '::regclass`
            // all name `parent` there — and why comparing the raw bytes answered `42P01`
            // for the first and third. `parser::reduce_name_literal` **is** that reader,
            // already written for `nextval('…')`: the name-literal half of this engine's
            // one identifier fold, with its three recorded divergences. A second copy
            // here would be a second fold.
            let bare = crate::parser::reduce_name_literal(name)?;
            if schema.relation_kind(&bare).is_some() {
                Ok(Value::Text(crate::lexer::quote_ident(&bare)))
            } else {
                Err(CatalogError::TableNotFound { table: bare }.into())
            }
        }
        other => Err(Error::NotImplemented(format!(
            "cannot cast {} to regclass",
            opt_type_oid_display_name(other.type_oid())
        ))),
    }
}

/// `to_regclass(name)` over a non-NULL argument: the relation the name resolves to,
/// rendered as [`eval_regclass`] renders it, or NULL.
///
/// The name is read as 18.6's `regclassin` reads it (`stringToQualifiedNameList`, then
/// `makeRangeVarFromNameList` and `RangeVarGetRelid`), with its **soft** errors answering
/// NULL as `to_regclass` makes them: a malformed name (`''`, an unterminated `"`, two
/// words), a schema that does not exist, and a relation the namespace does not hold. Its
/// hard errors stay errors, as they do there: a fourth dotted part is `42601`, and a
/// third that is not the current database is `0A000`. All measured on 18.7.
///
/// Tables, indexes and sequences are relations ([`Schema::relation_kind`]); a name pglet
/// serves as a catalog relation (`pg_class`) answers too, bare or under `pg_catalog`,
/// which is how 18.6 renders it with `pg_catalog` on the search path.
///
/// Refused `0A000` where the answer would be a guess: a bare name beginning `pg_` that
/// neither the catalog pglet serves nor a user relation holds (18.6's search path reads
/// `pg_catalog` first, and its relations are more than pglet's), a qualifier naming a
/// schema of 18.6's pglet does not model (`pg_catalog` beyond pglet's relations,
/// `pg_toast`, `information_schema`; a `pg_temp` qualifier answers NULL, since pglet
/// creates no temporary relation), and an all-digit or `-` argument, which
/// `regclassin` reads as an OID.
///
/// # Errors
///
/// As above, and `42883` for a non-text argument.
fn eval_to_regclass(schema: &Schema, args: &[Value]) -> Result<Value> {
    let Value::Text(literal) = &args[0] else {
        return Err(no_such_overload("to_regclass", args));
    };
    let refused = |what: &str| {
        Error::NotImplemented(format!(
            "to_regclass over {what} {literal:?} is not supported"
        ))
    };
    // `parseDashOrOid`'s test, on the argument as written: no whitespace is skipped.
    if literal == "-" || (!literal.is_empty() && literal.bytes().all(|b| b.is_ascii_digit())) {
        return Err(refused("an OID"));
    }
    let Ok(parts) = crate::parser::split_name_literal(literal) else {
        return Ok(Value::Null);
    };
    let found = |name: &str| Value::Text(crate::lexer::quote_ident(name));
    let qualified = |space: &str, name: &str| -> Result<Value> {
        match space {
            "public" => Ok(if schema.relation_kind(name).is_some() {
                found(name)
            } else {
                Value::Null
            }),
            "pg_catalog" if crate::catalog::CatalogTable::from_name(name).is_some() => {
                Ok(found(name))
            }
            "information_schema" => Err(refused("a relation in information_schema")),
            // A session's temporary schema: pglet creates no temporary relation.
            space if space.starts_with("pg_temp") => Ok(Value::Null),
            space if space.starts_with("pg_") => Err(refused("a relation in a system schema")),
            _ => Ok(Value::Null),
        }
    };
    match parts.as_slice() {
        [name] => {
            if crate::catalog::CatalogTable::from_name(name).is_some()
                || schema.relation_kind(name).is_some()
            {
                Ok(found(name))
            } else if name.starts_with("pg_") {
                Err(refused("a name pg_catalog may hold"))
            } else {
                Ok(Value::Null)
            }
        }
        [space, name] => qualified(space, name),
        [db, space, name] if db == crate::catalog::SessionSettings::ambient().database() => {
            qualified(space, name)
        }
        [_, _, _] => Err(Error::NotImplemented(format!(
            "cross-database references are not implemented: \"{}\"",
            parts.join(".")
        ))),
        _ => Err(Error::syntax(format!(
            "improper relation name (too many dotted names): {}",
            parts.join(".")
        ))),
    }
}

/// Require a boolean argument, failing loud with `42883` (Postgres's function
/// resolution) on anything else. NULL is screened by the hoisted check.
///
/// **Stricter than 18.6 for an unknown-typed literal**: `current_schemas('t')` and
/// `current_setting('search_path', 't')` resolve there (an `unknown` literal takes
/// the signature's `boolean`) and are refused here, because a bare string literal is
/// `text` to this engine rather than `unknown` — the same literal-as-text policy
/// `EDGES.md` row 54 records. The `true`/`false` spellings, which is what anything
/// generated writes, agree.
fn bool_arg(v: &Value, func: &str) -> Result<bool> {
    match v {
        Value::Bool(b) => Ok(*b),
        other => Err(Error::UndefinedFunction(format!(
            "function {func} does not exist for argument type {}",
            opt_type_oid_display_name(other.type_oid())
        ))),
    }
}

/// Require a text argument for a string function, failing loud with `42883` (like
/// Postgres's function resolution) on a non-text value. NULL is screened earlier.
fn str_arg<'a>(v: &'a Value, func: &str) -> Result<&'a str> {
    match v {
        Value::Text(s) => Ok(s.as_str()),
        other => Err(Error::UndefinedFunction(format!(
            "function {func} does not exist for argument type {}",
            opt_type_oid_display_name(other.type_oid())
        ))),
    }
}

/// Argument `i` of a call to `func` whose every overload takes an `int4` there: an
/// `int2` or `int4` value, read as 18.6's implicit `int2 → int4` cast reads it. Anything
/// else — an `int8` included, `int8 → int4` being an assignment cast only — resolves no
/// overload (`42883`), named with every argument's type.
fn int4_arg(args: &[Value], i: usize, func: &str) -> Result<i32> {
    match args.get(i) {
        Some(Value::Int4(n)) => Ok(*n),
        Some(Value::Int2(n)) => Ok(i32::from(*n)),
        _ => Err(no_such_overload(func, args)),
    }
}

/// Require an integer argument (`int2`/`int4`/`int8`) for a string function,
/// failing loud with `42883` on a non-integer value.
fn int_arg(v: &Value, func: &str) -> Result<i64> {
    value_as_i64(v).ok_or_else(|| {
        Error::UndefinedFunction(format!(
            "function {func} does not exist for argument type {}",
            opt_type_oid_display_name(v.type_oid())
        ))
    })
}

/// An optional text argument (the trim set / pad fill), defaulting to a single
/// space when omitted. A present non-text value fails loud (`42883`).
fn text_or_space<'a>(arg: Option<&'a Value>, func: &str) -> Result<&'a str> {
    arg.map_or(Ok(" "), |v| str_arg(v, func))
}

/// `length(x)` — the character count of a text value, or the **byte** count of a
/// `bytea` value (matching Postgres, which has both `length(text)` and
/// `length(bytea)`). Any other type resolves to no function (`42883`).
fn string_length(v: &Value) -> Result<Value> {
    let n = match v {
        Value::Text(s) => s.chars().count(),
        Value::Bytea(b) => b.len(),
        other => {
            return Err(Error::UndefinedFunction(format!(
                "function length does not exist for argument type {}",
                opt_type_oid_display_name(other.type_oid())
            )));
        }
    };
    i32::try_from(n)
        .map(Value::Int4)
        .map_err(|_| Error::NumericOutOfRange("integer"))
}

/// `replace(str, from, to)` — every non-overlapping `from` occurrence becomes `to`.
/// An empty `from` (or empty `str`) leaves `str` unchanged (Postgres); Rust's
/// `str::replace` would otherwise splice `to` between every character.
fn string_replace(s: &str, from: &str, to: &str) -> String {
    if from.is_empty() || s.is_empty() {
        return s.to_string();
    }
    s.replace(from, to)
}

/// `substring(str, start [, length])` — 1-based, character-based, with Postgres's
/// `text_substring` clip math. A negative explicit `length` fails loud (`22011`).
fn string_substring(s: &str, start: i64, length: Option<i64>) -> Result<Value> {
    let chars: Vec<char> = s.chars().collect();
    let (begin, end) = substring_range(chars.len(), start, length)?;
    Ok(Value::Text(chars[begin..end].iter().collect()))
}

/// `substring(bytes, start [, length])` / `substr(…)` over a `bytea` — 18.6's
/// `bytea_substring`, whose clip math is `text_substring`'s counted in bytes.
fn bytea_substring(b: &[u8], start: i64, length: Option<i64>) -> Result<Value> {
    let (begin, end) = substring_range(b.len(), start, length)?;
    Ok(Value::Bytea(b[begin..end].to_vec()))
}

/// The 0-based `[begin, end)` slice `substring(x, start [, length])` takes from a value
/// `n` units long: `S1 = max(start, 1)`, and the end `start + length` from the
/// **unclamped** start — so a zero or negative end is the empty slice and an
/// overflowing one runs to the end. A negative explicit `length` is `22011`.
fn substring_range(n: usize, start: i64, length: Option<i64>) -> Result<(usize, usize)> {
    let n = i64::try_from(n).unwrap_or(i64::MAX);
    let end0 = match length {
        None => n,
        Some(len) if len < 0 => return Err(Error::NegativeSubstringLength),
        // 1-based exclusive `start + length` is the 0-based exclusive end; a zero or
        // negative end (`start` ≤ 0) is the empty slice.
        Some(len) => match start.checked_add(len) {
            Some(e) if e < 1 => return Ok((0, 0)),
            Some(e) => e - 1,
            None => n,
        },
    };
    let begin = (start.max(1) - 1).clamp(0, n);
    let end0 = end0.clamp(begin, n);
    // both clamped into `0..=n`, which came from a `usize`
    Ok((
        usize::try_from(begin).unwrap_or(0),
        usize::try_from(end0).unwrap_or(0),
    ))
}

/// `left(str, n)` — the first `n` characters, or for a negative `n` all but the last
/// `|n|` (18.6's `text_left`).
fn string_left(s: &str, n: i32) -> Value {
    let count = s.chars().count();
    let take = if n >= 0 {
        usize::try_from(n).unwrap_or(usize::MAX)
    } else {
        count.saturating_sub(usize::try_from(n.unsigned_abs()).unwrap_or(usize::MAX))
    };
    Value::Text(s.chars().take(take).collect())
}

/// `right(str, n)` — the last `n` characters, or for a negative `n` all but the first
/// `|n|` (18.7's `text_right`).
///
/// `n = -2147483648` answers the empty string: its magnitude exceeds every string's
/// length, as 18.7 answers it (PostgreSQL commit `a63df266` fixed the negation that had
/// overflowed back to a negative clip length and answered the whole string).
fn string_right(s: &str, n: i32) -> Value {
    let count = s.chars().count();
    let skip = if n < 0 {
        usize::try_from(n.unsigned_abs()).unwrap_or(usize::MAX)
    } else {
        count.saturating_sub(usize::try_from(n).unwrap_or(usize::MAX))
    };
    Value::Text(s.chars().skip(skip).collect())
}

/// `ltrim`/`rtrim`/`btrim` — trim characters that are members of the `set` from the
/// leading (`left`) and/or trailing (`right`) ends. An empty `str` or `set` leaves
/// `str` unchanged (Postgres's `dotrim`). `set` is a character **set**, not a
/// substring; membership and iteration are character-based.
fn string_trim(s: &str, set: &str, left: bool, right: bool) -> String {
    if s.is_empty() || set.is_empty() {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let in_set = |c: char| set.chars().any(|x| x == c);
    let mut begin = 0usize;
    let mut end = chars.len();
    if left {
        while begin < end && in_set(chars[begin]) {
            begin += 1;
        }
    }
    if right {
        while end > begin && in_set(chars[end - 1]) {
            end -= 1;
        }
    }
    chars[begin..end].iter().collect()
}

/// `strpos(haystack, needle)` — the 1-based **character** position of the first
/// occurrence of `needle`, 0 if absent, 1 for an empty `needle` (Postgres). Backs
/// `POSITION(needle IN haystack)`.
fn string_strpos(haystack: &str, needle: &str) -> i64 {
    if needle.is_empty() {
        return 1;
    }
    haystack.find(needle).map_or(0, |byte_off| {
        haystack[..byte_off].chars().count() as i64 + 1
    })
}

/// The largest `lpad`/`rpad` target length pglet will build. Postgres caps at
/// ~1 GB of output; pglet uses a far smaller character cap so a hostile
/// `lpad('x', 2000000000)` fails loud (`54000`) instead of attempting a
/// multi-gigabyte allocation. Well beyond any realistic padded string.
const MAX_PAD_LEN: usize = 1_000_000;

/// `lpad`/`rpad` — pad or truncate `str` to `len` characters using `fill` (cycled).
/// A negative `len` is treated as 0; a `str` longer than `len` is truncated to its
/// leftmost `len` characters (both variants); an **empty** `fill` disables padding
/// (Postgres returns the shorter string unchanged). `left` pads on the left (lpad),
/// else on the right (rpad). A `len` past [`MAX_PAD_LEN`] fails loud (`54000`).
fn string_pad(s: &str, len: i64, fill: &str, left: bool) -> Result<String> {
    let target = usize::try_from(len.max(0)).unwrap_or(0);
    if target > MAX_PAD_LEN {
        return Err(Error::ProgramLimitExceeded(
            "requested length too large".into(),
        ));
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() >= target {
        // Truncate to the leftmost `target` characters (unchanged when equal).
        return Ok(chars[..target].iter().collect());
    }
    let fill_chars: Vec<char> = fill.chars().collect();
    if fill_chars.is_empty() {
        return Ok(s.to_string()); // nothing to pad with
    }
    let padding: String = (0..target - chars.len())
        .map(|i| fill_chars[i % fill_chars.len()])
        .collect();
    Ok(if left {
        format!("{padding}{s}")
    } else {
        format!("{s}{padding}")
    })
}

/// `split_part(str, delim, n)` — the 1-based `n`-th field of `str` split on the
/// literal `delim`. `n = 0` fails loud (`22023`); a negative `n` counts from the end
/// (`-1` = last); an out-of-range `n` yields `''`; an empty `str` yields `''`; an
/// empty `delim` treats the whole string as field 1 / -1 (else `''`).
fn string_split_part(s: &str, delim: &str, n: i64) -> Result<Value> {
    if n == 0 {
        return Err(Error::FieldPositionZero);
    }
    if s.is_empty() {
        return Ok(Value::Text(String::new()));
    }
    if delim.is_empty() {
        let field = if n == 1 || n == -1 { s } else { "" };
        return Ok(Value::Text(field.to_string()));
    }
    let parts: Vec<&str> = s.split(delim).collect();
    let count = parts.len() as i64;
    // n<0 counts from the end (-1 → last = count-1); n>0 is 1-based (1 → 0).
    let idx = if n < 0 { count + n } else { n - 1 };
    let field = if idx >= 0 && idx < count {
        parts[idx as usize]
    } else {
        ""
    };
    Ok(Value::Text(field.to_string()))
}

/// Evaluate a non-strict variadic function ([`VariadicFunc`]) — each applies its own
/// NULL rule rather than propagating NULL.
///
/// `enum_order` is the ordered member list `GREATEST`/`LEAST` compare by, resolved by
/// the caller from the argument *expressions* (`None` for every other function, and
/// where no argument is enum-typed). `arg_types` is each argument's declared type where
/// the caller derived them, which `concat`/`concat_ws` render by.
pub(super) fn eval_variadic_fn(
    func: VariadicFunc,
    args: &[Value],
    enum_order: Option<&[String]>,
    arg_types: Option<&[Option<TypeOid>]>,
) -> Result<Value> {
    match func {
        // Skip NULL args entirely (not even a separator gap); all-NULL → "".
        VariadicFunc::Concat => Ok(Value::Text(concat_values(args, arg_types, None))),
        VariadicFunc::ConcatWs => {
            // The separator (arg0) is strict: NULL → NULL. It must be text — a
            // non-text separator resolves to no function (`42883`, like Postgres),
            // rather than being silently stringified. The value args stay `any`.
            let Some((sep, rest)) = args.split_first() else {
                return Ok(Value::Text(String::new()));
            };
            match sep {
                Value::Null => Ok(Value::Null),
                Value::Text(s) => Ok(Value::Text(concat_values(
                    rest,
                    arg_types.and_then(|t| t.get(1..)),
                    Some(s),
                ))),
                other => Err(Error::UndefinedFunction(format!(
                    "function concat_ws does not exist for separator type {}",
                    opt_type_oid_display_name(other.type_oid())
                ))),
            }
        }
        VariadicFunc::Greatest => greatest_least(args, true, enum_order),
        VariadicFunc::Least => greatest_least(args, false, enum_order),
        VariadicFunc::NullIf => eval_nullif(args),
    }
}

/// The common type of the argument values for `greatest`/`least`/`ARRAY[…]`, via the
/// **same** [`select_common_type`](crate::planner::select_common_type) the planner
/// folds — so eval cannot resolve a type the planner would not have.
///
/// `None` only when every value is NULL. An incompatible pair fails loud with
/// Postgres's own split: `42804` across type categories (`int` vs `text`), `42846`
/// within one where no conversion exists (`time` vs `timestamp`). `context` is the
/// construct name the message quotes (`GREATEST`, `ARRAY`, …).
///
/// After plan-time unification the values are already homogeneous, so this normally
/// just confirms their shared type — but it is the *only* check on a path that reaches
/// eval without the planner's pass, which is why it shares the planner's rule rather
/// than re-deriving one.
pub(super) fn values_common_type(vals: &[&Value], context: &str) -> Result<Option<TypeOid>> {
    // A `Value` carries no enum identity, so this fold is plain by construction — the
    // identity-bearing fold is the planner's (`EDGES.md` row 114).
    let types: Vec<Option<ResolvedType>> = vals
        .iter()
        .map(|v| v.type_oid().map(ResolvedType::plain))
        .collect();
    Ok(crate::planner::select_common_type(&types, context)?.map(|t| t.oid))
}

/// `greatest`/`least` — the largest (`want_max`) or smallest non-NULL argument.
///
/// NULL arguments are skipped; all-NULL → NULL. The non-null values are resolved to
/// a common type and each coerced to it before comparison, so the returned value's
/// type matches the declared result type ([`crate::planner::variadic_fn_result_type`]) — no wire
/// mismatch. Incompatible argument types fail loud (`42804`).
fn greatest_least(args: &[Value], want_max: bool, enum_order: Option<&[String]>) -> Result<Value> {
    let non_null: Vec<&Value> = args.iter().filter(|v| !v.is_null()).collect();
    let Some(target) = values_common_type(&non_null, if want_max { "GREATEST" } else { "LEAST" })?
    else {
        return Ok(Value::Null); // all-null
    };
    let mut best: Option<Value> = None;
    for v in non_null {
        let coerced = coerce_value(v.clone(), target)?;
        best = Some(match best {
            None => coerced,
            Some(cur) => match values_compare_ordered(&coerced, &cur, enum_order) {
                Some(o) if (want_max && o.is_gt()) || (!want_max && o.is_lt()) => coerced,
                // Equal, opposite, or (defensively) incomparable → keep the current.
                _ => cur,
            },
        });
    }
    Ok(best.unwrap_or(Value::Null))
}

/// `nullif(a, b)` — NULL when `a` and `b` (coerced to their comparison type) are
/// equal, else `a` at `nullif_result_type` of the two **values**; a NULL argument
/// returns `a` without comparing. That is the declared result type where the plan
/// read the same two types (so `nullif(5, 6.0)` returns a `numeric`), and can be
/// another where it typed only the first: beside a `$N` the wasm binding decodes as a
/// `double precision`, a `numeric` `a` is returned as a `double precision` under the
/// `numeric` declaration (`EDGES.md` row 288). A pair with no `=` fails loud
/// (`42883`).
fn eval_nullif(args: &[Value]) -> Result<Value> {
    let [a, b] = args else {
        // Arity is enforced at build time; be defensive.
        return Ok(args.first().cloned().unwrap_or(Value::Null));
    };
    if a.is_null() || b.is_null() {
        return Ok(a.clone());
    }
    let (ta, tb) = (a.type_oid(), b.type_oid());
    // `NULLIF` is an `=` comparison throughout, so it uses the **operator** rulebook
    // end to end — not the `select_common_type` one its `greatest`/`least` IR
    // neighbours use. Borrowing theirs rejected `nullif(interval_col, time_col)`
    // (`42804`, "cannot be matched") where Postgres compares the pair happily as two
    // intervals, because `time` and `interval` are in different type *categories* and
    // only the operator rules bridge them.
    comparison_legality(
        ta.map(ResolvedType::plain),
        tb.map(ResolvedType::plain),
        "=",
    )?;
    let equal =
        match comparison_operand_type(ta.unwrap_or(TypeOid::Text), tb.unwrap_or(TypeOid::Text)) {
            Some(cmp_type) => values_equal(
                &coerce_value(a.clone(), cmp_type)?,
                &coerce_value(b.clone(), cmp_type)?,
            ),
            // No shared operand type (the legality gate above lets a `None`-typed operand
            // through); compare the values as they stand.
            None => values_equal(a, b),
        };
    if equal == Some(true) {
        return Ok(Value::Null);
    }
    // The **result** is argument one at `NULLIF`'s own result type, which is not the
    // comparison's type: Postgres declares argument one's type as the `=` resolution
    // left it, so `NULLIF(int4_col, int8_col)` is `int4` (compared in `int8`) while
    // `NULLIF(5, 6.0)` is `numeric` (no `int4 = numeric`, so argument one is coerced
    // and the result widens with it). Returning the comparison-coerced value declared
    // one type and sent another — an 8-byte `int8` under `int4`.
    //
    // Recomputed here from the values rather than trusted from the plan-time cast
    // (`coerce_nullif_args`): the two agree where the plan read the types the values
    // have, and doing it in both places keeps eval self-consistent on any path that
    // reaches it without the coercion pass. The coercion is the identity when the
    // plan-time cast already applied. Where the plan typed only the first argument the
    // two can part (`EDGES.md` row 288).
    nullif_result_type(ta, tb)
        .map_or_else(|| Ok(a.clone()), |result| coerce_value(a.clone(), result))
}

/// The shared body of `concat`/`concat_ws`: render every **non-NULL** argument to
/// its Postgres text form, joined by `sep` (nothing for plain `concat`). A NULL
/// argument contributes nothing — not even a separator. All-NULL → `""`. `types` is
/// each argument's declared type where known, so a `timestamptz` or `timestamptz[]`
/// argument keeps its `+00` (`value_to_text_with_oid`).
fn concat_values(args: &[Value], types: Option<&[Option<TypeOid>]>, sep: Option<&str>) -> String {
    let mut out = String::new();
    let mut first = true;
    for (i, v) in args.iter().enumerate() {
        if v.is_null() {
            continue;
        }
        let text = types
            .and_then(|t| t.get(i).copied().flatten())
            .map_or_else(
                || crate::types::value_to_text(v),
                |o| value_to_text_with_oid(v, o),
            )
            .unwrap_or_default();
        if !first && let Some(s) = sep {
            out.push_str(s);
        }
        out.push_str(&text);
        first = false;
    }
    out
}
