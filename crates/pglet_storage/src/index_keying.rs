//! Shared secondary-index key computation, predicate evaluation, and probe
//! encoding — the single point of truth both storage backends key through.
//!
//! The B+tree wrapper and the in-memory backend hold near-identical index logic:
//! whether a row participates in a (partial) index, the key tuple a row occupies,
//! and the order-preserving composite blob a key tuple folds to. Keeping two
//! copies is a standing drift risk — a fix to one (e.g. the NULL-vs-empty
//! collision below) must land in both, and a divergence can pass tests because
//! the spine runs on the B+tree while many unit tests run on memory. So the
//! shared half lives here in one [`IndexKeying`], which each backend's `IndexDef`
//! embeds alongside its backend-specific fields (`table_id` for the wrapper,
//! `entries` for memory).
//!
//! # Composite-key encoding (NULL-distinguishing)
//!
//! A key tuple folds to one composite blob by tagging each segment and framing
//! the tagged segments through [`encode_composite_index_key`]:
//!
//! - a **present** value → `0x01 ++ encode_index_key(value)`,
//! - a **NULL** segment → `0x00` (a one-byte tag, no payload).
//!
//! The per-segment presence tag is what keeps a NULL key distinct from a present
//! empty value. Without it, `Null`, `Text("")`, and `Bytea([])` all encode to the
//! empty byte string (see `Value::encode_index_key`), so under `NULLS NOT
//! DISTINCT` a NULL-keyed row and an empty-string-keyed row would fold to the
//! same blob — a false uniqueness conflict and a probe that returns the wrong
//! row. Postgres treats NULL and `''` as distinct; the tag restores that.
//!
//! The tag is order-preserving: `0x00 < 0x01`, so a NULL segment sorts before any
//! present value, and within present values the constant `0x01` prefix leaves the
//! underlying `encode_index_key` order intact. The single-segment fast path in
//! [`encode_composite_index_key`] stays coherent — one tagged segment is still
//! one segment, framed verbatim — so the `index_scan_at_root` equality bounds
//! (`enc ++ 0x00` exclusive upper) apply unchanged.

use crate::btree::{BtreeKey, MAX_KEY_LEN, encode_composite_index_key};
use crate::{Result, RowId, StorageError};
use pglet_sql::executor::index_row::{
    check_btree_item_size, check_entry_size_field, check_gin_entries, index_entry_size,
    key_part_is_enum,
};
use pglet_sql::executor::{IndexMethod, IndexSpec, eval_expr, predicate_holds};
use pglet_sql::gin::gin_decompose;
use pglet_sql::planner::{BoolContext, Expr, deserialize_expr};
use pglet_sql::types::{TypeOid, Value};

/// Presence tag for a NULL key segment (sorts before any present value).
const SEG_NULL: u8 = 0x00;
/// Presence tag for a present (non-NULL) key segment.
const SEG_PRESENT: u8 = 0x01;

/// The backend-independent half of an index definition.
///
/// Carries everything needed to decide a row's membership, compute its key tuple,
/// and fold that tuple into the on-disk/probe composite blob. Both backends'
/// `IndexDef`s embed one of these.
#[derive(Debug, Clone)]
pub struct IndexKeying {
    /// The index's name — what a size refusal names ([`Self::encode_row_entries`]).
    pub name: String,
    /// The indexed table's name — a B-tree size refusal's `t`.
    pub table: String,
    /// Each key part's type is an enum — the one fact about an entry's 18.6 size a
    /// `Value::Text` does not carry (`pglet_sql::executor::index_row`). Parallel to
    /// the key parts: `key_exprs` for an expression index, else `key_col_positions`.
    pub part_is_enum: Vec<bool>,
    /// Indexed column positions in row order (one for single-column, N for
    /// composite). Used only for a **plain** index (`key_exprs` is `None`); empty
    /// for an expression index.
    pub key_col_positions: Vec<usize>,
    /// For an **expression** index: each key part parsed once and cached,
    /// evaluated per row via `eval_expr` against `column_names` (`None` for a plain
    /// index, which keys via `key_col_positions`). A bare-column part is an
    /// `Expr::Column`, so the same eval path covers mixed bare/expression keys.
    pub key_exprs: Option<Vec<Expr>>,
    /// The partial index `WHERE` predicate, parsed once and cached (`None` for a
    /// full index). A row is in the index only when this **holds** at
    /// `BoolContext::Where` ([`Self::row_in_index`]): `Value::Bool(true)`, or a
    /// `Value::Text` the boolean input function reads as true — which is what an
    /// unresolved literal predicate indexes by, pinned as
    /// `bool_context/partial_index_predicate_literal_indexes_rows.sql`. NULL and
    /// `false` leave the row out, and any other type is that context's `42804`.
    pub predicate: Option<Expr>,
    /// `NULLS NOT DISTINCT`: when `true`, NULL key columns ARE indexed (two NULL
    /// keys conflict). When `false` (default), a NULL in any key column means the
    /// row is not indexed.
    pub nulls_not_distinct: bool,
    /// The indexed table's column names in row order — the `col_names` the
    /// predicate and key-expression evaluators (`eval_expr`) bind against.
    pub column_names: Vec<String>,
    /// The indexed table's declared column types, positionally parallel to
    /// `column_names` — see `IndexSpec::column_types`. Without them a stored
    /// expression that stringifies a `timestamptz` would index a different string
    /// than the query computes.
    pub column_types: Vec<TypeOid>,
    /// The indexed table's enum member orders, positionally parallel to
    /// `column_names` — see `IndexSpec::column_enum_orders`. Without them a stored
    /// predicate's `m > 'sad'` would decide membership by the label bytes while every
    /// comparison beside it decides by the member position.
    pub column_enum_orders: Vec<Option<Vec<String>>>,
    /// The access method. `Btree` keys a row to **one** composite blob; `Gin`
    /// decomposes the (single) key value into **many** element/key blobs — see
    /// [`encode_row_keys`](Self::encode_row_keys).
    pub method: IndexMethod,
}

