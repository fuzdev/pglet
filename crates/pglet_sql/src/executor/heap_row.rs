//! 18.6's size for a heap row and for one value in it — whether 18.6 would have run its
//! TOAST pass over a row, and what `pg_column_size` reports for a value.
//!
//! 18.6's `heap_form_tuple` lays a row out as a 23-byte header (`offsetof(t_bits)`), a
//! null bitmap of a bit per column when any column is NULL, the whole `MAXALIGN`ed, then
//! each non-NULL column: a varlena of at most 126 data bytes packed to a one-byte header
//! and **not** aligned, any other varlena at its type's alignment under a four-byte
//! header, a fixed-length value at its `typalign`. The per-value layouts are
//! `index_row`'s (one `pg_type` table for both), with two differences a heap row keeps
//! that an index entry does not: an enum **column** is the four-byte OID it is in
//! 18.6's heap, and an array is aligned by its element (`d` for an 8-byte element).
//!
//! A row whose `t_len` is past [`TOAST_TUPLE_THRESHOLD`] is one 18.6 hands to its TOAST
//! pass, which may compress a value or move it out of line — and a value it compressed
//! stays compressed through a later `UPDATE` of its row's other columns. pglet models
//! neither, so what it keeps is the store's per-table fact
//! (`pglet_core::Storage::rows_untoasted`) that no row of a table was ever written past
//! the threshold, ended by [`note_row_written`] at every row write.
//!
//! **`char(n)` is the one column this cannot size**: pglet stores it as the unpadded
//! `varchar` it is (`EDGES.md` row 95), where 18.6 pads it to `n`, so a row holding one
//! is sized short by the padding. So [`heap_tuple_len`] pads every length-capped column,
//! and over-counts a `varchar(n)` instead; it also sizes an enum array's elements by
//! their labels, never shorter than 18.6's four-byte OIDs. Both err toward ending the
//! watch, never toward an answer.

use super::index_row::{
    Layout, MAXIMUM_ALIGNOF, SHORT_VARLENA_MAX_DATA, VARHDRSZ, align_up, layout,
};
use crate::catalog::{Schema, Table};
use crate::planner::ColumnSizeOrigin;
use crate::types::{TypeOid, Value};
use crate::{Error, Result};
use pglet_core::Storage;

/// 18.6's `TOAST_TUPLE_THRESHOLD` over 8 KB pages: a row longer than this is handed to the
/// TOAST pass (`heap_prepare_insert`, `heap_update`).
pub const TOAST_TUPLE_THRESHOLD: usize = 2032;

/// `offsetof(HeapTupleHeaderData, t_bits)`.
const HEAP_TUPLE_HEADER: usize = 23;

/// The alignment 18.6 gives an array of `element` — its element's, at least `i`.
const fn array_align(element: TypeOid) -> usize {
    match element {
        TypeOid::Int8
        | TypeOid::Float8
        | TypeOid::Timestamp
        | TypeOid::Timestamptz
        | TypeOid::Time
        | TypeOid::Interval => 8,
        _ => VARHDRSZ,
    }
}

/// `value`'s layout as a value of a column whose type is an enum when `is_enum`, and the
/// alignment a four-byte-header varlena of it takes. `None` for a NULL, and for a value
/// with no layout.
fn column_layout(value: &Value, is_enum: bool) -> Option<(Layout, usize)> {
    let align = match value {
        Value::Array(array) => array_align(array.element_type),
        _ => VARHDRSZ,
    };
    layout(value, is_enum).map(|layout| (layout, align))
}

/// 18.6's `t_len` for `row` as `heap_form_tuple` lays it out for `table`, or `None` when a
/// value has no layout (an array of arrays, which pglet builds none of).
#[must_use]
pub fn heap_tuple_len(table: &Table, row: &[Value]) -> Option<usize> {
    let mut data = 0usize;
    for (value, column) in row.iter().zip(&table.columns) {
        if value.is_null() {
            continue;
        }
        // a `char(n)` is 18.6's `n` characters, blank-padded, and a length-capped
        // `varchar` is not told apart from one here: size both padded, which can only
        // over-count
        let padded;
        let value = match (value, column.varchar_length) {
            (Value::Text(s), Some(n)) => {
                let chars = s.chars().count();
                let n = usize::try_from(n).unwrap_or(usize::MAX);
                padded = Value::Text(format!("{s}{}", " ".repeat(n.saturating_sub(chars))));
                &padded
            }
            _ => value,
        };
        data = match column_layout(value, column.enum_type.is_some())? {
            (Layout::Fixed { len, align }, _) => align_up(data, align) + len,
            (Layout::Varlena(n), _) if n <= SHORT_VARLENA_MAX_DATA => data + 1 + n,
            (Layout::Varlena(n), align) => align_up(data, align) + VARHDRSZ + n,
        };
    }
    let bitmap = if row.iter().any(Value::is_null) {
        row.len().div_ceil(8)
    } else {
        0
    };
    Some(align_up(HEAP_TUPLE_HEADER + bitmap, MAXIMUM_ALIGNOF) + data)
}

