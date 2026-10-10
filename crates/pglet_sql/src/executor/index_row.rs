//! 18.6's size for an index entry, and the three refusals it answers above its limits.
//!
//! 18.6 lays an index entry out as `index_form_tuple` does — an 8-byte `IndexTupleData`
//! header, a null bitmap when any key part is NULL, then each non-NULL part at its
//! type's alignment, a varlena of at most 126 data bytes packed to a one-byte header —
//! and refuses the entry, `54000`, at three points:
//!
//! - **`index_form_tuple`**, for every access method: the tuple, `MAXALIGN`ed, must fit
//!   the header's 13-bit size field — `index row requires N bytes, maximum size is 8191`.
//! - **`_bt_check_third_page`**, a B-tree's: at most `BTMaxItemSize`, a third of an 8 KB
//!   page less its overheads — `index row size N exceeds btree version 4 maximum 2704`.
//!   A **unique** B-tree probes for a duplicate between the two (`_bt_doinsert` checks
//!   uniqueness before `_bt_findinsertloc` checks the size), and 18.6 inserts into a
//!   table's indexes one at a time in OID order, so a table's two indexes interleave the
//!   two codes by creation order (measured both ways). [`index_entry_size`] is the one
//!   size; the callers order the checks.
//! - **`GinFormTuple`**, a GIN's, per entry: at most `GinMaxItemSize`, 2 712 bytes —
//!   `index row size N exceeds maximum 2712`.
//!
//! **Compression is not modelled.** 18.6 pglz-compresses a varlena key part over
//! `TOAST_INDEX_TARGET` (510 bytes) before it measures, so a key part that compresses —
//! a repeated character — passes there at any length its compressed form fits, and is
//! measured here at its full length (`EDGES.md` row 191) — a `text`, a `jsonb` and an
//! array alike.
//!
//! The layouts are 18.6's `pg_type` rows (`typlen`, `typalign`): `bool` 1/`c`, `int2`
//! 2/`s`, `int4`, `float4`, `date` and an **enum** 4/`i` — an enum by its type, where the
//! key part's type is known to be one; behind a written cast it is not, and the part is
//! sized by its label (`EDGES.md` row 104) — `int8`, `float8`, `time` and the timestamps
//! 8/`d`, `interval` 16/`d`, `uuid` 16/`c`, and `text`, `varchar`, `bytea` and `numeric`
//! varlenas aligned `i`, as are a `jsonb` (`jsonb_data_len`) and an array
//! (`array_data_len`). A `char(n)` value is the `varchar` pglet stores, sized unpadded
//! where 18.6 pads it to `n` (row 95).
//! A `numeric` varlena carries a two-byte header when its display scale is at most 63
//! and its base-10000 weight is within ±64 (`NUMERIC_CAN_BE_SHORT`), else four, and
//! two bytes a base-10000 digit.

use pglet_core::error_fields::IndexRowSize;
use pglet_core::types::{JsonbValue, PgNumeric};

use super::expr::{EnumLeaves, expr_enum_with};
use crate::Error;
use crate::Result;
use crate::planner::Expr;
use crate::types::Value;

/// 18.6's `BTMaxItemSize` for a version-4 B-tree over 8 KB pages.
pub const BTREE_MAX_ITEM_SIZE: usize = 2704;

/// 18.6's `GinMaxItemSize` over 8 KB pages.
pub const GIN_MAX_ITEM_SIZE: usize = 2712;

/// 18.6's `INDEX_SIZE_MASK`: the most an index tuple's 13-bit size field can say.
pub const INDEX_SIZE_MASK: usize = 0x1FFF;

/// `MAXALIGN`'s alignment on the platforms 18.6 is measured on.
const MAXIMUM_ALIGNOF: usize = 8;

/// `sizeof(IndexTupleData)`.
const INDEX_TUPLE_HEADER: usize = 8;

/// `sizeof(IndexAttributeBitMapData)`, present when any key part is NULL.
const NULL_BITMAP: usize = 4;

/// A four-byte varlena header.
const VARHDRSZ: usize = 4;

/// The most data a one-byte-header ("short") varlena carries: `VARATT_SHORT_MAX` (127)
/// less its one-byte header.
const SHORT_VARLENA_MAX_DATA: usize = 126;

