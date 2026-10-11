//! `COPY ... TO STDOUT` / `COPY ... FROM STDIN` execution — text format only.
//!
//! Pure functions over the `Storage` trait and the `Schema` catalog: no I/O, no
//! wire framing. The byte stream is passed in (`FROM`) or returned (`TO`) as a
//! plain `Vec<u8>`/`&[u8]`, so the embed `Database::copy_in`/`copy_out` API and
//! (later) the server's CopyData path share one engine.
//!
//! # Text format
//!
//! Columns are separated by the delimiter byte (default tab `0x09`); each row is
//! terminated by a line feed `\n`. A SQL NULL renders as / is detected by the
//! `null_marker` token (default `\N`); an empty field is the empty string, not
//! NULL. Field bytes are backslash-escaped on output and unescaped on input via
//! [`copy_escape_field`] / [`copy_unescape_field`]. A lone `\.` line ends the
//! input stream.
//!
//! # Correct-or-loud
//!
//! `COPY FROM` ingests untrusted bulk input, so it **never panics**: a malformed
//! field count, a corrupt end-marker, input or a de-escaped field the server encoding
//! refuses, or a value that fails its column type's input function each return a
//! clean `Err` with the matching SQLSTATE (`22P04` framing, `22021` encoding, `22P02`
//! value, `23502` NOT NULL, …). The ingest runs
//! the **same** validation/constraint/FK/index path as `INSERT` (the shared
//! `constraints` helpers + `dml::resolve_default`), so a COPY-loaded row is as
//! checked as an inserted one.

use crate::catalog::{Schema, Table};
use crate::planner::{CopyFormat, CopyOptions};
use crate::types::{TypeOid, Value, value_to_text_with_oid};
use crate::{CatalogError, Error, Result};

use super::Storage;
use super::constraints::{
    coerce_column_value, enforce_row_constraints, prepare_checks, prepare_indexes, validate_column,
};
use super::dml::resolve_default;
use super::helpers::get_table_or_err;

/// The resolved fields of a `Plan::Copy`, borrowed for execution.
///
/// `columns` empty means "all columns, in catalog order". Direction is not carried
/// here — the caller picks [`execute_copy_to`] vs [`execute_copy_from`].
#[derive(Debug)]
pub struct CopyParams<'a> {
    /// The target/source table name.
    pub table: &'a str,
    /// The explicit column list, or empty for all columns in catalog order.
    pub columns: &'a [String],
    /// The on-wire format; only [`CopyFormat::Text`] is executed.
    pub format: CopyFormat,
    /// The field-separator byte.
    pub delimiter: u8,
    /// The NULL token (rendered for / detected on a NULL value).
    pub null_marker: &'a str,
    /// Whether a header line is emitted (`TO`) / skipped (`FROM`).
    pub header: bool,
}

/// Borrow a [`CopyOptions`] (owned, carried in `Plan::Copy` / the server's
/// `CopyInState`) as the engine's [`CopyParams`] — the single owned→borrowed view, so
/// the field set isn't re-spelled at every call site.
impl<'a> From<&'a CopyOptions> for CopyParams<'a> {
    fn from(o: &'a CopyOptions) -> Self {
        Self {
            table: &o.table,
            columns: &o.columns,
            format: o.format,
            delimiter: o.delimiter,
            null_marker: &o.null_marker,
            header: o.header,
        }
    }
}

/// Reject a non-text COPY format loud (`0A000`) — the single guard both directions
/// funnel through so `FORMAT csv`/`binary` never falls through to the text path.
fn require_text_format(format: CopyFormat) -> Result<()> {
    match format {
        CopyFormat::Text => Ok(()),
        CopyFormat::Csv => Err(Error::NotImplemented(
            "COPY FORMAT csv is not supported (text format only)".into(),
        )),
        CopyFormat::Binary => Err(Error::NotImplemented(
            "COPY FORMAT binary is not supported (text format only)".into(),
        )),
    }
}

