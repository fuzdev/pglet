//! The structured `ErrorResponse` field set — the **one** builder every host sends
//! an error through.
//!
//! Postgres's `send_message_to_frontend` (`elog.c`) writes an error as a sequence of
//! typed fields, not a string: severity (`S`, and the unlocalized `V`), SQLSTATE
//! (`C`), primary message (`M`), then whichever of detail (`D`), hint (`H`),
//! schema/table/column/constraint (`s`/`t`/`c`/`n`) and statement position (`P`) the
//! error carries. Drivers surface those by name — `pg`'s `DatabaseError.detail` /
//! `.constraint`, tokio-postgres's `DbError::table()` — so a field pglet does not
//! send is a field the consumer sees as `undefined`.
//!
//! Two rules govern this module:
//!
//! - **One builder.** [`Error::wire_fields`] is the only place a field set is
//!   assembled, so the native server's three send sites and the wasm host cannot
//!   drift into sending different fields for the same error.
//! - **Omit rather than invent.** A field pglet cannot fill *truthfully* is left out.
//!   `F`/`L`/`R` (the C source file, line and routine) are the clearest case: any
//!   value pglet put there would be a claim about a source tree it does not have, so
//!   they are never sent. The differential harness reads the same way — pglet
//!   omitting a field 18.6 sends is not a failure; pglet sending a *different* value
//!   is.
//!
//! Field order is 18.6's, from `send_message_to_frontend`: `S V C M D H W s t c d n P`.

use crate::error::{DropTargetName, Error, RelationKind, WrongTypeRoute};

/// The severity a host is sending this message at — `S` and `V`.
///
/// Postgres sends the localized severity in `S` and the unlocalized one in `V`; on an
/// unlocalized server they are the same string, which is what pglet is. `V` exists
/// because `S` is translated and therefore unparseable; `pg` drops `V` entirely,
/// tokio-postgres exposes it, and its *absence* is the single cheapest tell that a
/// response did not come from Postgres.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A statement failed; the session continues.
    Error,
    /// The backend is closing the connection.
    Fatal,
    /// A `NoticeResponse` Postgres raises at `NOTICE` (the `DROP … IF EXISTS` skip).
    Notice,
    /// A `NoticeResponse` Postgres raises at `WARNING` (`COMMIT`/`ROLLBACK` outside a
    /// transaction block).
    Warning,
}

impl Severity {
    /// The wire spelling, for both `S` and `V`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Fatal => "FATAL",
            Self::Notice => "NOTICE",
            Self::Warning => "WARNING",
        }
    }
}

/// pglet's single implicit schema, and the value of every `s` field it sends.
///
/// pglet models one schema and no `search_path` beyond it, so `s` is a constant
/// rather than a lookup — stated here once so a reader does not go looking for the
/// resolver that would fill it.
pub const IMPLICIT_SCHEMA: &str = "public";

/// The per-field byte budget of `Failing row contains (…)` — Postgres passes 64 as
/// `maxfieldlen` at both `ExecBuildSlotValueDescription` call sites (`execMain.c`).
const MAX_FIELD_LEN: usize = 64;

/// One row's columns rendered as text, in table order — the payload of Postgres's
/// `Failing row contains (…)` DETAIL.
///
/// `None` is a SQL NULL, which renders as the bare word `null`. The values are the
/// column type's *output function* text (pglet's `value_to_text_with_oid`), captured
/// unclipped at the error site; the 64-byte clip is applied here, at render time, so
/// the rule lives in one place.
///
/// Postgres renders these **raw** — no quoting and no escaping — so a text value
/// containing `, ` is indistinguishable from a field boundary. That is the oracle's
/// behaviour, deliberately reproduced rather than improved on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailingRow {
    /// Each column's rendered text, `None` for NULL, in the relation's column order.
    pub values: Vec<Option<String>>,
}

impl FailingRow {
    /// Build from a column-ordered list of rendered values.
    #[must_use]
    pub const fn new(values: Vec<Option<String>>) -> Self {
        Self { values }
    }

    /// Postgres's `Failing row contains (…).` sentence.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::from("Failing row contains (");
        for (i, value) in self.values.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            match value {
                None => out.push_str("null"),
                Some(text) => push_clipped(&mut out, text),
            }
        }
        out.push_str(").");
        out
    }
}

/// Append `text` to `out`, clipped to [`MAX_FIELD_LEN`] **bytes** at a character
/// boundary with a trailing `...`.
///
/// Postgres's rule is `if (vallen <= maxfieldlen)` keep the whole value, else
/// `pg_mbcliplen` then append `...` — so a value of exactly 64 bytes is **not**
/// ellipsized, and a multibyte value is cut at the last character that fits rather
/// than mid-sequence (measured on 18.6: `repeat('x',2000)` → 64 `x` + `...`;
/// `repeat('é',100)` → 32 `é` + `...`).
fn push_clipped(out: &mut String, text: &str) {
    if text.len() <= MAX_FIELD_LEN {
        out.push_str(text);
        return;
    }
    let mut end = MAX_FIELD_LEN;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    out.push_str(&text[..end]);
    out.push_str("...");
}

/// A key tuple named and valued, in Postgres's `BuildIndexValueDescription`
/// (`genam.c`) shape: `Key (a, b)=(1, x)`.
///
/// Unlike [`FailingRow`] this is **never truncated** — `BuildIndexValueDescription`
/// passes no `maxfieldlen` — and it also renders values raw, with `null` for a NULL
/// (reachable through `UNIQUE NULLS NOT DISTINCT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyDescription {
    /// The key columns' names, in key order.
    pub names: Vec<String>,
    /// Each key column's rendered value, `None` for NULL.
    pub values: Vec<Option<String>>,
}

impl KeyDescription {
    /// Build from paired names and rendered values.
    #[must_use]
    pub const fn new(names: Vec<String>, values: Vec<Option<String>>) -> Self {
        Self { names, values }
    }

    /// The `Key (a, b)=(1, x)` prefix, without the per-error sentence tail.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::from("Key (");
        out.push_str(&self.names.join(", "));
        out.push_str(")=(");
        for (i, value) in self.values.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(value.as_deref().unwrap_or("null"));
        }
        out.push(')');
        out
    }
}

