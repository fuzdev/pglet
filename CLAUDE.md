# pglet

A small Postgres-compatible-ish database engine in Rust, optimized for size and
startup speed. Read `README.md` first for what it is and its (early, unsafe-for-
real-data) status. This file is the dev/architecture context.

## How this doc surface is organized

- **This file** — the operating manual: what pglet is, the engine rules in
  short form, architecture, commands, the harness. Loaded into every session,
  so it stays lean.
- **`docs/`** — the full statements. Every rule below is summarized here and
  stated completely (mechanism, measured examples, residuals) in its doc — a `docs/`
  file, or, for the depth rule and the tooling, a module doc; when a rule gains a
  consequence or a new rule lands, it lands in **both** — the one-liner here, the body
  in its doc — never as a growing essay here.
- **`PLAN.md`** — the roadmap. **`docs/compatibility.md`** — per-feature status: a
  one-line-per-feature table over each feature's full record, the one `docs/` file that
  holds status rather than a rule. **`EDGES.md`** — the edge ledger: one stably-numbered
  row per recorded divergence the differential harness does not hold.
  Rules live here and in `docs/`; *evidence and status* live in those three.
- `TRADEOFFS.md` — the size/startup decision log. `tests/README.md` — the
  fixture harness. `crates/pglet_storage/src/btree/CLAUDE.md` — the byte-exact
  on-disk format.

## North star: full-but-optional PostgreSQL conformance, proven on real consumers

pglet's long-term aim is **full PostgreSQL conformance, delivered as optional
compile-time-additive layers** — a build carries only the features its consumer
needs, so artifacts stay as small as today's (or slimmer) while the surface
grows. pglet is never positioned as a replacement for upstream PostgreSQL; it
serves the places a real server doesn't fit: test suites, browsers, and
embedded/local-first apps — for fuz, as the **embedded backend for the fuz data
layer**, the local-first, file-based store that runs the same Postgres dialect
as a real server.

**The SQL real consumers actually issue comes first**, reached feature-by-feature as
optional layers so the size thesis survives, never by relaxing the differential bar.