/// Round `n` up to a multiple of `align` (a power of two).
const fn align_up(n: usize, align: usize) -> usize {
    (n + align - 1) & !(align - 1)
}

/// How 18.6 stores one non-NULL key part.
enum Layout {
    /// A fixed-length type: its `typlen` and `typalign` in bytes.
    Fixed { len: usize, align: usize },
    /// A varlena of this many data bytes, header excluded.
    Varlena(usize),
}

/// A `numeric`'s data bytes past its varlena header, as `numeric_in` stores it.
fn numeric_data_len(n: &PgNumeric) -> usize {
    // The binary send format carries exactly the fields the on-disk form is sized by:
    // the base-10000 digit count, the weight and the display scale.
    let wire = n.to_wire_bytes();
    let field = |at: usize| i16::from_be_bytes([wire[at], wire[at + 1]]);
    if n.is_nan() {
        return 2;
    }
    let ndigits = usize::try_from(field(0)).unwrap_or(0);
    let weight = field(2);
    let dscale = field(6);
    let short = (0..=63).contains(&dscale) && (-64..=63).contains(&weight);
    (if short { 2 } else { 4 }) + 2 * ndigits
}

/// `value`'s layout, or `None` for a value with none: a NULL, which takes no data.
/// `is_enum` is the key part's type, which a `Value::Text` cannot carry.
fn layout(value: &Value, is_enum: bool) -> Option<Layout> {
    let fixed = |len, align| Some(Layout::Fixed { len, align });
    match value {
        Value::Bool(_) => fixed(1, 1),
        Value::Int2(_) => fixed(2, 2),
        Value::Int4(_) | Value::Float4(_) | Value::Date(_) => fixed(4, 4),
        Value::Text(_) if is_enum => fixed(4, 4),
        Value::Int8(_) | Value::Float8(_) | Value::Time(_) | Value::Timestamp(_) => fixed(8, 8),
        Value::Interval(_) => fixed(16, 8),
        Value::Uuid(_) => fixed(16, 1),
        Value::Text(s) => Some(Layout::Varlena(s.len())),
        Value::Bytea(b) => Some(Layout::Varlena(b.len())),
        Value::Numeric(n) => Some(Layout::Varlena(numeric_data_len(n))),
        Value::Jsonb(j) => Some(Layout::Varlena(jsonb_data_len(j))),
        Value::Array(a) => array_data_len(&a.elements).map(Layout::Varlena),
        Value::Null => None,
    }
}

/// An array's data bytes past its varlena header, as 18.6's `ArrayType` stores it.
///
/// An empty array is the bare twelve-byte header (`ndim` 0). Any other is that header,
/// one dimension and its lower bound, and a null bitmap when an element is NULL, the
/// whole `MAXALIGN`ed with the varlena header counted; then each non-NULL element at
/// its type's alignment — a varlena element under a **four-byte** header whatever its
/// length, and padded to its alignment after. `None` for an element with no layout,
/// which an array of arrays would be and pglet has none of. An enum element is sized
/// by its label: the element's type is not here to read (`EDGES.md` row 104).
fn array_data_len(elements: &[Value]) -> Option<usize> {
    if elements.is_empty() {
        return Some(12);
    }
    let bitmap = if elements.iter().any(Value::is_null) {
        elements.len().div_ceil(8)
    } else {
        0
    };
    // Offsets from the array's own start, its varlena header included: that is what
    // each element is aligned against.
    let mut end = align_up(VARHDRSZ + 20 + bitmap, MAXIMUM_ALIGNOF);
    for element in elements.iter().filter(|e| !e.is_null()) {
        end = match layout(element, false)? {
            Layout::Fixed { len, align } => align_up(end, align) + len,
            Layout::Varlena(n) => align_up(end + VARHDRSZ + n, VARHDRSZ),
        };
    }
    Some(end - VARHDRSZ)
}

/// A `jsonb`'s data bytes past its varlena header, as 18.6's `JsonbContainer` stores it:
/// a top-level scalar is a one-element array holding it.
fn jsonb_data_len(value: &JsonbValue) -> usize {
    match value {
        JsonbValue::Array(_) | JsonbValue::Object(_) => jsonb_end(value, 0),
        scalar => jsonb_end(scalar, 8),
    }
}

