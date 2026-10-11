# ENUM types — `CREATE TYPE … AS ENUM`

An enum is a text label plus a catalog member set. `CREATE TYPE … AS ENUM` registers the
members, an enum column stores the label, and the write path, a cast to the enum and a
column `DEFAULT` each refuse a non-member. The examples below use
`CREATE TYPE cell_visibility AS ENUM ('private', 'public')`, the enum the fixtures under
`tests/fixtures/enum/` define. § Test coverage lists the guards.

## Design: enum = text label + catalog member set

PG enums round-trip as their text label, so pglet stores an enum value as
`TypeOid::Text` and keeps the enum's identity on the column. We don't add a
data-carrying `TypeOid` variant, because the closed `#[repr(u32)]` enum can't take one
cleanly. The catalog owns the type. The write path and every cast to the enum
(§ A cast to an enum converts) check membership.

- `Schema.enums: BTreeMap<String, EnumType>` maps the (ASCII-downcased) name to
  `{oid, labels}`: the catalog OID that `pg_type`/`pg_enum` join on, plus the ordered
  member labels. `Column.enum_type: Option<String>` is a key into it. Both are
  `#[serde(default)]` (a pre-stable on-disk break). It's a `BTreeMap`, like the other
  catalog maps, so `serialize_schema`'s postcard bytes stay canonical.
- `CREATE TYPE … AS ENUM (…)` is parsed by the native parser (`parser::ddl`) into
  `Plan::CreateType`, then inserted into the catalog. Only the `AS ENUM` form is built.
  The composite `CREATE TYPE t AS (a INT, b TEXT)` is `0A000` there.
- `DROP TYPE [IF EXISTS] … [CASCADE | RESTRICT]` becomes `Plan::DropType`, which lets
  fixtures reset idempotently. Its dependents are handled by `executor::dependencies`
  (see § A cast to an enum converts).
