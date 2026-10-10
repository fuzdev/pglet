//! B+tree key type for sorted storage.
//!
//! Keys live in a single keyspace that holds three kinds of entries under a
//! reserved leading discriminant byte:
//! - **Data rows** — `(table_id, row_id)`, the primary table storage.
//! - **Index entries** — `(index_id, encoded_value, row_id)` with an empty
//!   stored value, the secondary-index entries. The value is framed with an
//!   order-preserving, escape-terminated byte-string encoding so byte order
//!   equals value order regardless of value length (range-scan ready).
//! - **System entries** — `(sub-kind, id)`, the catalog/table-meta, kept in the tree
//!   rather than the meta slot. A `0x02 0x00` catalog chunk holds a slice
//!   of the postcard `Schema` blob;
//!   a `0x02 0x01` table-meta entry holds `(name, next_row_id)`. They ride the
//!   same tree/root/commit as the data they describe (atomic, MVCC for free),
//!   lifting the fixed-meta-slot size ceiling.
//!
//! All three kinds share one tree (one `root`), so the existing MVCC seqlock
//! covers their state with no new roots or snapshot machinery.
//!
//! Unlike LSM keys, B+tree keys don't include version - MVCC is handled
//! by root versioning (old roots = old snapshots).

/// B+tree key: a variable-length, byte-comparable key over a single keyspace.
///
/// # Encoding scheme
///
/// Every key encodes to a leading **discriminant byte** followed by a
/// kind-specific big-endian body. Big-endian fields make byte-wise comparison
/// match numeric ordering, and the discriminant keeps the two kinds disjoint
/// (all data keys sort before all index keys) so they never collide:
///
/// ```text
/// Data row  (discriminant 0x00):
///   [0x00] [table_id: u32 BE] [row_id: u64 BE]                     // 13 bytes
///
/// Index entry (discriminant 0x01):
///   [0x01] [index_id: u32 BE] [escaped value … 0x00 0x00] [row_id: u64 BE]
///
/// System entry (discriminant 0x02):
///   [0x02] [sub-kind: u8] [id: u32 BE]                             // 6 bytes
///   sub-kind 0x00 = catalog chunk (id = chunk index),
///            0x01 = table meta    (id = table_id)
/// ```
///
/// System keys sort after all data and index keys (discriminant `0x02`), and
/// within the system band catalog chunks (sub-kind `0x00`) sort before table
/// meta (`0x01`); both order by `id` big-endian.
///
/// The index value is framed with an **order-preserving, escape-terminated**
/// byte-string encoding (the FoundationDB / CockroachDB scheme): each `0x00`
/// byte in the value becomes `0x00 0xFF`, and the whole value is terminated by
/// `0x00 0x00`. Because no escaped value can contain a bare `0x00 0x00`, the
/// terminator is unambiguous, and because `0xFF > 0x00` a value that is a
/// strict prefix of another (its terminator `0x00 0x00` compares against the
/// longer value's next escaped byte) sorts first — exactly lexicographic
/// order, regardless of length. `row_id` follows the terminator, so equal
/// values order by `row_id`. This replaces the old `[value_len][value]`
/// framing, whose mid-key length prefix made byte order disagree with value
/// order across differing lengths (e.g. `"b"` sorted before `"ab"`), which
/// cannot support ordered/range scans.
///
/// **Value contract:** the bytes handed to [`index`](Self::index) are treated
/// as **already in an order-preserving form for their type** — byte order over
/// them must equal logical value order. Producing that form (e.g. big-endian
/// signed-int munging, collation-aware text keys) is the SQL layer's job;
/// raw bytes for `uuid`/`text`/`bytea` are already order-preserving under
/// pglet's binary compare. This type owns only the *framing*, which preserves
/// whatever order the value bytes already carry.
///
/// The hand-written [`Ord`] reproduces exactly the lexicographic order of
/// [`encode`](Self::encode), so in-memory `cmp` and on-disk byte order agree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BtreeKey {
    /// A primary data row: `(table_id, row_id)`.
    Data { table_id: u32, row_id: u64 },
    /// A secondary-index entry: `(index_id, encoded value, row_id)`.
    Index {
        index_id: u32,
        value: Vec<u8>,
        row_id: u64,
    },
    /// A system entry: catalog chunk or table meta. The `id` is a chunk index
    /// (`SysKind::Catalog`) or a `table_id` (`SysKind::Table`). The stored value
    /// (a `Vec<Value>` carrying one `Bytea`) holds the chunk/encoded-meta bytes.
    System { kind: SysKind, id: u32 },
}