/// The payload of an index-entry size refusal: the entry's size in 18.6's layout,
/// the index it was bound for, and the index's relation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRowSize {
    /// The entry's size in bytes, as 18.6's `index_form_tuple` lays it out.
    pub size: usize,
    /// The index's name — 18.6's `n`, and the name the message quotes.
    pub index: String,
    /// The indexed relation — 18.6's `t`.
    pub table: String,
}

/// The structured half of a `23505` — the relation the duplicate key was written to
/// and the key tuple that already existed.
///
/// Boxed into [`Error::UniqueViolation`]: the constraint's *name* is the message and
/// was always carried, but 18.6 also sends `t` (the table) and a
/// `Key (…)=(…) already exists.` DETAIL, and neither is derivable from the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniqueViolationDetail {
    /// The relation the write targeted — 18.6's `t`.
    pub table: String,
    /// The colliding key tuple, or `None` when it cannot be described truthfully — an
    /// **expression** index, whose key column has no spelled name here and whose value
    /// has no declared type to render under. `table` is still filled, so `s`/`t` go out
    /// as 18.6 sends them and only `D` is dropped.
    pub key: Option<KeyDescription>,
}

/// The structured half of a `42804 foreign key constraint "…" cannot be implemented`:
/// the constraint and the one column pair 18.6's DETAIL names.
///
/// The column names are printed **raw** inside their quotes (`strVal` of the written
/// list, not `quote_identifier`) and the type names as `format_type_be` spells them,
/// which is a type's SQL name with no modifier — the caller renders those two, since
/// only the catalog knows an enum's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKeyTypeMismatch {
    /// The constraint's name — the minted one where none was declared.
    pub constraint: String,
    /// The referencing column of the first pair 18.6 cannot implement.
    pub column: String,
    /// The referenced column it pairs with, by position in the key lists.
    pub referenced_column: String,
    /// The referencing column's type, as `format_type_be` prints it.
    pub column_type: String,
    /// The referenced column's type, likewise.
    pub referenced_type: String,
}

impl ForeignKeyTypeMismatch {
    /// The `D` sentence, verbatim from `ATAddForeignKeyConstraint`.
    #[must_use]
    pub fn detail(&self) -> String {
        format!(
            "Key columns \"{}\" of the referencing table and \"{}\" of the referenced table \
             are of incompatible types: {} and {}.",
            self.column, self.referenced_column, self.column_type, self.referenced_type
        )
    }
}

/// Which side of a foreign key the statement was writing, which decides both the
/// primary message and the DETAIL sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FkDirection {
    /// A child INSERT/UPDATE whose referenced parent row is absent.
    Insert,
    /// A parent DELETE/UPDATE that would orphan a child row.
    Delete,
}

/// The structured half of a `23503` / `23001`.
///
/// Replaces the pre-formatted sentence [`Error::ForeignKeyViolation`] used to carry,
/// because 18.6 sends four separate facts the sentence cannot be taken apart into —
/// and one of them is a trap: **`t` is the CHILD table in both directions**
/// (`errtableconstraint(fk_rel, …)` in `ri_triggers.c`), while the *message* names
/// the parent on the delete side. Deriving `t` from the message gets it backwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKeyViolationDetail {
    /// The referencing table — 18.6's `t` in **both** directions.
    pub child_table: String,
    /// The referenced table, named in the DETAIL sentence.
    pub parent_table: String,
    /// The constraint's name — 18.6's `n`.
    pub constraint: String,
    /// Which side was written.
    pub direction: FkDirection,
    /// The key tuple: the child's referencing values on [`FkDirection::Insert`], the
    /// parent's referenced values on [`FkDirection::Delete`].
    pub key: KeyDescription,
}

impl ForeignKeyViolationDetail {
    /// The `23503` primary message for this direction.
    #[must_use]
    pub fn no_action_message(&self) -> String {
        let Self {
            child_table,
            parent_table,
            constraint,
            direction,
            ..
        } = self;
        match direction {
            FkDirection::Insert => format!(
                "insert or update on table \"{child_table}\" violates foreign key constraint \
                 \"{constraint}\""
            ),
            FkDirection::Delete => format!(
                "update or delete on table \"{parent_table}\" violates foreign key constraint \
                 \"{constraint}\" on table \"{child_table}\""
            ),
        }
    }

    /// The `23001` primary message — the RESTRICT wording, which exists only on the
    /// delete side.
    #[must_use]
    pub fn restrict_message(&self) -> String {
        let Self {
            child_table,
            parent_table,
            constraint,
            ..
        } = self;
        format!(
            "update or delete on table \"{parent_table}\" violates RESTRICT setting of foreign \
             key constraint \"{constraint}\" on table \"{child_table}\""
        )
    }

    /// The `23503` DETAIL sentence for this direction.
    #[must_use]
    pub fn no_action_detail(&self) -> String {
        let key = self.key.render();
        match self.direction {
            FkDirection::Insert => {
                format!("{key} is not present in table \"{}\".", self.parent_table)
            }
            FkDirection::Delete => {
                format!(
                    "{key} is still referenced from table \"{}\".",
                    self.child_table
                )
            }
        }
    }

    /// The `23001` DETAIL sentence — the same shape as the delete-side `23503` one
    /// **without** the word "still" (measured on 18.6, `ri_triggers.c:2785`).
    #[must_use]
    pub fn restrict_detail(&self) -> String {
        format!(
            "{} is referenced from table \"{}\".",
            self.key.render(),
            self.child_table
        )
    }
}

/// The catalog object an error is *about* — 18.6's `s`/`t`/`c`/`n` quartet.
///
/// `s` accompanies `t` and never appears alone, because Postgres fills them through
/// one macro family (`errtable`, `errtablecol`, `errtableconstraint`); pglet's `s` is
/// therefore emitted exactly when `table` is `Some`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectLocation {
    /// `t` — the relation.
    pub table: Option<String>,
    /// `c` — the column.
    pub column: Option<String>,
    /// `n` — the constraint.
    pub constraint: Option<String>,
}

impl ObjectLocation {
    /// A location naming a table and one of its columns.
    fn column(table: &str, column: &str) -> Self {
        Self {
            table: Some(table.to_string()),
            column: Some(column.to_string()),
            constraint: None,
        }
    }

