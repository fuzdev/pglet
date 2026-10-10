//! DROP-time dependency resolution — what a `DROP TABLE` / `ALTER TABLE … DROP
//! COLUMN` must take with it, and what it must refuse to take.
//!
//! Postgres decides this in `dependency.c`: dropping an object visits its
//! `pg_depend` edges, and the *kind* of edge decides the outcome. An AUTO (or
//! INTERNAL) edge means the dependent object dies silently with the target; a NORMAL
//! edge means `RESTRICT` (the default) refuses with `2BP01` and `CASCADE` drops it
//! too. `reportDependentObjects` only reports the objects reached over a NORMAL edge.
//!
//! pglet models no `pg_depend`, so the edge kinds are re-derived from the catalog.
//! Verified against a live 18.6 instance:
//!
//! | drop                                   | dependent                          | edge | outcome |
//! |----------------------------------------|------------------------------------|------|---------|
//! | `DROP TABLE p`                         | FK on another table → `p`          | NORMAL | `2BP01` / CASCADE drops the **constraint**, not the child table |
//! | `DROP TABLE p`                         | FK on `p` itself (self-reference)  | —    | dies with the table |
//! | `ALTER TABLE t DROP COLUMN c`          | CHECK on `t` mentioning `c`        | AUTO | auto-dropped |
//! | `ALTER TABLE t DROP COLUMN c`          | FK on `t` whose child columns hold `c` | AUTO | auto-dropped |
//! | `ALTER TABLE t DROP COLUMN c`          | FK (**any** table, incl. `t`) referencing `t(c)` | NORMAL | `2BP01` unless CASCADE |
//! | `DROP TABLE t` / `DROP COLUMN c`       | sequence `OWNED BY` `t` / `t.c`    | AUTO | dropped with the target |
//! | `DROP TABLE t` / `DROP COLUMN c` / `DROP SEQUENCE s` | a column default `nextval` of a sequence being dropped | NORMAL | `2BP01` unless CASCADE, which clears that column's default |
//! | `DROP SEQUENCE s`                      | the **identity** column `s` implements | INTERNAL | `2BP01`, **CASCADE included** |
//! | `DROP TYPE e`                          | a `CHECK`, a column `DEFAULT` or an index expression converting to `e` | NORMAL | `2BP01` / CASCADE drops the **constraint**, clears the **default**, drops the **index** |
//!
//! Rows 4–5 are why "the table owns it" is *not* the rule: a
//! self-referencing FK blocks its own table's referenced-column drop
//! (`selfr_parent_id_fkey on table selfr depends on column id of table selfr`)
//! exactly as an external one does. But when one FK sits on **both** sides of the
//! same column (`FOREIGN KEY (id) REFERENCES t(id)`), the AUTO edge wins and the drop
//! succeeds — `reportDependentObjects` skips any object reached over an AUTO edge *at
//! any point in the search*. [`Dependents::resolve`] reproduces that by subtracting
//! the auto-dropped set from the blocking set.
//!
//! The sequence rows carry the same shape one level out, and the **deletion set** is
//! what decides it. Dropping a table takes its owned sequences with it, so a column
//! default drawing from one of them is a dependent — unless that column is itself
//! inside the deletion set. Hence `CREATE TABLE t (a SERIAL); DROP TABLE t` succeeds
//! (the default dies with the table) while the same sequence named by a *second*
//! table's default is `2BP01`, and — a case neither reading predicts — the
//! **column**-scoped deletion set makes `CREATE TABLE q (id SERIAL, b INT DEFAULT
//! nextval('q_id_seq')); ALTER TABLE q DROP COLUMN id` `2BP01` too, because `q.b` sits
//! outside it. All measured on 18.6.
//!
//! The INTERNAL row is the one edge kind that does **not** yield to `CASCADE`:
//! `findDependentObjects` raises it at the outermost recursion level with no
//! `DropBehavior` test in sight, which is why [`Dependents::ensure_droppable`] tests
//! it *before* the `cascade` short-circuit rather than beside the NORMAL edges.
//!
//! The type row is an **expression** edge: a stored expression converting a non-constant
//! operand to an enum is persisted naming it (`planner::StringFunc::EnumIn`), and 18.6
//! records the same edge from the expression's owner (`recordDependencyOnExpr`). A column
//! *declaring* the enum is the other NORMAL edge to a type, and this resolver does not
//! enforce it — `EDGES.md` row 261; [`Dependents::type_columns`] reads it only so a
//! dependency message does not name a partial list.

use std::collections::{BTreeMap, HashSet};

use crate::catalog::{CheckConstraint, ColumnDefault, IndexKey, Schema, SequenceOwner};
use crate::planner::Expr;
use crate::{ColumnDefaultDependency, DropTargetName, Error, IdentitySequenceDependency, Result};
use pglet_core::TypeDependency;

/// What a `DROP` is removing, for both the dependency lookup and the error message.
#[derive(Debug, Clone, Copy)]
pub enum DropTarget<'a> {
    /// `DROP TABLE t`.
    Table { table: &'a str },
    /// `ALTER TABLE t DROP COLUMN c`.
    Column { table: &'a str, column: &'a str },
    /// `DROP SEQUENCE s`.
    Sequence { sequence: &'a str },
    /// `DROP TYPE e` — `name` is the enum's catalog key.
    Type { name: &'a str },
}

impl DropTarget<'_> {
    /// The owned, message-shaped name of this target (`table "par"` /
    /// `column "id" of table "par"` / `sequence "s"`).
    fn name(self) -> DropTargetName {
        match self {
            Self::Table { table } => DropTargetName::Table {
                table: table.to_string(),
            },
            Self::Column { table, column } => DropTargetName::Column {
                table: table.to_string(),
                column: column.to_string(),
            },
            Self::Sequence { sequence } => DropTargetName::Sequence {
                sequence: sequence.to_string(),
            },
            Self::Type { name } => DropTargetName::Type {
                type_name: name.to_string(),
            },
        }
    }

    /// Whether `(table, column)` is inside this drop's **deletion set** — the columns
    /// that go away with the target, and so cannot be dependents of it.
    ///
    /// A whole table takes every one of its columns; a column drop takes only itself;
    /// a sequence takes no column at all (a `SERIAL`'s column survives its sequence's
    /// `CASCADE`, keeping its rows and losing only its default — measured on 18.6).
    fn deletes_column(self, table: &str, column: &str) -> bool {
        match self {
            Self::Table { table: t } => t == table,
            Self::Column {
                table: t,
                column: c,
            } => t == table && c == column,
            Self::Sequence { .. } | Self::Type { .. } => false,
        }
    }
}

/// One foreign key, identified by the table that declares it plus its **position** in
/// that table's `foreign_keys` — enough to find it again for removal and to name it in
/// an error.
///
/// The position, not the name, is the identity, and that is **not** a restatement of
/// the naming gate. Postgres disambiguates a duplicate *auto* name (`ch_a_fkey` /
/// `ch_a_fkey1`) and rejects a duplicated *declared* one (`42710`) — pglet now does
/// both, through `ConstraintNames::mint_constraint`. But that gate has been incomplete
/// more than once (a route that claimed nothing, an auto-name that was gated instead of
/// disambiguated), and a name-keyed resolver would have turned each of those into
/// silent integrity loss rather than a duplicate row: a name-keyed `retain` dropped
/// *both* constraints when an AUTO edge shed one, silently un-enforcing the survivor,
/// and a name-keyed `HashSet` let an AUTO edge suppress an unrelated same-named NORMAL
/// one. Keying on position makes this type correct on its own terms whatever the gate
/// lets through.
///
/// The position is read in the same read-only window as the rest of [`Dependents`] and
/// consumed by [`Dependents::drop_from_catalog`] and, for a multi-name `DROP`, by
/// [`Dependents::drop_all_from_catalog`] — which is the one that must exist, because a
/// position computed against the pre-drop catalog is stale the moment a sibling target
/// has shortened the same table's list; nothing between the resolve and either consumer
/// mutates any
/// table's `foreign_keys` (the callers touch `schema.indexes` and storage only).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FkRef {
    /// The (child) table the constraint is declared on.
    pub table: String,
    /// The constraint's index into that table's `foreign_keys`.
    pub position: usize,
    /// The constraint name, for the error message.
    pub name: String,
}