/// Where `value` ends when 18.6 writes it at offset `at` of a `jsonb`'s data.
///
/// A container is a four-byte header and a four-byte entry per element — two per pair,
/// key and value — then an object's key bytes, then the values in stored order; a
/// string is its bytes, `null` and a boolean live in their entry, and a number is its
/// `numeric` under a four-byte varlena header. A container and a number start on a
/// four-byte boundary.
fn jsonb_end(value: &JsonbValue, at: usize) -> usize {
    match value {
        JsonbValue::Null | JsonbValue::Bool(_) => at,
        JsonbValue::String(s) => at + s.len(),
        JsonbValue::Number(n) => align_up(at, VARHDRSZ) + VARHDRSZ + numeric_data_len(n),
        JsonbValue::Array(items) => {
            let start = align_up(at, VARHDRSZ) + 4 + 4 * items.len();
            items.iter().fold(start, |at, item| jsonb_end(item, at))
        }
        JsonbValue::Object(pairs) => {
            let start = align_up(at, VARHDRSZ) + 4 + 8 * pairs.len();
            let keys = pairs.iter().map(|(key, _)| key.len()).sum::<usize>();
            pairs
                .iter()
                .fold(start + keys, |at, (_, item)| jsonb_end(item, at))
        }
    }
}

/// The size 18.6's `index_form_tuple` gives an entry holding `parts`, in bytes.
///
/// Each part is a key value and whether its type is an enum; the size is `MAXALIGN`ed,
/// or `None` when a part has no layout — an array of arrays, which pglet builds none of.
///
/// A NULL part takes no data but adds the null bitmap to the header, so a NULL in one
/// part can push the entry over a limit it met with the part absent: `(NULL, text)`
/// is refused from 2 685 characters, `(text)` alone from 2 693 (both measured, with the
/// character before each accepted).
pub fn index_entry_size<'a>(parts: impl IntoIterator<Item = (&'a Value, bool)>) -> Option<usize> {
    let mut data = 0usize;
    let mut has_null = false;
    for (value, is_enum) in parts {
        if value.is_null() {
            has_null = true;
            continue;
        }
        match layout(value, is_enum)? {
            Layout::Fixed { len, align } => data = align_up(data, align) + len,
            Layout::Varlena(n) if n <= SHORT_VARLENA_MAX_DATA => data += 1 + n,
            Layout::Varlena(n) => data = align_up(data, VARHDRSZ) + VARHDRSZ + n,
        }
    }
    let header = if has_null {
        align_up(INDEX_TUPLE_HEADER + NULL_BITMAP, MAXIMUM_ALIGNOF)
    } else {
        INDEX_TUPLE_HEADER
    };
    Some(align_up(header + data, MAXIMUM_ALIGNOF))
}

/// `index_form_tuple`'s refusal, which every access method meets first.
///
/// # Errors
///
/// [`Error::IndexRowRequiresTooMuch`] when `size` is past [`INDEX_SIZE_MASK`].
pub const fn check_entry_size_field(size: Option<usize>) -> Result<()> {
    match size {
        Some(size) if size > INDEX_SIZE_MASK => Err(Error::IndexRowRequiresTooMuch { size }),
        _ => Ok(()),
    }
}

/// `_bt_check_third_page`'s refusal of a B-tree entry over [`BTREE_MAX_ITEM_SIZE`].
///
/// # Errors
///
/// [`Error::BtreeIndexRowTooLarge`] naming the index and its relation.
pub fn check_btree_item_size(size: Option<usize>, index: &str, table: &str) -> Result<()> {
    match size {
        Some(size) if size > BTREE_MAX_ITEM_SIZE => {
            Err(Error::BtreeIndexRowTooLarge(Box::new(IndexRowSize {
                size,
                index: index.to_string(),
                table: table.to_string(),
            })))
        }
        _ => Ok(()),
    }
}