/// Which kind of system entry a [`BtreeKey::System`] addresses. The byte value
/// is the on-disk sub-kind; `Catalog` (`0x00`) sorts before `Table` (`0x01`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SysKind {
    /// A chunk of the postcard `Schema` blob (`id` = chunk index, from 0).
    Catalog,
    /// One table's meta — `(name, next_row_id)` (`id` = `table_id`).
    Table,
}

/// Discriminant byte for a data-row key.
const TAG_DATA: u8 = 0x00;
/// Discriminant byte for an index-entry key.
const TAG_INDEX: u8 = 0x01;
/// Discriminant byte for a system-entry key.
const TAG_SYSTEM: u8 = 0x02;
/// System sub-kind byte: a catalog chunk.
const SUB_CATALOG: u8 = 0x00;
/// System sub-kind byte: a table-meta entry.
const SUB_TABLE: u8 = 0x01;

impl SysKind {
    /// The on-disk sub-kind byte.
    const fn to_byte(self) -> u8 {
        match self {
            Self::Catalog => SUB_CATALOG,
            Self::Table => SUB_TABLE,
        }
    }

    /// Decode a sub-kind byte. Only `0x00`/`0x01` are written; any other byte
    /// (a corrupt/fuzzed page that slipped the CRC) maps to `Table` so decode
    /// stays total and panic-free — such a key matches no real band.
    const fn from_byte(b: u8) -> Self {
        if b == SUB_CATALOG {
            Self::Catalog
        } else {
            Self::Table
        }
    }
}

/// Escape byte for the order-preserving index-value framing. A literal `0x00`
/// in the value is emitted as `ESC ESC_BYTE` (`0x00 0xFF`); the value is
/// terminated by `ESC TERM` (`0x00 0x00`).
const ESC: u8 = 0x00;
/// Follows [`ESC`] to encode a literal `0x00` value byte.
const ESC_BYTE: u8 = 0xFF;
/// Follows [`ESC`] to mark the end of the escaped value.
const TERM: u8 = 0x00;

/// Append the order-preserving, escape-terminated framing of `value` to `out`.
///
/// Each `0x00` byte becomes `0x00 0xFF`; a `0x00 0x00` terminator closes the
/// value. See the [`BtreeKey`] type docs for why this is order-preserving.
fn escape_value_into(value: &[u8], out: &mut Vec<u8>) {
    for &b in value {
        if b == ESC {
            out.push(ESC);
            out.push(ESC_BYTE);
        } else {
            out.push(b);
        }
    }
    out.push(ESC);
    out.push(TERM);
}

/// Encoded length of `value` under the escape-terminated framing: each `0x00`
/// byte costs two output bytes, every other byte one, plus the two-byte
/// terminator.
#[inline]
fn escaped_value_len(value: &[u8]) -> usize {
    value
        .iter()
        .map(|&b| if b == ESC { 2 } else { 1 })
        .sum::<usize>()
        + 2
}

/// Decode an escape-terminated value starting at `bytes[0]`, returning the
/// unescaped value and the number of bytes consumed (including the terminator).
///
/// # Panics
///
/// Panics if no `0x00 0x00` terminator is found before the end of `bytes`, or
/// `bytes` ends mid-escape. Callers decoding a key we wrote ourselves never hit
/// this; [`BtreeKey::try_decode`] guards untrusted/truncated input first.
// Mirrors `decode`'s contract: a well-formed key (the only thing we decode in
// the hot path) is always terminated, so the `panic!` is unreachable there and
// the `bytes[i + 1]` index is in bounds; truncated input goes through
// `try_decode`/`escaped_value_end` instead.
#[allow(clippy::indexing_slicing, clippy::panic)]
fn unescape_value(bytes: &[u8]) -> (Vec<u8>, usize) {
    let mut value = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == ESC {
            // ESC is always followed by ESC_BYTE (literal 0x00) or TERM (end).
            match bytes[i + 1] {
                ESC_BYTE => {
                    value.push(0x00);
                    i += 2;
                }
                // TERM (0x00): end of value. `i + 2` bytes consumed total.
                _ => return (value, i + 2),
            }
        } else {
            value.push(bytes[i]);
            i += 1;
        }
    }
    // Reachable only on a malformed buffer (no terminator); see the doc panic.
    panic!("unescape_value: missing 0x00 0x00 terminator");
}