    /// A location naming a table and one of its constraints.
    fn constraint(table: &str, constraint: &str) -> Self {
        Self {
            table: Some(table.to_string()),
            column: None,
            constraint: Some(constraint.to_string()),
        }
    }

    /// Whether any field is filled.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.table.is_none() && self.column.is_none() && self.constraint.is_none()
    }
}

/// The `Use DROP <KIND> to remove a <kind>.` hint Postgres's `DropErrorMsgWrongType`
/// attaches, keyed on the kind the name **actually** holds
/// (`tablecmds.c`'s `dropmsgstringarray`).
const fn drop_hint(actual: RelationKind) -> &'static str {
    match actual {
        RelationKind::Table => "Use DROP TABLE to remove a table.",
        RelationKind::Index => "Use DROP INDEX to remove an index.",
        RelationKind::Sequence => "Use DROP SEQUENCE to remove a sequence.",
    }
}

/// Postgres's `Use DROP ... CASCADE to drop the dependent objects too.` hint, shared
/// by every `reportDependentObjects` refusal (`dependency.c:1148`).
const CASCADE_HINT: &str = "Use DROP ... CASCADE to drop the dependent objects too.";

/// The dependent-object DETAIL line for a `DROP` blocked by a foreign key, in
/// Postgres's `getObjectDescription` shape — identifiers **unquoted**, as
/// `reportDependentObjects` writes them.
fn dependent_fk_detail(constraint: &str, child_table: &str, target: &DropTargetName) -> String {
    format!(
        "constraint {constraint} on table {child_table} depends on {}",
        target.unquoted()
    )
}