/// One column whose `DEFAULT` is `nextval(<sequence>)` — Postgres's `pg_attrdef` →
/// sequence NORMAL edge, keyed by the names the catalog resolves it with.
///
/// A name is the identity here where it is not for [`FkRef`], and the asymmetry is not
/// an oversight: `(table, column)` *is* a column's identity in this catalog, and there
/// is exactly one default per column. Position would be the wrong key — a column drop
/// renumbers the survivors, so a position captured before the mutation names a
/// different column after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDefaultRef {
    /// The table holding the column.
    pub table: String,
    /// The column whose default draws from the sequence.
    pub column: String,
    /// The sequence the default names.
    pub sequence: String,
}

/// A persisted expression that converts to a `DROP TYPE`'s target — the NORMAL edge
/// from the expression's owner to the type.
///
/// A `CHECK` is identified by `(table, position)`, as [`FkRef`] identifies a foreign key
/// and for the same reason; the name is the message's. A default is keyed by its column
/// and an index by its name, each of which is that object's identity in this catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpressionRef {
    /// A `CHECK` constraint on `table`, at `position` in its `checks`.
    Check {
        table: String,
        position: usize,
        name: String,
    },
    /// A column's `DEFAULT`.
    Default { table: String, column: String },
    /// An index whose key expression or predicate holds the conversion.
    Index { name: String },
}

impl ExpressionRef {
    /// The dependent as 18.6's `getObjectDescription` spells it — identifiers unquoted.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Check { table, name, .. } => format!("constraint {name} on table {table}"),
            Self::Default { table, column } => {
                format!("default value for column {column} of table {table}")
            }
            Self::Index { name } => format!("index {name}"),
        }
    }
}

/// The objects that depend on a drop target, split by Postgres's edge kinds.
#[derive(Debug, Default)]
pub struct Dependents {
    /// Foreign keys removed **with** the target, no `CASCADE` needed (an AUTO edge):
    /// the target column is one of the constraint's own referencing columns.
    pub auto_foreign_keys: Vec<FkRef>,
    /// `CHECK` constraints on the target's table whose predicate mentions the dropped
    /// column (an AUTO edge). Identified by position in `Table.checks`.
    pub auto_checks: Vec<usize>,
    /// Sequences removed **with** the target (an AUTO edge): every sequence whose
    /// `owner` names the dropped table (any column) or the dropped column.
    ///
    /// Empty for a `DropTarget::Sequence` — the target names the sequence directly, so
    /// its removal is the statement's own job rather than a dependent's.
    pub auto_sequences: Vec<String>,
    /// Foreign keys that reference the target and are **not** removed with it (a
    /// NORMAL edge): `2BP01` unless `CASCADE`, which drops the constraint (and only
    /// the constraint — the child table and its rows survive).
    pub blocking_foreign_keys: Vec<FkRef>,
    /// Column defaults drawing from a sequence this drop removes, on columns **outside**
    /// the deletion set (a NORMAL edge): `2BP01` unless `CASCADE`, which clears the
    /// default and leaves the column and its rows.
    pub blocking_column_defaults: Vec<ColumnDefaultRef>,
    /// The identity column a `DROP SEQUENCE`'s target implements (an INTERNAL edge):
    /// `2BP01` **with or without** `CASCADE`.
    pub internal_identity_owner: Option<IdentitySequenceDependency>,
    /// The persisted expressions converting to a `DROP TYPE`'s target (a NORMAL edge):
    /// `2BP01` unless `CASCADE`, which drops each constraint and index and clears each
    /// default — the table, its columns and its rows stay.
    pub blocking_expressions: Vec<ExpressionRef>,
    /// The columns declaring a `DROP TYPE`'s target, as `(table, column)`. Not an edge
    /// this resolver enforces (`EDGES.md` row 261): read only because 18.6's dependency
    /// `DETAIL` would name them too, so a message naming the expressions alone would be
    /// a partial list.
    pub type_columns: Vec<(String, String)>,
}

impl Dependents {
    /// Resolve every catalog object that depends on `target`, read-only.
    ///
    /// Never fails: the caller decides what to do with the split via
    /// [`Self::ensure_droppable`]. A target that does not exist resolves to no
    /// dependents (the caller has already raised `42P01`/`42703`).
    #[must_use]
    pub fn resolve(schema: &Schema, target: DropTarget<'_>) -> Self {
        let mut dependents = Self::resolve_constraints(schema, target);
        dependents.auto_sequences = owned_sequences(schema, target);
        // The sequences this statement removes: the AUTO-dropped ones for a table or
        // column target, and the target itself for a `DROP SEQUENCE`. A column default
        // drawing from any of them is the `pg_attrdef` NORMAL edge — unless the column
        // is itself inside the deletion set.
        let removed: Vec<&str> = match target {
            DropTarget::Sequence { sequence } => vec![sequence],
            _ => dependents
                .auto_sequences
                .iter()
                .map(String::as_str)
                .collect(),
        };
        dependents.blocking_column_defaults = schema
            .tables
            .values()
            .flat_map(|t| t.columns.iter().map(move |c| (t, c)))
            .filter_map(|(t, c)| match &c.default {
                // An identity column has no `pg_attrdef` row in Postgres (its
                // `column_default` is NULL and `is_identity` is YES), so it contributes
                // no NORMAL edge and has no default to shed — its sequence reaches it
                // over the INTERNAL edge below instead. What earns the exemption is
                // being *that* sequence's column, though, not carrying the `identity`
                // flag: pglet models the link as an ordinary `NextVal` default, so an
                // identity column can be left pointing at a sequence it does not own,
                // which is a state Postgres cannot represent and for which "no
                // `pg_attrdef` row" is simply false. Exempting on the flag alone made
                // every such default invisible, and the sequence then dropped out from
                // under it (see [`identity_sequence_owner`]).
                Some(ColumnDefault::NextVal(seq))
                    if removed.iter().any(|r| *r == seq)
                        && !is_identity_implementation(schema, &t.name, &c.name, seq) =>
                {
                    Some(ColumnDefaultRef {
                        table: t.name.clone(),
                        column: c.name.clone(),
                        sequence: seq.clone(),
                    })
                }
                // A general expression default carries the *same* `pg_attrdef` NORMAL
                // edge for every sequence it draws from — the edge is recorded per
                // `(attrdef, sequence)` pair, not per top-level node — so a
                // `DEFAULT nextval('sq1') + 1` blocks `DROP SEQUENCE sq1` exactly as
                // the bare spelling does (measured on 18.6: `2BP01`, and `CASCADE`
                // sheds the **whole** default, which is what the shed above does).
                // Reading only the closed arm made every such reference invisible, and
                // the sequence then dropped out from under a default that named it.
                //
                // No identity exemption here: an identity column's default is minted
                // by the engine as the closed `NextVal` arm and can never be an
                // expression, so the `is_identity_implementation` test above has
                // nothing to answer for on this arm.
                Some(default @ ColumnDefault::Expr(_)) => {
                    let refs = default_sequence_refs(default);
                    removed
                        .iter()
                        .find(|r| refs.iter().any(|s| s == *r))
                        .map(|seq| ColumnDefaultRef {
                            table: t.name.clone(),
                            column: c.name.clone(),
                            sequence: (*seq).to_string(),
                        })
                }
                _ => None,
            })
            .filter(|r| !target.deletes_column(&r.table, &r.column))
            .collect();
        dependents.internal_identity_owner = identity_owner(schema, target);
        if let DropTarget::Type { name } = target {
            dependents.blocking_expressions = expressions_converting_to(schema, name);
            dependents.type_columns = schema
                .tables
                .values()
                .flat_map(|t| t.columns.iter().map(move |c| (t, c)))
                .filter(|(_, c)| c.enum_type.as_deref() == Some(name))
                .map(|(t, c)| (t.name.clone(), c.name.clone()))
                .collect();
        }
        dependents
    }

