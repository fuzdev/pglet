# pglet

A small Postgres-compatible-ish database engine in Rust, optimized for size and
startup speed. Read `README.md` first for what it is and its status (early, not safe
for real data). This file is the dev and architecture context.

## How the docs are organized

- **This file** is the operating manual: what pglet is, the engine rules in short
  form, architecture, commands, the harness. It's loaded into every session, so keep
  it lean.
- **`docs/`** has the full version of each rule. Every rule below is summarized here
  and written out in full (mechanism, examples, residuals) in its doc, which is a
  `docs/` file or, for the depth rule and the tooling, a module doc. When a rule
  changes or a new one lands, update **both**: the one-liner here and the body in its
  doc. Don't let it grow into an essay here.
- **`PLAN.md`** is the roadmap. **`docs/compatibility.md`** is per-feature status: a
  table with one line per feature, each linking to its full record. It's the one
  `docs/` file that holds status instead of rules. **`EDGES.md`** is the edge ledger:
  one stably numbered row per known divergence the differential harness doesn't cover.
  Rules live here and in `docs/`; evidence and status live in those three.
- `TRADEOFFS.md` is the size/startup decision log. `tests/README.md` covers the
  fixture harness. `crates/pglet_storage/src/btree/CLAUDE.md` has the byte-exact
  on-disk format.

## North star: full-but-optional PostgreSQL conformance, proven on real consumers

The long-term aim is **full PostgreSQL conformance, delivered as optional
compile-time-additive layers**. A build carries only the features its consumer needs, so
artifacts stay as small as today's (or smaller) while the feature set grows. pglet
isn't a replacement for upstream PostgreSQL. It's for the places a real server
doesn't fit: test suites, browsers, and embedded/local-first apps. For fuz, it's the
**embedded backend for the fuz data layer**, a local-first, file-based store that
runs the same Postgres dialect as a real server.

**The SQL that real consumers issue comes first.** We add it feature by feature as
optional layers to keep the size small, and we never relax the differential bar to
get there.