**Release ordering.** The near-term release is a **fast test-fixture engine**, quicker to
spin up than Docker Postgres or pglite; the **embedded backend** is the medium-term goal,
gated on system-of-record maturity and soak confidence (`README.md` § What it's for). One
core serves both; the stripped `--no-default-features` build is a size-floor lever, not a
test-speed one (§ Build profiles).

**fuz_app's whole *spine* SQL surface — the auth, fact and cell SQL its data layer issues —
runs end-to-end** on both pglet legs (native server + in-process wasm) in fuz_app's `db`
matrix, each feature validated byte-for-byte against real Postgres by the differential
fixtures. The feature ledger and its caveats are **`docs/compatibility.md`** — the single
source of truth; this file doesn't re-narrate them.

The engine-wide principles are dev-context worth stating here; each is stated
in full — mechanism, measured examples, residuals — in its `docs/` file:

- **An identifier is folded once, where it is read, and compared exactly forever after.**
  An unquoted identifier is ASCII-downcased where the parser reads it
  (`lexer::Token::name`; `split_name_literal` for a name literal — the two name positions
  move together), a `"…"`-quoted one is verbatim, and every comparison downstream is a
  plain byte comparison. Full rule + residuals: `docs/parser.md` § Known lexical / parse
  divergences.
- **`lexer::quote_ident` is the `fmtId`-style renderer a name written back out is quoted
  by, and what an over-quote costs is the caller's, not the predicate's.** The SQL function
  `quote_ident` and `pg_get_serial_sequence`'s answer are values and take 18.6's rule
  instead (`lexer::pg_quote`). `docs/names.md` § A name written back out.
- **Eval-time binding — the plan is never mutated to bind.** Params and correlated columns
  resolve at eval time against an `EvalEnv` frame chain — alias-aware, level-aware
  (`planner/correlation.rs`), one walk with a four-way verdict
  (`Found`/`Claimed`/`Ambiguous`/`Unclaimed`): the first owning frame stops the walk, a
  qualifier no frame owns leaves the chain, a bare name counts exact claimants — and a
  qualifier the evaluating level's own FROM clause carries never reaches the chain: a
  resolver over a node's output names is told its level's FROM names
  (`SimpleColumns::within_level`). Full statement: `docs/scope.md`.
- **A grouping query's aggregates are values the scalar walk reads by node, and none is
  dropped but under a decider that is certain by its shape.** A branch goes, its
  aggregates with it, only under a literal `true` / `false` / `NULL`, a comparison of two
  number literals (a simple `CASE`'s operand against an arm's value included), or `NOT` /
  `AND` / `OR` / an `IS` test over those — or, in a `COALESCE`, after a number or boolean
  literal. A fold of 18.6's the list does not hold keeps its aggregates, so it is loud
  where one fails rather than an answer (`EDGES.md` row 89). The collection evaluates
  nothing but those deciders, and an aggregate over a column or a literal keeps its
  operand's type check where it is dropped, asked ahead of every group. `HAVING`, the select
  list and its junk columns then go through `eval_expr_full`, lent a
  `SubqueryExec::aggregate` lookup keyed by node identity; a node nothing was computed
  for is refused. `docs/scope.md` § A grouping query's aggregates.
- **A plan `execute_bound` holds answers as its text parsed again would — but for the
  native parser's `54001` — and `plan_cache`'s `Stamp` is what makes it so.** Every input a
  parse or the parameter inference reads must be a stamp field (`Schema::generation`, the
  thread's depth limit and unknown-type policy, natively its stack bounds) or empty the cache
  (`schema_mut`, `set_stack_budget`); an input that is neither is a held plan answering wrong
  with no error, unseen by the differential fixtures. `docs/param_binding.md` § The bound
  entry point's plan cache.
- **A pending set-op column's type is never read.** A side naming an **enclosing** level
  cannot be typed as it is parsed, and the provisional type goes to a reader list that is
  not enumerable — so the correlated re-walk's answer is recorded (`SetOpOuterTypes`) and
  the statement is **parsed a second time** with it. The returned plan holds no pending
  column; one the re-walk never unified is refused rather than returned.
  `docs/scope.md`, `docs/coercion.md`.
- **An `IN` list is converted to 18.6's one-type array form only where every operand's
  type is certain by construction, and is an `=` an item everywhere else.** Certain is a
  plain base-table column at the list's own level against two or more written constants;
  a list converted on an inferred type matches rows 18.6's does not (`EDGES.md` row 280).
  `docs/coercion.md`.
- **A scalar subquery is a *typed* branch of a `COALESCE`/`CASE`/`ARRAY`/`greatest`/
  `least`/`nullif` wherever its type is resolved, and untyped — deferring the unification —
  where it is not.** An invisible branch is otherwise cast to its sibling's type, and
  `COALESCE((SELECT 1.25), 2)` answered `1`. Resolved is what its column references
  resolve to, a guess where the sub-plan's column is one, and not the next rule's certain:
  a subquery whose column is certain is cast from the sub-plan's record, an operand under
  a branch as well as a branch, and the resolving reader is asked only for the rest.
  A branch set the gate can type is judged as 18.6 judges it; one it cannot keeps
  per-branch runtime types under one declared type — the residual, not the design
  (`EDGES.md` row 152). `docs/coercion.md`.
- **A type is certain where the coercion pass typed its expression as written, and that
  verdict has one producer.** An aggregate's argument and a relation's column are decided
  by the pass's computed-reach reader before the pass rewrites the expression, recorded on
  the plan (`ArgCertainty`, `CertainTypes`) and read from there — a rewritten form reads a
  guessed cast, and the pass's own cast over a branch subquery, as typed, and readers
  deriving it from their own scopes disagreed. A scalar subquery is certain where its
  sub-plan's column is, by the sub-plan's record, wherever it stands — and the pass's own
  readers type it only by the cast written over one under a fold. An unknown — a bare
  string, a `NULL`, an undeclared `$N` — takes its fold's target as a branch; as an
  operand of arithmetic no reader types it, nothing converting it to its sibling's type.
  A bare string compared with an aggregate is read at the aggregate's type where it is
  certain — decided from the written pair at the aggregate's own level, where every column
  its argument names must bind, never on the correlated re-run (`docs/coercion.md` § Where
  the plan-time coercion pass reaches).
  Not certain keeps the answer an unknown type gets (`EDGES.md` row 2), and a fold with a
  branch no reader types takes a computed sibling as its target only
  where the sibling is a `double precision` and the rest numeric (row 54).
  `docs/coercion.md` § A computed branch is typed where its type is certain.
- **An enum's identity reaches the gates that decide whether two operands have a
  comparison operator or a common type, and what those gates model is the `typcategory`.**
  The jsonb key/path, `LIKE`-family, `starts_with`, `||` and array-containment rules and the
  string functions still read an enum as `text`, and it writes (`EDGES.md` row 190). `docs/enums.md`, `docs/coercion.md`.
- **A column reference binds at plan time, and the gate closes only over a
  namespace it can prove whole.** `ColumnScope::check_binding` closes the
  zero-row and post-projection holes the eval chain cannot reach; its verdict
  is three-way (`Bound` / error / `Uncertain`) and "whole namespace" is
  computed, never assumed. `docs/scope.md`.
- **A FROM name is resolved by the item's kind, never by asking the catalog
  first.** A FROM item's effective name *replaces* the namespace entry at its level;
  `expand_qualified_wildcard` is still keyed on the name and wrong for an `unnest` item.
  `docs/scope.md`.
- **A clause's namespace is one object, and every reader of that clause reads it.**
  `RETURNING`'s is the DML target under its effective name plus its `FROM`'s relations
  (`returning_relations`); re-derive its readers from every site that reads a DML plan's
  `returning` items. `docs/scope.md` § A clause's namespace is one object.
- **A relation's column *names* are one object, and a rename is applied where
  each reader reads them.** A CTE's `name(cols)` list rides beside the plan
  through `alias_relation_columns`; both reference kinds re-derive them *at
  execution*; the compiler names only some of the readers. `docs/scope.md`.
- **A statement's recursion is bounded twice: bytes where a walker can fail, levels where
  it cannot.** Natively a fallible walker takes a byte check a level (`54001`); the parser's
  depth count bounds the infallible walks and is wasm's whole bound.
  `crates/pglet_sql/src/depth.rs` and its `stack` module.
- **Correct-or-loud.** A wrong-typed operand, unsupported type, or malformed value fails
  loud (`42883`/`0A000`/`42704`/`22P02`) rather than silently coercing or matching zero
  rows; one rulebook (`planner/legality.rs`) feeds the eval-time gates and the DDL-time
  static checker. A conversion has a **context** as well as a legality: the assignment
  gate refuses what 18.6's assignment rule refuses — an explicit-only `pg_cast` pair, and
  a pair with no `pg_cast` row at all. `docs/coercion.md`.
- **A boolean context resolves its operand to `boolean` where the operand is read, and
  only two things resolve: `boolean` itself, and an unresolved literal.** That is the rule
  above with one target filled in — 18.6's `coerce_to_boolean` is an assignment coercion —
  so every other type is `42804`, and a literal is converted through a `boolin` that is a
  prefix grammar. `docs/coercion.md`.
- **An `AND` / `OR` stops at a deciding left operand in each walk found to evaluate it,
  and a path that evaluates an operand out of that order defers the operand's failure to
  one that keeps it.** `connective_decided_by` is asked by the scalar walk, a grouping
  query's expressions included (the grouping computes a skipped operand's aggregates first,
  as 18.6's `Agg` does, unless the left operand decides by its shape alone); the GIN
  guard, evaluated ahead of every row, hands a failure to its recheck `Filter`. A NULL
  left operand decides nothing. `docs/coercion.md`; `EDGES.md` rows 60 and 172.
- **A canonical type name decomposes once — base, modifier, array suffix — and each of the
  three is then decided by the *type*, never by the spelling.** A modifier a type does not
  carry is **refused** where `typenameTypeMod` refuses it, never dropped — as is one a
  carrier's own typmodin refuses (`22023`); one it does carry is kept, clamped by the
  type's maximum and enforced, element-wise under a `[]`. `docs/coercion.md`,
  `docs/temporal.md`.
- **A string literal is assembled in 18.6's units, and each escape is decoded by the
  grammar 18.6's scanner matches.** A `\xHH` / `\ooo` escape contributes a **byte**,
  everything else a character's UTF-8, and the finished literal is validated once, left to
  right, so the first NUL or illegal byte is the failure. `docs/parser.md` § A string
  literal is assembled in 18.6's units.
- **An array literal is walked by 18.6's array grammar, and each element is converted as it
  is read.** `executor::array_parse` is `ReadArrayToken` and `ReadArrayStr`: a literal that
  walk refuses as malformed is `22P02` whatever the element type, an element's own input error comes
  ahead of a later structural one, and whitespace is `scanner_isspace`'s set only. A
  nested literal is walked the same way and then refused `0A000` (`EDGES.md` row 283).
  `docs/arrays.md` § What works.
- **Text entering the engine is validated in the server encoding wherever it enters,
  through one scan — that is the rule, and it has measured exceptions.**
  `pglet_core::encoding` finds the first NUL or illegal UTF-8 byte and answers `22021` at
  every door measured to reach it. The doors measured to bypass it are `docs/encoding.md`
  § Measured doors that do not reach it (`EDGES.md` rows 130, 133). `docs/encoding.md`.
- **A `DROP` decides between auto-drop and refusal, never between silence and a
  brick.** One resolver (`executor/dependencies.rs`) re-derives `pg_depend`'s edge kinds for
  `DROP TABLE`, `DROP COLUMN`, `DROP SEQUENCE` and `DROP TYPE` — which still drops an enum a
  column declares (`EDGES.md` row 261, open); a constraint's identity is `(table, position)`,
  never its name. `docs/ddl.md`.
- **A failed DDL statement must leave the catalog untouched** — the autocommit path cannot
  put it back, so a catalog-write-then-fail leaves a durable phantom. Everything fallible
  runs before the first catalog write, and a catalog fact the statement itself creates is
  staged locally. **Where** a check runs is a correctness decision here, and "everything
  fallible" reaches the statement's own **work** and every other side it moves — storage,
  an index build — not only its checks. `docs/ddl.md`.
- **A constraint name lives in two spaces, and a generated one is disambiguated
  rather than refused.** One name pool across all four constraint kinds
  (`42710`) plus the `pg_class` relation namespace for a PK/UNIQUE's index
  (`42P07`); `ConstraintNames` reads both as `catalog ∪ claimed`. A declared
  name is the user's to get wrong; an auto-name gets a digit appended.
  `docs/names.md`.
- **A constraint's name is minted once, at execute time, and is the same fact
  everywhere.** The parse side says declared-or-nothing, the catalog side is total,
  `ConstraintNames` is the only bridge, and the minted name is persisted; the mint order
  per route is part of the contract. `docs/names.md`.
- **The same mint rule governs a sequence's name, over a *different* namespace — and a
  sequence is a relation.** `Schema::relation_name_taken` reads tables, indexes, and
  sequences as one pool; a declared `SEQUENCE NAME` is refused (`42P07`), an auto
  `<t>_<c>_seq` disambiguated. `docs/names.md`.
- **An unnamed index's name is the engine's, chosen as a sequence's is, and both are cut to
  63 bytes.** `CREATE INDEX ON t (…)` takes `<table>_<key part names>_idx` through the one
  `choose_relation_name` (18.6's `makeObjectName` cut, a digit on collision); the constraint
  minters share the cut and the digit loop. `docs/names.md` § An unnamed index's name.
- **A table's name is a type name too, and each creating routine asks the namespace
  18.6's does.** `Schema::type_name_taken` holds the enums and each table's row type; a
  table or a sequence over an enum's name — a `SERIAL`'s minted one included — is `42710`
  behind the relation's `42P07`, an enum over a table's is `42710`, and an index asks
  nothing. `docs/names.md`.
- **A relation's column names are a set, and the heap owns some of them already.** No two
  columns of one relation share a name, and none takes `ctid`/`xmin`/`cmin`/`xmax`/
  `cmax`/`tableoid` (`oid` is ordinary, measured) — both `42701`, raised at the
  statements that add a column name (`CREATE TABLE`, `ADD COLUMN`), each where 18.6
  raises it, over names the parser already folded. That enumeration is what `Plan`
  has, not a completeness claim. `docs/names.md`.
- **A statement's target column list is a set.** A column is assigned at most once per
  statement — `42701` for an `INSERT`'s and a `COPY`'s column list, `42601` for an
  `UPDATE`'s and an `ON CONFLICT DO UPDATE`'s `SET` list — through two predicates over two
  keys: `claim_target_column` on the resolved position, `refuse_duplicate_assignments` on
  the folded name. `docs/names.md` § What the refusals do not carry.
- **An `ON CONFLICT` inference list is a set, and matching an index is set equality.**
  Not a sequence: `conflict_target_matches` compares as 18.6's `infer_arbiter_indexes`
  does — the plain attributes as sets, the key expressions by membership both ways — so
  order is irrelevant and a repeat collapses on each half. Two-way containment, so a
  subset still refuses `42P10`. `docs/indexes.md`.
- **A constraint's key list is validated by the constraint, not by the index it
  builds.** `PRIMARY KEY`/`UNIQUE` refuse a repeated key column (`42701`) and a
  foreign key's *referenced* list refuses one (`42830`), while `CREATE INDEX`'s key
  list and a foreign key's *referencing* list accept them — four independent facts,
  the two acceptances held by accept-side controls. Whose pass the key *resolution*
  belongs to is what makes the two PK/UNIQUE routes disagree. `docs/ddl.md`.
- **A gate standing in for *no* Postgres error must run after every gate that stands in
  for one** — including the gates that decide a Postgres *success*, like an `IF NOT
  EXISTS` skip; its mirror is that a gate which does stand in for a Postgres error goes
  where 18.6 raises it (`CREATE UNIQUE INDEX`'s build validation, `23505` inside
  `index_create`). `docs/ddl.md`.
- **A gate that reads the catalog is blind inside the statement that is
  building it.** The question to ask of every gate on the `CREATE TABLE` path:
  *which of the names this statement is creating can it not see?* A name the
  statement will register belongs in the in-flight namespace at the point
  Postgres's own catalog would hold it. `docs/ddl.md`.
- **A sequence's `start` is not its counter, and `owner` is a third fact.** The
  catalog `Sequence` record `{last_value, start, owner}`; a counter update
  moves `last_value` and nothing else; the dump renders it as three statements
  (two for an identity column), and a sequence name inside a
  `nextval('…')`/`setval('…')` literal carries its *identifier* quoting inside
  the string. `docs/sequences.md`.
- **A link Postgres cannot represent is a link pglet must forbid, and the
  resolver that reads it must not assume the forbidding.** The identity-column
  ↔ sequence tie is two ordinary movable facts here; the DDL gates that keep
  them together and the resolver's narrowed exemption are both required —
  neither substitutes for the other. `docs/sequences.md`.
- **Recording a persisted cross-object reference turns every pre-existing leak of the
  referenced object into a rendering bug.** The invariant is **the database state pglet's
  own accepted statements build, through any door whose state a dump path reads, must dump
  to something that loads into stock Postgres and back into pglet** — catalog and rows —
  and where a faithful dump is not possible the export refuses loudly rather than emit SQL
  that restores short with exit `0`. `docs/sequences.md`.
- **A stored expression has two renderings, and which one a surface takes is a correctness
  decision.** pglet stores the expression as written, 18.6 after parse analysis.
  `deparse.rs` is one exhaustive match over two `RenderMode`s: `Portable` (the dump, the
  render probe) keeps `parse(render(e)) == e`; `Catalog` (`column_default`,
  `check_clause`, `pg_get_constraintdef`, `indexdef`) reconstructs 18.6's tree as a
  *rendering*, never a plan normalisation. `docs/deparse.md`.
- **Gating a stored expression is a DDL-time concern.** A CHECK, index predicate or index
  key expression is validated up front in one bottom-up walk — binding, static operand
  typing, the boolean-context gate, a representative-row probe — then a stored predicate's
  clause-root boolean gate, plus a **render probe**: every accepted persisted expression
  must deparse, or one table's `DEFAULT` makes the whole database undumpable.
  `docs/ddl.md`.
- **Index probes are exact-or-decline.** A probe value coerced to the indexed column's type
  substitutes only when exactly equivalent, else the planner declines the index and a row
  filter runs — an inexact probe costs speed, never a result. The foreign-key probe takes
  the same rule except where 18.6 itself casts the child. `docs/indexes.md`.
- **An index range read answers no row whose key is NULL, whatever the index holds.** Its
  lower end is never below the least key a present value has
  (`index_keying::encode_range_bounds`, where both backends and the in-transaction read
  take their ends), so a `NULLS NOT DISTINCT` index's NULL keys are in no range — and a
  read with no bound answers every row only where no key is NULL. An ordered scan is
  planned where the column is *declared* `NOT NULL`, and each statement that sets that
  declaration over existing rows reads them; the identity clause, which reads none, is
  `55000` over a column not already declared (`EDGES.md` row 38 keeps its legacy store).
  `docs/indexes.md` § Index probes are exact-or-decline.
- **A simple `UPDATE` / `DELETE` takes the read path's equality probe only where nothing the
  probe leaves unevaluated can fail, and writes what it finds in `RowId` order.** The probe is
  chosen at plan time by the `SELECT` path's own selector (`dml_target_probe`); a range is declined (a
  B+tree write runs in a storage transaction, where a range lookup reads the whole table); the
  residual must pass `dml_residual_is_certain`'s whitelist, an unbound or operator-less parameter
  declines it at execution, and every candidate is tested against the whole `WHERE`.
  `docs/indexes.md` § A write's target rows.
- **A float key is the `float8` it widens to, in 18.6's `float8_cmp_internal` order, and
  what that order calls equal is one key.** `-0` keys as `0` and every NaN — either sign,
  any payload — as one key above `Infinity`'s, so both widths share a key space and the
  key order is the engine's own float comparator's. `docs/indexes.md` § A float key.
- **A container value's key is the comparator's order in bytes.** A `jsonb` and an array
  key so that `memcmp` of two keys is `values_compare` of the two values, `Equal` included
  — 18.6's `jsonb_ops` and `array_ops` orders, strings by their bytes (`EDGES.md` row 43).
  One shape is refused: a non-unique B-tree over a single bare container column, which
  `index_method_for` derives GIN for (row 25). `docs/indexes.md` § A container key.
- **An index entry is sized as 18.6 sizes it, at every door, in 18.6's order.** One size
  (`executor::index_row`, 18.6's `index_form_tuple` layout) and 18.6's three `54000`s,
  asked, in 18.6's order, of every entry a write, a foreign-key action or a build enters
  (over the table's indexes in creation order) and of every entry storage maintenance adds.
  Residuals: loud, `EDGES.md` rows 191 and 192; silent, rows 95 and 104; orders, rows 92,
  43, 166 and 194. `docs/indexes.md`.
- **Index selection must ask what an index *is*, not only what it covers.** Selection asks
  `executor::index_method_for` — the derivation storage maintains — so a GIN index is
  offered only containment, and asks the key column's **order**: `single_column_index_for`
  declines an enum key column, whose order is member position where the key encoder
  encodes label bytes. `docs/indexes.md`.
- **An enum's order is its members' positions, and it is `Schema.enums[name].labels` that
  says so** — read through `values_compare_ordered`, whose enum arm is a total order over
  member and non-member labels alike. Equality stays on the label bytes, so what moved is
  `< <= > >=`, `ORDER BY`, `MIN`/`MAX` and `GREATEST`/`LEAST` — and the **writes**: a
  `CHECK` over an enum expression and a partial index's membership. `docs/enums.md`.
- **The storage index specs follow `Schema.enums` wherever it moves.**
  `IndexSpec::column_enum_orders` is the one copy of a member order that outlives its
  statement, and a member list *can* move under it where a column's type cannot — so
  `Storage::rebuild_index` re-derives at the two forward DDL sites and after **every
  backward `Schema::restore_from`**, read through the one `enums_moved_by_restore`.
  `docs/enums.md` § A copy that outlives its statement.
- **An enum's labels are a set.** A repeated `CREATE TYPE … AS ENUM` label is 18.6's
  own `23505` on `pg_enum_typid_label_index`, behind the type name's `42710`; a label
  is a string literal and is never folded. What a repeat cost was the catalog and the
  dump — two `pg_enum` rows for one label, and a `CREATE TYPE` stock 18.6 refuses —
  never the order, `enum_rank` being `position()`. `docs/enums.md`.
- **A transaction block is one state machine, and every host drives the same one.**
  `pglet`'s `TransactionSession` owns the aborted block, the savepoint frame stack, whether
  the block holds the writer, and the exit classifier; hosts dispatch on the carried
  `TransactionCommand`, never on the command tag. `docs/storage.md` § The transaction session.
- **A write the device refuses is dropped with its statement where the host can take the
  statement back, and otherwise fails the store.** A dropped one answers `53100`/`58030`
  and the database serves on; otherwise the statement answers the same code and every later
  one `57P02` until a reopen reads the durable slot. `docs/storage.md` § Crash Recovery.
- **At most one connection's block stages writes: it takes *the writer* at its first write
  and holds it until it ends, re-basing every frame on what is committed at that moment.**
  A second writer waits (the server, bounded by `lock_timeout` → `55P03`) or is refused
  `55P03` (the embed). `docs/storage.md` § Why Single-Writer?, § Isolation Guarantees.
- **A sequence counter is not rolled back, and the exception is not a type of statement
  but a replaced *cell*.** A statement that swaps the cell (`RESTART`, a `DROP` that drops
  the sequence, …) restores the value read before it ran (`Plan::swapped_sequences`).
  `docs/sequences.md`.
- **The clock family answers one of two instants fixed for the statement and for the open
  block, and `clock_timestamp()` alone answers a new read at each call.** A host marks each
  statement's arrival (`builtins::start_statement`) and an open block carries its start in
  `SessionSettings`, so the list of marking sites is what goes stale.
  `docs/temporal.md` § The transaction and statement clocks.
- **A client that reads a structured error field gets what 18.6 sends, and a field pglet
  cannot fill truthfully is omitted rather than invented.** One builder
  (`Error::wire_fields`) feeds the native server's `ErrorResponse`, and the wasm error reads
  the same accessors it is built from; the differential harness scores an omitted field
  as agreement, so the unit, executor and host tests are the other half. `docs/errors.md`.
- **A result set of zero columns is not the absence of a result set.** Whether a statement
  returns one is carried as its own fact (`ResultShape`), never inferred from a column
  list's emptiness. The fact has two producers — `Plan::output_columns_scoped` and the
  executor's `QueryResult::shape` sites — and nothing compares them.
  `docs/declared_types.md` § Whether there is a descriptor at all comes first.
- **Which error wins is part of the contract, at three scales** — within an expression,
  between a statement's clauses (with a later tier beneath them for the gates 18.6 runs in
  its rewriter), between protocol phases. Each takes one interleaved walk; no pair of
  whole-tree passes reproduces any of them. Adding a gate is a change to statement-level
  error precedence every time. `docs/ddl.md` § Which error wins.

The **generic-conformance bar** (`PLAN.md` §7) — every common gap needing no
design decision and no size cost, closed to spec with differential tests — is
**met**, locked by `tests/gates_baseline.txt`; what remains open is tracked as
`EDGES.md`'s live rows and `docs/compatibility.md`'s ⚠️/🔲 rows. All the non-spine column
types land (DATE, NUMERIC, TIME, INTERVAL; an unrecognized type fails loud, not
a silent coerce).

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
  pglet_sql/       # SQL front-end: lexer + native parser (docs/parser.md), planner, executor, catalog. Single-home load-bearing modules:
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
- **`timestamptz` text encoding is load-bearing:** only the *column* OID carries
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
deno task test:power-loss         # dm-flakey power-loss rounds (root-only; skips honestly without the device)
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
