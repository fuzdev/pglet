//! Prepared-statement parameter **type inference**.
//!
//! [`infer_param_types`] runs at Parse time. It walks the planned statement
//! against the schema and assigns a `TypeOid` to each `$N` from its usage
//! context: explicit casts, column comparisons, `ANY`/`@>`, `IN` lists,
//! `LIMIT`/`OFFSET`, `||` / `LOWER()` arguments (→ text), an `UPDATE … SET`
//! value (incl. a `CASE`/`COALESCE` result branch, from the target column), and an
//! `ON CONFLICT` clause at every position it parameterizes (see
//! [`Inference::walk_on_conflict`], which names them). The
//! result is reported to the client via `ParameterDescription` *and* used to
//! decode the Bind wire bytes — one source of truth so the two never drift.
//!
//! Why this breadth: a `$N` left with no inferred type reports OID 0, which a
//! **binary-protocol** client (tokio-postgres, for one) trips over
//! during type introspection (a text-protocol client like node-postgres
//! tolerates it) — so each context that the spine actually parameterizes is typed.
//!
//! The *values* are not substituted into the plan: the decoded `Value`s travel in
//! an `EvalEnv` (`executor::expr`) threaded through execution, and an
//! `Expr::Parameter` resolves against it at eval time (no plan mutation).
//!
//! Inference is best-effort: a parameter with no typing context is left `None`
//! (the handler reports OID 0 / decodes as text). The fuz spine's parameters are
//! all resolvable, which is the conformance bar.
//!
//! What is **not** best-effort is the refusal: a `$N` 18.6's parse analysis leaves
//! without a type fails the statement here as it does there (`42P18` / `42P08`), and
//! that verdict is read off the positions of the parameter's occurrences rather than
//! off whether this module typed it — see [`Inference::first_unresolved`].
//
// Function-signature inference covers operators (comparison, `@>`, `ANY`, `IN`,
// `LIKE`) plus a per-function text-argument table for the `StringFunc` family
// (`string_fn_text_arg`): `UPPER`/`LENGTH`/`SUBSTRING`/`REPLACE`/
// `LTRIM`/`RTRIM`/`BTRIM`/`STRPOS`/`POSITION`/`LPAD`/`RPAD`/`SPLIT_PART`/`DATE_TRUNC`,
// each typing its text-position argument(s); `LOWER` (its own `Expr` node), `||`,
// `EXTRACT`'s field arg, and `concat_ws`'s separator are typed by their own walk
// arms. A function outside these leaves a bare `$N` arg untyped.
// TODO: an unresolved param decoded as text won't compare against a non-text
// column at execution (`values_equal` has no cross-type text arm). The spine's
// params are all resolvable, so this hasn't bitten; revisit if a genuinely
// context-free `$N` against a typed column appears.

use crate::catalog::{CatalogTable, Schema};
use crate::error::{Error, Result};
use crate::types::TypeOid;

use super::coercion::AssignmentSource;
use super::common_type::ResolvedType;
use super::dml::{
    ParameterSource, insert_returning_relations, target_list_entry_with_input,
    target_list_source_types,
};
use super::expr::{AggregateFunc, BinaryOp, Expr, Literal, StringFunc, VariadicFunc};
use super::legality::{comparison_symbol, unification_branches};
use super::plan::{
    InsertRows, LimitCount, OnConflictAction, OnConflictClause, Plan, PlannerScope,
    RelationColumns, TypeScope, on_conflict_action_relations, returning_relations,
};

// Resolution priority: an explicit client OID or cast beats inferred context.
/// The index of a bare `$N`, looking through an alias — `None` for anything with a
/// type of its own, which the ordinary rules already reach.
///
/// **A client-declared `$N` is not one of these.** A declared type is part of parse
/// analysis (`parser::parse_single_with_param_types`), which plans the parameter as an
/// `Expr::Cast` — so it fails this match, and the rules keyed on it are the rules that
/// apply to 18.6's `unknown`-typed parameter and to nothing else.
fn bare_param_index(expr: &Expr) -> Option<usize> {
    match expr {
        Expr::Parameter { index } => Some(*index),
        Expr::Alias { expr, .. } => bare_param_index(expr),
        _ => None,
    }
}

const PRIO_HINT: u8 = 4;
const PRIO_CAST: u8 = 3;
const PRIO_CONTEXT: u8 = 2;
/// A bare select-list or `RETURNING` entry's `text` — what 18.6 resolves it to once
/// nothing else in the statement has typed the parameter, so every other fact
/// outranks it.
const PRIO_UNTYPED_ENTRY: u8 = 1;

/// What each `$N` resolves to: the OID the wire decode reads, and the **enum
/// identity** no OID can carry.
///
/// One walk, two projections — the same shape [`ResolvedType`] has at the gates, and
/// for the same reason: an enum column's `TypeOid` is `Text`, so a parameter compared
/// against one is indistinguishable from a parameter compared against a `text` column
/// unless the identity rides beside the OID. 18.6 carries it as the parameter's
/// resolved type and spends it at `Bind`, running `enum_in` on the value the client
/// sent; [`bound_param_enum_legality`](crate::planner::bound_param_enum_legality) is
/// where pglet spends it, at each host's own decode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParamTargets {
    /// Each parameter's inferred `TypeOid`, indexed by `$N - 1`; `None` where
    /// inference could not pin one.
    pub types: Vec<Option<TypeOid>>,
    /// Each parameter's **enum target** by catalog key, indexed by `$N - 1` — `None`
    /// for every parameter whose position names no enum, and for every position this
    /// walk does not reach (`EDGES.md` row 114 names those).
    pub enums: Vec<Option<String>>,
}

/// Infer a `TypeOid` for each `$N` in a planned statement.
///
/// Returns a vector indexed by `param_index - 1`; `None` marks a parameter whose
/// type couldn't be determined. `hint` carries any client-supplied OIDs from the
/// Parse message (usually empty for tokio-postgres, which relies on server
/// inference); a present hint is authoritative. The same OIDs reach the plan itself only
/// if it was built by `parser::parse_single_with_param_types`, which is what the native
/// server's `Parse` does; this function reads a plan and cannot put them there.
///
/// The OID projection of [`infer_param_targets`], which is the one walk — a host that
/// also needs the enum identity calls that instead, never a second walk of its own.
///
/// # Errors
///
/// [`infer_param_targets`]' refusal.
pub fn infer_param_types(
    plan: &Plan,
    schema: &Schema,
    hint: &[Option<TypeOid>],
) -> Result<Vec<Option<TypeOid>>> {
    infer_param_targets(plan, schema, hint).map(|targets| targets.types)
}

/// Infer each `$N`'s [`ParamTargets`] — its `TypeOid` and its enum target.
///
/// # Errors
///
/// [`Error::IndeterminateParameter`] for a `$N` parse analysis leaves without a type
/// ([`Inference::first_unresolved`]) — `42P08` ahead of `42P18`, as 18.6's
/// `check_variable_parameters` runs ahead of its sweep of the parameter array. Not
/// raised over a statement whose `FROM` clause or DML target names a relation the
/// catalog lacks: that `42P01` is the executor's to raise ([`Inference::relation`]).
pub fn infer_param_targets(
    plan: &Plan,
    schema: &Schema,
    hint: &[Option<TypeOid>],
) -> Result<ParamTargets> {
    let mut inf = Inference {
        types: Vec::new(),
        enums: Vec::new(),
        prio: Vec::new(),
        from_scope: Vec::new(),
        clause_scope: None,
        position: Vec::new(),
        seq: 0,
        expr_depth: 0,
        root_fixes: true,
        first_fix: Vec::new(),
        first_unresolved: Vec::new(),
        lifted: false,
        missing_relation: false,
        expr_subqueries: 0,
        pending: Vec::new(),
        pending_marks: Vec::new(),
    };
    // Seed client-supplied OIDs (highest priority).
    for (i, h) in hint.iter().enumerate() {
        if let Some(t) = h {
            inf.record(i + 1, *t, PRIO_HINT);
        } else {
            inf.ensure(i + 1);
        }
    }
    inf.walk_scoped(plan, schema, plan_table(plan));
    // 18.6 raises this refusal after the whole of parse analysis, so a relation the
    // catalog lacks is its `42P01` first. pglet raises that one at execution, which the
    // refusal would pre-empt — so it stands down where that `42P01` is certain.
    if !inf.missing_relation {
        inf.refuse_unresolved()?;
    }
    // A `$N::<enum>` is the parameter's enum target only where it is the occurrence
    // that fixes the parameter's type — see [`Inference::first_fix`].
    for (slot, fix) in inf.enums.iter_mut().zip(&inf.first_fix) {
        if let Some((_, Some(name))) = fix {
            *slot = Some(name.clone());
        }
    }
    Ok(ParamTargets {
        types: inf.types,
        enums: inf.enums,
    })
}

/// Accumulates inferred parameter types, keeping the highest-priority fact per
/// slot and growing to the largest `$N` seen.
struct Inference {
    types: Vec<Option<TypeOid>>,
    /// Each `$N`'s enum target by catalog key — see [`ParamTargets::enums`]. Parallel
    /// to `types` and grown by the same [`Inference::ensure`], because a parameter can
    /// have an enum target at a position that pins no OID of its own.
    enums: Vec<Option<String>>,
    prio: Vec<u8>,
    /// The current query level's FROM relations as `(effective name, real table)` —
    /// the alias→table map for resolving a qualified column whose qualifier is a
    /// FROM alias the single `table` context can't (the multi-table-JOIN case, e.g.
    /// `act.id` over `FROM actor act JOIN account a`). Reset per query scope by
    /// [`Self::walk_scoped`] at each subquery boundary.
    from_scope: Vec<(String, String)>,
    /// The **closed namespace of the clause currently being walked**, else `None` — the
    /// relations that clause can name, ahead of the catalog lookup
    /// [`Self::infer_col_type`] otherwise falls to. The clauses that set it are named
    /// here rather than counted, and each does so because its namespace is *not* the
    /// catalog:
    ///
    /// - `RETURNING`, via [`Self::walk_returning`] over `returning_relations`.
    /// - the `ON CONFLICT DO UPDATE` **action**, via [`Self::walk_on_conflict`] over
    ///   `on_conflict_action_relations`. Not the arbiter — see there.
    ///
    /// Each was on `TypeScope::Catalog(Some(<target>))` and each was wrong for the same
    /// reason: that is not a namespace but a catalog lookup keyed by the qualifier, so a
    /// qualifier is resolved **in the whole database** and only an unmatched one falls
    /// back to the threaded table. Both misreadings are measured against 18.6.
    /// `RETURNING`: with `dt(id, b BIGINT)` the target and `ds(id, b INT)` the source
    /// aliased `s`, `RETURNING (s.b = $1)` reported `$1` as `int8` where 18.6 says
    /// `int4`, and over a source column the target lacks (`RETURNING (s.k = $1)`) it
    /// reported OID **0**, which the module doc above explains a binary-protocol client
    /// trips over. The action: a **real table named `excluded`** shadowed the
    /// pseudo-relation, so over `oc_probe(id INT PRIMARY KEY, v TEXT, n BIGINT)` beside
    /// `excluded(id INT, n TEXT, v TEXT)`, `… DO UPDATE SET v = 'z' WHERE excluded.n =
    /// $1` reported `{text}` where 18.6 reports `{bigint}` — and end-to-end the bound
    /// statement was a loud `42883 operator does not exist: bigint = text` where 18.6
    /// updates the row. In both cases the controls agreed on both engines throughout —
    /// for `RETURNING` the unaliased source (whose qualifier *is* a real table) and the
    /// target-qualified spelling; for the action the same statement with no `excluded`
    /// table in the catalog.
    ///
    /// Cleared at every subquery boundary by [`Self::walk_scoped`], so the inner level's
    /// own relations decide — see there for what that clearing does *not* claim.
    clause_scope: Option<RelationColumns>,
    /// Where the walk stands in 18.6's **parse-analysis order**: one `(clause rank,
    /// sequence)` pair per query level, outermost first — the rank of the clause being
    /// walked at that level ([`clause_rank`]) and, for an enclosing level, the sequence
    /// at which the walk descended into the subquery. Compared lexicographically, two
    /// positions order as parse analysis reaches them, whatever order this walk visits
    /// the plan's nodes in (a `Sort` is walked ahead of the select list it sorts, an
    /// `UPDATE`'s `SET` list ahead of its `WHERE`).
    position: AnalysisPosition,
    /// The sequence the next [`Self::fix_event`] or subquery descent takes.
    seq: u32,
    /// How deep inside a clause-root expression [`Self::walk_expr`] is — `0` at a root.
    expr_depth: u32,
    /// Whether a bare `$N` that **is** a clause root has its type fixed there — false for
    /// a select list and a `RETURNING` list, whose bare entries 18.6 resolves only after
    /// the whole list (`resolveTargetListUnknowns`).
    root_fixes: bool,
    /// Each `$N`'s **first type-fixing occurrence** in parse-analysis order, and whether
    /// it was a cast to an enum (`Some(name)`) or anything else (`None`).
    ///
    /// 18.6 fixes an unspecified parameter's type at the first occurrence whose transform
    /// coerces it (`variable_coerce_param_hook`); every later occurrence converts a value
    /// of *that* type. So `$1::mood` makes `$1` the enum — its value run through `enum_in`
    /// at `Bind` — only when it is that first occurrence: in `WHERE ($1 = '' OR m =
    /// $1::mood)` the comparison has already fixed `$1` as `text`, and the cast is a
    /// value conversion that raises only where it is evaluated. Measured both ways: over
    /// zero rows `SELECT $1::text, $1::mood` accepts a non-member and `SELECT $1::mood,
    /// $1::text` refuses it at `Bind`.
    ///
    /// An occurrence is timed when its **parent** node is transformed, since that is where
    /// 18.6 coerces it: `$1 = $1::mood` is the cast's (the comparison resolves after both
    /// operands), a bare `$N` under `IS [NOT] NULL` fixes nothing, and an alias leaves
    /// the timing to its own position.
    first_fix: Vec<Option<(AnalysisPosition, Option<String>)>>,
    /// Each `$N`'s **first occurrence that fixes nothing**, in parse-analysis order —
    /// a bare `$N` in a position whose transform leaves an `unknown` parameter as it
    /// found it ([`Timing::Unresolved`]), timed where 18.6 builds the `Param` node.
    ///
    /// 18.6 gives a `Param` the type the parameter has *when the reference is
    /// transformed*, so such an occurrence is harmless once an earlier one fixed the
    /// type and fatal otherwise: nothing later revisits the node, and
    /// `check_variable_parameters` refuses the statement over it. So the verdict is one
    /// comparison per parameter ([`Self::refuse_unresolved`]) — this position against
    /// [`Self::first_fix`] — and `(name = $1 OR $1 IS NULL)` is accepted where
    /// `($1 IS NULL OR name = $1)` is `42P08`, both measured.
    ///
    /// It is deliberately **not** "inference left the slot `None`": this module types
    /// fewer positions than 18.6 does (no type crosses an arithmetic operator), and a
    /// refusal keyed on that would refuse statements 18.6 answers.
    first_unresolved: Vec<Option<AnalysisPosition>>,
    /// Whether the walk is inside a conjunct the planner **lifted** out of a `WHERE`
    /// into a B-tree index-scan node — a comparison of the key column with a `$N` or a
    /// cast of one, which is all such a probe can be. Its place among the clause's other
    /// conjuncts is gone, so a fix inside it is timed at the clause's start: the
    /// statement is then accepted wherever some source order would be, never refused
    /// where 18.6 answers (`EDGES.md` row 276).
    lifted: bool,
    /// Whether a scan or a DML target the walk reached names a relation the catalog
    /// lacks ([`Self::relation`]).
    missing_relation: bool,
    /// How many **expression** subqueries the walk is inside — a scalar subquery, an
    /// `EXISTS`, an `IN (SELECT …)` — `0` at a level the statement always runs. The
    /// executor reaches such a query only for a row that evaluates it, so a relation it
    /// names, at any depth beneath it, is not one whose `42P01` is certain
    /// ([`Self::relation`]).
    expr_subqueries: u32,
    /// The operand occurrences queued by the nodes [`Self::walk_expr`] is inside, each
    /// node's run starting at its entry in [`Self::pending_marks`] — see
    /// [`Self::enter_expr`].
    pending: Vec<(usize, Timing)>,
    pending_marks: Vec<usize>,
}

/// When 18.6's parse analysis fixes the type of a bare `$N` that is a node's direct
/// operand — what [`Inference::queue_operands`] sorts each operand into.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Timing {
    /// When the **node** is transformed, after every operand was: an operator's or a
    /// function's resolution, a common-type construct's fold. `Some(name)` where the
    /// node is a cast to that enum.
    Node(Option<String>),
    /// When the **operand** is transformed, ahead of its later siblings: a boolean
    /// context (`coerce_to_boolean` runs per argument) and a simple `CASE`'s operand,
    /// which `transformCaseExpr` resolves to `text` before it reads a `WHEN`.
    Operand,
    /// Never — the node takes an `unknown` operand as it is: `IS [NOT] NULL` (and the
    /// `IS [NOT] DISTINCT FROM NULL` 18.6 rewrites to it), and an argument declared
    /// `"any"` — `count`'s, `concat`'s, `concat_ws`'s past the separator,
    /// `jsonb_build_object`'s.
    Unresolved,
}

/// A place in 18.6's parse-analysis order — see [`Inference::position`].
type AnalysisPosition = Vec<(u8, u32)>;

/// 18.6's parse-analysis order of a query level's clauses, as one ranking every
/// statement kind's order is a sub-sequence of: `transformSelectStmt` (`WITH`, `FROM`,
/// the select list, `WHERE`, `HAVING`, `ORDER BY`, `GROUP BY`, `DISTINCT`, `LIMIT`),
/// `transformInsertStmt` (the row source, `ON CONFLICT`, `RETURNING`),
/// `transformUpdateStmt` (`FROM`, `WHERE`, `RETURNING`, then the `SET` list) and
/// `transformDeleteStmt` (`WHERE`, `RETURNING`).
mod clause_rank {
    pub const WITH: u8 = 0;
    /// A `FROM` item, an `INSERT`'s row source, a set operation's branches.
    pub const FROM: u8 = 1;
    pub const ON_CONFLICT: u8 = 2;
    pub const TARGET: u8 = 3;
    pub const WHERE: u8 = 4;
    pub const HAVING: u8 = 5;
    pub const ORDER_BY: u8 = 6;
    pub const GROUP_BY: u8 = 7;
    pub const DISTINCT: u8 = 8;
    pub const LIMIT: u8 = 9;
    pub const RETURNING: u8 = 10;
    pub const SET: u8 = 11;
}

