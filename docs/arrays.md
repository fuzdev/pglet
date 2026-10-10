# Arrays — `text[]` / `uuid[]` + `@>`, `ANY`, `unnest`

Arrays are a real value/type, not a silent `Text("{a,b}")` coercion. A `text[]`
or `uuid[]` column round-trips as a typed one-dimensional array, `@>` / `ANY` /
`unnest` work scan-based (`@>` also GIN-accelerated, below), and arrays bind as
extended-protocol parameters (`$N::text[]` / `$N::uuid[]` — see
[param_binding.md](param_binding.md)). Guards: § Test coverage.

## Design: flat array OIDs + `Value::Array` carrying its element type

Postgres gives each array type its own OID. pglet mirrors that with flat `*Array`
variants on the closed `#[repr(u32)]` `TypeOid` (so it stays `Copy`); the element
type is carried on the value rather than nested in the OID — the same "don't mint a
data-carrying OID variant" move the enum type makes.

- `TypeOid::{Bool,Bytea,Int2,Int4,Int8,Float4,Float8,Text,Varchar,Timestamp,
  Timestamptz,Uuid,Jsonb,Date,Time,Interval,Numeric,Name}Array` with Postgres's canonical
  array OIDs (`text[]`=1009, `uuid[]`=2951, `date[]`=1182, `numeric[]`=1231, …); `name[]`
  is a declared type with no column form (repo `CLAUDE.md` § Type system).
  `array_of(elem)` / `array_element()` map between an element type and its array
  type; `is_array()` tests it. `from_oid` / `wire_size` cover them.
- `Value::Array(ArrayValue)` where `ArrayValue { element_type: TypeOid, elements:
  Vec<Value> }`. Carrying `element_type` keeps empty arrays and `unnest` output typed;
  the array's own `TypeOid` is `element_type.array_of()`. `ArrayValue::to_text_format`
  is the single source for the `{a,b,NULL}` literal (used by `array::text` and the text
  wire format).
- One dimension only. The element type is a scalar; pglet builds no nested arrays.

## What works

- **DDL / typing.** `refs TEXT[]`, `ids UUID[]`, `vals INT[]` resolve to a real array
  column. The DDL column path and the `CAST`/`::` path share one resolver
  (`executor`'s `resolve_type_name`, whose `decompose_type_name` takes the `[]` suffix
  off), so an array cast (`'{a,b}'::text[]`) types the same way a column does. A **`[]`
  beside a `(…)` makes the modifier the element's** — `varchar(3)[]` is an array of
  `varchar(3)`, `numeric(4,1)[]` an array of `numeric(4,1)` — because the canonical name
  decomposes once, the suffix off before the paren (`decompose_type_name`). The cap is
  then enforced **element-wise**, on store and on an explicit cast alike, and a `(…)` the
  element type refuses takes the whole spelling down as it does on 18.6 — at either of the
  two **name-bearing** `typenameTypeMod` refusals pglet reaches, which name *different*
  types: a modifier on an element that carries none is
  `42601 type modifier is not allowed for type "int4[]"` (`int4(3)[]`,
  naming the **array**), while one a carrier itself refuses is the element's own `22023`
  naming the **element** (`'{ab}'::char(0)[]` is `length for type char must be at least 1`,
  `'{ab}'::numeric(0)[]` is `NUMERIC precision 0 must be between 1 and 1000`), both
  measured on both engines. Which name a reader sees is therefore decided by *which*
  refusal fired, not by the `[]`. A third layer answers at this spelling too and names
  **neither** — the `int4` coercion of a `numeric` component, which names only `integer`,
  its own coercion target. It is reached only for a carrier (so never where the array-naming
  `42601` fires) and runs ahead of that carrier's own range checks:
  `'{ab}'::numeric(2147483648)[]` is `22003 integer out of range`
  (18.6: `value "2147483648" is out of range for type integer` — the message residual
  `EDGES.md` row 124 records) and
  `'{ab}'::numeric(1.5)[]` is `22P02 invalid input syntax for type integer: "1.5"` byte for
  byte on both, each measured at the array spelling.
  `EDGES.md` row 124, whose unit test
  (`the_length_refusal_names_the_internal_type_and_an_array_names_its_element`) and fixture
  (`types/refused_typmod_char_length_array_names_element_loud.sql`) pin the asymmetry.
  A `FLOAT(n)` is **not** that shape,
  though `float4`/`float8` carry no modifier either: there the `(n)` *selects* the
  element type in the grammar, before `typenameTypeMod` is reached, so `float(1)[]` is a
  `_float4` column and `float(30)[]` a `_float8` one — `udt_name` measured equal on both
  engines, and the fourth arm of the section this bullet already points at. Splitting on
  `(` first would resolve `varchar(3)[]` to the *scalar* element and lose the cap with
  the array, a write — `EDGES.md` row 120, closed; `docs/coercion.md` § A canonical type
  name decomposes once carries the rule.