/// Resolve the COPY column list to table-column positions (in transfer order).
///
/// An empty list selects every column in catalog order. A named column that does
/// not exist on the table fails loud (`42703`), and one the list already holds fails
/// loud too (`42701`) — a COPY column list is a set. Shared with the server's
/// CopyIn/OutResponse header sizing, so the column-resolution rules live in one
/// crate (it takes `.len()` of the result).
///
/// **Both directions**, because 18.6 resolves both through the one `CopyGetAttnums`
/// (`commands/copy.c`): `COPY t (a, a) TO STDOUT` is `42701` there just as the `FROM
/// STDIN` spelling is, where this emitted the column twice.
///
/// The two verdicts **interleave** per list item, as `CopyGetAttnums` interleaves them:
/// `COPY t (a, a, zz)` is `42701` and `COPY t (zz, a, a)` is `42703`, measured on 18.6 in
/// both directions. That ordering is this loop's, not
/// [`claim_target_column`](crate::planner::claim_target_column)'s, which carries none —
/// an `UPDATE` `SET` list reaches its duplicate check a whole phase after its names
/// resolve and so answers `42703` from either position.
pub fn resolve_columns(table: &str, table_def: &Table, columns: &[String]) -> Result<Vec<usize>> {
    if columns.is_empty() {
        return Ok((0..table_def.columns.len()).collect());
    }
    let mut selected = Vec::with_capacity(columns.len());
    for name in columns {
        let position = table_def
            .columns
            .iter()
            .position(|c| c.name == *name)
            .ok_or_else(|| {
                Error::Catalog(CatalogError::ColumnNotFound {
                    table: table.to_string(),
                    column: name.clone(),
                })
            })?;
        crate::planner::claim_target_column(&mut selected, position, name)?;
    }
    Ok(selected)
}

/// The number of columns a COPY transfers: the explicit list, else every table column.
///
/// It fails loud on a bad target — a table the catalog lacks (`42P01`), a named column
/// the relation lacks (`42703`), a column the list already holds (`42701`) — so a host
/// sends a `CopyInResponse`/`CopyOutResponse` for none of the three. It is the catalog
/// resolution a `COPY`'s plan leaves to execution, which a host resolves before it runs
/// the statement.
///
/// # Errors
///
/// `42P01`, `42703` or `42701`, as above.
pub fn copy_column_count(schema: &Schema, params: &CopyParams<'_>) -> Result<usize> {
    let table = get_table_or_err(schema, params.table)?;
    // The resolver owns the column-resolution rules (empty = all columns; a missing
    // named column → `42703`, a repeated one → `42701`, the two interleaved one list
    // item at a time); the header only needs the count.
    Ok(resolve_columns(params.table, table, params.columns)?.len())
}