/// Build the raw (pre-`encode_into`-framing) index value for a composite key
/// from its per-column order-preserving segments.
///
/// The whole returned blob is what [`BtreeKey::index`] takes as `encoded_value`
/// — it is framed exactly once by [`encode_into`](BtreeKey::encode_into), so the
/// blob itself must already be self-delimiting *between segments* and
/// order-preserving across tuples.
///
/// - **One segment** → the segment bytes verbatim, so a one-column composite key
///   is **byte-identical** to the single-column [`index`](BtreeKey::index) form
///   (the outer framing then matches today's on-disk entries exactly).
/// - **Many segments** → each segment is escape-terminated (`frame(s1) ++ frame(s2)
///   ++ …`) before the outer framing. Per-segment escape-termination makes the
///   segment boundaries unambiguous, so `(a, bc)` ≠ `(ab, c)`, and because each
///   `frame(s)` is itself order-preserving and self-delimiting, byte order over the
///   concatenation equals tuple order: `(a, b) < (a, c) < (b, a)`.
///
/// The blob is never decoded back into columns (index keys yield only `row_id`),
/// so this one-way framing has no inverse to maintain.
pub fn encode_composite_index_key(segments: &[&[u8]]) -> Vec<u8> {
    // One column: byte-identical to the single-column path (no inner framing), so
    // existing single-column on-disk entries stay valid.
    if let [only] = segments {
        return only.to_vec();
    }
    // Many columns: per-segment escape-termination makes boundaries unambiguous.
    let mut out = Vec::new();
    for seg in segments {
        escape_value_into(seg, &mut out);
    }
    out
}

/// Find the byte offset just past the `0x00 0x00` terminator of an
/// escape-terminated value beginning at `bytes[0]`, or `None` if the buffer
/// ends before a terminator is seen. Used to size a truncated index key.
fn escaped_value_end(bytes: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == ESC {
            match bytes.get(i + 1)? {
                &ESC_BYTE => i += 2,
                // TERM: terminator at i..i+2.
                _ => return Some(i + 2),
            }
        } else {
            i += 1;
        }
    }
    None
}

impl BtreeKey {
    /// Encoded size of a data-row key: `tag(1) + table_id(4) + row_id(8)`.
    pub const DATA_SIZE: usize = 1 + 4 + 8;

    /// Create a new data-row key.
    #[inline]
    pub const fn new(table_id: u32, row_id: u64) -> Self {
        Self::Data { table_id, row_id }
    }

    /// Create a new secondary-index key.
    ///
    /// `encoded_value` is the indexed column value **already in an
    /// order-preserving form for its type** (byte order == value order). This
    /// type frames it with an order-preserving terminator, so equality probes
    /// and ordered/range scans both work; the SQL layer owns producing the
    /// per-type order-preserving bytes.
    #[inline]
    pub fn index(index_id: u32, encoded_value: &[u8], row_id: u64) -> Self {
        Self::Index {
            index_id,
            value: encoded_value.to_vec(),
            row_id,
        }
    }

    /// Create a secondary-index key from per-column order-preserving `segments`.
    ///
    /// The composite counterpart to [`index`](Self::index): the segments are
    /// folded into one order-preserving value blob via
    /// [`encode_composite_index_key`], which a **one-element** `segments` reduces
    /// to the identical bytes [`index`](Self::index) would produce — so a
    /// one-column composite index is byte-for-byte the single-column form, and
    /// the same full-key equality-probe bounds (`index_start`/`index_end`,
    /// `index_scan_at_root`'s `enc ++ 0x00` exclusive upper) apply unchanged.
    #[inline]
    pub fn index_composite(index_id: u32, segments: &[&[u8]], row_id: u64) -> Self {
        Self::index(index_id, &encode_composite_index_key(segments), row_id)
    }

    /// Encoded size of a system-entry key: `tag(1) + sub-kind(1) + id(4)`.
    pub const SYSTEM_SIZE: usize = 1 + 1 + 4;

    /// Create a catalog-chunk system key (`chunk` = chunk index, from 0).
    #[inline]
    pub const fn sys_catalog(chunk: u32) -> Self {
        Self::System {
            kind: SysKind::Catalog,
            id: chunk,
        }
    }

    /// Create a table-meta system key (`table_id`).
    #[inline]
    pub const fn sys_table(table_id: u32) -> Self {
        Self::System {
            kind: SysKind::Table,
            id: table_id,
        }
    }