**Release ordering.** The near-term release is a **fast test-fixture engine** that
spins up faster than Docker Postgres or pglite. The **embedded backend** is the
medium-term goal, gated on system-of-record maturity and soak confidence
(`README.md` § What it's for). One core serves both. The stripped
`--no-default-features` build is for minimum size, not test speed (§ Build profiles).

**All of fuz_app's *spine* SQL (the auth, fact and cell SQL its data layer issues)
runs end-to-end** on both pglet legs (native server and in-process wasm) in fuz_app's
`db` matrix, and the differential fixtures check each feature byte-for-byte against
real Postgres. The feature list and its caveats live in **`docs/compatibility.md`**;
this file doesn't repeat them.

The engine-wide rules are worth having here. Each one is written out in full
(mechanism, examples, residuals) in its `docs/` file:

- **An identifier is folded once, when it's read, and compared exactly after that.**
  The parser ASCII-lowercases an unquoted identifier when it reads it
  (`lexer::Token::name`, and `split_name_literal` for a name literal; the two change
  together). A `"…"`-quoted one is kept as is. Every comparison after that is a plain
  byte comparison. Full rule and residuals: `docs/parser.md` § Known lexical / parse
  divergences.
- **Names written back out are quoted by `lexer::quote_ident`, the `fmtId`-style
  renderer, and the caller deals with any over-quoting.** The SQL function
  `quote_ident` and `pg_get_serial_sequence`'s result are values, so they use 18.6's
  rule instead (`lexer::pg_quote`). `docs/names.md` § A name written back out.
- **Parameters and correlated columns bind at eval time. The plan is never mutated to
  bind.** They resolve against an `EvalEnv` frame chain that knows about aliases and
  levels (`planner/correlation.rs`), in one walk that returns one of four results
  (`Found`/`Claimed`/`Ambiguous`/`Unclaimed`). The first frame that owns the name
  stops the walk, a qualifier no frame owns leaves the chain, and a bare name counts
  exact matches. A qualifier from the evaluating level's own FROM clause never reaches
  the chain: a resolver over a node's output names is given its level's FROM names
  (`SimpleColumns::within_level`). Full version: `docs/scope.md`.
- **In a grouping query, the scalar walk reads each aggregate's value by node, and an
  aggregate is only skipped when the branch it's in is decided by its shape alone.** A
  branch (and its aggregates) is dropped only under a literal `true` / `false` /
  `NULL`, a comparison of two number literals (including a simple `CASE`'s operand
  against an arm's value), or `NOT` / `AND` / `OR` / an `IS` test over those, or, in a
  `COALESCE`, after a number or boolean literal. Folds 18.6 does that aren't on this
  list keep their aggregates, so a failing aggregate errors instead of returning an
  answer (`EDGES.md` row 89). Collecting the aggregates evaluates only those deciders.
  A dropped aggregate over a column or literal still gets its operand's type check,
  before any group. `HAVING`, the select list and its junk columns then go through
  `eval_expr_full` with a `SubqueryExec::aggregate` lookup keyed by node identity; a
  node with no computed value is refused. `docs/scope.md` § A grouping query's
  aggregates.
- **A plan cached by `execute_bound` must give the same answer as parsing its text
  again (except for the native parser's `54001`), and `plan_cache`'s `Stamp` is what
  guarantees that.** Every input that parsing or parameter inference reads has to be
  either a stamp field (`Schema::generation`, the thread's depth limit and
  unknown-type policy, and natively its stack bounds) or something that clears the
  cache (`schema_mut`, `set_stack_budget`). An input that's neither means a cached plan
  can silently return wrong answers, and the differential fixtures won't catch it.
  `docs/param_binding.md` § The bound entry point's plan cache.
- **A pending set-op column's type is never read.** A side that names an
  **enclosing** level can't be typed while it's being parsed, and the provisional type
  goes to a list of readers we can't enumerate. So the correlated re-walk's result is
  recorded (`SetOpOuterTypes`) and the statement is **parsed again** with it. The
  returned plan has no pending columns; if the re-walk never unified one, the
  statement is refused. `docs/scope.md`, `docs/coercion.md`.
- **An `IN` list is converted to 18.6's single-type array form only when every
  operand's type is certain by construction. Otherwise each item is a separate `=`.**
  Certain means a plain base-table column at the list's own level against two or more
  written constants. Converting a list on an inferred type matches rows 18.6 doesn't
  (`EDGES.md` row 280). `docs/coercion.md`.
- **A scalar subquery is a *typed* branch of a `COALESCE`/`CASE`/`ARRAY`/`greatest`/
  `least`/`nullif` when its type is resolved, and untyped (deferring unification) when
  it isn't.** Otherwise the readers can't see the branch, and it gets cast to its sibling's type, and
  `COALESCE((SELECT 1.25), 2)` returned `1`. "Resolved" means what its column
  references resolve to, a guess when the sub-plan's column is a guess, and not
  "certain" in the next rule's sense. A subquery whose column is certain is cast from
  the sub-plan's record, whether it's a branch or an operand under one, and the
  resolving reader handles the rest. A set of branches the gate can type is judged the
  way 18.6 judges it. One it can't type keeps per-branch runtime types under one
  declared type, which is a residual, not the design (`EDGES.md` row 152).
  `docs/coercion.md`.
- **A type is certain when the coercion pass typed the expression as written, and only
  one place decides that.** An aggregate's argument and a relation's column are
  decided by the pass's computed-reach reader before the pass rewrites the expression.
  The result is recorded on the plan (`ArgCertainty`, `CertainTypes`) and read from
  there. Reading the rewritten form instead treats a guessed cast, and the pass's own
  cast over a branch subquery, as typed, and readers that worked it out from their own
  scopes disagreed. A scalar subquery is certain when its sub-plan's column is, per the
  sub-plan's record, wherever it appears; the pass's own readers only type it from a
  cast written over it under a fold. An unknown (a bare string, a `NULL`, an
  undeclared `$N`) takes its fold's target type as a branch. As an arithmetic operand
  no reader types it, so nothing converts it to its sibling's type. A bare string
  compared with an aggregate is read as the aggregate's type when that's certain. That's
  decided from the written pair at the aggregate's own level, where every column its
  argument names must bind, and never on the correlated re-run (`docs/coercion.md`
  § Where the plan-time coercion pass reaches). When the type isn't certain, it gets
  the answer an unknown type gets (`EDGES.md` row 2). A fold with a branch no reader
  types only takes a computed sibling as its target when the sibling is `double
  precision` and the rest are numeric (row 54). `docs/coercion.md` § A computed branch
  is typed where its type is certain.
- **An enum keeps its own type in the checks that decide whether two operands have a
  comparison operator or a common type, and those checks model `typcategory`.** The
  jsonb key/path, `LIKE`-family, `starts_with`, `||` and array-containment rules and
  the string functions still treat an enum as `text`, so they run where 18.6 refuses,
  and the statement writes (`EDGES.md` row 190). `docs/enums.md`, `docs/coercion.md`.
- **Column references bind at plan time, and the check only refuses a name when it can
  prove it has the whole namespace.** `ColumnScope::check_binding` covers the
  zero-row and post-projection cases the eval chain can't reach. It returns one of
  three results (`Bound` / error / `Uncertain`), and "whole namespace" is computed,
  never assumed. `docs/scope.md`.
- **A FROM name is resolved by what kind of item it is, never by asking the catalog
  first.** A FROM item's effective name *replaces* the namespace entry at its level.
  `expand_qualified_wildcard` is still keyed on the name, which is wrong for an
  `unnest` item. `docs/scope.md`.
- **A clause has one namespace, and everything that reads the clause reads that one.**
  `RETURNING`'s namespace is the DML target under its effective name plus its `FROM`
  relations (`returning_relations`). To find its readers, check every site that reads
  a DML plan's `returning` items. `docs/scope.md` § A clause's namespace is one
  object.
- **A relation's column *names* are one object, and a rename is applied wherever each
  reader reads them.** A CTE's `name(cols)` list travels with the plan through
  `alias_relation_columns`. Both kinds of reference re-derive the names *at
  execution*. The compiler only catches some of the readers. `docs/scope.md`.
- **A statement's recursion has two limits: bytes where a walker can fail, levels
  where it can't.** Natively, a fallible walker checks stack bytes at each level
  (`54001`). The parser's depth count limits the infallible walks, and it's the only
  limit in wasm. `crates/pglet_sql/src/depth.rs` and its `stack` module.
- **Correct-or-loud.** A wrong-typed operand, an unsupported type or a malformed value
  errors (`42883`/`0A000`/`42704`/`22P02`) instead of silently coercing or matching
  zero rows. One rulebook (`planner/legality.rs`) feeds both the eval-time checks and
  the DDL-time static checker. A conversion has a **context** as well as a legality:
  the assignment check refuses what 18.6's assignment rule refuses, which is a
  `pg_cast` pair that's explicit-only and a pair with no `pg_cast` row at all.
  `docs/coercion.md`.
- **A boolean context resolves its operand to `boolean` where the operand is read, and
  only two things resolve: `boolean` itself and an unresolved literal.** That's the
  rule above with the target set to `boolean` (18.6's `coerce_to_boolean` is an
  assignment coercion), so every other type is `42804`. A literal is converted by a
  `boolin` that matches on prefixes. `docs/coercion.md`.
- **`AND` / `OR` stop at a left operand that decides the result, in every walk we
  found that evaluates them. A path that evaluates operands out of that order hands an
  operand's failure off to a path that keeps the order.** `connective_decided_by` is
  used by the scalar walk, including a grouping query's expressions (the grouping
  computes a skipped operand's aggregates first, as 18.6's `Agg` does, unless the left
  operand decides by its shape alone). The GIN guard, which is evaluated before any
  row, hands a failure to its recheck `Filter`. A NULL left operand decides nothing.
  `docs/coercion.md`; `EDGES.md` rows 60 and 172.
- **A canonical type name is split once into base, modifier and array suffix, and
  each part is then handled by the *type*, never by the spelling.** A modifier the
  type doesn't take is **refused** where `typenameTypeMod` refuses it, never dropped,
  and so is one the type's own typmodin refuses (`22023`). A modifier the type does
  take is kept, clamped to the type's maximum, and enforced, element by element under
  a `[]`. `docs/coercion.md`, `docs/temporal.md`.
- **A string literal is built in the same units as 18.6, and each escape is decoded
  with the grammar 18.6's scanner uses.** A `\xHH` / `\ooo` escape adds a **byte**;
  everything else adds a character's UTF-8. The finished literal is validated once,
  left to right, so the first NUL or illegal byte is the error. `docs/parser.md` § A
  string literal is assembled in 18.6's units.
- **An array literal is parsed with 18.6's array grammar, and each element is
  converted as it's read.** `executor::array_parse` mirrors `ReadArrayToken` and
  `ReadArrayStr`. A literal that grammar rejects as malformed is `22P02` whatever the
  element type, an element's own input error comes before a later structural one, and
  whitespace means `scanner_isspace`'s set only. A nested literal is parsed the same
  way and then refused with `0A000` (`EDGES.md` row 283). `docs/arrays.md` § What
  works.
- **Text coming into the engine is validated in the server encoding, through one
  scan, with known exceptions.** `pglet_core::encoding` finds the first NUL or illegal
  UTF-8 byte and returns `22021` at every entry point we've checked that uses it. The
  ones we found that skip it are in `docs/encoding.md` § Measured doors that do not
  reach it (`EDGES.md` rows 130, 133). `docs/encoding.md`.
- **A `DROP` either drops dependents automatically or refuses. It never silently
  leaves a broken catalog.** One resolver (`executor/dependencies.rs`) re-derives
  `pg_depend`'s edge kinds for `DROP TABLE`, `DROP COLUMN`, `DROP SEQUENCE` and
  `DROP TYPE`. `DROP TYPE` still drops an enum that a column uses (`EDGES.md` row 261,
  open). A constraint is identified by `(table, position)`, never by its name.
  `docs/ddl.md`.
- **A failed DDL statement must leave the catalog untouched.** The autocommit path
  can't undo it, so writing to the catalog and then failing leaves a durable phantom.
  Everything that can fail runs before the first catalog write, and catalog facts the
  statement creates are staged locally. So **where** a check runs matters for
  correctness. "Everything that can fail" includes the statement's own **work** and
  everything else it touches (storage, an index build), not just its checks.
  `docs/ddl.md`.
- **Constraint names live in two namespaces, and a generated name gets a suffix
  instead of being refused.** There's one name pool across all four constraint kinds
  (`42710`), plus the `pg_class` relation namespace for a PK/UNIQUE's index (`42P07`).
  `ConstraintNames` reads both as `catalog ∪ claimed`. A declared name that collides is
  the user's mistake; an auto-generated one gets a digit appended. `docs/names.md`.
- **A constraint's name is generated once, at execute time, and is the same
  everywhere.** The parser only knows declared-or-nothing, the catalog always has a
  name, `ConstraintNames` is the only link between them, and the generated name is
  persisted. The order names are generated in, per route, is part of the contract.
  `docs/names.md`.
- **Sequence names follow the same rule over a *different* namespace, because a
  sequence is a relation.** `Schema::relation_name_taken` treats tables, indexes and
  sequences as one pool. A declared `SEQUENCE NAME` that collides is refused
  (`42P07`); an auto-generated `<t>_<c>_seq` gets a suffix. `docs/names.md`.
- **The engine names an unnamed index the same way it names a sequence, and both are
  truncated to 63 bytes.** `CREATE INDEX ON t (…)` gets `<table>_<key part
  names>_idx` from `choose_relation_name` (18.6's `makeObjectName` truncation, with a
  digit on collision). The constraint name generators share the truncation and the
  digit loop. `docs/names.md` § An unnamed index's name.
- **A table's name is also a type name, and each CREATE checks the same namespace
  18.6 checks.** `Schema::type_name_taken` holds the enums and each table's row type.
  A table or sequence with an enum's name (including a `SERIAL`'s generated one) is
  `42710`, after the relation check's `42P07`. An enum with a table's name is `42710`.
  An index doesn't check the type namespace. `docs/names.md`.
- **A relation's column names are a set, and some names are already taken by the
  heap.** No two columns in a relation can share a name, and none can be
  `ctid`/`xmin`/`cmin`/`xmax`/`cmax`/`tableoid` (`oid` is an ordinary name; we
  checked). Both are `42701`, raised by the statements that add a column name
  (`CREATE TABLE`, `ADD COLUMN`) at the same point 18.6 raises them, on names the
  parser already folded. That list is the statements `Plan` has, not a claim that it's
  complete. `docs/names.md`.
- **A statement's target column list is a set.** A column can be assigned at most once
  per statement: `42701` for an `INSERT`'s or `COPY`'s column list, `42601` for an
  `UPDATE`'s or `ON CONFLICT DO UPDATE`'s `SET` list. Two functions check this, on two
  different keys: `claim_target_column` on the resolved position and
  `refuse_duplicate_assignments` on the folded name. `docs/names.md` § What the
  refusals do not carry.
- **An `ON CONFLICT` inference list is a set, and matching an index means set
  equality.** `conflict_target_matches` compares the way 18.6's
  `infer_arbiter_indexes` does, with plain attributes as sets and key expressions by
  membership in both directions, so order is ignored and repeats collapse on each
  side. It's containment both ways, so a subset is still refused with `42P10`.
  `docs/indexes.md`.
- **A constraint validates its own key list; the index it builds doesn't.**
  `PRIMARY KEY`/`UNIQUE` refuse a repeated key column (`42701`) and a foreign key's
  *referenced* list refuses one (`42830`). `CREATE INDEX`'s key list and a foreign
  key's *referencing* list accept repeats. Those are four separate facts, and the two
  acceptances are covered by tests that check the statement succeeds. Which pass
  resolves the keys is why the two PK/UNIQUE routes behave differently. `docs/ddl.md`.
- **A check with no Postgres equivalent error must run after every check that has
  one.** That includes checks that decide a Postgres *success*, like an `IF NOT
  EXISTS` skip. The flip side: a check that does match a Postgres error runs where
  18.6 raises it (`CREATE UNIQUE INDEX`'s build validation, `23505` inside
  `index_create`). `docs/ddl.md`.
- **A check that reads the catalog can't see what the current statement is creating.**
  For every check on the `CREATE TABLE` path, ask: *which of the names this statement
  creates can it not see?* A name the statement will register belongs in the
  in-flight namespace at the point Postgres's own catalog would have it.
  `docs/ddl.md`.
- **A sequence's `start` is not its counter, and `owner` is a third fact.** The
  catalog `Sequence` record is `{last_value, start, owner}`. A counter update only
  changes `last_value`. The dump writes it as three statements (two for an identity
  column), and a sequence name inside a `nextval('…')`/`setval('…')` literal keeps its
  *identifier* quoting inside the string. `docs/sequences.md`.
- **pglet must forbid any link Postgres can't represent, and code that reads the link
  must not assume it's forbidden.** Here the identity-column ↔ sequence tie is two
  ordinary facts that can each change. The DDL checks that keep them together and the
  resolver's narrow exemption are both needed; neither replaces the other.
  `docs/sequences.md`.
- **Once a cross-object reference is persisted, any existing leak of the referenced
  object becomes a dump bug.** The invariant: **any database state built by statements
  pglet accepts, through any entry point whose state a dump reads, must dump to
  something that loads into stock Postgres and back into pglet**, catalog and rows.
  Where a faithful dump isn't possible, the export errors instead of writing SQL that
  restores incompletely with exit `0`. `docs/sequences.md`.
- **A stored expression has two renderings, and which one each output uses matters
  for correctness.** pglet stores the expression as written; 18.6 stores it after
  parse analysis. `deparse.rs` is one exhaustive match over two `RenderMode`s.
  `Portable` (the dump, the render probe) keeps `parse(render(e)) == e`. `Catalog`
  (`column_default`, `check_clause`, `pg_get_constraintdef`, `indexdef`) reconstructs
  18.6's tree as a *rendering*, never by normalizing the plan. `docs/deparse.md`.
- **Stored expressions are checked at DDL time.** A CHECK, index predicate or index
  key expression is validated up front in one bottom-up walk (binding, static operand
  typing, the boolean-context check, a probe against a representative row), then a
  stored predicate gets the boolean check at its root, plus a **render probe**: every
  accepted persisted expression must deparse, or one table's `DEFAULT` makes the whole
  database undumpable. `docs/ddl.md`.
- **Index probes are exact or not used.** A probe value coerced to the indexed
  column's type is only used if it's exactly equivalent. Otherwise the planner skips
  the index and filters rows, so an inexact probe costs speed, never correctness. The
  foreign-key probe follows the same rule, except where 18.6 itself casts the child.
  `docs/indexes.md`.
- **An index range read never returns a row whose key is NULL, whatever the index
  holds.** Its lower bound is never below the smallest key a non-NULL value has
  (`index_keying::encode_range_bounds`, which both backends and the in-transaction read
  use for their bounds), so a `NULLS NOT DISTINCT` index's NULL keys are never in a
  range. A read with no bounds returns every row only when no key is NULL. An ordered
  scan is planned when the column is *declared* `NOT NULL`, and every statement that
  sets that declaration on existing rows reads them. The identity clause, which reads
  none, is `55000` on a column not already declared `NOT NULL` (`EDGES.md` row 38
  keeps its legacy store). `docs/indexes.md` § Index probes are exact-or-decline.
- **A simple `UPDATE` / `DELETE` uses the read path's equality probe only when nothing
  the probe skips evaluating can fail, and it writes the rows it finds in `RowId`
  order.** The probe is chosen at plan time by the `SELECT` path's own selector
  (`dml_target_probe`). Ranges aren't used (a B+tree write runs in a storage
  transaction, where a range lookup reads the whole table). The rest of the `WHERE`
  must pass `dml_residual_is_certain`'s whitelist, an unbound parameter or one with no
  operator skips the probe at execution, and every candidate row is tested against the
  whole `WHERE`. `docs/indexes.md` § A write's target rows.
