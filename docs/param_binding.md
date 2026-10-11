# Prepared-statement parameter binding

How pglet binds extended-protocol parameters: it infers types at Parse, decodes values at
Bind, and resolves them at Execute. The shapes this path supports — `WHERE id = $1`,
`id = ANY($1::uuid[])`, `refs @> $1`, `unnest($1, $2)`, `WHERE $1`,
`LIMIT $1 OFFSET $2` — are pinned by `-- @param` fixtures. Those fixtures live next to
their subject instead of in a directory of their own. § Test coverage says where the
clusters are, and `grep -rl -- '@param' tests/fixtures/` lists them all.

## Why infer at Parse

Idiomatic tokio-postgres (the client the differential harness drives) sends `Parse` with
*empty* param types, then `Describe`. It relies on the server to infer the types and
report them in `ParameterDescription`, and then uses them to binary-encode each value. If
the server can't type an untyped `WHERE id = $1` (uuid), it reports OID 0, the client
encodes the value wrong, and the server decodes it wrong. So every binary-protocol client
needs Parse-time inference.

## Design: infer at Parse, decode at Bind, resolve at Execute

The decoded params are a **slice threaded through the executor** as an evaluation
environment (`EvalEnv`). The plan isn't rewritten. `infer_param_types` still types each
`$N` at Parse, but the decoded `Value`s travel in the env, and an `Expr::Parameter` reads
`params[N]` at eval time (`execute_plan_node` / `eval_expr_env` carry the env). The plan
is never mutated to bind, so a prepared statement keeps one immutable plan.