/// Apply the COPY **output** escape to one already-rendered field.
///
/// Escapes the backslash, the named control bytes (`\b \f \n \r \t \v`), and the
/// delimiter byte (as `\<delim>`); every other byte is emitted raw (high bytes and
/// other control chars are not escaped — matching Postgres text output). The
/// delimiter, when it is itself one of the named control bytes (the default tab →
/// `\t`), is covered by that arm, so it is not double-handled.
#[must_use]
fn copy_escape_field(field: &str, delimiter: u8) -> String {
    // Worst case every char doubles; one allocation sized for it.
    let mut out = String::with_capacity(field.len() + 8);
    for ch in field.chars() {
        match ch {
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0B}' => out.push_str("\\v"),
            '\\' => out.push_str("\\\\"),
            // A custom single-byte delimiter that isn't a named control char above
            // (e.g. `|`) is escaped as `\<delim>`. `ch as u8` is only meaningful for
            // ASCII, so guard on the code point.
            c if (c as u32) <= 0x7F && (c as u8) == delimiter => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// Decode the COPY **input** escapes of one raw field into its byte value, then
/// validate the result in the server encoding.
///
/// The inverse of `copy_escape_field`: `\b \f \n \r \t \v` → their control bytes,
/// `\0`..`\7` → a 1–3 digit octal byte (`& 0o377`), `\x` + 1–2 hex digits → that
/// byte, and `\` + any other char → that char literally (so `\\` → `\`, `\.` → `.`).
/// A trailing lone `\` is emitted as a literal backslash. NULL detection happens on
/// the **raw** field (before this), so this is only called on non-NULL fields.
///
/// Never panics on any input. A decoded field holding a NUL or illegal UTF-8 (`\000`,
/// an explicit `\xff`) is 18.6's `22021` — `CopyReadAttributesText`'s
/// `pg_verifymbstr` over the de-escaped field — naming bytes cut from the **field**,
/// where the raw-input check in [`execute_copy_from`] cuts them from the whole input.
///
/// # Errors
///
/// Returns [`Error::InvalidEncoding`] if the decoded bytes are not valid in the server
/// encoding.
pub fn copy_unescape_field(bytes: &[u8]) -> Result<String> {
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b != b'\\' {
            out.push(b);
            i += 1;
            continue;
        }
        // A backslash escape. A trailing lone `\` is a literal backslash.
        let Some(&next) = bytes.get(i + 1) else {
            out.push(b'\\');
            i += 1;
            continue;
        };
        match next {
            b'b' => {
                out.push(0x08);
                i += 2;
            }
            b'f' => {
                out.push(0x0C);
                i += 2;
            }
            b'n' => {
                out.push(b'\n');
                i += 2;
            }
            b'r' => {
                out.push(b'\r');
                i += 2;
            }
            b't' => {
                out.push(b'\t');
                i += 2;
            }
            b'v' => {
                out.push(0x0B);
                i += 2;
            }
            b'0'..=b'7' => {
                // 1–3 octal digits after the backslash.
                let mut val: u32 = 0;
                let mut j = i + 1;
                let mut digits = 0;
                while digits < 3 {
                    match bytes.get(j) {
                        Some(&d @ b'0'..=b'7') => {
                            val = val * 8 + u32::from(d - b'0');
                            j += 1;
                            digits += 1;
                        }
                        _ => break,
                    }
                }
                out.push((val & 0o377) as u8);
                i = j;
            }
            b'x' => {
                // 1–2 hex digits after `\x`; `\x` with no hex digit emits a literal
                // `x` (the backslash is consumed) — matching Postgres. Only lowercase
                // `\x` is a hex escape (Postgres `copyfromparse.c` has `case 'x':`
                // only); an uppercase `\X` falls through to the literal-char arm.
                let mut val: u32 = 0;
                let mut j = i + 2;
                let mut digits = 0;
                while digits < 2 {
                    match bytes.get(j).map(|d| (*d as char).to_digit(16)) {
                        Some(Some(d)) => {
                            val = val * 16 + d;
                            j += 1;
                            digits += 1;
                        }
                        _ => break,
                    }
                }
                if digits == 0 {
                    out.push(b'x');
                    i += 2;
                } else {
                    out.push((val & 0xff) as u8);
                    i = j;
                }
            }
            // `\` + any other char → that char literally (`\\` → `\`, `\.` → `.`).
            other => {
                out.push(other);
                i += 2;
            }
        }
    }
    pglet_core::encoding::text_from_utf8(out)
}

/// Execute `COPY <table> [(cols)] TO STDOUT` (text), returning the encoded bytes.
///
/// Reads every row of `table` (the committed/auto-commit snapshot of `storage`),
/// projects the selected columns, renders each value with the column-type-aware text
/// encoder, and applies the COPY output escape. A NULL emits the raw `null_marker`
/// (unescaped). With `header`, a leading column-name line is emitted.
///
/// # Errors
///
/// `0A000` for a non-text format, `42P01` for a missing table, `42703` for a missing
/// named column, `42701` for one the column list already holds ([`resolve_columns`],
/// which serves both directions), or a storage error from the scan.
pub fn execute_copy_to<S: Storage>(
    storage: &S,
    schema: &Schema,
    params: &CopyParams<'_>,
) -> Result<Vec<u8>> {
    require_text_format(params.format)?;
    let table_def = get_table_or_err(schema, params.table)?;
    let selected = resolve_columns(params.table, table_def, params.columns)?;

    let mut out: Vec<u8> = Vec::new();
    if params.header {
        for (k, &ci) in selected.iter().enumerate() {
            if k > 0 {
                out.push(params.delimiter);
            }
            let name = copy_escape_field(&table_def.columns[ci].name, params.delimiter);
            out.extend_from_slice(name.as_bytes());
        }
        out.push(b'\n');
    }

    for (_id, row) in storage.scan(params.table)? {
        for (k, &ci) in selected.iter().enumerate() {
            if k > 0 {
                out.push(params.delimiter);
            }
            let value = row.get(ci).unwrap_or(&Value::Null);
            if value.is_null() {
                // The null marker is emitted raw (not escaped).
                out.extend_from_slice(params.null_marker.as_bytes());
            } else {
                let oid = table_def.columns[ci].type_oid;
                let rendered = value_to_text_with_oid(value, oid).ok_or_else(|| {
                    Error::Execution(format!(
                        "cannot render value of column \"{}\" to COPY text",
                        table_def.columns[ci].name
                    ))
                })?;
                let escaped = copy_escape_field(&rendered, params.delimiter);
                out.extend_from_slice(escaped.as_bytes());
            }
        }
        out.push(b'\n');
    }
    Ok(out)
}

/// Execute `COPY <table> [(cols)] FROM STDIN` (text), ingesting `data` and returning
/// the inserted row count.
///
/// Splits `data` into lines (`\n` terminated, a trailing `\r` stripped), stopping at
/// a lone `\.` end-of-data line; skips the first line if `header`. Each line is split
/// into fields on **unescaped** delimiter bytes; a raw field equal to `null_marker`
/// is NULL, otherwise it is unescaped and coerced to its column type. Columns omitted
/// from the COPY list are filled with their `DEFAULT`. Every assembled row runs the
/// full INSERT validation/constraint/FK/index path.
///
/// # Errors
///
/// `0A000` non-text format; `42P01` missing table / `42703` missing column / `42701` a
/// column the list already holds ([`resolve_columns`]); `22021` for input or a
/// de-escaped field not valid in the server encoding; `22P04`
/// for a wrong field count or a corrupt `\.` marker; `22P02` for
/// a value that fails its type; `23502`/`23505`/`23503`/`23514` for a constraint
/// violation; a storage error from the write.
pub fn execute_copy_from<S: Storage>(
    storage: &mut S,
    schema: &mut Schema,
    params: &CopyParams<'_>,
    data: &[u8],
) -> Result<u64> {
    require_text_format(params.format)?;
    // Own the table def so `schema.sequences` can be mutated (DEFAULT resolution)
    // while the loop reads the (cloned) column list — the same borrow split as
    // `execute_insert`.
    let table_def = get_table_or_err(schema, params.table)?.clone();
    let selected = resolve_columns(params.table, &table_def, params.columns)?;
    let n_cols = table_def.columns.len();

    // The inverse map: for each table column, which transfer field feeds it (`None`
    // → not in the COPY list, so it takes its DEFAULT).
    let mut field_of_col: Vec<Option<usize>> = vec![None; n_cols];
    for (k, &ci) in selected.iter().enumerate() {
        field_of_col[ci] = Some(k);
    }

    let col_names: Vec<&str> = table_def.columns.iter().map(|c| c.name.as_str()).collect();
    // Declared types parallel to `col_names` — the CHECK / stored-expression
    // binding scope (see `ColumnResolver::column_type`).
    let col_types: Vec<TypeOid> = table_def.columns.iter().map(|c| c.type_oid).collect();
    let enums = schema.enums.clone();
    let checks = prepare_checks(&table_def);
    let indexes = prepare_indexes(schema, params.table);
    let null_bytes = params.null_marker.as_bytes();

    // The raw input is verified in the server encoding as one buffer, and the failure
    // is raised only once a line reaching it is read — 18.6's `CopyConvertBuf` verifies
    // ahead of the line reader and defers the error until the valid bytes are used up,
    // so an illegal byte after the `\.` marker went unreported there (measured), and the
    // byte list is cut from the whole remaining input rather than from the line.
    let invalid_at = pglet_core::encoding::first_invalid_utf8_offset(data);

    let mut count: u64 = 0;
    let mut rest: &[u8] = data;
    let mut skip_header = params.header;
    while !rest.is_empty() {
        let line_start = data.len() - rest.len();
        // Carve off the next line (up to `\n`, or the remainder if none). A `match`
        // (not `map_or`) keeps the empty-default slice lazy and reads clearer than the
        // nested closure form.
        #[allow(clippy::option_if_let_else)]
        let (mut line, next) = match rest.iter().position(|&b| b == b'\n') {
            Some(p) => (&rest[..p], &rest[p + 1..]),
            None => (rest, &rest[rest.len()..]),
        };
        rest = next;
        // Ahead of the `\.` check, the header skip and the field split: a line holding
        // the failure is one 18.6's line reader never receives.
        if let Some(offset) = invalid_at
            && offset < data.len() - rest.len()
        {
            debug_assert!(offset >= line_start, "an earlier line held the failure");
            return Err(Error::InvalidEncoding {
                bytes: pglet_core::encoding::invalid_encoding_bytes(data, offset),
            });
        }
        // A trailing `\r` is part of a CRLF terminator.
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        // End-of-data marker: a line that is exactly `\.`. A line that starts with `\.`
        // and goes on is `22P04`, under the sentence 18.6 sends for `\.x` (measured).
        if line.first() == Some(&b'\\') && line.get(1) == Some(&b'.') {
            if line.len() == 2 {
                break;
            }
            return Err(Error::CopyFormat(
                "end-of-copy marker is not alone on its line".to_string(),
            ));
        }
        if skip_header {
            skip_header = false;
            continue;
        }

        let fields = split_fields(line, params.delimiter);
        // Every field is de-escaped and validated before the field count is checked or
        // any value reaches its type: 18.6's `CopyReadAttributesText` does both for the
        // whole line ahead of `NextCopyFrom`'s count check and the input functions, so
        // an escaped NUL anywhere on the line outranks `extra data …` and a bad value
        // in an earlier column alike. A field equal to the NULL marker is compared raw
        // and never de-escaped.
        let mut decoded: Vec<Option<String>> = Vec::with_capacity(fields.len());
        for raw in &fields {
            decoded.push(if *raw == null_bytes {
                None
            } else {
                Some(copy_unescape_field(raw)?)
            });
        }
        if fields.len() != selected.len() {
            return Err(field_count_error(&fields, &selected, &table_def));
        }

        let mut row: Vec<Value> = Vec::with_capacity(n_cols);
        for (ci, col) in table_def.columns.iter().enumerate() {
            let value = match field_of_col[ci] {
                None => resolve_default(col, &mut schema.sequences)?,
                Some(k) => match decoded[k].take() {
                    None => Value::Null,
                    Some(text) => coerce_column_value(Value::Text(text), col)?,
                },
            };
            row.push(value);
        }

        // The same per-row write path INSERT runs (validation → CHECK → index entries
        // → FK → insert), via the shared `constraints` helpers.
        for (col, val) in table_def.columns.iter().zip(&row) {
            validate_column(params.table, col, val, &enums)?;
        }
        enforce_row_constraints(
            storage,
            schema,
            params.table,
            &table_def.columns,
            &checks,
            &col_names,
            &col_types,
            &indexes,
            &row,
            None,
        )?;
        super::heap_row::note_row_written(&*storage, &table_def, &row);
        storage.insert(params.table, row)?;
        count += 1;
    }
    Ok(count)
}

/// The `22P04` error for a data row whose field count doesn't match the selected
/// columns: "missing data for column" (too few) or "extra data after last expected
/// column" (too many) — the Postgres-shaped sentences.
fn field_count_error(fields: &[&[u8]], selected: &[usize], table_def: &Table) -> Error {
    if fields.len() < selected.len() {
        // The first column with no field is the one Postgres names.
        let missing = selected
            .get(fields.len())
            .and_then(|&ci| table_def.columns.get(ci))
            .map_or("?", |c| c.name.as_str());
        Error::CopyFormat(format!("missing data for column \"{missing}\""))
    } else {
        Error::CopyFormat("extra data after last expected column".to_string())
    }
}

/// Split a COPY text line into raw field byte-slices on **unescaped** delimiter
/// bytes.
///
/// A backslash escapes the following byte, so a delimiter byte preceded by a
/// backslash (e.g. an escaped `\t`) is part of the field, not a separator. Field
/// slices keep their escapes intact (decoded later by [`copy_unescape_field`]). A
/// trailing lone backslash stays in the final field (never panics).
fn split_fields(line: &[u8], delimiter: u8) -> Vec<&[u8]> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < line.len() {
        let b = line[i];
        if b == b'\\' {
            // Skip the backslash and the byte it escapes (the next byte can't be a
            // separator). A trailing lone backslash sets `i` past the end → loop ends.
            i += 2;
            continue;
        }
        if b == delimiter {
            fields.push(&line[start..i]);
            start = i + 1;
        }
        i += 1;
    }
    fields.push(&line[start..]);
    fields
}