impl Inference {
    /// Grow the slots so `$index` is representable (no type recorded).
    fn ensure(&mut self, index: usize) {
        if index > self.types.len() {
            self.types.resize(index, None);
            self.enums.resize(index, None);
            self.prio.resize(index, 0);
            self.first_fix.resize(index, None);
            self.first_unresolved.resize(index, None);
        }
    }

    /// Note a relation the catalog lacks, where the executor's `42P01` for it is
    /// certain: a scan in a `FROM` clause or a DML statement's target, outside any
    /// expression's subquery. The positions measured to raise it whatever the data,
    /// over empty tables: under a `WHERE false`, under a `LIMIT 0`, either side of a
    /// join, a derived table, a `WITH` query the plan holds, an `UPDATE`'s `FROM`, an
    /// `INSERT`'s source, a DML target. A relation named inside an expression's
    /// subquery is measured the other way — raised for a row that evaluates the
    /// expression, and not over an empty table — so letting it stand the refusal down
    /// would turn a refused statement into an answered one, and it does not.
    ///
    /// "Lacks" is by relation kind, each measured: a name no relation carries and an
    /// enum type's name are `42P01` on both engines, and an index's name is `42809` on
    /// both, so those stand the refusal down. A **sequence's** name does not — 18.6
    /// reads a sequence as a relation and goes on to refuse the parameter, where the
    /// executor here cannot open one (`42809`).
    ///
    /// The index-scan nodes need no call: index selection finds an index through the
    /// table's catalog entry (`single_column_index_for`, `gin_index_for`).
    fn relation(&mut self, table: &str, schema: &Schema) {
        if self.expr_subqueries == 0 {
            self.missing_relation |=
                schema.get_table(table).is_none() && !schema.sequences.contains_key(table);
        }
    }

    /// Enter clause `rank` of the current query level (see [`Self::position`]).
    fn clause(&mut self, rank: u8) {
        if let Some(level) = self.position.last_mut() {
            level.0 = rank;
        }
        self.root_fixes = !matches!(rank, clause_rank::TARGET | clause_rank::RETURNING);
    }

    /// The position the next event takes: [`Self::position`] with the current level's
    /// sequence advanced — or, `at_clause_start`, left at the clause's start, where a
    /// fix inside a lifted conjunct is timed ([`Self::lifted`]).
    fn next_position(&mut self, at_clause_start: bool) -> AnalysisPosition {
        let mut key = self.position.clone();
        if let Some(level) = key.last_mut() {
            level.1 = if at_clause_start {
                0
            } else {
                self.seq += 1;
                self.seq
            };
        }
        key
    }

    /// Record an occurrence of `$index` that fixes its type here — as the enum `target`
    /// when it is a cast to one — keeping the earliest in parse-analysis order
    /// ([`Self::first_fix`]).
    #[inline(never)]
    fn fix_event(&mut self, index: usize, target: Option<&str>) {
        let key = self.next_position(self.lifted);
        self.fix_event_at(index, target, key);
    }

    fn fix_event_at(&mut self, index: usize, target: Option<&str>, key: AnalysisPosition) {
        if index == 0 {
            return;
        }
        self.ensure(index);
        let slot = &mut self.first_fix[index - 1];
        if slot.as_ref().is_none_or(|(first, _)| key < *first) {
            *slot = Some((key, target.map(str::to_string)));
        }
    }

    /// Record a bare `$index` that **is** an entry of a select list or a `RETURNING`
    /// list, whose type 18.6 fixes as `text` only once the list's own clause — for a
    /// select list, the whole query level — has been analysed
    /// (`resolveTargetListUnknowns`): after every other clause of the level, and ahead
    /// of whatever the enclosing level analyses next.
    fn defer_event(&mut self, index: usize) {
        let mut key = self.position.clone();
        if let Some(level) = key.last_mut() {
            *level = match level.0 {
                clause_rank::RETURNING => (clause_rank::RETURNING, u32::MAX),
                _ => (u8::MAX, 0),
            };
        }
        self.fix_event_at(index, None, key);
        self.record(index, TypeOid::Text, PRIO_UNTYPED_ENTRY);
    }

    /// Record an occurrence of `$index` that fixes nothing, keeping the earliest in
    /// parse-analysis order ([`Self::first_unresolved`]).
    fn unresolved_event(&mut self, index: usize) {
        if index == 0 {
            return;
        }
        self.ensure(index);
        let key = self.next_position(false);
        let slot = &mut self.first_unresolved[index - 1];
        if slot.as_ref().is_none_or(|first| key < *first) {
            *slot = Some(key);
        }
    }

    /// The refusal 18.6 raises for a parameter left without a type: one whose first
    /// occurrence that fixes nothing ([`Self::first_unresolved`]) is reached before
    /// any occurrence that fixes its type ([`Self::first_fix`]).
    ///
    /// `42P08` where a later occurrence does fix it — the parameter has a type and one
    /// `Param` node disagrees with it — and that check runs inside parse analysis, so
    /// it outranks the `42P18` raised over the parameter array afterwards. A parameter
    /// whose type the client declared is skipped: it is a typed `Param` from its first
    /// reference.
    fn refuse_unresolved(&self) -> Result<()> {
        let mut never_fixed = None;
        let mut fixed_later: Option<(&AnalysisPosition, usize)> = None;
        for (k, unresolved) in self.first_unresolved.iter().enumerate() {
            let Some(unresolved) = unresolved else {
                continue;
            };
            if self.prio[k] == PRIO_HINT {
                continue;
            }
            match &self.first_fix[k] {
                // 18.6's sweep names the lowest-numbered slot left without a type, a
                // number the statement never writes included.
                None => {
                    never_fixed = never_fixed.or_else(|| {
                        (0..=k)
                            .find(|&j| self.first_fix[j].is_none() && self.prio[j] != PRIO_HINT)
                            .map(|j| j + 1)
                    });
                }
                Some((fix, _)) if unresolved < fix => {
                    if fixed_later.is_none_or(|(first, _)| unresolved < first) {
                        fixed_later = Some((unresolved, k + 1));
                    }
                }
                Some(_) => {}
            }
        }
        match (fixed_later, never_fixed) {
            (Some((_, index)), _) => Err(Error::IndeterminateParameter {
                index,
                fixed_later: true,
            }),
            (None, Some(index)) => Err(Error::IndeterminateParameter {
                index,
                fixed_later: false,
            }),
            (None, None) => Ok(()),
        }
    }

    /// Queue the bare `$N`s `expr` holds as its **direct** operands, each with the
    /// [`Timing`] 18.6's transform of this node gives it, and record the ones in a
    /// boolean context as `boolean` — one pass over the node's children for both. A
    /// [`Timing::Node`] operand is timed when the node is left ([`Self::leave_expr`]) —
    /// after the operands were walked, because that is where 18.6's transform of the
    /// parent coerces them — and a [`Timing::Operand`] or [`Timing::Unresolved`] one when
    /// the walk reaches the operand itself ([`Self::operand_event`]).
    fn queue_operands(&mut self, expr: &Expr) {
        // The nodes whose every operand is left `unknown`, and `concat_ws`, whose
        // separator is the one `text` argument ahead of its `"any"` list.
        let unresolved_from = match expr {
            Expr::Alias { .. } => return,
            Expr::StringFn {
                func: StringFunc::EnumIn,
                args,
            } => {
                if let Some((operand, name, _)) = crate::planner::enum_in_arg_parts(args)
                    && let Some(index) = bare_param_index(operand)
                {
                    self.pending
                        .push((index, Timing::Node(Some(name.to_string()))));
                }
                return;
            }
            Expr::IsNull(_)
            | Expr::IsNotNull(_)
            | Expr::JsonbBuildObject(_)
            | Expr::Aggregate {
                func: AggregateFunc::Count,
                distinct: false,
                ..
            }
            | Expr::VariadicFn {
                func: VariadicFunc::Concat,
                ..
            } => Some(0),
            // `x IS [NOT] DISTINCT FROM NULL` is a null test to 18.6's
            // `transformAExprDistinct`, whichever side the `NULL` is written on.
            Expr::BinaryOp {
                left,
                op: BinaryOp::IsDistinctFrom | BinaryOp::IsNotDistinctFrom,
                right,
            } if is_null_literal(left) || is_null_literal(right) => Some(0),
            Expr::VariadicFn {
                func: VariadicFunc::ConcatWs,
                ..
            } => Some(1),
            _ => None,
        };
        let simple_case = matches!(
            expr,
            Expr::Case {
                operand: Some(_),
                ..
            }
        );
        let pending = &mut self.pending;
        let mut bool_params: Vec<usize> = Vec::new();
        let mut ordinal = 0;
        expr.for_each_child_ctx(|child, ctx| {
            let unresolved = unresolved_from.is_some_and(|from| ordinal >= from);
            let at_operand = ctx.is_some() || (simple_case && ordinal == 0);
            ordinal += 1;
            if ctx.is_some()
                && let Some(index) = bare_param_index(child)
            {
                bool_params.push(index);
            }
            if unresolved {
                // Only a parameter written here itself: 18.6 reads the node's own
                // argument, and an alias never stands in this position.
                if let Expr::Parameter { index } = child {
                    pending.push((*index, Timing::Unresolved));
                }
            } else if let Some(index) = bare_param_index(child) {
                let timing = if at_operand {
                    Timing::Operand
                } else {
                    Timing::Node(None)
                };
                pending.push((index, timing));
            }
        });
        // A bare `$N` sitting **directly** in one of this node's boolean contexts is
        // `boolean`, as 18.6's `coerce_to_boolean` resolves an unknown Param there.
        // Read off `Expr::for_each_child_ctx` rather than a second arm list, so this and
        // the plan-time gate share one enumeration of node positions rather than
        // carrying two that must agree. What that enumeration leaves out it leaves out
        // for both: an `IS [NOT] TRUE` operand is marked `None` there and gated by its
        // own post-order check, so the `$N` in `WHERE $1 IS TRUE` stays OID 0 where 18.6
        // reports `boolean` (`docs/param_binding.md` § Deferred / known gaps).
        // `PRIO_CONTEXT`, so an explicit `$1::int` still wins — and then fails the
        // gate, which is 18.6's answer too.
        for index in bool_params {
            self.record(index, TypeOid::Bool, PRIO_CONTEXT);
        }
    }

    /// Time a bare `$index` operand the walk has just reached, against what its parent
    /// queued for it ([`Self::queue_operands`]): the operands 18.6 settles as it
    /// transforms them rather than when the node is done.
    fn operand_event(&mut self, index: usize) {
        let mark = self.pending_marks.last().copied().unwrap_or(0);
        let mut fixes = false;
        let mut unresolved = false;
        for (queued, timing) in &self.pending[mark..] {
            if *queued == index {
                fixes |= *timing == Timing::Operand;
                unresolved |= *timing == Timing::Unresolved;
            }
        }
        if unresolved {
            self.unresolved_event(index);
        }
        if fixes {
            self.fix_event(index, None);
        }
    }

    /// Record `$index : t` at priority `p`, keeping the strongest fact.
    fn record(&mut self, index: usize, t: TypeOid, p: u8) {
        if index == 0 {
            return;
        }
        self.ensure(index);
        let k = index - 1;
        if p > self.prio[k] {
            self.types[k] = Some(t);
            self.prio[k] = p;
        }
    }

    /// Record `$index`'s **enum target**, first fact winning — the same equal-priority
    /// rule [`Self::record`] keeps, and there is no priority ladder here because a
    /// client-declared parameter is never a [`bare_param_index`] in the first place.
    fn record_enum(&mut self, index: usize, name: &str) {
        if index == 0 {
            return;
        }
        self.ensure(index);
        let slot = &mut self.enums[index - 1];
        if slot.is_none() {
            *slot = Some(name.to_string());
        }
    }

    /// Record the **enum target** of every bare `$N` this node puts at an enum
    /// position.
    ///
    /// 18.6 resolves an unspecified parameter's type during parse analysis and then
    /// runs that type's input function at `Bind` — for an enum target `enum_in`, which
    /// is the whole of the `22P02`. pglet cannot run the conversion at plan time
    /// (the value has not arrived), so what plan time records is the **target**, and
    /// each host runs the membership test on the decoded value where 18.6 runs it
    /// ([`bound_param_enum_legality`](super::legality::bound_param_enum_legality)).
    /// Deciding it at eval time instead would be a different rule: 18.6 refuses a
    /// non-member `$1` over an **empty** table and inside a `WHERE false`, both
    /// measured, because nothing about the refusal depends on a row.
    ///
    /// The positions are read off `planner::coercion`'s `check_expr_node_semantics`
    /// arm list rather than invented — the ones it makes an `=`/`<`/… at (a
    /// comparison, a simple-`CASE` label, an `IN` item, `nullif`'s pair, an
    /// `op ANY`/`op ALL`'s array element) and then the branch fold over
    /// [`unification_branches`], the one enumeration of the five `select_common_type`
    /// constructs, in that same order.
    ///
    /// **One arm of that list has no counterpart here, by construction.** A target this
    /// walk can record is one a sibling `Expr` names, and at the `IN (SELECT …)`
    /// position the enum is a **sub-plan's output column** — which `expr_enum_with`
    /// answers `None` for by design (a subquery is one of its two `None`s) and which
    /// the gate reads through a second, `ColumnScope`-bound reader this walk does not
    /// hold. So `$1 IN (SELECT m FROM w)` records no target and a non-member binds
    /// where 18.6 refuses it at `Bind`; the set-operation door has the same shape and
    /// the same gap, and there it **writes**. `EDGES.md` row 114 carries both.
    fn record_enum_targets(&mut self, expr: &Expr, schema: &Schema, table: Option<&str>) {
        match expr {
            Expr::BinaryOp { left, op, right } if comparison_symbol(*op).is_some() => {
                self.pair_enum_target(left, right, schema, table);
            }
            Expr::Case {
                operand: Some(operand),
                when_clauses,
                ..
            } => {
                for (condition, _) in when_clauses {
                    self.pair_enum_target(operand, condition, schema, table);
                }
            }
            Expr::InList {
                expr: subject,
                list,
                ..
            } => {
                for item in list {
                    self.pair_enum_target(subject, item, schema, table);
                }
            }
            Expr::VariadicFn {
                func: VariadicFunc::NullIf,
                args,
            } => {
                if let [a, b] = args.as_slice() {
                    self.pair_enum_target(a, b, schema, table);
                }
            }
            // `x op ANY/ALL (array)`: the left-hand side takes the **element**'s enum,
            // and [`Self::expr_enum_name`] already answers an array operand at its
            // element's width, so the array expression is handed to the pair whole. The
            // other direction cannot fire and must not: a bare `$N` is never the array,
            // and one *inside* the array is an element of an all-`unknown` set 18.6
            // resolves to `text` — `m = ANY(ARRAY[$1])` is its `42883`, not a `22P02`,
            // which is [`Self::fold_enum_target`]'s rule and reached through the
            // `ARRAY[…]` branch fold below.
            Expr::AnyAll {
                left, op, right, ..
            } if comparison_symbol(*op).is_some() => {
                self.pair_enum_target(left, right, schema, table);
            }
            // `$1::<enum>` — the cast's own target — is not recorded here: it is the
            // parameter's target only when it is the occurrence that fixes the
            // parameter's type, which is an order question this node cannot answer
            // alone ([`Self::first_fix`]). Where it is, 18.6 runs `enum_in` at `Bind`,
            // so a non-member is refused whatever the rows (measured: over an empty
            // table, `22P02` with `CONTEXT: unnamed portal parameter $1`). A
            // client-declared `$N` is a `Cast` here, not a bare parameter, and its
            // conversion runs on the value, as 18.6's does for a declared `text`
            // parameter.
            _ => {}
        }
        if let Some((branches, _)) = unification_branches(expr) {
            self.fold_enum_target(&branches, schema, table);
        }
    }

    /// A comparison pair: a bare `$N` on either side takes the **other** side's enum,
    /// which is what 18.6's operator resolution coerces the `unknown` side to.
    fn pair_enum_target(
        &mut self,
        left: &Expr,
        right: &Expr,
        schema: &Schema,
        table: Option<&str>,
    ) {
        if let Some(index) = bare_param_index(left)
            && let Some(name) = self.expr_enum_name(right, schema, table)
        {
            self.record_enum(index, &name);
        }
        if let Some(index) = bare_param_index(right)
            && let Some(name) = self.expr_enum_name(left, schema, table)
        {
            self.record_enum(index, &name);
        }
    }

    /// A branch set: every bare `$N` branch takes the enum the **typed** branches
    /// agree on, which is what `select_common_type` lands on before `coerce_type`
    /// converts each `unknown` branch to it.
    ///
    /// Agreement is required rather than assumed, and the two ways it can fail are
    /// each 18.6's own answer at this node, not this rule's: no typed branch names an
    /// enum (`ARRAY[$1]`, whose all-`unknown` element set 18.6 resolves to `text` —
    /// `m > ANY(ARRAY[$1])` is its `42883`, not a `22P02`), and two typed branches
    /// naming **different** enums, which the fold itself refuses `42846` one gate
    /// earlier. `ARRAY[m, $1]` has one and is `22P02` on both engines.
    fn fold_enum_target(&mut self, branches: &[&Expr], schema: &Schema, table: Option<&str>) {
        let mut target: Option<String> = None;
        for branch in branches {
            if bare_param_index(branch).is_some() {
                continue;
            }
            if let Some(name) = self.expr_enum_name(branch, schema, table) {
                match &target {
                    Some(seen) if *seen != name => return,
                    _ => target = Some(name),
                }
            }
        }
        let Some(name) = target else {
            return;
        };
        for branch in branches {
            if let Some(index) = bare_param_index(branch) {
                self.record_enum(index, &name);
            }
        }
    }

    /// The enum `expr` carries in this walk's scope — [`Self::infer_col_resolved`]'s
    /// identity half, `None` where the operand names no enum or carries one this
    /// engine cannot decide ([`EnumIdentity::Undecided`](super::EnumIdentity), which
    /// a written `x::mood` produces and `EDGES.md` row 104 owns).
    fn expr_enum_name(&self, expr: &Expr, schema: &Schema, table: Option<&str>) -> Option<String> {
        self.infer_col_resolved(expr, schema, table)?
            .enum_type
            .name()
            .map(str::to_string)
    }