impl IndexKeying {
    /// Build an `IndexKeying` from an `IndexSpec`, deserializing + caching the
    /// partial predicate and any expression key parts.
    ///
    /// # Errors
    ///
    /// Returns `Unindexable` if the partial predicate or a key expression fails to
    /// deserialize (should not happen for forms captured at `CREATE INDEX`).
    pub fn from_spec(spec: &IndexSpec) -> Result<Self> {
        let key_exprs = deserialize_index_key_parts(spec.key_part_ir.as_deref())?;
        let enum_column = |name: &str| {
            spec.column_names
                .iter()
                .position(|c| c == name)
                .is_some_and(|i| spec.column_enum_orders.get(i).is_some_and(Option::is_some))
        };
        let part_is_enum = key_exprs.as_ref().map_or_else(
            || {
                spec.key_col_positions
                    .iter()
                    .map(|&pos| {
                        spec.column_enum_orders
                            .get(pos)
                            .is_some_and(Option::is_some)
                    })
                    .collect()
            },
            |exprs| {
                exprs
                    .iter()
                    .map(|e| key_part_is_enum(e, &enum_column))
                    .collect()
            },
        );
        Ok(Self {
            name: spec.name.clone(),
            table: spec.table.clone(),
            part_is_enum,
            key_col_positions: spec.key_col_positions.clone(),
            key_exprs,
            predicate: deserialize_index_predicate(spec.predicate_ir.as_deref())?,
            nulls_not_distinct: spec.nulls_not_distinct,
            column_names: spec.column_names.clone(),
            column_types: spec.column_types.clone(),
            column_enum_orders: spec.column_enum_orders.clone(),
            method: spec.method,
        })
    }

    /// The borrowed form of [`Self::column_enum_orders`] the evaluator takes.
    ///
    /// The spec owns its label lists — `pglet_storage` holds no catalog to borrow them
    /// from — so this is the one place they are re-borrowed, once per evaluation.
    fn enum_orders(&self) -> Vec<Option<&[String]>> {
        self.column_enum_orders
            .iter()
            .map(|o| o.as_deref())
            .collect()
    }

    /// Whether `row` participates in this index (its partial predicate, if any, holds
    /// for the row). A full index always participates.
    ///
    /// The predicate is resolved through `pglet_sql`'s one boolean-context resolver,
    /// which is also what `executor::constraints`'s `prepared_unique_index_key` reads
    /// the **same** predicate over the **same** row with, one crate up. Two
    /// implementations of one rule is a desync waiting to happen in both directions —
    /// a row enforced-unique but unindexed, or indexed but unenforced — and nothing
    /// compares them, so they move together or not at all.
    ///
    /// # Errors
    ///
    /// Propagates an evaluation error from the predicate, including the `42804` of a
    /// predicate that is not boolean.
    pub fn row_in_index(&self, row: &[Value]) -> Result<bool> {
        let Some(pred) = &self.predicate else {
            return Ok(true);
        };
        let col_names: Vec<&str> = self.column_names.iter().map(String::as_str).collect();
        match eval_expr(
            pred,
            &col_names,
            &self.column_types,
            &self.enum_orders(),
            row,
        )
        .and_then(|v| predicate_holds(&v, BoolContext::Where))
        {
            Ok(holds) => Ok(holds),
            // The predicate's own error (its operator-resolution failure, malformed
            // value, …) passes through with its real SQLSTATE — it is a SQL error
            // that happened to occur during index maintenance, not an opclass gap.
            Err(e) => Err(StorageError::ExprEval(Box::new(e))),
        }
    }

    /// Compute the key tuple for `row` in key-part order.
    ///
    /// For a **plain** index, each segment is `row[pos]`; for an **expression**
    /// index, each segment is `eval_expr(part, column_names, row)`. The single
    /// point of truth for the key computation, shared by every maintenance and
    /// probe path so the stored key and the write-time probe agree exactly.
    ///
    /// # Errors
    ///
    /// Returns `ExprEval` (carrying the expression's own error) if a key
    /// expression fails to evaluate.
    // both arms do real work (the `Some` arm builds `col_names` + maps with error
    // handling, the `None` arm maps positions), so the if-let reads clearer than a
    // `map_or_else` with two multi-line closures
    #[allow(clippy::option_if_let_else)]
    pub fn key_values(&self, row: &[Value]) -> Result<Vec<Value>> {
        if let Some(exprs) = &self.key_exprs {
            let col_names: Vec<&str> = self.column_names.iter().map(String::as_str).collect();
            exprs
                .iter()
                .map(|e| {
                    // As in `row_in_index`: keep the key expression's own error
                    // rather than relabeling it an opclass problem.
                    eval_expr(e, &col_names, &self.column_types, &self.enum_orders(), row)
                        .map_err(|err| StorageError::ExprEval(Box::new(err)))
                })
                .collect()
        } else {
            // Plain index: a missing position yields a NULL segment (the encoder
            // then drops the row under default NULLS DISTINCT, matching the old
            // `row.get(pos)` → `None` behavior).
            Ok(self
                .key_col_positions
                .iter()
                .map(|&pos| row.get(pos).cloned().unwrap_or(Value::Null))
                .collect())
        }
    }

