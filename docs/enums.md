# ENUM types — `CREATE TYPE … AS ENUM`

An enum is a text label plus a catalog member set: `CREATE TYPE … AS ENUM`
registers the members, an enum column stores the label, and the write path, a
cast to the enum and a column `DEFAULT` each refuse a non-member. The examples below use
`CREATE TYPE cell_visibility AS ENUM ('private', 'public')` — the enum the
fixtures under `tests/fixtures/enum/` define; § Test coverage lists the guards.

## Design: enum = text label + catalog member set

PG enums round-trip as their text label, so pglet stores an enum value as
`TypeOid::Text` and carries the enum's identity on the column rather than minting
a data-carrying `TypeOid` variant (which the closed `#[repr(u32)]` enum can't take
cleanly). The catalog owns the type; the write path, and every cast to the enum
(§ A cast to an enum converts), validate membership.

- `Schema.enums: BTreeMap<String, EnumType>` — (ASCII-downcased) name → `{oid, labels}`:
  the catalog OID `pg_type`/`pg_enum` join on, plus the ordered member labels.
  `Column.enum_type: Option<String>` — keys into it. Both
  `#[serde(default)]` (pre-stable on-disk break). `BTreeMap` (like the other
  catalog maps) keeps `serialize_schema`'s postcard bytes canonical.
- `CREATE TYPE … AS ENUM (…)` parses (the native parser, `parser::ddl`) →
  `Plan::CreateType` → catalog insert. Only the `AS ENUM` form is built; the composite
  `CREATE TYPE t AS (a INT, b TEXT)` is `0A000` there.
- `DROP TYPE [IF EXISTS] … [CASCADE | RESTRICT]` → `Plan::DropType` (lets fixtures reset
  idempotently); its dependents are `executor::dependencies`' — see § A cast to an enum
  converts.
- CREATE TABLE / ALTER ADD COLUMN: `resolve_column_type` maps a declared type
  name to `(TypeOid, enum_type)` — an enum name → `(Text, Some(name))`, keyed on the
  decomposed base; any other name goes through the shared `type_oid_from_parts`. A
  builtin's name is the builtin's even beside an enum of that name (`CREATE TYPE int4 AS
  ENUM (…)` leaves `c int4` and `'5'::int4` integers, as 18.6's search path, `pg_catalog`
  first, does) — the enum is asked only for a name `type_oid_from_name` does not resolve, or a `SERIAL` spelling (which 18.6 holds as no type) — which is
  why a builtin 18.6 has and pglet does not (`oid`, `json`, `regclass`) still resolves to a
  same-named enum here (`EDGES.md` row 118).