- CREATE TABLE / ALTER ADD COLUMN: `resolve_column_type` maps a declared type name to
  `(TypeOid, enum_type)`. An enum name becomes `(Text, Some(name))`, keyed on the
  decomposed base. Any other name goes through the shared `type_oid_from_parts`. A
  builtin's name still means the builtin even when an enum has that name: `CREATE TYPE
  int4 AS ENUM (…)` leaves `c int4` and `'5'::int4` as integers, as 18.6's search path
  does with `pg_catalog` first. The enum is only consulted for a name
  `type_oid_from_name` doesn't resolve, or for a `SERIAL` spelling (which 18.6 holds as
  no type). That's why a builtin 18.6 has and pglet doesn't (`oid`, `json`, `regclass`)
  still resolves to a same-named enum here (`EDGES.md` row 118).
- Member validation runs inside `validate_column`, the single per-column write-path entry
  point. INSERT, UPDATE, `ON CONFLICT DO UPDATE`, bulk INSERT and COPY all go through it.
  `validate_enum_member` is its helper, and also does the `ADD COLUMN` back-fill's member
  test. A non-member is `Error::InvalidEnumValue` (SQLSTATE `22P02`, matching PG). NULL
  passes, since nullability is a separate check.
- A column `DEFAULT` converts too, when it's cooked (`executor::ddl`'s
  `cook_column_default`). A bare label for an enum column is tested against the member
  list on `CREATE TABLE`, `SET DEFAULT` and `ADD COLUMN`, whatever rows exist, and the
  value `ADD COLUMN` back-fills is tested however it was spelled. So the only route known
  to store a label the enum lacks is `DROP TYPE` + re-`CREATE TYPE` under a column
  (`EDGES.md` row 261; row 105 records the routes that were closed).

## A cast to an enum converts

In 18.6, a cast to an enum is `enum_in`, and it runs wherever that operand's conversion
runs. For a string literal that's parse analysis, so over no row, and even in a `CASE`
branch no row takes. For an undeclared `$N` it's at `Bind`, when the cast is what fixes
the parameter's type. For anything else (a column, `'q'::text`, a declared parameter, an
undeclared one an earlier use already typed) it's on the value. pglet's parser reads the
type name once (`resolve_column_type`, the column form's resolver) and builds every
spelling (`::`, `CAST(… AS …)`, the typed constant `mood 'ok'`) through one builder,
`planner::build_enum_cast`:

- A string literal the enum holds, and `NULL`, keep the plain cast to `text`. An enum
  value is its label, so nothing that reads one sees a conversion.
- Every other operand becomes the conversion `StringFunc::EnumIn` under that same cast.
  Its arguments are the operand, the enum's name, and the member labels as the parser
  read them. The labels are stored in the node so the test can be decided where no
  catalog is available: a `CHECK` on a write, an index maintained in storage, the DDL
  probe. That makes them a copy of `Schema.enums[name]` that outlives its statement when
  the node is persisted. `DROP TYPE` is the only statement that changes a member list
  while a persisted expression survives it (`DROP SCHEMA` and a rollback move the tables
  along with the types). It treats such an expression as a dependency, as 18.6 does:
  `2BP01` without `CASCADE`, and with it the constraint is dropped or the default
  cleared, through the DROP-time resolver `executor::dependencies`. So the list can't
  change under a persisted node. An index can't hold the node at all. 18.6 refuses it
  with `42P17` because `enum_in` isn't immutable, and `CREATE INDEX` here does too,
  unless the operand is already the enum (a relabel, which is no function call on 18.6
  either). (A statement prepared before a `DROP TYPE` + re-`CREATE TYPE` and bound after
  it tests the new members on the native server, as we measured, where 18.6 answers
  `XX000 cache lookup failed for type`.)

Where each case is decided follows the existing constant-cast split, not a new rule:

- A literal the enum lacks is caught by `planner::enum_in_constant_legality`, called from
  the plan-time `validate_const_casts` (at every constant cast's position) and from the
  DDL walk `check_expr_node_operand_types` (a `DEFAULT`, `CHECK`, index predicate or key,
  over no row).
- An undeclared `$N` is caught by the enum target that `ParamTargets::enums` records, at
  each host's `Bind`. It's recorded only where the cast is the parameter's first
  type-fixing use in 18.6's parse-analysis order (`planner::param_types`' `first_fix`).
  In `WHERE ($1 = '' OR m = $1::mood)` the comparison typed `$1` as `text` first, so the
  cast converts on the value. A `WITH` query is analysed before the statement's own
  clauses wherever it's read.
- Everything else is caught by `executor::expr`'s `eval_enum_in`, on the value. A string
  gets a membership test, NULL stays NULL, and any other value is 18.6's
  `42846 cannot cast type <T> to <enum>`.

A label is compared as written, so `'SAD'::mood` isn't the member `sad`. The node renders
as the cast that was written, `(s)::mood`, in both deparse modes. So
`pg_get_constraintdef` and `column_default` print 18.6's strings, and a `CHECK` or
`DEFAULT` holding it dumps to a statement both engines load.

What it doesn't change is the **identity**. The node is a string function under a cast to
`text`, and the walk answers `None` for both, so `ORDER BY s::mood` and `m::mood > 'ok'`
order by the label bytes (§ The boundary, and `EDGES.md` row 104). The residuals are in
`EDGES.md` row 241:

- a constant cast's statement position (ahead of the range table);
- a non-string source's `42846` being raised on the value instead of in parse analysis;
- a constant 18.6 folds while planning under an `AND`/`OR` operand that its left operand
  decides, which is silent here since that operand isn't evaluated (row 172);
- what `DROP TYPE … CASCADE` doesn't reproduce (a declaring column, row 261, and a member
  literal's dependency).

## An enum's labels are a set

`CREATE TYPE e AS ENUM ('b', 'a', 'b')` is refused. 18.6 enforces this with an ordinary
unique index over `pg_enum (enumtypid, enumlabel)`, so its error is that index's
`23505 duplicate key value violates unique constraint "pg_enum_typid_label_index"`.
pglet raises the same error with the same constraint name from `execute_create_type`.
It comes **after** the type name's own `42710`, because the labels are only checked once
the type is known to be new: `CREATE TYPE ctdt AS ENUM ('a'); CREATE TYPE ctdt AS ENUM
('b', 'b')` is *type "ctdt" already exists* on both engines
(`enum/create_type_duplicate_type_name_beats_the_duplicate_label.sql`).

A label is a **string literal**, not an identifier, so it's compared as written and never
folded: `('A', 'a')` is two members on both engines. This is where the label rule and the
identifier fold ([parser.md](parser.md)) differ. The type *name* next to the labels is an
identifier and does fold, with the re-fold residual § Deferred records.

**A repeated label affects the catalog and the dump, but never the order.** The
comparator ranks a member through `enum_rank`, which is `position()`, the first match. So
a second `'b'` is never ranked, distinct labels keep distinct ranks, and
`values_compare_ordered`'s enum arm stays a total order with `Equal` ⟺ byte-equal.
`executor::expr`'s `a_duplicated_label_keeps_the_comparator_a_total_order` records this,
measured from SQL on an engine without the gate, across `ORDER BY`, `min`, `max`,
`greatest` and `least`. What a repeat does affect: `pg_enum` returns two rows for one
label, which makes this doc's own *one member is one label is one OID* premise false for
that type. And it produces a dump stock Postgres refuses, since `CREATE TYPE e AS ENUM
('b', 'a', 'b')` is exactly the text `pglet::dump` emits. The invariant that one such type
breaks is the catalog part of the dump invariant ([sequences.md](sequences.md)): *any
catalog pglet's own accepted statements build must dump to something that loads into
stock Postgres and back into pglet*. That invariant belongs to the **whole database**, so
one such type means the whole database fails it.

**The gate stops new duplicates, but it can't clean up existing state.** The persisted
shape has no mark saying the gate was present, so a store written by a build without the
gate can hold such a type, reopen, and be read through the comparator's enum arm. That's
what the test above pins. It's a pin on legacy state, not a guard for the gate: it passes
with the gate and without it.

The structured **error fields** differ here, on the omission side of
[errors.md](errors.md)'s rule. On the raw wire, 18.6 sends `S V C M D s t n F L R` and
pglet sends `S V C M n`. `F`/`L`/`R` are that rule's standing omission, because pglet has
no C source tree. The rest, `D` and `s`/`t`, have different causes:

- **`s`/`t` is a deliberate choice.** 18.6 fills them from a **catalog** relation
  (`pg_catalog` / `pg_enum`, the index being `pg_enum_typid_label_index`). pglet only
  sends `s` together with `t`, and its `s` is always `IMPLICIT_SCHEMA`, so filling `t`
  here would send a wrong `public`.
- **`D` is a gap.** 18.6's is `Key (enumtypid, enumlabel)=(<oid>, b) already exists.`.
  Its `enumtypid` is a live OID, so it isn't byte-stable across runs, but both parts can be
  derived at the gate: the label as written, and the OID the statement would take
  (`next_object_oid`, read just below the gate without advancing it). What prevents it is
  the payload shape. The error is `Error::UniqueViolation` with `key: None`, and a
  `UniqueViolationDetail` carries the relation together with the key description, so the
  shape that would fill `D` also fills `t`. Tracked in `EDGES.md` row 76.

The 18.6 *position* isn't new either. The `LOCATION` is `_bt_check_unique`, the same one
an ordinary unique violation reports; we checked both on the oracle.

## An enum's order is its members' positions, and one list says so

18.6 orders an enum by `pg_enum.enumsortorder`. pglet stores the ordered member labels on
the type (`Schema.enums[name].labels`), and any site that decides an **order** over an
enum value reads that list. A query site borrows the slice. The three sites that can't
borrow it (`IndexSpec::column_enum_orders`, `PreparedIndex`, `JoinColumnMap`) copy it,
and only the first of those outlives the statement that built it (§ A copy that outlives
its statement). There's one comparator, `executor::expr`'s `values_compare_ordered`. Its
enum arm ranks each label with `enum_rank`, which returns `Ok(position)` for a member and
`Err(label)` for a label the type doesn't hold. Rust's `Ord` on `Result` then orders
*every member before every non-member, and non-members among themselves by bytes*. That
rule has three properties no interleaving would have: it's a **total order** (which
`sort_by` requires), it can't make a readable table unreadable, and it keeps `Equal` ⟺
the labels are byte-equal.

**Equality doesn't change**, and that's correct, not an omission. 18.6's
`enum_eq`/`enum_ne` are plain OID identity (`enum.c`), and one member is one label is one
OID. So `=`, `<>`, `IN`, `DISTINCT`, `GROUP BY`, window partitioning and the
`ON CONFLICT`/FK matchers stay on the label bytes and are right to. What orders by member
position:

- `<`/`<=`/`>`/`>=`, and `BETWEEN`, which the parser desugars into that pair;
- `ORDER BY`, meaning the `Plan::Sort` node (which a `DISTINCT ON`'s effective sort and a
  set operation's trailing `ORDER BY` also lower to), plus the window's own key path;
- `MIN`/`MAX`;
- `GREATEST`/`LEAST`.

### The identity lives on the column

An enum value is a `Value::Text` and its `TypeOid` is `Text`, so nothing in the value says
which enum it belongs to. The **column** does. `Column.enum_type` names the type,
`ColumnDesc::enum_type` carries it through the executor, and
`ColumnResolver::column_enum_order` is what an expression asks. It's the same object, and
the same `resolve`, that answers for the value and for the declared type.

### An expression's enum is its type's, and one exhaustive walk says so

18.6 needs no walk at all. `COALESCE(m, 'sad')` just *has type* `mood`, by ordinary type
unification, and every site that orders it reads that. pglet has to derive the identity,
and `executor::expr`'s `expr_enum_with` does it. The rule: **a node's enum is the unified
enum of the operands its result type comes from**, and a node whose result type isn't one
of its operands' has none. So `COALESCE`, `CASE` (its `THEN`s and its `ELSE`, never its
operand or its `WHEN`s), `greatest`/`least`, `nullif`, `min`/`max`, and the
value-preserving wrappers (an alias, unary `+`) all carry the enum from below. A
comparison, a string function and a cast don't.

**An array node answers with its *element's* enum**, which the rule as stated wouldn't
give it, since `ARRAY[…]`'s result type is `element[]`, not `element`. What settles it is
what the answer is used *for*. Every consumer passes the member list to
`values_compare_ordered`, whose array arm compares element by element and reads the same
list for each element. So the array node's answer is its elements' unified enum, and one
list serves both readings. `Expr::Unnest` is the inverse and answers the same way, since
its result type *is* its operand's element type. § An `ARRAY[…]` is not an enum array
column explains why that arm is needed and what a `None` there costs.

Three properties of the walk matter:

- **The match is exhaustive, with no `_` arm.** An allowlist of a column reference and its
  value-preserving wrappers with `_ => None` lets every type-unifying kind fall through.
  Then `ORDER BY COALESCE(m, 'sad')` sorts by label bytes,
  `CHECK (COALESCE(m, 'sad') > 'ok')` admits the *opposite* row set from 18.6's with no
  error on the row it accepts, and a unique partial index over the same predicate takes a
  duplicate 18.6 refuses. With a catch-all, the next such kind is another silent wrong
  answer. With an exhaustive match it's a compile error. That's also why `deparse.rs` is
  exhaustive.
- **One walk answers for both facts that need it.** `expr_enum_with` is generic in what
  its leaf returns. The ordering sites instantiate it with the borrowed member list
  (`expr_enum_order_with`), and `expr_output_column` instantiates it with the enum's
  **name**, to fill the projected `ColumnDesc::enum_type` that an enclosing clause then
  reads. The operator and common-type gates (`docs/coercion.md` § What the gate reads)
  instantiate the same walk over a clause's namespace and over the catalog. So the
  ordering sites, the descriptor and those gates share one list of arms. Two lists would
  have to agree, and when they drift it shows as a split: if the descriptor list lacked
  the ordering list's `MIN`/`MAX` arm, `SELECT g FROM (SELECT id AS g, min(m) AS mm FROM t
  GROUP BY id) q ORDER BY q.mm` would sort by label bytes while `HAVING min(m) < 'ok'`,
  one clause earlier, is right.
- **It asks the outside world two questions, not one.** `EnumLeaves` carries a column leaf
  *and* a subquery leaf, the same pair `planner::LeafScope` carries, for the same reason:
  those are the two nodes the walk can't answer from the node and its children. Without
  the second, the `Subquery` arm has nothing to ask and answers `None`. What that costs is
  in § The boundary, and the scalar subquery, along with `EDGES.md` row 104's correction.

### A column unified from two others keeps the enum either side declares

A set operation's output column and a `JOIN … USING (m)`'s merged column are both typed
by a *unified* type, not by one source column. So both build a fresh descriptor with no
passthrough wire metadata. But the enum isn't wire metadata. It's what tells the column's
`ORDER BY` to order by member position, and 18.6 types both spellings as the enum.

The rule is `executor::expr::unify_enum_pair`, and **a side that declares no enum adds no
information; it doesn't veto**. That matches 18.6's `select_common_type`: an
`unknown`-typed literal or a `NULL` next to an enum branch resolves to the enum, so
`SELECT m FROM t UNION ALL SELECT 'ok'` is a `mood` column. Treating the `(Some, None)`
pair as a refusal would leave the merged column on the label bytes, and that's the pair
real SQL produces most (a set operation with both branches enum is the rarer spelling).
Measured over `mood`, that gave `happy, ok, ok, sad` against 18.6's `sad, ok, ok, happy`,
and `WHERE x > 'ok'` returned a different **row**, not just a different order. Two
*different* enums stay `None`, a pair 18.6 refuses to unify at all.

**That's the rule for a *pair*, and folding it over more than two isn't sticky.**
Stickiness is the one property `unify_enum` adds for an expression list, and these two
sites don't inherit it. A nested set operation and a `JOIN … USING` chain both fold
`unify_enum_pair` pairwise over already-merged descriptors. There a `None` reads as "no
enum" whether the side never had one or had one **cancelled** below, so a third branch
could bring back what the first two cancelled: `mood ∪ weather` by the label bytes,
`mood ∪ weather ∪ mood` by `mood`'s member positions, and the three-branch spelling under
`WHERE x > 'ok'` a different row set. **No statement reaches that, and no descriptor
carries a third state.** Every shape that reaches this fold carries two *different*
enums, which is the only pair `unify_enum_pair` cancels, and that pair is refused
*before* the fold. It's refused at the gates that decide whether two operands have a
comparison operator or a common type. Two enums share `typcategory` `E`, so the set
operation is `42846 UNION could not convert type weather to mood` and the merge's
equality is `42883 operator does not exist: mood = weather`, here as on 18.6, at every
branch count. The pairwise arms stay right for the ordering question they answer.
`EDGES.md` row 109, closed.

**The cancelling pair also shows up outside a merge.** Two *different* enum columns
compared or folded directly (`m > y`, `greatest(m, y)`) reach the same `(Some, Some)` arm
with no branch count involved, and the answer falls back to the label bytes. Over
`mood AS ENUM ('sad','ok','happy')`, `m3 AS ENUM ('sad','happy','ok')` and rows
`('ok','sad')`, `('ok','happy')`, `('ok','ok')`, the bytes give `SELECT (m > y)` as
`f, t, f` (where `mood`'s positions give `t, f, f` and `m3`'s give `t, t, f`), and
`greatest(m, y)` as `sad, ok, ok`. The same gates refuse both, as 18.6 does:
`42883 operator does not exist: mood > m3` and
`42846 GREATEST could not convert type m3 to mood`. That `42846` isn't the `text` pair's
`42804`. With two enums, 18.6's `select_common_type` succeeds by taking the first branch's
type, and it's `coerce_to_common_type` that fails. Without the gate, the pair writes, in a
`DELETE` predicate and in a `CHECK` or partial index. `EDGES.md` row 115, closed.

Where this rule alone couldn't follow 18.6: an enum against a plain `text` **column**
reads `None` on that side too, the same as a literal, and 18.6 refuses that pair outright
(`UNION types mood and text cannot be matched`, `operator does not exist: mood = text`).
Left to this rule, pglet would answer either way (by the member positions under it, by
the label bytes without it), and a `CHECK` or a partial index over `greatest(m, t)` would
persist that answer. `EDGES.md` row 114, the `enum <op> text` family, is **closed** by
passing the identity to the comparison and common-type gates (`docs/coercion.md` § What
the gate reads). The pair is `42883` in the comparison positions. Inside a `COALESCE`,
`CASE`, `greatest`/`least` or `nullif` it's 18.6's
`42804 GREATEST types mood and text cannot be matched` (and `nullif`'s own
`42883 operator does not exist: mood = text`), on both engines. The identity doesn't reach
the jsonb key/path, `LIKE`-family, `starts_with`, `||` and array-containment rules, or the
string functions `EDGES.md` row 190 measures. There an enum is read as the `text` of its
label, so `m LIKE t`, `j ? m`, `upper(m)`, `m || m` and `ARRAY[m] @> ARRAY[t]` return a
result where 18.6 is `42883`, and they write.

**One rule.** `unify_enum_pair` is also what `unify_enum` (the expression list) and
`executor::helpers::unified_enum_type` (a merged column) read. So a comparison, a
`COALESCE` and a merged column can't disagree about what an untyped side means. A grouping
query's `HAVING min(m) < 'ok'` is the scalar walk's comparison, with the grouping lending
that walk its aggregates' values. Written out separately per site, the rule drifts.

**Two shapes, four sites.** Each shape's descriptors are built twice, once by the
executor's node and once by `Plan::output_columns_scoped`, and neither reads the other.
Re-derive that list by grep, not from this sentence. If a pair drops the enum because a
merged column carries "no single-column wire metadata", `ORDER BY m` over the merged name
sorts by label bytes and `WHERE m > 'sad'` over it matches nothing at all.

**Both halves of both pairs have a reader.** The planner's half is read by a **scalar
subquery** over a shape with no passthrough column. Setting the planner set-op arm to
`None` fails `enum/subquery_over_set_op_member_position`, and the planner `USING` arm
fails `enum/subquery_over_join_using_member_position`. The executor halves fail the
`set_op_*` and `join_using_*` families. Each of the four arms has a fixture that tells it
apart.

**Both *computed-projection* halves of the pair derive the enum.** The executor's
`expr_output_column` derives a projected expression's enum onto its descriptor, and
`Expr::to_column_desc`, the planner's, derives it too. If the planner half didn't derive
it, the cost would depend on the reader. The **ordering** readers take its `None` to mean
*order by the label bytes* (`EDGES.md` row 107), and the identity **gates** take it to
mean *not an enum*. Five namespaces they read a derived relation through are built from
that half: a derived table's and a CTE's scope entries, an `UPDATE … FROM` source's,
`Plan::output_columns_scoped`'s relations, and `executor::subquery_enum_name`. So
ordinary SQL at all five would be refused `42883`. The derivation lives in
`to_column_desc` rather than in the namespaces because the namespace list is the one that
goes short, while the site that builds the descriptor is a single place.

**Those five aren't all of them.** When a CTE does recurse, its relation is built by neither
producer. `parser::select`'s `recursive_cte_columns` **rebuilds** the seed's descriptors
to apply the `name(cols)` list, and a rebuild through `ColumnDesc::new` alone answers
`None` whatever the seed derived. The working table's column then orders by its label
bytes (a wrong row set, not just a wrong order), and its comparisons with a real enum
column are `42883`, as measured. That's why row 114 lists it separately from a plain CTE.
It's a *third* producer of the computed-projection class, and a derivation can't fix a
namespace it doesn't build. So the question to ask of each namespace is **who builds
it**, not whether the list is complete. So that rebuild copies the seed's identity,
field by field, and deliberately drops `table_oid`/`column_id`, which 18.6 also sends as `0`
here.

### The boundary, and the scalar subquery

A **cast's** identity is erased where the parser reads the type name. A member literal's
cast is built as the plain cast to `text`, so `'happy'::mood` is byte-identical to
`'happy'::text` in the plan and no walk can recover it. Any other operand's cast is a
membership test, which the walk doesn't read (§ A cast to an enum converts), under that
same cast (`EDGES.md` row 104). That's the boundary.

**And the boundary writes**, which it has in common with the `enum <op> text` family next
to it. What separates them isn't which row a spelling is filed under but **the operand
next to the cast**. The pairs are listed as measured instead of as a rule over neighbour
kinds, because 18.6's verdict, pglet's answer and pglet's *rule* each vary with the
neighbour, and not together.

- 18.6 returns a result next to a bare `unknown` literal, which it resolves to the cast's
  own enum (`pg_typeof(greatest('sad'::mood, 'ok'))` is `mood`), next to a second cast to
  the **same** enum, and with no neighbour at all (`ORDER BY m::mood`). It orders by the
  member positions, while pglet orders by the label bytes, and there's no refusal to set
  against pglet's answer.
- 18.6 refuses next to a `text` operand: `'happy'::mood > 'sad'::text` is `f` here and
  `42883` there. That's row 114's over-acceptance reached through a cast, and it's still
  answered here by the label bytes, not by that row's member positions.
- 18.6 also refuses next to a **different enum**: `'ok'::mood > y` over an `m3` column is
  `t, t, f` here (`m3`'s member positions, not the label bytes) and
  `42883 operator does not exist: mood > m3` there.

`unify_enum_pair`'s `(Some, None)` arm is why those last two split. The cast's erased
side yields to whatever the neighbour declares, so the label bytes are only what's left
when neither side declares an enum.

The erased operand reaches a `DELETE` predicate, an `UPDATE … SET` right-hand side, and
each of the four persisted-expression positions `docs/ddl.md`'s render probe runs at: a
column `DEFAULT`, a `CHECK`, an index predicate and an index **key** expression. Both
engines accept the DDL there, while the cast-free `CHECK` over `greatest(m, t)` is
`42804` on both. A written cast of a **member literal** at any of the four renders as
`::text` in the dump, which stock 18.6 refuses with `42804` or `42883` (which one depends
on the statement, not the position). So the database fails the catalog part of the dump
invariant, *any catalog pglet's own accepted statements build must dump to something that
loads into stock Postgres and back into pglet*. The control is a bare-literal spelling at
each of the four positions, which dumps and replays cleanly, as measured. `EDGES.md` row
104 has the measured routes with the statement and code for each (some report 18.6's own
tag and differ only in what's stored), and the restore's per-process silence. A cast of
any other operand renders as the enum cast it is (§ A cast to an enum converts), so
`CREATE INDEX … ((m::mood))` dumps as `((m)::mood)` and restores.

A **scalar subquery** isn't a second boundary. A walk that resolves a column reference
through its leaf and holds no plan output descriptors compares `(SELECT m FROM u) < 'ok'`
by the label bytes, the reverse verdict for `mood`. `ORDER BY (SELECT …)`, where a scalar
subquery is refused, is `0A000` here ("scalar subquery requires execution context")
because that site evaluates with no subquery callback, which has nothing to do with
enums. What gets refused is that **class** of evaluation, and the class isn't a list of
SQL positions and isn't specific to enums. It's every evaluation that has no subquery
callback of the kind the node needs, which `grep SubqueryExec::NONE` finds and `EDGES.md`
row 89 describes. Its members include `GROUP BY (SELECT n …)` and
`INSERT … VALUES ((SELECT n …))`, which carry no enum at all. Read the row for the
derivation. The other examples in this file carry enums because enums are this file's
subject, not because the class is about enums. Reachable, and silent under a blind leaf
(measured): a projected comparison, `WHERE`, `BETWEEN`, `COALESCE`, `CASE`,
`greatest`/`least`, subquery-against-subquery, and the two that write, a `DELETE` and an
`UPDATE` predicate. There `DELETE FROM t WHERE (SELECT m FROM u) < 'ok'` removes
**every** row where 18.6 removes none, and the `UPDATE` spelling reports `UPDATE 3`
against `UPDATE 0`, with no error on either engine.

So the walk asks for a subquery's enum the same way it asks for a column's. `EnumLeaves`
is a **pair** of leaves, and the second is the enum of the plan's first output column
(`subquery_enum_order` / `subquery_enum_name`). Things to keep in mind:

- It reads the enum from the **planner-built** descriptor (`Plan::output_columns`), as
  `planner::coercion`'s `resolved_subquery_type_in` also does.
- It only sees what that descriptor carries. A subquery projecting a *passthrough column*
  carries the enum because the projection copies the whole descriptor. One projecting an
  **expression** (`SELECT min(m) FROM u`, `SELECT COALESCE(m, 'sad') FROM u`) carries it
  because `Expr::to_column_desc` **derives** it, the planner's side of the
  computed-projection pair above (`EDGES.md` row 107). That derivation's own subquery
  leaf declines, for a cost reason measured both ways at
  `PlannerScope::expr_enum_shallow`. So a projection that *is* a scalar subquery
  (`SELECT (SELECT min(m) FROM u) FROM u`) is still ordered by label bytes. So is the
  **no-FROM** spelling of that nesting, because the `Values` arm only derives an identity
  from a column held in its record of enclosing references. That's the residual of that
  row.
- **The `ORDER BY` key site and the window's order-key site pass the leaf along but can't
  observe its answer**, and the code says so where a reader will find it.
  `executor::query`'s `order_key_enum_order` and `window.rs`'s `compute_window_column`
  both take the catalog only to answer this leaf, and a scalar subquery in either
  position is `0A000` before any comparison happens. This keeps them consistent by
  construction; it isn't dead code. A blind leaf at either site would be a second walk
  with a different shape, and it would silently answer wrong the day row 89's class is
  closed.
- **It's only asked where its answer can be read.** The comparator's enum arm needs both
  operands to be `Value::Text`, so the ordered-comparison sites call
  `operand_can_be_enum` on the values they've already evaluated before walking at all.
  That's a cost rule, not a semantic one. The leaf costs a `Plan::output_columns` per
  evaluation, which for a comparison operand is per row. Without the guard, an `int`
  comparison against a subquery, with no enum anywhere in the statement, takes 38.0 ms
  against 29.9 ms with it, and a bare `i > 5` 5.21 ms against 3.82 ms (release, 20 000
  rows, medians of three).

A subquery *next to* an enum column in a comparison is answered by the sibling, through
`operands_enum_order`, which makes the shape look narrower than it is.

### An `ARRAY[…]` is not an enum array column

*Enum arrays have no column form here* is **true of a column, but it says nothing about
the node**. `ms mood[]` is `type "mood" is not supported` on this engine (§ Deferred),
while `ARRAY[m]` is an ordinary expression pglet builds, orders, compares and writes
through. So the `Expr::Array` / `Expr::Unnest` arms answer the elements' enum. A `None`
there, inferred from the column form, would leave an array comparison on the label bytes:
the same problem a `_ => None` allowlist causes for `COALESCE`, one type constructor up.

**Three readers had to change together, and each one alone changes nothing:**

- the walk's arm (the elements' unified enum);
- `values_compare_ordered`'s array arm, which compares each element pair under the
  caller's order (the *unordered* `values_compare` there throws away any order the caller
  resolved);
- `operand_can_be_enum`, which admits an array whose elements could be labels. Without it
  the widened walk is never asked.

That last one is a **gate**, and its contract goes one way: a superset is safe and an
undercount isn't. So it recurses into the elements with the same `Value::Text` test the
arm reads. An array with no `Value::Text` anywhere (empty, or all-NULL) can't reach the
arm at all, since the element loop decides those by the NULL rule or by comparing
lengths. The recursion costs time on the same axis the gate saves it: an ordered `text[]`
comparison walks where a non-recursing gate would skip. **The cost is recorded as
measured pairs, not as a range over them.** `operand_can_be_enum`'s own doc has the
pairs, next to the ones that justify the gate. A range would be wrong here: the two
`text[]` shapes share a sign but not a magnitude, and the fraction a ratio gives changes
with the choice of denominator.

Those three plus a **fourth** close `EDGES.md` row 111: the `AnyAll` arm passes both
operands, as `BinaryOp` does. The fourth alone changes nothing. `ARRAY[m]` is the operand
carrying the identity, and only the walk's `Array` arm answers for it. With the `AnyAll`
arm widened, reverting only the `Array` arm leaves the `ANY` spellings wrong (measured).

The `text`-side identity follows the element rule. `ARRAY[m] < ARRAY['ok']`, with bare
literals rather than casts on the other side, is `operator does not exist: mood[] <
text[]` on 18.6 and here. The comparison gate reads a computed operand through the full
type walk wherever the pair names an enum, and it types both sides of this one as
`text[]`. What decides is `erases_enum_identity`'s array arm, which doesn't let a bare
string or `NULL` literal element make the array undecidable. 18.6's `transformArrayExpr`
folds the elements through `select_common_type`, and an `unknown` element contributes
nothing to that fold. So an array's identity is the unification of its **typed**
elements, which is also what `expr_enum_with`'s `Expr::Array` arm computes.

It decides the **accept** side too, which makes it a decision in its own right:
`ARRAY[m, 'ok']` is a named `mood[]` and `ARRAY[y, 'ok']` an `m3[]`. It also closes the
same pair inside a fold: `COALESCE(ARRAY[m], ARRAY['ok'])` is
`42846 could not convert type text[] to mood[]` on both engines, next to the cast spelling
`COALESCE(ARRAY[m], ARRAY[t])` that `concrete_branch_type`'s `Array` arm refuses. What's
left is a `NULL` next to a typed element. This engine casts it to the fold's target
before any gate reads it, so it can't tell it from a written `NULL::text`; that's row
104's. The spellings measured to diverge are the ones where a written cast next to an
enum is read as that pass's output: `ARRAY[m, NULL::text]` returns a result where 18.6
raises `42804`, and `ARRAY[s, NULL::varchar(3)]` (a `CastTypmod`, which the exclusion
doesn't match at all) returns a result where 18.6 raises `42883`.

All three write paths reach the array comparison, and on the label bytes each one is
silent. Measured against 18.6 over `mood AS ENUM ('sad','ok','happy')`:

- a `DELETE` under `ARRAY[m] < ARRAY['ok'::mood]` reports `DELETE 1` on both engines but
  deletes a different row;
- a `CHECK (ARRAY[m] < ARRAY['ok'::mood])` admits the opposite row set with no error on
  the row it takes;
- a `CREATE UNIQUE INDEX … WHERE ARRAY[m] > ARRAY['sad'::mood]` holds a duplicate 18.6
  refuses.

What this **doesn't** reach is a FROM-item `unnest`'s own column (`EDGES.md` row 112).
`execute_unnest` and `Plan::FunctionScan`'s `output_columns_scoped` both build that
descriptor with `ColumnDesc::new` and derive no enum. The only spelling that puts a
recoverable enum into such an array is a **correlated** column reference, and its
identity reaches the executor as a member *order* on an `OuterFrame`, never as the name a
`ColumnDesc` would need. The cast spelling (`unnest(ARRAY['happy'::mood, …])`) falls
under row 104's boundary, not that descriptor, and the scalar-subquery spelling is
`0A000` in that position (row 89's class).

### An index over an enum column is declined

The B-tree key encoder has no enum arm. It encodes a label's bytes like any other
`Value::Text`, so an index over an enum column is ordered by something the engine doesn't
order by. `planner::index_selection`'s `single_column_index_for` declines an enum key
column, which removes it from all three of its consumers at once (equality, range, and
the `ORDER BY` `Sort` elision). The cost is **performance** only: the row filter is
exact, and equality over the label bytes is still enum equality. Encoding the member
position in the key instead would be a persisted-shape change. 18.6's own
`pg_enum.enumsortorder` is a `float4` precisely so `ALTER TYPE … ADD VALUE …
BEFORE/AFTER` can insert between two members without rewriting them.

### The two write paths, and the one that is not a query

A `CHECK` over an enum column is a stored expression re-evaluated on every write, so the
member order decides which rows land. `CHECK (m < 'ok')` keeps the members declared
before `ok` and refuses the rest, and for `mood` the label bytes would keep and refuse
*exactly the opposite* set. `enforce_checks` takes the orders alongside the declared
types it already took. Re-derive its call sites by grep rather than from a count here.
They cover the `INSERT` path; the shared `enforce_row_checks` (`UPDATE`, and through
`enforce_row_constraints`, `COPY` and `ON CONFLICT DO UPDATE`, plus an `UPDATE`'s FK
actions' `SET NULL` and repoint writes); a `DELETE`'s FK actions' `SET NULL` images
(`ActionState::rewrite`); the `ADD COLUMN` back-fill; and the `ADD CONSTRAINT … CHECK`
existing-row scan.

The second is a **partial index whose predicate orders an enum**. Its membership is
decided in two places that nothing compares: the executor's `PreparedIndex`, rebuilt from
the live catalog once per statement, and the storage layer's `IndexKeying`, which holds no
catalog at all and reads the orders off the `IndexSpec` (`column_enum_orders`, next to
`column_types`). If they disagree, the result isn't a wrong row order but a silent wrong
**write**: a *unique* partial index that holds neither of two rows reports no conflict
for a duplicate 18.6 refuses.

### A copy that outlives its statement

`IndexSpec::column_enum_orders` is the only copy of the member order that outlives the
statement that took it. It's built at `CREATE INDEX` and on reopen, and it's modelled on
`IndexSpec::column_types`. But `column_types` is safe to keep for a reason that doesn't
apply here: a column's declared **type** can't change in pglet (`ALTER COLUMN … SET DATA
TYPE` is `0A000`), while an enum's **member list** can. `DROP TYPE` under a live
dependent column is `2BP01` on 18.6 and accepted here, which is its own gap (`EDGES.md`
row 261). So `DROP TYPE` then `CREATE TYPE` with the members reordered changes what the
copy froze. Since the member order matters, that leak produces the silent wrong write
above. Measured over `r(id, k, m mood NOT NULL)` with
`CREATE UNIQUE INDEX r_u ON r (k) WHERE m > 'ok'`: a reorder against a frozen copy leaves
the executor on the live order and storage on the frozen one, and two rows sharing `k`
both land. This is the repo's own rule ([sequences.md](sequences.md)): **recording a
persisted cross-object reference turns every pre-existing leak of the referenced object
into a bug.**

The rule that fixes it: **the storage index specs follow `Schema.enums` wherever it
moves.** It's one operation, called from five sites in two directions. The backward
sites are exactly `Schema::restore_from`'s own call sites, so re-derive them from there
rather than from this sentence:

- `execute_create_type` and `execute_drop_type` go forward, through
  `rebuild_indexes_over_enum_staged`, over the indexes whose *table* has a column of that
  enum. `column_enum_orders` is table-wide, so that's exactly when the spec's bytes
  change. A narrower "does the predicate mention it" test would be a second derivation of
  the same fact. `DROP TYPE` counts because a column left declaring a dropped type orders
  by label bytes, which is a change like any other.
- Every **backward** `Schema::restore_from`: `TransactionSession::rollback`,
  `::rollback_to_savepoint` (those two through one `resync_index_specs`), and the `DO`
  block's undo in `executor::mod`, which runs for a caught body before its handler and
  for a `DO` statement that fails. A restore moves the enums back, which the forward
  sites can't see, so the forward direction alone leaves this case open. It only runs
  when the enum maps differ, computed before the restore
  (`executor::enums_moved_by_restore`, the one derivation all three read). So an ordinary
  `ROLLBACK`, which every fixture runs, re-derives nothing.

  **That difference has two directions, and the second isn't a reorder.** A name the
  *snapshot* holds and the live catalog doesn't is also a change. That's a `DROP TYPE`
  inside the block, whose forward rebuild moved the specs to the label bytes, and it's a
  different arm of the derivation from the reordered-name one. A script that changes the
  enum by *reordering* it only reaches the **first** arm. With the second arm deleted,
  every reorder test stays green while `BEGIN; DROP TYPE mood; ROLLBACK;` takes a
  duplicate under a unique partial index, with no error on any statement (measured).
  Pinned by `crates/pglet/tests/enum_reorder_index_specs.rs`'s
  `a_rolled_back_drop_type_with_no_re_create_leaves_the_specs_on_the_restored_order`, and
  directly by `pglet_sql`'s `enums_moved_tests`.

  The `DO` handler is the site that's easy to miss, and without the resync it gets out of
  sync silently. A `DROP TYPE` + re-`CREATE TYPE` reorder inside a caught body moves the
  catalog and the specs forward. The handler puts the catalog back, the specs keep the
  block's order, and a unique partial index over `WHERE m > 'ok'` then takes a duplicate
  with **no error on any statement**. It needs no second connection and no refused key
  type. 18.6 isn't the oracle for it, since its own `DROP TYPE` under a live dependent
  column is `2BP01`. What the script violates is this engine's own invariant that its
  catalog and its index specs agree. Pinned by
  `crates/pglet/tests/enum_reorder_index_specs.rs`'s
  `a_caught_do_block_leaves_the_specs_on_the_restored_order`, with the
  enum-untouched-body control next to it.

The two **forward** sites stage. They pass the rebuild the enum map they're about to
install and only write `Schema.enums` once it returns. The rebuild can fail and this is
DDL, so the repo's standing rule applies: *a failed DDL statement must leave the catalog
untouched*, because the autocommit path can't put it back. And this failure is
reachable. If a key can't be derived for a row the change newly admits, `rebuild_index`
raises. For example, an expression key `(10 / d)` over a row holding zero is `22012`
there, since the key is only evaluated for an admitted row. If the catalog were written
first, `DROP TYPE` and the re-`CREATE TYPE` would both report that error and keep the
change, leaving a type whose members no statement claims to have set. The **backward**
site doesn't stage, and that's correct: there the catalog write is the intended outcome,
not a step that has to be undoable.

Staging the catalog is only **half** of it, and on its own it produces the same desync
the other way round. The rebuild loop is a second thing being mutated, and an index it
has already moved sits on the staged order while `Schema.enums`, held back, doesn't.
Measured without the unwind, over two tables using the same enum, with a unique partial
index on the first and a partial index whose key raises for the re-admitted row on the
second: the failed `DROP TYPE` lets two rows sharing the unique key both land, where the
same script without it raises `23505`. Which way it goes depends on `Schema.indexes`'
`BTreeMap` iteration, which is **lexical index-name order**. Wrapping it in a block
doesn't fix it, because a catalog that staging never moved gives `enums_moved_by_restore`
nothing to resync. So the loop **unwinds**: both orders are derived before the first
mutation, and on failure the indexes already moved are rebuilt back onto the order they
came from. The failing index itself needs no unwinding because of
`Storage::rebuild_index`'s **all-or-nothing** contract, which the in-memory backend keeps
too. A rebuild that dropped the old definition before building the new one would delete
the index from storage on a refused key encoding while the catalog still held it.

**The unwind helps at the two forward callers, as measured, and does nothing at the
rollback one, also as measured.** At `CREATE TYPE` / `DROP TYPE` its `from` order is the
pre-statement map, so a failed rebuild puts the moved specs back (the two-index script
above, pinned in § Test coverage). At `TransactionSession::rollback` two things combine.
The restore has already written the catalog (`Schema::restore_from` runs, then
`resync_index_specs`), so nothing was staged. And the unwind's `from` order is then the
*destination*, so it re-derives the same bytes the failure left. We measured this through
a route the writer closes (below), on the default B+tree backend, native server, two
connections, over the same two-table shape. The failing index was keyed on a value the
encoder refused (a `float8` under an encoder without a float arm) and named to sort
**first**. The rollback's resync raises on it, the catalog stays on the restored order,
the unique index's spec keeps the order the block installed, and a duplicate that index
should refuse lands: `(2 rows)`, where the same script without the failing index rolls
back cleanly and answers `23505`. The failing index's own spec keeps the block's order
too, shown by a later insert that the restored order puts outside its predicate and the
block's order puts inside. `ROLLBACK TO SAVEPOINT` raises the same way from its own call,
in its own method. Recorded as `EDGES.md` row 110: that route is closed, no other is
known, and the mechanism remains.

The **shape** applies to every backward site, the `DO` handler included: none of them
stages, so none has anything to put back. The **reach** has only been measured at the
two `TransactionSession` sites, and the difference is where the rows come from. Reaching
it takes a row that satisfies a partial index's predicate under the order being restored
*to*, while carrying a key that can't be derived. The two spellings that build that pair
directly both raise up front: an `INSERT` of such a row, and a `CREATE INDEX` over a table
already holding one (both measured). What's left is changing the *order* under an
existing pair. Row 110's script did that from a second connection while the block held
the changed order. Such commits wait for the block, which holds the writer (the
embed refuses them `55P03`), so no route is known. A `DO` block's undo has no such window.
A `DoBlock` is never read-only, so the native server runs it under `lock_both()` for the
whole statement, and the embed and wasm hosts run a statement behind `&mut self`, so a
second logical connection there (`Database::execute_as`) can't interleave one. Its own
body rows are discarded before the restore, so what it restores to is the state at body
entry, the pair storage already held, which `rebuild_index` re-derives from
`(spec, current rows)`.

The closest route we tried fails a step earlier still. Take a partial index whose key
raises for its row (a key the encoder refuses, measured with a `float8` key under an
encoder without a float arm) and whose predicate a reorder inside the body newly
satisfies. The body's *forward* rebuild raises and stages, so the handler restores a
catalog that never moved, `enums_moved_by_restore` finds nothing, and the unique index
that follows still refuses its duplicate (measured). No route has been constructed at
that site.

A write still **pending** in the transaction that changes the enum is outside what the
rebuild reads (it re-derives from committed rows), and that isn't a hole. The pending
write is materialized at commit through `emit_index_ops` against the `index_defs` the
rebuild has already replaced, so it lands under the new rule. Pinned, over orders where
neither the pre-change order nor the label bytes can stand in for the destination's.

The operation is `Storage::rebuild_index`, and neither of its two neighbours would work;
we measured both as wrong here. `reattach_index` swaps the definition and keeps the
entries the old rule placed, so a row stored *before* the change keeps a membership the
live rule disagrees with, and on the in-memory backend it drops every entry outright (its
defs own their entries). `drop_index` + `create_index` rebuilds correctly on that backend
but **not** on the B+tree. There a drop taken inside a transaction (which the autocommit
path opens too) is deferred to commit and deregisters the definition the create had just
restored, which shows up as `index not found` on the next write.

## Test coverage

- Engine cases in `crates/pglet_sql/tests/engine_ddl.rs` (enum roundtrip + member
  validation).
- Differential fixtures vs real PG under `tests/fixtures/enum/`. Read the directory for
  the list rather than a list here.
  - The **membership** family: default + explicit member, equality filter, invalid
    insert/update, cast-to-enum, update-to-member.
  - The **cast-conversion** family (`cast_*`, `bare_non_member_*`,
    `drop_type_under_a_converting_check_loud`, and `params/enum_cast_text_param_*`) takes
    each spelling and operand kind to each place a cast is read, compared, written or
    stored. The zero-row accepts (`cast_text_operand_zero_rows_accepts`,
    `cast_column_zero_rows_accepts`) are the controls showing that a non-literal is
    converted on the value and not at plan time. The undeclared `$N` half can't be in the
    corpus by construction (`tests/README.md` § `-- @param`). It's covered by
    `crates/pglet/tests/enum_identity_gates.rs`, `pglet_server`'s Bind test, and the wasm
    and npm suites. `executor::expr`'s `enum_in_tests` pin the node's shapes and the value
    rule, and `pglet`'s dump round-trip corpus pins its rendering.
  - The **member-order** family names, in each file, the site it reaches, so
    `ls tests/fixtures/enum/` is the coverage list, and a site with no fixture shows up as
    a name that isn't there. Read three of them as **accept-side** pins, not divergence
    locks: `order_by_text_cast_is_lexical` (a cast away from the enum is still ordered by
    label bytes, on both engines), `order_by_alphabetical_enum_unchanged` (the spine's own
    `cell_visibility`, whose declared order *is* its lexical one, which is why its
    traffic can't show the divergence), and `equality_indexed_unaffected` (declining the
    index costs an equality nothing but speed). `partial_unique_index_member_position`
    and its `_excludes_` sibling are the same pair for the write side.
- The comparator's own rules are in `executor::expr`'s `enum_order_tests`: the total
  order, `Equal` ⟺ byte-equal, the non-member arm, two enums over the same labels, and the
  pairing between `operand_can_be_enum` and the arm that reads a member order. (That's the
  rule, not its wiring: a call site that dropped the gate would cost time and fail
  nothing.) The index decline as a **plan shape** is in `planner::tests`'
  `enum_indexed_col_is_declined`, with its
  `text_indexed_col_beside_an_enum_one_still_accelerates` control. The stored-non-member
  case and the two DDL-time `CHECK` validation paths are in
  `crates/pglet_sql/tests/engine_ddl.rs`.
- The **node rule**, arm by arm, is in `executor::expr`'s `expr_enum_walk_tests`. What it
  covers that no fixture can is the arms whose whole content is a `None`: the cast
  boundary, and every node whose result type isn't its operands'. A fixture compares two
  answers, and 18.6 is *right* where pglet declines, so over-widening the walk (giving
  `Cast` an arm) would silently change what `EDGES.md` row 104 records and fail nothing.
  The `Subquery` arm's two tests are there for the opposite reason. They pin that the arm
  **asks** its leaf, with a deliberately blind leaf next to them, so an arm that
  hard-codes an answer fails them too. The `Array` / `Unnest` pair's tests cover the two
  things a fixture is weakest on: that *every* element is unified, not just the first (a
  mixed cast-then-column array, which the single-element spelling can't tell apart), and
  that an empty array is deliberately `None` rather than a guess.
- The **array comparison** is in `executor::expr`'s `enum_order_tests`: two arrays compare
  element by element against one member list. Each assertion is paired with the same call
  passing `None`. Without that, an element loop that dropped the order would pass, because
  the label bytes happen to agree with the member positions on some pairs. Its sibling
  pins the **gate**'s array recursion over the shapes that can't reach the arm (empty,
  all-NULL, non-text), each under the gate's own contract: *a pair the gate rejects must
  not read the member order.*
- The `ARRAY[…]`-expression family under `tests/fixtures/enum/`: the filter, the range
  filter, the two `ORDER BY`s (one of them the mixed-element discriminator), the `DELETE`,
  the `CHECK`, and the unique partial index. Plus `any_all_array_side_member_position`,
  `any_all_scalar_side_member_position` and `delete_over_any_array_side_member_position`
  (the second is the accept-side control that agrees on both engines), and
  `order_by_unnest_of_array_member_position`, the only spelling in the corpus that reaches
  the `Unnest` arm. Breaking any one of the `Array` arm, the comparator's element call, the
  gate, the `AnyAll` walk's width or the `Unnest` arm fails a different subset of that
  set. The `Array` arm reading its first element instead of unifying is caught only by
  the mixed-element fixture and the walk test. `EDGES.md` row 111 has the subset for each
  mutation.
- The **unification rule**'s own arms are in `executor::helpers`'s
  `unified_enum_type_tests`. The reachable `(Some, None)` pair is held by fixtures
  (`enum/set_op_*`, in both operand orders). The two-different-enums pair is held by
  fixtures too (`enum/two_enums_setop_loud.sql` and its siblings), since the gate above
  this arm refuses it as 18.6 does. What only these unit tests can hold is the **arm's
  own** verdict below that gate, which no statement reaches. Measured: a version that
  always returns the **left** side fails two of these tests *and*
  `enum/set_op_literal_branch_on_the_left_member_position`, which is why that fixture's
  mirror image exists instead of just one of the pair.
- The `IndexSpec` resync is in `crates/pglet/tests/enum_reorder_index_specs.rs`. It isn't
  a fixture because of the **harness**: 18.6 refuses to enter the state (`2BP01`), so a
  differential fixture's `@setup` fails on the oracle side and that becomes the fixture's
  whole result. There's no answer to compare against. What each test covers:
  - **B+tree, forward and backward**, plus the unrelated-type and same-member-list
    controls, the `DROP TYPE`-without-re-create route, and the expression-key route.
  - **The caught `DO … EXCEPTION` handler**, the third backward `Schema::restore_from`.
    It fails with the resync deleted, with the resync moved *before* the restore, and
    with `enums_moved_by_restore` read *after* it (where the two maps are already equal
    and it finds nothing), as measured. The enum-untouched-body control next to it is an
    **accept-side** pin, not a further separation. A blanket resync over every enum in the
    snapshot leaves it green (measured), because re-deriving an unchanged spec produces
    the same bytes. So what the `moved` comparison saves *in behaviour* is work, which no
    assertion in that file reads. Its **output** is a separation, tested elsewhere: the
    `enums_moved_tests` entry below.
  - **A failed `DO` with no handler**, the same undo reached by a statement's failure.
    `a_failed_do_block_leaves_the_specs_on_the_restored_order` reads the restored member
    list and then the `k = 1` collision. It fails with the undo deleted (on the member
    list) and with only the resync deleted (on the collision), as measured.
  - **The `DROP TYPE`-only rollback**, in
    `a_rolled_back_drop_type_with_no_re_create_leaves_the_specs_on_the_restored_order`.
    That's the derivation's *second* arm, a name only the snapshot holds. It reads the
    restored member list as its premise before asserting anything, and checks *which*
    error the duplicate raises, not just that there was one. Its **entries** arm is the
    row stored before the block. Its `k = 9` pair inserts both rows after the
    `ROLLBACK`, so the restored *definition* alone decides it and a def-only backward
    resync passes it. The pre-block row's entry is different: the forward `DROP TYPE`
    took it out of the index and only a re-derivation puts it back, so that's what such a
    resync fails. With `rebuild_indexes_over_enums` reattaching instead of rebuilding,
    that arm fails, as measured.
  - **In-memory, forward**: the backend whose defs *own* their entries, where the two
    rejected neighbours fail differently. `Storage::rebuild_index`'s **all-or-nothing**
    contract applies to that backend too, and is pinned at the storage level instead of
    here (`pglet_storage`'s `a_failed_rebuild_leaves_the_index_standing`), because a rebuild that
    refuses a key can never afterwards hold a row, so an index dropped by a non-atomic
    rebuild can't be observed through SQL.
  - **In-memory, backward** is a *behaviour* pin, not a guard on the resync. As measured,
    it stays green with the rollback resync deleted, because that backend restores the
    whole store and the spec comes along with it. The B+tree rollback test is the one
    that shows the resync is needed, since it does fail.
  - **A write pending across the reorder**, with orders chosen so that neither the
    starting order nor the label bytes the interim `DROP TYPE` falls back to can stand in
    for the destination's.
  - **The staging**: a `CREATE TYPE` / `DROP TYPE` whose rebuild *fails* leaves the
    catalog untouched. The failure comes from a key expression, `(10 / d)`, that raises
    for a row admitted by a predicate the change newly satisfies. Without staging, both
    statements report that error and keep the catalog change. Both of those build only
    **one** index, so they can't reach the partial-rebuild split.
  - **The unwind**, which needs two indexes: a unique partial index on one table and the
    `(10 / d)`-keyed one on another, named so `Schema.indexes`' `BTreeMap` order puts the
    unique one *first*. That's deliberate, and the reversed-name sibling next to it is the
    control that shows it (it answers correctly with or without the unwind). It runs on
    the B+tree backend, in memory, and inside a `BEGIN … ROLLBACK`, since the rollback
    isn't the fix.
  - **The OID counter**: a `CREATE TYPE` whose rebuild fails must not have advanced
    `next_oid`, checked through the `pg_class` OIDs of two tables on either side of it.
    It can't see a counter that was never advanced at all, which `next_object_oid`'s
    re-seed from the catalog max hides. That's a different rule, and it's unguarded.
- The **restore derivation itself** is in `executor::ddl`'s `enums_moved_tests`. It's the
  only test in the workspace that reads `enums_moved_by_restore`'s *answer* instead of a
  write downstream of it. That's needed for the **blanket** mutation: replacing the body
  with `snapshot.enums.keys().cloned().collect()` is correct but slower, so nothing writes
  differently. Measured over `cargo test --workspace --no-fail-fast`, exactly one test
  fails, this one. It also fails when the second arm is deleted, alongside the
  behavioural guard above.
- The un-accelerated key-scan fallback's enum arm is in `executor::query::scan`'s
  `enum_key_fallback_tests`, over a row stub. Nothing else in the repo executes that arm.
  We measured this by forcing the fallback's order to `None` and finding exactly those two
  tests failing across the fixture corpus and the whole `cargo test` workspace. No plan
  can reach it while `single_column_index_for` declines an enum key column. An arm that's
  never executed isn't tested, which is the whole reason those tests exist (§ An index
  over an enum column is declined).

### Harness note (not a pglet bug)

**A fixture must not project a bare enum column, or an enum-valued expression under an
unaliased name.** Three things block it, and only the first is about pglet:

- pglet's `RowDescription` declares the column `text` (OID 25) where 18.6 declares the
  enum's own OID, so the harness's **declared-type** comparison differs on both legs.
- The binary leg's decoder has no arm for a custom enum OID. Its fallback is an
  `UndecodedType` whose `accepts()` is `false`, so the read is a loud `CLIENT-FAULT` on
  the Postgres side.
- `row_to_json` keys its map by **column name**, and the two engines name an unaliased
  enum expression differently (`SELECT 'happy'::mood` is `mood` on 18.6 and `text`
  here).

So the value-returning fixtures project `<col>::text AS <label>`, and the label matters.
An *unaliased* `m::text` is still named `m`, so an `ORDER BY m` binds that output column
instead and both engines agree lexically, which hides the member-order rule entirely.
`filter_by_member` needs no cast at all: it exercises enum equality server-side and only
returns `id`.

## Deferred

- **An enum type has a catalog OID, but a column still reports `text` on the wire.**
  The type's OID is persisted (`EnumType.oid`, since store magic `PGMJ`). It's what
  `pg_type.oid` and `pg_enum.enumtypid` are joined on, and `pg_attribute` reports it as
  an enum column's `atttypid`, so introspection resolves the type. What's still deferred
  is the **wire** half: a column's `RowDescription` type is `TypeOid::Text`, not the
  enum's OID, and declaring a param as the enum's own OID (so tokio-postgres sees
  `Kind::Enum`) is deferred too. A text param cast to the enum
  (`$N::text::cell_visibility`) binds; `tests/fixtures/params/enum_cast.sql` and
  `test_binary`'s `Params text::enum` case pin it (see [param_binding.md](param_binding.md)).
- **A cast's enum identity is erased** (§ The boundary, and `EDGES.md` row 104): an
  expression with no enum-typed *column* under it orders by label bytes. What a cast
  **converts** is built. A label the enum lacks is refused wherever 18.6 refuses it
  (§ A cast to an enum converts; `EDGES.md` row 241, closed), and an
  `ADD COLUMN … DEFAULT` back-fill of one is refused at the `ALTER` (row 105, closed).
  (Also noted in [compatibility.md](compatibility.md).)
- **No `ALTER TYPE … ADD VALUE`**, and no enum array **column**. `cell_visibility[]` is
  `type "cell_visibility" is not supported` at `resolve_column_type`, as are the
  `ALTER TABLE … ADD COLUMN` spelling and a `::cell_visibility[]` cast target. That's a
  consistent *loud* over-rejection where 18.6 accepts the declaration. General array
  support exists; the enum-array **column** is what's still unbuilt. An `ARRAY[…]`
  **expression** over enum-typed elements isn't part of this item and is ordered by member
  position (§ An `ARRAY[…]` is not an enum array column). `index_method_for` only reports
  GIN for a container type, so an enum column never reaches a GIN index either.
- **The enum name is ASCII-downcased again where it's stored.** This is a residual of the
  identifier case-fold (`docs/parser.md` has the fold rule; `execute_create_type` /
  `execute_drop_type` / `resolve_column_type` re-fold the catalog key). A *quoted* type
  name doesn't parse in the type position, so the re-fold only affects the statements
  that name a type outside it, `CREATE TYPE` and `DROP TYPE`. A quoted `CREATE TYPE`
  name: `CREATE TYPE "Loud" AS ENUM (…)` then `CREATE TABLE t (m loud)` is accepted here
  and `42704` on Postgres. And a quoted or unquoted `DROP TYPE` name, silently:
  `DROP TYPE "Rd"` drops an enum `rd`, and `DROP TYPE rd` drops one created as `"Rd"`,
  where Postgres answers `42704` to each. The exception is when a table holds the name as
  written, whose row type `execute_drop_type` finds first (`2BP01`, `EDGES.md` row 248).
  The fold is ASCII-only, so `CAFÉ`/`café` stay two types (matching Postgres), while
  `"Mood-X"`, an unquoted `Sequence`, or `MÖÖD` each store a name `pglet_server dump`
  can't render. The dump then exits without emitting anything, so the whole database
  can't be exported (`EDGES.md`).