#[cfg(test)]
mod tests {
    use super::{copy_escape_field, copy_unescape_field, split_fields};

    #[test]
    fn escape_covers_backslash_and_controls() {
        // bytea-shaped: a leading backslash doubles.
        assert_eq!(copy_escape_field("\\xdeadbeef", b'\t'), "\\\\xdeadbeef");
        // tab/newline/cr escaped by their named forms.
        assert_eq!(copy_escape_field("a\tb\nc\rd", b'\t'), "a\\tb\\nc\\rd");
        // other control chars.
        assert_eq!(copy_escape_field("\u{08}\u{0C}\u{0B}", b'\t'), "\\b\\f\\v");
        // high bytes / quotes / braces emitted raw (array text).
        assert_eq!(copy_escape_field("{a,\"b c\"}", b'\t'), "{a,\"b c\"}");
    }

    #[test]
    fn escape_handles_custom_delimiter() {
        // A `|` delimiter inside a field is escaped as `\|`.
        assert_eq!(copy_escape_field("a|b", b'|'), "a\\|b");
        // ...but a tab is raw when the delimiter is `|` (tab is still a named control,
        // so it is escaped by its own rule regardless).
        assert_eq!(copy_escape_field("a\tb", b'|'), "a\\tb");
    }

    #[test]
    fn unescape_is_inverse_of_escape() {
        for raw in [
            "\\xdeadbeef",
            "a\tb\nc\rd",
            "{a,\"b c\"}",
            "plain",
            "\\N",
            "",
        ] {
            let escaped = copy_escape_field(raw, b'\t');
            assert_eq!(
                copy_unescape_field(escaped.as_bytes()).unwrap(),
                raw,
                "roundtrip {raw:?}"
            );
        }
    }