- Member validation runs inside `validate_column` — the single per-column
  write-path entry point (INSERT, UPDATE, `ON CONFLICT DO UPDATE`, bulk INSERT,
  and COPY all funnel through it; `validate_enum_member` is its helper, and the
  `ADD COLUMN` back-fill's member test too).
  Non-member → `Error::InvalidEnumValue` (SQLSTATE `22P02`, matching PG). NULL
  passes (nullability is separate).
- A column `DEFAULT` converts too, as it is cooked (`executor::ddl`'s
  `cook_column_default`): a bare label into an enum column is tested against the
  member list on `CREATE TABLE`, `SET DEFAULT` and `ADD COLUMN`, whatever the rows, and
  the value `ADD COLUMN` back-fills is tested whatever spelled it — so the one route
  known to store a label the enum lacks is `DROP TYPE` + re-`CREATE TYPE` under a
  column (`EDGES.md` row 261; row 105 records the routes closed).

## A cast to an enum converts

18.6's cast to an enum is `enum_in`, run where the conversion of that operand is run:
a string literal in parse analysis (so over no row, and in a `CASE` branch no row
takes), an undeclared `$N` at `Bind` where the cast is what fixes the parameter's
type, anything else — a column, `'q'::text`, a declared parameter, an undeclared one
an earlier use already typed — on the value. pglet's parser reads the type name once
(`resolve_column_type`, the column form's resolver) and builds every spelling — `::`,
`CAST(… AS …)`, the typed constant `mood 'ok'` — through one builder,
`planner::build_enum_cast`:

- a string literal the enum holds, and `NULL`, keep the plain cast to `text` — an enum
  value is its label — so nothing that reads one sees a conversion;
- every other operand is the conversion `StringFunc::EnumIn` under that same cast, its
  arguments the operand, the enum's name, and the member labels as the parser read
  them. The labels ride in the node so the test is decidable where no catalog is lent —
  a `CHECK` on a write, an index maintained in storage, the DDL probe — which makes them
  a copy of `Schema.enums[name]` that outlives its statement when the node is
  persisted. `DROP TYPE` is the one statement that moves a member list while a
  persisted expression survives it (`DROP SCHEMA` and a rollback move the tables with
  the types), and it treats such an expression as 18.6's dependency: `2BP01` without
  `CASCADE`, and with it the constraint dropped or the default cleared — the one DROP-time
  resolver, `executor::dependencies` — so the list cannot move under a persisted one.
  An index cannot hold the node at all: 18.6 refuses it `42P17`, `enum_in` not being
  immutable, and so does `CREATE INDEX` here, unless the operand is already the enum (a
  relabel, no function on 18.6 either). (A statement prepared before a `DROP TYPE` +
  re-`CREATE TYPE` and bound after it tests the new members on the native server,
  measured — where 18.6 answers `XX000 cache lookup failed for type`.)

Where each is decided is the constant-cast rule's split, not a new one: a literal the
enum lacks by `planner::enum_in_constant_legality`, from the plan-time
`validate_const_casts` (every constant cast's position) and the DDL walk
`check_expr_node_operand_types` (a `DEFAULT`, `CHECK`, index predicate or key, over no
row); an undeclared `$N` by the enum target `ParamTargets::enums` records, at each
host's `Bind` — recorded only where the cast is the parameter's first type-fixing use in
18.6's parse-analysis order (`planner::param_types`' `first_fix`: in `WHERE ($1 = '' OR
m = $1::mood)` the comparison typed `$1` as `text` first, so the cast converts on the
value; a `WITH` query is analysed ahead of the statement's own clauses wherever it is
read); everything else by `executor::expr`'s `eval_enum_in`, on the value — a
string is a membership test, NULL is NULL, and any other value is 18.6's
`42846 cannot cast type <T> to <enum>`. A label is compared as written: `'SAD'::mood`
is not the member `sad`. The node renders as the one cast written — `(s)::mood` —
in both deparse modes, so `pg_get_constraintdef` and `column_default` print 18.6's
strings, and a `CHECK` or `DEFAULT` holding it dumps to a statement both engines load.

What it does not change is the **identity**: the node is a string function under a
cast to `text`, and the walk answers `None` for both, so `ORDER BY s::mood` and
`m::mood > 'ok'` order by the label bytes — § The boundary, and `EDGES.md` row 104. The
residuals are `EDGES.md` row 241's: a constant cast's statement position (ahead of the
range table), a non-string source's `42846` raised on the value rather than in parse
analysis, a constant 18.6 folds while planning under an `AND`/`OR` operand its left
operand decides — silent here, since such an operand is not evaluated (row 172) — and what `DROP TYPE … CASCADE` does not reproduce (a declaring
column — row 261 — and a member literal's dependency).

## An enum's labels are a set

`CREATE TYPE e AS ENUM ('b', 'a', 'b')` is refused. 18.6 enforces it with an ordinary
unique index over `pg_enum (enumtypid, enumlabel)`, so the refusal is that index's
`23505 duplicate key value violates unique constraint "pg_enum_typid_label_index"`,
and pglet raises the same error under the same constraint name from
`execute_create_type` — **below** the type name's own `42710`, since the labels are
only reached once the type is known to be new: `CREATE TYPE ctdt AS ENUM ('a');
CREATE TYPE ctdt AS ENUM ('b', 'b')` is *type "ctdt" already exists* on both engines
(`enum/create_type_duplicate_type_name_beats_the_duplicate_label.sql`).

A label is a **string literal**, not an identifier, so it is compared as written and
never folded: `('A', 'a')` is two members on both engines. That is where this rule and
the identifier fold ([parser.md](parser.md)) part company — the type *name* beside them
is an identifier and does fold, with the re-fold residual § Deferred records.

**What a repeat costs is the catalog and the dump, never the order.** The comparator
ranks a member through `enum_rank`, which is `position()` — the first match — so a
second `'b'` is never ranked, distinct labels keep distinct ranks, and
`values_compare_ordered`'s enum arm stays a total order with `Equal` ⟺ byte-equal
(the measurement `executor::expr`'s
`a_duplicated_label_keeps_the_comparator_a_total_order` records, taken from SQL on an
engine without the gate across `ORDER BY`, `min`, `max`, `greatest` and `least`). What a
repeat does cost is a `pg_enum` answering two rows for one label — which makes this doc's
own *one member is one label is one OID* premise locally false — and a dump stock
Postgres refuses: `CREATE TYPE e AS ENUM ('b', 'a', 'b')` is exactly the text
`pglet::dump` emits, so the invariant one such type breaks — the dump invariant's catalog part
([sequences.md](sequences.md)), *any catalog pglet's own accepted statements build must
dump to something that loads into stock Postgres and back into pglet* — is the **whole
database's** and not that type's.

**The gate closes the producers, not the state.** The persisted shape carries no mark of
the gate, so a store written by a build without it can hold such a type, reopens, and is
read through the comparator's enum arm — which is what that test pins, and why it is a
legacy-state pin rather than a guard for the gate: it passes with the gate and without it.

The structured **error fields** diverge here, on the omission side of
[errors.md](errors.md)'s rule. Measured on the raw wire: 18.6 sends
`S V C M D s t n F L R` and pglet sends `S V C M n`. `F`/`L`/`R` are that rule's
standing omission — pglet has no C source tree — and what remains, `D` and `s`/`t`, does
not come from one cause. **`s`/`t` is the posture**: 18.6 fills them from a **catalog**
relation (`pg_catalog` / `pg_enum`, the index being `pg_enum_typid_label_index`), while
pglet emits `s` only beside `t` and its `s` is the single `IMPLICIT_SCHEMA` — so filling
`t` here would send a wrong `public`. **`D` is a gap**: 18.6's is
`Key (enumtypid, enumlabel)=(<oid>, b) already exists.`, whose `enumtypid` is a live OID
and so is not byte-stable across runs, and both of its components are derivable at the
gate — the label as written, and the OID the statement would take (`next_object_oid`,
read just below it without advancing). What drops it is the payload shape: the refusal is
`Error::UniqueViolation` with `key: None`, and a `UniqueViolationDetail` carries the
relation beside the key description, so the shape that would fill `D` fills `t` with it.
Tracked in `EDGES.md` row 76. The 18.6 *position* is not a new one either — the
`LOCATION` is `_bt_check_unique`, the same one an ordinary unique violation reports,
measured on the oracle for both.

## An enum's order is its members' positions, and one list says so

18.6 orders an enum by `pg_enum.enumsortorder`; pglet stores the ordered member labels
on the type (`Schema.enums[name].labels`), and a site that decides an **order** over an
enum value reads that list. A query site borrows the slice; the three sites that cannot
(`IndexSpec::column_enum_orders`, `PreparedIndex`, `JoinColumnMap`) copy it, and
only the first outlives the statement that built it — § A copy that outlives its
statement. The one comparator is
`executor::expr`'s `values_compare_ordered`, and its enum arm ranks each label by
`enum_rank` — `Ok(position)` for a member, `Err(label)` for one the type does not hold,
which Rust's `Ord` on `Result` orders as *every member before every non-member,
non-members among themselves by bytes*. Three properties that rule has and no
interleaving has: it is a **total order** (`sort_by`'s requirement), it cannot brick a
readable table, and it keeps `Equal` ⟺ the labels are byte-equal.

**Equality does not move**, and that is a conformance fact rather than an omission:
18.6's `enum_eq`/`enum_ne` are bare OID identity (`enum.c`), and one member is one label
is one OID — so `=`, `<>`, `IN`, `DISTINCT`, `GROUP BY`, window partitioning and the
`ON CONFLICT`/FK matchers stay on the label bytes and are right there. What orders by
member position is `<`/`<=`/`>`/`>=` (and `BETWEEN`, which the parser desugars into that
pair), `ORDER BY`
— the `Plan::Sort` node, which is also what a `DISTINCT ON`'s effective sort and a
set operation's trailing `ORDER BY` lower to, plus the window's own key path —
`MIN`/`MAX`, and `GREATEST`/`LEAST`.

### The identity lives on the column

An enum value is a `Value::Text` and its `TypeOid` is `Text`, so nothing in the value
says which enum it belongs to. The **column** does: `Column.enum_type` names the type,
`ColumnDesc::enum_type` carries it through the executor, and
`ColumnResolver::column_enum_order` is what an expression asks — the same object, and
the same `resolve`, that answers for the value and for the declared type.

### An expression's enum is its type's, and one exhaustive walk says so

18.6 needs no walk at all: `COALESCE(m, 'sad')` simply *has type* `mood`, by ordinary
type unification, and every site that orders it reads that. pglet has to derive the
identity, and `executor::expr`'s `expr_enum_with` is the derivation. Its rule: **a
node's enum is the unified enum of the operands its own result type is drawn from**, and
a node whose result type is not one of its operands' has none. So `COALESCE`, `CASE`
(its `THEN`s and its `ELSE`, never its operand or its `WHEN`s), `greatest`/`least`,
`nullif`, `min`/`max`, and the value-preserving wrappers (an alias, unary `+`) all carry
the enum under them; a comparison, a string function and a cast do not.

**An array node answers at its *element's* width**, which the rule as stated would not
give it: `ARRAY[…]`'s result type is `element[]`, not `element`. What the walk's answer is
*for* settles it — every consumer hands the member list to `values_compare_ordered`, whose
array arm compares element-wise and reads one list at every element, so the array node's
answer is its elements' unified enum and the same list serves both readings.
`Expr::Unnest` is the inverse and answers the same way: its result type *is* its operand's
element type. § An `ARRAY[…]` is not an enum array column has why that arm is needed and
what a `None` there costs.

Three properties of that walk are load-bearing:

- **The match is exhaustive — no `_` arm.** An allowlist of a column reference and its
  value-preserving wrappers with `_ => None` lets every type-unifying kind fall through:
  `ORDER BY COALESCE(m, 'sad')` sorts by label bytes, `CHECK (COALESCE(m, 'sad') > 'ok')`
  admits the *opposite* row set from 18.6's with no error on the row it accepts, and a
  unique partial index over the same predicate takes a duplicate 18.6 refuses. A catch-all
  makes the next such kind another silent wrong answer; an exhaustive match makes it a
  compile error, the reason `deparse.rs` is exhaustive too.
- **One walk answers for both facts that need it.** `expr_enum_with` is generic in what
  its leaf returns: the ordering sites instantiate it at the borrowed member list
  (`expr_enum_order_with`), and `expr_output_column` instantiates it at the enum's
  **name**, to fill the projected `ColumnDesc::enum_type` an enclosing clause then reads.
  The operator and common-type gates (`docs/coercion.md` § What the gate reads)
  instantiate the same walk over a clause's namespace and over the catalog, so the
  ordering sites, the descriptor and those gates are one arm list. Two lists would have to
  agree, and drift shows as a split: a descriptor list without the ordering list's
  `MIN`/`MAX` arm sorts `SELECT g FROM (SELECT id AS g, min(m) AS mm FROM t GROUP BY id) q
  ORDER BY q.mm` by label bytes while `HAVING min(m) < 'ok'`, one clause earlier, is right.
- **It asks the outside world two questions, not one.** `EnumLeaves` carries a column
  leaf *and* a subquery leaf — the same pair `planner::LeafScope` carries, for the same
  reason: those are the two nodes the walk cannot answer from the node and its children.
  Without the second, the `Subquery` arm has nothing to ask and answers `None` — what that
  costs is § The boundary, and the scalar subquery, with `EDGES.md` row 104's correction.

### A column unified from two others keeps the enum either side declares

A set operation's output column and a `JOIN … USING (m)`'s merged column are both typed
by a *unified* type rather than by one source column, so both build a fresh descriptor
with no passthrough wire metadata — and the enum is **not** wire metadata. It is what
says the column's `ORDER BY` orders by member position, and 18.6 types both spellings as
the enum.

The rule is `executor::expr::unify_enum_pair`, and **a side that declares no enum is no
information rather than a veto**. That is 18.6's `select_common_type`: an `unknown`-typed
literal or a `NULL` beside an enum branch resolves to the enum, so
`SELECT m FROM t UNION ALL SELECT 'ok'` is a `mood` column. Reading the `(Some, None)`
pair as a refusal — which is the pair real SQL actually produces, a both-enum-branch set
operation being the rarer spelling — leaves the merged column on the label bytes: measured
over `mood`, `happy, ok, ok, sad` against 18.6's `sad, ok, ok, happy`, and
`WHERE x > 'ok'` answering a different **row**, not a different order. Two *different*
enums stay `None`, a pair 18.6 refuses to unify at all.

**That is the rule over a *pair*, and the fold over it is not sticky** — the one property
`unify_enum` adds for an expression list and these two sites do not inherit. A nested set
operation and a `JOIN … USING` chain both fold `unify_enum_pair` pairwise over
already-merged descriptors, where a `None` reads as "no enum" whether the side carries
none or **cancelled** one below, so a third branch could revive what the first two
cancelled: `mood ∪ weather` by the label bytes, `mood ∪ weather ∪ mood` by `mood`'s member
positions, and the three-branch spelling under `WHERE x > 'ok'` a different row set. **No
statement reaches that, and no descriptor carries a third state.** Every shape that
reaches this fold carries two *different* enums — the only pair `unify_enum_pair`
cancels — and that pair is refused *above* the fold, at the gates that decide whether
two operands have a comparison operator or a common type: two enums share `typcategory`
`E`, so the set operation is `42846 UNION could not convert type weather to mood` and the
merge's equality `42883 operator does not exist: mood = weather`, here as there, at every branch
count. The pairwise arms stay right for the ordering question they answer. `EDGES.md`
row 109, closed.

**The cancelling pair is not confined to a merge.** Two *different* enum columns compared
or folded directly — `m > y`, `greatest(m, y)` — reach the same `(Some, Some)` arm with no
branch count in it, where the answer falls to the label bytes: over
`mood AS ENUM ('sad','ok','happy')`, `m3 AS ENUM ('sad','happy','ok')` and rows
`('ok','sad')`, `('ok','happy')`, `('ok','ok')`, the bytes give `SELECT (m > y)` `f, t, f` —
`mood`'s positions give `t, f, f` and `m3`'s `t, t, f` — and `greatest(m, y)` `sad, ok, ok`.
The same gates refuse both, as 18.6 does: `42883 operator does not exist: mood > m3` and
`42846 GREATEST could not convert type m3 to mood`. That `42846` is not the `text` pair's
`42804`: with two enums 18.6's `select_common_type` succeeds, taking the first branch's
type, and `coerce_to_common_type` is what fails. Ungated, the pair writes, at a `DELETE`
predicate and at a `CHECK` or partial index. `EDGES.md` row 115, closed.

Where this rule alone could not follow 18.6: an enum against a plain `text` **column**
reads `None` on that side too, indistinguishable from the literal, and 18.6 refuses that
pair outright (`UNION types mood and text cannot be matched`, `operator does not exist:
mood = text`). Left to this rule, pglet answers either way — by the member positions under
it, by the label bytes without it — and a `CHECK` or a partial index over `greatest(m, t)`
persists that answer. `EDGES.md` row 114, the `enum <op> text` family, is **closed** by the
identity reaching the comparison and common-type gates (`docs/coercion.md` § What the
gate reads): the pair is `42883` at the comparison positions, and inside a `COALESCE`,
`CASE`, `greatest`/`least` or `nullif` it is 18.6's
`42804 GREATEST types mood and text cannot be matched` (and `nullif`'s own
`42883 operator does not exist: mood = text`) on both engines. The identity does not reach
the jsonb key/path, `LIKE`-family, `starts_with`, `||` and array-containment rules, nor the
string functions `EDGES.md` row 190 measures: there an enum is read as the `text` of its
label, so `m LIKE t`, `j ? m`, `upper(m)`, `m || m` and `ARRAY[m] @> ARRAY[t]` answer where
18.6 is `42883`, and they write.

**One rule** — `unify_enum_pair` is also what `unify_enum` (the expression
list) and `executor::helpers::unified_enum_type` (a merged column) read, so a comparison,
a `COALESCE` and a merged column cannot disagree about what an untyped side means; a
grouping query's `HAVING min(m) < 'ok'` is the scalar walk's comparison, the grouping
lending that walk its aggregates' values. Written out per site, it drifts.

**Two shapes, four sites** — each shape's descriptors are built twice, once by the
executor's node and once by `Plan::output_columns_scoped`, and neither reads the other.
Re-derive that list by grep rather than from this sentence. A pair that drops the enum
because a merged column carries "no single-column wire metadata" sends `ORDER BY m` over
the merged name by label bytes and makes `WHERE m > 'sad'` over it match nothing at all.

**Both halves of both pairs have a reader.** The planner's half is read by a **scalar
subquery** over a shape with no passthrough column: setting the planner set-op arm to
`None` reddens `enum/subquery_over_set_op_member_position`, and the planner `USING` arm
reddens `enum/subquery_over_join_using_member_position`; the executor halves redden the
`set_op_*` and `join_using_*` families. Each of the four arms has a fixture that
separates it.

**The pair's *computed-projection* halves both derive.** The executor's
`expr_output_column` derives a projected expression's enum onto its descriptor, and
`Expr::to_column_desc`, the planner's, derives it too. An underived planner half costs by
who reads it: the **ordering** readers take its `None` for *order by the label bytes*
(`EDGES.md` row 107), and the identity **gates** take it for *not an enum* — and five
namespaces they read a derived relation through — a derived table's and a CTE's scope
entries, an `UPDATE … FROM` source's, `Plan::output_columns_scoped`'s relations,
`executor::subquery_enum_name` — are built from that half, so ordinary SQL at all five
doors is refused `42883`. The derivation sits on `to_column_desc` rather than in the
namespaces because the namespace list is the one that goes short, and the site that
builds the descriptor is one.

**Those five are not all of them.** A *genuinely recursive* CTE's relation is built by
neither producer: `parser::select`'s `recursive_cte_columns` **rebuilds** the seed's
descriptors to apply the `name(cols)` list, and a rebuild through `ColumnDesc::new` alone
answers `None` whatever the seed derived — the working table's column then orders by its
label bytes (a wrong row set, not only a wrong order) and its comparisons with a real enum
column measure `42883`, at a door row 114 lists apart from a plain CTE for exactly this
reason. It is a *third* producer of the computed-projection class, and a derivation cannot
close a namespace it does not build, so the question to ask of each is **who builds it**,
not whether the list is complete. So that rebuild carries the seed's identity, field by
field — and deliberately drops `table_oid`/`column_id`, which 18.6 also sends as `0` here.

### The boundary, and the scalar subquery

A **cast's** identity is erased where the parser reads the type name: a member
literal's cast is built as the plain cast to `text`, so `'happy'::mood` is
byte-identical to `'happy'::text` in the plan and no walk can recover it, and any other
operand's cast is a membership test the walk does not read (§ A cast to an enum
converts) under that same cast (`EDGES.md` row 104). That is the boundary.

**And the boundary writes** — which it *shares* with the `enum <op> text` family beside
it. What separates them is not which row a spelling is filed under but **the operand
standing beside the cast**, and the pairs are stated as measured rather than as a rule
over neighbour kinds, because 18.6's verdict, pglet's answer and pglet's *rule* each vary
with the neighbour and not together. 18.6 answers beside a bare `unknown` literal, which
it resolves to the cast's own enum (`pg_typeof(greatest('sad'::mood, 'ok'))` is `mood`),
beside a second cast to the **same** enum, and beside no neighbour at all
(`ORDER BY m::mood`) — by the member positions, where pglet answers by the label bytes
and there is no refusal to set against pglet's answer. It refuses beside a `text` one
(`'happy'::mood > 'sad'::text` is `f` here and `42883` there, row 114's over-acceptance
reached by a cast, still answered here by the label bytes rather than by that row's member
positions) and beside a **different enum** — `'ok'::mood > y` over an `m3` column is
`t, t, f` here, `m3`'s member positions rather than the label bytes, and
`42883 operator does not exist: mood > m3` there. `unify_enum_pair`'s `(Some, None)` arm
is why those two split: the cast's erased side yields to whatever the neighbour declares,
so the label bytes are what is left only when neither side declares an enum. The erased
operand reaches a `DELETE` predicate, an
`UPDATE … SET` right-hand side, and — both engines accepting the DDL, where the
cast-free `CHECK` over `greatest(m, t)` is `42804` on both — each of the four
persisted-expression
positions `docs/ddl.md`'s render probe runs at: a column `DEFAULT`, a `CHECK`, an index
predicate and an index **key** expression. A written cast of a **member literal** at
any of the four renders `::text` in the dump, which stock 18.6 refuses under `42804` or
`42883`, decided by the statement and not by the position — so the database fails the
dump invariant's catalog
part, *any catalog pglet's own accepted statements build must dump to something that
loads into stock Postgres and back into pglet* — the control is a bare-literal spelling at each of
the four positions, which dumps and replays clean, measured. `EDGES.md` row 104 carries
the measured routes with the statement and code at each, some of which report 18.6's own
tag and differ only in what is
stored, and the restore's per-process silence. A cast of any other operand renders as
the enum cast it is (§ A cast to an enum converts), so `CREATE INDEX … ((m::mood))`
dumps as `((m)::mood)` and restores.

A **scalar subquery** is not a second boundary. A walk that resolves a column reference
through its leaf and holds no plan output descriptors compares `(SELECT m FROM u) < 'ok'`
by the label bytes — the reverse verdict for `mood` — and `ORDER BY (SELECT …)`, where a
scalar subquery is refused, is `0A000` here ("scalar subquery requires execution
context") because that site evaluates with no subquery callback lent to it — nothing to
do with enums. What refuses is that **class**, and the class is not a list of
SQL positions and is not enum-shaped: it is every evaluation lent no subquery callback of
the kind the node needs, which `grep SubqueryExec::NONE` names and `EDGES.md` row 89
states — with `GROUP BY (SELECT n …)` and `INSERT … VALUES ((SELECT n …))` among the
members carrying no enum at all. Read the row for the derivation. The other examples in
this file carry enums because enums are its subject; they are not the class. Reachable,
and silent under a blind leaf (measured): a projected comparison, `WHERE`, `BETWEEN`,
`COALESCE`, `CASE`, `greatest`/`least`, subquery-against-subquery, and — the two that
write — a `DELETE` and an `UPDATE` predicate, where
`DELETE FROM t WHERE (SELECT m FROM u) < 'ok'` removes **every** row where 18.6 removes
none and the `UPDATE` spelling reports `UPDATE 3` against `UPDATE 0`, both with no error
on either engine.

So the walk asks for a subquery's enum the way it asks for a column's: `EnumLeaves` is a
**pair** of leaves, and its second is the plan's first output column's enum
(`subquery_enum_order` / `subquery_enum_name`). Consequences worth carrying:

- It reads the **planner-built** descriptor's enum (`Plan::output_columns`), as
  `planner::coercion`'s `resolved_subquery_type_in` also does.
- It reaches only what that descriptor carries. A subquery
  projecting a *passthrough column* carries the enum because the projection propagates the
  whole descriptor; one projecting an **expression** (`SELECT min(m) FROM u`,
  `SELECT COALESCE(m, 'sad') FROM u`) carries it because `Expr::to_column_desc` **derives**
  it — the planner's side of the computed-projection pair above (`EDGES.md` row 107).
  That derivation's own subquery leaf declines,
  for a cost reason measured on both sides at `PlannerScope::expr_enum_shallow`, so a
  projection that *is* a scalar subquery (`SELECT (SELECT min(m) FROM u) FROM u`) is still
  label-byte ordered, and so is the **no-FROM** spelling of that nesting, the `Values` arm
  deriving an identity only from a column its record of enclosing references holds. That
  is that row's residual.
- **The `ORDER BY` key site and the window's order-key site thread the leaf and cannot
  observe its answer**, and say so where a reader will find it: `executor::query`'s
  `order_key_enum_order` and `window.rs`'s `compute_window_column` both take the catalog
  solely to answer this leaf, and a scalar subquery in either position is `0A000` before
  any comparison happens. Consistent-by-construction rather than dead: a blind leaf at
  either site is a second, differently-shaped walk that answers wrongly — silently — the
  day row 89's class closes.
- **It is asked only where its answer can be read.** The comparator's enum arm needs both
  operands to be `Value::Text`, so the ordered-comparison sites ask
  `operand_can_be_enum` of the values they already evaluated before walking at all. That
  is a cost rule, not a semantic one: the leaf costs a `Plan::output_columns` per
  evaluation, which on a comparison operand is per row, and without the guard an `int`
  comparison against a subquery — with no enum anywhere in the statement — pays 38.0 ms
  against 29.9 ms with it, and a bare `i > 5` 5.21 ms against 3.82 ms (release, 20 000
  rows, medians of three).

A subquery *beside* an enum column in a comparison is answered by the sibling, through
`operands_enum_order`, which makes the shape look narrower than it is.

### An `ARRAY[…]` is not an enum array column

*Enum arrays have no column form here* is **true of a column and is not a bound on the
node**: `ms mood[]` is `type "mood" is not supported` on this engine (§ Deferred), while
`ARRAY[m]` is an ordinary expression pglet builds, orders, compares and writes through.
So the `Expr::Array` / `Expr::Unnest` arms answer the elements' enum; a `None` there,
read off the column form, leaves an array comparison on the label bytes — the same shape a
`_ => None` allowlist leaves `COALESCE` on, one type constructor up.

**Three readers had to move together, and each alone changes nothing.** The walk's arm
(the elements' unified enum); `values_compare_ordered`'s array arm, which compares each
element pair under the caller's order — the *unordered* `values_compare` there discards
any order a caller resolved; and `operand_can_be_enum`, which admits an array whose
elements could be labels
— without it the widened walk is never asked. That last one is a **gate**, and its
contract is one-directional: a superset is safe and an undercount is not, so it recurses
into the elements by the same `Value::Text` test the arm reads. An array with no
`Value::Text` anywhere — empty, or all-NULL — cannot reach the arm at all, the element
loop deciding those by the NULL rule or by the length comparison. The recursion costs on
the axis the gate above it saves on: an ordered `text[]` comparison walks where an
unrecursing gate would skip. **What that costs is measured pairs and not a band over
them** — `operand_can_be_enum`'s own doc carries the pairs, beside the ones that justify
the gate. A band is arithmetically false here: the two `text[]` shapes share a sign and
not a magnitude, and which fraction a ratio names moves with the choice of denominator.

Those three plus a **fourth** close `EDGES.md` row 111: the `AnyAll` arm passes both
operands, as `BinaryOp` does. The fourth alone moves nothing — `ARRAY[m]` is the operand
carrying the identity, and only the walk's `Array` arm answers for it: with the `AnyAll`
arm widened, reverting only the `Array` arm leaves the `ANY` spellings wrong (measured).

The `text`-side identity is the element rule's: `ARRAY[m] < ARRAY['ok']`, with bare
literals rather than casts on the other side, is `operator does not exist: mood[] <
text[]` on 18.6 and here. The comparison gate reads a computed operand through the
complete type walk wherever the pair names an enum, and it types both sides of this one
`text[]`; what decides is `erases_enum_identity`'s array arm, which does not let a bare
string or `NULL` literal element make the array undecidable — 18.6's `transformArrayExpr`
folds the elements through `select_common_type`, and an `unknown` element contributes
nothing to that fold. So an array's identity is the unification of its **typed**
elements', which is what `expr_enum_with`'s `Expr::Array` arm computes too.

It decides the **accept** side too, which makes it its own decision: `ARRAY[m, 'ok']` is
a named `mood[]` and `ARRAY[y, 'ok']` an `m3[]`. And it closes the same pair at a fold:
`COALESCE(ARRAY[m], ARRAY['ok'])` is `42846 could not convert type text[] to mood[]` on
both engines, beside the cast spelling `COALESCE(ARRAY[m], ARRAY[t])` that
`concrete_branch_type`'s `Array` arm refuses.
What is left is a `NULL` beside a typed element, which this engine casts to the fold's
target before any gate reads it and so cannot tell from a written `NULL::text` — row
104's. The spellings measured to diverge are the ones where a written cast beside an enum
is read as that pass's output: `ARRAY[m, NULL::text]` answers where 18.6 raises `42804`,
and `ARRAY[s, NULL::varchar(3)]` — a `CastTypmod`, which the exclusion does not match at
all — answers where 18.6 raises `42883`.

All three write paths reach the array comparison, and on the label bytes each is
silent — measured against 18.6 over `mood AS ENUM ('sad','ok','happy')`: a `DELETE` under
`ARRAY[m] < ARRAY['ok'::mood]` reports `DELETE 1` on both engines while destroying a
different row; a `CHECK (ARRAY[m] < ARRAY['ok'::mood])` admits the opposite row set with
no error on the row it takes; and a `CREATE UNIQUE INDEX … WHERE ARRAY[m] >
ARRAY['sad'::mood]` holds a duplicate 18.6 refuses.

What this does **not** reach is a FROM-item `unnest`'s own column
(`EDGES.md` row 112): `execute_unnest` and `Plan::FunctionScan`'s `output_columns_scoped` both
build that descriptor with `ColumnDesc::new` and derive no enum, and the only spelling that
puts a recoverable enum into such an array is a **correlated** column reference — whose
identity reaches the executor as a member *order* on an `OuterFrame` and never as the
name a `ColumnDesc` would need. The cast spelling
(`unnest(ARRAY['happy'::mood, …])`) is row 104's boundary rather than that descriptor's,
and the scalar-subquery spelling is `0A000` in that position (row 89's class).

### An index over an enum column is declined

The B-tree key encoder has no enum arm — it encodes a label's bytes, as it does any
`Value::Text` — so an index over an enum column is ordered by something the engine does
not order by. `planner::index_selection`'s `single_column_index_for` declines an
enum key column, which drops it for all three of its consumers (equality, range, and the
`ORDER BY` `Sort` elision) at once. The cost is **performance**: the row filter is
exact, and equality over the label bytes is still enum equality. Encoding the member
position in the key instead is a persisted-shape change, and 18.6's own
`pg_enum.enumsortorder` is a `float4` precisely so `ALTER TYPE … ADD VALUE …
BEFORE/AFTER` can insert between two members without rewriting them.

### The two write paths, and the one that is not a query

A `CHECK` over an enum column is a stored expression re-evaluated on every write, so the
member order decides which rows land — `CHECK (m < 'ok')` keeps the members declared
before `ok` and refuses the rest, and the label bytes keep and refuse *exactly the
opposite* set for `mood`. `enforce_checks` takes the orders beside the declared types it
already took — re-derive its call-site list by grep rather than from a count here; it
spans the `INSERT` path, the shared `enforce_row_checks` (`UPDATE`, and through
`enforce_row_constraints` `COPY` and `ON CONFLICT DO UPDATE`, and an `UPDATE`'s FK actions'
`SET NULL` and repoint writes), a `DELETE`'s FK actions' `SET NULL` images
(`ActionState::rewrite`), the
`ADD COLUMN` back-fill, and the `ADD CONSTRAINT … CHECK` existing-row scan.

The second is a **partial index whose predicate orders an enum**, whose membership is
decided in two objects nothing compares — the executor's `PreparedIndex`, rebuilt
from the live catalog once per statement, and the storage layer's `IndexKeying`, which
holds no catalog at all and reads the orders off the `IndexSpec` (`column_enum_orders`,
beside `column_types`). Disagreement there is not a wrong row order but a silent wrong
**write**: a *unique* partial index that holds neither of two rows reports no conflict
for a duplicate 18.6 refuses.

### A copy that outlives its statement

`IndexSpec::column_enum_orders` is the one copy of the member order that outlives the
statement that took it — built at `CREATE INDEX` and on reopen, and modelled on
`IndexSpec::column_types`, whose lifetime is safe for a reason it does not share: a
column's declared **type** cannot change here (`ALTER COLUMN … SET DATA TYPE` is
`0A000`), while an enum's **member list** can. `DROP TYPE` under a live dependent column
is `2BP01` on 18.6 and accepted here — a gap of its own, `EDGES.md` row 261 — so `DROP TYPE`
then `CREATE TYPE` with the members reordered moves what the copy froze. With the member
order load-bearing, that leak is the silent wrong write above: measured over
`r(id, k, m mood NOT NULL)` under `CREATE UNIQUE INDEX r_u ON r (k) WHERE m > 'ok'`, a
reorder against a frozen copy leaves the executor on the live order and storage on the
frozen one, and two rows sharing `k` both land. **Recording a persisted cross-object
reference turns every pre-existing leak of the referenced object into a bug** — this
repo's own rule ([sequences.md](sequences.md)).

The rule that closes it: **the storage index specs follow `Schema.enums` wherever it
moves.** One operation, reached from five sites in two directions — and the backward
half's membership is exactly `Schema::restore_from`'s own call-site list, which is where
to re-derive it rather than from this sentence:

- `execute_create_type` and `execute_drop_type` — forward, through
  `rebuild_indexes_over_enum_staged`, over the indexes whose *table* carries a column of that
  enum (`column_enum_orders` is table-wide, so that is exactly the condition under which
  the spec's bytes change; a narrower "does the predicate mention it" test would be a
  second derivation of the same fact). `DROP TYPE` counts because a column left declaring
  a gone type orders by label bytes, which is a move like any other.
- Every **backward** `Schema::restore_from` — `TransactionSession::rollback`,
  `::rollback_to_savepoint` (those two through one `resync_index_specs`), and the `DO`
  block's undo in `executor::mod` — run for a caught body before its handler, and for a
  `DO` statement that fails. A restore moves the enums back, which the
  forward sites cannot see; the forward direction alone leaves this one open. Gated on the
  enum maps actually differing, computed before the restore
  (`executor::enums_moved_by_restore`, the one derivation all three read), so an ordinary
  `ROLLBACK` — every fixture runs one — re-derives nothing.

  **That difference has two directions, and the second is not the reorder.** A name the
  *snapshot* holds and the live catalog does not is a move as well — a `DROP TYPE` inside
  the block, whose forward rebuild put the specs on the label bytes — and it is a
  different arm of the derivation from the reordered-name one. A script that moves the
  enum by *reordering* it reaches only the **first** arm: with the second deleted, every
  reorder test stays green while `BEGIN; DROP TYPE mood; ROLLBACK;` takes a duplicate under
  a unique partial index, with no error on any statement (measured). Pinned by
  `crates/pglet/tests/enum_reorder_index_specs.rs`'s
  `a_rolled_back_drop_type_with_no_re_create_leaves_the_specs_on_the_restored_order`, and
  directly by `pglet_sql`'s `enums_moved_tests`.

  The `DO` handler is the site an enumeration easily misses, and without the resync it
  desyncs silently: a `DROP TYPE` + re-`CREATE TYPE` reorder inside a caught body moves the
  catalog and the specs forward, the handler puts the catalog back, the specs keep the
  block's order, and a unique partial index over `WHERE m > 'ok'` then takes a duplicate
  with **no error on any statement**. It needs no second connection and no refused key type, and 18.6 is not
  its oracle: its own `DROP TYPE` under a live dependent column is `2BP01`, so what the
  script violates is this engine's own invariant that its catalog and its index specs
  agree. Pinned by `crates/pglet/tests/enum_reorder_index_specs.rs`'s
  `a_caught_do_block_leaves_the_specs_on_the_restored_order`, with the
  enum-untouched-body control beside it.

The two **forward** sites stage: they hand the rebuild the enum map they are about to
install and write `Schema.enums` only once it returns. The rebuild is fallible and this
is DDL, so the repo's standing rule applies — *a failed DDL statement must leave the
catalog untouched*, because the autocommit path cannot put it back — and the failure is
reachable rather than theoretical. A key that cannot be derived for a row the move newly
admits makes `rebuild_index` raise — an expression key `(10 / d)` over a row holding zero is
`22012` there, the key being evaluated only for an admitted row —
and with the write first, `DROP TYPE` and the re-`CREATE TYPE` alike would report that
error and keep the move, leaving a type whose members no statement claims to have set. The
**backward** site does not stage, and correctly: there the catalog write is the intended
outcome, not a step that has to be undoable.

Staging the catalog is **half** of it, and on its own it points the same desync the other
way: the rebuild loop is a second mutable side, and an index it has already moved sits on
the staged order while `Schema.enums` — held back — does not. Measured without the unwind,
over two tables carrying the one enum, a unique partial index on the first and a partial
index whose key raises for the re-admitted row on the second: the failed `DROP TYPE` lets
two rows sharing the unique key both land, where the same script without it raises
`23505`. Which way it falls is
`Schema.indexes`' `BTreeMap` iteration — **lexical index-name order** — and the block
spelling is not a repair, because a catalog staging never moved gives
`enums_moved_by_restore` nothing to resync. So the loop **unwinds**: both orders are
derived before the first mutation, and a failure rebuilds the indexes already moved back
onto the order they came from. That the failing index itself needs no unwinding is
`Storage::rebuild_index`'s **all-or-nothing** contract, which the in-memory backend keeps
too: a rebuild that drops the old definition before building the new one deletes the index
from storage on a refused key encoding while the catalog still holds it.

**What the unwind buys is measured at the two forward callers, and measured vacuous at
the rollback one.** At `CREATE TYPE` / `DROP TYPE` its `from` order is the pre-statement map,
so a failed rebuild puts the moved specs back — the two-index script above, pinned in
§ Test coverage. At `TransactionSession::rollback` two things compound: the restore has
already written the catalog (`Schema::restore_from` runs, then `resync_index_specs`), so
nothing was staged, and the unwind's `from` order is then the *destination*, so it
re-derives the bytes the failure left. Measured through a route the writer closes
(below), on the default B+tree backend, native server, two connections, over the same
two-table shape with the failing index — keyed on a value the encoder refused, a `float8` under
an encoder without a float arm — named to sort **first**: the rollback's resync raises on it,
the catalog stands on the
restored order, the unique index's spec keeps the order the block installed, and a
duplicate that index should refuse lands — `(2 rows)` where the same script without the
failing index rolls back cleanly and answers `23505`. The failing index's own spec keeps
the block's order too, measured by a later insert the restored order puts outside its
predicate and the block's order puts inside. `ROLLBACK TO SAVEPOINT` raises the same way
from its own call, in its own method. Recorded as `EDGES.md` row 110: that route is
closed, no other is known, and the mechanism stands.

The **shape** is every backward site's, the `DO` handler included: none of them stages, so
none has anything to put back. The **reach** is what has been measured at only the two
`TransactionSession` ones, and the difference is where the rows come from. Reaching it
takes a row that satisfies a partial index's predicate under the order being restored *to*
while carrying a key that cannot be derived. The two spellings that build that pair directly
both raise up front — an `INSERT` of such a row, and a `CREATE INDEX` over a table already
holding one, both measured — so what is left is moving the *order* under an existing pair,
and row 110's script did that from a second connection while the block held the moved
order — commits that wait for the block, which holds the writer (the embed refuses them
`55P03`), so no route is known. A `DO` block's undo has no such window: a `DoBlock` is never read-only, so the
native server runs it under `lock_both()` for the whole statement, and the embed and wasm
hosts run a statement behind `&mut self`, so a second logical connection there
(`Database::execute_as`) cannot interleave one. Its own body rows are discarded before the
restore,
so what it restores to is the body-entry state — the pair storage already held, which
`rebuild_index` re-derives from `(spec, current rows)`.

The closest attempted route fails a step earlier still: with a partial index whose key
raises for its row (a key the encoder refuses — measured with a `float8` key under an encoder
without a float arm) and
whose predicate a reorder inside the body newly satisfies, the body's *forward* rebuild
raises and stages, so the handler restores a catalog that never moved, `enums_moved_by_restore`
finds nothing, and the unique index that follows still refuses its duplicate — measured. No
route has been constructed at that site.

A write still **pending** in the transaction that moves the enum is outside what the
rebuild reads (it re-derives from committed rows), and that is not a hole: the pending
write is materialized at commit through `emit_index_ops` against the `index_defs` the
rebuild has already replaced, so it lands under the new rule. Pinned, over orders where
neither the pre-move order nor the label bytes can stand in for the destination's.

The operation is `Storage::rebuild_index`, and it is neither of its two neighbours —
both measured wrong here. `reattach_index` swaps the definition and leaves the entries
the old rule placed, so a row stored *before* the move keeps a membership the live rule
disagrees with, and on the in-memory backend it drops every entry outright (its defs own
their entries). `drop_index` + `create_index` rebuilds correctly on that backend and
**not** on the B+tree, where a drop taken inside a transaction — which the autocommit
path opens too — is deferred to commit and deregisters the definition the create had just
restored, surfacing as `index not found` on the next write.

## Test coverage

- Engine cases in `crates/pglet_sql/tests/engine_ddl.rs` (enum roundtrip + member
  validation).
- Differential fixtures vs real PG under `tests/fixtures/enum/`, read off the directory
  rather than off a list here. The **membership** family (default +
  explicit member, equality filter, invalid insert/update,
  cast-to-enum, update-to-member). The **cast-conversion** family (`cast_*`,
  `bare_non_member_*`, `drop_type_under_a_converting_check_loud`, and
  `params/enum_cast_text_param_*`) takes each spelling and operand kind to each place a
  cast is read, compared, written or stored, with the zero-row accepts
  (`cast_text_operand_zero_rows_accepts`, `cast_column_zero_rows_accepts`) as the
  controls that say a non-literal is converted on the value and not at plan time. The
  undeclared `$N` half is off-corpus by construction (`tests/README.md` § `-- @param`):
  `crates/pglet/tests/enum_identity_gates.rs`, `pglet_server`'s Bind test, and the wasm
  and npm suites; `executor::expr`'s `enum_in_tests` pin the node's shapes and the value
  rule, and `pglet`'s dump round-trip corpus its rendering. The **member-order** family names, in each file, the
  site it reaches, so `ls tests/fixtures/enum/` is the coverage list and a site with no
  fixture is a name that is not there. Read three of them as **accept-side** pins rather
  than divergence locks: `order_by_text_cast_is_lexical` (a cast away from the enum is
  still label-byte ordered, on both engines), `order_by_alphabetical_enum_unchanged` (the
  spine's own `cell_visibility`, whose declared order *is* its lexical one — which is why
  its traffic cannot show the divergence), and `equality_indexed_unaffected` (the
  index decline costs an equality nothing but speed). `partial_unique_index_member_position`
  and its `_excludes_` sibling are the same pair for the write side.
- The comparator's own rules — the total order, `Equal` ⟺ byte-equal, the non-member
  arm, two enums over the same labels, and the pairing between `operand_can_be_enum` and
  the arm that reads a member order (the rule, not its wiring: a call site that dropped
  the gate costs time and reddens nothing) — in `executor::expr`'s `enum_order_tests`;
  the index decline as a **plan shape** in `planner::tests`'
  `enum_indexed_col_is_declined` with its `text_indexed_col_beside_an_enum_one_still_accelerates`
  control; the stored-non-member and the two DDL-time `CHECK` validation paths in
  `crates/pglet_sql/tests/engine_ddl.rs`.
- The **node rule** arm by arm in `executor::expr`'s `expr_enum_walk_tests`. What it
  covers that no fixture can is the arms whose whole content is a `None` — the cast
  boundary, and every node whose result type is not its operands'. A fixture compares
  two answers and 18.6 is *right* where pglet declines, so over-widening the walk (giving
  `Cast` an arm) would silently change what `EDGES.md` row 104 records and redden nothing.
  The `Subquery` arm's two tests are there for the opposite reason: they pin that the arm
  **asks** its leaf — with a deliberately blind leaf beside them, so an arm hard-coding an
  answer fails them too. The `Array` / `Unnest` pair's tests hold the two things a fixture
  is weakest on: that *every* element is unified rather than the first (a mixed cast-then-column
  array, which the single-element spelling cannot separate), and that an empty array is
  deliberately `None` rather than a guess.
- The **array comparison** in `executor::expr`'s `enum_order_tests`: two arrays compare
  element-wise against one member list, each assertion paired with the same call passing
  `None` — without which an element loop that dropped the order would pass, the label bytes
  agreeing with the member positions on some pairs by luck. Its sibling pins the **gate**'s
  array recursion over the shapes that cannot reach the arm (empty, all-NULL, non-text),
  each under the gate's own contract: *a pair the gate rejects must not read the member
  order.*
- The `ARRAY[…]`-expression family under `tests/fixtures/enum/` — the filter, the range
  filter, the two `ORDER BY`s (one of them the mixed-element discriminator), the `DELETE`,
  the `CHECK`, and the unique partial index — plus `any_all_array_side_member_position`,
  `any_all_scalar_side_member_position` and `delete_over_any_array_side_member_position`,
  the second of which is the accept-side control that agrees on both engines, and
  `order_by_unnest_of_array_member_position`, the one spelling in the corpus that reaches
  the `Unnest` arm. Breaking any one of the `Array` arm, the comparator's element call, the
  gate, the `AnyAll` walk's width or the `Unnest` arm reddens a different subset of that
  set, and the `Array` arm reading its first element rather than unifying is caught by the
  mixed-element fixture and the walk test alone. `EDGES.md` row 111 carries the
  per-mutation subsets.
- The **unification rule**'s own arms in `executor::helpers`'s `unified_enum_type_tests`.
  The reachable `(Some, None)` pair is held by fixtures (`enum/set_op_*`, in both operand
  orders). The two-different-enums pair is held by fixtures too
  (`enum/two_enums_setop_loud.sql` and its siblings), the gate above this arm refusing it
  as 18.6 does; what only these unit tests can hold is the **arm's own** verdict below that
  gate, which no statement reaches. Measured: a version returning the **left** side
  unconditionally fails two of these tests *and*
  `enum/set_op_literal_branch_on_the_left_member_position` — which is why that fixture's
  mirror image exists rather than one of the pair.
- The `IndexSpec` resync in `crates/pglet/tests/enum_reorder_index_specs.rs`. Not a
  fixture, and the reason is the **harness's**: 18.6 refuses to enter the state
  (`2BP01`), so a differential fixture's `@setup` fails on the oracle side and is recorded
  as the fixture's whole result. There is no answer to compare against. What each test
  covers, stated at its own width:
  - **B+tree, forward and backward**, plus the unrelated-type and same-member-list
    controls, the `DROP TYPE`-without-re-create route, and the expression-key route.
  - **The caught `DO … EXCEPTION` handler**, the third backward `Schema::restore_from`.
    Red with the resync deleted, with the resync moved *before* the restore, and with
    `enums_moved_by_restore` read *after* it (where the two maps are already equal and it
    finds nothing), measured. The enum-untouched-body control beside it is an
    **accept-side** pin rather than a further separation: a blanket resync over every enum in the snapshot leaves it green, measured,
    because re-deriving an unmoved spec produces the same bytes — so what the `moved`
    comparison saves *behaviourally* is work, which no assertion in that file reads. Its
    **output** is a separation, and a different test's — the `enums_moved_tests` entry
    below.
  - **A failed `DO` with no handler**, the same undo reached by a statement's failure:
    `a_failed_do_block_leaves_the_specs_on_the_restored_order` reads the restored member
    list and then the `k = 1` collision, and is red with the undo deleted (on the member
    list) and with the resync alone deleted (on the collision), measured.
  - **The `DROP TYPE`-only rollback**, in
    `a_rolled_back_drop_type_with_no_re_create_leaves_the_specs_on_the_restored_order` —
    the derivation's *second* arm, a name only the snapshot holds. It reads the restored
    member list as its premise before asserting anything, and reads *which* error the
    duplicate raises rather than that there was one. Its **entries** arm is the row stored
    before the block: its `k = 9` pair inserts both rows after the `ROLLBACK`, so the
    restored *definition* alone decides it and a def-only backward resync passes it, while
    the pre-block row's entry — which the forward `DROP TYPE` took out of the index, and
    only a re-derivation puts back — is what such a resync fails: with
    `rebuild_indexes_over_enums` reattaching rather than rebuilding, that arm fails,
    measured.
  - **In-memory, forward** — the arm whose defs *own* their entries, where the two
    rejected neighbours fail differently. `Storage::rebuild_index`'s **all-or-nothing**
    contract is that backend's too, and is pinned at the storage level rather than here
    (`pglet_storage`'s `a_failed_rebuild_leaves_the_index_standing`), because a rebuild
    that refuses a key can never afterwards hold a row — so an index a non-atomic rebuild
    drops is unobservable through SQL.
  - **In-memory, backward** is a *behaviour* pin and not a guard on the resync:
    measured, it stays green with the rollback resync deleted, because that backend
    restores the whole store and the spec rides along. The resync's own necessity is the
    B+tree rollback test's, which does redden.
  - **A write pending across the reorder**, whose orders are chosen so neither the
    starting order nor the label bytes the interim `DROP TYPE` falls back to can stand in
    for the destination's.
  - **The staging**: a `CREATE TYPE` / `DROP TYPE` whose rebuild *fails* leaves the
    catalog untouched. The reaching failure is a key expression, `(10 / d)`, that raises
    for a row a predicate the move newly satisfies admits; unstaged, both statements
    report that error and keep the catalog move. Both of those build
    **one** index and so cannot reach the partial-rebuild split.
  - **The unwind**, which needs two: a unique partial index on one table and the
    `(10 / d)`-keyed one on another, named so that `Schema.indexes`' `BTreeMap` order puts
    the unique one *first* — deliberately, and the reversed-name sibling beside it is the
    control that says so (it answers correctly with the unwind and without). On the B+tree
    backend, in memory, and inside a `BEGIN … ROLLBACK`, since the rollback is not the
    repair.
  - **The OID counter**: a `CREATE TYPE` whose rebuild fails must not have advanced
    `next_oid`, read through the `pg_class` OIDs of two tables either side of it. What it
    cannot see is a counter never advanced at all, which `next_object_oid`'s re-seed from
    the catalog max hides — a different rule, and unguarded.
- The **restore derivation itself**, in `executor::ddl`'s `enums_moved_tests` — the one
  test in the workspace that reads `enums_moved_by_restore`'s *answer* rather than a write
  downstream of it. What wants that shape is the **blanket** mutation: replacing the body
  with `snapshot.enums.keys().cloned().collect()` is correct-but-slower, so nothing writes
  differently — measured over `cargo test --workspace --no-fail-fast`, exactly one test
  reddens, this one. It reddens under the second-arm deletion too, beside the behavioural
  guard above.
- The un-accelerated key-scan fallback's enum arm in `executor::query::scan`'s
  `enum_key_fallback_tests`, over a row stub. That arm is **unexecuted** by everything
  else in the repo — measured, by forcing the fallback's order to `None` and finding
  exactly those two tests red across the fixture corpus and the whole `cargo test`
  workspace — and no plan can reach it while `single_column_index_for` declines an enum
  key column. An unexecuted arm is not a tested one, which is the whole reason those
  tests exist (§ An index over an enum column is declined).

### Harness note (not a pglet bug)

**A fixture must not project a bare enum column, or an enum-valued expression under an
unaliased name.** What blocks it, and only the first is about pglet:

- pglet's `RowDescription` declares the column `text` (OID 25) where 18.6 declares the
  enum's own OID, so the harness's **declared-type** comparison diverges on both legs.
- the binary leg's decoder has no arm for a custom enum OID; its fallback is an
  `UndecodedType` whose `accepts()` is `false`, so the read is a loud `CLIENT-FAULT` on
  the Postgres side.
- `row_to_json` keys its map by **column name**, and the two engines name an unaliased
  enum expression differently (`SELECT 'happy'::mood` is `mood` on 18.6 and `text`
  here).

So the value-returning fixtures project `<col>::text AS <label>`, and the label is
load-bearing rather than cosmetic: an *unaliased* `m::text` is still named `m`, so an
`ORDER BY m` binds that output column instead and both engines agree lexically — which
hides the member-order rule entirely. `filter_by_member` needs no cast at all: it
exercises enum equality server-side and returns only `id`.

## Deferred

- **An enum type has a catalog OID; a column still reports `text` on the wire.**
  The type's OID is persisted (`EnumType.oid`, since store magic `PGMJ`) and is what
  `pg_type.oid` and `pg_enum.enumtypid` are joined on, and `pg_attribute` reports it
  as an enum column's `atttypid` — so introspection resolves the type. What is still
  deferred is the **wire** half: a column's `RowDescription` type is `TypeOid::Text`,
  not the enum's OID, and a param declared as the enum's own OID (so tokio-postgres
  sees `Kind::Enum`). A text param cast to the enum (`$N::text::cell_visibility`)
  binds — `tests/fixtures/params/enum_cast.sql` and `test_binary`'s
  `Params text::enum` case pin it (see [param_binding.md](param_binding.md)).
- **A cast's enum identity is erased** — § The boundary, and `EDGES.md` row 104: an
  expression with no enum-typed *column* under it orders by label bytes. What a cast
  **converts** is built: a label the enum lacks is refused wherever 18.6 refuses it
  (§ A cast to an enum converts; `EDGES.md` row 241, closed), and an
  `ADD COLUMN … DEFAULT` back-fill of one is refused at the `ALTER` (row 105, closed).
  (Also noted in [compatibility.md](compatibility.md).)
- **No `ALTER TYPE … ADD VALUE`**, and no enum array **column** (`cell_visibility[]` is
  `type "cell_visibility" is not supported` at `resolve_column_type`, as is the
  `ALTER TABLE … ADD COLUMN` spelling and a `::cell_visibility[]` cast target — a
  uniform *loud* over-rejection where 18.6 accepts the declaration). General array
  support exists; the enum-array **column** is what remains unbuilt. An `ARRAY[…]`
  **expression** over enum-typed elements is not in this item and is ordered by member
  position — § An `ARRAY[…]` is not an enum array column. `index_method_for` reports GIN
  only for a container type, so an enum column never reaches a GIN index either.
- **The enum name is ASCII-downcased again where it is stored** — a residual of the
  identifier case-fold (`docs/parser.md` carries the fold rule; `execute_create_type` /
  `execute_drop_type` / `resolve_column_type` re-fold the catalog key). Because a
  *quoted* type name does not parse in the type position, that re-fold reaches the
  statements that name a type outside it, `CREATE TYPE` and `DROP TYPE`. A quoted
  `CREATE TYPE` name: `CREATE TYPE
  "Loud" AS ENUM (…)` then `CREATE TABLE t (m loud)` is accepted here and `42704` on
  Postgres. And a quoted or unquoted `DROP TYPE` name, silently: `DROP TYPE "Rd"` drops
  an enum `rd`, and `DROP TYPE rd` one created as `"Rd"`, where Postgres answers `42704`
  to each — unless a table holds the name as written, whose row type
  `execute_drop_type` finds first (`2BP01`, `EDGES.md` row 248). The fold is ASCII-width, so
  `CAFÉ`/`café` stay two types (matching Postgres) while `"Mood-X"`, an unquoted
  `Sequence`, or `MÖÖD` each store a name `pglet_server dump` cannot render — the
  dump then exits without emitting anything, making the whole database unexportable
  (`EDGES.md`).