    /// The constraint half of [`Self::resolve`] — the foreign keys and `CHECK`s,
    /// which only a table or column target can reach (a sequence has neither).
    fn resolve_constraints(schema: &Schema, target: DropTarget<'_>) -> Self {
        match target {
            DropTarget::Sequence { .. } | DropTarget::Type { .. } => Self::default(),
            DropTarget::Table { table } => Self {
                // A table's own FKs (including a self-reference) die with it, so only
                // FKs declared *elsewhere* block.
                blocking_foreign_keys: schema
                    .referencing_foreign_keys(table)
                    .filter(|(child, _, _)| child.name != table)
                    .map(|(child, position, fk)| FkRef {
                        table: child.name.clone(),
                        position,
                        name: fk.name.clone(),
                    })
                    .collect(),
                ..Self::default()
            },
            DropTarget::Column { table, column } => {
                let auto_foreign_keys: Vec<FkRef> = schema
                    .get_table(table)
                    .into_iter()
                    .flat_map(|t| {
                        t.foreign_keys
                            .iter()
                            .enumerate()
                            .map(move |(i, fk)| (t, i, fk))
                    })
                    .filter(|(_, _, fk)| fk.columns.iter().any(|c| c == column))
                    .map(|(t, i, fk)| FkRef {
                        table: t.name.clone(),
                        position: i,
                        name: fk.name.clone(),
                    })
                    .collect();
                // An FK reached over the AUTO edge above is never reported again over
                // its NORMAL (referenced-column) edge — Postgres's "if, at any stage
                // of the recursive search, we reached the object via an AUTO …
                // dependency, then it's okay to delete it even in RESTRICT mode".
                let auto: HashSet<&FkRef> = auto_foreign_keys.iter().collect();
                let blocking_foreign_keys: Vec<FkRef> = schema
                    .referencing_foreign_keys(table)
                    .filter(|(_, _, fk)| fk.parent_columns.iter().any(|c| c == column))
                    .map(|(child, position, fk)| FkRef {
                        table: child.name.clone(),
                        position,
                        name: fk.name.clone(),
                    })
                    .filter(|r| !auto.contains(r))
                    .collect();
                let auto_checks = schema
                    .get_table(table)
                    .map(|t| {
                        t.checks
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| check_references_column(c, column))
                            .map(|(i, _)| i)
                            .collect()
                    })
                    .unwrap_or_default();
                Self {
                    auto_foreign_keys,
                    auto_checks,
                    blocking_foreign_keys,
                    ..Self::default()
                }
            }
        }
    }

    /// Raise `2BP01` when a dependent object outside the deletion set remains and no
    /// `CASCADE` was given.
    ///
    /// `RESTRICT` and a bare `DROP` behave identically (Postgres's default *is*
    /// `RESTRICT`); the parser folds them, passing `cascade = false` for both.
    ///
    /// **The INTERNAL edge is tested before the `cascade` short-circuit**, and that
    /// placement is the whole rule rather than an ordering preference: Postgres raises
    /// it in `findDependentObjects` at the outermost recursion level with no
    /// `DropBehavior` test anywhere in the path, so `DROP SEQUENCE <an identity
    /// column's sequence> CASCADE` is `2BP01` exactly like the bare spelling (measured
    /// on 18.6). Inside the `cascade` arm it would silently become a successful drop
    /// that leaves an identity column with no sequence.
    ///
    /// Between the two NORMAL edges there is no **kind** order to reproduce, so the
    /// choice decides only which dependent pglet names. The foreign key goes first,
    /// keeping every shape that predates the sequence edges reporting exactly what it
    /// did.
    ///
    /// What Postgres does instead is report *every* blocking dependent in one message,
    /// in an order neither kind- nor OID-keyed. `findDependentObjects` sorts each
    /// recursion level's dependents with `object_address_comparator` — **OID
    /// descending** — walks them depth-first appending each object *after* its own
    /// dependents, and `reportDependentObjects` then reads the array **back to front**.
    /// So the sort direction and the report direction cancel, and a dependent reached
    /// one level deeper takes its position from the *object it hangs off*, not from its
    /// own OID: a column default is reached through the sequence, so it is placed by
    /// the **sequence's** OID. Measured on 18.6, dropping a table that both owns a
    /// sequence another table defaults from and is referenced by a foreign key:
    ///
    /// | sequence vs FK constraint OID | reported first |
    /// |-------------------------------|----------------|
    /// | `seq` < `fkey` (default's own OID *below* the FK's) | the column default |
    /// | `fkey` < `seq` | the FK constraint |
    /// | `seq` < `fkey` < the default's own OID | the column default |
    ///
    /// The first two falsify "ordered by kind"; the first and third together falsify
    /// "ordered by the dependent's own OID" in either direction. pglet naming one
    /// dependent is therefore a divergence in *completeness*, not in ordering, and
    /// nothing about Postgres's order argues for or against which one it picks.
    ///
    /// # Errors
    ///
    /// [`Error::SequenceRequiredByIdentity`], [`Error::DependentForeignKey`],
    /// [`Error::DependentColumnDefault`] or [`Error::DependentOnType`] — all `2BP01`.
    pub fn ensure_droppable(&self, target: DropTarget<'_>, cascade: bool) -> Result<()> {
        if let Some(owner) = &self.internal_identity_owner {
            return Err(Error::SequenceRequiredByIdentity(Box::new(owner.clone())));
        }
        if cascade {
            return Ok(());
        }
        if let Some(fk) = self.blocking_foreign_keys.first() {
            return Err(Error::DependentForeignKey {
                target: Box::new(target.name()),
                constraint: fk.name.clone(),
                child_table: fk.table.clone(),
            });
        }
        if let Some(dep) = self.blocking_column_defaults.first() {
            return Err(Error::DependentColumnDefault(Box::new(
                ColumnDefaultDependency {
                    target: target.name(),
                    table: dep.table.clone(),
                    column: dep.column.clone(),
                    sequence: dep.sequence.clone(),
                },
            )));
        }
        if let (DropTarget::Type { name }, [first, rest @ ..]) =
            (target, self.blocking_expressions.as_slice())
        {
            let only = rest.is_empty() && self.type_columns.is_empty();
            return Err(Error::DependentOnType(Box::new(TypeDependency {
                type_name: name.to_string(),
                dependent: only.then(|| first.describe()),
            })));
        }
        Ok(())
    }

    /// Remove the dependents this drop takes with it: the AUTO-edge ones always, plus
    /// the blocking ones under `CASCADE`.
    ///
    /// A `CASCADE` past a NORMAL edge drops the *dependent object*, never the relation
    /// holding it — the FK edge sheds the **constraint** and leaves the child table
    /// populated and thereafter unconstrained, and the `pg_attrdef` edge sheds the
    /// **default** and leaves the column and its rows (both measured against 18.6).
    ///
    /// Constraint removals are **by position, high-to-low** so earlier positions stay
    /// valid — and, for the foreign keys, because the position is the identity
    /// ([`FkRef`]): a `retain(|f| f.name != …)` here once shed a same-named sibling
    /// along with its target, silently un-enforcing a constraint no one asked to drop.
    /// A column default is keyed by name instead, for the reason [`ColumnDefaultRef`]
    /// gives.
    ///
    /// The AUTO-dropped **sequences** go here rather than in the callers, so a route
    /// that resolves dependents cannot forget to shed them; leaving one behind is not a
    /// tidy-up matter, since a sequence outliving its owner dumps to an
    /// `ALTER SEQUENCE … OWNED BY` naming a relation the script never creates.
    pub fn drop_from_catalog(&self, schema: &mut Schema, target: DropTarget<'_>, cascade: bool) {
        Self::drop_all_from_catalog(schema, std::slice::from_ref(self), cascade);
        // The `CHECK` positions are the one part that is not self-describing: they
        // index `Table.checks` on the *target's* table, which the struct does not
        // record, so only a caller holding the target can apply them. That is also
        // why `drop_all_from_catalog` debug-asserts against a multi-target set
        // carrying any — see its doc.
        if let DropTarget::Column { table, .. } = target
            && let Some(t) = schema.get_table_mut(table)
        {
            for &i in self.auto_checks.iter().rev() {
                if i < t.checks.len() {
                    t.checks.remove(i);
                }
            }
        }
    }

    /// [`Self::drop_from_catalog`] over **several** targets at once, as **one** merged
    /// removal rather than one call per target.
    ///
    /// # Why this cannot be a loop over `drop_from_catalog`
    ///
    /// [`FkRef`] keys on `(table, position)`, and the positions are read in one
    /// read-only window *before* anything mutates. Removing them high-to-low keeps the
    /// earlier positions of **that call** valid — it says nothing about a *second*
    /// call's, which were computed against the same pre-mutation catalog and are stale
    /// the moment the first call shortens a `foreign_keys` vector. The guard
    /// `if i < child.foreign_keys.len()` then **silently no-ops** instead of failing
    /// loud. Measured shape, accepted DDL on both engines:
    ///
    /// ```sql
    /// CREATE TABLE ca (id INT PRIMARY KEY);
    /// CREATE TABLE cb (id INT PRIMARY KEY);
    /// CREATE TABLE cext (x INT REFERENCES ca, y INT REFERENCES cb);
    /// DROP TABLE ca, cb CASCADE;
    /// ```
    ///
    /// Per target: dropping `ca` sheds `cext`'s position 0, sliding the `cb` constraint
    /// down to 0; the second call asks for position 1 and does nothing, leaving
    /// `cext_y_fkey` referencing a table that no longer exists — the exact dangling
    /// state [`Self::ensure_droppable`]'s refusal exists to prevent. 18.6 sheds both.
    /// Merging first makes every position in the set current against the one catalog
    /// they were all read from.
    ///
    /// The `CHECK` positions are **not** merged: they carry no table, so a set
    /// containing any would be ambiguous. No caller produces one — only a
    /// [`DropTarget::Column`] resolves checks, and pglet has no multi-column
    /// `DROP COLUMN` — and the debug assertion below keeps it that way rather than
    /// leaving the omission to a comment.
    pub fn drop_all_from_catalog(schema: &mut Schema, all: &[Self], cascade: bool) {
        debug_assert!(
            all.len() == 1 || all.iter().all(|d| d.auto_checks.is_empty()),
            "a merged removal cannot place CHECK positions: they index the target's own table",
        );
        let fks = all.iter().flat_map(|d| {
            let blocking: &[FkRef] = if cascade {
                &d.blocking_foreign_keys
            } else {
                &[]
            };
            d.auto_foreign_keys.iter().chain(blocking)
        });
        // Group by table so each table's positions can be removed high-to-low; one
        // drop can reach several constraints on the same table (two child-side FKs
        // over the dropped column) and several tables (a CASCADE fan-in) — and several
        // *targets* can reach the same table, which is what makes the grouping have to
        // span the whole set.
        let mut by_table: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for fk in fks {
            by_table
                .entry(fk.table.as_str())
                .or_default()
                .push(fk.position);
        }
        for (table, mut positions) in by_table {
            let Some(child) = schema.get_table_mut(table) else {
                continue;
            };
            positions.sort_unstable();
            positions.dedup();
            for &i in positions.iter().rev() {
                if i < child.foreign_keys.len() {
                    child.foreign_keys.remove(i);
                }
            }
        }
        if cascade {
            for dep in all.iter().flat_map(|d| &d.blocking_column_defaults) {
                if let Some(t) = schema.get_table_mut(&dep.table)
                    && let Some(c) = t.columns.iter_mut().find(|c| c.name == dep.column)
                {
                    c.default = None;
                }
            }
            drop_expressions_from_catalog(schema, all.iter().flat_map(|d| &d.blocking_expressions));
        }
        for name in all.iter().flat_map(|d| &d.auto_sequences) {
            schema.sequences.remove(name);
        }
    }

    /// Drop every **blocking** dependent whose *owning* relation is itself one of this
    /// statement's targets — Postgres's reason for resolving all the relations before
    /// deleting any (`RemoveRelations`: *"to avoid unwanted DROP RESTRICT errors if one
    /// of the relations depends on another"*).
    ///
    /// `targets` is the set of relation names the statement is dropping. The relation
    /// that must be *in* it is the one the dependent is declared **on** — the child
    /// table of a foreign key, the table holding a defaulted column — never the one it
    /// references, which is the target itself and is in the set by construction.
    /// Measured on 18.6: `DROP TABLE f1, f2` succeeds over a mutual FK cycle that
    /// refuses in either single-target order, and `DROP TABLE sa, sb` succeeds where
    /// `sb.x DEFAULT nextval('sa_id_seq')` makes `DROP TABLE sa` alone `2BP01`.
    ///
    /// It is a **one-pass** filter, not a fixed point: `DROP TABLE … CASCADE` never
    /// pulls a new *relation* into the deletion set (it sheds a constraint or a
    /// default on one outside it), unlike `TRUNCATE … CASCADE`, whose closure grows.
    /// Inert for a sequence target set, whose blocking column defaults are owned by
    /// tables and whose foreign keys are empty — a sequence and a table cannot share a
    /// name ([`Schema::relation_name_taken`]), so no such entry can match.
    pub fn retain_dependents_outside(&mut self, targets: &HashSet<&str>) {
        self.blocking_foreign_keys
            .retain(|fk| !targets.contains(fk.table.as_str()));
        self.blocking_column_defaults
            .retain(|d| !targets.contains(d.table.as_str()));
    }
}