- **A float key is the `float8` it widens to, ordered like 18.6's
  `float8_cmp_internal`, and values that order calls equal share one key.** `-0` keys
  as `0`, and every NaN (either sign, any payload) is one key above `Infinity`. So both
  float widths share a key space, and the key order matches the engine's own float
  comparison. `docs/indexes.md` § A float key.
- **A container value's key encodes the comparator's order as bytes.** `jsonb` and
  array keys are built so that `memcmp` of two keys equals `values_compare` of the two
  values, including `Equal`. That's 18.6's `jsonb_ops` and `array_ops` orders, with
  strings compared by their bytes (`EDGES.md` row 43). One shape is refused: a
  non-unique B-tree on a single bare container column, which `index_method_for` makes
  a GIN index instead (row 25). `docs/indexes.md` § A container key.
- **An index entry is sized the way 18.6 sizes it, at every entry point, checked in
  18.6's order.** There's one size (`executor::index_row`, 18.6's `index_form_tuple`
  layout) and 18.6's three `54000` errors, checked in 18.6's order, for every entry a
  write, a foreign-key action or an index build adds (over the table's indexes in
  creation order), and for every entry storage maintenance adds. Residuals that error:
  `EDGES.md` rows 191 and 192. Silent ones: rows 95 and 104. Order differences: rows
  92, 43, 166 and 194. `docs/indexes.md`.