    /// Type each bare `$N` entry of an `INSERT … SELECT` source's select list from the
    /// column it is assigned to.
    ///
    /// 18.6's `transformInsertStmt` analyses the source with its `unknown` entries left
    /// unresolved and hands an `unknown` `Param` entry itself to `transformInsertRow`,
    /// which coerces it to the target column — so `INSERT INTO kv (k, v, n) SELECT $1,
    /// $2, $3` describes `{text,jsonb,integer}`, measured. An entry a sort, grouping,
    /// `DISTINCT` or window key names was resolved to `text` inside the source first
    /// and is not one of these; `target_list_source_types` under
    /// `ParameterSource::Unknown` is that rule, the one a set operation's side reads.
    ///
    /// Called after the source is walked, so the target outranks only the entry's own
    /// `text` ([`PRIO_UNTYPED_ENTRY`]): a type an occurrence inside the source recorded
    /// stays, as it is the earlier one in 18.6's order too. The entry's fix is already
    /// timed at the source level's end ([`Self::defer_event`]), which is where
    /// `transformInsertRow` runs — ahead of `ON CONFLICT` and `RETURNING`.
    fn record_insert_select_targets(
        &mut self,
        source: &Plan,
        columns: &[usize],
        table: &str,
        schema: &Schema,
    ) {
        let Some(table_def) = schema.get_table(table) else {
            return;
        };
        let mut sources = None;
        for (i, column) in columns.iter().enumerate() {
            let Some(index) =
                target_list_entry_with_input(source, i).and_then(|(e, _)| bare_param_index(e))
            else {
                continue;
            };
            let sources = sources.get_or_insert_with(|| {
                target_list_source_types(source, schema, ParameterSource::Unknown)
            });
            if sources.get(i) == Some(&AssignmentSource::Unknown)
                && let Some(column) = table_def.columns.get(*column)
            {
                self.record(index, column.type_oid, PRIO_CONTEXT);
            }
        }
    }

    /// Type a bare `$N` probe value from its indexed `key_column` — the
    /// column-comparison rule re-applied for the index-scan nodes the planner lifts a
    /// `col OP $N` conjunct *into* (`IndexScan` / `IndexRangeScan` / `GinScan`), since
    /// inference runs after that lift.
    #[cfg(feature = "index_acceleration")]
    fn record_key_column_param(
        &mut self,
        index: usize,
        key_column: &str,
        table: Option<&str>,
        schema: &Schema,
    ) {
        let key_col = Expr::Column {
            table: None,
            name: key_column.to_string(),
        };
        if let Some(ty) = key_col.infer_type(&PlannerScope::catalog(schema, table)) {
            self.record(index, ty, PRIO_CONTEXT);
        }
    }

    /// Walk a plan node, recording parameter types from its expressions and
    /// recursing into child plans. `table` is the table whose columns unqualified
    /// references resolve against (for column-context typing).
    fn walk_plan(&mut self, plan: &Plan, schema: &Schema, table: Option<&str>) {
        match plan {
            Plan::LockRows { input } => self.walk_plan(input, schema, table),
            // A bare `$N` entry is fixed as `text` once the whole list is analysed —
            // a `VALUES` list's columns, and a no-FROM select list, which lowers to one
            // row here — so its fix is timed after the last row rather than at the entry.
            Plan::Values { rows, .. } => {
                self.clause(clause_rank::FROM);
                self.root_fixes = false;
                for e in rows.iter().flatten() {
                    self.walk_expr(e, schema, table);
                }
                self.root_fixes = true;
                for e in rows.iter().flatten() {
                    if let Some(index) = bare_param_index(e) {
                        self.fix_event(index, None);
                    }
                }
            }
            Plan::Filter { input, predicate } => {
                self.clause(clause_rank::WHERE);
                self.record_bool_clause(predicate);
                self.walk_expr(predicate, schema, plan_table(input));
                self.walk_plan(input, schema, table);
            }
            Plan::Project { input, exprs, .. } => {
                let inner = plan_table(input);
                self.clause(clause_rank::TARGET);
                for e in exprs {
                    self.walk_expr(e, schema, inner);
                }
                self.walk_plan(input, schema, table);
            }
            // No-FROM projection-unnest: walk each projection expr (no input table
            // context). `unnest($2::text[])` types `$2` via its inner cast.
            Plan::ProjectUnnest { exprs } => {
                self.clause(clause_rank::TARGET);
                for e in exprs {
                    self.walk_expr(e, schema, None);
                }
            }
            // Every field named, `overriding` included: this arm's `..` is what swallowed
            // `on_conflict` and left a bound upsert unbindable, so a field added to
            // `Plan::Insert` from here on makes the compiler ask.
            Plan::Insert {
                table: t,
                rows,
                on_conflict,
                returning,
                overriding: _,
            } => {
                self.relation(t, schema);
                self.clause(clause_rank::FROM);
                match rows {
                    // In the order each row is written, which is the order 18.6
                    // analyses its cells in — not the table's, which the rows are
                    // stored in, unless `written` is empty and the two are one.
                    InsertRows::Values { rows, written } => {
                        for row in rows {
                            for e in written.iter().filter_map(|&i| row.get(i)) {
                                self.walk_expr(e, schema, Some(t));
                            }
                            if written.is_empty() {
                                self.walk_exprs(row, schema, Some(t));
                            }
                        }
                    }
                    // `INSERT … SELECT`: type params inside the source query (e.g.
                    // `unnest($2::text[])`) from the source's own scan/projection, then
                    // its bare entries from the columns they are assigned to.
                    InsertRows::Select { source, columns } => {
                        self.walk_scoped(source, schema, plan_table(source));
                        self.record_insert_select_targets(source, columns, t, schema);
                    }
                }
                // The clause sits **between** the row source and `RETURNING`, which is
                // `transformInsertStmt`'s own order and is observable: `record` keeps the
                // first fact at equal priority, so a `$N` written in two clauses takes the
                // earlier one's type. Measured on 18.6 with `oc_probe(id INT PRIMARY KEY,
                // v TEXT, n BIGINT)` — `VALUES (1,$1) … WHERE n > $1` is
                // `42883 bigint > text` (the row source typed it first) and
                // `… DO UPDATE SET v = 'z' WHERE oc_probe.n = $1 RETURNING (v = $1)` is
                // `42883 text = bigint` pointing at the `RETURNING` (the action typed it
                // first), each the collision reported from the *later* clause.
                //
                // **The `RETURNING` example has to put the collision in the action's
                // `WHERE` and not in its `SET` list**, or it stops being about this
                // order: `coerce_to_type` wraps a `SET` right-hand side in an
                // `Expr::Cast`, which records at `PRIO_CAST` and so outranks the
                // `RETURNING` context fact *whichever* clause is walked first. Measured
                // both ways here — `… SET n = $1 RETURNING (v = $1)` reports `{int8}`
                // with this call above `walk_returning` and `{int8}` with it below,
                // while the `WHERE` spelling reports `{int8}` above and `{text}` below.
                // (18.6 refuses both spellings rather than choosing, because it unifies a
                // parameter's deductions across the statement and pglet does not — the
                // no-unification gap in `docs/compatibility.md`'s parameter-inference row. So what is
                // differential here is the SQLSTATE and *which clause it points at*, not
                // the OID.)
                if let Some(clause) = on_conflict {
                    self.clause(clause_rank::ON_CONFLICT);
                    self.walk_on_conflict(clause, t, schema);
                }
                self.walk_returning(
                    returning,
                    t,
                    || insert_returning_relations(t, schema),
                    schema,
                );
            }
            Plan::Update {
                table: t,
                alias,
                from,
                assignments,
                filter,
                // A copy of one `WHERE` conjunct's key, which the `filter` walk below
                // already types in its written position — walking it again would time
                // its parameters a second time.
                target_probe: _,
                returning,
            } => {
                self.relation(t, schema);
                self.clause(clause_rank::SET);
                self.walk_assignments(assignments, t, schema);
                // A `FROM` source may itself carry parameters (e.g. a derived
                // table); type them against the source's own table context.
                if let Some(f) = from {
                    self.clause(clause_rank::FROM);
                    self.walk_plan(f, schema, plan_table(f));
                }
                // The WHERE may span the target and the `FROM` source. We type
                // params against the target context (the established single-table
                // behavior); a `FROM`-qualified column whose qualifier is an alias
                // falls back through `infer_type`, which the spine's queries resolve.
                if let Some(f) = filter {
                    self.clause(clause_rank::WHERE);
                    self.record_bool_clause(f);
                    self.walk_expr(f, schema, Some(t));
                }
                self.walk_returning(
                    returning,
                    t,
                    || {
                        returning_relations(
                            t,
                            alias.as_deref().unwrap_or(t),
                            from.as_deref(),
                            schema,
                        )
                    },
                    schema,
                );
            }
            Plan::Delete {
                table: t,
                filter,
                // See `Plan::Update`'s arm: the `filter` walk types the probe's key.
                target_probe: _,
                returning,
            } => {
                self.relation(t, schema);
                if let Some(f) = filter {
                    self.clause(clause_rank::WHERE);
                    self.record_bool_clause(f);
                    self.walk_expr(f, schema, Some(t));
                }
                self.walk_returning(
                    returning,
                    t,
                    || returning_relations(t, t, None, schema),
                    schema,
                );
            }
            Plan::Sort {
                input, order_by, ..
            } => {
                let inner = plan_table(input);
                self.clause(clause_rank::ORDER_BY);
                for key in order_by {
                    self.walk_expr(&key.expr, schema, inner);
                }
                self.walk_plan(input, schema, table);
            }
            Plan::Limit {
                input,
                limit,
                offset,
            } => {
                // Postgres types LIMIT/OFFSET parameters as bigint.
                self.clause(clause_rank::LIMIT);
                for slot in [limit, offset] {
                    if let Some(LimitCount::Param(i)) = slot {
                        self.record(*i, TypeOid::Int8, PRIO_CONTEXT);
                        self.fix_event(*i, None);
                    }
                }
                self.walk_plan(input, schema, table);
            }
            Plan::Distinct { input } | Plan::JunkFilter { input, .. } => {
                self.walk_plan(input, schema, table);
            }
            Plan::DistinctOn { on_exprs, input } => {
                let inner = plan_table(input);
                self.clause(clause_rank::DISTINCT);
                for e in on_exprs {
                    self.walk_expr(e, schema, inner);
                }
                self.walk_plan(input, schema, table);
            }
            Plan::Aggregate {
                input,
                group_by,
                exprs,
                having,
                ..
            } => {
                let inner = plan_table(input);
                self.clause(clause_rank::GROUP_BY);
                for e in group_by {
                    self.walk_expr(e, schema, inner);
                }
                self.clause(clause_rank::TARGET);
                for e in exprs {
                    self.walk_expr(e, schema, inner);
                }
                if let Some(h) = having {
                    self.clause(clause_rank::HAVING);
                    self.record_bool_clause(h);
                    self.walk_expr(h, schema, inner);
                }
                self.walk_plan(input, schema, table);
            }
            Plan::Join {
                left, right, on, ..
            } => {
                // Join columns are table-qualified; rely on the qualifier, not a
                // single current-table context.
                // The two sides ahead of the condition, which is `transformFromClauseItem`'s
                // order and is what times a `$N` the condition shares with a derived table
                // on either side; the clause is re-entered because a side leaves its own
                // rank behind.
                self.walk_plan(left, schema, None);
                self.walk_plan(right, schema, None);
                self.clause(clause_rank::FROM);
                self.record_bool_clause(on);
                self.walk_expr(on, schema, None);
            }
            Plan::SubqueryScan {
                subquery, cte_rank, ..
            } => {
                self.clause(clause_rank::FROM);
                match cte_rank {
                    Some(rank) => self.walk_with_query(*rank, subquery, schema, None),
                    None => self.walk_scoped(subquery, schema, None),
                }
            }
            // Each set-op side opens its own FROM scope; type any `$N` in both.
            Plan::SetOp { left, right, .. } => {
                self.clause(clause_rank::FROM);
                self.walk_scoped(left, schema, plan_table(left));
                self.walk_scoped(right, schema, plan_table(right));
            }
            // A materialized-CTE statement: type params inside each CTE's sub-plans —
            // a data-modifying CTE's DML (e.g. `UPDATE … revoked_by = $2 WHERE
            // scope_id = $1`) from its own table, a recursive CTE's two terms each in
            // its own FROM scope (a self-reference is a `CteScan`, which carries no
            // params) — then params in the body.
            Plan::With { ctes, body } => {
                for cte in ctes {
                    for plan in cte.plans() {
                        self.walk_with_query(cte.rank, plan, schema, plan_table(plan));
                    }
                }
                self.clause(clause_rank::FROM);
                self.walk_scoped(body, schema, plan_table(body));
            }
            Plan::FunctionScan { args, .. } => {
                self.clause(clause_rank::FROM);
                for e in args {
                    self.walk_expr(e, schema, table);
                }
            }
            Plan::Window {
                input,
                window_exprs,
                ..
            } => {
                let inner = plan_table(input);
                self.clause(clause_rank::TARGET);
                for e in window_exprs {
                    self.walk_expr(e, schema, inner);
                }
                self.walk_plan(input, schema, table);
            }
            // An IndexScan carries the equality's value as `key_value`; if it is a
            // `$N`, type it from the indexed column (the planner lifts the
            // `col = $N` predicate into this node before inference runs, so the
            // column-comparison rule must be re-applied here). Then walk the value
            // for any nested params.
            #[cfg(feature = "index_acceleration")]
            Plan::IndexScan {
                table: t,
                key_column,
                key_value,
                ..
            } => {
                if let Expr::Parameter { index } = key_value.as_ref() {
                    self.record_key_column_param(*index, key_column, Some(t), schema);
                }
                self.clause(clause_rank::WHERE);
                self.lifted = true;
                self.walk_expr(key_value, schema, Some(t));
                self.lifted = false;
            }
            // An IndexRangeScan carries each bound as `lower`/`upper`; a `$N` bound
            // types from the indexed column (the planner lifts the `col {<,>,…} $N`
            // predicate into this node before inference runs, so the
            // column-comparison rule must be re-applied here, as for `IndexScan`).
            // Then walk both bound exprs for any nested params.
            #[cfg(feature = "index_acceleration")]
            Plan::IndexRangeScan {
                table: t,
                key_column,
                lower,
                upper,
                ..
            } => {
                self.clause(clause_rank::WHERE);
                self.lifted = true;
                for bound in [lower, upper].into_iter().flatten() {
                    if let Expr::Parameter { index } = bound.value.as_ref() {
                        self.record_key_column_param(*index, key_column, Some(t), schema);
                    }
                    self.walk_expr(&bound.value, schema, Some(t));
                }
                self.lifted = false;
            }
            // A GinScan carries the containment constants as `probes` (OR-unioned)
            // plus any scan-independent `guards`; a bare `$N` probe types from the
            // indexed column (the planner lifts the `col @> $N` predicate into this
            // node before inference runs). Nested params (`ARRAY[$n]::t[]`,
            // `jsonb_build_object('k', $n)`) and guard params (`$n::text IS NULL`) are
            // typed by walking each expr — the same cast/array/jsonb rules as anywhere.
            #[cfg(feature = "index_acceleration")]
            Plan::GinScan {
                table: t,
                key_column,
                probes,
                guards,
                ..
            } => {
                // Not a lifted conjunct ([`Self::lifted`]): the containment stays in the
                // recheck `Filter` above this node, which is walked first and in source
                // order, so these are second occurrences of what it already timed.
                self.clause(clause_rank::WHERE);
                for probe in probes {
                    if let Expr::Parameter { index } = probe {
                        self.record_key_column_param(*index, key_column, Some(t), schema);
                    }
                    self.walk_expr(probe, schema, Some(t));
                }
                for guard in guards {
                    self.walk_expr(guard, schema, Some(t));
                }
            }
            Plan::Scan { table: t, .. } => self.relation(t, schema),
            // No parameter-bearing expressions: a materialized-CTE read (its rows are
            // already computed), a virtual scan, DDL, transaction control.
            Plan::CteScan { .. }
            | Plan::VirtualScan { .. }
            // COPY carries no `$N` params (Postgres doesn't parameterize a COPY).
            | Plan::Copy { .. }
            | Plan::CreateTable { .. }
            | Plan::DropTable { .. }
            | Plan::Truncate { .. }
            | Plan::CreateIndex { .. }
            | Plan::DropIndex { .. }
            | Plan::CreateType { .. }
            | Plan::DropType { .. }
            | Plan::DropSchema { .. }
            | Plan::CreateSchema { .. }
            | Plan::AlterTableAddColumn { .. }
            | Plan::AlterTableDropColumn { .. }
            | Plan::AlterTableAddCheck { .. }
            | Plan::AlterTableAddConstraint { .. }
            | Plan::AlterTableSetDefault { .. }
            | Plan::AlterTableSetNotNull { .. }
            | Plan::AlterTableAddIdentity { .. }
            | Plan::CreateSequence { .. }
        | Plan::AlterSequence { .. }
        | Plan::DropSequence { .. }
            // pg_dump-preamble no-ops carry no `$N` params.
            | Plan::Noop { .. }
            | Plan::AlterTableUnperformed { .. }
            // A DO block can't carry bound `$N` params (Postgres doesn't parameterize
            // anonymous blocks), so its body statements need no param-type walk.
            | Plan::DoBlock { .. }
            | Plan::Empty
            | Plan::Savepoint { .. }
        | Plan::RollbackToSavepoint { .. }
        | Plan::ReleaseSavepoint { .. }
        | Plan::BeginTransaction { .. }
            | Plan::CommitTransaction
            | Plan::RollbackTransaction => {}
        }
    }

    /// Record a **clause-root** boolean context's bare `$N` — a `WHERE`, a `HAVING`
    /// or a `JOIN … ON` that is nothing but `$N`.
    ///
    /// The node positions are covered inside [`Self::walk_expr`] by the same rule; a
    /// clause root hangs on no node, so its own walker says so. `WHERE $1` bound to
    /// `t` is one row on 18.6 and was zero here, which is the spine's real binding
    /// path reporting OID 0 and decoding the value as text.
    ///
    /// A **stored** predicate takes no call site here and needs none: `CREATE TABLE t
    /// (a INT CHECK ($1))` and `CREATE INDEX i ON t (a) WHERE $1` describe **zero**
    /// parameters on 18.6 and here alike.
    fn record_bool_clause(&mut self, expr: &Expr) {
        if let Some(index) = bare_param_index(expr) {
            self.record(index, TypeOid::Bool, PRIO_CONTEXT);
        }
    }