- **Parse** (`pglet_server` handler): parse and plan the statement, then call
  `infer_param_types(&plan, &schema, hint)` (`pglet_sql/src/planner/param_types.rs`). It
  walks the plan and types each `$N` by priority: client-declared OID (authoritative, as
  in Postgres) > explicit cast > context (including an `INSERT … SELECT` entry's target
  column) > the `text` of a bare select-list or `RETURNING` entry. A parameter the
  statement leaves without a type is refused there (§ A parameter left without a type is
  refused). A client-declared OID is part of the plan too, not only of the report:
  § A declared type is part of parse analysis. A statement the planner refuses raises
  its error at `Parse`, instead of advertising zero params and failing the later `Bind`.
  `params/returning_gate_error_at_parse.sql` and `params/set_rhs_gate_error_at_parse.sql`
  pin this.

  Context means: a column comparison (`col <op> $N`), an `ANY`/`@>` element/array type,
  an `IN` list (an item takes the left operand's type — [coercion.md](coercion.md) § An
  `IN` list is compared at one type), INSERT/UPDATE targets (via the existing
  coerce-to-column cast), `LIMIT`/`OFFSET` (→ `int8`), `||` / `LOWER()` arguments
  (→ `text`), a **boolean position** (→ `bool`, below), a unary `+` operand (→ `float8`,
  as 18.6 also resolves it), a `jsonb` key or path operand (`?` / `->` / `->>` → `text`,
  `?|` / `?&` / `#>` / `#>>` → `text[]`), and a bare `$N` in an `UPDATE … SET col = …`
  value, including a `CASE`/`COALESCE` result branch, typed from the target column.

  That last probe covers less than it sounds like. Where a branch has a *typed* sibling,
  the lowering's branch unification has already coerced the parameter and the ordinary
  cast rule types it. So the probe only matters for an **all**-unknown `CASE`/`COALESCE`.
  We checked both the `UPDATE` and the `ON CONFLICT` arms:
  `SET v = CASE WHEN $1::bool THEN $2 ELSE $3 END` reports `$2`/`$3` as OID 0 without the
  probe and `{boolean,text,text}` with it, matching 18.6.

  An `ON CONFLICT` clause is typed at every position it parameterizes: the arbiter's key
  parts and its partial-index predicate, the `DO UPDATE` `SET` list (the
  assignment-target rule again, `CASE`/`COALESCE` branches included), and the action's
  `WHERE`. The **action**'s two positions are typed against a namespace of their own,
  `on_conflict_action_relations` (the target plus `excluded`, which has the target's
  shape), not against the catalog. So an `excluded.<col>` reference finds the
  pseudo-relation even when a real table is *named* `excluded`. Typed off the catalog
  instead, that shadow table would win: `{text}` for `WHERE excluded.n = $1` against
  18.6's `{bigint}`, and the bound statement would then fail loud with
  `42883 operator does not exist: bigint = text` where 18.6 updates the row. The
  **arbiter** stays outside that namespace. That's 18.6's rule, not caution on our part:
  `ON CONFLICT ((excluded.id)) …` and `ON CONFLICT (id) WHERE excluded.n > 0 …` are both
  `42P01 invalid reference to FROM-clause entry for table "excluded"` there.

  A `RETURNING` item is typed against that clause's own namespace
  (`Inference::walk_returning` over `returning_relations`: the DML target under its
  effective name plus the relations an `UPDATE … FROM` brings in), so a source-qualified
  `$N` context types from the source column. These clauses are walked in
  `transformInsertStmt`'s order: row source, then `ON CONFLICT`, then `RETURNING`. You
  can see the order, because a `$N` written in two of them keeps the earlier one's type,
  on both engines (`Inference::record` keeps the first fact at equal priority). The
  *at equal priority* part matters. An assignment right-hand side's coercion cast records
  at the higher cast priority, so it outranks an earlier clause's context fact whatever
  the order. That's one collision the rule doesn't cover, recorded with the shapes we
  tested in [compatibility.md](compatibility.md)'s row for this feature.

  The walk descends into a **data-modifying CTE**'s DML body (`Plan::With`), so a `$N`
  inside `WITH x AS (UPDATE … SET c = $2 WHERE k = $1 …)` types from that CTE's table.

  A column-comparison context is **alias-aware across a JOIN**. The walk carries a
  per-scope FROM alias→table map (reset at each subquery boundary), so a qualified `$N`
  context like `act.id = ANY($1)` over `FROM actor act JOIN account a` resolves `act` →
  `actor` and types `$1` as `uuid[]` (a single-table context can't). This works **only
  while the alias names no real relation**. `infer_col_type` tries the catalog
  (`TypeScope::Catalog`) before that map, so a FROM alias that collides with a table name
  is typed from *that table's* column instead. With `dt(id INT, b BIGINT)` and
  `ds(id INT, b INT, k INT)`, `SELECT ds.b = $1 FROM dt ds` reports `int4` here and `int8`
  on 18.6, so a text bind of `5000000000` is `22P02` here and `f` there. The same
  statement aliased `zq` agrees.

  **The `ON CONFLICT` arbiter is the extreme case of the same problem.** It resolves no
  names at all, so its qualifier is a plain catalog lookup. Over
  `oc_probe(id INT PRIMARY KEY, v TEXT, n BIGINT)` with an unmentioned
  `oc_other(id INT, n SMALLINT)` in the schema,
  `ON CONFLICT (id) WHERE oc_other.n > $1 DO NOTHING` reports `{int2}`. That's *that*
  relation's column, which is how you can tell it's the lookup and not a fallback. A name
  the catalog lacks (`zzz.n`) falls back to the target and reports `{int8}`. 18.6 refuses
  both with `42P01` at Parse. This is recorded with the clause's other residuals in
  [compatibility.md](compatibility.md)'s row for this feature.

  **An `excluded` reference inside a *subquery* in the action** loses the pseudo-relation
  the same way. `Inference::walk_scoped` clears the clause namespace at every subquery
  boundary. A cleared `RETURNING` scope falls back to a catalog lookup that finds the
  outer relation again, because that relation *is* a catalog table, but `excluded` isn't
  a catalog table under any spelling. Over `oc_probe(id INT PRIMARY KEY, v TEXT, n BIGINT)`
  and `oc_src(id INT, v TEXT)`, `… DO UPDATE SET v = 'z' WHERE EXISTS (SELECT 1 FROM
  oc_src s WHERE s.id = $1 AND excluded.n = $2)` reports `{integer,0}` here against
  18.6's `{integer,bigint}`. With a real `excluded(id INT, n TEXT, v TEXT)` next to the
  target, which 18.6 ignores, it reports `{integer,text}`. The control is the same
  comparison written directly in the action's `WHERE`, which gives `{bigint}` on both
  engines either way. This is **masked**, not harmless. At a conflicting key the action's
  `EXISTS` hits pglet's own `0A000 … requires execution context` at execute, and the
  statement fails there. At a free key the action isn't evaluated at all, and the
  `INSERT` succeeds on a `VALUES` row `$2` doesn't reach. So the OID reaches a `Describe`
  reply and a `Bind` decode, but no row holds a value read under it. No fixture pins it,
  and the reason goes beyond this shape: the harness compares output-column declared
  types and never reads `ParameterDescription`, so it only sees a parameter OID through a
  `Bind` that fails.

  **No type crosses an arithmetic operator**, and none crosses its call spellings
  `mod($1, 5)` and `power($1, 2)` either. It's worth saying next to the contexts above,
  because it takes the canonical upsert idiom out of the bindable set:
  `SET n = <rel>.n + $1` (and the plain `UPDATE … SET n = n + $1`) reports OID 0 where
  18.6 types it from the operator. The parameter then decodes as text, and the statement
  fails at execute with `42804 type error: operator + requires numeric operands`. The
  explicit `$1::int8` spelling binds, and so does a bare `SET n = $1`, which the
  assignment-target rule covers.

  A filter on a **virtual-table** column (`WHERE table_name = $1` over
  `information_schema.*` / `pg_catalog.*`) types from the catalog view's column list
  (text). `column_resolved_in_relation` resolves a column against real `Schema.tables`
  **then** virtual catalog tables (`CatalogTable::from_name(relation)?.columns()`), so a
  binary-protocol client never sees OID 0 there (the `params/virtual_table_filter`
  fixture guards it).

  **A boolean position types a bare `$N` as `bool`**, matching 18.6's
  `coerce_to_boolean` resolving an unknown `Param` there. A clause root isn't attached to
  any node, so `record_bool_clause` runs where the plan walk reaches one. A **node**
  position comes from `Expr::for_each_child_ctx` instead of a second list of arms, so the
  inference and the gate share one list of node positions instead of keeping two that
  must agree. It's recorded at context priority, so an explicit `$1::int` still outranks
  it and then fails the gate, which is what 18.6 does too. That failure happens at
  `Parse`, before any `ParameterDescription`, so `boolean_context_verdicts.rs` pins that
  priority, not the wire. These all describe `{bool}` in `ParameterDescription` on 18.6
  and here: `WHERE $1`, `WHERE $1 AND a = 1`, `WHERE NOT $1`, `WHERE $1 OR a = 1`,
  `CASE WHEN $1`, `HAVING $1`, `JOIN … ON $1`, `UPDATE … WHERE $1`,
  `DELETE … WHERE $1`, and an `ON CONFLICT … DO UPDATE` action's `WHERE $1`.

  The **`IS [NOT] TRUE` family is a measured residual**. Its gate is an ordinary
  post-order check, not a child context, so `for_each_child_ctx` gives that operand no
  context and the **inference** doesn't reach it, though the gate still does
  (`argument of IS TRUE`). `WHERE $1 IS TRUE` describes `{bool}` on 18.6 and `{0}` here,
  which costs a binary-protocol client the same OID 0 described in the OID-0 gap below. A
  **stored** predicate isn't a gap: neither engine describes a parameter there.
  `CREATE TABLE t (a INT CHECK ($1))` and `CREATE INDEX i ON t (a) WHERE $1` describe
  **zero** parameters on both engines.

  The result is stored on the prepared statement and is the single source of truth for
  both the `ParameterDescription` reply and the Bind decode.
- **Bind**: `decode_param(bytes, oid, format, param_number)` (`pglet_server/src/encoding.rs`)
  turns each value into a `Value`. Text format first passes the bytes through
  `pglet_core::encoding::text_from_utf8`, the server-encoding scan, for every parameter
  type, and then goes through `coerce_value` (the same parsing as `::text` casts). Binary
  format decodes via `value_from_binary`, the inverse of `value_to_binary` /
  `encode_array_binary` (scalars and one-dim arrays, NULL elements). It reads the payload
  the way 18.6 does (§ A binary parameter is consumed exactly), and its `text`/`varchar`
  and `jsonb` arms run the same scan, with an array's text elements reaching the `text`
  arm one at a time. A NUL or illegal UTF-8 byte the scan finds is `22021`
  (`docs/encoding.md`). The result is stored as the portal's
  `bound_values: Vec<Option<Value>>` (NULL = `None`).
- **Execute**: the portal's `bound_values` become an `EvalEnv::with_params(...)` passed
  into `execute_with_storage`. An `Expr::Parameter { index }` resolves to
  `params[index - 1]` at eval time. A `LIMIT`/`OFFSET` `$N` resolves from the env in the
  `Plan::Limit` arm, and `Plan::Limit` carries `LimitCount::{Const,Param}` so a `$N` limit
  survives planning. The plan itself is the cached, immutable one from Parse. It's
  re-planned when the schema generation has moved since Parse (a DDL, or a rollback that
  undid one).

Binding is one engine path reached two ways. **Over the wire** it happens
**server-side**: the server decodes the portal's params (`decode_param`) into the portal's
`bound_values` and passes them through `execute_with_storage`. **In-process**, the embed
`Database` facade exposes the same path through `execute_bound(sql, conn_id, bind)`. It
parses once (and may reuse the parse of a text it already parsed under the same catalog
and thread settings), infers each `$N` (`pglet_sql::planner::infer_param_types`), hands
the inferred OIDs to a `bind` callback so the caller converts its own values, and passes
the result through the same `EvalEnv::with_params`. The **wasm hosts**, `PgletBtree` and
(in a build with the crate's `opfs` feature) `PgletBtreePersistent`, run their `exec` and
`query` through `execute_bound` with no parameters. Only `PgletBtree` takes bound
parameters, through `query_params` (JS `queryParams`). Its `bind` callback is
`pglet_wasm`'s `bind_js_params`. For each parameter it runs
`refuse_text_outside_the_server_encoding` (a JS string holding a NUL is `22021`), and then
a JS→`Value` decoder that reuses the `coerce_value` codec (`pglet_sql::executor`). Those
two are the wasm-specific part of binding. Parse, inference and execution belong to the
facade.

### The bound entry point's plan cache

`Database::execute_bound` keeps, per database and keyed by the statement's text, what it
computes before binding: the plan and the parameters' inferred targets
(`crates/pglet/src/plan_cache.rs`). A hit replaces the parse, the statement count and the
inference, and nothing else. Everything else the entry point does (the aborted-block gate,
the session commands, installing the ambient session, binding and execution) runs the same
on a hit as on a miss.

**A cached plan is only valid for the catalog and settings it was planned under, and the
stamp is how the cache knows which those are.** The cache holds plans for one stamp: the
catalog's `Schema::generation`, the calling thread's depth limit and unknown-type policy,
and natively the thread's stack bounds. A lookup under a different stamp empties it. So
any catalog change that a parse or inference reads must move the generation or empty the
cache. Executing a plan for which `Plan::mutates_schema` holds moves the generation. That
includes a `CREATE SEQUENCE`, since inference asks whether a relation of that name exists.
Restoring a catalog snapshot taken at another generation moves it too
(`Schema::restore_from`: a block's rollback, a savepoint's, a `DO` block's undo). A
snapshot taken at the live generation differs from the live catalog in nothing a parse or
inference reads, so restoring it keeps the generation and the cache keeps its plans. That
covers the rollback of a block that ran only DML, except in a block the host's
`Database::schema_mut` reached, whose rollbacks do move it. `Database::schema_mut` can
change anything, or replace the catalog, without moving the generation forward, so it
empties the cache, as `Database::set_stack_budget` does. A `Database::fork` starts empty.
Any other input the parser or the inference starts reading must become a stamp field or
empty the cache when it changes. If it does neither, a held plan answers for a catalog or
a setting it wasn't planned under, and no error says so.

**Bounds.** At most 128 texts (`CAP`), none longer than 4 KiB (`MAX_TEXT_BYTES`), and at
most 64 KiB of text held (`HELD_TEXT_BUDGET`). A statement that would go over either total
empties the cache first. Some things aren't held: a plan that changes what a parse reads
(`Plan::mutates_schema`), a text whose parse, statement count or inference fails, and
natively, on a thread whose depth limit is above `pglet_sql::depth::FALLBACK_MAX_DEPTH`
(the limit sized for the smallest stack a thread gets by default), a plan deeper than that
limit. A plan whose binding or execution fails is held. The bounds are on text, not on
plans: a short `SELECT *` over a join of wide tables holds a plan with every column.

**What a hit does not answer: natively, the parser's byte check.** A hit runs no parse, so
it isn't refused with `54001` where its parse would be. That happens in two cases. One is
a statement held from a call made higher in the caller's stack and then called again from
deeper. The other is a statement run on one database from inside another's `bind`
callback, which parses under the outer database's stack budget. No stamp holds that
budget, so a plan held under one budget is used under another. Its execution's own checks
still run. The wasm build has no byte check.

**What reaches it.** Only `execute_bound`: a native embed's call, and the wasm hosts'
`exec`, `query` and `queryParams`, which means the npm package. `Database::execute` /
`execute_as` and the native server never consult it, so the differential fixtures can't
see it. Its guards are `crates/pglet`'s own tests (`plan_cache.rs`'s among them) and the
wasm suite.

### A declared type is part of parse analysis

A type the client declares at `Parse` is the parameter's type in parse analysis, the same
as in 18.6's `exec_parse_message`. There the Parse message's OIDs seed the array that
`variable_paramref_hook` reads (`parse_param.c`), so a declared `$N` is a `Param` of that
type from its first reference, and the operators, functions, `select_common_type`
resolutions and result columns over it all work from the declared type. The native server
plans through `parser::parse_single_with_param_types`, which reads a declared `$N` as
`Expr::Cast { Parameter, <type> }` wherever the **expression** parser reads the token.
That's the shape a written `$N::<type>` lowers to, so the coercion pass, the gates and
both declared-type walkers (`docs/declared_types.md`) see a typed operand through the arms
they already have, and can't disagree about it.

That's also as far as it reaches. A position that reads the `$N` token some other way
doesn't see the declaration. `LIMIT` and `OFFSET` take the token as a `LimitCount::Param`
(`parser/select.rs`), so `SELECT n FROM tn LIMIT $1` declared `text` is
`22000 LIMIT/OFFSET must be an integer` at `Execute`, against 18.6's
`42804 argument of LIMIT must be type bigint, not type text` at `Parse`. Declared
`numeric` and bound `1`, it's that same `22000` where 18.6 returns the row (we tested
this, and the `OFFSET` spelling behaves the same). The bound value is decoded as the
declared type, so the cast converts nothing. Checked on both engines with a raw
extended-protocol client: `coalesce($1, 1)` declared `numeric` and bound `1.5` is
`numeric` `1.5`, `boolean` beside `1` is `42804`, a bare `SELECT $1` declared
`timestamptz` is `1184` with its `+00`, and `DELETE … WHERE n = coalesce($1, 0)` removes
the `1.5` row.

These consequences follow from that shape, and we tested each one:

- **An undeclared parameter keeps its inference**, and is still a bare `Expr::Parameter`.
  Where that inference reads its sibling's type, it reads the declared one, as on 18.6.
  With `$1` declared `int4` and `$2` undeclared, `$1 = $2`, `$1 < $2`, `$1 IN ($2)`,
  `$1 BETWEEN $2 AND $3`, `coalesce($1, $2)`, `greatest`/`least`, `CASE WHEN … THEN $1
  ELSE $2 END`, `ARRAY[$1, $2]` and a set operation's `SELECT $1 UNION SELECT $2` all
  describe `$2` as `int4`, and `$1 || $2` / `$1 LIKE $2` declared `text` describe it as
  `text` (each agrees with 18.6). **This doesn't hold for every construct**, because
  pglet's inference has no rule there that reads the declaration. Against 18.6's
  `{int4,int4}`, each of these describes `$2` as `0` and then refuses at `Execute`:
  `nullif($1, $2)` and `CASE $1 WHEN $2 …` (`42883 integer = text`), and `$1 + $2`,
  `$1 - $2` and `$1 * $2` (`42804 operator + requires numeric operands`). `$2 IN ($1)`,
  with the undeclared parameter on the left, is `{numeric,0}` and `42883` against
  `{numeric,numeric}`. Over an `INT` column, `a IN ($1, $2)` with `$1` declared `numeric`
  describes `$2` as `int4` against 18.6's `numeric` (the column's type, not the list's
  common type), so binding `1.0` and `1.5` is `22P02` there, where 18.6 returns the row
  `1`. `$1 = ANY(ARRAY[$2])` is `42883` on both engines: pglet's at `Execute` after
  describing `{int4,0}`, 18.6's at `Parse`.
