# Prepared-statement parameter binding

How pglet binds extended-protocol parameters: infer at Parse, decode at Bind,
resolve at Execute. The shapes this path carries — `WHERE id = $1`,
`id = ANY($1::uuid[])`, `refs @> $1`, `unnest($1, $2)`, `WHERE $1`,
`LIMIT $1 OFFSET $2` — are pinned by `-- @param` fixtures, which sit beside their
subject rather than in a directory of their own; § Test coverage names where the
clusters are, and `grep -rl -- '@param' tests/fixtures/` is the enumeration.

## Why infer at Parse

Idiomatic tokio-postgres (the client the differential harness drives) sends
`Parse` with *empty* param types, then `Describe` — relying on the server to
infer them and report via `ParameterDescription`, which the client then uses to
binary-encode each value. A server that can't type an untyped `WHERE id = $1`
(uuid) reports OID 0, the client mis-encodes, and the server mis-decodes — so
Parse-time inference is load-bearing for any binary-protocol client, not a nicety.

## Design: infer at Parse, decode at Bind, resolve at Execute

A **params slice threaded through the executor** as an evaluation environment
(`EvalEnv`), not a plan rewrite: `infer_param_types` still types each `$N` at
Parse, but the decoded `Value`s travel in the env and an `Expr::Parameter` reads
`params[N]` at eval time (`execute_plan_node` / `eval_expr_env` carry the env). The
plan is never mutated to bind, so a prepared statement keeps one immutable plan.