    /// Walk an expression, recording parameter types from typed contexts and
    /// recursing into children.
    fn walk_expr(&mut self, expr: &Expr, schema: &Schema, table: Option<&str>) {
        self.record_node(expr, schema, table);
        // The chain-forming nodes recurse from this frame, every other one from
        // [`Self::walk_expr_rest`]'s, so a chain costs one frame a level
        // (`crate::depth`) — this walk cannot take the byte check. Each arm leaves the
        // node itself ([`Self::leave_expr`]), `walk_expr_rest` for its own.
        match expr {
            Expr::BinaryOp { left, right, .. } | Expr::AnyAll { left, right, .. } => {
                self.walk_expr(left, schema, table);
                self.walk_expr(right, schema, table);
                self.leave_expr();
            }
            Expr::Cast { expr: inner, .. }
            | Expr::CastTypmod { expr: inner, .. }
            | Expr::UnaryPlus(inner)
            | Expr::Negate(inner)
            | Expr::Not(inner)
            | Expr::IsNull(inner)
            | Expr::IsNotNull(inner)
            | Expr::BooleanTest { expr: inner, .. } => {
                self.walk_expr(inner, schema, table);
                self.leave_expr();
            }
            Expr::InList {
                expr: inner, list, ..
            } => {
                self.walk_expr(inner, schema, table);
                self.walk_exprs(list, schema, table);
                self.leave_expr();
            }
            // The subquery ahead of the left operand: `transformSubLink` analyses it
            // first.
            Expr::InSubquery {
                expr: inner, plan, ..
            } => {
                self.walk_scoped(plan, schema, None);
                self.walk_expr(inner, schema, table);
                self.leave_expr();
            }
            _ => self.walk_expr_rest(expr, schema, table),
        }
    }

    /// [`Self::walk_expr`]'s entry bookkeeping for [`Self::first_fix`], run from
    /// [`Self::record_node`]: a bare `$N` that **is** a clause root is timed here, by the
    /// clause, and this node's own operands are queued for its exit
    /// ([`Self::queue_operands`]).
    ///
    /// Neither half adds to `walk_expr`'s frame, which is what a chain pays a level of —
    /// on wasm the whole bound (`crate::depth`). The entry runs inside a frame that has
    /// returned before the recursion, and the exit reads the queue rather than the node,
    /// from a call in each arm: `wasm-opt` inlines a function with one caller whatever
    /// the Rust attribute says, and one exit call at the end of `walk_expr` came back
    /// inlined and put a `$1::int4 + $1::int4 + …` chain past V8's stack inside the depth
    /// limit (measured, `tests-web/integration/depth_edge.ts`).
    fn enter_expr(&mut self, expr: &Expr) {
        if self.expr_depth == 0 {
            if let Some(index) = bare_param_index(expr) {
                if self.root_fixes {
                    self.fix_event(index, None);
                } else {
                    self.defer_event(index);
                }
            }
        } else if let Expr::Parameter { index } = expr {
            self.operand_event(*index);
        }
        self.expr_depth += 1;
        self.pending_marks.push(self.pending.len());
        self.queue_operands(expr);
    }

    /// [`Self::walk_expr`]'s exit bookkeeping: the node's queued operands are timed now
    /// that they are walked.
    #[inline(never)]
    fn leave_expr(&mut self) {
        self.expr_depth -= 1;
        let mark = self.pending_marks.pop().unwrap_or(0);
        for (index, timing) in self.pending.split_off(mark) {
            match timing {
                Timing::Node(target) => self.fix_event(index, target.as_deref()),
                // Already timed where the walk reached it, unless an alias stood
                // between; the earliest event is the one kept.
                Timing::Operand => self.fix_event(index, None),
                Timing::Unresolved => {}
            }
        }
    }

    /// [`Self::walk_expr`] over each of `exprs`, in order.
    #[inline(never)]
    fn walk_exprs(&mut self, exprs: &[Expr], schema: &Schema, table: Option<&str>) {
        for e in exprs {
            self.walk_expr(e, schema, table);
        }
    }