/// Remove the [`ExpressionRef`]s a `CASCADE` takes: each `CHECK` by position,
/// high-to-low per table (the reason [`Dependents::drop_all_from_catalog`] gives for the
/// foreign keys), each default cleared, each index's **catalog** entry dropped. An index's
/// storage is the caller's to drop, ahead of this, being the one fallible part.
fn drop_expressions_from_catalog<'a>(
    schema: &mut Schema,
    refs: impl Iterator<Item = &'a ExpressionRef>,
) {
    let mut checks: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for r in refs {
        match r {
            ExpressionRef::Check {
                table, position, ..
            } => checks.entry(table.as_str()).or_default().push(*position),
            ExpressionRef::Default { table, column } => {
                if let Some(t) = schema.get_table_mut(table)
                    && let Some(c) = t.columns.iter_mut().find(|c| c.name == *column)
                {
                    c.default = None;
                }
            }
            ExpressionRef::Index { name } => {
                schema.drop_index(name);
            }
        }
    }
    for (table, mut positions) in checks {
        let Some(t) = schema.get_table_mut(table) else {
            continue;
        };
        positions.sort_unstable();
        positions.dedup();
        for &i in positions.iter().rev() {
            if i < t.checks.len() {
                t.checks.remove(i);
            }
        }
    }
}