    /// Encode `row`'s index key (its key columns or key expressions) into one
    /// composite blob, or `None` if a key part is NULL under default `NULLS
    /// DISTINCT`.
    ///
    /// Computes the key tuple via [`key_values`](Self::key_values) and folds it
    /// through [`encode_probe_key`], so a stored entry and a probe for the same row
    /// always agree.
    ///
    /// NULL semantics:
    /// - default `NULLS DISTINCT` (`nulls_not_distinct = false`): a NULL in **any**
    ///   key part means the row is not a uniqueness/equality candidate, so it is
    ///   not indexed — matching Postgres, where a unique index with a NULL in any
    ///   column never conflicts.
    /// - `NULLS NOT DISTINCT` (`nulls_not_distinct = true`): NULL key parts are
    ///   encoded (as the `SEG_NULL` tag), so two NULL keys fold to equal blobs and
    ///   conflict — and stay distinct from a present empty value.
    ///
    /// # Errors
    ///
    /// Returns `ExprEval` if a key expression fails to evaluate; the key encoder's own
    /// error, which it has none of today, rides `unindexable`.
    pub fn encode_row_key(&self, row: &[Value]) -> Result<Option<Vec<u8>>> {
        let values = self.key_values(row)?;
        if values.is_empty() {
            return Ok(None);
        }
        encode_probe_key(&values, self.nulls_not_distinct)
    }

    /// Encode **all** of `row`'s index-entry key blobs.
    ///
    /// A B-tree index yields the single [`encode_row_key`](Self::encode_row_key)
    /// blob (zero or one). A **GIN** index decomposes the (single) key value into
    /// its element/key set ([`gin_decompose`]) and yields one blob per element — the
    /// same single-column `encode_probe_key` encoding a B-tree key uses, so a GIN
    /// entry is byte-for-byte an ordinary index entry, just many per row. A row
    /// whose GIN value decomposes to nothing (a NULL/empty array, a non-container)
    /// yields no entries.
    ///
    /// # Errors
    ///
    /// Returns `ExprEval` if a key expression fails to evaluate; the key encoder's own
    /// error, which it has none of today, rides `unindexable`.
    pub fn encode_row_keys(&self, row: &[Value]) -> Result<Vec<Vec<u8>>> {
        match self.method {
            IndexMethod::Btree => Ok(self.encode_row_key(row)?.into_iter().collect()),
            IndexMethod::Gin => {
                // GIN is single-column: decompose the one key value into entries.
                let values = self.key_values(row)?;
                let Some(value) = values.first() else {
                    return Ok(Vec::new());
                };
                let mut blobs = Vec::new();
                for key in gin_decompose(value) {
                    // A decomposed key is non-NULL, so `false` (NULLS DISTINCT) still
                    // encodes it; the `Option` is `Some` in practice.
                    if let Some(blob) = encode_probe_key(std::slice::from_ref(&key), false)? {
                        blobs.push(blob);
                    }
                }
                Ok(blobs)
            }
        }
    }
}