    /// What [`Self::walk_expr`] records at `expr` before it walks any child: the
    /// boolean contexts and enum positions every node has, then the chain-forming
    /// nodes' own facts.
    #[inline(never)]
    fn record_node(&mut self, expr: &Expr, schema: &Schema, table: Option<&str>) {
        // Entering the node queues its operands, and types the ones it puts in a
        // boolean context ([`Self::queue_operands`]).
        self.enter_expr(expr);
        // The **enum** target of a bare `$N` this node puts at an enum position, which
        // is a second fact from the OID and not a refinement of one: an enum column's
        // `TypeOid` is `Text` either way.
        self.record_enum_targets(expr, schema, table);
        match expr {
            // Explicit cast directly on a parameter types it (the inner cast of
            // `$1::text::jsonb` wins, so the client sends the inner type).
            Expr::Cast {
                expr: inner,
                target_type,
            } => {
                match inner.as_ref() {
                    Expr::Parameter { index } => self.record(*index, *target_type, PRIO_CAST),
                    // `ARRAY[$n]::elem[]` — each element parameter takes the element type.
                    Expr::Array(elems) if target_type.is_array() => {
                        if let Some(elem_t) = target_type.array_element() {
                            for e in elems {
                                if let Expr::Parameter { index } = e {
                                    self.record(*index, elem_t, PRIO_CAST);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            // A typmod'd cast on a parameter types it the same as a plain cast
            // (`$1::varchar(3)` → text, `$1::numeric(4,1)` → numeric).
            Expr::CastTypmod {
                expr: inner,
                type_oid,
                ..
            } => {
                if let Expr::Parameter { index } = inner.as_ref() {
                    self.record(*index, *type_oid, PRIO_CAST);
                }
            }
            // Comparison / array-containment / LIKE against a typed sibling.
            Expr::BinaryOp { left, op, right } => {
                if matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::Lt
                        | BinaryOp::Le
                        | BinaryOp::Gt
                        | BinaryOp::Ge
                        | BinaryOp::Like
                        | BinaryOp::NotLike
                        | BinaryOp::Ilike
                        | BinaryOp::NotIlike
                        | BinaryOp::StartsWith
                        | BinaryOp::Containment
                        | BinaryOp::ContainedBy
                        | BinaryOp::IsDistinctFrom
                        | BinaryOp::IsNotDistinctFrom
                ) {
                    self.infer_from_sibling(left, right, schema, table);
                } else if matches!(op, BinaryOp::RegexMatch | BinaryOp::NotRegexMatch) {
                    // Every regex operator 18.6 has takes `text` on the right (`text ~ text`,
                    // `name ~ text`, `bpchar ~ text`), and a bare `$N` on the left resolves
                    // to `text` too — measured, `$1 ~ '^a'` binds a `text`. So a parameter
                    // operand is `text` whatever its sibling (kysely binds `nspname !~ $1`).
                    for side in [left.as_ref(), right.as_ref()] {
                        if let Expr::Parameter { index } = side {
                            self.record(*index, TypeOid::Text, PRIO_CONTEXT);
                        }
                    }
                } else if matches!(op, BinaryOp::Concat) {
                    // `text || text`: a parameter operand of string concatenation is
                    // text, regardless of the other operand (the spine's `LIKE $1 ||
                    // '%'` prefix search). Leaving it untyped reports OID 0, which a
                    // binary-protocol client (tokio-postgres) then tries to resolve —
                    // an unbounded type-introspection loop. Typing it text closes that.
                    for side in [left.as_ref(), right.as_ref()] {
                        if let Expr::Parameter { index } = side {
                            self.record(*index, TypeOid::Text, PRIO_CONTEXT);
                        }
                    }
                } else if matches!(
                    op,
                    BinaryOp::JsonExists | BinaryOp::JsonGet | BinaryOp::JsonGetText
                ) {
                    // `jsonb ? $1`: the right operand (the key) is text — force-typed
                    // like `Concat`'s operand (not sibling-inferred). Left untyped it
                    // reports OID 0, tripping a binary-protocol client's introspection.
                    //
                    // `jsonb -> $1` / `->> $1` resolve to **text** as well, and that is
                    // Postgres's own answer, not a convenience: `->` is overloaded on
                    // `(jsonb, text)` and `(jsonb, integer)`, and an unknown-typed
                    // parameter takes the *preferred* type of the string category —
                    // `PREPARE p AS SELECT '[1,2]'::jsonb -> $1` reports
                    // `parameter_types = {text}`, and `execute p(1)` looks up the
                    // object key `"1"` (NULL for an array). An array index therefore
                    // needs the explicit `-> $1::int` spelling on both engines. Typing
                    // it here also keeps a JS number argument (bound `int8` by the wasm
                    // adapter's runtime-type inference) from resolving to the
                    // nonexistent `(jsonb, bigint)` operator.
                    if let Expr::Parameter { index } = right.as_ref() {
                        self.record(*index, TypeOid::Text, PRIO_CONTEXT);
                    }
                } else if matches!(
                    op,
                    BinaryOp::JsonExistsAny
                        | BinaryOp::JsonExistsAll
                        | BinaryOp::JsonPathGet
                        | BinaryOp::JsonPathGetText
                ) {
                    // `jsonb ?|/?&/#>/#>> $1`: the right operand is a `text[]` —
                    // force-typed like the `?` key above.
                    if let Expr::Parameter { index } = right.as_ref() {
                        self.record(*index, TypeOid::TextArray, PRIO_CONTEXT);
                    }
                }
            }
            // `left op ANY($arr)` / `ALL` — a bare array parameter is the array of
            // the left operand's type.
            Expr::AnyAll { left, right, .. } => {
                if let Expr::Parameter { index } = right.as_ref()
                    && let Some(arr_t) = self
                        .infer_col_type(left, schema, table)
                        .and_then(TypeOid::array_of)
                {
                    self.record(*index, arr_t, PRIO_CONTEXT);
                }
            }
            // `expr IN ($1, $2, ...)` — list parameters take the left expr's type.
            Expr::InList {
                expr: inner, list, ..
            } => {
                if let Some(t) = self.infer_col_type(inner, schema, table) {
                    for item in list {
                        if let Expr::Parameter { index } = item {
                            self.record(*index, t, PRIO_CONTEXT);
                        }
                    }
                }
            }
            // `+$N` types the parameter **float8**, from the operator — not from a
            // sibling column. Postgres has no polymorphic unary `+`, so resolving
            // `+ unknown` picks the `float8` form (the same resolution that makes
            // `SELECT +'abc'` fail `22P02 invalid input syntax for type double
            // precision`), and the sibling never gets a say: `WHERE int_col = +$1`
            // reports `$1` as float8 there, not int4.
            //
            // Typing it here at all matters because every other probe in this module
            // matches a **bare** `Expr::Parameter`. Once the parser began keeping the
            // `+` node (`Expr::UnaryPlus`) rather than discarding it, `+$1` stopped
            // matching any of them and reported OID 0 — the unknown-type report this
            // module exists to avoid. `PRIO_CONTEXT`, so an explicit cast still wins.
            Expr::UnaryPlus(inner) => {
                if let Expr::Parameter { index } = inner.as_ref() {
                    self.record(*index, TypeOid::Float8, PRIO_CONTEXT);
                }
            }
            _ => {}
        }
    }

    /// [`Self::walk_expr`]'s arms for every node it does not walk itself.
    #[inline(never)]
    fn walk_expr_rest(&mut self, expr: &Expr, schema: &Schema, table: Option<&str>) {
        match expr {
            // Walked by `walk_expr` itself: never here.
            Expr::Cast { .. }
            | Expr::CastTypmod { .. }
            | Expr::BinaryOp { .. }
            | Expr::UnaryPlus(_)
            | Expr::Negate(_)
            | Expr::Not(_)
            | Expr::IsNull(_)
            | Expr::IsNotNull(_)
            | Expr::BooleanTest { .. }
            | Expr::AnyAll { .. }
            | Expr::InList { .. }
            | Expr::InSubquery { .. } => {
                // `walk_expr` enters and leaves the node itself; this frame's entry is
                // left first, so the queue stays balanced.
                self.leave_expr();
                return self.walk_expr(expr, schema, table);
            }
            // A scalar `(SELECT ...)` or `[NOT] EXISTS(SELECT ...)`: walk the inner
            // plan with no outer-table context so a `$N` inside it (e.g. `hash = $1`
            // in `EXISTS(SELECT 1 FROM fact WHERE hash = $1)`) gets typed from the
            // inner scan's columns. Both are non-correlated — the plan resolves its
            // own tables.
            Expr::Subquery { plan } | Expr::Exists { plan, .. } => {
                self.walk_scoped(plan, schema, None);
            }
            Expr::Alias { expr: inner, .. }
            // unnest(arr): recurse into the inner array expression. The spine's
            // arg is `$2::text[]` (typed by the `Cast{Parameter}` arm above), so
            // recursion is all that's needed here.
            | Expr::Unnest(inner) => self.walk_expr(inner, schema, table),
            // lower(text): its argument is text, so a bare `$N` arg types as text
            // (the spine's `LOWER(username) = LOWER($1)` account lookups). Left
            // untyped it reports OID 0, which trips a binary-protocol client's
            // (tokio-postgres) type introspection — the same hazard the `||` arm
            // closes. A nested cast (`LOWER($1::text)`) already wins at PRIO_CAST.
            Expr::Lower(inner) => {
                if let Expr::Parameter { index } = inner.as_ref() {
                    self.record(*index, TypeOid::Text, PRIO_CONTEXT);
                }
                self.walk_expr(inner, schema, table);
            }
            Expr::Aggregate { arg, .. } => self.walk_expr(arg, schema, table),
            Expr::Window {
                partition_by,
                order_by,
                ..
            } => {
                for e in partition_by {
                    self.walk_expr(e, schema, table);
                }
                for key in order_by {
                    self.walk_expr(&key.expr, schema, table);
                }
            }
            Expr::Array(elems) => {
                for e in elems {
                    self.walk_expr(e, schema, table);
                }
            }
            // jsonb_build_object / coalesce / a math function (abs/round/…) / a
            // sequence function (`setval('s', $1)`): recurse into each argument.
            // Their `$N` args are typed by an enclosing cast (the `Cast{Parameter}`
            // arm above) or by the function's numeric family, so recursion is all
            // that's needed here (a sequence function's name is a literal, not an arg).
            Expr::JsonbBuildObject(args)
            | Expr::Coalesce(args)
            | Expr::MathFn { args, .. }
            | Expr::SequenceFn { args, .. } => {
                for e in args {
                    self.walk_expr(e, schema, table);
                }
            }
            // A string function (upper/substring/trim/…): a bare `$N` in a **text**
            // argument position types as text (like `Lower` above) — left untyped it
            // reports OID 0, which trips a binary-protocol client's (tokio-postgres)
            // type introspection. A nested cast (`upper($1::text)`) already wins at
            // PRIO_CAST; the int positions (substring's start/length, lpad's length)
            // are left to their own typing.
            Expr::StringFn { func, args } => {
                // `$1::<enum>` is a cast on the parameter, and types it as the `text`
                // cast it is planned under always did — at `PRIO_CAST`, not as a
                // function argument's context.
                if matches!(func, StringFunc::EnumIn)
                    && let Some(Expr::Parameter { index }) = args.first()
                {
                    self.record(*index, TypeOid::Text, PRIO_CAST);
                }
                for (i, e) in args.iter().enumerate() {
                    if let Expr::Parameter { index } = e {
                        if string_fn_text_arg(*func, i) {
                            self.record(*index, TypeOid::Text, PRIO_CONTEXT);
                        } else if let Some(t) = string_fn_integer_arg(*func, i, args.len()) {
                            self.record(*index, t, PRIO_CONTEXT);
                        }
                    }
                    self.walk_expr(e, schema, table);
                }
            }
            // A variadic function: `concat_ws`'s separator (arg0) is text, so a bare
            // `$N` there types as text; every other variadic argument is polymorphic
            // (`any`), so only the separator is typed.
            Expr::VariadicFn { func, args } => {
                if matches!(func, VariadicFunc::ConcatWs)
                    && let Some(Expr::Parameter { index }) = args.first()
                {
                    self.record(*index, TypeOid::Text, PRIO_CONTEXT);
                }
                for e in args {
                    self.walk_expr(e, schema, table);
                }
            }
            // `EXTRACT`/`date_part`: the field arg is text (a bare `$N` there types as
            // text, like `concat_ws`'s separator); the source is polymorphic temporal,
            // typed by its own cast. Recurse into both.
            Expr::Extract { source, field, .. } => {
                if let Expr::Parameter { index } = field.as_ref() {
                    self.record(*index, TypeOid::Text, PRIO_CONTEXT);
                }
                self.walk_expr(field, schema, table);
                self.walk_expr(source, schema, table);
            }
            Expr::Case {
                operand,
                when_clauses,
                else_result,
            } => {
                if let Some(op) = operand {
                    self.walk_expr(op, schema, table);
                }
                for (condition, result) in when_clauses {
                    self.walk_expr(condition, schema, table);
                    self.walk_expr(result, schema, table);
                }
                if let Some(else_expr) = else_result {
                    self.walk_expr(else_expr, schema, table);
                }
            }
            // A bare parameter occurrence: ensure the slot exists (it may carry no
            // inferable type, but the count must still cover it).
            Expr::Parameter { index } => self.ensure(*index),
            // Leaves with no parameters.
            Expr::Column { .. }
            | Expr::Literal(_)
            | Expr::Default
            | Expr::Bound(_)
            | Expr::Builtin(_) => {}
        }
        self.leave_expr();
    }

    /// Type a bare `$N` in an assignment value (or a CASE/COALESCE result branch)
    /// as `col_ty` — the column an `UPDATE … SET col = <value>` RHS is coerced to.
    ///
    /// Peels the lowering-added outer coercion cast to the column type (so a param
    /// buried in `CASE WHEN … THEN $N …` is reached) but never descends through an
    /// *inner* cast to a different type (`$3::text` inside a `… ::jsonb` value keeps
    /// its own text type — recorded at the higher `PRIO_CAST` by `walk_expr`). Only
    /// a genuinely bare `$N` is typed here.
    fn record_value_param(&mut self, expr: &Expr, col_ty: TypeOid) {
        match expr {
            Expr::Parameter { index } => self.record(*index, col_ty, PRIO_CONTEXT),
            // The lowering coercion cast to the column type — its operand is the
            // real value (e.g. a CASE whose result branches coerce to the column).
            Expr::Cast {
                expr: inner,
                target_type,
            } if *target_type == col_ty => self.record_value_param(inner, col_ty),
            Expr::Case {
                when_clauses,
                else_result,
                ..
            } => {
                for (_, result) in when_clauses {
                    self.record_value_param(result, col_ty);
                }
                if let Some(e) = else_result {
                    self.record_value_param(e, col_ty);
                }
            }
            Expr::Coalesce(args) => {
                for a in args {
                    self.record_value_param(a, col_ty);
                }
            }
            _ => {}
        }
    }

    /// Walk a `SET` list — an `UPDATE`'s or an `ON CONFLICT DO UPDATE` action's, which
    /// are the same rule and are written here once.
    ///
    /// Two probes per assignment, and which one wins is **not** decided by their order
    /// alone. [`Self::record_value_param`] records at `PRIO_CONTEXT`;
    /// [`Self::walk_expr`] records at `PRIO_CAST` for a cast and at `PRIO_CONTEXT` for
    /// everything else it types. So a cast anywhere in the right-hand side outranks the
    /// assignment target whatever the order, while an *equal*-priority context inside it
    /// does not — [`Self::record`] keeps the first fact at equal priority, and the target
    /// probe is written first. Reversing these two lines is therefore a behaviour change
    /// for a right-hand side that carries both, not a reordering.
    ///
    /// **What the target-column probe actually carries is narrower than it reads.** The
    /// lowering wraps a whole right-hand side in an `Expr::Cast` to the column type, and branch
    /// unification wraps each `CASE`/`COALESCE` branch that has a *typed* sibling in a
    /// cast of its own — both of which `walk_expr`'s `Cast` arm reads. So `updated_by =
    /// $7` and `path = CASE WHEN $5::bool THEN $6 ELSE path END` are typed **without**
    /// this probe (measured: deleting it leaves `cell_update_partial_reports_all_nine_params`
    /// green, and the lowered plan carries `Cast { Parameter(7), Uuid }` and
    /// `Cast { Parameter(6), Text }`). What has no sibling to unify against is an
    /// **all**-unknown one: `SET v = CASE WHEN $1::bool THEN $2 ELSE $3 END` lowers with
    /// `$2`/`$3` bare inside the assignment cast, so without this probe they report OID
    /// 0 — which trips a binary-protocol client's (tokio-postgres) type introspection
    /// into the `Parameters` error — against `{boolean,text,text}` on 18.6 *and* here.
    ///
    /// `col` is looked up on the **catalog** rather than through
    /// [`Self::infer_col_type`], because an assignment target is a column of the DML
    /// target and not a row reference — the `SET` left-hand side is the one name in
    /// either clause that no qualifier and no `excluded` can reach.
    fn walk_assignments(&mut self, assignments: &[(String, Expr)], table: &str, schema: &Schema) {
        for (col, e) in assignments {
            if let Some(col_ty) = (Expr::Column {
                table: None,
                name: col.clone(),
            })
            .infer_type(&PlannerScope::catalog(schema, Some(table)))
            {
                self.record_value_param(e, col_ty);
            }
            self.walk_expr(e, schema, Some(table));
        }
    }

    /// Record a parameter on either side of a comparison from its typed sibling.
    fn infer_from_sibling(
        &mut self,
        left: &Expr,
        right: &Expr,
        schema: &Schema,
        table: Option<&str>,
    ) {
        if let Expr::Parameter { index } = left
            && let Some(t) = self.infer_col_type(right, schema, table)
        {
            self.record(*index, t, PRIO_CONTEXT);
        }
        if let Expr::Parameter { index } = right
            && let Some(t) = self.infer_col_type(left, schema, table)
        {
            self.record(*index, t, PRIO_CONTEXT);
        }
    }

    /// Infer an expression's type, falling back to the FROM-scope alias map when a
    /// qualified column's qualifier is a FROM **alias** the single `table` context
    /// can't resolve. Plain [`Expr::infer_type`] already handles the single-table and
    /// real-table-qualifier cases (and non-column expressions); this adds alias→table
    /// resolution across a join's FROM list (`act.id` over `FROM actor act JOIN …`).
    fn infer_col_type(&self, expr: &Expr, schema: &Schema, table: Option<&str>) -> Option<TypeOid> {
        self.infer_col_resolved(expr, schema, table).map(|t| t.oid)
    }

    /// [`Self::infer_col_type`] with the **enum identity** beside the OID — one
    /// derivation the two callers project, rather than a second lookup that would have
    /// to agree with the first. The identity comes from `PlannerScope::resolved_type`,
    /// which is the gates' own reader.
    fn infer_col_resolved(
        &self,
        expr: &Expr,
        schema: &Schema,
        table: Option<&str>,
    ) -> Option<ResolvedType> {
        // Inside a clause whose namespace is not the catalog's — `RETURNING`'s range
        // table, or the `ON CONFLICT` action's target-plus-`excluded` (see
        // [`Self::clause_scope`]) — that namespace is the *whole* namespace: the catalog
        // lookup below would answer for a relation the statement never mentions, and for
        // the action it answers for one the statement's qualifier only *spells*.
        if let Some(relations) = &self.clause_scope {
            let columns = relations.flatten();
            return PlannerScope::new(
                schema,
                TypeScope::Input {
                    relations,
                    columns: &columns,
                    enclosing: None,
                },
            )
            .resolved_type(expr);
        }
        if let Some(t) = PlannerScope::catalog(schema, table).resolved_type(expr) {
            return Some(t);
        }
        let Expr::Column {
            table: qualifier,
            name,
        } = expr
        else {
            return None;
        };
        // Resolve the column's owning relation: an alias-qualified column via the
        // FROM-scope map (`act` → `actor`, or a virtual scan's qualified name), a
        // directly-qualified column via the qualifier itself, else the threaded
        // `table` context (which `plan_table` sets to a virtual scan's qualified
        // name). `column_resolved_in_relation` then resolves against real OR virtual
        // tables — `infer_type` above only saw `Schema.tables`, so a
        // `VirtualScan` column (`information_schema.tables.table_name`) reaches here.
        let relation = match qualifier {
            Some(q) => self
                .from_scope
                .iter()
                .find(|(alias, _)| alias == q)
                .map_or(q.as_str(), |(_, t)| t.as_str()),
            None => table?,
        };
        column_resolved_in_relation(schema, relation, name)
    }

    /// Walk a plan that opens its **own** FROM scope (the statement root or a
    /// subquery), with [`Self::from_scope`] set to that plan's FROM relations for
    /// the duration and restored after — so a subquery's qualified columns resolve
    /// against its own tables rather than being overridden by an enclosing scope's.
    ///
    /// **Replaces rather than chains, and that is a gap, not the rule.** Postgres
    /// chains: the innermost level wins and the outer levels still bind, which is why
    /// a correlated `$N` there is typed from the level it actually names. Here the
    /// outer level's alias→table map is gone for the duration, so a qualified outer
    /// reference falls through to reading its qualifier as a relation name — which
    /// resolves for an unaliased outer relation and reports OID 0 for an aliased one.
    /// Measured on `ParameterDescription` (extended protocol, raw v3 `Parse` +
    /// `Describe(S)` + `Sync`) against 18.6, and recorded with its controls in
    /// `EDGES.md` — including why it has no fixture, which is
    /// that a faithful `@param` one is **red** rather than that the harness is blind.
    fn walk_scoped(&mut self, plan: &Plan, schema: &Schema, table: Option<&str>) {
        let mut scope = Vec::new();
        collect_from_aliases(plan, &mut scope);
        let saved = std::mem::replace(&mut self.from_scope, scope);
        // A clause namespace is per-level in the same way: it must not override a
        // subquery's own FROM relations. Whether an outer level should stay *reachable*
        // behind them is the gap above, not something this clearing decides.
        //
        // **But "the gap above" was written before a clause namespace carried a
        // pseudo-relation, and for that member the clearing is not merely a lost
        // shortcut.** A cleared `RETURNING` scope degrades to the catalog lookup, which
        // re-finds an outer relation that *is* a catalog table (an aliased one is the
        // OID-0 half of the gap above, so this is a narrowing, not an exemption); the
        // `ON CONFLICT` action's `excluded` is a catalog table under no spelling, so
        // nothing re-finds it and the reference types off whatever else answers.
        // Measured over `oc_probe(id INT PRIMARY KEY, v TEXT, n BIGINT)` and
        // `oc_src(id INT, v TEXT)`: `… DO UPDATE SET v = 'z' WHERE EXISTS (SELECT 1 FROM
        // oc_src s WHERE s.id = $1 AND excluded.n = $2)` reports `{integer,0}` here
        // against 18.6's `{integer,bigint}` — and `{integer,text}` with a real
        // `excluded(id INT, n TEXT, v TEXT)` in the catalog, which 18.6 ignores. The
        // control that separates the subquery from the reference is the same comparison
        // written directly in the action's `WHERE`: `{bigint}` on both engines, shadow
        // table or not. It is **masked** rather than harmless — at a conflicting key the
        // action's `EXISTS` is pglet's own `0A000 … requires execution context` at
        // execute and the statement fails there, writing nothing, and at a free key the
        // action is not evaluated at all and the `INSERT` succeeds on a `VALUES` row
        // `$2` does not reach; the wrong OID reaches a `Describe` reply and a `Bind`
        // decode, and no row carries a value read under it. Recorded in `docs/compatibility.md`'s
        // parameter-inference row.
        let saved_clause = self.clause_scope.take();
        // A new query level in parse-analysis order, entered at this point of the
        // enclosing one (see [`Self::position`]).
        self.seq += 1;
        if let Some(level) = self.position.last_mut() {
            level.1 = self.seq;
        }
        self.position.push((clause_rank::WITH, 0));
        let saved_depth = std::mem::replace(&mut self.expr_depth, 0);
        // Entered from inside an expression, this level is an expression's subquery.
        let from_expr = u32::from(saved_depth > 0);
        self.expr_subqueries += from_expr;
        let saved_root_fixes = self.root_fixes;
        self.walk_plan(plan, schema, table);
        self.root_fixes = saved_root_fixes;
        self.expr_subqueries -= from_expr;
        self.expr_depth = saved_depth;
        self.position.pop();
        self.clause_scope = saved_clause;
        self.from_scope = saved;
    }

    /// Walk one `WITH` query's plan — a materialized entry of [`Plan::With`], or an
    /// ordinary read-only CTE wherever a reference inlined it — at the place 18.6's
    /// parse analysis reaches it: in the statement's `WITH` clause, ahead of every clause
    /// of the statement's own query (`transformWithClause` runs first in each of
    /// `transformSelectStmt`, `transformInsertStmt`, `transformUpdateStmt` and
    /// `transformDeleteStmt`), the list's queries in their analysis order (`rank`).
    ///
    /// So the position is **re-rooted** rather than descended into: a reference inside a
    /// scalar subquery of the select list reads a query 18.6 analysed before that select
    /// list, and one CTE's reference inside another's body reads a query analysed before
    /// that body. A `WITH` list belongs to the statement's own level (pglet parses none on
    /// a nested query), so the root is the outermost level. The second level is the
    /// sequence of this walk: it orders two references to one CTE, which walk the same
    /// query, so whichever comes first answers for both.
    ///
    /// A `WITH` query this never walks is one no reference in the plan reaches: one the
    /// statement does not reference, or reads only from a CTE it does not reference
    /// (`EDGES.md` row 241) — so a parameter typed only there is left untyped for the
    /// statement that is walked (row 276).
    fn walk_with_query(&mut self, rank: u32, plan: &Plan, schema: &Schema, table: Option<&str>) {
        let outer = std::mem::replace(
            &mut self.position,
            vec![(clause_rank::WITH, rank), (clause_rank::WITH, 0)],
        );
        self.walk_scoped(plan, schema, table);
        self.position = outer;
    }

    /// Walk a `RETURNING` list with [`Self::clause_scope`] set to that clause's own
    /// namespace — `returning_relations`, the same derivation the plan-time binding gate
    /// (`lower_returning`) and the planner-declared type walk
    /// (`Plan::returning_output_columns`) read. (The executor's projection reaches the
    /// same relations off its materialized rows instead; see `returning_relations`.)
    ///
    /// `relations` is that namespace, built by the **caller** and taken lazily so an
    /// empty clause pays nothing for it. Each DML arm names its own derivation there
    /// rather than passing the parts of one: an `INSERT`'s is
    /// [`insert_returning_relations`], the shared spelling of its two `None`s (no target
    /// alias, no source relation) whose readers that function enumerates — this walk
    /// among them, since spelling `(t, t, None)` inline here is exactly the seam that
    /// function's doc warns about. An `UPDATE`'s takes the target's **effective** name
    /// (its alias when one was written) and the `UPDATE … FROM` source, so a
    /// source-qualified column types from the source rather than falling back to the
    /// target's column of the same name; a `DELETE`'s carries neither, and that is its
    /// own fact rather than the `INSERT` one — `Plan::Delete` has no alias field and no
    /// `USING`.
    ///
    /// `table` is **inert for the duration of this walk**, and saying it is what an
    /// *unqualified* column types against gets it backwards: [`Self::infer_col_type`]
    /// returns off `clause_scope` before `TypeScope::Catalog(table)` is ever consulted, so
    /// the clause namespace answers a bare name too. Measured over `dt(id, b BIGINT)` and
    /// `ds(id, b INT, k INT) s` — `RETURNING (k = $1)` reports `int4` from the source,
    /// where the threaded target has no `k` at all and would report OID 0. Nor does it
    /// come back once a subquery clears the scope: the expressions that reach
    /// [`Self::walk_scoped`] are `Expr::Subquery`, `Expr::Exists` and `Expr::InSubquery`,
    /// and each passes `None` for the table itself, so the inner level types off its own
    /// relations. It is threaded only because [`Self::walk_expr`] takes it — contrast
    /// [`Self::walk_on_conflict`], where the same parameter *is* live, through
    /// [`Self::walk_assignments`]' target-column probe, which reads
    /// `TypeScope::Catalog(Some(table))` directly rather than through
    /// [`Self::infer_col_type`] and so never meets that short-circuit.
    fn walk_returning(
        &mut self,
        returning: &[Expr],
        table: &str,
        relations: impl FnOnce() -> RelationColumns,
        schema: &Schema,
    ) {
        if returning.is_empty() {
            return;
        }
        let saved = self.clause_scope.replace(relations());
        self.clause(clause_rank::RETURNING);
        for e in returning {
            self.walk_expr(e, schema, Some(table));
        }
        self.clause_scope = saved;
    }

    /// Walk an `ON CONFLICT` clause, typing every `$N` it carries — the arbiter's against
    /// the catalog under the insert target, the action's against the target plus
    /// `excluded`.
    ///
    /// The clause used to be swallowed by the `Plan::Insert` arm's `..`, so a `$N`
    /// living **only** here was never even counted: `INSERT … ON CONFLICT (id) DO UPDATE
    /// SET v = $1` reported `ParameterDescription` **n = 0** and every `Bind` failed
    /// `08P01 bind message supplies 1 parameters, but prepared statement "" requires 0`
    /// — a bound upsert unusable from any extended-protocol driver. The control that
    /// identifies the missing arm rather than a typing gap: the same statement's
    /// `VALUES ($1, …)` position was counted throughout.
    ///
    /// The positions are walked in `transformOnConflictClause`'s order, and they are
    /// these: the **arbiter** (its key parts, then its partial-index predicate), then
    /// the `DO UPDATE` `SET` list, then the action's `WHERE`. That is not cosmetic: with
    /// `record` keeping the first fact at equal priority, the arbiter's predicate types
    /// `$1` in `ON CONFLICT (id) WHERE n > $1 DO UPDATE SET v = 'z' WHERE t.v = $1` and
    /// the action's `WHERE` does not re-type it — which is 18.6's answer too, reported
    /// there as `42883 operator does not exist: text = bigint` pointing at the action's
    /// `WHERE`.
    ///
    /// **The order is not the whole story, and one collision escapes it.** `record`'s
    /// first-wins rule holds only at *equal* priority, and `coerce_to_type` wraps an
    /// assignment right-hand side in a plain `Expr::Cast` that [`Self::walk_expr`]
    /// cannot tell from a user cast — so a `$N` written in the arbiter **and** in the
    /// `SET` list takes the `SET` list's type however the walk is ordered.
    /// `ON CONFLICT ((id + $1)) DO UPDATE SET v = $1` is 18.6's `{integer}` and `text`
    /// here; `ON CONFLICT (id) WHERE n > $1 DO UPDATE SET v = $1` is its `{bigint}` and
    /// `text` here. The class is older and wider than this clause — its `Plan::Update`
    /// member is `UPDATE t SET v = $1 WHERE n = $1`, likewise `{bigint}` there and
    /// `text` here — and it is recorded in `docs/compatibility.md`'s parameter-inference row.
    ///
    /// The `SET` list is not this function's rule: it is an assignment list like an
    /// `UPDATE`'s, so both go through [`Self::walk_assignments`], where what the
    /// target-column probe does and does not carry is stated once.
    ///
    /// **The action walks under a namespace of its own and the arbiter does not**, which
    /// is the split the two `self.clause_scope` assignments below draw. Reading
    /// `excluded.<col>` off `TypeScope::Catalog`'s fallback — a qualifier naming no
    /// catalog table falls back to the threaded `table` — lands on the target's columns
    /// and so *looks* right, and is right until the catalog holds a table called
    /// `excluded`, at which point the fallback never runs and the shadow table answers.
    /// Measured over `oc_probe(id INT PRIMARY KEY, v TEXT, n BIGINT)` beside a real
    /// `excluded(id INT, n TEXT, v TEXT)`: `… DO UPDATE SET v = 'z' WHERE excluded.n =
    /// $1` reported `{text}` against 18.6's `{bigint}`, and the bound statement was a
    /// loud `42883 operator does not exist: bigint = text` where 18.6 updates the row —
    /// a false rejection out of an ordinary table name. The scope is
    /// `on_conflict_action_relations`, the [`RelationColumns`] twin of the namespace
    /// `lower_on_conflict` gates the same two positions against.
    ///
    /// The arbiter is **excluded from that scope on Postgres's own authority**, not out
    /// of caution: `ON CONFLICT ((excluded.id)) DO UPDATE …` and `ON CONFLICT (id) WHERE
    /// excluded.n > 0 DO UPDATE …` are both `42P01 invalid reference to FROM-clause entry
    /// for table "excluded"` on 18.6 (measured), so `excluded` is the action's alone.
    /// Setting one scope over the whole clause would type an arbiter reference 18.6
    /// refuses to resolve at all.
    ///
    /// What the closed scope does **not** buy is a name gate: `TypeScope::Input` falls
    /// through an unmatched qualifier to the flattened column list, so a bogus qualifier
    /// still types loosely here. What bounds *that* is one layer up — `lower_on_conflict`
    /// resolves both halves of the action against `on_conflict_action_scope`, a closed
    /// `ColumnScope`, so a qualifier naming anything else — a real sibling table, a typo,
    /// or the quoted `"EXCLUDED"` the fold does not reach — is `42P01` before any plan
    /// exists to infer over (measured identical on both engines). The **arbiter** has no
    /// such layer, which is where the looseness is still visible; see `docs/compatibility.md`'s
    /// parameter-inference row.
    fn walk_on_conflict(&mut self, clause: &OnConflictClause, table: &str, schema: &Schema) {
        if let Some(target) = &clause.target {
            for key in &target.keys {
                self.walk_expr(key, schema, Some(table));
            }
            if let Some(predicate) = &target.predicate {
                self.walk_expr(predicate, schema, Some(table));
            }
        }
        let OnConflictAction::DoUpdate {
            assignments,
            where_predicate,
        } = &clause.action
        else {
            return;
        };
        let saved = self
            .clause_scope
            .replace(on_conflict_action_relations(table, schema));
        self.walk_assignments(assignments, table, schema);
        if let Some(predicate) = where_predicate {
            self.record_bool_clause(predicate);
            self.walk_expr(predicate, schema, Some(table));
        }
        self.clause_scope = saved;
    }
}

/// A bare `NULL` literal — the operand that makes an `IS [NOT] DISTINCT FROM` a null
/// test ([`Inference::queue_operands`]).
const fn is_null_literal(expr: &Expr) -> bool {
    matches!(expr, Expr::Literal(Literal::Null))
}

/// Whether argument `i` of a string function is a **text** position, so a bare `$N`
/// there types as text. Mirrors `eval_string_fn`'s per-argument text/int split: the
/// integer positions (substring's start/length, lpad/rpad's length) return `false`.
// arms grouped per function for documentation; several share an `i <= 1` body
#[allow(clippy::match_same_arms)]
const fn string_fn_text_arg(func: StringFunc, i: usize) -> bool {
    match func {
        // arg0 text; substring's start/length are integer.
        StringFunc::Upper | StringFunc::Length | StringFunc::Substring => i == 0,
        // (str, from, to) — all text.
        StringFunc::Replace => i <= 2,
        // (str, chars) — both text.
        StringFunc::Ltrim | StringFunc::Rtrim | StringFunc::Btrim => i <= 1,
        // (str, sub) — both text.
        StringFunc::Strpos | StringFunc::Position => i <= 1,
        // (str, len, fill) — arg1 (len) is integer.
        StringFunc::Lpad | StringFunc::Rpad => i == 0 || i == 2,
        // (str, delim, n) — arg2 (n) is integer.
        StringFunc::SplitPart => i <= 1,
        // (field, source) — arg0 (field) is text; the source is temporal, not text.
        StringFunc::DateTrunc => i == 0,
        // (name, missing_ok) — arg0 (name) is text; arg1 is boolean.
        StringFunc::CurrentSetting => i == 0,
        // (include_implicit) — boolean, no text argument at all.
        StringFunc::CurrentSchemas => false,
        // (oid) — an integer, not text.
        StringFunc::PgGetConstraintdef => false,
        // `x::regclass` takes either an OID or a relation name, so a bare `$N` there is
        // not a *text* position on 18.6 either (it resolves the cast's own input type);
        // reporting `false` leaves the parameter untyped rather than mis-typed.
        StringFunc::Regclass => false,
        // (date | timestamp | timestamptz) — no text position.
        StringFunc::IsFinite => false,
        // (oid, int4) and (bigint) / (int4, int4) — integer positions, typed by
        // `string_fn_integer_arg`.
        StringFunc::ColDescription | StringFunc::AdvisoryXactLock => false,
        // (name), (table, column), (schema, privilege) — every argument text.
        StringFunc::QuoteIdent
        | StringFunc::PgGetSerialSequence
        | StringFunc::HasSchemaPrivilege => true,
        // `$1::<enum>` — the operand; the name and labels are the node's own literals.
        // Typed at `PRIO_CAST` by the walk's own arm, as the `text` cast it replaced was.
        StringFunc::EnumIn => i == 0,
        // (text | bytea) — an untyped `$N` resolves to `text`, the string category's
        // preferred type, as an unknown literal there does.
        StringFunc::Md5 | StringFunc::OctetLength => i == 0,
        // (str, n) and (str, start [, length]) — the integer positions are
        // `string_fn_integer_arg`'s; an untyped `$1` resolves to `text` there too.
        StringFunc::Left | StringFunc::Right | StringFunc::Substr => i == 0,
    }
}

/// The integer type 18.6 resolves argument `i` of a call to `func` with `n` arguments to,
/// where every overload of that arity agrees — so a bare `$N` there types as 18.6 types it
/// (`pg_advisory_xact_lock($1)` is `bigint`, `pg_advisory_xact_lock($1, $2)` two `integer`s,
/// `col_description`'s column number `integer`). `None` elsewhere, left to the argument's
/// own typing: `col_description`'s `oid` position among them, which pglet has no parameter
/// decoding for.
const fn string_fn_integer_arg(func: StringFunc, i: usize, n: usize) -> Option<TypeOid> {
    match (func, n) {
        (StringFunc::AdvisoryXactLock, 1) => Some(TypeOid::Int8),
        (StringFunc::AdvisoryXactLock, 2) => Some(TypeOid::Int4),
        (StringFunc::ColDescription, 2) if i == 1 => Some(TypeOid::Int4),
        // Every overload takes an `integer` past the first argument.
        (StringFunc::Left | StringFunc::Right | StringFunc::Substr, _) if i > 0 => {
            Some(TypeOid::Int4)
        }
        _ => None,
    }
}

/// A column's [`ResolvedType`] within a named relation, resolving against a real
/// `Schema.tables` table first and then a virtual catalog table
/// (`information_schema.*` / `pg_*`), whose columns are synthesized rather than
/// stored in the schema. `relation` is a `from_name`-resolvable name (a real table
/// name, or a virtual scan's [`CatalogTable::qualified_name`]).
///
/// Both sides carry the identity, so a real table's enum column keeps it on this
/// fallback path too — the path is reached for a FROM **alias** the single-table
/// context cannot resolve, which is an ordinary join spelling and not an exotic one.
/// A virtual catalog table's `ColumnDesc` carries an `enum_type` field and no virtual
/// column sets it, so that side reads whatever the descriptor says rather than
/// asserting `None`.
fn column_resolved_in_relation(
    schema: &Schema,
    relation: &str,
    column: &str,
) -> Option<ResolvedType> {
    if let Some(table_def) = schema.get_table(relation) {
        return table_def
            .columns
            .iter()
            .find(|c| c.name == *column)
            .map(|c| ResolvedType::new(c.type_oid, c.enum_type.clone()));
    }
    CatalogTable::from_name(relation)?
        .columns()
        .into_iter()
        .find(|c| c.name == *column)
        .map(|c| ResolvedType::of_column(&c))
}

/// Collect a plan subtree's FROM relations as `(effective name, relation)` for
/// alias-aware parameter typing. The effective name is the FROM-clause alias if
/// present, else the table name; the relation is the real table name, or a virtual
/// scan's [`CatalogTable::qualified_name`] (a `from_name`-resolvable name, so a
/// later virtual-column lookup round-trips). Does **not** descend into subqueries (a
/// different FROM scope) — [`Inference::walk_scoped`] recomputes it per scope.
fn collect_from_aliases(plan: &Plan, out: &mut Vec<(String, String)>) {
    match plan {
        Plan::Scan { table, alias } => {
            out.push((super::scan_effective_name(table, alias), table.clone()));
        }
        #[cfg(feature = "index_acceleration")]
        Plan::IndexScan { table, alias, .. }
        | Plan::IndexRangeScan { table, alias, .. }
        | Plan::GinScan { table, alias, .. } => {
            out.push((super::scan_effective_name(table, alias), table.clone()));
        }
        Plan::VirtualScan {
            catalog_table,
            alias,
        } => {
            let name = catalog_table.qualified_name();
            out.push((super::scan_effective_name(name, alias), name.to_string()));
        }
        Plan::Join { left, right, .. } => {
            collect_from_aliases(left, out);
            collect_from_aliases(right, out);
        }
        _ => {
            if let Some(input) = passthrough_input(plan) {
                collect_from_aliases(input, out);
            }
        }
    }
}

/// The single input of a **pass-through** plan node — one that neither adds nor
/// renames the FROM relations beneath it (projection, filter, sort, limit, distinct,
/// window, aggregate). `None` for leaves, joins, scope-openers, and DDL.
///
/// The two FROM-scope walkers ([`collect_from_aliases`] and [`plan_table`]) both
/// descend through these wrappers; routing both through this one helper keeps their
/// transparent-node sets identical, so a newly added wrapper can't be handled by one
/// and silently dropped by the other (a `Plan::Distinct` missing here once left a
/// `SELECT DISTINCT`'s FROM-alias map empty → an untyped join-ON array param).
fn passthrough_input(plan: &Plan) -> Option<&Plan> {
    match plan {
        Plan::Filter { input, .. }
        | Plan::Project { input, .. }
        | Plan::Sort { input, .. }
        | Plan::Limit { input, .. }
        | Plan::Distinct { input, .. }
        | Plan::DistinctOn { input, .. }
        | Plan::JunkFilter { input, .. }
        | Plan::Window { input, .. }
        | Plan::Aggregate { input, .. } => Some(input),
        _ => None,
    }
}

/// The table whose columns an unqualified reference in this node resolves
/// against, threaded through pass-through nodes. `None` for joins (columns are
/// qualified) and for nodes with no single table.
///
/// The index-scan nodes carry their FROM table too: the planner lifts an indexed
/// `col OP $N` conjunct *into* the scan node but leaves any **residual** conjuncts
/// (`active = $1` in `WHERE active = $1 AND id > $2`) in a `Filter` *above* it. Those
/// residual columns must still resolve to a table for parameter typing, so an index
/// scan is a pass-through to its `table` (matching the plain `Scan` arm) — without
/// it, the residual filter's params are left untyped.
fn plan_table(plan: &Plan) -> Option<&str> {
    match plan {
        Plan::Scan { table, .. } => Some(table),
        #[cfg(feature = "index_acceleration")]
        Plan::IndexScan { table, .. }
        | Plan::IndexRangeScan { table, .. }
        | Plan::GinScan { table, .. } => Some(table),
        // A virtual catalog scan's qualified name (`information_schema.tables`) is a
        // `from_name`-resolvable relation, so a bare `WHERE table_name = $1` over it
        // types via `column_resolved_in_relation`.
        Plan::VirtualScan { catalog_table, .. } => Some(catalog_table.qualified_name()),
        _ => passthrough_input(plan).and_then(plan_table),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    /// `users(id INT PRIMARY KEY, name TEXT, active BOOL)` — the PK gives `id` an
    /// index, so a `WHERE` conjunct on `id` lifts into an index scan and a sibling
    /// conjunct on `active` stays in the residual `Filter` above it.
    fn indexed_users() -> Schema {
        let mut schema = Schema::new();
        let sql = "CREATE TABLE users (id INT PRIMARY KEY, name TEXT, active BOOL)";
        let plan = parse(sql, &schema).unwrap().remove(0);
        crate::executor::execute(&plan, &mut schema).unwrap();
        schema
    }

    fn infer(schema: &Schema, sql: &str) -> Vec<Option<TypeOid>> {
        let plan = parse(sql, schema).unwrap().remove(0);
        infer_param_types(&plan, schema, &[None, None]).unwrap()
    }

    #[test]
    fn lower_param_arg_types_as_text() {
        // `LOWER(name) = LOWER($1)` (the spine's `query_account_by_username`): the
        // param inside LOWER must type as text. Left untyped it reports OID 0, which
        // trips a binary-protocol client's (tokio-postgres) type introspection.
        let schema = indexed_users();
        assert_eq!(
            infer(
                &schema,
                "SELECT id FROM users WHERE LOWER(name) = LOWER($1)"
            )[0],
            Some(TypeOid::Text)
        );
    }

    #[test]
    fn cell_update_partial_reports_all_nine_params() {
        // A 9-param variant of fuz_app's `query_cell_update` (`db/cell_queries.ts`,
        // which binds 10): several params inside CASE exprs in SET, plus a cast to
        // the `cell_visibility` enum. Empty hints mirror
        // tokio-postgres's untyped Parse — the ParameterDescription count must still
        // cover all 9 `$N`, or a strict client rejects the bind (`Parameters` error).
        let mut schema = Schema::new();
        for sql in [
            "CREATE TYPE cell_visibility AS ENUM ('private', 'public')",
            "CREATE TABLE cell (id uuid PRIMARY KEY, data jsonb, refs text[], path text, \
             visibility cell_visibility NOT NULL DEFAULT 'private', updated_by uuid, \
             updated_at timestamptz, deleted_at timestamptz)",
        ] {
            let plan = parse(sql, &schema).unwrap().remove(0);
            crate::executor::execute(&plan, &mut schema).unwrap();
        }
        let sql = "UPDATE cell SET \
             data = CASE WHEN $2::bool THEN $3::text::jsonb ELSE data END, \
             refs = CASE WHEN $2::bool THEN $4::text[] ELSE refs END, \
             path = CASE WHEN $5::bool THEN $6 ELSE path END, \
             updated_by = $7, \
             visibility = CASE WHEN $8::bool THEN $9::text::cell_visibility ELSE visibility END, \
             updated_at = NOW() \
             WHERE id = $1 AND deleted_at IS NULL RETURNING id, path";
        let plan = parse(sql, &schema).unwrap().remove(0);
        let types = infer_param_types(&plan, &schema, &[]).unwrap();
        assert_eq!(types.len(), 9, "all 9 params must appear; got {types:?}");
        // Every param must be typed — a `None` reports OID 0, which trips a
        // binary-protocol client's (tokio-postgres) type introspection.
        //
        // **Neither $6 nor $7 is typed by the target-column probe**:
        // the lowering wraps `updated_by = $7` in `Cast { Parameter(7),
        // Uuid }` and branch unification wraps `$6` in `Cast { Parameter(6), Text }`
        // beside its typed `ELSE path` sibling, and `walk_expr`'s `Cast` arm reads both
        // at the higher `PRIO_CAST`. Measured by mutation: deleting
        // `record_value_param`'s call from `walk_assignments` leaves this test green.
        // The shape that does need the probe is an **all**-unknown branch list, which
        // `an_all_unknown_case_is_typed_from_the_update_target_column` covers.
        assert_eq!(
            types,
            vec![
                Some(TypeOid::Uuid),      // $1 id
                Some(TypeOid::Bool),      // $2 data_provided
                Some(TypeOid::Text),      // $3 data_text (inner ::text)
                Some(TypeOid::TextArray), // $4 refs
                Some(TypeOid::Bool),      // $5 path_provided
                Some(TypeOid::Text),      // $6 path_value (CASE unification cast to text)
                Some(TypeOid::Uuid),      // $7 updated_by (assignment coercion cast)
                Some(TypeOid::Bool),      // $8 visibility_provided
                Some(TypeOid::Text),      // $9 visibility (inner ::text)
            ],
            "every param typed; got {types:?}",
        );
    }

    #[test]
    fn an_all_unknown_case_is_typed_from_the_update_target_column() {
        // The `UPDATE` half of the guard the `ON CONFLICT` half already had, and the
        // shape that is actually `record_value_param`'s: every branch is a bare `$N`, so
        // branch unification has nothing to unify against and the lowered plan carries
        // `$2`/`$3` bare inside the assignment's coercion cast — the one place
        // `walk_expr` cannot reach them. Deleting `record_value_param`'s call from
        // `walk_assignments` answers `[Some(Bool), None, None]` here, which is OID 0 on
        // the wire; `cell_update_partial_reports_all_nine_params` stays green under that
        // same mutation, which is why this test exists rather than being redundant.
        //
        // Measured on both engines, `{16,25,25}`: the assignment target `v` is text and
        // 18.6 folds the unknown branches to it as well.
        let schema = on_conflict_probe();
        assert_eq!(
            infer_unhinted(
                &schema,
                "UPDATE oc_probe SET v = CASE WHEN $1::bool THEN $2 ELSE $3 END WHERE id = 1"
            ),
            vec![
                Some(TypeOid::Bool),
                Some(TypeOid::Text),
                Some(TypeOid::Text)
            ]
        );
        // The `COALESCE` spelling reaches the same probe arm and is the one place the two
        // engines part: `n` is bigint, so pglet answers `{20,20}` where 18.6 folds the
        // unknowns to text and refuses the whole statement `42804 column "n" is of type
        // bigint but expression is of type text` (measured). Recorded in `docs/compatibility.md`'s
        // parameter-inference row; pinned here because it is this probe's doing.
        assert_eq!(
            infer_unhinted(
                &schema,
                "UPDATE oc_probe SET n = COALESCE($1, $2) WHERE id = 1"
            ),
            vec![Some(TypeOid::Int8), Some(TypeOid::Int8)]
        );
    }

    #[test]
    fn param_in_residual_filter_above_index_range_scan_is_typed() {
        // `id > $2` lifts into an `IndexRangeScan`; `active = $1` stays in the residual
        // `Filter` above it. BOTH must type — the regression: `$1` was left `None`
        // because `plan_table` didn't see through the index scan to the table.
        let schema = indexed_users();
        assert_eq!(
            infer(
                &schema,
                "SELECT id FROM users WHERE active = $1 AND id > $2"
            ),
            vec![Some(TypeOid::Bool), Some(TypeOid::Int4)]
        );
    }

    /// The refusal's SQLSTATE and the parameter it names, or `None` where the statement
    /// is accepted.
    fn refusal(schema: &Schema, sql: &str, hint: &[Option<TypeOid>]) -> Option<(String, usize)> {
        let plan = parse(sql, schema).unwrap().remove(0);
        match infer_param_types(&plan, schema, hint) {
            Ok(_) => None,
            Err(e @ Error::IndeterminateParameter { index, .. }) => {
                Some((e.sqlstate().to_string(), index))
            }
            Err(e) => panic!("{sql}: {e}"),
        }
    }

    #[test]
    fn a_parameter_left_untyped_is_refused_by_position_not_by_what_inference_typed() {
        // The differential verdicts are `tests/fixtures/params/undetermined_param_*`;
        // these are statements beside them that a fixture cannot hold, because this
        // engine reports no OID for a parameter it does accept there.
        let schema = indexed_users();
        for sql in [
            // No type crosses an arithmetic operator here, so `$1` is `None` — and the
            // statement is 18.6's to answer: the operator typed the parameter.
            "SELECT id FROM users WHERE id = $1 + 1 OR $1 IS NULL",
            "SELECT count(DISTINCT $1) FROM users",
        ] {
            assert_eq!(refusal(&schema, sql, &[]), None, "{sql}");
            assert_eq!(infer_unhinted(&schema, sql), vec![None], "{sql}");
        }
        assert_eq!(
            refusal(&schema, "SELECT id FROM users WHERE $1 IS NULL", &[]),
            Some(("42P18".to_string(), 1))
        );
    }

    #[test]
    fn a_hinted_parameter_is_never_left_untyped() {
        // A plan built without the client's declared types still honours them: the
        // hint is the parameter's type from its first reference.
        let schema = indexed_users();
        let sql = "SELECT id FROM users WHERE $1 IS NULL AND $2 IS NULL";
        assert_eq!(
            refusal(&schema, sql, &[Some(TypeOid::Text), Some(TypeOid::Int4)]),
            None
        );
        assert_eq!(
            refusal(&schema, sql, &[Some(TypeOid::Text), None]),
            Some(("42P18".to_string(), 2))
        );
        // …and a hinted one is not "typed too late" either.
        let sql = "SELECT id FROM users WHERE $1 IS NULL OR name = $1";
        assert_eq!(refusal(&schema, sql, &[Some(TypeOid::Text)]), None);
        assert_eq!(
            refusal(&schema, sql, &[None]),
            Some(("42P08".to_string(), 1))
        );
    }

    #[test]
    fn a_bare_select_list_parameter_is_text_unless_something_types_it() {
        let schema = indexed_users();
        assert_eq!(
            infer_unhinted(&schema, "SELECT $1 AS p FROM users"),
            vec![Some(TypeOid::Text)]
        );
        // Every other fact outranks the entry's `text`, whichever this walk reaches
        // first: 18.6 analyses `FROM` ahead of the select list, so the comparison has
        // made `$1` an `integer` before the entry is read, and this walk reads the
        // entry first. Each reports `{integer}` on 18.6 too.
        for sql in [
            "SELECT $1 AS p FROM (SELECT id FROM users WHERE id = $1) s",
            "SELECT $1::int4 AS q, $1 AS p",
        ] {
            assert_eq!(
                infer_unhinted(&schema, sql),
                vec![Some(TypeOid::Int4)],
                "{sql}"
            );
        }
    }

    /// `users` beside `kv(k TEXT PRIMARY KEY, v JSONB, n INT)`, an `INSERT … SELECT`
    /// target whose columns are three different types.
    fn users_and_kv() -> Schema {
        let mut schema = indexed_users();
        let sql = "CREATE TABLE kv (k TEXT PRIMARY KEY, v JSONB, n INT)";
        let plan = parse(sql, &schema).unwrap().remove(0);
        crate::executor::execute(&plan, &mut schema).unwrap();
        schema
    }

    #[test]
    fn an_insert_select_entry_takes_its_target_columns_type() {
        // 18.6 describes each of these as written here (`Parse` + `Describe`, no
        // declared types): the entry is coerced to the column it is assigned to, in the
        // column list's order, through the wrappers that resolve nothing.
        let schema = users_and_kv();
        let (text, jsonb, int4) = (
            Some(TypeOid::Text),
            Some(TypeOid::Jsonb),
            Some(TypeOid::Int4),
        );
        for (sql, expected) in [
            (
                "INSERT INTO kv (k, v, n) SELECT $1, $2, $3",
                vec![text, jsonb, int4],
            ),
            ("INSERT INTO kv SELECT $1, $2, $3", vec![text, jsonb, int4]),
            ("INSERT INTO kv (n, k) SELECT $1, $2", vec![int4, text]),
            (
                "INSERT INTO kv (k, n) SELECT $1 AS a, $2 AS b",
                vec![text, int4],
            ),
            (
                "INSERT INTO kv (k, n) SELECT name, $1 FROM users WHERE id = $2",
                vec![int4, int4],
            ),
            (
                "INSERT INTO kv (k, n) SELECT $1, $2 FROM users ORDER BY id LIMIT 1",
                vec![text, int4],
            ),
            (
                "INSERT INTO kv (k, n) SELECT name, $1 FROM users GROUP BY name",
                vec![int4],
            ),
            // The target types the entry ahead of `RETURNING` and of `ON CONFLICT`, so
            // a null test there finds the parameter typed.
            (
                "INSERT INTO kv (k, n) SELECT $1, $2 RETURNING $2 IS NULL",
                vec![text, int4],
            ),
            (
                "INSERT INTO kv (k, n) SELECT $1, $2 ON CONFLICT (k) DO UPDATE SET n = \
                 excluded.n WHERE $2 IS NOT NULL",
                vec![text, int4],
            ),
        ] {
            assert_eq!(infer_unhinted(&schema, sql), expected, "{sql}");
        }
    }

    #[test]
    fn an_insert_select_entry_the_source_resolves_stays_text() {
        // A sort, grouping or `DISTINCT` key naming the entry resolves it to `text`
        // inside the source, before the target column is consulted — 18.6 refuses each
        // of these `42804` (`text` into `integer`), where the unclaimed entry is typed
        // by its column.
        let schema = users_and_kv();
        for sql in [
            "INSERT INTO kv (k, n) SELECT $1, $2 ORDER BY 2",
            "INSERT INTO kv (k, n) SELECT DISTINCT $1, $2",
            "INSERT INTO kv (k, n) SELECT $1, $2 FROM users GROUP BY 1, 2",
        ] {
            assert_eq!(
                infer_unhinted(&schema, sql),
                vec![Some(TypeOid::Text), Some(TypeOid::Text)],
                "{sql}"
            );
        }
        // A type an occurrence inside the source gave the parameter is the earlier one
        // and stays.
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO kv (k, n) SELECT $1, $2 FROM users WHERE name = $2"
            ),
            vec![Some(TypeOid::Text), Some(TypeOid::Text)]
        );
        // The entry's fix is timed at the end of the source: a null test inside the
        // source is ahead of it (`42P08` on 18.6).
        assert_eq!(
            refusal(
                &schema,
                "INSERT INTO kv (k, n) SELECT $1, $2 FROM users WHERE $2 IS NOT NULL",
                &[]
            ),
            Some(("42P08".to_string(), 2))
        );
    }

    #[test]
    fn a_missing_relation_is_not_pre_empted_by_the_refusal() {
        // 18.6 answers `42P01` for each: the range table is resolved before any
        // parameter is looked at. Here that error is the executor's, raised whatever
        // the data for a relation in a `FROM` clause or a DML target, so inference
        // refuses nothing and the statement goes on to it.
        let schema = users_and_kv();
        for sql in [
            "SELECT id FROM nope WHERE $1 IS NULL",
            "SELECT id FROM nope WHERE ($1 IS NULL OR id = $1)",
            "UPDATE nope SET n = 1 WHERE $1 IS NULL",
            "DELETE FROM nope WHERE $1 IS NULL",
            "INSERT INTO nope (a) VALUES (1) RETURNING $1 IS NULL",
            "INSERT INTO kv (k) SELECT name FROM nope WHERE $1 IS NULL",
            "SELECT u.id FROM users u JOIN nope n ON n.id = u.id WHERE $1 IS NULL",
            "SELECT s.id FROM (SELECT id FROM nope) s WHERE $1 IS NULL",
            "UPDATE kv SET n = 1 FROM nope WHERE $1 IS NULL",
            "WITH x AS (DELETE FROM nope RETURNING 1) SELECT $1 IS NULL",
            "SELECT $1 IS NULL FROM users UNION ALL SELECT true FROM nope",
        ] {
            assert_eq!(refusal(&schema, sql, &[]), None, "{sql}");
        }
        // The controls: the same shapes over relations the catalog holds are refused.
        for sql in [
            "SELECT id FROM users WHERE $1 IS NULL",
            "UPDATE kv SET n = 1 WHERE $1 IS NULL",
            "DELETE FROM kv WHERE $1 IS NULL",
            "INSERT INTO kv (k) VALUES ('a') RETURNING $1 IS NULL",
            "SELECT id FROM users WHERE $1 IS NULL AND id IN (SELECT id FROM users)",
            // An expression's subquery is run only for a row that evaluates it, so a
            // missing relation there is not certain to be reported and the refusal
            // stands (18.6: `42P01`).
            "SELECT id FROM users WHERE $1 IS NULL AND id IN (SELECT id FROM nope)",
            "SELECT id FROM users WHERE $1 IS NULL AND EXISTS (SELECT 1 FROM (SELECT id FROM nope) s)",
            "SELECT (SELECT id FROM nope), $1 IS NULL FROM users",
            "UPDATE kv SET n = (SELECT id FROM nope) WHERE $1 IS NULL",
            "SELECT 1 FROM information_schema.tables WHERE $1 IS NULL",
            "WITH x AS (SELECT 1 AS a) SELECT a FROM x WHERE $1 IS NULL",
        ] {
            assert_eq!(
                refusal(&schema, sql, &[]),
                Some(("42P18".to_string(), 1)),
                "{sql}"
            );
        }
    }

    #[test]
    fn a_sequence_or_an_index_is_not_a_relation_the_catalog_lacks() {
        // The stand-down is for a name whose `42P01` / `42809` 18.6 raises ahead of
        // the parameter. A sequence is a relation 18.6 reads, so the parameter is what
        // it refuses (`42P18`); an index's name and an enum type's name are refused as
        // the range table is resolved (`42809`, `42P01`), like a name nothing carries.
        let mut schema = users_and_kv();
        for sql in [
            "CREATE SEQUENCE sq",
            "CREATE INDEX kv_n_idx ON kv (n)",
            "CREATE TYPE mood AS ENUM ('a', 'b')",
        ] {
            let plan = parse(sql, &schema).unwrap().remove(0);
            crate::executor::execute(&plan, &mut schema).unwrap();
        }
        for sql in [
            "SELECT 1 FROM sq WHERE $1 IS NULL",
            "DELETE FROM sq WHERE $1 IS NULL",
            "SELECT 1 FROM users JOIN sq ON true WHERE $1 IS NULL",
        ] {
            assert_eq!(
                refusal(&schema, sql, &[]),
                Some(("42P18".to_string(), 1)),
                "{sql}"
            );
        }
        for sql in [
            "SELECT 1 FROM kv_n_idx WHERE $1 IS NULL",
            "DELETE FROM kv_n_idx WHERE $1 IS NULL",
            "SELECT 1 FROM mood WHERE $1 IS NULL",
        ] {
            assert_eq!(refusal(&schema, sql, &[]), None, "{sql}");
        }
    }

    #[test]
    fn an_insert_row_is_timed_in_its_written_order() {
        const NULL_TEST: &str = "CASE WHEN $1 IS NULL THEN 0 ELSE 1 END";
        // `t2`'s columns are `(n, k, v)` and `kv`'s `(k, v, n)`: each statement is given
        // over a column list in the table's order and over one against it, and the
        // verdict follows the row as written both times. 18.6 answers the first group
        // and refuses the second `42P08`.
        let mut schema = users_and_kv();
        let sql = "CREATE TABLE t2 (n INT, k TEXT, v JSONB)";
        let plan = parse(sql, &schema).unwrap().remove(0);
        crate::executor::execute(&plan, &mut schema).unwrap();
        for table in ["kv", "t2"] {
            for row in [
                format!("(k, n) VALUES ($1::text, {NULL_TEST})"),
                format!("(k, n) VALUES ($1 || 'x', {NULL_TEST})"),
                format!("(v, k, n) VALUES ('{{}}', upper($1), {NULL_TEST})"),
                format!("(k, n) VALUES ($1::text, {NULL_TEST}), ('q', 2)"),
                format!("(k, n) VALUES ('q', 2), ($1 || 'x', {NULL_TEST})"),
            ] {
                let sql = format!("INSERT INTO {table} {row}");
                assert_eq!(refusal(&schema, &sql, &[]), None, "{sql}");
            }
            for row in [
                format!("(n, k) VALUES ({NULL_TEST}, $1::text)"),
                format!("(n, k) VALUES ({NULL_TEST}, $1 || 'x')"),
                format!("(n, v, k) VALUES ({NULL_TEST}, '{{}}', upper($1))"),
                format!("(n, k) VALUES (2, 'q'), ({NULL_TEST}, $1::text)"),
                format!("(k, n) VALUES ('q', {NULL_TEST}), ($1 || 'x', 2)"),
            ] {
                let sql = format!("INSERT INTO {table} {row}");
                assert_eq!(
                    refusal(&schema, &sql, &[]),
                    Some(("42P08".to_string(), 1)),
                    "{sql}"
                );
            }
        }
        // With no column list the written order is the table's.
        let sql = format!("INSERT INTO t2 VALUES ({NULL_TEST}, $1 || 'x')");
        assert_eq!(refusal(&schema, &sql, &[]), Some(("42P08".to_string(), 1)));
        let sql = "INSERT INTO kv VALUES ($1 || 'x', jsonb_build_object('a', $1))";
        assert_eq!(refusal(&schema, sql, &[]), None);
    }

    #[test]
    fn param_operand_of_concat_is_text() {
        // `$1 || '%'` (the spine's `LIKE $1 || '%'` prefix search): a parameter
        // operand of string concatenation types as text. Left untyped it reports
        // OID 0, which loops a binary-protocol client's type introspection.
        let schema = indexed_users();
        assert_eq!(
            infer(&schema, "SELECT id FROM users WHERE name LIKE $1 || '%'")[0],
            Some(TypeOid::Text)
        );
    }

    #[test]
    fn param_in_residual_filter_above_index_scan_is_typed() {
        // Same shape, but `id = $2` lifts into an equality `IndexScan`; the residual
        // `active = $1` above it must still type.
        let schema = indexed_users();
        assert_eq!(
            infer(
                &schema,
                "SELECT id FROM users WHERE active = $1 AND id = $2"
            ),
            vec![Some(TypeOid::Bool), Some(TypeOid::Int4)]
        );
    }

    #[test]
    fn virtual_table_column_filter_param_is_typed() {
        // The spine's `auth_tables_exist` probe: `WHERE table_name = $1` over
        // `information_schema.tables`. The column lives on a synthesized catalog view,
        // not `Schema.tables`, so it was left untyped (OID 0) — which a binary-protocol
        // client (tokio-postgres) rejects in `ParameterDescription`. It must type as
        // text via the virtual table's column list.
        let schema = Schema::new();
        assert_eq!(
            infer(
                &schema,
                "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
                 WHERE table_schema = 'public' AND table_name = $1)"
            )[0],
            Some(TypeOid::Text)
        );
    }

    #[test]
    fn virtual_table_aliased_column_filter_param_is_typed() {
        // The same, but the virtual scan is aliased and the column qualified
        // (`t.table_name = $1`) — resolved through the FROM-scope map to the catalog
        // view's qualified name.
        let schema = Schema::new();
        assert_eq!(
            infer(
                &schema,
                "SELECT 1 FROM information_schema.columns t WHERE t.table_name = $1"
            )[0],
            Some(TypeOid::Text)
        );
    }

    /// `dt(id INT, b BIGINT)` the `UPDATE` target, `ds(id INT, b INT)` the `FROM`
    /// source — same column *names*, different widths, so a reference resolved against
    /// the wrong relation reports a different OID rather than the same one.
    fn returning_from_pair() -> Schema {
        let mut schema = Schema::new();
        for sql in [
            "CREATE TABLE dt (id INT, b BIGINT)",
            "CREATE TABLE ds (id INT, b INT, k INT)",
            // A third relation for the subquery-scope test: it carries `k` at a
            // **different** type from `ds.k`, which is the whole discriminator there.
            "CREATE TABLE dz (k TEXT)",
        ] {
            let plan = parse(sql, &schema).unwrap().remove(0);
            crate::executor::execute(&plan, &mut schema).unwrap();
        }
        schema
    }

    #[test]
    fn returning_param_types_against_the_from_source_not_the_target() {
        // This walk is the `RETURNING` clause's fifth reader and it was left on
        // `TypeScope::Catalog(Some(<target>))`, which looks the qualifier up in the
        // **catalog** and falls back to the target — so an alias, which names no
        // catalog table, silently took the target's same-named column. Measured against
        // 18.6: `RETURNING (s.b = $1)` reported `int8` (the target's `b`) where 18.6
        // says `int4`.
        //
        // A fixture *can* cover this, contrary to how it was first written up:
        // `update/returning_from_source_param_type.sql` binds an `int4` against this
        // very statement, and tokio-postgres refuses a value whose Rust type does not
        // accept the reported OID. What a fixture cannot see is a wrong OID the bound
        // value still accepts — which is what keeps this unit test beside it.
        //
        // The order of the clause scope against the catalog attempt is load-bearing, not
        // incidental: appending it as a *fallback* leaves this assertion red, because the
        // catalog attempt answers first with the target's column.
        let schema = returning_from_pair();
        assert_eq!(
            infer(
                &schema,
                "UPDATE dt SET id = dt.id FROM ds s WHERE s.id = dt.id RETURNING (s.b = $1)"
            )[0],
            Some(TypeOid::Int4),
            "an aliased source's column types the param, not the target's"
        );
    }

    #[test]
    fn returning_param_types_a_source_only_column_rather_than_leaving_it_untyped() {
        // The second shape of the same defect, split into its own test because a
        // panicking assertion pre-empts every later one and these two can regress
        // independently: a source column the target does not carry at all had no
        // fallback to reach, so it reported **no** type. OID 0 is what a
        // binary-protocol client's introspection trips over (see the module doc), and
        // 18.6 reports `int4`.
        let schema = returning_from_pair();
        assert_eq!(
            infer(
                &schema,
                "UPDATE dt SET id = dt.id FROM ds s WHERE s.id = dt.id RETURNING (s.k = $1)"
            )[0],
            Some(TypeOid::Int4),
            "a source-only column must type the param rather than leaving it OID 0"
        );
    }

    #[test]
    fn returning_param_types_keep_the_target_and_the_unaliased_source() {
        // Both polarities of the control, and they are what validate the fix rather
        // than merely exercising it: each was **already** correct against 18.6 — the
        // target-qualified spelling because the fallback happened to be right, the
        // unaliased source because its qualifier *is* a catalog table — so a repair
        // that widened too far shows up here and nowhere else.
        let schema = returning_from_pair();
        assert_eq!(
            infer(
                &schema,
                "UPDATE dt SET id = dt.id FROM ds s WHERE s.id = dt.id RETURNING (dt.b = $1)"
            )[0],
            Some(TypeOid::Int8)
        );
        assert_eq!(
            infer(
                &schema,
                "UPDATE dt SET id = dt.id FROM ds WHERE ds.id = dt.id RETURNING (ds.b = $1)"
            )[0],
            Some(TypeOid::Int4)
        );
        // A target alias is the same rule from the other side: `o` names no catalog
        // table either, so the clause's own namespace is the only thing that can bind
        // it.
        assert_eq!(
            infer(
                &schema,
                "UPDATE dt o SET id = o.id FROM ds s WHERE s.id = o.id RETURNING (o.b = $1)"
            )[0],
            Some(TypeOid::Int8)
        );
    }

    #[test]
    fn a_subquery_inside_returning_does_not_inherit_the_clause_scope() {
        // The clause's namespace must not *override* a subquery's own relations. The
        // discriminator is the **type**, not the presence of an answer: `TypeScope::Input`
        // falls back to a flat by-name lookup for a qualifier its relations do not name,
        // so an uncleared scope still answers — with `ds.k`'s `int4`. Written over `ds x`
        // (which is what the first draft did) the two lookups return the same OID and the
        // test is vacuous under its own mutation, measured. `dz.k` is `text`, so the two
        // disagree.
        //
        // What this does **not** assert: that the enclosing level stops being reachable.
        // `x` is the subquery's own alias, so a chained scope searching the inner level
        // first still answers `text` — the shape Postgres's innermost-wins rule takes, and
        // the shape closing the `walk_scoped` gap would need. This guard does not stand in
        // its way.
        let schema = returning_from_pair();
        assert_eq!(
            infer(
                &schema,
                "UPDATE dt SET id = dt.id FROM ds s WHERE s.id = dt.id \
                 RETURNING (SELECT count(*) FROM dz x WHERE x.k = $1)"
            )[0],
            Some(TypeOid::Text)
        );
    }

    /// `oc_probe(id INT PRIMARY KEY, v TEXT, n BIGINT)` — the `ON CONFLICT` probe
    /// table. The three column types are deliberately distinct, so a `$N` typed from
    /// the wrong position reports a different OID rather than the same one, and the
    /// `PRIMARY KEY` is what an `ON CONFLICT (id)` arbiter matches.
    fn on_conflict_probe() -> Schema {
        let mut schema = Schema::new();
        for sql in [
            "CREATE TABLE oc_probe (id INT PRIMARY KEY, v TEXT, n BIGINT)",
            "CREATE TABLE oc_src (id INT, v TEXT)",
        ] {
            let plan = parse(sql, &schema).unwrap().remove(0);
            crate::executor::execute(&plan, &mut schema).unwrap();
        }
        schema
    }

    /// [`on_conflict_probe`] plus a **real table named `excluded`**, whose columns carry
    /// the target's names under *different* types — the only shape that can tell the
    /// pseudo-relation's columns from a catalog lookup's, since the two agree everywhere
    /// else by construction.
    fn on_conflict_probe_with_shadow_excluded() -> Schema {
        let mut schema = on_conflict_probe();
        let sql = "CREATE TABLE excluded (id INT, n TEXT, v TEXT)";
        let plan = parse(sql, &schema).unwrap().remove(0);
        crate::executor::execute(&plan, &mut schema).unwrap();
        schema
    }

    /// Infer with **no** client hints — tokio-postgres's untyped `Parse`, so the
    /// reported count is entirely the walk's own.
    fn infer_unhinted(schema: &Schema, sql: &str) -> Vec<Option<TypeOid>> {
        let plan = parse(sql, schema).unwrap().remove(0);
        infer_param_types(&plan, schema, &[]).unwrap()
    }

    #[test]
    fn on_conflict_do_update_reports_every_param() {
        // The headline: a `$N` in the `ON CONFLICT` action used to be **uncounted** —
        // the `Plan::Insert` arm's `..` swallowed the clause — so this statement
        // reported `ParameterDescription` n = 2 and `Bind` answered
        // `08P01 bind message supplies 3 parameters, but prepared statement "" requires
        // 2`, which made a bound upsert unusable from any extended-protocol driver. The
        // control that identifies the missing arm rather than a typing gap is in the same
        // statement: `$1`/`$2` sit in the `VALUES` position and were counted throughout.
        // 18.6 reports `{integer,text,text}`.
        let schema = on_conflict_probe();
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v) VALUES ($1, $2) \
                 ON CONFLICT (id) DO UPDATE SET v = $3"
            ),
            vec![
                Some(TypeOid::Int4),
                Some(TypeOid::Text),
                Some(TypeOid::Text)
            ]
        );
    }