/// A GIN index's refusals over the entries a row's key value decomposes into
/// (`gin::gin_decompose`), each entry a one-part tuple asked both of `GinFormTuple`'s
/// questions in turn.
///
/// 18.6 extracts a row's entries, sorts them, and inserts them in that order, so the
/// entry it names is the first oversized one in its sort; the entries are sorted by
/// their index-key bytes here, which is the `C` collation's order and not every
/// collation's. Only an **array** key is asked: a `jsonb` key decomposes into tokens
/// 18.6's `jsonb_ops` hashes past 125 bytes, so none is ever over its limit there.
///
/// # Errors
///
/// [`Error::IndexRowRequiresTooMuch`] or [`Error::GinIndexRowTooLarge`] for the first
/// oversized entry.
pub fn check_gin_entries(key: &Value, entries: &[Value], index: &str, table: &str) -> Result<()> {
    if !matches!(key, Value::Array(_)) {
        return Ok(());
    }
    let mut sized: Vec<(Vec<u8>, usize)> = entries
        .iter()
        .filter_map(|entry| {
            let size = index_entry_size([(entry, false)])?;
            let order = entry.encode_index_key().unwrap_or_default();
            Some((order, size))
        })
        .filter(|&(_, size)| size > GIN_MAX_ITEM_SIZE)
        .collect();
    sized.sort();
    let Some(&(_, size)) = sized.first() else {
        return Ok(());
    };
    check_entry_size_field(Some(size))?;
    Err(Error::GinIndexRowTooLarge(Box::new(IndexRowSize {
        size,
        index: index.to_string(),
        table: table.to_string(),
    })))
}

/// The longest key, in bytes, pglet's B+tree stores.
///
/// What an internal page can carry as a separator beside its `first_child`:
/// `pglet_storage`'s `btree::MAX_KEY_LEN`, which a
/// test there holds this equal to; the build gates below read it here because this crate
/// cannot read that one.
pub const PAGE_KEY_LEN_MAX: usize = 4044;

/// The length of the B+tree key pglet's storage writes for an entry holding `values`.
///
/// `None` when it writes none (a NULL part under `NULLS DISTINCT`) or a part has no
/// key encoding (the encoder's own refusal, which is asked elsewhere).
///
/// The framing is `pglet_storage`'s: each part tagged (`0x00` NULL, `0x01 ++` its
/// `encode_index_key` bytes), one part taken verbatim and several each escaped and
/// terminated, and the whole value escaped and terminated once more inside the index key
/// beside its 1-byte tag, 4-byte index id and 8-byte row id — every `0x00` escaped to two
/// bytes each time. A test in `pglet_storage` holds this equal to the key the storage
/// actually builds. It is pglet's own limit, [`PAGE_KEY_LEN_MAX`], that this is asked
/// against: 18.6's layout packs a NUL-dense `bytea` and a long `numeric` tighter than this
/// encoding does (`EDGES.md` row 192).
#[must_use]
pub fn encoded_key_len(values: &[Value], nulls_not_distinct: bool) -> Option<usize> {
    fn escaped(bytes: impl Iterator<Item = u8>) -> usize {
        bytes.map(|b| if b == 0 { 2 } else { 1 }).sum::<usize>() + 2
    }
    if !nulls_not_distinct && values.iter().any(Value::is_null) {
        return None;
    }
    let mut segments: Vec<Vec<u8>> = Vec::with_capacity(values.len());
    for value in values {
        if value.is_null() {
            segments.push(vec![0x00]);
        } else {
            let mut segment = vec![0x01];
            segment.extend(value.encode_index_key().ok()?);
            segments.push(segment);
        }
    }
    let blob: Vec<u8> = match segments.as_slice() {
        [one] => one.clone(),
        many => {
            let mut out = Vec::new();
            for segment in many {
                for &b in segment {
                    if b == 0 {
                        out.extend([0x00, 0xFF]);
                    } else {
                        out.push(b);
                    }
                }
                out.extend([0x00, 0x00]);
            }
            out
        }
    };
    Some(1 + 4 + escaped(blob.into_iter()) + 8)
}