impl IndexKeying {
    /// Every entry `row` **writes** into this index — a B-tree's one composite blob, a
    /// GIN's one blob per decomposed element — each refused where 18.6 would refuse it
    /// for its size, and where pglet's own page could not hold its key.
    ///
    /// The maintenance encoder for an entry being added: the storage backends call it
    /// wherever a row enters an index, so every door that writes an index entry meets
    /// these refusals — the executor's per-row gates (`executor::constraints`'
    /// `enforce_index_entries` and its foreign-key-action forms) and its build gates meet
    /// 18.6's first, in 18.6's order, and this is the belt behind them and where a write
    /// meets pglet's page bound. The storage backends ask a row's indexes in creation
    /// order, so the index a refusal here names is the one 18.6 would name first.
    /// The 18.6 sizes are `pglet_sql::executor::index_row`'s — one implementation, two
    /// callers. A row whose partial predicate excludes it is the caller's to skip, as
    /// for [`Self::encode_row_keys`].
    ///
    /// The page bound: an entry's key must fit an internal B+tree page as a separator,
    /// [`MAX_KEY_LEN`] bytes. pglet's key encoding is longer than 18.6's layout for a
    /// `bytea` holding NUL bytes (each escaped to two) and a long `numeric` (a byte a
    /// digit), so a key 18.6 holds can be refused here —
    /// [`pglet_core::Error::IndexKeyTooLongForPage`] (`EDGES.md` row 192).
    ///
    /// # Errors
    ///
    /// [`StorageError::IndexEntry`] carrying the refusal; `ExprEval` as
    /// [`Self::encode_row_keys`].
    pub fn encode_row_entries(&self, row: &[Value]) -> Result<Vec<Vec<u8>>> {
        let refused = |e: pglet_core::Error| StorageError::IndexEntry(Box::new(e));
        let values = self.key_values(row)?;
        let blobs: Vec<Vec<u8>> = match self.method {
            IndexMethod::Btree => {
                let size = index_entry_size(values.iter().zip(self.part_is_enum.iter().copied()));
                check_entry_size_field(size).map_err(refused)?;
                check_btree_item_size(size, &self.name, &self.table).map_err(refused)?;
                if values.is_empty() {
                    Vec::new()
                } else {
                    encode_probe_key(&values, self.nulls_not_distinct)?
                        .into_iter()
                        .collect()
                }
            }
            IndexMethod::Gin => {
                let Some(value) = values.first() else {
                    return Ok(Vec::new());
                };
                let elements = gin_decompose(value);
                check_gin_entries(value, &elements, &self.name, &self.table).map_err(refused)?;
                let mut blobs = Vec::with_capacity(elements.len());
                for element in &elements {
                    if let Some(blob) = encode_probe_key(std::slice::from_ref(element), false)? {
                        blobs.push(blob);
                    }
                }
                blobs
            }
        };
        for blob in &blobs {
            let len = BtreeKey::index_encoded_len(blob);
            if len > MAX_KEY_LEN {
                return Err(refused(pglet_core::Error::IndexKeyTooLongForPage {
                    index: self.name.clone(),
                    len,
                    max: MAX_KEY_LEN,
                }));
            }
        }
        Ok(blobs)
    }
}

/// Carry a `Value::encode_index_key` failure into a `StorageError` **without
/// re-rendering it**.
///
/// [`StorageError::Unindexable`] is a *type-name* slot: `From<StorageError>` turns it
/// back into `pglet_core::Error::UnindexableType`, whose own text is `data type {0} has
/// no default operator class for a B-tree index`. Stringifying an `UnindexableType`
/// into the slot rendered the sentence twice, which is what every write to a
/// container-keyed index read while the encoder refused one. The encoder raises nothing
/// today — every value has a key — so this is the shape an arm that refuses again
/// would take: the bare type name for that variant, and any other error on the verbatim
/// passthrough rather than re-labelled an opclass failure.
///
/// "Type-name slot" describes the variant's only *live* writers, not its only writers:
/// three siblings in this file put non-type-name text there — `encode_range_bounds`'s
/// `"range bound encodes to no key"` and the two `deserialize_*` failures — each of which
/// would render as this sentence wrapped around a phrase that is not a type. All three
/// are unreachable today (the range bound filters its NULLs before the call, and both IR
/// blobs are the ones `CREATE INDEX` captured), which is why they are documented rather
/// than re-typed.
fn unindexable(e: pglet_core::Error) -> StorageError {
    match e {
        pglet_core::Error::UnindexableType(type_name) => StorageError::Unindexable(type_name),
        other => StorageError::ExprEval(Box::new(other)),
    }
}

/// Encode an explicit per-column probe tuple into one composite index key blob,
/// or `None` if any value is NULL under default `NULLS DISTINCT` (no equality
/// match).
///
/// The probe-side counterpart to [`IndexKeying::encode_row_key`]: callers pass the
/// key values already gathered in column order. Each segment is tagged
/// (`SEG_PRESENT ++ encode_index_key(value)` for a present value, `SEG_NULL` for a
/// NULL) so a NULL key stays distinct from a present empty value, then the tagged
/// segments are folded via `encode_composite_index_key`. Under `NULLS NOT
/// DISTINCT` a NULL probe encodes (two NULLs collide); otherwise a NULL probe
/// matches nothing.
///
/// # Errors
///
/// The key encoder's error, through `unindexable`; it raises none today.
pub fn encode_probe_key(values: &[Value], nulls_not_distinct: bool) -> Result<Option<Vec<u8>>> {
    if !nulls_not_distinct && values.iter().any(Value::is_null) {
        return Ok(None);
    }
    let mut segments: Vec<Vec<u8>> = Vec::with_capacity(values.len());
    for value in values {
        // Per-segment presence tag: a NULL is the bare `SEG_NULL` tag; a present
        // value is `SEG_PRESENT` ++ its order-preserving bytes. This keeps NULL
        // distinct from an empty (but present) value — `Null`, `Text("")`, and
        // `Bytea([])` all encode to empty bytes, so without the tag they'd collide.
        if value.is_null() {
            segments.push(vec![SEG_NULL]);
        } else {
            let mut seg = Vec::with_capacity(1);
            seg.push(SEG_PRESENT);
            seg.extend_from_slice(&value.encode_index_key().map_err(unindexable)?);
            segments.push(seg);
        }
    }
    let refs: Vec<&[u8]> = segments.iter().map(Vec::as_slice).collect();
    Ok(Some(encode_composite_index_key(&refs)))
}

/// Intersect two **ascending-sorted** row-id lists (a GIN posting-list AND).
///
/// A single index value's entries order by `row_id`, so each posting list is sorted
/// and the merge is linear. Combines the posting lists of a multi-key `@>` probe — a
/// row matches only when it appears under *every* query key.
#[must_use]
fn intersect_sorted(a: &[RowId], b: &[RowId]) -> Vec<RowId> {
    use std::cmp::Ordering;
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}