    /// Decompose a system key into `(sub-kind, id)`, or `None` for data/index.
    #[inline]
    pub const fn as_system(&self) -> Option<(SysKind, u32)> {
        match self {
            Self::System { kind, id } => Some((*kind, *id)),
            _ => None,
        }
    }

    /// Inclusive lower bound of the catalog-chunk band (chunk 0). The exclusive
    /// upper is [`sys_table_start`](Self::sys_table_start) — table meta sorts
    /// strictly after every catalog chunk — so `[sys_catalog_start ..
    /// sys_table_start)` brackets exactly the catalog chunks.
    #[inline]
    pub const fn sys_catalog_start() -> Self {
        Self::sys_catalog(0)
    }

    /// Inclusive lower bound of the table-meta band (`table_id` 0); also the
    /// exclusive upper of the catalog band.
    #[inline]
    pub const fn sys_table_start() -> Self {
        Self::sys_table(0)
    }

    /// Exclusive upper bound of the table-meta band: `sys_table(next_table_id)`.
    /// Every live `table_id` is `< next_table_id` (ids only ever increment), so
    /// `[sys_table_start .. sys_table_end(next_table_id))` brackets exactly the
    /// table-meta entries.
    #[inline]
    pub const fn sys_table_end(next_table_id: u32) -> Self {
        Self::sys_table(next_table_id)
    }

    /// Get the table ID, or `None` for an index/system key.
    #[inline]
    pub const fn table_id(&self) -> Option<u32> {
        match self {
            Self::Data { table_id, .. } => Some(*table_id),
            Self::Index { .. } | Self::System { .. } => None,
        }
    }

    /// Get the row ID. Data and index keys carry one; a system key has none, so
    /// this returns 0 (system keys never flow through row-id-consuming paths).
    #[inline]
    pub const fn row_id(&self) -> u64 {
        match self {
            Self::Data { row_id, .. } | Self::Index { row_id, .. } => *row_id,
            Self::System { .. } => 0,
        }
    }

    /// The encoded length of an index-entry key over `encoded_value`, whatever its
    /// index id and row id — the length [`Self::index`] would produce, counted without
    /// building the key.
    #[inline]
    #[must_use]
    pub fn index_encoded_len(encoded_value: &[u8]) -> usize {
        1 + 4 + escaped_value_len(encoded_value) + 8
    }

    /// Encoded length in bytes (without allocating).
    #[inline]
    pub fn encoded_len(&self) -> usize {
        match self {
            Self::Data { .. } => Self::DATA_SIZE,
            // tag(1) + index_id(4) + escaped value (incl. 0x00 0x00 term) + row_id(8)
            Self::Index { value, .. } => Self::index_encoded_len(value),
            Self::System { .. } => Self::SYSTEM_SIZE,
        }
    }