impl Error {
    /// This error's `D` (DETAIL), or `None` when pglet holds no truthful one.
    ///
    /// A variant that *could* carry a detail but was raised on a path with no row to
    /// show returns `None` — the `Option` payloads exist for exactly that case (an
    /// `ALTER TABLE … ADD CONSTRAINT` validation failure carries no
    /// `Failing row contains`, and 18.6 sends none there either).
    #[must_use]
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::NotNullViolation { row, .. } => row.as_ref().map(|r| r.render()),
            Self::CheckViolation { row, .. } => row.as_ref().map(|r| r.render()),
            Self::UniqueViolation { key: Some(d), .. } => d
                .key
                .as_ref()
                .map(|k| format!("{} already exists.", k.render())),
            // *duplicated*, not *already exists* — the index-build route's own
            // sentence (measured).
            Self::DuplicateIndexKey {
                detail: Some(d), ..
            } => d
                .key
                .as_ref()
                .map(|k| format!("{} is duplicated.", k.render())),
            Self::ForeignKeyViolation(fk) => Some(fk.no_action_detail()),
            Self::ForeignKeyCannotBeImplemented(mismatch) => Some(mismatch.detail()),
            Self::RestrictViolation(fk) => Some(fk.restrict_detail()),
            Self::WrongRelationKind { kind, .. } | Self::SequenceOwnerWrongKind { kind, .. } => {
                // `RelationKind`'s `Display` is already the **plural** noun — `indexes`,
                // not `index` — so the sentence takes it verbatim. Appending an `s`
                // here produced `…for indexess.`, which the fixture harness caught the
                // first time it compared a `D` field.
                Some(format!("This operation is not supported for {kind}."))
            }
            Self::DependentForeignKey {
                target,
                constraint,
                child_table,
            } => Some(dependent_fk_detail(constraint, child_table, target)),
            Self::DependentColumnDefault(dep) => Some(format!(
                "default value for column {} of table {} depends on sequence {}",
                dep.column, dep.table, dep.sequence
            )),
            Self::DependentOnType(dep) => dep
                .dependent
                .as_ref()
                .map(|dependent| format!("{dependent} depends on type {}", dep.type_name)),
            Self::IdentitySequenceOwnershipChange { sequence, table } => Some(format!(
                "Sequence \"{sequence}\" is linked to table \"{table}\"."
            )),
            Self::GeneratedAlwaysInsert { column, .. }
            | Self::GeneratedAlwaysUpdate { column, .. } => Some(format!(
                "Column \"{column}\" is an identity column defined as GENERATED ALWAYS."
            )),
            _ => None,
        }
    }

    /// This error's `H` (HINT), or `None`.
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        match self {
            Self::WrongObjectType {
                actual,
                route: WrongTypeRoute::Drop,
                ..
            } => Some(drop_hint(*actual).to_string()),
            Self::DependentObjectsExist(_)
            | Self::DependentForeignKey { .. }
            | Self::DependentColumnDefault(_)
            | Self::DependentOnType(_) => Some(CASCADE_HINT.to_string()),
            Self::SequenceRequiredByIdentity(dep) => Some(format!(
                "You can drop column {} of table {} instead.",
                dep.column, dep.table
            )),
            Self::IndexRequiredByConstraint { index, table } => Some(format!(
                "You can drop constraint {index} on table {table} instead — pglet has no \
                 ALTER TABLE ... DROP CONSTRAINT, so that means dropping table {table}."
            )),
            // 18.6 sends this hint on the INSERT route only; the UPDATE route
            // (`column "a" can only be updated to DEFAULT`) is `[SVCMDFLR]` — detail,
            // no hint (both measured).
            Self::InvalidParameterValue { hint, .. } => hint.clone(),
            Self::GeneratedAlwaysInsert { .. } => {
                Some("Use OVERRIDING SYSTEM VALUE to override.".to_string())
            }
            Self::TypeRequiredByTable(table) => {
                Some(format!("You can drop table {table} instead."))
            }
            Self::RelationTypeAlreadyExists(_) => Some(
                "A relation has an associated type of the same name, so you must use a name \
                 that doesn't conflict with any existing type."
                    .to_string(),
            ),
            Self::OnConflictRowAffectedTwice => Some(
                "Ensure that no rows proposed for insertion within the same command have \
                 duplicate constrained values."
                    .to_string(),
            ),
            // `_bt_check_third_page`'s hint, two lines as 18.6 sends it (measured).
            Self::BtreeIndexRowTooLarge(_) => Some(
                "Values larger than 1/3 of a buffer page cannot be indexed.\nConsider a \
                 function index of an MD5 hash of the value, or use full text indexing."
                    .to_string(),
            ),
            _ => None,
        }
    }

    /// This error's `P` (STATEMENT POSITION) — a **1-based Unicode codepoint index**
    /// into the statement text, never a byte offset.
    ///
    /// Measured on 18.6: `SELECT 'ααααα' AS a, FROM t` reports `P=22` (bytes would be
    /// 27) and `SELECT '👍👍👍' AS a, FROM t` reports `P=20` (one per emoji, not two
    /// UTF-16 units). It is filled where a token's offset is in hand, which is the
    /// parser: a `42601` built by `syntax_at` / `syntax_sentence_at`, and `opt_float`'s
    /// two `22023` range refusals. A position-bearing SQLSTATE outside those arms sends
    /// no `P`, and for one of two reasons: most would need a span threaded through
    /// `Plan`/`Expr`, which carry none; the exception is what
    /// `executor::type_resolution`'s `resolve_type_name` refuses, described in the
    /// paragraph below. `EDGES.md` row 76 holds the measured members, and carries no
    /// total.
    ///
    /// **The SQLSTATE is not the predicate, the producer is.** `opt_float`'s two `22023`s
    /// fill a `P` because the **parser** raises them, from a reader holding the number's
    /// own token. What `resolve_type_name` refuses does not: it takes a type *name* and
    /// no span, so none of its four callers can hand one down. An unrecognized type name
    /// ([`Error::UnsupportedType`]'s `42704`, measured at each caller) and the five a
    /// *carrier's* modifier reader raises ([`Error::CharacterLengthTooSmall`] and its
    /// four siblings) are both that shape. The omission is that **signature**, not the
    /// tree's missing spans and not any one caller's position: two of the four callers
    /// are the parser itself (`parser::expr`'s cast target and typed constant), holding a
    /// cursor and running the very reader that fills
    /// [`Self::FloatPrecisionTooSmall`]'s `P` — `parser::type_name`'s `float_type_name`,
    /// which both reach through `parse_scalar_type_name`: the cast target by way of
    /// `parse_type_name`, the typed constant directly, because `parse_type_name` is the
    /// wrapper that also consumes the `[]` suffix a typed constant must refuse — and
    /// the other two the executor (`executor::ddl`'s `CREATE TABLE` column walk and
    /// `ADD COLUMN`).
    #[must_use]
    pub const fn position(&self) -> Option<u32> {
        match self {
            Self::Syntax { position, .. } => *position,
            // `opt_float`'s two range refusals point at the number, as
            // `parser_errposition(@2)` does — the position is carried, not optional.
            Self::FloatPrecisionTooSmall { position }
            | Self::FloatPrecisionTooLarge { position } => Some(*position),
            _ => None,
        }
    }

    /// The catalog object this error is about — `s`/`t`/`c`/`n`.
    #[must_use]
    pub fn object_location(&self) -> ObjectLocation {
        match self {
            Self::NotNullViolation { table, column, .. }
            | Self::NotNullColumnHasNulls { table, column } => {
                ObjectLocation::column(table, column)
            }
            Self::CheckViolation {
                table, constraint, ..
            }
            | Self::CheckConstraintNotSatisfied { table, constraint } => {
                ObjectLocation::constraint(table, constraint)
            }
            Self::DuplicateIndexKey { index, detail } => detail.as_ref().map_or_else(
                || ObjectLocation {
                    constraint: Some(index.clone()),
                    ..ObjectLocation::default()
                },
                |d| ObjectLocation::constraint(&d.table, index),
            ),
            // The detail is carried whole or not at all, and **inside** it the
            // relation and the key description are separate facts:
            // `UniqueViolationDetail::table` fills `s`/`t` while its own
            // `KeyDescription` is the part that may be missing (an expression index
            // names no key), so those two are not dropped together. A `key` of `None`
            // here is the outer case — no detail at all — and leaves `n` filled from
            // the constraint with no `t` beside it.
            Self::UniqueViolation { constraint, key } => key.as_ref().map_or_else(
                || ObjectLocation {
                    constraint: Some(constraint.clone()),
                    ..ObjectLocation::default()
                },
                |d| ObjectLocation::constraint(&d.table, constraint),
            ),
            Self::ForeignKeyViolation(fk) | Self::RestrictViolation(fk) => {
                ObjectLocation::constraint(&fk.child_table, &fk.constraint)
            }
            // `errtableconstraint(heap, index)`: the heap and the **index**'s name in
            // `n`, the one index-size refusal 18.6 attaches a relation to.
            Self::BtreeIndexRowTooLarge(size) => {
                ObjectLocation::constraint(&size.table, &size.index)
            }
            // Everything else: 18.6 sends no `s`/`t`/`c`/`n`, and pglet holding a
            // table or column name is not a licence to send one. Measured absent on
            // 42703 (all four routes, and a foreign key's column lists), 42P01, 42601,
            // 42P07, 42710, 42701, 42803, 42830, a foreign key's 42804, 42P10, 42P16,
            // 428C9, 2BP01, 42809, 22001, 22003, 22012, 22P02
            // and 25P02 — the `errtable*` macros are called from the constraint
            // paths and almost nowhere else.
            _ => ObjectLocation::default(),
        }
    }

    /// Every `ErrorResponse` field for this error at `severity`, in 18.6's field
    /// order — the **one** builder each host sends through.
    ///
    /// `F`/`L`/`R` are deliberately absent: see this module's header.
    #[must_use]
    pub fn wire_fields(&self, severity: Severity) -> Vec<(u8, String)> {
        let mut fields = message_fields(severity, self.sqlstate(), &self.to_string());
        push_structured(
            &mut fields,
            self.detail(),
            self.hint(),
            &self.object_location(),
            self.position(),
        );
        fields
    }
}

/// The `S`/`V`/`C`/`M` prefix every message carries, whatever produced it.
#[must_use]
pub fn message_fields(severity: Severity, code: &str, message: &str) -> Vec<(u8, String)> {
    vec![
        (b'S', severity.as_str().to_string()),
        (b'V', severity.as_str().to_string()),
        (b'C', code.to_string()),
        (b'M', message.to_string()),
    ]
}