    #[test]
    fn on_conflict_action_where_types_an_excluded_qualified_param() {
        // `excluded` is not a catalog relation, and the action does not look for it in
        // the catalog: [`Inference::walk_on_conflict`] sets `clause_scope` to
        // `on_conflict_action_relations`, so both qualifiers here are resolved by
        // `TypeScope::Input` against that namespace — the target under its own name,
        // then the pseudo-relation carrying the target's shape. 18.6 reports `{bigint}`
        // for both spellings; the target-qualified one is the control, since it names
        // the namespace's *other* relation. What this test cannot tell apart is the
        // namespace from the catalog lookup it replaced — over `on_conflict_probe`'s
        // schema the two agree — which is what
        // `a_real_table_named_excluded_does_not_shadow_the_pseudo_relation` and its
        // shadow-table schema are for.
        let schema = on_conflict_probe();
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v) VALUES (1, 'a') ON CONFLICT (id) \
                 DO UPDATE SET v = excluded.v WHERE excluded.n = $1"
            ),
            vec![Some(TypeOid::Int8)],
            "an excluded-qualified reference types the param from the target's column"
        );
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v) VALUES (1, 'a') ON CONFLICT (id) \
                 DO UPDATE SET v = excluded.v WHERE oc_probe.n = $1"
            ),
            vec![Some(TypeOid::Int8)]
        );
    }

    #[test]
    fn on_conflict_assignment_case_branch_types_from_the_target_column() {
        // The `Plan::Update` arm's `record_value_param` rule, re-applied to the action's
        // `SET` list: a `$N` buried in a `CASE` result branch has no sibling to type it,
        // and left untyped it reports OID 0, which a binary-protocol client's
        // introspection trips over.
        let schema = on_conflict_probe();
        // **Two shapes, because only the second one is the rule's.** With a typed branch
        // beside it the lowering's own branch unification (`resolve_expr_types_gated`)
        // already coerces `$2` to text and `walk_expr` reads that cast — measured: the
        // assertion below holds with `record_value_param` deleted, so on its own it is a
        // guard for a line it does not reach.
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v) VALUES (1, 'a') ON CONFLICT (id) \
                 DO UPDATE SET v = CASE WHEN $1::bool THEN $2 ELSE oc_probe.v END"
            ),
            vec![Some(TypeOid::Bool), Some(TypeOid::Text)],
            "18.6: {{boolean,text}}"
        );
        // Every branch unknown, so unification has nothing to unify and the assignment
        // target is the only thing left that can type them. Deleting `record_value_param`
        // answers `[Bool, None, None]` here. 18.6: `{boolean,text,text}`.
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v) VALUES (1, 'a') ON CONFLICT (id) \
                 DO UPDATE SET v = CASE WHEN $1::bool THEN $2 ELSE $3 END"
            ),
            vec![
                Some(TypeOid::Bool),
                Some(TypeOid::Text),
                Some(TypeOid::Text)
            ],
            "an all-unknown CASE types its branches from the assignment target"
        );
    }

    #[test]
    fn the_assignment_target_probe_is_written_before_the_expression_walk() {
        // [`Inference::walk_assignments`] says reversing its two probes is a behaviour
        // change and not a reordering, and until this test that sentence was true and
        // unguarded — the swap left every other test here green. What discriminates is a
        // right-hand side carrying **two equal-priority** context facts about one `$N`:
        // `record` keeps the first at equal priority, so whichever probe runs first wins.
        //
        // `$1` sits in a text position (`LOWER($1)`) *and* in a `CASE` result branch,
        // which the target probe types from the column. Today the target probe is first,
        // so `$1` is the column's `bigint`; with the two lines swapped `walk_expr`'s text
        // fact lands first and `$1` reports `text`. `$2` is the control — it has only the
        // target probe, so it is `bigint` either way and separates "the order changed"
        // from "the probe stopped running".
        //
        // **Not a differential fact.** 18.6 refuses this statement outright —
        // `42804 column "n" is of type bigint but expression is of type text` (measured
        // live on 18.6 over this exact table) — because it unifies the parameter's two
        // deductions and pglet has no unification at all (`docs/compatibility.md`'s
        // parameter-inference row). So this pins pglet's internal probe order, and the
        // day unification lands the statement becomes an error here too and this test
        // is replaced rather than updated.
        let schema = on_conflict_probe();
        assert_eq!(
            infer_unhinted(
                &schema,
                "UPDATE oc_probe SET n = CASE WHEN LOWER($1) = 'x' THEN $1 ELSE $2 END"
            ),
            vec![Some(TypeOid::Int8), Some(TypeOid::Int8)],
            "the target-column probe runs first, so it wins at equal priority"
        );
    }

    #[test]
    fn the_on_conflict_walk_sits_between_the_row_source_and_returning() {
        // `record` keeps the **first** fact at equal priority, so the walk order is
        // observable whenever one `$N` is written in two clauses — and this is the guard
        // that reddens if someone reorders it. Each of the three orders asserted below is
        // measured on 18.6 (the first through its mirror spelling), where the collision
        // surfaces as the *later* clause's operator error, naming which clause typed the
        // parameter first.
        //
        // **Both boundaries of "between" are guarded here**, plus one order inside the
        // clause: the row source below (assertion 1, red when the clause is hoisted above
        // the `match rows`), `RETURNING` above (assertion 3, red when the clause is moved
        // below `walk_returning`), and the arbiter ahead of the action's `WHERE`
        // (assertion 2). The upper boundary was the unguarded one: before assertion 3 the
        // whole lib suite stayed green with `walk_on_conflict` moved below
        // `walk_returning` (measured), and now that move reddens here.
        let schema = on_conflict_probe();
        // Row source before the action: `VALUES ($1, 'a')` types int4, and the action's
        // `SET v = $1` does not re-type it to text. On 18.6 the mirror spelling
        // `VALUES (1, $1) … WHERE n > $1` is `42883 bigint > text` — the row source won.
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v) VALUES ($1, 'a') \
                 ON CONFLICT (id) DO UPDATE SET v = $1"
            ),
            vec![Some(TypeOid::Int4)],
            "the row source is walked before the ON CONFLICT clause"
        );
        // Arbiter before the action's `WHERE`: the partial-index predicate types `$1`
        // bigint and the action's `WHERE oc_probe.v = $1` does not re-type it to text.
        // 18.6 answers `42883 operator does not exist: text = bigint`, pointing at the
        // action's `WHERE` — so there too the arbiter went first.
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v) VALUES (1, 'a') ON CONFLICT (id) \
                 WHERE n > $1 DO UPDATE SET v = 'z' WHERE oc_probe.v = $1"
            ),
            vec![Some(TypeOid::Int8)],
            "the arbiter is walked before the action's WHERE"
        );
        // The action before `RETURNING`: the action's `WHERE oc_probe.n = $1` types `$1`
        // bigint and the `RETURNING (v = $1)` does not re-type it to text. 18.6 answers
        // `42883 operator does not exist: text = bigint` with the caret on the
        // `RETURNING` (measured), so there too the action went first.
        //
        // The collision has to sit in the action's `WHERE` rather than its `SET` list.
        // `coerce_to_type` wraps a `SET` right-hand side in an `Expr::Cast`, which records
        // at `PRIO_CAST` and outranks the `RETURNING` context fact whichever clause runs
        // first — so `… DO UPDATE SET n = $1 RETURNING (v = $1)` reports `{int8}` under
        // **both** orders and asserts nothing about this one. Measured both ways; the
        // spelling below reports `{int8}` today and `{text}` with `walk_on_conflict`
        // moved under `walk_returning`.
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v) VALUES (1, 'a') ON CONFLICT (id) \
                 DO UPDATE SET v = 'z' WHERE oc_probe.n = $1 RETURNING (v = $1)"
            ),
            vec![Some(TypeOid::Int8)],
            "the ON CONFLICT clause is walked before RETURNING"
        );
    }

    #[test]
    fn on_conflict_arbiter_positions_type_their_params() {
        // The arbiter carries two parameter positions of its own, and neither is part of
        // the action. A partial-index predicate types by the ordinary comparison-sibling
        // rule (18.6: `{bigint}`); a key **expression** does not, because pglet infers no
        // type across an arithmetic operator — the documented divergence class
        // (`docs/param_binding.md`) — so what this test asserts there is the **count**, and
        // the explicit cast is the spelling that types it (18.6 reports `{integer}` for
        // both spellings).
        let schema = on_conflict_probe();
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v) VALUES (1, 'a') \
                 ON CONFLICT (id) WHERE n > $1 DO NOTHING"
            ),
            vec![Some(TypeOid::Int8)]
        );
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v) VALUES (1, 'a') \
                 ON CONFLICT ((id + $1)) DO NOTHING"
            ),
            vec![None],
            "an arithmetic key expression counts the param without typing it"
        );
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v) VALUES (1, 'a') \
                 ON CONFLICT ((id + $1::int)) DO NOTHING"
            ),
            vec![Some(TypeOid::Int4)]
        );
    }

    #[test]
    fn a_real_table_named_excluded_does_not_shadow_the_pseudo_relation() {
        // The action's `excluded.<col>` used to resolve through `TypeScope::Catalog`'s
        // fallback — a qualifier naming no catalog table falls back to the threaded
        // target — which is the right answer only while no such table exists. With one
        // in the catalog the fallback never runs and the *shadow table* answers:
        // measured, this reported `{text}` (its `n TEXT`) against 18.6's `{bigint}`, and
        // end-to-end the bound statement was a loud
        // `42883 operator does not exist: bigint = text` where 18.6 updates the row — a
        // false rejection out of nothing but an ordinary table name.
        //
        // The scope is `on_conflict_action_relations`, consulted **ahead** of the
        // catalog; reverting that order (or dropping the scope) reddens this and leaves
        // every other test here green, which is why the shadow table is its own schema.
        let schema = on_conflict_probe_with_shadow_excluded();
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v, n) VALUES (1, 'a', 5) ON CONFLICT (id) \
                 DO UPDATE SET v = 'z' WHERE excluded.n = $1"
            ),
            vec![Some(TypeOid::Int8)],
            "excluded.n is the target's bigint, not the shadow table's text"
        );
        // The controls, both `{bigint}` on 18.6 too: the target-qualified spelling, which
        // never needed the fallback, and the `SET` right-hand side, which reaches the
        // same scope by the other position.
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v, n) VALUES (1, 'a', 5) ON CONFLICT (id) \
                 DO UPDATE SET v = 'z' WHERE oc_probe.n = $1"
            ),
            vec![Some(TypeOid::Int8)]
        );
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v, n) VALUES (1, 'a', 5) ON CONFLICT (id) \
                 DO UPDATE SET v = 'z' WHERE excluded.n > $1 AND excluded.n < $2"
            ),
            vec![Some(TypeOid::Int8), Some(TypeOid::Int8)]
        );
    }

    #[test]
    fn an_arbiter_reference_is_not_given_the_actions_namespace() {
        // `excluded` is the **action's** relation and not the clause's: on 18.6 both
        // `ON CONFLICT ((excluded.id)) DO UPDATE …` and
        // `ON CONFLICT (id) WHERE excluded.n > 0 DO UPDATE …` are
        // `42P01 invalid reference to FROM-clause entry for table "excluded"` (measured),
        // so a scope set over the whole clause would type a reference 18.6 refuses to
        // resolve at all. pglet does not gate the arbiter (see `docs/compatibility.md`'s
        // parameter-inference row), so what is observable here is *which* relation
        // answers — and with a real `excluded` in the catalog the arbiter reads it,
        // where the action does not.
        //
        // The action must be a `DO UPDATE`, because that is the spelling which pins
        // **which relation answered** — and the regression this guards has two shapes,
        // which `DO NOTHING` tells apart differently. Hoisting the scope *above* the
        // arbiter walk (restoring it on [`Inference::walk_on_conflict`]'s non-`DoUpdate`
        // early return) walks every arbiter under the action's namespace; moving the
        // scope **and** the arbiter walk inside the `DoUpdate` block does that only for
        // a `DO UPDATE`, and drops a `DO NOTHING` arbiter from the walk entirely.
        // Measured, this statement against its `DO NOTHING` twin: today both answer
        // `{25}`; hoisted, both answer `{20}` (the pseudo-relation's `n`, the target's
        // bigint); inside the block, the `DO UPDATE` answers `{20}` while the
        // `DO NOTHING` reports **no parameter at all** — an empty
        // `ParameterDescription`, which is a count fact and not a namespace one, and
        // which `on_conflict_arbiter_positions_type_their_params` already carries. So a
        // `DO NOTHING` spelling of *this* assertion would redden under both shapes, but
        // under the second it would redden for the other test's reason and stop
        // discriminating the namespace at all.
        let schema = on_conflict_probe_with_shadow_excluded();
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v, n) VALUES (1, 'a', 5) \
                 ON CONFLICT (id) WHERE excluded.n > $1 DO UPDATE SET v = 'z'"
            ),
            vec![Some(TypeOid::Text)],
            "the arbiter reads the catalog, so the shadow table's text answers there"
        );
    }

    #[test]
    fn on_conflict_is_walked_over_an_insert_select_too() {
        // The clause sits outside the `match rows`, so the `InsertRows::Select` source
        // and the action are both walked. 18.6: `{integer,bigint}`.
        let schema = on_conflict_probe();
        assert_eq!(
            infer_unhinted(
                &schema,
                "INSERT INTO oc_probe (id, v) SELECT id, v FROM oc_src WHERE id = $1 \
                 ON CONFLICT (id) DO UPDATE SET n = $2"
            ),
            vec![Some(TypeOid::Int4), Some(TypeOid::Int8)]
        );
    }

    #[test]
    fn a_quoted_excluded_qualifier_never_reaches_inference() {
        // The action's namespace resolves `excluded.<col>` without the catalog, but it is
        // no *name gate*: `TypeScope::Input` falls through a qualifier its relations do
        // not name to the flattened column list (`Plan::find_projected_column`), so the
        // quoted `"EXCLUDED"` — which the fold leaves alone, so it matches neither
        // relation — would still type `n` off that flat list. What bounds it is one layer
        // up: `lower_on_conflict` resolves both halves of the action against the closed
        // `on_conflict_action_scope`, so the statement is `42P01` and there is no plan to
        // infer over. Naming the layer is the point — this property is not the walk's,
        // and a change that opened the action's scope would take it away silently.
        let schema = on_conflict_probe();
        let err = parse(
            "INSERT INTO oc_probe (id, v) VALUES (1, 'a') ON CONFLICT (id) \
             DO UPDATE SET v = 'z' WHERE \"EXCLUDED\".n = $1",
            &schema,
        )
        .expect_err("a quoted EXCLUDED names no relation");
        assert_eq!(err.sqlstate(), "42P01", "got {err:?}");
    }
}