/// Intersect the posting lists of a GIN query's keys into a candidate row-id set.
///
/// Each key is encoded with [`encode_probe_key`] (single segment, NULLS DISTINCT)
/// and `fetch` returns that key's row-id-sorted posting list; the result is the
/// sorted intersection across all keys — a candidate superset the caller rechecks
/// with the real `@>`. This is the backend-independent half shared by both stores
/// (posting lists from a `HashMap` for memory, from an `index_scan_at_root` for
/// the B+tree), so the intersection loop lives once rather than drifting in two
/// copies.
///
/// `keys` empty (the `x @> '{}'` case) returns empty — the caller full-scans
/// instead. A key that can't be encoded (a NULL element) yields no matches.
///
/// # Errors
///
/// Propagates a key-encoding error, or any error `fetch` returns.
pub fn gin_intersect_posting_lists(
    keys: &[Value],
    mut fetch: impl FnMut(&[u8]) -> Result<Vec<RowId>>,
) -> Result<Vec<RowId>> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let mut acc: Option<Vec<RowId>> = None;
    for key in keys {
        let Some(enc) = encode_probe_key(std::slice::from_ref(key), false)? else {
            return Ok(Vec::new());
        };
        let list = fetch(&enc)?;
        acc = Some(match acc {
            None => list,
            Some(prev) => intersect_sorted(&prev, &list),
        });
        // A running intersection that empties stays empty.
        if acc.as_ref().is_some_and(Vec::is_empty) {
            return Ok(Vec::new());
        }
    }
    Ok(acc.unwrap_or_default())
}

/// Build the half-open `[start, end)` `BtreeKey` bounds of an index range read from
/// optional lower/upper value bounds.
///
/// Each bound is `Some((value, inclusive))` or `None` (no bound that side).
/// [`encode_range_bounds`] decides both ends as blobs — it is where a range read's NULL
/// rule lives, for this backend and the in-memory one alike — and this maps them into
/// the B+tree's keyspace: the lower end through [`BtreeKey::index_range_start`], the
/// upper through [`BtreeKey::index_range_end`], or [`BtreeKey::index_table_end`] where
/// there is none. The start is never the band's own ([`BtreeKey::index_table_start`]),
/// which sorts below a NULL key.
///
/// The bounds are a **single-column** index's. A bound's blob is one segment, framed
/// verbatim, where a composite key's leading segment is escape-terminated
/// ([`encode_composite_index_key`]), so a bound does not sit in a composite index's key
/// order: over `(a, b)` keys `(1, 10)`, `(2, 20)` and `(3, 30)`, `a < 3` names no entry
/// and `a >= 2` names every one. What holds for a key of any arity is the start's NULL
/// rule alone, and the planner offers a range to a single bare column's index only
/// (`single_column_index_for`).
///
/// # Errors
///
/// As [`encode_range_bounds`].
pub fn index_range_bounds_for(
    index_id: u32,
    lower: Option<(&Value, bool)>,
    upper: Option<(&Value, bool)>,
) -> Result<(BtreeKey, BtreeKey)> {
    let ((lower, lower_inclusive), upper) = encode_range_bounds(lower, upper)?;
    let start = BtreeKey::index_range_start(index_id, &lower, lower_inclusive);
    let end = match upper {
        Some((blob, inclusive)) => BtreeKey::index_range_end(index_id, &blob, inclusive),
        None => BtreeKey::index_table_end(index_id),
    };
    Ok((start, end))
}

/// The least blob a key whose leading part is **present** can have: the bare
/// `SEG_PRESENT` tag, which is the whole key of a value that encodes to no bytes — an
/// empty `text`, for one.
///
/// A key blob begins with its leading segment's tag — a single segment is framed
/// verbatim, and a composite key's escaping leaves `SEG_PRESENT` as it is and turns
/// `SEG_NULL` into two bytes that still begin with it ([`encode_composite_index_key`]).
/// So this blob sorts above every key whose leading part is NULL and at or below every
/// key whose leading part is present, whatever the index's arity.
const LEAST_PRESENT_KEY: [u8; 1] = [SEG_PRESENT];

/// The lower end of an index range read, encoded for either backend.
///
/// A key blob and whether the range includes it. A range read always has one — see
/// [`encode_range_bounds`].
pub type EncodedLowerBound = (Vec<u8>, bool);

/// The upper end of an index range read, encoded for either backend.
///
/// The blob [`encode_probe_key`] yields for the bound value, plus whether the range
/// includes it. `None` = no upper bound, the range running to the index's last key.
pub type EncodedUpperBound = Option<(Vec<u8>, bool)>;