/// Record a write of `row` to `table` with the store: a row 18.6 would hand to its TOAST
/// pass — or one this cannot size — ends the table's untoasted fact for good
/// (`pglet_core::Storage::note_toasted_row`).
///
/// Called before every row write the executor makes, at each site rather than in the
/// store, because the size needs the column types the store does not have.
pub fn note_row_written<S: Storage>(storage: &S, table: &Table, row: &[Value]) {
    if heap_tuple_len(table, row).is_none_or(|len| len > TOAST_TUPLE_THRESHOLD) {
        storage.note_toasted_row(&table.name);
    }
}

/// [`note_row_written`] for a table looked up in `schema` by name — a table the catalog
/// has no entry for is one this cannot size, so its watch ends.
pub fn note_row_written_to<S: Storage>(storage: &S, schema: &Schema, table: &str, row: &[Value]) {
    match schema.get_table(table) {
        Some(table_def) => note_row_written(storage, table_def, row),
        None => storage.note_toasted_row(table),
    }
}

/// `pg_column_size(value)` for an argument whose origin the coercion pass read as
/// `origin`; `untoasted` is the store's answer for a stored value's table
/// (`pglet_core::Storage::rows_untoasted`).
///
/// A NULL is NULL whatever its origin. Otherwise the answer is 18.6's where the origin
/// settles the bytes — a constant under a four-byte header (a bare string literal as the
/// C string `unknown` is), a stored fixed-length value at its length, a stored varlena of
/// a table no row of which reached the TOAST pass under the header `heap_form_tuple`
/// packed it to — and `0A000` everywhere else, the bytes being ones pglet does not model.
///
/// # Errors
///
/// [`Error::ColumnSizeNotModelled`] (`0A000`) for a non-NULL value whose bytes are not
/// known.
pub fn column_size(value: &Value, origin: &ColumnSizeOrigin, untoasted: bool) -> Result<Value> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    let size = match origin {
        ColumnSizeOrigin::Constant { unknown: true } => match value {
            Value::Text(s) => Ok(s.len() + 1),
            _ => Err("a value of no modelled layout"),
        },
        ColumnSizeOrigin::Constant { unknown: false } => match layout(value, false) {
            Some(Layout::Fixed { len, .. }) => Ok(len),
            Some(Layout::Varlena(n)) => Ok(VARHDRSZ + n),
            None => Err("a value of no modelled layout"),
        },
        // an enum array's elements are 18.6's four-byte OIDs, which `layout` sizes by label
        ColumnSizeOrigin::Stored { is_enum: true, .. } if matches!(value, Value::Array(_)) => {
            Err("a stored enum array")
        }
        ColumnSizeOrigin::Stored { is_enum, .. } => match layout(value, *is_enum) {
            Some(Layout::Fixed { len, .. }) => Ok(len),
            Some(Layout::Varlena(n)) if untoasted && n <= SHORT_VARLENA_MAX_DATA => Ok(1 + n),
            Some(Layout::Varlena(n)) if untoasted => Ok(VARHDRSZ + n),
            Some(Layout::Varlena(_)) => Err(
                "a varying-length value of a table whose rows may have reached the TOAST threshold",
            ),
            None => Err("a value of no modelled layout"),
        },
        ColumnSizeOrigin::Unclassified => {
            Err("an argument in this clause (one 18.6 sizes before the row is stored)")
        }
        ColumnSizeOrigin::Unsupported => Err(
            "a computed argument, or a column read through a subquery, a derived table, an \
             enclosing query or a length cap",
        ),
    };
    size.map_err(Error::ColumnSizeNotModelled).and_then(|size| {
        i32::try_from(size)
            .map(Value::Int4)
            .map_err(|_| Error::ColumnSizeNotModelled("a value past int4"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Column;
    use crate::types::{ArrayValue, PgNumeric};
    use pglet_core::types::parse_jsonb;

    fn col(name: &str, type_oid: TypeOid, position: i16) -> Column {
        Column {
            name: name.to_string(),
            type_oid,
            nullable: true,
            position,
            default: None,
            enum_type: None,
            numeric_typmod: None,
            varchar_length: None,
            temporal_precision: None,
            identity: None,
        }
    }

    /// `hl (id int, s text, b bytea, r float8, ia int8[], n numeric, j jsonb, e int)`.
    fn hl() -> Table {
        Table {
            name: "hl".to_string(),
            schema: "public".to_string(),
            oid: 1,
            checks: Vec::new(),
            foreign_keys: Vec::new(),
            columns: vec![
                col("id", TypeOid::Int4, 1),
                col("s", TypeOid::Text, 2),
                col("b", TypeOid::Bytea, 3),
                col("r", TypeOid::Float8, 4),
                col("ia", TypeOid::Int8Array, 5),
                col("n", TypeOid::Numeric, 6),
                col("j", TypeOid::Jsonb, 7),
                col("e", TypeOid::Int4, 8),
            ],
        }
    }

    /// The `t_len` 18.6 gives each row, read as `pg_column_size(hl.*)` (a whole-row datum
    /// is the row's tuple) — measured on 18.6.
    #[test]
    fn a_rows_length_is_18_6s_t_len() {
        let null = || Value::Null;
        let row = |id: i32| -> Vec<Value> {
            let mut row = vec![null(); 8];
            row[0] = Value::Int4(id);
            row
        };
        // 1: a 1 990-byte `bytea` beside a short `text` — 2 030, just within the threshold
        let mut one = row(1);
        one[1] = Value::Text("hello".into());
        one[2] = Value::Bytea(vec![0x78; 1990]);
        // 2: every column, each short varlena packed and unaligned between fixed ones
        let two = vec![
            Value::Int4(2),
            Value::Text("hello".into()),
            Value::Bytea(vec![0xde, 0xad]),
            Value::Float8(1.5),
            Value::Array(ArrayValue::new(
                TypeOid::Int8,
                vec![Value::Int8(1), Value::Int8(2)],
            )),
            Value::Numeric(PgNumeric::parse("12.5").expect("numeric")),
            Value::Jsonb(parse_jsonb(r#"{"a":1}"#).expect("jsonb")),
            Value::Int4(7),
        ];
        // 3: all NULL but one: the bitmap rides in the header's padding
        let three = row(3);
        // 4: a 300-byte `text`, past the short form, under a four-byte header
        let mut four = row(4);
        four[1] = Value::Text("y".repeat(300));
        let table = hl();
        for (row, expected) in [(one, 2030), (two, 124), (three, 28), (four, 332)] {
            assert_eq!(heap_tuple_len(&table, &row), Some(expected), "{row:?}");
        }
    }

    #[test]
    fn a_stored_varlena_takes_the_header_its_row_packed_it_to() {
        let stored = ColumnSizeOrigin::Stored {
            table: "t".into(),
            is_enum: false,
        };
        let short = Value::Bytea(vec![0; 4]);
        let long = Value::Text("x".repeat(200));
        assert_eq!(
            column_size(&short, &stored, true).ok(),
            Some(Value::Int4(5))
        );
        assert_eq!(
            column_size(&long, &stored, true).ok(),
            Some(Value::Int4(204))
        );
        // a table not known untoasted refuses a varlena and answers a fixed-length value
        assert!(column_size(&short, &stored, false).is_err());
        assert_eq!(
            column_size(&Value::Int8(1), &stored, false).ok(),
            Some(Value::Int4(8))
        );
        let constant = ColumnSizeOrigin::Constant { unknown: false };
        assert_eq!(
            column_size(&short, &constant, false).ok(),
            Some(Value::Int4(8))
        );
        let unknown = ColumnSizeOrigin::Constant { unknown: true };
        assert_eq!(
            column_size(&Value::Text("abc".into()), &unknown, false).ok(),
            Some(Value::Int4(4))
        );
        assert!(column_size(&short, &ColumnSizeOrigin::Unsupported, true).is_err());
        assert_eq!(
            column_size(&Value::Null, &ColumnSizeOrigin::Unsupported, true).ok(),
            Some(Value::Null)
        );
    }
}