/// Every persisted expression converting to the enum `key`
/// ([`crate::planner::enum_in_parts`]) — each table's `CHECK`s and column `DEFAULT`s in
/// catalog order, then the indexes' predicates and key expressions.
fn expressions_converting_to(schema: &Schema, key: &str) -> Vec<ExpressionRef> {
    let mut found = Vec::new();
    for table in schema.tables.values() {
        for (position, check) in table.checks.iter().enumerate() {
            if expr_converts_to_enum(&check.expr, key) {
                found.push(ExpressionRef::Check {
                    table: table.name.clone(),
                    position,
                    name: check.name.clone(),
                });
            }
        }
        for column in &table.columns {
            if let Some(ColumnDefault::Expr(expr)) = &column.default
                && expr_converts_to_enum(expr, key)
            {
                found.push(ExpressionRef::Default {
                    table: table.name.clone(),
                    column: column.name.clone(),
                });
            }
        }
    }
    for index in schema.indexes.values() {
        let keys = match &index.key {
            IndexKey::Exprs(exprs) => exprs.as_slice(),
            IndexKey::Columns(_) => &[],
        };
        if index
            .predicate
            .iter()
            .chain(keys)
            .any(|e| expr_converts_to_enum(e, key))
        {
            found.push(ExpressionRef::Index {
                name: index.name.clone(),
            });
        }
    }
    found
}

/// Whether `expr` holds a conversion to the enum `key` anywhere.
fn expr_converts_to_enum(expr: &Expr, key: &str) -> bool {
    if crate::planner::enum_in_parts(expr).is_some_and(|(_, name, _)| name == key) {
        return true;
    }
    let mut found = false;
    expr.for_each_child(|child| {
        if !found && expr_converts_to_enum(child, key) {
            found = true;
        }
    });
    found
}

/// The sequences an AUTO edge takes with `target` — Postgres's `getOwnedSequences`,
/// read off the persisted `Sequence.owner`.
///
/// The owner edge is what decides this, not the columns' `NextVal` defaults: a
/// sequence a column merely *references* is nobody's dependent, while an owned one
/// dies with its owner whether or not any default names it.
///
/// Two readers: [`Dependents::resolve`], for what the drop removes, and
/// `Plan::swapped_sequences`, for the counter cells a rollback of that drop puts back —
/// one derivation, so the cells a rollback restores are the cells the drop removed.
pub fn owned_sequences(schema: &Schema, target: DropTarget<'_>) -> Vec<String> {
    let owned_by_target = |owner: &SequenceOwner| match target {
        DropTarget::Table { table } => owner.table == table,
        DropTarget::Column { table, column } => owner.table == table && owner.column == column,
        DropTarget::Sequence { .. } | DropTarget::Type { .. } => false,
    };
    schema
        .sequences
        .iter()
        .filter(|(_, s)| s.owner.as_ref().is_some_and(owned_by_target))
        .map(|(name, _)| name.clone())
        .collect()
}

/// The identity column a `DROP SEQUENCE`'s target is the internal implementation of,
/// if any — Postgres's INTERNAL edge (`sequence.c`'s `DEPENDENCY_INTERNAL`).
fn identity_owner(schema: &Schema, target: DropTarget<'_>) -> Option<IdentitySequenceDependency> {
    let DropTarget::Sequence { sequence } = target else {
        return None;
    };
    identity_sequence_owner(schema, sequence)
}

/// The identity column `sequence` is the INTERNAL implementation of, if any —
/// Postgres's `sequenceIsOwned(seq, DEPENDENCY_INTERNAL, …)`.
///
/// Both halves are required, and requiring only the first is the near-miss: the
/// sequence must be `OWNED BY` an **identity** column *and* be the one that column's
/// `DEFAULT` draws from. An `ALTER SEQUENCE … OWNED BY` can point any sequence at any
/// column, and one merely pointed at an identity column is not its implementation —
/// dropping it is an ordinary drop, not the refusal.
///
/// This is the engine's **one** derivation of that edge, and it has three consumers
/// that must not drift: the `DROP SEQUENCE` refusal ([`identity_owner`]), the
/// `pg_attrdef` exemption ([`is_identity_implementation`]), and the
/// `ALTER SEQUENCE … OWNED BY` gate (`executor::ddl::execute_alter_sequence`). The
/// last is what keeps the other two honest — it is the reason a live catalog answers
/// "yes" for every identity column, so the exemption really is Postgres's "no
/// `pg_attrdef` row" rather than a name for a state that can be broken apart.
///
/// The dump emitter is **deliberately not a fourth consumer**, which is worth stating
/// because it renders this very edge and so looks like one. It asks the *column*-side
/// question — is this column an identity column, and what does it draw from — which
/// the column's own `identity` + `NextVal` answer, and it does **not** require the
/// sequence's `owner` to point back. The two agree on every catalog this engine's DDL
/// can build, and where a legacy store has them apart the emitter's wider answer is
/// the safe one: it re-records the edge, where this narrower one would say "not an
/// identity sequence" and let the restore drop the identity fact silently. Sharing
/// one predicate would force one of those two behaviours onto the other's callers.
#[must_use]
pub fn identity_sequence_owner(
    schema: &Schema,
    sequence: &str,
) -> Option<IdentitySequenceDependency> {
    let owner = schema.sequences.get(sequence)?.owner.as_ref()?;
    let column = schema
        .get_table(&owner.table)?
        .columns
        .iter()
        .find(|c| c.name == owner.column)?;
    if column.identity.is_none()
        || !matches!(&column.default, Some(ColumnDefault::NextVal(s)) if s == sequence)
    {
        return None;
    }
    Some(IdentitySequenceDependency {
        sequence: sequence.to_string(),
        table: owner.table.clone(),
        column: owner.column.clone(),
    })
}