    /// Encode into a caller-provided buffer (no allocation in the hot path).
    ///
    /// Appends this key's bytes to `out`; see the type docs for the layout.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Self::Data { table_id, row_id } => {
                out.push(TAG_DATA);
                out.extend_from_slice(&table_id.to_be_bytes());
                out.extend_from_slice(&row_id.to_be_bytes());
            }
            Self::Index {
                index_id,
                value,
                row_id,
            } => {
                out.push(TAG_INDEX);
                out.extend_from_slice(&index_id.to_be_bytes());
                // Order-preserving escape-terminated framing (0x00 -> 0x00 0xFF,
                // terminated by 0x00 0x00), then the row_id.
                escape_value_into(value, out);
                out.extend_from_slice(&row_id.to_be_bytes());
            }
            Self::System { kind, id } => {
                out.push(TAG_SYSTEM);
                out.push(kind.to_byte());
                out.extend_from_slice(&id.to_be_bytes());
            }
        }
    }

    /// Encode to a freshly allocated byte vector.
    ///
    /// Prefer [`encode_into`](Self::encode_into) on the codec hot path to avoid
    /// a per-key allocation.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.encoded_len());
        self.encode_into(&mut out);
        out
    }

    /// Decode a key from the start of `bytes`, returning the key and the number
    /// of bytes consumed.
    ///
    /// # Panics
    ///
    /// Panics if `bytes` is shorter than the encoded key. Callers that may have
    /// a truncated buffer should use [`try_decode`](Self::try_decode).
    // Fixed-layout decode: the `try_into` targets are statically-sized slices of
    // a length the caller guarantees (a well-formed page we wrote), so they
    // never fail.
    #[allow(clippy::unwrap_used)]
    pub fn decode(bytes: &[u8]) -> (Self, usize) {
        if bytes[0] == TAG_INDEX {
            let index_id = u32::from_be_bytes(bytes[1..5].try_into().unwrap());
            // Unescape the value up to (and including) the 0x00 0x00 terminator.
            let (value, value_consumed) = unescape_value(&bytes[5..]);
            let row_off = 5 + value_consumed;
            let row_id = u64::from_be_bytes(bytes[row_off..row_off + 8].try_into().unwrap());
            (
                Self::Index {
                    index_id,
                    value,
                    row_id,
                },
                row_off + 8,
            )
        } else if bytes[0] == TAG_SYSTEM {
            // [0x02][sub-kind: u8][id: u32 BE] — fixed 6 bytes.
            let kind = SysKind::from_byte(bytes[1]);
            let id = u32::from_be_bytes(bytes[2..6].try_into().unwrap());
            (Self::System { kind, id }, Self::SYSTEM_SIZE)
        } else {
            // TAG_DATA (and any other byte, treated as data for a fixed-stride decode)
            let table_id = u32::from_be_bytes(bytes[1..5].try_into().unwrap());
            let row_id = u64::from_be_bytes(bytes[5..13].try_into().unwrap());
            (Self::Data { table_id, row_id }, Self::DATA_SIZE)
        }
    }

    /// Try to decode a key from the start of `bytes`, returning `None` if the
    /// buffer is too short to hold the full encoding.
    pub fn try_decode(bytes: &[u8]) -> Option<(Self, usize)> {
        // Determine how many bytes the full encoding needs, bailing if the
        // buffer can't even hold the header or the value's terminator.
        let tag = *bytes.first()?;
        let needed = if tag == TAG_INDEX {
            // tag(1) + index_id(4), then escape-terminated value, then row_id(8).
            if bytes.len() < 5 {
                return None;
            }
            // Offset just past the 0x00 0x00 terminator; None if it's truncated.
            let value_end = 5 + escaped_value_end(&bytes[5..])?;
            value_end + 8
        } else if tag == TAG_SYSTEM {
            Self::SYSTEM_SIZE
        } else {
            Self::DATA_SIZE
        };
        if bytes.len() < needed {
            return None;
        }
        Some(Self::decode(bytes))
    }

    /// Create a key for scanning a table from the beginning.
    #[inline]
    pub const fn table_start(table_id: u32) -> Self {
        Self::new(table_id, 0)
    }

    /// Create a key for scanning a table to the end (exclusive upper bound).
    /// Uses next `table_id` with `row_id=0`.
    #[inline]
    pub const fn table_end(table_id: u32) -> Self {
        Self::new(table_id + 1, 0)
    }

    /// Check if this key is a data row within a table's range.
    #[inline]
    pub const fn is_in_table(&self, table_id: u32) -> bool {
        matches!(self, Self::Data { table_id: t, .. } if *t == table_id)
    }

    /// Lower bound (inclusive) for probing an index value: `row_id = 0`.
    #[inline]
    pub fn index_start(index_id: u32, encoded_value: &[u8]) -> Self {
        Self::index(index_id, encoded_value, 0)
    }

    /// Upper bound (inclusive) for probing an index value: `row_id = u64::MAX`.
    ///
    /// A probe of `value` scans `[index_start(id, value) ..= index_end(id, value)]`.
    #[inline]
    pub fn index_end(index_id: u32, encoded_value: &[u8]) -> Self {
        Self::index(index_id, encoded_value, u64::MAX)
    }

    /// Lower bound (inclusive) for **all** entries of one index: the empty-value
    /// start of `index_id`'s band.
    ///
    /// The index counterpart to [`table_start`](Self::table_start). Sorts at or
    /// below every `(index_id, value, row_id)` entry (an empty value with
    /// `row_id = 0` is the least key in the band), so `[index_table_start(id) ..
    /// index_table_end(id))` brackets exactly `index_id`'s entries — the walk that
    /// wants every entry, a NULL key's among them, as dropping a band does. A value
    /// range does not start here: its start is
    /// [`index_range_start`](Self::index_range_start) of a blob its caller chose.
    #[inline]
    pub fn index_table_start(index_id: u32) -> Self {
        Self::index_start(index_id, &[])
    }

    /// Upper bound (exclusive) for **all** entries of one index: the empty-value
    /// start of the *next* index id.
    ///
    /// The index counterpart to [`table_end`](Self::table_end). Because index ids
    /// are a contiguous big-endian field, the empty-value start of `index_id + 1`
    /// sorts strictly after every entry of `index_id` (and `saturating_add` keeps
    /// the top id from wrapping), so it is the half-open exclusive upper for the
    /// whole band — the end of a range scan with no upper bound.
    #[inline]
    pub fn index_table_end(index_id: u32) -> Self {
        Self::index_start(index_id.saturating_add(1), &[])
    }

    /// The key just past every `(index_id, enc, row_id)` entry: `index(id, enc ++
    /// 0x00, 0)`.
    ///
    /// Under the framing a value is a strict prefix of `value ++ any_byte`, so this
    /// sorts strictly after *every* entry of `enc` (including `row_id = u64::MAX`)
    /// yet at or below every entry of a greater value — the same exclusive boundary
    /// an equality probe ends at.
    fn index_past(index_id: u32, enc: &[u8]) -> Self {
        let mut v = Vec::with_capacity(enc.len() + 1);
        v.extend_from_slice(enc);
        v.push(0x00);
        Self::index(index_id, &v, 0)
    }

    /// The inclusive start of a range scan whose lower end is the key blob `enc`:
    /// `col >= x` (`inclusive`) or `col > x`.
    ///
    /// `enc` is a key blob in order-preserving form (`index_keying`'s
    /// `encode_range_bounds` produces it). The scan runs over the half-open `[start,
    /// end)`, so an inclusive lower end starts at `enc`'s first entry and an exclusive
    /// one just past its last:
    ///
    /// - `>= x` → `index_start(id, enc)`
    /// - `> x`  → `index(id, enc ++ 0x00, 0)`
    ///
    /// There is no start for "no lower end" here: which keys a range with no lower
    /// bound holds is the caller's decision, made as a blob.
    pub fn index_range_start(index_id: u32, enc: &[u8], inclusive: bool) -> Self {
        if inclusive {
            Self::index_start(index_id, enc)
        } else {
            Self::index_past(index_id, enc)
        }
    }

    /// The exclusive end of a range scan whose upper end is the key blob `enc`:
    /// `col <= x` (`inclusive`) or `col < x`.
    ///
    /// The counterpart to [`index_range_start`](Self::index_range_start):
    ///
    /// - `< x`  → `index_start(id, enc)`
    /// - `<= x` → `index(id, enc ++ 0x00, 0)`
    ///
    /// A range with no upper end runs to [`index_table_end`](Self::index_table_end).
    /// An end at or below the start is an empty range, which `range_scan_at_root`
    /// yields naturally.
    pub fn index_range_end(index_id: u32, enc: &[u8], inclusive: bool) -> Self {
        if inclusive {
            Self::index_past(index_id, enc)
        } else {
            Self::index_start(index_id, enc)
        }
    }
}