- **Construction + casts.** `ARRAY['a','b']` (planner `Expr::Array`), the `'{a,b}'`
  text literal, and `ARRAY[...]::uuid[]` / `'{...}'::text[]` casts. `array::text` renders
  the canonical literal with Postgres quoting.
- **The literal's grammar is 18.6's.** `executor::array_parse` is 18.6's `array_in` in two
  functions: `read_array_token` (its `ReadArrayToken` — a brace, a delimiter, a quoted
  element, or an unquoted one with backslash escapes, trailing whitespace dropped and a
  bare `NULL` recognised) and `parse_array_literal` (its `ReadArrayStr` — where each token
  may stand). Whitespace is 18.6's `scanner_isspace` set and nothing else, so a Unicode
  space is an element's character. A literal that walk refuses as malformed is `22P02`
  whatever the element type — an empty element (`'{a,,b}'`), a brace or a quote inside an unquoted
  element, anything but whitespace after the closing brace. **Each element is converted
  as it is read**, so the order of two errors is 18.6's: an element's own input error
  comes ahead of a structural error later in the literal (`'{x}b}'::date[]` is `22007`),
  and an element a token cut short is never converted (`'{x{}'::date[]` is `22P02`).
  The same reader takes the jsonb path operand (`#>`, `#>>`).
- **`@>` / `<@` containment** (`BinaryOp::Containment` / `ContainedBy`):
  set-membership semantics matching PG's `array_contain_compare` — duplicates are
  irrelevant (`{1,1} @> {1}` and `{1} @> {1,1}` both true), the empty array is
  contained by anything, and **NULL elements match Postgres in both directions**: a
  NULL in the *contained* operand makes `@>` false (PG breaks with `result = false`),
  a NULL in the *container* simply never satisfies an element (PG `continue`s past
  it). Both fall out of `values_equal` returning `None` for a NULL operand rather
  than needing a special case, and both are differential-locked. Operands must be two
  arrays of the **same** element type — Postgres resolves `anyarray` per call rather
  than unifying across operands, so `integer[] @> bigint[]` is `42883` there and here
  (`planner::containment_legality`). The rule reads pglet's array type, and an
  `ARRAY[…]` over an enum is `text[]` here, so `ARRAY[m] @> ARRAY[t]` answers where 18.6
  is `42883 … e1[] @> text[]` (`EDGES.md` row 190). Scan-based, or **GIN-accelerated** for a
  top-level `col @> <const>` conjunct (see *GIN-accelerated `@>`* below). `<@` is `@>`
  with operands swapped.
- **`= ANY(array)` / `op ALL(array)`** (`Expr::AnyAll`): element-wise comparison with
  Postgres NULL/empty semantics — the empty-array short-circuit precedes the NULL-LHS
  check (`ANY` ∅ → false, `ALL` ∅ → true even for a NULL LHS), matching PG's
  `ExecEvalScalarArrayOp`.