- **Index selection has to ask what an index *is*, not just what it covers.**
  Selection asks `executor::index_method_for` (the same derivation storage uses), so a
  GIN index is only offered for containment. It also checks the key column's
  **order**: `single_column_index_for` skips an enum key column, because the enum's
  order is member position while the key encoder encodes label bytes.
  `docs/indexes.md`.
- **An enum's order is its members' positions, as listed in
  `Schema.enums[name].labels`.** It's read through `values_compare_ordered`, whose enum
  arm is a total order over member and non-member labels alike. Equality still compares
  label bytes, so the order affects `< <= > >=`, `ORDER BY`, `MIN`/`MAX` and
  `GREATEST`/`LEAST`, and also **writes**: a `CHECK` over an enum expression and
  whether a row is in a partial index. `docs/enums.md`.
- **The storage index specs follow `Schema.enums` whenever it changes.**
  `IndexSpec::column_enum_orders` is the only copy of a member order that outlives its
  statement, and a member list *can* change under it even when a column's type can't.
  So `Storage::rebuild_index` re-derives it at the two forward DDL sites and after
  **every backward `Schema::restore_from`**, via `enums_moved_by_restore`.
  `docs/enums.md` § A copy that outlives its statement.
- **An enum's labels are a set.** A repeated `CREATE TYPE … AS ENUM` label gets
  18.6's own `23505` on `pg_enum_typid_label_index`, after the type name's `42710`. A
  label is a string literal and is never folded. A repeat used to break the catalog
  and the dump (two `pg_enum` rows for one label, and a `CREATE TYPE` that stock 18.6
  refuses), but never the order, since `enum_rank` is `position()`. `docs/enums.md`.
- **A transaction block is one state machine, and every host drives the same one.**
  `pglet`'s `TransactionSession` owns the aborted-block state, the savepoint stack,
  whether the block holds the writer, and the exit classifier. Hosts dispatch on the
  `TransactionCommand` it carries, never on the command tag. `docs/storage.md` § The
  transaction session.
- **When the device refuses a write, the statement is dropped if the host can take it
  back. Otherwise the store fails.** A dropped statement returns `53100`/`58030` and
  the database keeps working. Otherwise the statement returns the same code and every
  later one gets `57P02` until a reopen reads the durable slot. `docs/storage.md`
  § Crash Recovery.
- **Only one connection's block can stage writes at a time. It takes *the writer* at
  its first write and holds it until the block ends, re-basing every savepoint frame on
  what's committed at that moment.** A second writer waits (on the server, up to
  `lock_timeout`, then `55P03`) or is refused with `55P03` (in the embed).
  `docs/storage.md` § Why Single-Writer?, § Isolation Guarantees.
- **Sequence counters aren't rolled back. The exception is about a replaced *cell*,
  not a kind of statement.** A statement that swaps the cell (`RESTART`, a `DROP` that
  drops the sequence, …) restores the value read before it ran
  (`Plan::swapped_sequences`). `docs/sequences.md`.
- **The clock functions return one of two instants, fixed per statement and per open
  block. Only `clock_timestamp()` reads the clock on every call.** A host marks each
  statement's start (`builtins::start_statement`), and an open block stores its start
  in `SessionSettings`, so the thing that can go stale is the list of places that mark
  a statement. `docs/temporal.md` § The transaction and statement clocks.
- **A client reading a structured error field gets what 18.6 sends, and a field pglet
  can't fill in truthfully is left out, not made up.** One builder
  (`Error::wire_fields`) feeds the native server's `ErrorResponse`, and the wasm error
  reads the same accessors it's built from. The differential harness counts a missing
  field as agreement, so unit, executor and host tests cover the rest.
  `docs/errors.md`.
- **A result set with zero columns is still a result set.** Whether a statement
  returns one is tracked as its own fact (`ResultShape`), never inferred from an empty
  column list. Two places produce that fact, `Plan::output_columns_scoped` and the
  executor's `QueryResult::shape` sites, and nothing checks they agree.
  `docs/declared_types.md` § Whether there is a descriptor at all comes first.
- **Which error wins is part of the contract, at three levels:** within an expression,
  between a statement's clauses (with a later tier for the checks 18.6 runs in its
  rewriter), and between protocol phases. Each needs one interleaved walk; running two
  separate whole-tree passes can't reproduce any of them. Adding a check always changes
  statement-level error precedence. `docs/ddl.md` § Which error wins.