/// Encode the two ends of an index range read, for both backends.
///
/// **A range read answers the rows whose key is a non-NULL value within the bounds**, so
/// its lower end is never below the least present key: with no lower bound it is
/// `LEAST_PRESENT_KEY`, inclusive, and a lower bound's own blob is `SEG_PRESENT`-tagged
/// and so at or above it. A key whose leading part is NULL — a `NULLS NOT DISTINCT`
/// index holds one, an index with the default `NULLS DISTINCT` does not
/// ([`IndexKeying::encode_row_key`]) — sorts below that end and is in no range, with a
/// bound given or with none: a comparison is not true of a NULL. So a read with no bound
/// at all answers every row whose key is non-NULL, in key order, and that is every row
/// of the table only where the key cannot be NULL.
///
/// A **NULL bound value** is not a comparison either (`col <op> NULL` is SQL
/// `unknown`), so it is dropped to no bound that side rather than encoded; a caller that
/// means `col <op> NULL` answers no row without asking. A present value folds to its
/// tagged single-segment blob via [`encode_probe_key`] (`false` / NULLS DISTINCT is
/// irrelevant — a non-NULL value always encodes).
///
/// Both the B+tree builder ([`index_range_bounds_for`], → `BtreeKey` bounds) and the
/// in-memory backend's `index_range_lookup` (→ `std::ops::Bound`) adapt this one
/// encoded pair, so the NULL semantics and the encode error can't drift between them
/// (the very drift this module exists to prevent).
///
/// # Errors
///
/// The key encoder's error, through `unindexable`; it raises none today.
pub fn encode_range_bounds(
    lower: Option<(&Value, bool)>,
    upper: Option<(&Value, bool)>,
) -> Result<(EncodedLowerBound, EncodedUpperBound)> {
    let encode_side = |bound: Option<(&Value, bool)>| -> Result<Option<(Vec<u8>, bool)>> {
        match bound.filter(|(v, _)| !v.is_null()) {
            Some((value, inclusive)) => {
                let blob =
                    encode_probe_key(std::slice::from_ref(value), false)?.ok_or_else(|| {
                        StorageError::Unindexable("range bound encodes to no key".into())
                    })?;
                Ok(Some((blob, inclusive)))
            }
            None => Ok(None),
        }
    };
    let lower = encode_side(lower)?.unwrap_or_else(|| (LEAST_PRESENT_KEY.to_vec(), true));
    Ok((lower, encode_side(upper)?))
}

/// Deserialize a partial index's predicate IR into a cached `Expr`, or `None`.
///
/// The catalog stores the predicate as the serialized planner `Expr`; the
/// `IndexSpec` carries those opaque bytes (`predicate_ir`), read back here via
/// `pglet_sql`'s `deserialize_expr`.
///
/// # Errors
///
/// Returns `Unindexable` if the predicate fails to deserialize (should not happen
/// for predicates captured at `CREATE INDEX`).
fn deserialize_index_predicate(predicate_ir: Option<&[u8]>) -> Result<Option<Expr>> {
    let Some(bytes) = predicate_ir else {
        return Ok(None);
    };
    let expr = deserialize_expr(bytes).map_err(|e| match e {
        // Too deep for this thread's bounds (`pglet_sql::depth`): the refusal itself.
        pglet_sql::Error::StackDepthExceeded => StorageError::ExprEval(Box::new(e)),
        e => StorageError::Unindexable(format!("partial index predicate deserialize: {e}")),
    })?;
    Ok(Some(expr))
}