/// pglet's own refusal of an entry whose key outgrows [`PAGE_KEY_LEN_MAX`].
///
/// # Errors
///
/// [`Error::IndexKeyTooLongForPage`] naming the index.
pub fn check_page_key_len(len: Option<usize>, index: &str) -> Result<()> {
    match len {
        Some(len) if len > PAGE_KEY_LEN_MAX => Err(Error::IndexKeyTooLongForPage {
            index: index.to_string(),
            len,
            max: PAGE_KEY_LEN_MAX,
        }),
        _ => Ok(()),
    }
}

/// Whether a key expression's type is an enum, over the indexed table's columns — the
/// one fact about a key part's type a `Value::Text` does not carry. `enum_column`
/// answers for a column name.
pub fn key_part_is_enum(expr: &Expr, enum_column: &dyn Fn(&str) -> bool) -> bool {
    expr_enum_with(
        expr,
        &EnumLeaves {
            column: &|_, name| enum_column(name).then_some(()),
            subquery: &|_| None,
        },
    )
    .is_some()
}

/// The enum a key expression's type is, by name, over the indexed table's columns.
///
/// It is what orders the part's values in the index; `enum_column` names a column's
/// enum. The same walk as [`key_part_is_enum`], so the two cannot disagree about which
/// parts are enums; a written cast is invisible to both (`EDGES.md` row 104).
pub fn key_part_enum(expr: &Expr, enum_column: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    expr_enum_with(
        expr,
        &EnumLeaves {
            column: &|_, name| enum_column(name),
            subquery: &|_| None,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(n: usize) -> Value {
        Value::Text("x".repeat(n))
    }

    /// Each boundary below is an 18.6 measurement over incompressible text: the
    /// longest key accepted, and one more character refused with the size shown.
    #[test]
    fn sizes_match_the_measured_boundaries() {
        let size = |parts: &[(&Value, bool)]| index_entry_size(parts.iter().copied()).unwrap();
        // `(text)`: 2 692 accepted, 2 693 is 2 712.
        assert_eq!(size(&[(&text(2692), false)]), 2704);
        assert_eq!(size(&[(&text(2693), false)]), 2712);
        assert_eq!(size(&[(&text(3000), false)]), 3016);
        // `(int8, text)`: 2 684 / 2 685.
        let i8 = Value::Int8(1);
        assert_eq!(size(&[(&i8, false), (&text(2684), false)]), 2704);
        assert_eq!(size(&[(&i8, false), (&text(2685), false)]), 2712);
        // `(NULL, text)`: the bitmap moves the data to 16 — 2 684 / 2 685.
        assert_eq!(size(&[(&Value::Null, false), (&text(2684), false)]), 2704);
        assert_eq!(size(&[(&Value::Null, false), (&text(2685), false)]), 2712);
        // `(text, text)`: 1 343 each accepted, 1 350 each is 2 720.
        assert_eq!(size(&[(&text(1343), false), (&text(1343), false)]), 2704);
        assert_eq!(size(&[(&text(1350), false), (&text(1350), false)]), 2720);
        // A short varlena is unaligned: `('ab', text)` holds 2 688.
        assert_eq!(size(&[(&text(2), false), (&text(2688), false)]), 2704);
        // An enum is four bytes whatever its label: 2 688 / 2 689.
        let label = text(62);
        assert_eq!(size(&[(&label, true), (&text(2688), false)]), 2704);
        assert_eq!(size(&[(&label, true), (&text(2689), false)]), 2712);
        // `(bool, int2, text)`: 2 688 / 2 689.
        let (b, i2) = (Value::Bool(true), Value::Int2(1));
        assert_eq!(
            size(&[(&b, false), (&i2, false), (&text(2688), false)]),
            2704
        );
        assert_eq!(
            size(&[(&b, false), (&i2, false), (&text(2689), false)]),
            2712
        );
        // `(uuid, text)`: 2 676 / 2 677.
        let u = Value::Uuid([1; 16]);
        assert_eq!(size(&[(&u, false), (&text(2676), false)]), 2704);
        assert_eq!(size(&[(&u, false), (&text(2677), false)]), 2712);
        // `(text, interval)`: the interval is aligned `d` — 2 676 holds.
        let iv = Value::Interval(pglet_core::types::PgInterval {
            months: 0,
            days: 1,
            micros: 0,
        });
        assert_eq!(size(&[(&text(2676), false), (&iv, false)]), 2704);
        // `index_form_tuple`'s field: 8 173 characters is 8 192 bytes.
        assert_eq!(size(&[(&text(8172), false)]), 8184);
        assert_eq!(size(&[(&text(8173), false)]), 8192);
        assert!(check_entry_size_field(Some(8184)).is_ok());
        assert!(matches!(
            check_entry_size_field(Some(8192)),
            Err(Error::IndexRowRequiresTooMuch { size: 8192 })
        ));
        // A GIN entry is a one-part tuple against 2 712: 2 700 holds, 2 701 is 2 720.
        assert_eq!(size(&[(&text(2700), false)]), 2712);
        assert_eq!(size(&[(&text(2701), false)]), 2720);
    }

    /// Each fact of the layout at an offset where it moves the boundary — a part after
    /// the first, where `MAXALIGN` of the whole row does not hide it. Every pair is an
    /// 18.6 measurement: the longest trailing text accepted, and one more refused.
    #[test]
    fn each_layout_fact_moves_a_measured_boundary() {
        let size = |parts: &[(&Value, bool)]| index_entry_size(parts.iter().copied()).unwrap();
        let i4 = Value::Int4(1);
        // `int8` and `float8` align to 8 after an `int4`: 2 676 / 2 677.
        for fixed in [Value::Int8(1), Value::Float8(1.0)] {
            assert_eq!(
                size(&[(&i4, false), (&fixed, false), (&text(2676), false)]),
                2704
            );
            assert_eq!(
                size(&[(&i4, false), (&fixed, false), (&text(2677), false)]),
                2712
            );
        }
        // `interval` is 16 bytes aligned to 8 after an `int4`: 2 668 / 2 669.
        let iv = Value::Interval(pglet_core::types::PgInterval {
            months: 0,
            days: 1,
            micros: 0,
        });
        assert_eq!(
            size(&[(&i4, false), (&iv, false), (&text(2668), false)]),
            2704
        );
        assert_eq!(
            size(&[(&i4, false), (&iv, false), (&text(2669), false)]),
            2712
        );
        // `uuid` is aligned to 1, right after a 2-byte short varlena: 2 672 / 2 673.
        let (a, u) = (text(1), Value::Uuid([1; 16]));
        assert_eq!(
            size(&[(&a, false), (&u, false), (&text(2672), false)]),
            2704
        );
        assert_eq!(
            size(&[(&a, false), (&u, false), (&text(2673), false)]),
            2712
        );
        // A 126-byte varlena keeps the one-byte header, a 127-byte one takes four:
        // `(text 126, text)` 2 564 / 2 565, `(text 127, text)` 2 560 / 2 561.
        assert_eq!(size(&[(&text(126), false), (&text(2564), false)]), 2704);
        assert_eq!(size(&[(&text(126), false), (&text(2565), false)]), 2712);
        assert_eq!(size(&[(&text(127), false), (&text(2560), false)]), 2704);
        assert_eq!(size(&[(&text(127), false), (&text(2561), false)]), 2712);
        // A `numeric`'s display scale past 63 takes the long numeric header, which moves
        // an `int2` after it: `(numeric, int2, text)` 2 684 / 2 685 at scale 63, 2 680 /
        // 2 681 at scale 64.
        let i2 = Value::Int2(1);
        for (zeros, at) in [(63, 2684), (64, 2680)] {
            let n = Value::Numeric(PgNumeric::parse(&format!("1.{}", "0".repeat(zeros))).unwrap());
            assert_eq!(size(&[(&n, false), (&i2, false), (&text(at), false)]), 2704);
            assert_eq!(
                size(&[(&n, false), (&i2, false), (&text(at + 1), false)]),
                2712
            );
        }
    }

    /// A `numeric` of 2 000 significant digits is 500 base-10000 digits under a long
    /// header — 1 008 bytes as 18.6's `pg_column_size` reports it, measured — and
    /// `(numeric, text)` then holds 1 684 characters and refuses 1 685.
    #[test]
    fn a_long_numeric_is_sized_by_its_base_10000_digits() {
        let digits: String = (0..2000)
            .map(|i| char::from(b'1' + (i % 9) as u8))
            .collect();
        let n = Value::Numeric(PgNumeric::parse(&digits).unwrap());
        let Value::Numeric(inner) = &n else {
            unreachable!()
        };
        assert_eq!(numeric_data_len(inner) + VARHDRSZ, 1008);
        let size = |m| index_entry_size([(&n, false), (&text(m), false)]).unwrap();
        assert_eq!(size(1684), 2704);
        assert_eq!(size(1685), 2712);
        // A short-header numeric: `1.5` is one weight-0 digit and one fractional one.
        let short = PgNumeric::parse("1.5").unwrap();
        assert_eq!(numeric_data_len(&short), 2 + 2 * 2);
    }

    /// A stored value's size on 18.6, header included — `pg_column_size` of a table
    /// column, which packs a varlena as an index entry does.
    fn stored(value: &Value) -> usize {
        match layout(value, false).unwrap() {
            Layout::Varlena(n) if n <= SHORT_VARLENA_MAX_DATA => 1 + n,
            Layout::Varlena(n) => VARHDRSZ + n,
            Layout::Fixed { len, .. } => len,
        }
    }

    /// Each size is 18.6's `pg_column_size` of the stored value, measured.
    #[test]
    fn a_jsonb_is_sized_as_its_container() {
        for (text, size) in [
            ("null", 9),
            ("true", 9),
            ("0", 15),
            ("1", 17),
            ("1.5", 19),
            ("12345678901234567890", 25),
            ("1e-70", 19),
            ("1e300", 19),
            (r#""""#, 9),
            (r#""a""#, 10),
            (r#""abcd""#, 13),
            ("[]", 5),
            ("{}", 5),
            ("[1]", 17),
            ("[1,2]", 29),
            (r#"["a",1]"#, 25),
            (r#"["ab",1]"#, 25),
            ("[null,true]", 13),
            (r#"{"a":1}"#, 25),
            (r#"{"a":"b"}"#, 15),
            (r#"{"ab":"b","c":1}"#, 34),
            (r#"{"a":[1,2],"b":{"c":null}}"#, 66),
            (r#"[[1],"abc",2.5]"#, 47),
            ("[[]]", 13),
            (r#"["a",[]]"#, 21),
            (r#"["a",{}]"#, 21),
            (r#"["a",[1]]"#, 33),
        ] {
            let j = Value::Jsonb(pglet_core::types::parse_jsonb(text).unwrap());
            assert_eq!(stored(&j), size, "{text}");
        }
        // The one-byte header holds 126 data bytes: a string of 118 is 127 stored, of
        // 119 is 131.
        let string = |n: usize| Value::Jsonb(JsonbValue::String("x".repeat(n)));
        assert_eq!(stored(&string(118)), 127);
        assert_eq!(stored(&string(119)), 131);
        // The B-tree boundary over an incompressible string: 2 684 characters accepted,
        // 2 685 refused as 2 712; and `index_form_tuple`'s at 8 173, as 8 200.
        let size = |n| index_entry_size([(&string(n), false)]).unwrap();
        assert_eq!(size(2684), 2704);
        assert_eq!(size(2685), 2712);
        assert_eq!(size(8173), 8200);
    }

    /// Each size is 18.6's `pg_column_size` of the stored array, measured.
    #[test]
    fn an_array_is_sized_as_its_header_bitmap_and_aligned_elements() {
        use pglet_core::types::{ArrayValue, TypeOid};
        let array = |ty, elements: Vec<Value>| Value::Array(ArrayValue::new(ty, elements));
        let ints = |elements: &[Option<i32>]| {
            array(
                TypeOid::Int4,
                elements
                    .iter()
                    .map(|e| e.map_or(Value::Null, Value::Int4))
                    .collect(),
            )
        };
        let run = |n: i32| (1..=n).map(Some).collect::<Vec<_>>();
        let with_null = |n: i32| run(n).into_iter().chain([None]).collect::<Vec<_>>();
        assert_eq!(stored(&ints(&[])), 13);
        assert_eq!(stored(&ints(&[Some(1)])), 25);
        assert_eq!(stored(&ints(&[Some(1), Some(2)])), 29);
        // A NULL element takes a bitmap byte per eight elements and no data.
        assert_eq!(stored(&ints(&[Some(1), None])), 33);
        assert_eq!(stored(&ints(&[None])), 29);
        assert_eq!(stored(&ints(&with_null(7))), 57);
        assert_eq!(stored(&ints(&with_null(8))), 61);
        assert_eq!(stored(&ints(&[None; 33])), 29);
        // The one-byte header's edge: 26 elements are 124 data bytes, 27 are 128.
        assert_eq!(stored(&ints(&run(26))), 125);
        assert_eq!(stored(&ints(&run(27))), 132);
        // Fixed-length elements at their own alignment.
        let (i2, i8) = (TypeOid::Int2, TypeOid::Int8);
        assert_eq!(stored(&array(i2, vec![Value::Int2(1); 3])), 27);
        assert_eq!(
            stored(&array(
                i2,
                vec![Value::Int2(1), Value::Null, Value::Int2(3)]
            )),
            33
        );
        assert_eq!(stored(&array(i8, vec![Value::Int8(1); 2])), 37);
        assert_eq!(stored(&array(i8, vec![Value::Null, Value::Int8(2)])), 37);
        assert_eq!(
            stored(&array(TypeOid::Bool, vec![Value::Bool(true); 3])),
            24
        );
        assert_eq!(
            stored(&array(TypeOid::Uuid, vec![Value::Uuid([1; 16]); 2])),
            53
        );
        let day = Value::Interval(pglet_core::types::PgInterval {
            months: 0,
            days: 1,
            micros: 0,
        });
        assert_eq!(stored(&array(TypeOid::Interval, vec![day; 2])), 53);
        // A varlena element keeps a four-byte header and is padded to four after.
        let texts = |items: &[Option<&str>]| {
            array(
                TypeOid::Text,
                items
                    .iter()
                    .map(|e| e.map_or(Value::Null, |s| Value::Text(s.to_string())))
                    .collect(),
            )
        };
        assert_eq!(stored(&texts(&[Some("")])), 25);
        assert_eq!(stored(&texts(&[Some("a")])), 29);
        assert_eq!(stored(&texts(&[Some("abcd")])), 29);
        assert_eq!(stored(&texts(&[Some("abcde")])), 33);
        assert_eq!(stored(&texts(&[Some("abcde"), Some("f")])), 41);
        assert_eq!(stored(&texts(&[Some("a"), None, Some("b")])), 45);
        assert_eq!(stored(&texts(&[Some(&"x".repeat(100))])), 125);
        assert_eq!(stored(&texts(&[Some(&"x".repeat(126))])), 156);
        assert_eq!(stored(&texts(&[Some(&"x".repeat(130)), Some("a")])), 168);
        let numerics = |items: &[&str]| {
            array(
                TypeOid::Numeric,
                items
                    .iter()
                    .map(|n| Value::Numeric(PgNumeric::parse(n).unwrap()))
                    .collect(),
            )
        };
        assert_eq!(stored(&numerics(&["0"])), 29);
        assert_eq!(stored(&numerics(&["1.5"])), 33);
        assert_eq!(stored(&numerics(&["1", "2"])), 37);
        assert_eq!(stored(&numerics(&["123456789.123"])), 37);
        // A `jsonb` element is its own container under a four-byte header.
        let jsonbs = |items: &[&str]| {
            array(
                TypeOid::Jsonb,
                items
                    .iter()
                    .map(|j| Value::Jsonb(pglet_core::types::parse_jsonb(j).unwrap()))
                    .collect(),
            )
        };
        assert_eq!(stored(&jsonbs(&["1"])), 41);
        assert_eq!(stored(&jsonbs(&["null"])), 33);
        assert_eq!(stored(&jsonbs(&[r#""a""#, r#"{"a":1}"#])), 65);
        // The B-tree boundary over incompressible `int4`s: 668 accepted, 669 refused
        // as 2 712; and `index_form_tuple`'s at 2 040, as 8 192.
        let size = |n| index_entry_size([(&ints(&run(n)), false)]).unwrap();
        assert_eq!(size(668), 2704);
        assert_eq!(size(669), 2712);
        assert_eq!(size(2040), 8192);
    }
}