/// Every sequence a column `DEFAULT` draws from, in tree order.
///
/// The engine's **one** derivation of the `pg_attrdef` → sequence reference, over
/// both the closed [`ColumnDefault::NextVal`] arm and a general
/// [`ColumnDefault::Expr`].
///
/// Its readers ask three different questions of it and must not answer them from
/// three walks. *Does this default reference a sequence this statement is removing* —
/// [`Dependents::resolve`], the NORMAL edge `DROP SEQUENCE` refuses with `2BP01`.
/// *Does it reference one at all* — `pglet::dump`'s `default_names_a_sequence`
/// (whether the default is hoisted out of the `CREATE TABLE`, so no table's creation
/// names a sequence), and `cook_column_default`'s eager/deferred split (a draw is not
/// a value the cook may probe eagerly). *Which sequences does it name* —
/// `execute_create_table` and `execute_alter_table_add_column`, which claim and
/// register each of them, and the latter's backfill, which draws from each. A default
/// one of them saw as sequence-free would be emitted ahead of its own
/// `CREATE SEQUENCE`, left holding a dropped one, or minted over.
///
/// **Every sequence function counts, not `nextval` alone.** Postgres records the
/// dependency from the `regclass` **constant** in the cooked expression, so
/// `DEFAULT currval('s')` carries it too, and reading only `nextval` would let `s`
/// drop out from under one.
#[must_use]
pub fn default_sequence_refs(default: &ColumnDefault) -> Vec<String> {
    let mut names = Vec::new();
    match default {
        ColumnDefault::NextVal(seq) => names.push(seq.clone()),
        ColumnDefault::Expr(expr) => collect_sequence_refs(expr, &mut names),
        _ => {}
    }
    names
}

/// [`default_sequence_refs`]' tree walk.
fn collect_sequence_refs(expr: &Expr, names: &mut Vec<String>) {
    if let Expr::SequenceFn { seq, .. } = expr {
        names.push(seq.clone());
    }
    expr.for_each_child(|child| collect_sequence_refs(child, names));
}

/// Whether `table.column` is the identity column `sequence` INTERNALLY implements —
/// the exemption [`Dependents::resolve`] applies to the `pg_attrdef` NORMAL edge.
fn is_identity_implementation(schema: &Schema, table: &str, column: &str, sequence: &str) -> bool {
    identity_sequence_owner(schema, sequence)
        .is_some_and(|owner| owner.table == table && owner.column == column)
}

/// Whether a `CHECK` predicate references `column` anywhere in its expression tree.
///
/// Matches on the bare name, ignoring any table qualifier — the same rule
/// `check_stored_expr_node_binding` uses when it validates a stored expression, so a
/// `CHECK (t.a > 0)` written with a qualifier is auto-dropped with `a` rather than
/// left behind to fail every later write.
fn check_references_column(check: &CheckConstraint, column: &str) -> bool {
    expr_references_column(&check.expr, column)
}

/// Whether `expr` (or any scalar sub-expression) is a column reference named
/// `column`.
fn expr_references_column(expr: &Expr, column: &str) -> bool {
    if matches!(expr, Expr::Column { name, .. } if name == column) {
        return true;
    }
    let mut found = false;
    expr.for_each_child(|child| {
        if !found && expr_references_column(child, column) {
            found = true;
        }
    });
    found
}

#[cfg(test)]
mod tests {
    use super::{Dependents, DropTarget};
    use crate::catalog::{
        CheckConstraint, Column, ColumnDefault, ForeignKey, IdentityGeneration, ReferentialAction,
        Schema, Sequence, Table,
    };
    use crate::planner::{BinaryOp, Expr, Literal};
    use crate::types::TypeOid;