- **Parse** (`pglet_server` handler) — parse + plan the statement, then
  `infer_param_types(&plan, &schema, hint)` (`pglet_sql/src/planner/param_types.rs`)
  walks the plan and types each `$N` by priority: client-declared OID (authoritative,
  as in Postgres) > explicit cast > context (an `INSERT … SELECT` entry's target column
  included) > the `text` of a bare select-list or `RETURNING` entry. A parameter the statement leaves without a type is refused there
  (§ A parameter left without a type is refused). A client-declared OID is part of the plan
  too, not only of the report: § A declared type is part of parse analysis. (A
  statement the planner refuses raises its error at `Parse` rather than advertising zero params and failing the later
  `Bind` — `params/returning_gate_error_at_parse.sql` /
  `params/set_rhs_gate_error_at_parse.sql` pin it.) Context = column comparison (`col <op> $N`), `ANY`/`@>` element/array
  type, `IN` list (an item takes the left operand's type — [coercion.md](coercion.md) § An
  `IN` list is compared at one type), INSERT/UPDATE targets (via the existing coerce-to-column cast),
  `LIMIT`/`OFFSET` (→ `int8`), `||` / `LOWER()` arguments (→ `text`), a **boolean
  position** (→ `bool`, below), a unary `+` operand (→ `float8`, the resolution 18.6
  reaches too), a `jsonb` key or path operand (`?` / `->` / `->>` → `text`,
  `?|` / `?&` / `#>` / `#>>` → `text[]`), a bare
  `$N` in an `UPDATE … SET col = …` value — including a `CASE`/`COALESCE` result
  branch — typed from the target column. (That probe's own reach is narrower than it
  reads: where a branch has a *typed* sibling the lowering's branch unification has
  already coerced the parameter and the ordinary cast rule types it, so what the probe
  carries is an **all**-unknown `CASE`/`COALESCE` — measured on both the `UPDATE` and
  the `ON CONFLICT` arms, `SET v = CASE WHEN $1::bool THEN $2 ELSE $3 END` reports
  `$2`/`$3` as OID 0 without it and `{boolean,text,text}` with it, matching 18.6.) An
  `ON CONFLICT`
  clause at every position it parameterizes, which are these — the arbiter's key parts
  and its partial-index predicate, the `DO UPDATE` `SET` list (the assignment-target
  rule again, `CASE`/`COALESCE` branches included), and the action's `WHERE`. The
  **action**'s two positions are typed against a namespace of their own —
  `on_conflict_action_relations`, the target plus `excluded` carrying the target's
  shape — rather than against the catalog, so an `excluded.<col>` reference reaches
  the pseudo-relation even when a real table is *named* `excluded` (typed off the
  catalog instead, that shadow table would answer: `{text}` for `WHERE excluded.n = $1`
  against 18.6's `{bigint}`, and the bound statement then fails loud `42883
  operator does not exist: bigint = text` where 18.6 updates the row). The
  **arbiter** stays outside that namespace, which is 18.6's rule and not caution:
  `ON CONFLICT ((excluded.id)) …` and `ON CONFLICT (id) WHERE excluded.n > 0 …` are
  both `42P01 invalid reference to FROM-clause entry for table "excluded"` there.
  And a `RETURNING`
  item, typed against that clause's own namespace (`Inference::walk_returning` over
  `returning_relations` — the DML target under its effective name plus the relations
  an `UPDATE … FROM` introduces), so a source-qualified `$N` context types from the
  source column. Those clauses are walked in `transformInsertStmt`'s own order — row
  source, then `ON CONFLICT`, then `RETURNING` — and the order is observable, because
  a `$N` written in two of them keeps the earlier one's type, on both engines
  (`Inference::record` keeps the first fact at equal priority). *At equal priority* is
  the load-bearing qualifier: an assignment right-hand side's coercion cast records at
  the higher cast priority and so outranks an earlier clause's context fact whatever
  the order — one collision escaping the rule, recorded with its measured shapes in
  [compatibility.md](compatibility.md)'s row for this feature. The walk descends into a **data-modifying CTE**'s DML
  body (`Plan::With`), so a `$N` inside `WITH x AS (UPDATE … SET c = $2 WHERE k = $1
  …)` types from that CTE's table. A column-comparison context is **alias-aware across
  a JOIN**: the walk carries a per-scope FROM alias→table map (reset at each subquery
  boundary), so a qualified `$N` context like `act.id = ANY($1)` over `FROM actor act
  JOIN account a` resolves `act` → `actor` to type `$1` as `uuid[]` (a single-table
  context can't) — **while the alias names no real relation**: `infer_col_type` tries the
  catalog (`TypeScope::Catalog`) before that map, so a FROM alias colliding with a table
  name is typed from *that table's* column instead. Measured with `dt(id INT, b BIGINT)`
  and `ds(id INT, b INT, k INT)`: `SELECT ds.b = $1 FROM dt ds` reports `int4` here and
  `int8` on 18.6, so a text bind of `5000000000` is `22P02` here and `f` there, while the
  same statement aliased `zq` agrees. **The `ON CONFLICT` arbiter is the same class in
  its extreme form** — it resolves no names at all, so its qualifier is a plain catalog
  lookup: over `oc_probe(id INT PRIMARY KEY, v TEXT, n BIGINT)` with an unmentioned
  `oc_other(id INT, n SMALLINT)` in the schema, `ON CONFLICT (id) WHERE oc_other.n > $1
  DO NOTHING` reports `{int2}` — *that* relation's column, which is what identifies the
  lookup rather than a fallback — while a name the catalog lacks (`zzz.n`) falls back to
  the target and reports `{int8}`. 18.6 refuses both `42P01` at Parse. Recorded with the
  clause's other residuals in [compatibility.md](compatibility.md)'s row for this feature.
  **An `excluded` reference inside a *subquery* in the action** loses the
  pseudo-relation the same way: `Inference::walk_scoped` clears the clause namespace at
  every subquery boundary, and where a cleared `RETURNING` scope degrades to a catalog
  lookup that re-finds an outer relation which *is* a catalog table, `excluded` is a
  catalog table under no spelling. Over
  `oc_probe(id INT PRIMARY KEY, v TEXT, n BIGINT)` and `oc_src(id INT, v TEXT)`,
  `… DO UPDATE SET v = 'z' WHERE EXISTS (SELECT 1 FROM oc_src s WHERE s.id = $1 AND
  excluded.n = $2)` reports `{integer,0}` here against 18.6's `{integer,bigint}`, and
  `{integer,text}` with a real `excluded(id INT, n TEXT, v TEXT)` beside the target
  that 18.6 ignores; the control is the same comparison written directly in the
  action's `WHERE`, `{bigint}` on both engines either way. It is **masked** rather
  than harmless — at a conflicting key the action's `EXISTS` is pglet's own `0A000 …
  requires execution context` at execute and the statement fails there, and at a free
  key the action is not evaluated at all and the `INSERT` succeeds on a `VALUES` row
  `$2` does not reach, so the OID reaches a `Describe` reply and a `Bind` decode and
  no row carries a value read under it. No fixture pins it, for a reason that
  outlives this shape: the harness compares output-column declared types and never
  reads `ParameterDescription`, so a parameter OID reaches it only through a `Bind`
  that fails.
  **No type crosses an arithmetic operator** — nor its call spellings, `mod($1, 5)` and
  `power($1, 2)` — which is worth stating beside the contexts
  above because it removes the canonical upsert idiom from the bindable set: `SET n =
  <rel>.n + $1` (and the plain `UPDATE … SET n = n + $1`) reports OID 0 where 18.6 types
  it from the operator, and the parameter then decodes as text and the statement fails
  at execute — `42804 type error: operator + requires numeric operands`. The explicit
  `$1::int8` spelling binds; so does a bare `SET n = $1`, which the assignment-target
  rule covers. A filter on a **virtual-table** column (`WHERE table_name = $1` over
  `information_schema.*` / `pg_catalog.*`) types from the catalog view's column list
  (text): `column_resolved_in_relation` resolves a column against real `Schema.tables`
  **then** virtual catalog tables (`CatalogTable::from_name(relation)?.columns()`), so
  a binary-protocol client never sees OID 0 there (the `params/virtual_table_filter`
  fixture guards it).
  **A boolean position types a bare `$N` as `bool`**, which is 18.6's
  `coerce_to_boolean` resolving an unknown `Param` there. A clause root hangs on no
  node, so `record_bool_clause` runs where the plan walk reaches one; a **node**
  position is swept out of `Expr::for_each_child_ctx` rather than off a second arm
  list, so the inference and the gate share one enumeration of node positions rather
  than carrying two that must agree. Recorded at context priority, so an explicit
  `$1::int` still outranks it and then fails the gate, which is 18.6's answer too —
  at `Parse`, before any `ParameterDescription`, so what pins that priority is
  `boolean_context_verdicts.rs` rather than the wire. Read off
  `ParameterDescription`, these describe `{bool}` on 18.6 and here:
  `WHERE $1`, `WHERE $1 AND a = 1`, `WHERE NOT $1`,
  `WHERE $1 OR a = 1`, `CASE WHEN $1`, `HAVING $1`, `JOIN … ON $1`,
  `UPDATE … WHERE $1`, `DELETE … WHERE $1`, and an `ON CONFLICT … DO UPDATE` action's
  `WHERE $1`. The **`IS [NOT] TRUE` family is a measured residual**: its gate is an
  ordinary post-order check rather than a child context, so `for_each_child_ctx` hands
  that operand no context and the **inference** does not reach it, though the gate
  still does (`argument of IS TRUE`) — `WHERE $1 IS TRUE` describes
  `{bool}` on 18.6 and `{0}` here, which costs a binary-protocol client the same OID 0
  the OID-0 gap below describes. A **stored** predicate is out of reach
  rather than missed: `CREATE TABLE t (a INT CHECK ($1))` and
  `CREATE INDEX i ON t (a) WHERE $1` describe **zero** parameters on both engines.
  The result is stored on the prepared statement and is the single
  source of truth for both the `ParameterDescription` reply and the Bind decode.
- **Bind** — `decode_param(bytes, oid, format, param_number)` (`pglet_server/src/encoding.rs`)
  turns each value into a `Value`. Text format first passes the bytes through
  `pglet_core::encoding::text_from_utf8`, the server-encoding scan, whatever the
  parameter's type, and then routes through `coerce_value` (same parsing as `::text`
  casts); binary decodes via `value_from_binary`, the inverse of `value_to_binary` /
  `encode_array_binary` (scalars + one-dim arrays, NULL elements), which reads the
  payload as 18.6 reads it (§ A binary parameter is consumed exactly) and whose
  `text`/`varchar` and `jsonb` arms run the same scan, an array's text elements
  reaching the `text` arm one at a time. A NUL or an illegal UTF-8 byte the scan finds
  is `22021` (`docs/encoding.md`). Stored as the
  portal's `bound_values: Vec<Option<Value>>` (NULL = `None`).
- **Execute** — the portal's `bound_values` become an `EvalEnv::with_params(...)`
  threaded into `execute_with_storage`. An `Expr::Parameter { index }` resolves to
  `params[index - 1]` at eval time; a `LIMIT`/`OFFSET` `$N` resolves from the env in
  the `Plan::Limit` arm. `Plan::Limit` carries `LimitCount::{Const,Param}` so a `$N`
  limit survives planning. The plan itself is the cached, immutable one from Parse
  (re-planned when the schema generation has moved since Parse: a DDL, or a rollback that
  took one back).

Binding is one engine path reached two ways. **Over the wire** it is
**server-side**: the server decodes the portal's params (`decode_param`) into the
portal's `bound_values` and threads them through `execute_with_storage`.
**In-process** the embed `Database` facade exposes the same path via
`execute_bound(sql, conn_id, bind)` — it parses once (and may reuse the parse of a
text it already parsed under the same catalog and thread settings), infers each `$N`
(`pglet_sql::planner::infer_param_types`), hands the inferred OIDs to a `bind`
callback so the caller marshals its own values, and threads the result over the
same `EvalEnv::with_params`. The **wasm hosts** — `PgletBtree` and, in a build with the
crate's `opfs` feature, `PgletBtreePersistent` — run their `exec` and `query` through `execute_bound` with no
parameters, and `PgletBtree` alone takes bound ones, through `query_params` (JS
`queryParams`). The `bind` callback is `pglet_wasm`'s `bind_js_params`, which for each
parameter runs `refuse_text_outside_the_server_encoding` — a JS string holding a NUL is
`22021` — and then a JS→`Value` decoder reusing the `coerce_value` codec
(`pglet_sql::executor`). Those two are the wasm-specific part of binding, with parse,
inference and execution owned by the facade.

### The bound entry point's plan cache

`Database::execute_bound` holds, per database and keyed by the statement's text, what it
computes before binding — the one plan and the parameters' inferred targets
(`crates/pglet/src/plan_cache.rs`). A hit stands in for the parse, the statement count and
the inference, and for nothing else: everything the entry point does besides — the
aborted-block gate, the session commands, the ambient session's install, binding and
execution — runs on a hit as on a miss.

**A cached plan answers for the catalog and the settings it was planned under, and the
stamp is what says so.** The cache holds plans for one stamp — the catalog's
`Schema::generation`, the calling thread's depth limit and unknown-type policy, and
natively the thread's stack bounds — and a lookup under another empties it. So a catalog
change any parse or inference reads must move the generation or empty the cache. Executing
a plan that `Plan::mutates_schema` moves the generation — a `CREATE SEQUENCE` among them,
since inference asks whether a relation of that name exists — and so does a restore of a
catalog snapshot taken at another generation (`Schema::restore_from`: a block's rollback, a
savepoint's, a `DO` block's undo). A snapshot taken at the live generation differs from the
live catalog in nothing a parse or inference reads, so its restore keeps the generation and
the cache keeps its plans — the rollback of a block that ran only DML — except in a block
the host's `Database::schema_mut` reached, whose rollbacks move it. `Database::schema_mut`,
which can change anything or replace the catalog without moving the generation forward,
empties the cache, as `Database::set_stack_budget` does; a `Database::fork` starts empty. Any other input the parser or the inference starts
reading must become a stamp field or empty the cache where it moves; an input that does
neither is a held plan answering for a catalog or a setting it was not planned under, with
no error.

**Bounds.** At most 128 texts (`CAP`), none longer than 4 KiB (`MAX_TEXT_BYTES`), at most
64 KiB of text held (`HELD_TEXT_BUDGET`); the statement that would pass either total
empties the cache first. Not held: a plan that
changes what a parse reads (`Plan::mutates_schema`), a text whose parse, statement count
or inference fails, and natively, on a thread whose depth limit is above
`pglet_sql::depth::FALLBACK_MAX_DEPTH` (the limit sized for the smallest stack a thread has
by default), a plan deeper than that limit. A plan whose binding or execution fails is
held. The bounds are on text, not on plans: a short `SELECT *` over a join of wide tables
holds a plan with every column.

**What a hit does not answer: natively, the parser's byte check.** A hit runs no parse, so
it is not refused `54001` where its parse would be: a statement held from a call made
higher in the caller's stack, called again from deeper; and a statement run on one
database from inside another's `bind` callback, which parses under the outer database's
stack budget — a budget no stamp holds, so a plan held under one budget answers under
another. Its execution's own checks still run. The wasm build has no byte check.

**What reaches it.** `execute_bound` alone — a native embed's call, and the wasm hosts'
`exec`, `query` and `queryParams`, so the npm package. `Database::execute` / `execute_as`
and the native server never consult it, so the differential fixtures cannot see it; its
guards are `crates/pglet`'s own tests (`plan_cache.rs`'s among them) and the wasm suite.

### A declared type is part of parse analysis

A type the client declares at `Parse` is the parameter's type in parse analysis, as
18.6's `exec_parse_message` makes it: the Parse message's OIDs seed the array
`variable_paramref_hook` reads (`parse_param.c`), so a declared `$N` is a `Param` of that
type from its first reference, and the operators, functions, `select_common_type`
resolutions and result columns over it proceed from the declared type. The native server
plans through `parser::parse_single_with_param_types`, which reads a declared `$N` as
`Expr::Cast { Parameter, <type> }` where the **expression** parser reads the token — the
shape a written `$N::<type>` lowers to, so the coercion pass, the gates and both
declared-type walkers (`docs/declared_types.md`) see a typed operand through the arms they
already carry, and cannot split over it. That is where it reaches, and a position that
reads the `$N` token some other way does not see the declaration: `LIMIT` and `OFFSET`
take the token as a `LimitCount::Param` (`parser/select.rs`), so `SELECT n FROM tn LIMIT
$1` declared `text` is `22000 LIMIT/OFFSET must be an integer` at `Execute` against
18.6's `42804 argument of LIMIT must be type bigint, not type text` at `Parse`, and
declared `numeric` bound `1` is that same `22000` where 18.6 answers the row (measured,
the `OFFSET` spelling alike). The bound value is decoded as the declared type, so the cast
converts nothing. Measured on both engines over a raw extended-protocol client:
`coalesce($1, 1)` declared `numeric` and bound `1.5` is `numeric` `1.5`, `boolean` beside
`1` is `42804`, a bare `SELECT $1` declared `timestamptz` is `1184` with its `+00`, and
`DELETE … WHERE n = coalesce($1, 0)` removes the `1.5` row.

Consequences follow from that shape, each measured:

- **An undeclared parameter keeps its inference**, and is still a bare `Expr::Parameter`.
  Where that inference reads its sibling's type it reads the declared one, as on 18.6:
  with `$1` declared `int4` and `$2` undeclared, `$1 = $2`, `$1 < $2`, `$1 IN ($2)`,
  `$1 BETWEEN $2 AND $3`, `coalesce($1, $2)`, `greatest`/`least`, `CASE WHEN … THEN $1
  ELSE $2 END`, `ARRAY[$1, $2]` and a set operation's `SELECT $1 UNION SELECT $2`
  describe `$2` as `int4`, and `$1 || $2` / `$1 LIKE $2` declared `text` as `text`
  (measured, each agreeing with 18.6). **It is not every construct**, because pglet's
  inference has no rule there to read the declaration with — measured against 18.6's
  `{int4,int4}`, each of these describes `$2` as `0` and then refuses at `Execute`:
  `nullif($1, $2)` and `CASE $1 WHEN $2 …` (`42883 integer = text`), `$1 + $2`,
  `$1 - $2` and `$1 * $2` (`42804 operator + requires numeric operands`). `$2 IN ($1)`,
  the undeclared parameter on the left, is `{numeric,0}` and `42883` against
  `{numeric,numeric}`, and over an `INT` column `a IN ($1, $2)` with `$1` declared
  `numeric` describes `$2` as `int4` against 18.6's `numeric` (the column's type, not the
  list's common type), so binding `1.0` and `1.5` is `22P02` there where 18.6 answers the row `1`.
  `$1 = ANY(ARRAY[$2])` is `42883` on both engines, pglet's at `Execute` after describing
  `{int4,0}`, 18.6's at `Parse`.
- **A re-plan keeps the declared types.** The prepared statement and its portal carry the
  Parse message's own OIDs (`declared_param_types`) apart from the inferred list, and a
  plan made stale by a catalog change (a DDL, or a rollback that took one back) is rebuilt
  with them at `Describe` and at `Execute`.
- **A declared parameter is still an index probe**: `is_row_independent` admits a `Cast`
  directly over a `$N`, and `probe_key_value` exact-or-declines its value as it does a
  bare parameter's (`docs/indexes.md`). Declined, a point read over a 100 000-row key
  measured about 19 ms against 0.05 ms.
- **A string literal beside a declared `$N` resolves to the declared type at a
  comparison and in a common-type construct** — `$1 = '1.50'` and the other comparison
  operators, `BETWEEN`, `CASE $1 WHEN '1'`, `coalesce`, `greatest`/`least`, `CASE … THEN`,
  `ARRAY[$1, '6']` and a set operation or `VALUES` list beside it (measured, each agreeing
  with 18.6) — through the comparison coercion's cast anchor and the common-type pass. Not
  in every position: `$1 IN ('1')` and `nullif($1, '1.5')` are `42883` here, and
  `$1 = ANY('{1,2}')` is `0A000`, where 18.6 resolves each literal to the declared type and
  answers; `$1 + '2'` is `42804` against 18.6's `3`. Each of these refuses the same
  spelling with a written `x::int4` in place of the `$N`, so they are the literal
  resolution's residuals rather than the declaration's.

The in-process doors declare nothing: `Database::execute_bound`'s `bind` callback is
handed the inferred OIDs and returns values, and the wasm hosts' `queryParams` takes
values only, so every `$N` there is inferred. The fixture directive that reaches this
path is `-- @param-declared` (`tests/README.md`); the guards are the `params/declared_*`
fixtures and `crates/pglet_sql/tests/engine_declared_params.rs`, which asserts the
executor walk as well as `Describe`'s. A declared OID pglet does not model still reaches
no analysis — § Deferred / known gaps.

### A parameter left without a type is refused

18.6 gives a parameter reference the type the parameter has **when that reference is
analysed**, and some positions coerce nothing: an `IS [NOT] NULL` operand (and
`IS [NOT] DISTINCT FROM NULL`, which its parser rewrites to one), and an argument
declared `"any"` — `count`'s without `DISTINCT`, `concat`'s, `concat_ws`'s past the
separator, `jsonb_build_object`'s. A reference left `unknown` there is never revisited,
so the statement is refused with `could not determine data type of parameter $N`:

- **`42P18`** where nothing in the statement types the parameter — `WHERE $1 IS NULL`,
  `count($1)`;
- **`42P08`** where a **later** occurrence does — `($1 IS NULL OR name = $1)` — raised
  from inside parse analysis and so ahead of any `42P18` another parameter earns.

An **earlier** occurrence that types the parameter makes the same position harmless,
which is why the optional-filter idiom is accepted comparison-first
(`(name = $1 OR $1 IS NULL)`) and with a cast (`($1::text IS NULL OR name = $1)`), on
both engines. "Earlier" is parse-analysis order, not source order: `FROM` (a join's two
sides, then its condition), the select list, `WHERE`, `HAVING`, `ORDER BY`, `LIMIT`; an
`INSERT`'s row source, then `ON CONFLICT`, then `RETURNING`; an `UPDATE`'s `WHERE`, then
`RETURNING`, then its `SET` list; a `WITH` query ahead of the statement that reads it;
an `IN (SELECT …)`'s subquery ahead of its left operand. Inside an expression an
operator's or a function's operands are typed when the node is resolved, after each was
read — so `jsonb_build_object('k', $1, 'j', $1::text)` is `42P08` and the two arguments
swapped is not — while a boolean operand (`AND` / `OR` / `NOT`, a searched `CASE`'s
condition) and a simple `CASE`'s operand are typed as they are read. A bare select-list
entry is `text` only once its query level is analysed, a bare `RETURNING` entry once
that list is, and a `VALUES` list's bare entry once its last row is (each measured:
`SELECT $1 AS p, $1 IS NULL` and `SELECT * FROM (VALUES ($1, $1 IS NULL)) v` are
`42P08`). A bare entry of an `INSERT … SELECT` source is not that `text` where
nothing in the source resolves it first: it is coerced to the column it is assigned to
once the source is analysed — `INSERT INTO kv (k, v, n) SELECT $1, $2, $3` describes
`{text,jsonb,integer}` — ahead of `ON CONFLICT` and `RETURNING`; a sort, grouping,
`DISTINCT` or window key naming the entry resolves it to `text` first, as does a set
operation whose sides leave the column unknown (`EDGES.md` row 150). An `INSERT` row's cells are analysed in the order the row
is written — the column list's, whatever the table's — and a `SET` list's in the list's;
their bare entries are coerced to their columns once the whole row or list is analysed.

pglet's inference walk times the two kinds of occurrence (`Inference::first_fix`,
`Inference::first_unresolved`) and compares them per parameter. It follows the order
above except where a plan does not carry it; the measured exceptions are `EDGES.md`
row 276 — an `INSERT` row's or a `SET` list's bare entry is one, timed where it is
written rather than at the end of its row or list. The row's written order is not the
lowered row's, which is the table's: where a column list departs from the table's order
the walk reads it from `InsertRows::Values`' `written`.
The verdict is **not** "inference left the slot untyped": this engine types fewer
positions than 18.6 (§ Deferred / known gaps), and a refusal keyed on that would refuse
statements 18.6 answers — `id = $1 + 1 OR $1 IS NULL` reports OID 0 here, and this rule
refuses it on neither engine. The refusal is `infer_param_targets`' error, raised where
it is called: `Parse` on the native server, with no `ParseComplete`, and
`Database::execute_bound` ahead of its binding callback — the wasm binding reads no
value's JS type for a refused statement — failing an open block either way. A parameter
the client declared at `Parse` as a type this engine models is typed from its first
reference and is not refused.

18.6 raises the refusal once parse analysis is done, so an error parse analysis raises
outranks it. Here it follows planning, which puts it behind the errors the parser and
planner raise (a missing column's `42703`) and ahead of the ones raised at execution.
A relation the catalog lacks is one of those, and where the executor is certain to
report it — a `FROM` clause's or a DML target's — the refusal yields: inference refuses
nothing over such a statement, and `SELECT id FROM nope WHERE $1 IS NULL` is `42P01` on
both engines. An index's name yields the same way (`42809` on both); a sequence's does
not, since 18.6 reads a sequence as a relation and goes on to refuse the parameter
(`42P18` on both). A relation named inside an expression's subquery is reported for a row
that evaluates it and not over an empty table, so the refusal stands there. The errors
it does outrank — that one, a constant's `22P02` — are measured in `EDGES.md` row 276.

Where the plan does not say which of two occurrences came first — a conjunct the
planner lifted into a B-tree index scan, a bare entry the lowering has cast (a set
operation's, a `FROM`-clause `VALUES` list's, an `INSERT` row's, a `SET` list's) — the
statement is accepted; a parameter typed only in a `WITH` query the statement never
reads, or declared as a type this engine does not model, is refused. Each is `EDGES.md`
row 276. Guards:
`tests/fixtures/params/undetermined_param_*`, `crates/pglet/tests/undetermined_param.rs`,
`handlers.rs`'s
`parse_refuses_a_parameter_left_without_a_type_unless_the_client_declared_it`, and the
wasm and npm suites.

`INSERT … SELECT` binds too: `Plan::Insert` carries a bound `source` plan, a
projection `unnest($N)` row-expands via `Plan::ProjectUnnest`, and the native parser
handles the no-FROM `INSERT … SELECT … ON CONFLICT` shape directly
(`tests/fixtures/spine/fact/put_fact_refs.sql` runs it bound end to end).

### A binary parameter is consumed exactly

A binary-format parameter is read the way 18.6's `exec_bind_message` reads it: the
type's reader runs over the payload, and a payload that reader did not consume to the
last byte is **`22P03 incorrect binary data format in bind parameter <n>`**, `<n>` the
parameter's 1-based position in the Bind message. `value_from_binary` takes that number
from `decode_param`, which takes it from the Bind loop's own position. `BinaryReader`
stands in for 18.6's message buffer, and `recv_binary` for the type's `*recv`: it reads
what the type reads and leaves the cursor where it stopped, so whether anything is left
over is its caller's question.

A read past the end is the **message reader's** `08P01`, and which sentence it carries
is decided by the reader the type's `*recv` calls, not by how many bytes it asks for:
`pq_getmsgbyte` says `no data left in message` — `boolrecv` reads through it — and
`pq_getmsgint`, `pq_getmsgint64` and `pq_getmsgbytes` say
`insufficient data left in message`. `jsonb_recv` reads its one version byte through
`pq_getmsgint(buf, 1)`, so an empty `jsonb` payload is `insufficient`, not `no data`.
`text`, `varchar` and `bytea` read everything that is left, so no length is wrong for
them. A value a reader reads in full and then refuses — an out-of-range `time`, say —
earns that refusal ahead of the leftover check, whatever follows it.

**`numeric`** keeps `numeric_recv`'s order, because the order decides which refusal a
malformed body earns: the digit count read **unsigned** (so `0xFFFF` asks for 65535
digits), the weight, the sign — `22P03 invalid sign in external "numeric" value`, before
the scale or any digit is read, so it beats a digit shortfall and a trailing tail alike —
the display scale (`22P03 invalid scale …` for a bit outside `0x3FFF`), then each digit
(`22P03 invalid digit …` outside `0 ..= 9999`). A `NaN` reads and checks the digits its
header declares too. `Infinity` and `-Infinity` pass 18.6's sign check and are accepted
there; `PgNumeric` has no infinity, so pglet refuses a complete one with
`22P02 invalid input syntax for type numeric: "<binary>"`, after the fields 18.6 reads.

**An array** is `array_recv` and `ReadArrayBinary`. The dimension length is gated where
`ArrayGetNItems` gates it, before any element is read —
`54000 array size exceeds the maximum allowed (134217727)` for a negative length or one
above `134217727` — and then **spent**: exactly that many elements are read, so what
follows them is the parameter's leftover rather than further elements, and a payload
short of them is the reader's `08P01`. `ndim = 0` reads no dimension and no element.
Each element's length field below `-1`, or past the bytes left, is
`22P03 insufficient data left in message` — the message reader's words under
`ReadArrayBinary`'s code, so neither the code nor the words alone identify the raiser.
Each element is read by its type's reader over exactly its own bytes: a leftover there is
`22P03 improper binary format in array element <i>`, counted from `1` with NULL elements
counted, and a read past the element's end is that reader's `08P01`. A header whose
element OID names an **array** type `TypeOid::from_oid` recognises is refused as soon as
the OID is read, with
`array_recv`'s `42804 binary data has array element type <oid> (<name>) instead of
expected <oid> (<name>)` — so an element is never decoded as another array, and a payload
nesting array headers cannot make the decoder recurse. An OID pglet does not recognise —
an array type it does not model, such as `oid[]` (`1028`), included — falls back to the
parameter's own element type, which 18.6 refuses below its built-in floor. That fallback
and the header's other fields, a scalar element OID included, are still taken at face
value (`EDGES.md` row 140).

**Two values a reader constructs** follow its 18.6 counterpart too. A `numeric` is
truncated toward zero to its display scale, as `numeric_recv`'s `trunc_var` truncates it
— digits `1, 5678` sent with scale `1` are `1.5` — and a value that truncates to zero is
a positive zero. And an `interval` whose three fields are all at their maximum or all at
their minimum, which 18.6 reads as `infinity` and `-infinity`, is refused with the
`22007 invalid input syntax for type interval: "infinity"` (or `"-infinity"`) the text
path gives those words, since `PgInterval` has neither. The text path does not refuse the
triple itself: spelled as numbers, it is a finite interval there (`EDGES.md` row 143).

**A refusal here fails an open block.** The refusals this section describes are raised at
`Bind`, and an error at `Bind` fails an open block (`docs/storage.md` § The transaction
session). Where 18.6 refuses the same payload it fails its block the same way. Where 18.6
accepts it — the interval sentinels above, and members `EDGES.md` row 140
records — pglet's refusal discards what the block wrote and 18.6 keeps it.

No `test_fixtures` fixture can hold this: `@param` binds through tokio-postgres, which
encodes each value at its own width, and the directive grammar has no raw-bytes form.
The guards are `binary.rs`'s `a_fixed_width_value_is_read_to_its_width_and_no_further`,
`a_binary_numeric_is_read_in_numeric_recvs_order`,
`a_binary_numeric_infinity_is_refused_after_its_fields_are_read`,
`a_binary_arrays_dimension_length_is_spent`,
`a_binary_arrays_dimension_length_is_gated_before_it_is_spent`,
`a_binary_array_element_is_read_to_its_own_length`,
`a_binary_array_header_naming_an_array_element_type_is_refused`,
`a_binary_numeric_is_truncated_to_its_display_scale` and
`a_binary_interval_infinity_is_refused_as_the_text_path_refuses_the_word`, plus `handlers.rs`'s
`a_binary_param_is_consumed_exactly_and_refused_at_bind_with_its_number`, which binds a
refused **second** parameter behind a well-formed first and reads the code and message
off the wire at Bind.

### A binary `jsonb` payload's first byte is its version

A binary-format `jsonb` parameter's **first byte is its version number, read
unconditionally**, and only `1` is supported; the JSON text is everything after it.
That is 18.6's `jsonb_recv` — `pq_getmsgint(buf, 1)` and then
`pq_getmsgtext(buf, buf->len - buf->cursor, …)` — so there is no such thing as a
payload sent *without* a version byte: whatever the sender meant by its first byte,
that byte is the version, and `{"a":1}` binds as version 123 (`0x7b`), `123` as
version 49, `"hi"` as version 34.

A version byte that is not `1` is
**`XX000 unsupported jsonb version number <n>`**, with `<n>` the byte's *unsigned*
decimal value — `0` through `255`, so `0xff` prints `255`. The `internal_error` class
is not a tidier code passed over: nothing on that path calls `errcode()`, and
`errstart` selects a default from the elevel — `ERRCODE_INTERNAL_ERROR` for anything at
`ERROR` or above. `elog` is not what makes that happen; it expands to
`ereport(elevel, errmsg_internal(...))`, so an `ereport(ERROR, ...)` written with no
`errcode()` of its own lands on the same default. pglet reproduces the code 18.6 sends,
and this is the engine's only `XX000` producer. 18.6 also sends a `W`
(`unnamed portal parameter $1`) with it and pglet sends no `W` field anywhere
(`EDGES.md` row 76).

**The split runs ahead of the server-encoding scan**, which is 18.6's own order and is
what keeps a `0x00` or a high leading byte a version number rather than a byte the
encoding scan gets to judge. Behind a version byte of `1` the remainder — never the
whole buffer — goes through `pglet_core::encoding::text_from_utf8` and then the jsonb
reader exactly as before, so a NUL or illegal UTF-8 byte in the *text* is still `22021`,
and text the reader refuses carries whichever of the reader's own codes it earns —
`22P02` where it cannot parse the text, and, where it parses and then refuses, that
refusal's own code: `22P05` for a `\u0000` escape and `22003` for a number past the
numeric cap are the two measured here.

An **empty** payload has no first byte to read: `08P01 insufficient data left in
message`, from inside `pq_getmsgint`, before any version logic (§ A binary parameter is
consumed exactly).

**Reach.** One door reaches `value_from_binary` from outside the `binary` module —
`decode_param`'s `format.is_binary()` branch — so the rule is reachable only through
the native server's extended protocol; inside the module `decode_binary_array` reaches
the same arm again through `recv_binary`, once per array element, and never recurses
further: an array header whose element OID names an array type pglet recognises is
refused, and one it does not recognise falls back to the parameter's scalar element type
(§ A binary parameter is consumed exactly). The wasm hosts bind from JS through the text path,
and `COPY … BINARY` parses but fails loud `0A000` at execution without reaching a
binary decoder. A **`jsonb[]` element** does reach the arm — but on a condition of its
own, because `decode_binary_array` takes the element type from the **wire array
header**: the gate runs on an element only where that header names `jsonb`, or names an
OID pglet does not recognise and the fallback — the parameter's own element type — is
`jsonb`. A header naming another scalar type pglet recognises decodes every element as
*that* type and never reaches this gate; the header's element OID is checked against the
parameter's type only for an array type pglet recognises, and the rest is `EDGES.md` row
140.

**18.6 reaches its version byte on a narrower condition, and the two conditions cross.**
`array_recv` refuses a header whose element OID differs from the expected one only where
**both** OIDs are below `FirstGenbkiObjectId` (`10000`); at or above that floor it assigns
the expected element type and carries on, which is pglet's fallback exactly. So where the
header names `jsonb`, and where it names an unrecognised OID at or above the floor, the
two engines run the same version gate and were measured agreeing at every leading byte
probed — the `01` control, the three no-version-byte spellings above, and `0x20`, `0x00`,
`0xff` and `0x02`. Where it names an unrecognised OID *below* the floor, 18.6 has already
refused the whole parameter `42804` — measured at `9999`, the correct-version control
included — and pglet's fallback runs this gate instead. A check added here must reproduce
that condition rather than validate the header unconditionally, or it refuses what 18.6
accepts.

No `test_fixtures` fixture can hold any of this: `@param $N jsonb '<literal>'` binds
through tokio-postgres, which constructs a well-formed payload, and the directive
grammar has no raw-bytes form. The guards are `binary.rs`'s
`a_binary_jsonb_payloads_first_byte_is_its_version` and
`a_binary_jsonb_array_element_carries_its_own_version_byte`, plus `handlers.rs`'s
`bind_binary_jsonb_param_with_a_bad_version_byte_errors_xx000_at_bind`, which proves
the refusal reaches the wire at Bind.

### Equality coverage

`values_equal` carries uuid / bytea / timestamp / jsonb / float arms — load-bearing
for `WHERE id = $1` and `id = ANY(...)` over those column types (a missing arm
silently matches nothing).

## Test coverage

- `test_binary` extended-protocol cases (via tokio-postgres): `ANY(uuid[])`
  (cast + inferred), `@>` (bound array + `ARRAY[$n]`), `unnest`,
  untyped `WHERE id = $1`, `$1::text::jsonb`,
  `$1::text::timestamptz`, `$1::text::<enum>`, tri-state optional filter.
- Differential vs real PG: `tests/fixtures/params/*` (uuid eq, `ANY`, `@>`,
  `unnest`, optional filter, `LIMIT/OFFSET`, jsonb cast,
  the `ON CONFLICT` positions — the action's `SET`/`WHERE` under
  `on_conflict_do_update_bound` / `on_conflict_excluded_qualified_bound`, the
  pseudo-relation against a real table of that name under
  `on_conflict_excluded_not_shadowed_by_a_real_table`, and the arbiter under
  `on_conflict_arbiter_key_expr_bound` — and the errors-raise-at-`Parse` pair)
  + `tests/fixtures/spine/*`, via the `-- @param` fixture directive (see
  `tests/README.md`). Those two directories are clusters and not the set — a
  `-- @param` fixture lives beside its subject. The `RETURNING` clause scope's are
  `update/returning_from_source_param_type.sql` and
  `update/returning_from_source_only_param_type.sql`; the boolean position's are
  `bool_context/where_bound_param_is_boolean.sql`, its negative control
  `bool_context/where_bound_param_false_matches_nothing.sql`, the node-position case
  `bool_context/and_bound_param_is_boolean.sql`, and the explicit cast's
  `bool_context/where_bound_param_explicit_cast_still_wins.sql`.
- A parameter left without a type: the `params/undetermined_param_*` fixtures, the
  refusals beside neighbours 18.6 answers (§ A parameter left without a type is
  refused).
- A client-declared type (`-- @param-declared`): the `params/declared_*` fixtures —
  `COALESCE`, `CASE`, the variadics, `ARRAY[…]`, the `42804` refusals, the
  `DELETE`/`INSERT`/`UPDATE` write shapes, a declared parameter beside an undeclared one,
  the result-column types, and `@profile-lock` index probes.
- `spy_proxy` parses Parse/Bind/ParameterDescription off a live extended-protocol
  client's traffic (`deno task spy:spine:{cell,fact}` are the recording tasks),
  unit-tested in `tools/spy_proxy/main_test.ts`.

## Deferred / known gaps

- **A parameter 18.6 types and this engine has no rule for reports OID 0.** A `$N` in a
  position the inference does not reach is reported as OID 0 and decoded as text by the
  native server, or by its JS runtime type by the wasm binding — where a JS number is an
  `int8` if it is a whole number an `int8` holds and a `float8` otherwise, so a whole
  number of magnitude 2^63 or more answers as a float where 18.6 refuses it or answers
  exactly (measured through the npm adapter: `SELECT i + $1` over an `int4` column and
  `SELECT $1 + 1`, bound `2 ** 63`, answer `9.223372036854776e+18` under declared OID
  `23` where 18.6 is `22003`, and bound `1e21` answer `1e+21` where it is `22P02`;
  `SELECT n + $1` over a `numeric` column bound `1e21` answers `1e+21` where 18.6
  answers `1000000000000000000001`; `SELECT -$1` likewise answers, where 18.6 is
  `42725` — `EDGES.md` row 288, which also holds `nullif`'s second argument, where a
  fractional JS number changes the type of the value handed back). A typed parameter does not: `$1::numeric` answers as 18.6 does and an `int8`
  target refuses (`22P02`; 18.6 `22003`), while a `float4` target bound
  `Number.MAX_VALUE` answers `Infinity` where 18.6 is `22003`. Postgres never reports
  OID 0 for a used parameter. (What it settles without a context is settled
  here too: a bare select-list or `RETURNING` entry is `text`, and a parameter nothing
  types is refused — § A parameter left without a type is refused.) pglet's OID 0 trips
  a **binary-protocol** client (tokio-postgres) during type introspection — a
  text-protocol client (node-postgres) tolerates it. That introspection query joins
  `pg_catalog.pg_range`, a relation pglet does not carry, so the lookup is `42P01` here;
  inside a transaction block that aborts the block and every later statement reports
  `25P02`, and the driver's own `UNDEFINED_TABLE` fallback query does not rescue it
  (measured — `EDGES.md` row 57's bullet).
  Where this engine *does* type a `$N` is sketched under **Parse** above and in the
  **Function-signature inference** bullet below; neither sketch is closed, and the
  enumeration is `param_types.rs`'s own `Inference::record` call sites — that method
  being the one writer of the map. A `$N` none of them reaches decodes as
  text, and the comparison then fails **loud** rather than mis-decoding — `42883
  operator does not exist: <type> = text` against a non-text column (the spelled
  type is the operand's as pglet names it — `timestamp without time zone` for a
  `timestamptz` column, where 18.6 says `with`)
  (measured for `CASE`, `COALESCE`, `greatest` and `ANY(ARRAY[$1])`, which between
  them cover `int4`, `bigint`, `numeric`, `float8`, `bool` and `timestamptz`
  columns; 18.6 answers `42883` on each shape too), `42804` across an arithmetic
  operator (`n = n + $1`, `id > $1 + 0`) and `0A000` for `abs($1)` — the last two
  where 18.6 resolves instead, so pglet is **stricter** there, not looser.
- **An unrecognized *client-declared* OID silently collapses to text-inference**,
  not fail-loud. `Parse` priority puts a client-declared OID first ("authoritative,
  as in Postgres"), but `TypeOid::from_oid` maps **both** OID 0 ("infer") **and any
  real-but-unmodeled OID** (`point` 600, `inet` 869, `money` 790, `bit`
  1560) to `None`, so a binary client that explicitly types `$1` as one of those gets
  the same text fallback rather than PG's `42704` — and, since the declared type never
  reaches parse analysis either, the same inference: `coalesce($1, 1)` declared `point` and
  bound `(1,2)` is `22P02` here against 18.6's `42804 COALESCE types point and integer
  cannot be matched` — and, with nothing typing it, `SELECT $1 IS NULL` declared `point`
  is `42P18` here where 18.6 answers (`EDGES.md` row 276). This is the one type-declaration
  entry point that doesn't route through the fail-loud resolver — a narrow
  correct-or-loud hole, reachable only by a client that declares a param OID itself:
  tokio-postgres, the client the harness drives, sends empty param types for `@param`
  (§ Why infer at Parse) and `@param-declared` declares only types pglet models, so no
  fixture exercises the fallback. The fix is
  to fail loud on an unrecognized non-zero declared OID — but not as a check bolted on
  where the fallback happens, because by then the distinction is gone: `Parse` decoding
  runs every declared OID through `pglet_core`'s `TypeOid::from_oid`, whose `0` arm and
  whose unknown arm both answer `None` — that function's own comment records the two as
  "distinct meanings" sharing a body — so `param_types` cannot
  tell "infer this for me" from "I named a type you don't model". Closing it means
  carrying that difference out of `pglet_wire`'s decoder first; a gate over the collapsed
  `None` would reject the protocol's ordinary `0`.
- **The Execute message's max-row-count is ignored.** The field is decoded, then
  dropped (`handlers.rs`), and pglet never emits `PortalSuspended` — a
  row-limited Execute returns the full buffered result plus `CommandComplete`.
  That matches Postgres only for the fetch-all case (`max_rows = 0`, which is
  what tokio-postgres and node-postgres send); a cursor-style client
  (`pg-cursor`, row-limited `FETCH`) would get everything at once, and each further
  `Execute` it sends for the rest runs the statement again — a data-modifying one
  writing again (`EDGES.md` row 169). True portal
  suspend/resume is a natural companion to a streaming (volcano-model) executor
  ([storage.md](storage.md) § Known bottlenecks), if one ever lands.
- **Function-signature inference** covers operators plus a per-function argument
  table for the `StringFunc` family (`string_fn_text_arg` in `param_types.rs`):
  `UPPER`, `LENGTH`, `SUBSTRING`, `REPLACE`, `LTRIM`/`RTRIM`/`BTRIM`,
  `STRPOS`/`POSITION`, `LPAD`/`RPAD`, `SPLIT_PART`, `DATE_TRUNC`, `CURRENT_SETTING` —
  each typing its text-position argument(s) as `text` (the table also names
  `CURRENT_SCHEMAS`, `PG_GET_CONSTRAINTDEF` and the `::regclass` cast as typing no
  position; re-derive the list from the match, not from here), leaving integer positions (substring's
  start/length, `lpad`/`rpad`'s length) to their own context. `LOWER` (its own
  `Expr` node), `||`, `EXTRACT`'s field argument, and `concat_ws`'s separator are
  typed by their own walk arms rather than that table. A function outside these
  isn't typed from a signature.

The per-feature ledger for this path is [compatibility.md](compatibility.md).