/// Append the optional fields in `send_message_to_frontend`'s order — `D H s t c n P`
/// — skipping every one that is absent.
fn push_structured(
    fields: &mut Vec<(u8, String)>,
    detail: Option<String>,
    hint: Option<String>,
    location: &ObjectLocation,
    position: Option<u32>,
) {
    if let Some(detail) = detail {
        fields.push((b'D', detail));
    }
    if let Some(hint) = hint {
        fields.push((b'H', hint));
    }
    if let Some(table) = &location.table {
        // `s` is emitted with `t` and never alone — Postgres fills both through one
        // `errtable*` macro, and pglet models exactly one schema.
        fields.push((b's', IMPLICIT_SCHEMA.to_string()));
        fields.push((b't', table.clone()));
    }
    if let Some(column) = &location.column {
        fields.push((b'c', column.clone()));
    }
    if let Some(constraint) = &location.constraint {
        fields.push((b'n', constraint.clone()));
    }
    if let Some(position) = position {
        fields.push((b'P', position.to_string()));
    }
}

/// The field set for a message with no [`Error`] behind it.
///
/// A `NoticeResponse`, a `FATAL` startup refusal, or one of the wire-framing codes the
/// server raises directly. Same `S`/`V`/`C`/`M` prefix, and no structured fields,
/// because there is no variant to derive them from.
#[must_use]
pub fn plain_wire_fields(severity: Severity, code: &str, message: &str) -> Vec<(u8, String)> {
    message_fields(severity, code, message)
}

#[cfg(test)]
mod tests {
    use super::{
        FailingRow, FkDirection, ForeignKeyTypeMismatch, ForeignKeyViolationDetail, KeyDescription,
        Severity, UniqueViolationDetail,
    };
    use crate::error::{Error, RelationKind, WrongTypeRoute};

    // Every expected string in this module was captured from 18.6 on
    // `127.0.0.1:5432` with a raw v3 client (session `TimeZone=UTC`, the same
    // contract the differential harness pins).