    fn col(name: &str, position: i16) -> Column {
        Column {
            name: name.to_string(),
            type_oid: TypeOid::Int4,
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

    fn fk(name: &str, columns: &[&str], parent: &str, parent_columns: &[&str]) -> ForeignKey {
        ForeignKey {
            name: name.to_string(),
            columns: columns.iter().map(|c| (*c).to_string()).collect(),
            parent_table: parent.to_string(),
            parent_columns: parent_columns.iter().map(|c| (*c).to_string()).collect(),
            on_delete: ReferentialAction::NoAction,
            on_update: ReferentialAction::NoAction,
        }
    }

    /// `CHECK (<qualifier?>.<name> > 0)`.
    fn check(name: &str, qualifier: Option<&str>, column: &str) -> CheckConstraint {
        CheckConstraint {
            name: name.to_string(),
            expr: Expr::BinaryOp {
                left: Box::new(Expr::Column {
                    table: qualifier.map(str::to_string),
                    name: column.to_string(),
                }),
                op: BinaryOp::Gt,
                right: Box::new(Expr::Literal(Literal::Int(0))),
            },
        }
    }

    fn table(name: &str, columns: &[&str]) -> Table {
        Table {
            name: name.to_string(),
            schema: "public".to_string(),
            columns: columns
                .iter()
                .enumerate()
                .map(|(i, c)| col(c, (i + 1) as i16))
                .collect(),
            oid: 1,
            checks: Vec::new(),
            foreign_keys: Vec::new(),
        }
    }

    /// `idt(id GENERATED ALWAYS AS IDENTITY, b)` with its sequence named `sequence`,
    /// owned by `idt.id` and drawn from by it — the catalog a live `CREATE TABLE`
    /// produces, and the one the INTERNAL edge is defined against.
    fn identity_schema(sequence: &str) -> Schema {
        let mut schema = Schema::new();
        let mut t = table("idt", &["id", "b"]);
        t.columns[0].identity = Some(IdentityGeneration::Always);
        t.columns[0].default = Some(ColumnDefault::NextVal(sequence.to_string()));
        schema.create_table(t);
        schema
            .sequences
            .insert(sequence.to_string(), Sequence::owned_by(1, "idt", "id"));
        schema
    }

    /// `par(id, code)` ← `ch(cid, pid)` via `ch_pid_fkey`.
    fn parent_child() -> Schema {
        let mut schema = Schema::new();
        schema.create_table(table("par", &["id", "code"]));
        let mut ch = table("ch", &["cid", "pid"]);
        ch.foreign_keys
            .push(fk("ch_pid_fkey", &["pid"], "par", &["id"]));
        schema.create_table(ch);
        schema
    }

    #[test]
    fn dropping_a_referenced_table_is_blocked_by_the_external_fk() {
        let schema = parent_child();
        let d = Dependents::resolve(&schema, DropTarget::Table { table: "par" });
        assert_eq!(d.blocking_foreign_keys.len(), 1);
        assert_eq!(d.blocking_foreign_keys[0].name, "ch_pid_fkey");
        assert_eq!(d.blocking_foreign_keys[0].table, "ch");
        let target = DropTarget::Table { table: "par" };
        assert!(d.ensure_droppable(target, false).is_err());
        assert!(d.ensure_droppable(target, true).is_ok());
    }

    #[test]
    fn cascade_drops_only_the_constraint_not_the_child_table() {
        let mut schema = parent_child();
        let target = DropTarget::Table { table: "par" };
        let d = Dependents::resolve(&schema, target);
        d.drop_from_catalog(&mut schema, target, true);
        let ch = schema
            .get_table("ch")
            .expect("child table survives CASCADE");
        assert!(
            ch.foreign_keys.is_empty(),
            "the child's FK constraint is what CASCADE sheds"
        );
        assert_eq!(ch.columns.len(), 2, "the child keeps its columns");
    }

    #[test]
    fn a_self_referencing_fk_never_blocks_its_own_tables_drop() {
        let mut schema = Schema::new();
        let mut t = table("selfr", &["id", "parent_id"]);
        t.foreign_keys
            .push(fk("selfr_parent_id_fkey", &["parent_id"], "selfr", &["id"]));
        schema.create_table(t);
        let d = Dependents::resolve(&schema, DropTarget::Table { table: "selfr" });
        assert!(d.blocking_foreign_keys.is_empty());
    }

    #[test]
    fn dropping_a_child_side_fk_column_auto_drops_the_constraint() {
        let schema = parent_child();
        let d = Dependents::resolve(
            &schema,
            DropTarget::Column {
                table: "ch",
                column: "pid",
            },
        );
        assert_eq!(d.auto_foreign_keys.len(), 1);
        assert!(d.blocking_foreign_keys.is_empty());
    }

    #[test]
    fn dropping_a_referenced_column_is_blocked_even_by_a_self_reference() {
        // Postgres: `selfr_parent_id_fkey on table selfr depends on column id of
        // table selfr` — the constraint's edge to a *referenced* column is NORMAL
        // whichever table declares it, so "the table owns it" is not the rule.
        let mut schema = Schema::new();
        let mut t = table("selfr", &["id", "parent_id"]);
        t.foreign_keys
            .push(fk("selfr_parent_id_fkey", &["parent_id"], "selfr", &["id"]));
        schema.create_table(t);
        let d = Dependents::resolve(
            &schema,
            DropTarget::Column {
                table: "selfr",
                column: "id",
            },
        );
        assert_eq!(d.blocking_foreign_keys.len(), 1);
        assert!(d.auto_foreign_keys.is_empty());
    }

    #[test]
    fn an_auto_edge_beats_a_normal_edge_on_the_same_constraint() {
        // `FOREIGN KEY (id) REFERENCES t(id)` puts one constraint on both sides of
        // `id`. Postgres reaches it over the AUTO (child-column) edge and therefore
        // never reports it, so `ALTER TABLE t DROP COLUMN id` succeeds — measured on
        // 18.6. Without the subtraction this would be a spurious 2BP01.
        let mut schema = Schema::new();
        let mut t = table("sr", &["id", "x"]);
        t.foreign_keys.push(fk("sr_self", &["id"], "sr", &["id"]));
        schema.create_table(t);
        let d = Dependents::resolve(
            &schema,
            DropTarget::Column {
                table: "sr",
                column: "id",
            },
        );
        assert_eq!(d.auto_foreign_keys.len(), 1);
        assert!(
            d.blocking_foreign_keys.is_empty(),
            "the AUTO edge suppresses the NORMAL one for the same constraint"
        );
    }

    #[test]
    fn a_check_is_auto_dropped_with_any_column_it_names_qualified_or_not() {
        let mut schema = Schema::new();
        let mut t = table("ck", &["a", "b"]);
        t.checks.push(check("ck_a", None, "a"));
        t.checks.push(check("ck_qual_b", Some("ck"), "b"));
        schema.create_table(t);

        for (column, expected) in [("a", 0_usize), ("b", 1)] {
            let target = DropTarget::Column {
                table: "ck",
                column,
            };
            let d = Dependents::resolve(&schema, target);
            assert_eq!(
                d.auto_checks,
                vec![expected],
                "dropping {column} must shed exactly the CHECK naming it"
            );
        }
    }

    #[test]
    fn shedding_multiple_checks_removes_the_right_ones() {
        // Removal is by index into `Table.checks`, so it must walk high-to-low.
        let mut schema = Schema::new();
        let mut t = table("multi", &["a", "b"]);
        t.checks.push(check("c0", None, "a"));
        t.checks.push(check("c1", None, "b"));
        t.checks.push(check("c2", None, "a"));
        schema.create_table(t);
        let target = DropTarget::Column {
            table: "multi",
            column: "a",
        };
        let d = Dependents::resolve(&schema, target);
        assert_eq!(d.auto_checks, vec![0, 2]);
        d.drop_from_catalog(&mut schema, target, false);
        let names: Vec<&str> = schema
            .get_table("multi")
            .unwrap()
            .checks
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["c1"]);
    }

    #[test]
    fn shedding_two_child_side_fks_over_one_column_removes_both_by_position() {
        // Two FKs whose child column is the dropped one are both AUTO-dropped. They
        // are removed by position, so the walk must go high-to-low; the third FK, on
        // another column, must survive at its shifted position.
        let mut schema = Schema::new();
        schema.create_table(table("p", &["id"]));
        let mut t = table("two", &["a", "b"]);
        t.foreign_keys.push(fk("f_a1", &["a"], "p", &["id"]));
        t.foreign_keys.push(fk("f_b", &["b"], "p", &["id"]));
        t.foreign_keys.push(fk("f_a2", &["a"], "p", &["id"]));
        schema.create_table(t);
        let target = DropTarget::Column {
            table: "two",
            column: "a",
        };
        let d = Dependents::resolve(&schema, target);
        assert_eq!(
            d.auto_foreign_keys
                .iter()
                .map(|r| r.position)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
        d.drop_from_catalog(&mut schema, target, false);
        let names: Vec<&str> = schema
            .get_table("two")
            .unwrap()
            .foreign_keys
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(names, vec!["f_b"]);
    }

    /// A constraint's **name is not its identity** here — its position is.
    ///
    /// The state is built directly against the catalog rather than through SQL. Not
    /// because SQL cannot reach it — every incomplete corner of the naming gate has
    /// produced exactly this pair, and more will — but because the resolver must be
    /// correct on its own terms rather than inheriting correctness from a gate someone
    /// could later relax or forget to extend to a new route. The two name-keyed defects
    /// it once had (a `retain` that shed the sibling, a `HashSet` that suppressed an
    /// unrelated blocking edge) both reproduce here.
    #[test]
    fn same_named_constraints_are_told_apart_by_position() {
        let mut schema = Schema::new();
        schema.create_table(table("p", &["id"]));
        let mut t = table("dup", &["a", "b"]);
        t.foreign_keys.push(fk("same", &["a"], "p", &["id"]));
        t.foreign_keys.push(fk("same", &["b"], "p", &["id"]));
        schema.create_table(t);

        let target = DropTarget::Column {
            table: "dup",
            column: "a",
        };
        let d = Dependents::resolve(&schema, target);
        assert_eq!(d.auto_foreign_keys.len(), 1, "only the FK over `a` is AUTO");
        assert_eq!(d.auto_foreign_keys[0].position, 0);
        d.drop_from_catalog(&mut schema, target, false);
        let survivors = &schema.get_table("dup").unwrap().foreign_keys;
        assert_eq!(
            survivors.len(),
            1,
            "the same-named FK over `b` must survive — a name-keyed retain took both"
        );
        assert_eq!(survivors[0].columns, vec!["b".to_string()]);
    }

    #[test]
    fn a_same_named_auto_edge_does_not_suppress_an_unrelated_blocking_one() {
        // One constraint on `sup` puts `k` on its *child* side (AUTO for dropping
        // `k`); a second, same-named, references `sup(k)` from the same table (a
        // NORMAL edge). They are different constraints, so the AUTO edge must not
        // subtract the NORMAL one — `2BP01` stands.
        let mut schema = Schema::new();
        schema.create_table(table("other", &["oid"]));
        let mut t = table("sup", &["k", "px"]);
        t.foreign_keys.push(fk("same", &["k"], "other", &["oid"]));
        t.foreign_keys.push(fk("same", &["px"], "sup", &["k"]));
        schema.create_table(t);

        let target = DropTarget::Column {
            table: "sup",
            column: "k",
        };
        let d = Dependents::resolve(&schema, target);
        assert_eq!(d.auto_foreign_keys.len(), 1);
        assert_eq!(
            d.blocking_foreign_keys.len(),
            1,
            "the self-referencing FK still blocks — it is a different constraint"
        );
        assert!(d.ensure_droppable(target, false).is_err());
    }

    /// A sequence `OWNED BY` an identity column is **not** automatically that column's
    /// INTERNAL implementation, and the difference is measurable on both engines:
    /// `ALTER SEQUENCE other OWNED BY t.<an identity column>` is accepted on 18.6 and
    /// `DROP SEQUENCE other` then **succeeds**, because the edge Postgres records for
    /// it is AUTO, not the INTERNAL one `CREATE TABLE` minted for the identity's own
    /// sequence. Reading only the owner's `identity` flag would refuse it.
    #[test]
    fn only_the_sequence_an_identity_column_draws_from_is_its_internal_one() {
        let mut schema = Schema::new();
        let mut t = table("idt", &["id"]);
        t.columns[0].identity = Some(IdentityGeneration::Always);
        t.columns[0].default = Some(ColumnDefault::NextVal("idt_id_seq".to_string()));
        schema.create_table(t);
        schema
            .sequences
            .insert("idt_id_seq".to_string(), Sequence::owned_by(1, "idt", "id"));
        schema
            .sequences
            .insert("other".to_string(), Sequence::owned_by(1, "idt", "id"));

        let own = Dependents::resolve(
            &schema,
            DropTarget::Sequence {
                sequence: "idt_id_seq",
            },
        );
        assert!(own.internal_identity_owner.is_some());
        assert!(
            own.ensure_droppable(
                DropTarget::Sequence {
                    sequence: "idt_id_seq"
                },
                true
            )
            .is_err()
        );

        let other = Dependents::resolve(&schema, DropTarget::Sequence { sequence: "other" });
        assert!(
            other.internal_identity_owner.is_none(),
            "a sequence merely pointed at an identity column is an ordinary drop"
        );
        assert!(
            other
                .ensure_droppable(DropTarget::Sequence { sequence: "other" }, false)
                .is_ok()
        );
    }

    /// An identity column contributes no `pg_attrdef` edge, so it never appears as a
    /// blocking *default* — its `column_default` is NULL in Postgres and there is
    /// nothing for a `CASCADE` to shed. The exemption is live here: `idt_id_seq` is in
    /// the removed set, `idt.id` draws from it, and the sequence target deletes no
    /// column, so without the exemption this would be one blocking default.
    ///
    /// (`ensure_droppable` refuses this drop either way — the INTERNAL edge is tested
    /// first — so what the exemption decides is which *edge kind* the catalog reports,
    /// which is the fact a `CASCADE` path or an introspecting reader consumes.)
    #[test]
    fn an_identity_columns_own_sequence_is_never_a_blocking_column_default() {
        let schema = identity_schema("idt_id_seq");
        let d = Dependents::resolve(
            &schema,
            DropTarget::Sequence {
                sequence: "idt_id_seq",
            },
        );
        assert!(
            d.internal_identity_owner.is_some(),
            "the INTERNAL edge is what stands in for the absent pg_attrdef row"
        );
        assert!(
            d.blocking_column_defaults.is_empty(),
            "an identity column has no default to shed"
        );
    }

    /// ...but what earns that exemption is being the sequence's **own** column, not
    /// carrying the `identity` flag, and the difference is the whole of D7b's F1.
    ///
    /// Postgres cannot represent this catalog: an identity column's `pg_attrdef` row
    /// does not exist, and `ALTER SEQUENCE … OWNED BY` / `ALTER COLUMN … SET DEFAULT`
    /// are both refused on one precisely so the link cannot be taken apart. pglet
    /// models the link as an ordinary `NextVal` default, so the state is one gate away
    /// rather than unrepresentable — and it was reachable, by three separate routes,
    /// while the exemption read the flag alone. Every one of them ended the same way:
    /// the sequence dropped, the default left naming it, every write `42P01`, and a
    /// dump emitting `DEFAULT nextval(…)` with no `CREATE SEQUENCE` to match.
    ///
    /// So the resolver is asserted on its own terms, as [`FkRef`] is, rather than
    /// inheriting correctness from gates someone could later relax or forget to extend
    /// to a new route. Widening the exemption back to `c.identity.is_none()` turns this
    /// red and leaves the differential corpus green — the corpus cannot see this branch
    /// at all, because the only SQL that reaches it is SQL both engines now refuse.
    #[test]
    fn an_identity_column_drawing_from_a_foreign_sequence_still_blocks() {
        // The catalog `ALTER SEQUENCE idt_id_seq OWNED BY NONE` used to produce: the
        // column still draws from `idt_id_seq`, but the sequence owns nothing, so it is
        // no longer that column's internal implementation.
        let mut schema = identity_schema("idt_id_seq");
        schema
            .sequences
            .insert("idt_id_seq".to_string(), Sequence::new(1));
        let target = DropTarget::Sequence {
            sequence: "idt_id_seq",
        };
        let d = Dependents::resolve(&schema, target);
        assert!(
            d.internal_identity_owner.is_none(),
            "a detached sequence is no column's internal implementation"
        );
        assert_eq!(
            d.blocking_column_defaults
                .iter()
                .map(|r| (r.table.as_str(), r.column.as_str()))
                .collect::<Vec<_>>(),
            vec![("idt", "id")],
            "the default is a NORMAL edge once it is not the identity's own"
        );
        assert!(d.ensure_droppable(target, false).is_err());
    }

    /// The same, reached from the other half of the fact: the *default* moved rather
    /// than the owner. `s2` is nobody's internal implementation (it owns nothing), so
    /// dropping it must report the identity column's default — which the flag-keyed
    /// exemption hid, and which no `internal_identity_owner` check can catch, since
    /// there is no INTERNAL edge to `s2` at all.
    #[test]
    fn an_identity_column_repointed_at_a_plain_sequence_still_blocks() {
        let mut schema = identity_schema("idt_id_seq");
        if let Some(t) = schema.get_table_mut("idt") {
            t.columns[0].default = Some(ColumnDefault::NextVal("s2".to_string()));
        }
        schema.sequences.insert("s2".to_string(), Sequence::new(1));
        let target = DropTarget::Sequence { sequence: "s2" };
        let d = Dependents::resolve(&schema, target);
        assert!(d.internal_identity_owner.is_none());
        assert_eq!(d.blocking_column_defaults.len(), 1);
        assert!(d.ensure_droppable(target, false).is_err());
    }

    /// The deletion set is what separates the accept side from the refusal, and it is
    /// **column**-scoped on a column drop: the same catalog answers "no dependents" to
    /// `DROP TABLE q` and "one blocking default" to `DROP COLUMN q.id`.
    #[test]
    fn the_deletion_set_scope_decides_whether_a_sibling_default_blocks() {
        let mut schema = Schema::new();
        let mut t = table("q", &["id", "b"]);
        t.columns[0].default = Some(ColumnDefault::NextVal("q_id_seq".to_string()));
        t.columns[1].default = Some(ColumnDefault::NextVal("q_id_seq".to_string()));
        schema.create_table(t);
        schema
            .sequences
            .insert("q_id_seq".to_string(), Sequence::owned_by(1, "q", "id"));

        let whole = Dependents::resolve(&schema, DropTarget::Table { table: "q" });
        assert_eq!(whole.auto_sequences, vec!["q_id_seq".to_string()]);
        assert!(
            whole.blocking_column_defaults.is_empty(),
            "every dependent column dies with the table"
        );

        let column = Dependents::resolve(
            &schema,
            DropTarget::Column {
                table: "q",
                column: "id",
            },
        );
        assert_eq!(column.auto_sequences, vec!["q_id_seq".to_string()]);
        assert_eq!(
            column
                .blocking_column_defaults
                .iter()
                .map(|r| r.column.as_str())
                .collect::<Vec<_>>(),
            vec!["b"],
            "the sibling column sits outside a column drop's deletion set"
        );
    }
}