impl Ord for BtreeKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self, other) {
            // Same kind: compare bodies in the same order encode() lays them out.
            (
                Self::Data {
                    table_id: a_t,
                    row_id: a_r,
                },
                Self::Data {
                    table_id: b_t,
                    row_id: b_r,
                },
            ) => a_t.cmp(b_t).then_with(|| a_r.cmp(b_r)),
            (
                Self::Index {
                    index_id: a_i,
                    value: a_v,
                    row_id: a_r,
                },
                Self::Index {
                    index_id: b_i,
                    value: b_v,
                    row_id: b_r,
                },
            ) => a_i
                .cmp(b_i)
                // Plain lexicographic value compare: the escape-terminated
                // framing makes encoded byte order equal value order across
                // lengths (a strict prefix sorts first via its 0x00 0x00
                // terminator vs. 0xFF/value byte), and row_id breaks ties so
                // probes over equal values stay contiguous.
                .then_with(|| a_v.cmp(b_v))
                .then_with(|| a_r.cmp(b_r)),
            (
                Self::System {
                    kind: a_k,
                    id: a_id,
                },
                Self::System {
                    kind: b_k,
                    id: b_id,
                },
            ) => a_k
                .to_byte()
                .cmp(&b_k.to_byte())
                .then_with(|| a_id.cmp(b_id)),
            // Cross-kind: discriminant byte decides (data < index < system).
            (Self::Data { .. }, _) | (Self::Index { .. }, Self::System { .. }) => Ordering::Less,
            (_, Self::Data { .. }) | (Self::System { .. }, Self::Index { .. }) => Ordering::Greater,
        }
    }
}

impl PartialOrd for BtreeKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests;