    fn row(values: &[Option<&str>]) -> FailingRow {
        FailingRow::new(
            values
                .iter()
                .map(|v| v.map(ToString::to_string))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn failing_row_renders_null_as_the_bare_word() {
        assert_eq!(
            row(&[None, Some("a")]).render(),
            "Failing row contains (null, a)."
        );
    }

    #[test]
    fn failing_row_renders_empty_string_as_nothing() {
        // 18.6: `INSERT INTO t (s) VALUES ('')` with a NOT NULL sibling gives
        // `Failing row contains (, null).` — an empty field, not `""`.
        assert_eq!(
            row(&[Some(""), None]).render(),
            "Failing row contains (, null)."
        );
    }

    #[test]
    fn failing_row_renders_text_raw_and_unquoted() {
        // Postgres neither quotes nor escapes, so a value containing `, ` is
        // indistinguishable from a field boundary. Reproduced, not improved on.
        assert_eq!(
            row(&[Some("q\"uo'te,back"), Some("{\"p, q\",r}")]).render(),
            "Failing row contains (q\"uo'te,back, {\"p, q\",r})."
        );
    }

    #[test]
    fn failing_row_clips_each_field_at_64_bytes() {
        let long = "x".repeat(2000);
        let rendered = row(&[Some(&long), None]).render();
        assert_eq!(
            rendered,
            format!("Failing row contains ({}..., null).", "x".repeat(64))
        );
    }

    #[test]
    fn failing_row_clips_multibyte_at_a_character_boundary() {
        // 100 × `é` is 200 bytes; 18.6 emits 32 of them (64 bytes) then `...`.
        let long = "é".repeat(100);
        let rendered = row(&[Some(&long)]).render();
        assert_eq!(
            rendered,
            format!("Failing row contains ({}...).", "é".repeat(32))
        );
    }

    #[test]
    fn failing_row_keeps_a_value_of_exactly_the_budget() {
        // `if (vallen <= maxfieldlen)` keeps the whole value — 64 bytes is not
        // ellipsized, 65 is.
        let exact = "x".repeat(64);
        assert_eq!(
            row(&[Some(&exact)]).render(),
            format!("Failing row contains ({exact}).")
        );
        let over = "x".repeat(65);
        assert_eq!(
            row(&[Some(&over)]).render(),
            format!("Failing row contains ({}...).", "x".repeat(64))
        );
    }

    #[test]
    fn key_description_renders_names_and_values_in_two_parens() {
        let key = KeyDescription::new(
            vec!["a".into(), "b".into()],
            vec![Some("1".into()), Some("x".into())],
        );
        assert_eq!(key.render(), "Key (a, b)=(1, x)");
    }

    #[test]
    fn key_description_never_truncates() {
        // `BuildIndexValueDescription` passes no `maxfieldlen`, unlike the
        // failing-row builder — so a long key value goes out whole.
        let long = "x".repeat(2000);
        let key = KeyDescription::new(vec!["a".into()], vec![Some(long.clone())]);
        assert_eq!(key.render(), format!("Key (a)=({long})"));
    }

    #[test]
    fn key_description_renders_null() {
        // Reachable through `UNIQUE NULLS NOT DISTINCT`.
        let key = KeyDescription::new(vec!["a".into()], vec![None]);
        assert_eq!(key.render(), "Key (a)=(null)");
    }

    #[test]
    fn not_null_violation_sends_detail_schema_table_column() {
        let err = Error::NotNullViolation {
            table: "t_nn".into(),
            column: "id".into(),
            row: Some(Box::new(row(&[None, Some("a")]))),
        };
        assert_eq!(
            err.wire_fields(Severity::Error),
            vec![
                (b'S', "ERROR".to_string()),
                (b'V', "ERROR".to_string()),
                (b'C', "23502".to_string()),
                (
                    b'M',
                    "null value in column \"id\" of relation \"t_nn\" violates not-null constraint"
                        .to_string()
                ),
                (b'D', "Failing row contains (null, a).".to_string()),
                (b's', "public".to_string()),
                (b't', "t_nn".to_string()),
                (b'c', "id".to_string()),
            ]
        );
    }

    #[test]
    fn the_index_size_refusals_send_18_6s_fields() {
        use crate::error_fields::IndexRowSize;
        let size = |size| {
            Box::new(IndexRowSize {
                size,
                index: "e1_pkey".into(),
                table: "e1".into(),
            })
        };
        // Measured on 18.6 over `ALTER TABLE e1 ADD PRIMARY KEY (a)`: the hint's two
        // lines, `s`/`t` and the **index** in `n`, and a `D` naming a heap `ctid` pglet
        // does not have, so it is omitted.
        assert_eq!(
            Error::BtreeIndexRowTooLarge(size(3016)).wire_fields(Severity::Error),
            vec![
                (b'S', "ERROR".to_string()),
                (b'V', "ERROR".to_string()),
                (b'C', "54000".to_string()),
                (
                    b'M',
                    "index row size 3016 exceeds btree version 4 maximum 2704 for index \"e1_pkey\""
                        .to_string()
                ),
                (
                    b'H',
                    "Values larger than 1/3 of a buffer page cannot be indexed.\nConsider a \
                     function index of an MD5 hash of the value, or use full text indexing."
                        .to_string()
                ),
                (b's', "public".to_string()),
                (b't', "e1".to_string()),
                (b'n', "e1_pkey".to_string()),
            ]
        );
        // GIN's item limit and `index_form_tuple`'s size field send none beside the
        // message (both measured).
        for (err, message) in [
            (
                Error::GinIndexRowTooLarge(size(2720)),
                "index row size 2720 exceeds maximum 2712 for index \"e1_pkey\"",
            ),
            (
                Error::IndexRowRequiresTooMuch { size: 9016 },
                "index row requires 9016 bytes, maximum size is 8191",
            ),
        ] {
            assert_eq!(
                err.wire_fields(Severity::Error),
                vec![
                    (b'S', "ERROR".to_string()),
                    (b'V', "ERROR".to_string()),
                    (b'C', "54000".to_string()),
                    (b'M', message.to_string()),
                ]
            );
        }
    }

    #[test]
    fn an_expression_index_conflict_keeps_the_table_and_drops_only_the_detail() {
        // Measured on 18.6 over `CREATE UNIQUE INDEX ex1_i ON ex1 (lower(a))`:
        // `s=public t=ex1 n=ex1_i D=Key (lower(a))=(a) already exists.` pglet cannot
        // derive that `D` — no spelled key name, no declared type for the value — but
        // the relation is known, so dropping `t` with it would omit a truthful field.
        let err = Error::UniqueViolation {
            constraint: "ex1_i".into(),
            key: Some(Box::new(UniqueViolationDetail {
                table: "ex1".into(),
                key: None,
            })),
        };
        let fields = err.wire_fields(Severity::Error);
        assert_eq!(field(&fields, b'D'), None);
        assert_eq!(field(&fields, b's'), Some("public"));
        assert_eq!(field(&fields, b't'), Some("ex1"));
        assert_eq!(field(&fields, b'n'), Some("ex1_i"));
    }

    #[test]
    fn unique_violation_sends_the_key_and_the_table() {
        let err = Error::UniqueViolation {
            constraint: "t_u_pkey".into(),
            key: Some(Box::new(UniqueViolationDetail {
                table: "t_u".into(),
                key: Some(KeyDescription::new(
                    vec!["id".into()],
                    vec![Some("1".into())],
                )),
            })),
        };
        let fields = err.wire_fields(Severity::Error);
        assert_eq!(
            field(&fields, b'M'),
            Some("duplicate key value violates unique constraint \"t_u_pkey\"")
        );
        assert_eq!(field(&fields, b'D'), Some("Key (id)=(1) already exists."));
        assert_eq!(field(&fields, b't'), Some("t_u"));
        assert_eq!(field(&fields, b'n'), Some("t_u_pkey"));
        assert_eq!(field(&fields, b'c'), None);
    }

    #[test]
    fn foreign_key_table_field_is_the_child_in_both_directions() {
        // The trap: on the delete side the *message* names the parent while `t`
        // names the child (`errtableconstraint(fk_rel, …)`).
        let insert = ForeignKeyViolationDetail {
            child_table: "c_fk".into(),
            parent_table: "p_fk".into(),
            constraint: "c_fk_a_fkey".into(),
            direction: FkDirection::Insert,
            key: KeyDescription::new(vec!["a".into()], vec![Some("99".into())]),
        };
        let err = Error::ForeignKeyViolation(Box::new(insert.clone()));
        let fields = err.wire_fields(Severity::Error);
        assert_eq!(
            field(&fields, b'M'),
            Some(
                "insert or update on table \"c_fk\" violates foreign key constraint \
                 \"c_fk_a_fkey\""
            )
        );
        assert_eq!(
            field(&fields, b'D'),
            Some("Key (a)=(99) is not present in table \"p_fk\".")
        );
        assert_eq!(field(&fields, b't'), Some("c_fk"));

        let delete = ForeignKeyViolationDetail {
            direction: FkDirection::Delete,
            key: KeyDescription::new(vec!["id".into()], vec![Some("1".into())]),
            ..insert
        };
        let err = Error::ForeignKeyViolation(Box::new(delete.clone()));
        let fields = err.wire_fields(Severity::Error);
        assert_eq!(
            field(&fields, b'M'),
            Some(
                "update or delete on table \"p_fk\" violates foreign key constraint \
                 \"c_fk_a_fkey\" on table \"c_fk\""
            )
        );
        assert_eq!(
            field(&fields, b'D'),
            Some("Key (id)=(1) is still referenced from table \"c_fk\".")
        );
        assert_eq!(field(&fields, b't'), Some("c_fk"));

        // 23001 drops the word "still".
        let err = Error::RestrictViolation(Box::new(delete));
        let fields = err.wire_fields(Severity::Error);
        assert_eq!(
            field(&fields, b'D'),
            Some("Key (id)=(1) is referenced from table \"c_fk\".")
        );
    }

    #[test]
    fn an_unimplementable_foreign_key_sends_its_pair_and_no_object_fields() {
        // `ATAddForeignKeyConstraint`'s `[SVCMDFLR]`: the names raw inside their quotes,
        // the types as `format_type_be` prints them, and no `errtable*` call.
        let err = Error::ForeignKeyCannotBeImplemented(Box::new(ForeignKeyTypeMismatch {
            constraint: "My FK".into(),
            column: "A b".into(),
            referenced_column: "k".into(),
            column_type: "text".into(),
            referenced_type: "integer".into(),
        }));
        assert_eq!(err.sqlstate(), "42804");
        let fields = err.wire_fields(Severity::Error);
        assert_eq!(
            field(&fields, b'M'),
            Some("foreign key constraint \"My FK\" cannot be implemented")
        );
        assert_eq!(
            field(&fields, b'D'),
            Some(
                "Key columns \"A b\" of the referencing table and \"k\" of the referenced \
                 table are of incompatible types: text and integer."
            )
        );
        let codes: Vec<u8> = fields.iter().map(|(code, _)| *code).collect();
        assert_eq!(codes, b"SVCMD".to_vec());
    }

    #[test]
    fn alter_validated_check_has_no_detail() {
        // 18.6's `ATRewriteTable` route sends `[SVCMstnFLR]` — a different message
        // and **no** `D`, because there is no single failing row to show.
        let err = Error::CheckConstraintNotSatisfied {
            table: "t_c3".into(),
            constraint: "t_c3_pos".into(),
        };
        let fields = err.wire_fields(Severity::Error);
        assert_eq!(
            field(&fields, b'M'),
            Some("check constraint \"t_c3_pos\" of relation \"t_c3\" is violated by some row")
        );
        assert_eq!(field(&fields, b'D'), None);
        assert_eq!(field(&fields, b't'), Some("t_c3"));
        assert_eq!(field(&fields, b'n'), Some("t_c3_pos"));
    }

    #[test]
    fn a_syntax_error_carries_a_codepoint_position_and_a_tokenless_error_carries_none() {
        let err = Error::Syntax {
            message: "syntax error at or near \"FROM\"".into(),
            position: Some(22),
        };
        assert_eq!(field(&err.wire_fields(Severity::Error), b'P'), Some("22"));
        assert_eq!(Error::DivisionByZero.position(), None);
        assert_eq!(
            field(&Error::DivisionByZero.wire_fields(Severity::Error), b'P'),
            None
        );
    }

    /// A **carrier's** modifier refusal sends `S`/`V`/`C`/`M` and nothing else — no `P`,
    /// where 18.6 sends one.
    ///
    /// The differential harness cannot see the **absence**: it scores an omitted field as
    /// agreement, so every one of the `types/refused_typmod_*` fixtures stays green with
    /// the `P` omitted, and nothing there detects that decision. The blindness runs one
    /// way only. What the comparison *would* see is a `P` pglet sends where 18.6 sends
    /// none, or sends at a different offset — an *invention*, and red there
    /// (`pglet_tests`' `ErrorFields::divergences`); one equal to the oracle's would agree.
    /// And it is live rather than vacuous at this window: the statement a non-waived
    /// fixture in that set pins carries a `P` on the oracle —
    /// `SELECT nosuchcol::char(0) FROM rtmc`
    /// (`types/refused_typmod_beats_missing_column_loud.sql`) is `22023` under a caret on
    /// 18.6 and under none here, measured — while the fixtures that waive the comparison
    /// do so with a justified `@no-error-fields`. So what no fixture can hold is the
    /// omission, and that is what this test stands in for.
    ///
    /// **The window is three SQLSTATEs wide, not one.** A carrier's reader is handed the
    /// modifier as *text*, so a `numeric` component is `int4`-coerced
    /// (`typmod_component_to_int4`) ahead of every range check and answers `22003` or
    /// `22P02` in place of the `22023` — `'ab'::char(0)` and `1.5::numeric(2147483648)`
    /// are both carried by this one claim, and 18.6 sends a `P` at each. Those two
    /// variants are *shared* — the integer-input path raises each from many other sites —
    /// so what the loop proves is the variant's omission rather than this window's alone.
    /// At this window they come from `typmod_component_to_int4`, whose only callers are
    /// `resolve_type_name`'s two private modifier readers (`parse_length_typmod`,
    /// `parse_numeric_typmod`), so the spanless signature above them is the same one and no
    /// span exists to send here either.
    ///
    /// `22023` is not the predicate either — `opt_float`'s two range refusals carry a `P`
    /// because the **parser** raises them from a reader that holds the number's own
    /// token. These come instead from `executor::type_resolution`'s `resolve_type_name`,
    /// a resolver that takes a type *name* and no span, so none of its four callers can
    /// hand one down. Two of those callers are the **parser** (`parser::expr`'s cast
    /// target and typed constant, where a cursor is in hand and the type-name reader they
    /// run — `parser::type_name` — is where `opt_float`'s `P` is
    /// filled) and two the executor (`executor::ddl`'s `CREATE TABLE`
    /// column walk and `ADD COLUMN`). So the omission is the resolver's signature, not a
    /// property of where the statement was read — the parser halves *could* thread a
    /// span if the `P` were ever wanted.
    #[test]
    fn a_carrier_modifier_refusal_sends_no_position() {
        for (err, code) in [
            (
                Error::CharacterLengthTooSmall { type_name: "char" },
                "22023",
            ),
            (
                Error::CharacterLengthTooLarge {
                    type_name: "varchar",
                },
                "22023",
            ),
            (Error::NumericPrecisionOutOfRange { precision: 0 }, "22023"),
            (Error::NumericScaleOutOfRange { scale: 1001 }, "22023"),
            (Error::InvalidNumericTypeModifier, "22023"),
            // `typmod_component_to_int4`'s two, which a `numeric` component earns ahead
            // of all five above — the same resolver, the same absent span, and no
            // fixture able to see the omission either.
            (Error::NumericOutOfRange("integer"), "22003"),
            (
                Error::InvalidInputSyntax {
                    type_name: "integer",
                    value: "1.5".to_string(),
                },
                "22P02",
            ),
        ] {
            let fields = err.wire_fields(Severity::Error);
            assert_eq!(field(&fields, b'C'), Some(code), "{err:?}");
            assert_eq!(field(&fields, b'P'), None, "{err:?}");
            assert_eq!(field(&fields, b'D'), None, "{err:?}");
            assert_eq!(field(&fields, b'H'), None, "{err:?}");
            for code in *b"stcn" {
                assert_eq!(
                    field(&fields, code),
                    None,
                    "field {} for {err:?}",
                    code as char
                );
            }
        }
        // The polarity control: the sibling `22023` the **parser** raises does carry one,
        // so the omission is the producer's and not the SQLSTATE's.
        assert_eq!(
            field(
                &Error::FloatPrecisionTooSmall { position: 10 }.wire_fields(Severity::Error),
                b'P'
            ),
            Some("10"),
        );
    }

    /// The sentences a carrier's refusal renders, byte for byte as 18.6 renders them —
    /// `anychar_typmodin`'s two bounds, and `numerictypmodin`'s precision, scale and
    /// arity. Each measured on the live 18.6, whose `LOCATION` for the very refusal
    /// asserted below names `anychar_typmodin`, `numerictypmodin` (the arity), or the
    /// `make_numeric_typmod_safe` `numerictypmodin` calls (the precision and the scale).
    ///
    /// The `types/refused_typmod_*` fixtures carry `@expect-message` for each, so this is
    /// the hermetic half rather than the only guard; what it adds is the bound each
    /// sentence names, which a fixture spells only in the one value it happens to use.
    #[test]
    fn a_carrier_modifier_refusal_renders_18_6s_sentence() {
        assert_eq!(
            Error::CharacterLengthTooSmall { type_name: "char" }.to_string(),
            "length for type char must be at least 1"
        );
        assert_eq!(
            Error::CharacterLengthTooLarge {
                type_name: "varchar"
            }
            .to_string(),
            "length for type varchar cannot exceed 10485760"
        );
        assert_eq!(
            Error::NumericPrecisionOutOfRange { precision: -1 }.to_string(),
            "NUMERIC precision -1 must be between 1 and 1000"
        );
        assert_eq!(
            Error::NumericScaleOutOfRange { scale: -1001 }.to_string(),
            "NUMERIC scale -1001 must be between -1000 and 1000"
        );
        assert_eq!(
            Error::InvalidNumericTypeModifier.to_string(),
            "invalid NUMERIC type modifier"
        );
    }

    #[test]
    fn no_error_ever_sends_the_c_source_fields() {
        // The other polarity of the omission rule: pglet has no C source tree, so
        // `F`/`L`/`R` would be inventions. A later "helpful" fill reddens here.
        for err in [
            Error::DivisionByZero,
            Error::NotNullViolation {
                table: "t".into(),
                column: "c".into(),
                row: None,
            },
            Error::Syntax {
                message: "syntax error at end of input".into(),
                position: Some(3),
            },
        ] {
            let fields = err.wire_fields(Severity::Error);
            for code in *b"FLRWpqd" {
                assert_eq!(
                    field(&fields, code),
                    None,
                    "field {} for {err:?}",
                    code as char
                );
            }
        }
    }

    #[test]
    fn truncate_shares_the_drop_message_and_sends_no_hint() {
        // Both statements word the message through the same string
        // (`"x" is not a table`) and only `DropErrorMsgWrongType` attaches a hint —
        // `truncate_check_rel` (`tablecmds.c:2404`) is message-only. Measured on 18.6
        // for an index and for a sequence. This is the polarity that catches a
        // "helpful" hint being added back to the TRUNCATE route.
        let make = |route| Error::WrongObjectType {
            relation: "t_wo_idx".into(),
            actual: RelationKind::Index,
            expected: RelationKind::Table,
            route,
        };
        let dropped = make(WrongTypeRoute::Drop);
        assert_eq!(dropped.to_string(), "\"t_wo_idx\" is not a table");
        assert_eq!(
            dropped.hint().as_deref(),
            Some("Use DROP INDEX to remove an index.")
        );
        let truncated = make(WrongTypeRoute::Truncate);
        assert_eq!(truncated.to_string(), "\"t_wo_idx\" is not a table");
        assert_eq!(truncated.hint(), None);
        assert!(truncated.object_location().is_empty());
    }

    #[test]
    fn the_open_wrong_kind_route_sends_the_kind_as_a_detail() {
        // The other 42809 wording — `validate_relation_kind` — puts the kind in `D`,
        // and `RelationKind`'s Display is already plural, so the sentence takes it
        // verbatim rather than appending an `s`.
        let err = Error::WrongRelationKind {
            relation: "t_wo_idx".into(),
            kind: RelationKind::Index,
        };
        assert_eq!(err.to_string(), "cannot open relation \"t_wo_idx\"");
        assert_eq!(
            err.detail().as_deref(),
            Some("This operation is not supported for indexes.")
        );
        assert_eq!(err.hint(), None);
    }

    #[test]
    fn an_error_holding_names_18_6_does_not_send_stays_bare() {
        // The second polarity of "omit rather than invent", from the other side: pglet
        // *has* the table and column here and must still send neither, because 18.6's
        // `42703` is `[SVCMPFLR]` on all four of its routes (measured). Likewise
        // `428C9`, which carries a detail and a hint but no `s`/`t`/`c`.
        let missing = Error::Catalog(crate::error::CatalogError::ColumnNotFound {
            table: "t".into(),
            column: "c".into(),
        });
        assert!(missing.object_location().is_empty());
        let generated = Error::GeneratedAlwaysInsert {
            table: "t_ga".into(),
            column: "a".into(),
        };
        assert!(generated.object_location().is_empty());
        assert_eq!(
            generated.detail().as_deref(),
            Some("Column \"a\" is an identity column defined as GENERATED ALWAYS.")
        );
        assert_eq!(
            generated.hint().as_deref(),
            Some("Use OVERRIDING SYSTEM VALUE to override.")
        );
        // The UPDATE route has the detail and **no** hint (measured).
        assert_eq!(
            Error::GeneratedAlwaysUpdate {
                table: "t_ga".into(),
                column: "a".into(),
            }
            .hint(),
            None
        );
    }

    #[test]
    fn severity_fills_both_s_and_v() {
        let fields = Error::DivisionByZero.wire_fields(Severity::Error);
        assert_eq!(field(&fields, b'S'), Some("ERROR"));
        assert_eq!(field(&fields, b'V'), Some("ERROR"));
        assert_eq!(Severity::Warning.as_str(), "WARNING");
    }

    fn field(fields: &[(u8, String)], code: u8) -> Option<&str> {
        fields
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, v)| v.as_str())
    }
}