- **`unnest(a, b, …)` in FROM** (`Plan::FunctionScan`, the node a FROM-clause
  `generate_series` is too — `docs/compatibility.md`): one column per array, zipped
  element-wise and NULL-padded to the longest, with `AS p(c1, c2)` column aliases.
  The node carries its effective FROM name (the alias when written, else `unnest`),
  and default column names follow Postgres's `chooseScalarFunctionAlias`: a
  **single**-array `FROM unnest(a) AS v` names its one column `v`, while a
  multi-array call names **every** column `unnest` — so there a bare `unnest`
  reference is `42702` and the alias names the composite row. The item is not
  lateral — an argument cannot name an earlier `FROM` item's column (`EDGES.md` rows
  312 and 313). That name is what
  scope claimant-counting (e.g. `RETURNING`'s) and the executor's relation keys
  resolve against. (`SELECT v.* FROM unnest(…) AS v` is `0A000` here where
  Postgres expands it — a different walk from the qualified-column one, and
  `0A000` only where no base table is spelled `v`; where one is, that walk falls
  through to the catalog and silently expands *its* columns. Same walk, same
  polarities, for a bare `*` that is not the projection's sole item over a
  non-join `FROM`. `EDGES.md` row 8.)
- **Wire.** Text format `{…}`; binary format is the full Postgres array layout (ndim,
  flags, element OID, dim length+lbound, length-prefixed elements), so the differential
  harness (binary protocol) decodes both engines.

## Test coverage

- Engine cases in `crates/pglet_sql/tests/engine_arrays.rs`: roundtrip, `@>`/`<@`,
  `ANY`, `unnest` single + zip, `array::text`.
- Differential fixtures vs real PG (`tests/fixtures/array/*`): text[]/int[]/uuid[]
  roundtrip incl. empty + NULL, `@>`, `<@`, `ANY`, `unnest` single + zip, `array::text`.

### Harness note (array result decoding)

The differential harness **decodes array result columns** in `pglet_tests`'
`column_value_to_json` — one arm per array type pglet declares, `int8[]` and `uuid[]`
elements as strings (matching node-postgres) — so bare array columns compare directly.
A type with no arm falls to `UndecodedType`, whose `accepts()` is `false`: a loud client
fault, not a `null` — one of the reasons the enum fixtures cast to `text`
([enums.md](enums.md) § Harness note).

## Parameter binding (`$N::text[]`)

Arrays pass as bound params in four shapes the corpus pins: `refs @> $1`
(`tests/fixtures/params/array_contains.sql`), `id = ANY($1::uuid[])`
(`tests/fixtures/params/any_uuid_array.sql`), `refs @> ARRAY[$1]::text[]`
(`tests/fixtures/spine/cell/cell_list_refs_guard.sql`), and
`unnest($1::text[], $2::uuid[])` (`tests/fixtures/params/unnest_pairs.sql`).
Real prepared-statement parameter binding ([param_binding.md](param_binding.md))
decodes a bound `Vec<Uuid>`/`Vec<String>` into a `Value::Array` (the inverse of
the binary array encoder) and infers the array type from context, so a bound
array reaches the operators a literal one does. Those four and the rest of
`tests/fixtures/params/*` and `tests/fixtures/spine/*` run vs real Postgres.

## GIN-accelerated `@>`

`CREATE INDEX … USING gin(col)` builds an inverted index over an array or JSONB
column (one entry per decomposed element / JSONB key+value, in the same B+tree
keyspace), and the planner routes a top-level `col @> <const>` conjunct to a
`GinScan` that intersects the query keys' posting lists. The probe constant may be
a literal, a `$N` parameter, or a **correlated outer-scope** column (a
`NOT EXISTS (… c.refs @> ARRAY[f.hash]::text[])`, as in
`tests/fixtures/spine/fact/orphan_facts_list.sql`), resolved per execution.
An array's entries are its elements' own keys, so a `jsonb[]` element is matched by
`jsonb`'s equality (`ARRAY['{"a":1.0}'::jsonb]` finds a row holding `{"a": 1}`), and a
probe whose type is not the column's — `numeric[] @> ARRAY[1]`, which has no operator —
is not looked up at all: the scan hands the recheck every row and the recheck answers
for the pair ([indexes.md](indexes.md) § A container key). The type is the probe value's: an `ARRAY[…]` over `timestamptz` or `varchar` elements is a `timestamp[]` / `text[]`
value and declines against a `timestamptz[]` / `varchar[]` column, where the recheck refuses it `42883` and 18.6
answers (`EDGES.md` row 25).
Partial GIN indexes (`… WHERE deleted_at IS NULL`) are used when the query implies
their predicate. Because the index is **lossy** (JSONB containment, NULL array
elements), `GinScan` yields a candidate superset and is always wrapped in a recheck
`Filter` re-applying the real `@>`, so results stay exact. See `docs/storage.md`
§ Known bottlenecks (the in-transaction GIN probe) and the
`tests/fixtures/index/gin_*` differential fixtures; `deno task bench:gin`
measures the speedup over the scan-based path.

Detection covers a **top-level AND conjunct** *and* an `OR` of containments. A
`GinScan` carries `probes` (the `@>` constants to decompose) plus `guards`
(scan-independent booleans). A top-level `OR` qualifies when every disjunct is
either a same-column `@>` containment (→ a `probes` entry, posting lists
**unioned**) or a scan-independent boolean guard (→ a `guards` entry). Neither a probe
nor a guard holds a volatile call (`random()`, `gen_random_uuid()`, `clock_timestamp()`):
evaluated once it would answer once for the scan, so the `OR` takes the scan instead. At execution
any guard that evaluates true makes the `OR` unconditionally satisfied → a full scan
(the recheck `Filter` keeps it correct), and so does a guard that **fails**: it is
evaluated once, ahead of every row, where the `OR` it came from evaluates it per row and
only when no disjunct before it decides the row, so its error is left to the `Filter` to
raise for a row that reaches it — the stripped build's answer, and 18.6's. This routes two `OR` shapes — a multi-key
probe (`col @> a OR col @> b`, `tests/fixtures/spine/cell/audit_metadata_multikey.sql`)
and a param-null guard (`$p IS NULL OR col @> $p`, where a null-bound `$p` fires
the guard — `tests/fixtures/spine/cell/cell_list_refs_guard.sql` and its `_null`
twin). Because guards resolve at eval time, a prepared statement keeps one
immutable cached plan (no per-binding rewrite). A disjunct that is scan-dependent
and not such a containment, or probes spanning more than one column, fall back to
the scan-based `@>`. That is where the corpus's third `OR` shape lands —
`d @> … OR d @> … OR tags @> …`, spanning two columns, in
`tests/fixtures/jsonb/containment_bare_literal.sql`.

## Not covered (deferred)

- **`||` concatenation** (`array_cat` and the array/element forms) — Postgres-legal,
  unimplemented here, so it fails `0A000` rather than stringifying both operands and
  joining them. Also **`&&` overlap**, `array_agg`, subscripting (`a[1]`),
  `array_length`/`cardinality`, and `= ANY(subquery)` (only `ANY(array)` is built; the
  subquery form is covered by `IN (SELECT …)`). Add as consumers need them.
- **Multi-dimensional arrays.** One dimension only, and both spellings refuse a second:
  the constructor (`ARRAY[ARRAY['a','b']]`) and a nested **literal** (`'{{a,b}}'::text[]`)
  are `0A000`, whatever the element type. The literal is walked as 18.6 walks it first —
  its elements converted, its sub-array lengths compared — so one 18.6 refuses keeps
  18.6's answer (`'{{1,2},{3}}'::int[]` is `22P02`, `'{{x}}'::date[]` `22007`), one
  holding no element is the empty array as there (`'{{}}'`), and only one 18.6 would
  build takes the `0A000`. Explicit bounds (`'[1:2]={a,b}'`) are refused `22P02`, and a
  seventh nesting level is `0A000` where 18.6 answers `54000`. `EDGES.md` row 283.
- **Array functions.** `array_append`, `array_prepend` and `array_length` are among the
  ones measured `0A000` (`EDGES.md` row 44).
- **A constructor's type where an operator or a cast reads it.** `ARRAY[…]` over
  `timestamptz`, `varchar` or `name` elements is compared as `timestamp[]` / `text[]` by
  `@>`, `<@` and `=` (`EDGES.md` row 281), and under `ARRAY[…]::T[]` the constructor is
  resolved before the cast, so `ARRAY['NaN', 0]::float8[]` is `22P02` and `ARRAY['01',
  0]::text[]` is `{1,0}` (`EDGES.md` row 282). A cast on the constructor to the column's
  array type, and a typed element, answer as 18.6 does.
- **Text-format fidelity for non-text/uuid elements** is best-effort (e.g. float /
  timestamp element spelling; element quoting tests Rust `is_whitespace`, a
  superset of the `scanner_isspace` set the literal reader skips, so an element holding
  a Unicode space is quoted where 18.6 prints it bare and reads back the same —
  `EDGES.md` row 294). Binary is the tested path; the harness
  uses binary.
- **Enum arrays** (`cell_visibility[]`) — an enum column is `TypeOid::Text` + a
  catalog name, with no array OID, so `cell_visibility[]` is not yet representable.
  Folds in when/if a consumer needs it.