    #[test]
    fn unescape_octal_and_hex() {
        assert_eq!(copy_unescape_field(b"\\101").unwrap(), "A"); // 0o101 = 65 = 'A'
        assert_eq!(copy_unescape_field(b"\\x41").unwrap(), "A");
        // `\x` with no hex digit → literal `x`.
        assert_eq!(copy_unescape_field(b"\\xz").unwrap(), "xz");
        // Uppercase `\X` is NOT a hex escape in Postgres (only `\x` is), so it
        // decodes as a literal `X` followed by the raw digits.
        assert_eq!(copy_unescape_field(b"\\X41").unwrap(), "X41");
        // backslash-dot → literal dot (so `\.` mid-field is data).
        assert_eq!(copy_unescape_field(b"a\\.b").unwrap(), "a.b");
    }

    #[test]
    fn unescape_never_panics_on_truncation() {
        // A trailing lone backslash is a literal backslash, not a panic.
        assert_eq!(copy_unescape_field(b"abc\\").unwrap(), "abc\\");
        // `\xff` decodes to byte 0xFF, which is invalid UTF-8 → clean Err.
        assert!(copy_unescape_field(b"\\xff").is_err());
    }

    #[test]
    fn split_respects_escaped_delimiters() {
        let f = split_fields(b"a\tb\tc", b'\t');
        assert_eq!(f, vec![&b"a"[..], &b"b"[..], &b"c"[..]]);
        // An escaped tab (`\` + tab byte) does not split.
        let f = split_fields(b"a\\\tb\tc", b'\t');
        assert_eq!(f, vec![&b"a\\\tb"[..], &b"c"[..]]);
        // An escaped backslash before a real tab: `\\` then tab splits.
        let f = split_fields(b"a\\\\\tb", b'\t');
        assert_eq!(f, vec![&b"a\\\\"[..], &b"b"[..]]);
    }
}