/// Deserialize an expression index's key-part IR into cached `Expr`s, or `None`
/// for a plain (position-keyed) index.
///
/// Each part is a serialized planner `Expr` (a bare column is an `Expr::Column`, so
/// a mixed bare/expression key folds through one eval path), read back via
/// `deserialize_expr`.
///
/// # Errors
///
/// Returns `Unindexable` if a key part fails to deserialize (should not happen for
/// parts captured at `CREATE INDEX`).
fn deserialize_index_key_parts(key_part_ir: Option<&[Vec<u8>]>) -> Result<Option<Vec<Expr>>> {
    let Some(parts) = key_part_ir else {
        return Ok(None);
    };
    let exprs = parts
        .iter()
        .map(|bytes| {
            deserialize_expr(bytes).map_err(|e| match e {
                pglet_sql::Error::StackDepthExceeded => StorageError::ExprEval(Box::new(e)),
                e => StorageError::Unindexable(format!("index key expression deserialize: {e}")),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(exprs))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain single-column `IndexKeying` over column `pos`, with the given
    /// `nulls_not_distinct`.
    fn plain(pos: usize, nulls_not_distinct: bool) -> IndexKeying {
        IndexKeying {
            name: "i".to_string(),
            table: "t".to_string(),
            part_is_enum: vec![false],
            key_col_positions: vec![pos],
            key_exprs: None,
            predicate: None,
            nulls_not_distinct,
            column_names: Vec::new(),
            column_types: Vec::new(),
            column_enum_orders: Vec::new(),
            method: IndexMethod::Btree,
        }
    }

    #[test]
    fn null_key_is_distinct_from_empty_text_and_empty_bytea_under_nnd() {
        // BUG-E regression: under NULLS NOT DISTINCT, a NULL key must NOT fold to
        // the same blob as an empty-string or empty-bytea key. All three encode to
        // empty bytes via `Value::encode_index_key`, so without the per-segment
        // presence tag they collide.
        let null_blob = encode_probe_key(&[Value::Null], true).unwrap().unwrap();
        let empty_text = encode_probe_key(&[Value::Text(String::new())], true)
            .unwrap()
            .unwrap();
        let empty_bytea = encode_probe_key(&[Value::Bytea(Vec::new())], true)
            .unwrap()
            .unwrap();

        assert_ne!(null_blob, empty_text, "NULL must differ from ''");
        assert_ne!(null_blob, empty_bytea, "NULL must differ from empty bytea");
        // '' and empty-bytea both encode to empty bytes and are the same SQL-empty
        // present value here (distinguishing those two is a type concern the index
        // doesn't carry); the load-bearing distinction is NULL vs present-empty.
        assert_eq!(empty_text, empty_bytea);

        // Two NULLs still collide (the NND conflict must survive the tag).
        let null_blob_2 = encode_probe_key(&[Value::Null], true).unwrap().unwrap();
        assert_eq!(null_blob, null_blob_2, "two NULLs must still collide");
        // Two empty strings still collide.
        let empty_text_2 = encode_probe_key(&[Value::Text(String::new())], true)
            .unwrap()
            .unwrap();
        assert_eq!(empty_text, empty_text_2);
    }

    #[test]
    fn stored_expr_eval_error_keeps_its_own_sqlstate() {
        // A database created before DDL-time type-checking can still hold a
        // predicate / key expression the operand gate rejects. Maintenance must then
        // report *that* error — `42883`, naming the operator — not the old blanket
        // `Unindexable` relabel, which claimed `0A000` "data type … has no default
        // operator class for a B-tree index" about an operator-resolution failure.
        let keying = IndexKeying {
            name: "i".to_string(),
            table: "t".to_string(),
            part_is_enum: Vec::new(),
            key_col_positions: vec![],
            key_exprs: None,
            // `id LIKE '1%'` over an integer column: no `integer ~~ text` operator.
            predicate: Some(Expr::BinaryOp {
                left: Box::new(Expr::Column {
                    table: None,
                    name: "id".to_string(),
                }),
                op: pglet_sql::planner::BinaryOp::Like,
                right: Box::new(Expr::Literal(pglet_sql::planner::Literal::String(
                    "1%".to_string(),
                ))),
            }),
            nulls_not_distinct: false,
            column_names: vec!["id".to_string()],
            column_types: Vec::new(),
            column_enum_orders: Vec::new(),
            method: IndexMethod::Btree,
        };
        let err = keying
            .row_in_index(&[Value::Int4(1)])
            .expect_err("no integer ~~ text operator");
        let sql_err: pglet_core::Error = err.into();
        assert_eq!(sql_err.sqlstate(), "42883");
        assert!(
            sql_err.to_string().contains("operator does not exist"),
            "message should name the operator failure, got: {sql_err}"
        );
    }

    #[test]
    fn present_values_remain_order_preserving_with_the_tag() {
        // The constant `SEG_PRESENT` prefix must not disturb the underlying
        // order-preserving encoding: "a" < "b" still holds on the tagged blobs.
        let a = encode_probe_key(&[Value::Text("a".into())], false)
            .unwrap()
            .unwrap();
        let b = encode_probe_key(&[Value::Text("b".into())], false)
            .unwrap()
            .unwrap();
        assert!(a < b, "present values stay ordered under the tag");

        // A NULL segment sorts before any present value (SEG_NULL < SEG_PRESENT).
        let null_blob = encode_probe_key(&[Value::Null], true).unwrap().unwrap();
        assert!(null_blob < a, "NULL sorts before present values");
    }

    #[test]
    fn a_range_has_a_lower_end_whatever_bounds_it_is_given() {
        // No lower bound, and a NULL one, are the least present key, inclusive.
        let null = Value::Null;
        let five = Value::Int4(5);
        for lower in [None, Some((&null, true)), Some((&null, false))] {
            for upper in [None, Some((&five, false)), Some((&null, true))] {
                let ((blob, inclusive), _) = encode_range_bounds(lower, upper).unwrap();
                assert_eq!(blob, vec![SEG_PRESENT], "{lower:?}..{upper:?}");
                assert!(inclusive, "{lower:?}..{upper:?}");
            }
        }
        // A present lower bound is its own blob, with its own inclusivity.
        let five_blob = encode_probe_key(std::slice::from_ref(&five), false)
            .unwrap()
            .unwrap();
        for inclusive in [true, false] {
            let (lower, upper) = encode_range_bounds(Some((&five, inclusive)), None).unwrap();
            assert_eq!(lower, (five_blob.clone(), inclusive));
            assert_eq!(upper, None);
        }
        // The upper end: a bound's blob, or none — a NULL bound is none.
        let (_, upper) = encode_range_bounds(None, Some((&five, true))).unwrap();
        assert_eq!(upper, Some((five_blob, true)));
        let (_, upper) = encode_range_bounds(None, Some((&null, true))).unwrap();
        assert_eq!(upper, None);
    }

    #[test]
    fn the_least_present_key_divides_keys_by_their_leading_part() {
        // What a range's lower end rests on: every key whose leading part is NULL sorts
        // below `LEAST_PRESENT_KEY`, every key whose leading part is present at or above
        // it — the least value of each type among them — for one key part and for
        // several.
        let least_values = [
            Value::Text(String::new()),
            Value::Bytea(Vec::new()),
            Value::Bytea(vec![0x00]),
            Value::Bool(false),
            Value::Int2(i16::MIN),
            Value::Int4(i32::MIN),
            Value::Int8(i64::MIN),
            Value::Float8(f64::NEG_INFINITY),
            Value::Uuid([0; 16]),
        ];
        let least = LEAST_PRESENT_KEY.to_vec();
        let key = |values: &[Value]| encode_probe_key(values, true).unwrap().unwrap();

        assert!(key(&[Value::Null]) < least, "a NULL key");
        for value in &least_values {
            assert!(key(std::slice::from_ref(value)) >= least, "{value:?}");
            // Composite: the leading part decides, whatever follows it.
            for second in [Value::Null, Value::Int4(i32::MIN), Value::Int4(7)] {
                assert!(
                    key(&[value.clone(), second.clone()]) >= least,
                    "({value:?}, {second:?})"
                );
                assert!(
                    key(&[Value::Null, second.clone(), value.clone()]) < least,
                    "(NULL, {second:?}, {value:?})"
                );
            }
        }
        // The empty present value is the least key itself, so the end must include it.
        assert_eq!(key(&[Value::Text(String::new())]), least);
    }

    #[test]
    fn default_nulls_distinct_drops_null_keyed_rows() {
        // Under default NULLS DISTINCT a NULL key part yields no blob (the row is
        // not a uniqueness candidate), independent of the presence tag.
        let keying = plain(1, false);
        let none = keying
            .encode_row_key(&[Value::Int4(1), Value::Null])
            .unwrap();
        assert!(
            none.is_none(),
            "NULL-keyed row is not indexed under NULLS DISTINCT"
        );

        // Under NULLS NOT DISTINCT the same row IS indexed (a tagged NULL blob).
        let keying_nnd = plain(1, true);
        let some = keying_nnd
            .encode_row_key(&[Value::Int4(1), Value::Null])
            .unwrap();
        assert!(
            some.is_some(),
            "NULL-keyed row IS indexed under NULLS NOT DISTINCT"
        );
    }

    #[test]
    fn row_key_and_probe_key_agree_for_present_values() {
        // The maintenance key (`encode_row_key`) and the probe key
        // (`encode_probe_key`) must fold identically for the same value.
        let keying = plain(0, false);
        let row_key = keying
            .encode_row_key(&[Value::Text("hi".into())])
            .unwrap()
            .unwrap();
        let probe_key = encode_probe_key(&[Value::Text("hi".into())], false)
            .unwrap()
            .unwrap();
        assert_eq!(row_key, probe_key);
    }

    /// The executor's build gates size pglet's key through
    /// `pglet_sql::executor::index_row::encoded_key_len`, a second implementation of
    /// this module's framing (that crate cannot call this one); it must give the length
    /// of the key storage actually builds, for every shape — one part and several, NULL
    /// parts under either rule, NUL bytes that escape once per framing level.
    #[test]
    fn the_executor_key_length_is_the_key_storage_builds() {
        use pglet_sql::executor::index_row::{PAGE_KEY_LEN_MAX, encoded_key_len};
        assert_eq!(PAGE_KEY_LEN_MAX, MAX_KEY_LEN);
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = |n: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % n
        };
        for round in 0..3000 {
            let parts = 1 + next(3) as usize;
            let values: Vec<Value> = (0..parts)
                .map(|_| match next(6) {
                    0 => Value::Null,
                    1 => Value::Int4(next(1000) as i32 - 500),
                    2 => Value::Text("ab\u{e9}".repeat(next(40) as usize)),
                    3 => Value::Bytea((0..next(60)).map(|_| (next(4) * 60) as u8).collect()),
                    4 => Value::Numeric(
                        pglet_sql::types::PgNumeric::parse(&format!(
                            "{}.{}",
                            next(99_999),
                            next(999)
                        ))
                        .unwrap(),
                    ),
                    _ => Value::Int8(0),
                })
                .collect();
            for nnd in [false, true] {
                let want = encode_probe_key(&values, nnd)
                    .unwrap()
                    .map(|blob| BtreeKey::index_encoded_len(&blob));
                assert_eq!(
                    encoded_key_len(&values, nnd),
                    want,
                    "round {round}: {values:?}"
                );
            }
        }
    }

    /// The maintenance encoder refuses what 18.6 refuses and what the page cannot hold,
    /// and writes the entry otherwise; a NULL part is sized though no entry is written.
    #[test]
    fn encode_row_entries_refuses_oversized_entries() {
        let keying = plain(0, false);
        let code = |row: Vec<Value>| match keying.encode_row_entries(&row) {
            Err(StorageError::IndexEntry(e)) => Some(e.sqlstate()),
            Ok(_) => None,
            Err(other) => panic!("{other}"),
        };
        assert_eq!(code(vec![Value::Text("x".repeat(2692))]), None);
        assert_eq!(code(vec![Value::Text("x".repeat(2693))]), Some("54000"));
        let nul_dense: Vec<u8> = (0..1346).flat_map(|_| [0u8, 7]).collect();
        assert_eq!(code(vec![Value::Bytea(nul_dense)]), Some("54000"));
        assert_eq!(code(vec![Value::Bytea(vec![7; 2692])]), None);
        assert_eq!(
            keying.encode_row_entries(&[Value::Null]).unwrap(),
            Vec::<Vec<u8>>::new()
        );
    }
}