The **generic-conformance bar** (`PLAN.md` §7) is **met**: every common gap that needs
no design decision and costs no size is closed to spec with differential tests, locked
by `tests/gates_baseline.txt`. What's still open is tracked in `EDGES.md`'s live rows
and `docs/compatibility.md`'s ⚠️/🔲 rows. All the non-spine column types are in (DATE,
NUMERIC, TIME, INTERVAL), and an unrecognized type errors instead of being silently
coerced.

## Architecture

- **Sync core.** No async in `pglet` / `pglet_storage`; the server handles I/O.
- **Thread-per-connection** native server.
- **Storage trait** (defined in the `pglet_core` leaf crate) with swappable
  backends: in-memory (tests) and a Copy-on-Write B+tree (the default; in-memory
  unless `--data-dir` makes it persistent). See `docs/storage.md`.
- **MVCC via root versioning.** Autocommit SELECTs read a lock-free snapshot
  (atomic root pointer); writes take **the writer** (single-writer), held by a block from
  its first write to its end and by an autocommit write for its statement
  (`pglet_server`'s `writer.rs`; `docs/storage.md` § Why Single-Writer?).
- **Wire protocol** is a pure codec (`pglet_wire`) — `&[u8]` ↔ message structs,
  no I/O. The server owns sockets and session state.
- **The socket is configured, not merely accepted, and the response path is
  buffered.** `set_nodelay(true)` on every accepted connection (a silently
  Nagle'd one serves every query ~70× slow, measured), and a `BufWriter` every
  waiting path must flush or the two ends deadlock. `docs/storage.md` § The
  socket and the response path.

### Crates

```
crates/
  pglet_core/      # leaf crate (serde/time/thiserror only): Value/TypeOid/ColumnDesc, Error + SQLSTATE, RowId/IndexSpec, TransactionStatus, the Storage trait, the value codec (text/binary/index-key) + JSON parser, `encoding` (the server-encoding scan — a NUL or an illegal UTF-8 byte is `22021`, `docs/encoding.md`), and `file_lock` (the one flock, default-off feature) — every other crate depends on it
  pglet_sql/       # SQL front-end: lexer + native parser (docs/parser.md), planner, executor, catalog. Modules that are the single home for a rule:
                   #   planner/correlation.rs — correlated re-resolution against the level-chained scope
                   #   planner/legality.rs — the one operand-type rulebook
                   #   executor/dependencies.rs — the one DROP-time dependency resolver
                   #   deparse.rs — the one Expr deparser: one exhaustive match over two RenderModes, Portable for pglet::dump and the DDL render probe,
                   #     Catalog for column_default / check_clause / pg_get_constraintdef / indexdef, which print 18.6's post-parse-analysis tree (docs/deparse.md)
                   #   catalog/constraints_view.rs — the one constraint enumeration and renderer, feeding pg_constraint's rows, pg_get_constraintdef and the dump's ADD CONSTRAINT
                   #   depth.rs — the one statement-tree depth bound and its per-host limits
  pglet_storage/   # Storage impls + btree/ (CoW B+tree; double-meta single-file store over BlockIo; native FileBlockIo shim; memory). Depends on pglet_sql (not the reverse) for stored-Expr eval — expression/partial-index maintenance + GIN decompose
  pglet_wire/      # Postgres wire-protocol codec (no I/O); its one workspace dep is pglet_core (TypeOid, TransactionStatus), not the SQL front-end
  pglet/           # embed facade: Database (open/execute/execute_bound/commit/sync/fork), generic over the storage backend — the native server (via into_parts), bench, and pglet_wasm all wrap it; the parse→bind→execute→persist driver lives here once (file-backed `open` behind the default `file_store` feature and `StorageBackend::Memory` / `Database::in_memory` behind the default `memory_storage`, both off in the wasm build — `pglet_wasm`'s `lib.rs` fails that build if `memory_storage` returns). transaction.rs is the TransactionSession every host drives (docs/storage.md); a `Database` dropped with a block open on a durable store rolls it back (`impl Drop for Database`), and one whose store failed a durable write refuses every statement `57P02` until reopened. fork is the per-test-fixture primitive: a cheap CoW branch of an in-memory database's committed state — docs/storage.md § Fork; with_btree_for_one_connection is the one-connection store the wasm in-memory database runs on (docs/storage.md § The transaction session); plan_cache.rs is execute_bound's text-keyed plan cache (docs/param_binding.md)
  pglet_server/    # native TCP server binary (sockets, session, dispatch); server.rs reopens a store that failed a durable write for the next connection, the data-dir lock held throughout
  pglet_wasm/      # wasm-bindgen bindings: in-memory CoW-btree PgletBtree (one connection's, Database::with_btree_for_one_connection; Node-usable: bound $N params via query_params + BEGIN/COMMIT/ROLLBACK) + browser/OPFS PgletBtreePersistent behind the default-off `opfs` feature (persistent.rs + opfs_block_io.rs; absent from the build the npm package ships and the size gate measures); now()/gen_random_uuid()/random() sourced from the JS host (Date.now/Math.random)
  pglet_bench/     # native bench harness — commit latency + write-amp (vs rusqlite), read concurrency, query latency, GIN containment, front-end parse/plan, CPU attribution, peak RSS / wasm linear-memory high-water; std::time medians
  pglet_tests/     # test binaries (fixtures vs real PG, binary-format, concurrency, kill-9 crash/soak, COPY, portable dump + pg_dump round-trip, large-dataset soak: churn+kill+in-process audit + a per-test-DB-reuse/index-OID-desync regression-lock leg)
  pglet_testing/   # dev-dependency-only test toolbox: BlockIo doubles (MemBlockIo + fault injection), store-image editing and power-loss images of a recorded write log, the MemStorage oracle, open_database/ScratchDir and the recover-or-loud durability invariant, the server spawn/kill harness, a /proc RSS reader, a seedable PRNG. Feature-gated (block-io / embed / oracle / server) so a leaf consumer pulls only what it needs; never reachable from a shipped artifact
  xtask/           # dev-only: `cargo xtask check-release` (the release firewall — a `cargo tree -e normal` walk of each shipped artifact's runtime closure), `cargo xtask gates` (the locked gate-set runner — see § The gate runner) and `cargo xtask measure` (the quiet-box measurement set)
```

The durable on-disk layout is **single-file double-meta, no WAL** (LMDB-style
two meta slots; commit = write CoW pages → fsync → write the alternate slot → fsync,
the commit point). The slot alone is not the guarantee: collection frees no page the
durable slot reaches, and the free-list chain is checksum-linked from the slot. Native and
the `opfs` wasm build share **one** `DoubleMetaStore` core, differing only in the `BlockIo` shim (`FileBlockIo` native / `OpfsBlockIo` over OPFS); the default wasm build links no durable store. See
`docs/storage.md` and `PLAN.md` §3.

### Type system

Types live in the `pglet_core` leaf crate (`crates/pglet_core/src/types/` —
`value`/`type_oid`/`text`/`index_key`/`numeric`/`interval`/`precision`/`json`/`binary`/
`infinite`/`temporal_domain` submodules); SQL-type mapping in `executor/type_resolution.rs` matches each
type explicitly.

- **Real types:** bool, int2/4/8, float4/8, text, varchar, bytea,
  timestamp(/tz, UTC only) and date — each with `infinity` / `-infinity` — time, numeric,
  interval, jsonb, uuid, and
  one-dimensional **arrays** of those (a `Value::Array` carrying its element
  type, with per-type array OIDs — `docs/arrays.md`; enums are text + a catalog
  member set, ordered by member position — `docs/enums.md`).
- **NUMERIC / DECIMAL** — an in-house base-10 bignum (`PgNumeric`, no external
  dep — the decided posture): exact comparison (never via `f64`),
  order-preserving index key, Postgres's per-operator scale rules, exact math
  functions, and one `apply_typmod` reached from the column form and from a cast
  alike, at the one store finalizer every write path storing a non-NULL value funnels
  through.
  `docs/numeric.md`.
- **DATE / TIME / INTERVAL** — full column types: text/binary/index-key codecs,
  comparison and range, casts, complete arithmetic, and the SQL value functions
  (`CURRENT_DATE`, `CURRENT_TIME`, `LOCALTIMESTAMP`, … — parenless special
  forms, not functions). Malformed input is `22007`, out-of-range `22008` — for
  DATE, TIME and INTERVAL; **the timestamp types answer `22P02` for malformed input**
  and 18.6's `22008` / `22009` for a field out of range, their reader splitting a
  spelling into fields as `ParseDateTime` does (`EDGES.md` row 96). `docs/temporal.md`.
- **The date domain is `0001-01-01` through `9999-12-31`** — `date`, `timestamp`, and
  `timestamptz` read in UTC; pglet models no BC. A finite value outside it is refused
  where it is produced, bound through `execute_bound` or stored, through
  `check_date_domain` / `check_timestamp_domain` (`pglet_core::types`); `infinity` and
  `-infinity` pass. A refusal is loud, mostly `22008` (`EDGES.md` rows 142 and 51 are its
  residuals). `docs/temporal.md` § The date and timestamp domain.
- **Sub-second precision** — `timestamp(p)`/`timestamptz(p)`/`time(p)`/`interval(p)`
  round half away from zero about the value's *internal* zero (µs since 2000-01-01,
  not the wall clock), through one `round_micros_to_precision` in `pglet_core`; `p > 6`
  clamps to 6, `p = 6` is a captured no-op modifier and not the absence of one, and a
  rounding carry past year 9999 is `22008` where 18.6 answers a year-10000 value.
  `docs/temporal.md` § Sub-second precision.
- **`timestamptz` text encoding matters:** only the *column* OID carries
  the tz/no-tz bit, so the text encoder is column-type-aware, an array's elements
  included — a missing `+00` offset makes a text-protocol driver silently shift the
  instant.
  `docs/temporal.md`.
- **`name` / `name[]` are declared types with no column form** — as are `oid` and
  `"char"` (`type_oid_to_sql_name`'s `None` arms). OIDs 19/1003, the
  session functions' (`current_user`, `current_schema`, `current_schemas`); shape-free
  like `varchar` (a `Value::Text` + the OID), in the string category
  (`legality::is_string`), and refused as a column type or a cast target.
  `docs/compatibility.md`'s *Session information functions* row.
- **Fail-loud gaps:** `TIMETZ` and any unrecognized type are `42704`, never a
  silent coerce — one type-name reader (`parser::parse_type_name`, whose scalar half
  the typed constant reads directly), one resolver (`executor`'s `resolve_type_name`,
  entered as `resolve_column_type` or `resolve_constant_type`). Legacy lenient coercion is the
  opt-in `--coerce-unknown-types` flag / `Database::set_coerce_unknown_types`.
- **Store versioning:** the persisted-shape magic is `PGMV`/v30, and a magic is never
  reused (v29 skipped v1's `PGMT`; the next is `PGMW`). The bump ledger and the scheme:
  `docs/storage.md` § Store versioning.

## Commands

```bash
cargo build                       # build all crates
deno task build                   # cargo build --release
cargo clippy --workspace --all-targets -- -D warnings   # lint (what `check` runs)
cargo run -p pglet_server         # server on :5433 (btree storage)
cargo run -p pglet_server -- --storage memory   # MemoryStorage: no MVCC, no persistence;
                                                # a failed autocommit DML's partial effects
                                                # stand (EDGES.md row 40)
cargo run -p pglet_server -- --data-dir <dir>   # persistent
cargo run -p pglet_server -- --lock-timeout 5s  # how long a write waits for the writer before 55P03
                                                # (a lock_timeout value; default 1s, 0 = no bound)
cargo run -p pglet_server -- --help             # every flag; an unknown, repeated or unparsable one is refused
deno task server                  # dev server, rebuilt and restarted on a .rs change (`server:restart` stops it;
#                                   the next .rs save rebuilds and starts it)

cargo xtask gates                 # run the gate set once, under an exclusive lock (see below)
cargo xtask gates --check         # ...and fail if a counter regressed against the baseline
cargo xtask gates --update        # ...and record this run's counters as the baseline
cargo xtask gates --only test:fixtures,test:rust   # ...restricted to named tasks
cargo xtask gates --skip test:pg-dump   # ...minus named tasks (--skip beats --only; an unselected
#                                         gate's baseline counters are not asked for)
deno task gates:check             # cargo xtask gates --check, through deno (gates:update likewise)
deno task size:check              # wasm size gate: raw bytes under the lower of baseline +2% and the 1650 KB hard cap (needs wasm-pack, gzip)
deno task size:update             # ...and record this build as the new baseline
deno task check                   # clippy -D warnings + stripped + wasm-features + fmt + docs + release audit
deno task check:stripped | check:wasm-features | check:release   # check's own steps, runnable alone (the last is
#                                   cargo xtask check-release)
deno task fmt                     # cargo fmt (check runs cargo fmt --check)
deno task check:docs              # rustdoc: every doc-comment link resolves (see below)
deno task dump --data-dir <dir>   # print a portable SQL dump of a persisted database
deno task lint:edges              # EDGES.md's structural invariants (tools/edges_lint.ts's header lists them)
deno task size                    # print the release server's size; size:wasm the wasm's, size:gzip the server's gzipped — ungated

deno task test                    # the suites deno.json's "test" chains (auto-spawn servers) — not every test:*;
#                                   test:binary, test:integration:server, test:power-loss, test:soak are among those outside it
cargo test --workspace            # Rust unit + integration tests (incl. crash fuzz); without `--features pglet_wasm/opfs`
#                                   it skips the persistent binding's host tests, which test:rust runs
deno task test:fixtures           # SQL fixtures vs real Postgres
deno task test:fixtures:update    # regenerate expected output from Postgres
deno task test:fixtures:profile-lock  # stripped-profile regression lock (the `@profile-lock` subset)
deno task test:rust               # cargo test --workspace --features pglet_wasm/opfs (the gate wrapper)
deno task test:binary             # binary-format wire tests (self-spawning server; in no aggregate or gate)
deno task test:dump               # portable SQL dump: pglet-vs-psql round-trip (needs psql)
deno task test:pg-dump            # pg_dump round-trip (needs psql + pg_dump)
deno task test:copy               # COPY FROM/TO fixtures vs Postgres
deno task test:keywords           # keyword-probe sweep vs Postgres
deno task test:wasm               # wasm build + the Deno wasm integration suite (needs wasm-pack)
deno task wasm:build              # the wasm build: wasm-pack of crates/pglet_wasm into its pkg/ (what test:wasm and
#                                   size:check run first; the npm package is build:npm)
deno task wasm:build:opfs         # the same with the `opfs` feature on, into pkg-opfs/ — the one build exporting
#                                   PgletBtreePersistent (examples/browser's persistent page, test:browser's OPFS worker)
deno task test:web                # the driver/ORM/npm suites (spy-proxy parsers, integration, drizzle, kysely, postgres, pg, npm);
#                                   each is its own task (test:spy, test:integration, test:drizzle, test:kysely,
#                                   test:postgres, test:pg, test:npm) — tests-web/README.md
deno task test:power-loss         # dm-flakey power-loss rounds (root-only; skips without the device)
deno task test:concurrent         # concurrency / MVCC isolation (spawns a server)
deno task test:crash              # real-process kill -9 + soak durability test
deno task test:soak               # large-dataset soak: churn + kill -9 + in-process audit, plus a re-migration/index-OID-desync regression-lock leg (opt-in; PGLET_SOAK_ROWS / _DURATION_SECS / _SEED / _CYCLES / _KILL_CYCLES, --long, which test:soak:long runs in release)
deno task test:browser            # WASM (pkg/) + OPFS (pkg-opfs/) under Playwright, building either when missing or
#                                   older than the newest .rs it links;
#                                   `test:browser:install` fetches its chromium once; test:browser:headed shows the browser
deno task fuzz                    # coverage-guided cargo-fuzz recovery target (needs nightly + cargo-fuzz; see fuzz/README.md)
deno task fuzz:seed               # (re)write the valid seed image into the fuzz corpus

deno task matrix:artifacts        # build release server + wasm pkg, print the fuz_app db-matrix env exports
deno task matrix:env              # re-print those exports without rebuilding (eval-safe: eval "$(deno task matrix:env)")
deno task spy:start               # wire-protocol logging proxy, :5434 → :5432 (tools/spy_proxy/main.ts's header);
#                                   5434 is also the port tests-web's suites spawn their server on
#                                   (tests-web/lib/server.ts), so stop the proxy before those
deno task spy:spine:cell | spy:spine:fact   # the proxy recording to tests/captures/spine_{cell,fact}.jsonl
deno task wasm:serve | wasm:test  # examples/browser's static server on :8001 and its Deno smoke test
#                                   (examples/browser/README.md)

deno task build:npm               # assemble @fuzdev/pglet-wasm (pkg-npm/ emptied → wasm-pack → patch_npm_package.ts)
#                                   the npm name is kebab (`-wasm`, like `@fuzdev/tsv-wasm`); the crate and its generated
#                                   `pglet_wasm.js` / `pglet_wasm_bg.wasm` stay snake_case — patch_npm_package.ts sets the name
deno task test:npm                # build:npm + the adapter suite (coercion contract, shape, DX) + a Node smoke run
#                                   (refusing a Node below the package's `engines` floor) + the packed tarball
#                                   installed offline and imported by bare specifier (scripts/test_npm_pack.mjs);
#                                   the registry identity fields live in scripts/npm_metadata.ts;
#                                   test:npm:node is build:npm + the Node smoke and pack legs alone
deno task publish:npm             # publish @fuzdev/pglet-wasm — USER-RUN, dry-run default; gates on check, test:npm and the shipped wasm's size,
#                                   then `npm publish --dry-run` (`--wetrun`: the publish) and prints the `v<version>` tag command;
#                                   any other flag is refused (CONTRIBUTING.md § Releasing)

deno task bench:commit            # commit-latency + write-amp sweep vs SQLite (prints)
deno task bench:commit:update     # ...and refresh benches/phase1_results.json
deno task bench:reads             # read-concurrency scaling (page-cache contention)
deno task bench:query             # query/scan latency vs SQLite (point lookup / scan / aggregate)
deno task bench:gin               # GIN-accelerated `@>` vs full-scan containment latency (pglet-vs-pglet)
deno task bench:index-accel       # index acceleration ON vs OFF (wasm): size delta + read-latency ratio across table sizes (builds both profiles)
deno task bench:memory            # peak RSS (native embed, /proc VmHWM) + WASM linear-memory high-water — pglet-internal longitudinal
deno task bench:wasm-compare      # in-memory wasm startup+query: pglet vs pglite vs sqlite-wasm (Deno/Node/Bun;
#                                   :deno / :node / :bun run one runtime, :install populates benches/node_modules
#                                   once — benches/README.md)
deno task bench:size-compare      # pglet-vs-pglite payload bytes, raw + gzip: the shipped npm wasm vs pglite's wasm + data
#                                   (:update rewrites benches/pglite_size_comparison.json)
deno task bench:exit-proof        # the four-leg fuz_app `db` suite macro (pglite, pglet-wasm, pglet-server, pg)
deno task bench:startup           # native server spawn→ready latency
deno task bench:wasm-init         # wasm init phases (compile / construct / first query)
deno task bench:parse             # fused parse+plan latency (pglet-vs-pglet)
deno task bench:attribute         # commit-CPU attribution

deno task measure:release         # the whole quiet-box measurement set behind the README's figures
```

**`measure:release` (`cargo xtask measure`) is the counterpart to `cargo xtask gates`**
— *what are the numbers* rather than *is the engine correct*, meaningful only on an idle
box with a live Postgres. Only its exit-proof step needs a fuz_app checkout
(`PGLET_FUZ_APP_DIR`, default `../fuz_app`, as `deno task bench:exit-proof` reads it);
`--skip exit-proof` runs the rest without one. Every bench step overwrites a recorded
results JSON, so probe it with `--dry-run`. It ends by printing every figure the README, the
npm README and `TRADEOFFS.md` quote from a file its steps write, as the run just wrote it —
the multiples and size ratios are computed by the benches, never divided by hand. Every file
it writes is stamped `-dirty` when the tree differs from its commit (`-dirty?` when that could
not be read). The rest: `crates/xtask/src/measure.rs`'s module doc.

Benches print by default; `--update` (or the `:update` task) refreshes the recorded JSON
(`benches/README.md`). Run them with `--release` (the tasks do). `benches/lib/bench_stats.ts`
and `crates/pglet_bench/src/stats.rs` reproduce `@fuzdev/fuz_util`'s statistics — a change to
one is a change to all three, or it is a bug.

Ports: 5432 real Postgres (fixture generation), 5433 dev server, 5434–5499 for
test-spawned servers (`PGLET_TEST_PORT` is the start of the dynamic range,
`DYNAMIC_PORT_END` its exclusive end), 8001 browser tests. Address the Postgres oracle and
the pglet servers as `127.0.0.1`, not `localhost`.

## The gate runner

`cargo xtask gates` runs the gate set — `check`, `size:check`, `test:wasm`, `test:rust`,
`test:fixtures`, `test:fixtures:profile-lock`, `test:dump`, `test:pg-dump`, `test:copy`,
`test:keywords`, `test:concurrent`, in that order — under an exclusive lock, writing a log per
task under `target/gates/`. Carry two things when reading a run: interleaved runs fail in ways
that look exactly like a real regression, and **a killed run's log reads green right up to
the kill** — read the summary's `KILLED` / `FAIL(n)`. `--check` fails a run against
`tests/gates_baseline.txt` (its header says on what — a skipped gate included), and
`--update` refuses a run with failures. It is not in `deno task check`, which
stays hermetic. The why: `crates/xtask/src/gates.rs` and `runner.rs`'s module docs;
prerequisites: `CONTRIBUTING.md` § Running the checks.

`deno task check:docs` (rustdoc, chained into `check`) fails a `` [`Name`] `` in a doc
comment that resolves to no item, and an invalid HTML tag; it cannot see whether the
sentence around a link is true. It is not the only gate that reads doc comments:
`check`'s clippy fails some prose structure too (`doc_lazy_continuation`, measured). A
reference the link syntax cannot express — a `#[cfg(test)]` test name, a feature-gated
module — takes backticks instead.

## The oracle override

Every *harness* site that reaches the Postgres oracle — the routed ring
`crates/pglet_tests/README.md` § Environment defines — reads `pglet_tests`'
`POSTGRES_CONFIG`, derived once per process from **`PGLET_PG_URL`** (a libpq URL or a
`key=value` conninfo; unset, `host=127.0.0.1 port=5432 user=postgres dbname=postgres`;
malformed, a loud refusal naming the variable, never a fallback). **The harnesses reset
the `public` schema of whatever it names.** `cargo xtask measure` reaches
`127.0.0.1:5432` from its own constant instead. The rest — the unrouted ring, session
identity, what a password reaches, the CI job — is that README section's; the
contributor-facing warning is `CONTRIBUTING.md`.

## Build profiles

Two build shapes over one core, selected by the default-on `index_acceleration` Cargo
feature: **default** (acceleration ON) and **stripped** (`--no-default-features`: the
planner emits `Filter(Scan)` for every predicate, so an index is *intended* to change only
speed, never a result). The measured exceptions against that intent — rows 56, 148 and 276
and row 180's bullet, where one build answers or accepts and the other refuses, row 126,
which moves an error's construct word, and row 38's legacy-store residual, where both
answer and the accelerated build leaves a row out — are named in `EDGES.md`'s header; they
are the measured members, not a bound.
The strip is a **size-floor lever, not a fixture-speed one**, so the test-fixture audience
keeps acceleration ON. Run the full stripped pass (`PGLET_NO_INDEX_ACCEL=1 deno task
test:fixtures`) by hand when a change touches index selection, the `IndexScan` /
`IndexRangeScan` / `GinScan` read paths, or a gate one profile reaches and the other
cannot; what a green `@profile-lock` run does and does not prove is `docs/indexes.md`
§ Build profiles.

## SQL fixtures

Fixtures in `tests/fixtures/` compare pglet output against real Postgres:

```sql
-- @setup
CREATE TABLE users (id INT, name TEXT);
-- @test
SELECT * FROM users;
```

`tests/README.md` § Directives is the contract for writing one — the directives, their
grammar, the required justifications and the known limits. To carry: **an unknown
directive is a hard parse error**, not inert comment text; an `@expect-error:<label>`
fixture must match Postgres's SQLSTATE wherever Postgres also errors (where Postgres
accepts, any error passes); and the `@test`'s last statement's rows are compared in both
wire formats unless `@text`/`@param` narrows it to one, with each column's declared type
OID compared on both legs (`docs/declared_types.md`). `deno task test:fixtures:update`
regenerates expected output from the Postgres on :5432. This differential harness is
pglet's strongest correctness asset.

**Conformance reference — keep a PostgreSQL source checkout at `../postgres` and consult it
when implementing** (docs under `doc/src/sgml/`, plus the C for exact behavior). Before
writing code for a SQL feature or fixing a divergence, read the relevant source/docs for
the exact semantics — result type, scale, NULL handling, SQLSTATE, edge cases — rather than
guessing or web-searching (workspace policy), then lock it with a differential fixture: the
running :5432 instance is the ground truth for the harness, `../postgres` for *why*. A
checkout of the development branch (its `configure.ac` names the version) runs ahead of
the 18.x oracle, so a catalog entry there may not exist on :5432; when they split, the
**running instance wins** — it is what the fixtures assert — and the rule's own comment
names the divergence.

## Conventions

- `snake_case` crates/files/functions; `PascalCase` types.
- Avoid `.unwrap()`/`.expect()` in library code — return `Result` (thiserror).
- `unsafe` is forbidden by `[workspace.lints.rust] unsafe_code = "forbid"`. `pglet_wasm`
  alone re-pastes the lint set with it at `deny`, which `opfs_block_io.rs` alone lifts
  (`#![allow(unsafe_code)]`, compiled only under the `opfs` feature) for the OPFS sync
  handle's `unsafe impl Send`/`Sync`, justified by single-threaded WASM — keep it audited
  and rare.
- Minimal dependencies; every dep is weighed against the size budget
  (`TRADEOFFS.md`).

### fuz-stack conformance

pglet takes the fuz-stack conventions — the skill is the source of truth, the ecosystem's
other Rust workspaces the working references — and not sibling crates: no manifest has a
path dependency outside the repo, so a lone clone, anywhere on disk, builds, checks and
runs its gate set with no sibling checkout present (the gate set's own prerequisites are
`CONTRIBUTING.md` § Running the checks). Its primitives are in-tree, each stated in its module
doc: `pglet_core::file_lock::FileLock` — the one `flock`, taken by
`pglet_server`'s `--data-dir` guard and the xtask runners, a default-off feature so the
wasm build never compiles `std::fs` — and `cargo xtask check-release`
(`crates/xtask/src/release.rs`, chained into `check`), which exits 65 when a shipped
artifact (`pglet_server`, `pglet_wasm`, `pglet_bench`) reaches the dev-only
`pglet_testing`.

The workspace is edition 2024, `rust-version = 1.89` (the floor `std::fs::File::try_lock`
sets), `publish = false`, built with the toolchain `rust-toolchain.toml` pins (1.98.0);
the wasm size baseline is that toolchain's output, so moving it moves the size gate.