- **A re-plan keeps the declared types.** The prepared statement and its portal keep the
  Parse message's own OIDs (`declared_param_types`) separate from the inferred list, and a
  plan made stale by a catalog change (a DDL, or a rollback that undid one) is rebuilt
  with them at `Describe` and at `Execute`.
- **A declared parameter is still an index probe.** `is_row_independent` admits a `Cast`
  directly over a `$N`, and `probe_key_value` exact-or-declines its value the same way it
  does a bare parameter's (`docs/indexes.md`). When the probe is declined, a point read
  over a 100 000-row key takes about 19 ms, against 0.05 ms.
- **A string literal beside a declared `$N` resolves to the declared type at a comparison
  and in a common-type construct.** That covers `$1 = '1.50'` and the other comparison
  operators, `BETWEEN`, `CASE $1 WHEN '1'`, `coalesce`, `greatest`/`least`, `CASE … THEN`,
  `ARRAY[$1, '6']`, and a set operation or `VALUES` list beside it (each agrees with
  18.6). It works through the comparison coercion's cast anchor and the common-type pass.
  It doesn't work in every position: `$1 IN ('1')` and `nullif($1, '1.5')` are `42883`
  here, and `$1 = ANY('{1,2}')` is `0A000`, where 18.6 resolves each literal to the
  declared type and answers. `$1 + '2'` is `42804` against 18.6's `3`. Each of these also
  refuses the same spelling with a written `x::int4` in place of the `$N`, so they're
  residuals of the literal resolution, not of the declaration.

The in-process APIs declare nothing. `Database::execute_bound`'s `bind` callback gets the
inferred OIDs and returns values, and the wasm hosts' `queryParams` takes values only, so
every `$N` there is inferred. The fixture directive for this path is `-- @param-declared`
(`tests/README.md`). The guards are the `params/declared_*` fixtures and
`crates/pglet_sql/tests/engine_declared_params.rs`, which asserts the executor walk as
well as `Describe`'s. A declared OID pglet doesn't model still reaches no analysis
(§ Deferred / known gaps).

### A parameter left without a type is refused

18.6 gives a parameter reference the type the parameter has **when that reference is
analysed**, and some positions coerce nothing: an `IS [NOT] NULL` operand (and
`IS [NOT] DISTINCT FROM NULL`, which its parser rewrites to one), and an argument declared
`"any"` (`count`'s without `DISTINCT`, `concat`'s, `concat_ws`'s after the separator,
`jsonb_build_object`'s). A reference left `unknown` there is never revisited, so the
statement is refused with `could not determine data type of parameter $N`:

- **`42P18`** where nothing in the statement types the parameter: `WHERE $1 IS NULL`,
  `count($1)`;
- **`42P08`** where a **later** occurrence does: `($1 IS NULL OR name = $1)`. It's raised
  from inside parse analysis, so it comes before any `42P18` another parameter gets.

An **earlier** occurrence that types the parameter makes the same position harmless.
That's why the optional-filter idiom is accepted comparison-first
(`(name = $1 OR $1 IS NULL)`) and with a cast (`($1::text IS NULL OR name = $1)`), on
both engines. "Earlier" means parse-analysis order, not source order: `FROM` (a join's
two sides, then its condition), the select list, `WHERE`, `HAVING`, `ORDER BY`, `LIMIT`;
an `INSERT`'s row source, then `ON CONFLICT`, then `RETURNING`; an `UPDATE`'s `WHERE`,
then `RETURNING`, then its `SET` list; a `WITH` query before the statement that reads it;
an `IN (SELECT …)`'s subquery before its left operand.

Inside an expression, an operator's or a function's operands are typed when the node is
resolved, after each one was read. So `jsonb_build_object('k', $1, 'j', $1::text)` is
`42P08`, and the same call with the two arguments swapped isn't. A boolean operand
(`AND` / `OR` / `NOT`, a searched `CASE`'s condition) and a simple `CASE`'s operand are
typed as they're read. A bare select-list entry becomes `text` only once its query level
is analysed, a bare `RETURNING` entry once that list is, and a `VALUES` list's bare entry
once its last row is. We tested each: `SELECT $1 AS p, $1 IS NULL` and
`SELECT * FROM (VALUES ($1, $1 IS NULL)) v` are `42P08`. A bare entry of an
`INSERT … SELECT` source doesn't get that `text` when nothing in the source resolves it
first. It's coerced to the column it's assigned to once the source is analysed, before
`ON CONFLICT` and `RETURNING`: `INSERT INTO kv (k, v, n) SELECT $1, $2, $3` describes
`{text,jsonb,integer}`. A sort, grouping, `DISTINCT` or window key naming the entry
resolves it to `text` first, and so does a set operation whose sides leave the column
unknown (`EDGES.md` row 150). An `INSERT` row's cells are analysed in the order the row is
written (the column list's order, whatever the table's), and a `SET` list's cells in the
list's order. Their bare entries are coerced to their columns once the whole row or list
is analysed.

pglet's inference walk records when each kind of occurrence happens
(`Inference::first_fix`, `Inference::first_unresolved`) and compares them per parameter.
It follows the order above except where a plan doesn't carry it. The measured exceptions
are `EDGES.md` row 276. One of them is an `INSERT` row's or a `SET` list's bare entry,
which is timed where it's written instead of at the end of its row or list. The row's
written order isn't the lowered row's order, which is the table's, so where a column list
departs from the table's order the walk reads it from `InsertRows::Values`' `written`.

The check is **not** "inference left the slot untyped". This engine types fewer positions
than 18.6 (§ Deferred / known gaps), and a refusal keyed on that would refuse statements
18.6 answers: `id = $1 + 1 OR $1 IS NULL` reports OID 0 here, and this rule refuses it on
neither engine. The refusal is `infer_param_targets`' error, raised where it's called: at
`Parse` on the native server, with no `ParseComplete`, and in `Database::execute_bound`
before its binding callback (the wasm binding reads no value's JS type for a refused
statement). Either way it fails an open block. A parameter the client declared at `Parse`
as a type this engine models is typed from its first reference and isn't refused.

18.6 raises the refusal once parse analysis is done, so an error raised during parse
analysis outranks it. Here it follows planning, which puts it after the errors the parser
and planner raise (a missing column's `42703`) and before the ones raised at execution. A
relation the catalog lacks is one of those. Where the executor is certain to report it (a
`FROM` clause's or a DML target's), the refusal yields: inference refuses nothing over
such a statement, and `SELECT id FROM nope WHERE $1 IS NULL` is `42P01` on both engines.
An index's name yields the same way (`42809` on both). A sequence's doesn't, since 18.6
reads a sequence as a relation and goes on to refuse the parameter (`42P18` on both). A
relation named inside an expression's subquery is only reported for a row that evaluates
it, not over an empty table, so the refusal stands there. The errors the refusal does
outrank, that one and a constant's `22P02`, are recorded in `EDGES.md` row 276.

Where the plan doesn't say which of two occurrences came first, the statement is
accepted. That's a conjunct the planner lifted into a B-tree index scan, or a bare entry
the lowering has cast (a set operation's, a `FROM`-clause `VALUES` list's, an `INSERT`
row's, a `SET` list's). A parameter typed only in a `WITH` query the statement never
reads, or declared as a type this engine doesn't model, is refused. Each of these is
`EDGES.md` row 276. Guards:
`tests/fixtures/params/undetermined_param_*`, `crates/pglet/tests/undetermined_param.rs`,
`handlers.rs`'s
`parse_refuses_a_parameter_left_without_a_type_unless_the_client_declared_it`, and the
wasm and npm suites.

`INSERT … SELECT` binds too. `Plan::Insert` carries a bound `source` plan, a projection
`unnest($N)` row-expands via `Plan::ProjectUnnest`, and the native parser handles the
no-FROM `INSERT … SELECT … ON CONFLICT` shape directly
(`tests/fixtures/spine/fact/put_fact_refs.sql` runs it bound end to end).

### A binary parameter is consumed exactly

A binary-format parameter is read the way 18.6's `exec_bind_message` reads it. The type's
reader runs over the payload, and if it didn't consume the payload to the last byte,
that's **`22P03 incorrect binary data format in bind parameter <n>`**, where `<n>` is the
parameter's 1-based position in the Bind message. `value_from_binary` gets that number
from `decode_param`, which gets it from the Bind loop's own position. `BinaryReader` plays
the part of 18.6's message buffer, and `recv_binary` plays the type's `*recv`. It reads
what the type reads and leaves the cursor where it stopped, so its caller decides whether
anything is left over.

A read past the end is the **message reader's** `08P01`. Which message it carries depends
on which reader the type's `*recv` calls, not on how many bytes it asks for.
`pq_getmsgbyte` says `no data left in message` (`boolrecv` reads through it), and
`pq_getmsgint`, `pq_getmsgint64` and `pq_getmsgbytes` say
`insufficient data left in message`. `jsonb_recv` reads its one version byte through
`pq_getmsgint(buf, 1)`, so an empty `jsonb` payload is `insufficient`, not `no data`.
`text`, `varchar` and `bytea` read everything that's left, so no length is wrong for
them. A value a reader reads in full and then refuses (an out-of-range `time`, say) gets
that refusal before the leftover check, whatever follows it.

**`numeric`** keeps `numeric_recv`'s order, because the order decides which refusal a
malformed body gets. First the digit count, read **unsigned** (so `0xFFFF` asks for 65535
digits), then the weight, then the sign. A bad sign is
`22P03 invalid sign in external "numeric" value`, raised before the scale or any digit is
read, so it beats both a digit shortfall and a trailing tail. Then the display scale
(`22P03 invalid scale …` for a bit outside `0x3FFF`), then each digit
(`22P03 invalid digit …` outside `0 ..= 9999`). A `NaN` reads and checks the digits its
header declares too. `Infinity` and `-Infinity` pass 18.6's sign check and are accepted
there. `PgNumeric` has no infinity, so pglet refuses a complete one with
`22P02 invalid input syntax for type numeric: "<binary>"`, after reading the fields 18.6
reads.

**An array** follows `array_recv` and `ReadArrayBinary`. The dimension length is checked
where `ArrayGetNItems` checks it, before any element is read: a negative length or one
above `134217727` is `54000 array size exceeds the maximum allowed (134217727)`. Then the
length is **spent**: exactly that many elements are read, so anything after them is the
parameter's leftover, not more elements, and a payload short of them is the reader's
`08P01`. `ndim = 0` reads no dimension and no element. An element's length field below
`-1`, or past the bytes left, is `22P03 insufficient data left in message`. That's the
message reader's words under `ReadArrayBinary`'s code, so neither the code nor the words
alone tell you which one raised it. Each element is read by its type's reader over exactly
its own bytes. A leftover there is `22P03 improper binary format in array element <i>`,
counted from `1` with NULL elements counted, and a read past the element's end is that
reader's `08P01`.

A header whose element OID names an **array** type that `TypeOid::from_oid` recognises is
refused as soon as the OID is read, with `array_recv`'s
`42804 binary data has array element type <oid> (<name>) instead of
expected <oid> (<name>)`. So an element is never decoded as another array, and a payload
that nests array headers can't make the decoder recurse. An OID pglet doesn't recognise,
including an array type it doesn't model such as `oid[]` (`1028`), falls back to the
parameter's own element type, which 18.6 refuses below its built-in floor. That fallback
and the header's other fields, including a scalar element OID, are still taken at face
value (`EDGES.md` row 140).

**Two values a reader constructs** also follow their 18.6 counterpart. A `numeric` is
truncated toward zero to its display scale, as `numeric_recv`'s `trunc_var` truncates it:
digits `1, 5678` sent with scale `1` are `1.5`, and a value that truncates to zero is a
positive zero. An `interval` whose three fields are all at their maximum or all at their
minimum, which 18.6 reads as `infinity` and `-infinity`, is refused with the
`22007 invalid input syntax for type interval: "infinity"` (or `"-infinity"`) that the
text path gives for those words, since `PgInterval` has neither. The text path doesn't
refuse the triple itself: written as numbers, it's a finite interval there (`EDGES.md`
row 143).

**A refusal here fails an open block.** The refusals in this section are raised at
`Bind`, and an error at `Bind` fails an open block (`docs/storage.md` § The transaction
session). Where 18.6 refuses the same payload, it fails its block the same way. Where 18.6
accepts it (the interval sentinels above, and the cases `EDGES.md` row 140 records),
pglet's refusal discards what the block wrote, and 18.6 keeps it.

No `test_fixtures` fixture can cover this: `@param` binds through tokio-postgres, which
encodes each value at its own width, and the directive grammar has no raw-bytes form.
The guards are `binary.rs`'s `a_fixed_width_value_is_read_to_its_width_and_no_further`,
`a_binary_numeric_is_read_in_numeric_recvs_order`,
`a_binary_numeric_infinity_is_refused_after_its_fields_are_read`,
`a_binary_arrays_dimension_length_is_spent`,
`a_binary_arrays_dimension_length_is_gated_before_it_is_spent`,
`a_binary_array_element_is_read_to_its_own_length`,
`a_binary_array_header_naming_an_array_element_type_is_refused`,
`a_binary_numeric_is_truncated_to_its_display_scale` and
`a_binary_interval_infinity_is_refused_as_the_text_path_refuses_the_word`, plus
`handlers.rs`'s `a_binary_param_is_consumed_exactly_and_refused_at_bind_with_its_number`.
That last one binds a refused **second** parameter after a well-formed first and reads the
code and message off the wire at Bind.

### A binary `jsonb` payload's first byte is its version

A binary-format `jsonb` parameter's **first byte is always read as its version number**,
and only `1` is supported. The JSON text is everything after it. That's how 18.6's
`jsonb_recv` works (`pq_getmsgint(buf, 1)` and then
`pq_getmsgtext(buf, buf->len - buf->cursor, …)`), so there's no such thing as a payload
sent *without* a version byte. Whatever the sender meant by its first byte, that byte is
the version: `{"a":1}` binds as version 123 (`0x7b`), `123` as version 49, `"hi"` as
version 34.

A version byte other than `1` is **`XX000 unsupported jsonb version number <n>`**, with
`<n>` the byte's *unsigned* decimal value, `0` through `255`, so `0xff` prints `255`. `XX000`
isn't a code we chose over a tidier one. It's what 18.6 sends:
nothing on that path calls `errcode()`, and `errstart` picks a default from the elevel,
`ERRCODE_INTERNAL_ERROR` for anything at `ERROR` or above. `elog` isn't what causes this.
It expands to `ereport(elevel, errmsg_internal(...))`, so an `ereport(ERROR, ...)` written
without an `errcode()` of its own gets the same default. pglet sends the code 18.6 sends,
and this is the only place the engine produces `XX000`. 18.6 also sends a `W`
(`unnamed portal parameter $1`) with it, and pglet sends no `W` field anywhere
(`EDGES.md` row 76).

**The split runs before the server-encoding scan.** That's 18.6's order too, and it's
what keeps a `0x00` or a high leading byte a version number instead of a byte the
encoding scan judges. After a version byte of `1`, the remainder (never the whole buffer)
goes through `pglet_core::encoding::text_from_utf8` and then the jsonb reader, as before.
So a NUL or illegal UTF-8 byte in the *text* is still `22021`, and text the reader
refuses gets whichever of the reader's own codes applies: `22P02` where it can't parse
the text, and where it parses and then refuses, that refusal's own code. `22P05` for a
`\u0000` escape and `22003` for a number past the numeric cap are the two we tested.

An **empty** payload has no first byte to read. It's `08P01 insufficient data left in
message`, from inside `pq_getmsgint`, before any version logic (§ A binary parameter is
consumed exactly).

**Reach.** Only one call site outside the `binary` module reaches `value_from_binary`:
`decode_param`'s `format.is_binary()` branch. So the rule is reachable only through the
native server's extended protocol. Inside the module, `decode_binary_array` reaches the
same arm again through `recv_binary`, once per array element, and never recurses further.
An array header whose element OID names an array type pglet recognises is refused, and
one it doesn't recognise falls back to the parameter's scalar element type (§ A binary
parameter is consumed exactly). The wasm hosts bind from JS through the text path, and
`COPY … BINARY` parses but fails loud with `0A000` at execution without reaching a binary
decoder.

A **`jsonb[]` element** does reach the arm, but under its own condition, because
`decode_binary_array` takes the element type from the **wire array header**. The check
runs on an element only where that header names `jsonb`, or names an OID pglet doesn't
recognise and the fallback (the parameter's own element type) is `jsonb`. A header naming
another scalar type pglet recognises decodes every element as *that* type and never
reaches this check. The header's element OID is checked against the parameter's type only
for an array type pglet recognises, and the rest is `EDGES.md` row 140.

**18.6 reaches its version byte under a narrower condition, and the two conditions only
partly overlap.** `array_recv` refuses a header whose element OID differs from the
expected one only where **both** OIDs are below `FirstGenbkiObjectId` (`10000`). At or
above that floor it assigns the expected element type and carries on, which is exactly
pglet's fallback. So where the header names `jsonb`, and where it names an unrecognised
OID at or above the floor, the two engines run the same version check. They agreed at
every leading byte we tried: the `01` control, the three no-version-byte spellings above,
and `0x20`, `0x00`, `0xff` and `0x02`. Where the header names an unrecognised OID *below*
the floor, 18.6 has already refused the whole parameter with `42804` (tested at `9999`,
the correct-version control included), and pglet's fallback runs this check instead. A
check added here has to reproduce that condition instead of validating the header
unconditionally, or it will refuse what 18.6 accepts.

No `test_fixtures` fixture can cover any of this: `@param $N jsonb '<literal>'` binds
through tokio-postgres, which builds a well-formed payload, and the directive grammar has
no raw-bytes form. The guards are `binary.rs`'s
`a_binary_jsonb_payloads_first_byte_is_its_version` and
`a_binary_jsonb_array_element_carries_its_own_version_byte`, plus `handlers.rs`'s
`bind_binary_jsonb_param_with_a_bad_version_byte_errors_xx000_at_bind`, which shows the
refusal reaches the wire at Bind.

### Equality coverage

`values_equal` has uuid / bytea / timestamp / jsonb / float arms. `WHERE id = $1` and
`id = ANY(...)` over those column types depend on them, because a missing arm silently
matches nothing.

## Test coverage

- `test_binary` extended-protocol cases (via tokio-postgres): `ANY(uuid[])` (cast and
  inferred), `@>` (bound array and `ARRAY[$n]`), `unnest`, untyped `WHERE id = $1`,
  `$1::text::jsonb`, `$1::text::timestamptz`, `$1::text::<enum>`, and the tri-state
  optional filter.
- Differential vs real PG: `tests/fixtures/params/*` (uuid eq, `ANY`, `@>`, `unnest`,
  optional filter, `LIMIT/OFFSET`, jsonb cast, the `ON CONFLICT` positions, and the
  errors-raise-at-`Parse` pair) plus `tests/fixtures/spine/*`, via the `-- @param`
  fixture directive (see `tests/README.md`). For `ON CONFLICT`, the action's
  `SET`/`WHERE` are under `on_conflict_do_update_bound` /
  `on_conflict_excluded_qualified_bound`, the pseudo-relation against a real table of
  that name under `on_conflict_excluded_not_shadowed_by_a_real_table`, and the arbiter
  under `on_conflict_arbiter_key_expr_bound`. Those two directories are clusters, not the
  whole set: a `-- @param` fixture lives beside its subject. The `RETURNING` clause
  scope's fixtures are `update/returning_from_source_param_type.sql` and
  `update/returning_from_source_only_param_type.sql`. The boolean position's are
  `bool_context/where_bound_param_is_boolean.sql`, its negative control
  `bool_context/where_bound_param_false_matches_nothing.sql`, the node-position case
  `bool_context/and_bound_param_is_boolean.sql`, and the explicit cast's
  `bool_context/where_bound_param_explicit_cast_still_wins.sql`.
- A parameter left without a type: the `params/undetermined_param_*` fixtures, which put
  the refusals next to neighbours 18.6 answers (§ A parameter left without a type is
  refused).
- A client-declared type (`-- @param-declared`): the `params/declared_*` fixtures, covering
  `COALESCE`, `CASE`, the variadics, `ARRAY[…]`, the `42804` refusals, the
  `DELETE`/`INSERT`/`UPDATE` write shapes, a declared parameter beside an undeclared one,
  the result-column types, and `@profile-lock` index probes.
- `spy_proxy` parses Parse/Bind/ParameterDescription off a live extended-protocol
  client's traffic (`deno task spy:spine:{cell,fact}` are the recording tasks), and is
  unit-tested in `tools/spy_proxy/main_test.ts`.

## Deferred / known gaps

- **A parameter 18.6 types and this engine has no rule for reports OID 0.** A `$N` in a
  position the inference doesn't reach is reported as OID 0. The native server then
  decodes it as text, and the wasm binding decodes it by its JS runtime type: a JS number
  is an `int8` if it's a whole number an `int8` holds, and a `float8` otherwise. So a
  whole number of magnitude 2^63 or more comes back as a float where 18.6 refuses it or
  answers exactly. We tested this through the npm adapter. `SELECT i + $1` over an `int4`
  column and `SELECT $1 + 1`, bound `2 ** 63`, answer `9.223372036854776e+18` under
  declared OID `23` where 18.6 is `22003`, and bound `1e21` they answer `1e+21` where
  18.6 is `22P02`. `SELECT n + $1` over a `numeric` column bound `1e21` answers `1e+21`
  where 18.6 answers `1000000000000000000001`. `SELECT -$1` likewise answers, where 18.6
  is `42725`. That's `EDGES.md` row 288, which also covers `nullif`'s second argument,
  where a fractional JS number changes the type of the value handed back. A typed
  parameter isn't decoded by its JS type: `$1::numeric` answers as 18.6 does and an `int8`
  target refuses (`22P02`; 18.6 `22003`), while a `float4` target bound
  `Number.MAX_VALUE` answers `Infinity` where 18.6 is `22003`. Postgres never reports OID
  0 for a used parameter. (What it settles without a context is settled here too: a bare
  select-list or `RETURNING` entry is `text`, and a parameter nothing types is refused —
  § A parameter left without a type is refused.)

  pglet's OID 0 trips up a **binary-protocol** client (tokio-postgres) during type
  introspection. A text-protocol client (node-postgres) tolerates it. That introspection
  query joins `pg_catalog.pg_range`, a relation pglet doesn't have, so the lookup is
  `42P01` here. Inside a transaction block that aborts the block, every later statement
  reports `25P02`, and the driver's own `UNDEFINED_TABLE` fallback query doesn't rescue it
  (measured; `EDGES.md` row 57's bullet).

  Where this engine *does* type a `$N` is sketched under **Parse** above and in the
  **Function-signature inference** bullet below. Neither sketch is complete. The full list
  is `param_types.rs`'s own `Inference::record` call sites, since that method is the only
  writer of the map. A `$N` none of them reaches decodes as text, and the comparison then
  fails **loud** instead of mis-decoding: `42883 operator does not exist: <type> = text`
  against a non-text column. (The type named is the operand's as pglet names it:
  `timestamp without time zone` for a `timestamptz` column, where 18.6 says `with`.) We
  tested that for `CASE`, `COALESCE`, `greatest` and `ANY(ARRAY[$1])`, which between them
  cover `int4`, `bigint`, `numeric`, `float8`, `bool` and `timestamptz` columns, and 18.6
  answers `42883` on each shape too. Across an arithmetic operator (`n = n + $1`,
  `id > $1 + 0`) it's `42804`, and `abs($1)` is `0A000`. In those last two 18.6 resolves
  the type instead, so pglet is **stricter** there, not looser.
- **An unrecognized *client-declared* OID silently falls back to text inference**
  instead of failing loud. `Parse` priority puts a client-declared OID first
  ("authoritative, as in Postgres"), but `TypeOid::from_oid` maps **both** OID 0
  ("infer") **and any real-but-unmodeled OID** (`point` 600, `inet` 869, `money` 790,
  `bit` 1560) to `None`. So a binary client that explicitly types `$1` as one of those
  gets the same text fallback instead of PG's `42704`. Since the declared type never
  reaches parse analysis either, it also gets the same inference: `coalesce($1, 1)`
  declared `point` and bound `(1,2)` is `22P02` here against 18.6's
  `42804 COALESCE types point and integer cannot be matched`, and with nothing typing it,
  `SELECT $1 IS NULL` declared `point` is `42P18` here where 18.6 answers (`EDGES.md`
  row 276). This is the one type-declaration entry point that doesn't go through the
  fail-loud resolver. It's a narrow correct-or-loud hole, reachable only by a client that
  declares a param OID itself. tokio-postgres, the client the harness drives, sends empty
  param types for `@param` (§ Why infer at Parse), and `@param-declared` declares only
  types pglet models, so no fixture exercises the fallback.

  The fix is to fail loud on an unrecognized non-zero declared OID. That can't be a check
  added where the fallback happens, because by then the distinction is gone. `Parse`
  decoding runs every declared OID through `pglet_core`'s `TypeOid::from_oid`, whose `0`
  arm and unknown arm both return `None` (that function's own comment records the two as
  "distinct meanings" sharing a body). So `param_types` can't tell "infer this for me"
  from "I named a type you don't model". Closing the hole means carrying that difference
  out of `pglet_wire`'s decoder first. A check on the collapsed `None` would reject the
  protocol's ordinary `0`.
- **The Execute message's max-row-count is ignored.** The field is decoded and then
  dropped (`handlers.rs`), and pglet never sends `PortalSuspended`, so a row-limited
  Execute returns the full buffered result plus `CommandComplete`. That matches Postgres
  only for the fetch-all case (`max_rows = 0`, which is what tokio-postgres and
  node-postgres send). A cursor-style client (`pg-cursor`, row-limited `FETCH`) would get
  everything at once, and each further `Execute` it sends for the rest runs the statement
  again, so a data-modifying one writes again (`EDGES.md` row 169). Real portal
  suspend/resume would pair naturally with a streaming (volcano-model) executor
  ([storage.md](storage.md) § Known bottlenecks), if one ever lands.
- **Function-signature inference** covers operators plus a per-function argument table
  for the `StringFunc` family (`string_fn_text_arg` in `param_types.rs`): `UPPER`,
  `LENGTH`, `SUBSTRING`, `REPLACE`, `LTRIM`/`RTRIM`/`BTRIM`, `STRPOS`/`POSITION`,
  `LPAD`/`RPAD`, `SPLIT_PART`, `DATE_TRUNC`, `CURRENT_SETTING`. Each types its
  text-position argument(s) as `text`. The table also names `CURRENT_SCHEMAS`,
  `PG_GET_CONSTRAINTDEF` and the `::regclass` cast as typing no position; take the
  current list from the match, not from this doc. Integer positions (substring's
  start/length, `lpad`/`rpad`'s length) are left to their own context. `LOWER` (its own
  `Expr` node), `||`, `EXTRACT`'s field argument, and `concat_ws`'s separator are typed by
  their own walk arms instead of that table. A function outside these isn't typed from a
  signature.

The per-feature ledger for this path is [compatibility.md](compatibility.md).
