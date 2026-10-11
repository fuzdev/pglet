# Arrays — `text[]` / `uuid[]` + `@>`, `ANY`, `unnest`

Arrays are a real value and type in pglet. They aren't coerced to a `Text("{a,b}")`
string. A `text[]` or `uuid[]` column round-trips as a typed one-dimensional array.
`@>`, `ANY` and `unnest` work scan-based, and `@>` can also use a GIN index (below).
Arrays bind as extended-protocol parameters (`$N::text[]` / `$N::uuid[]`, see
[param_binding.md](param_binding.md)). The tests that cover this are in § Test coverage.

## Design: flat array OIDs + `Value::Array` carrying its element type

Postgres gives each array type its own OID. pglet does the same with flat `*Array`
variants on the closed `#[repr(u32)]` `TypeOid`, so it stays `Copy`. The element type is
stored on the value instead of nested in the OID. The enum type makes the same choice:
we don't add an OID variant that carries data.

- `TypeOid::{Bool,Bytea,Int2,Int4,Int8,Float4,Float8,Text,Varchar,Timestamp,
  Timestamptz,Uuid,Jsonb,Date,Time,Interval,Numeric,Name}Array` with Postgres's canonical
  array OIDs (`text[]`=1009, `uuid[]`=2951, `date[]`=1182, `numeric[]`=1231, …). `name[]`
  is a declared type with no column form (repo `CLAUDE.md` § Type system).
  `array_of(elem)` and `array_element()` map between an element type and its array type,
  and `is_array()` tests for one. `from_oid` and `wire_size` cover them.
- `Value::Array(ArrayValue)` where `ArrayValue { element_type: TypeOid, elements:
  Vec<Value> }`. Storing `element_type` keeps empty arrays and `unnest` output typed. The
  array's own `TypeOid` is `element_type.array_of()`. `ArrayValue::to_text_format` is the
  only place the `{a,b,NULL}` literal is built (`array::text` and the text wire format
  both use it).
- One dimension only. The element type is a scalar, and pglet builds no nested arrays.

## What works

- **DDL / typing.** `refs TEXT[]`, `ids UUID[]` and `vals INT[]` resolve to a real array
  column. The DDL column path and the `CAST`/`::` path share one resolver (`executor`'s
  `resolve_type_name`, whose `decompose_type_name` strips the `[]` suffix), so an array
  cast like `'{a,b}'::text[]` is typed the same way a column is.

  **A `[]` next to a `(…)` makes the modifier the element's.** `varchar(3)[]` is an array
  of `varchar(3)`, and `numeric(4,1)[]` is an array of `numeric(4,1)`. That's because the
  canonical name is decomposed once, with the suffix taken off before the paren
  (`decompose_type_name`). The cap is then enforced **element-wise**, both on store and on
  an explicit cast. A `(…)` the element type refuses fails the whole spelling, as it does
  on 18.6.

  pglet reaches two of `typenameTypeMod`'s refusals that include a type name, and they
  name *different* types. A modifier on an element type that takes none is
  `42601 type modifier is not allowed for type "int4[]"` (`int4(3)[]`), which names the
  **array**. A modifier the type itself refuses is the element's own `22023`, which names
  the **element**: `'{ab}'::char(0)[]` is `length for type char must be at least 1`, and
  `'{ab}'::numeric(0)[]` is `NUMERIC precision 0 must be between 1 and 1000`. Both were
  checked on both engines. So which name you see depends on which refusal fired, not on
  the `[]`.

  A third check also fires at this spelling and names **neither** type. It's the `int4`
  coercion of a `numeric` modifier component, which names only `integer`, its own
  coercion target. It's only reached for a type that takes a modifier (so never where the
  array-naming `42601` fires), and it runs before that type's own range checks.
  `'{ab}'::numeric(2147483648)[]` is `22003 integer out of range` (18.6 says
  `value "2147483648" is out of range for type integer`, the message difference recorded
  in `EDGES.md` row 124). `'{ab}'::numeric(1.5)[]` is
  `22P02 invalid input syntax for type integer: "1.5"`, byte for byte on both. Each was
  tested at the array spelling. `EDGES.md` row 124's unit test
  (`the_length_refusal_names_the_internal_type_and_an_array_names_its_element`) and fixture
  (`types/refused_typmod_char_length_array_names_element_loud.sql`) lock in which
  type each refusal names.

  `FLOAT(n)` works differently, even though `float4` and `float8` take no modifier either.
  There the `(n)` *selects* the element type in the grammar, before `typenameTypeMod` is
  reached. So `float(1)[]` is a `_float4` column and `float(30)[]` a `_float8` one, with
  `udt_name` equal on both engines. That's the fourth case in `docs/coercion.md`
  § A canonical type name decomposes once, which holds the rule. Splitting on `(` first
  would resolve `varchar(3)[]` to the *scalar* element type and drop the cap along with
  the array, which affects writes (`EDGES.md` row 120, closed).
- **Construction + casts.** `ARRAY['a','b']` (planner `Expr::Array`), the `'{a,b}'` text
  literal, and `ARRAY[...]::uuid[]` / `'{...}'::text[]` casts. `array::text` renders the
  canonical literal with Postgres quoting.
- **The literal's grammar matches 18.6.** `executor::array_parse` is 18.6's `array_in`
  split into two functions. `read_array_token` is its `ReadArrayToken`: it reads a brace,
  a delimiter, a quoted element, or an unquoted element with backslash escapes, dropping
  trailing whitespace and recognising a bare `NULL`. `parse_array_literal` is its
  `ReadArrayStr`, which decides where each token may appear. Whitespace is exactly 18.6's
  `scanner_isspace` set, so a Unicode space counts as part of an element. A literal that
  this walk refuses as malformed is `22P02` for every element type. That covers an empty
  element (`'{a,,b}'`), a brace or quote inside an unquoted element, and anything but
  whitespace after the closing brace. **Each element is converted as it's read**, so when
  there are two errors, the one reported matches 18.6. An element's own input error comes
  before a structural error later in the literal (`'{x}b}'::date[]` is `22007`), and an
  element cut short by a token is never converted (`'{x{}'::date[]` is `22P02`). The same
  reader parses the jsonb path operand (`#>`, `#>>`).
- **`@>` / `<@` containment** (`BinaryOp::Containment` / `ContainedBy`). These use
  set-membership semantics, matching PG's `array_contain_compare`. Duplicates don't matter
  (`{1,1} @> {1}` and `{1} @> {1,1}` are both true), and the empty array is contained by
  anything. **NULL elements match Postgres in both directions.** A NULL in the
  *contained* operand makes `@>` false (PG breaks with `result = false`), and a NULL in
  the *container* never satisfies an element (PG `continue`s past it). Both follow from
  `values_equal` returning `None` for a NULL operand, with no special case, and both are
  locked by differential fixtures.

  Both operands must be arrays of the **same** element type. Postgres resolves
  `anyarray` per call and doesn't unify across operands, so `integer[] @> bigint[]` is
  `42883` there and here (`planner::containment_legality`). The rule reads pglet's array
  type, and an `ARRAY[…]` over an enum is `text[]` here, so `ARRAY[m] @> ARRAY[t]` returns
  a result where 18.6 gives `42883 … e1[] @> text[]` (`EDGES.md` row 190). Containment is
  scan-based, or **GIN-accelerated** for a top-level `col @> <const>` conjunct (see
  *GIN-accelerated `@>`* below). `<@` is `@>` with the operands swapped.
- **`= ANY(array)` / `op ALL(array)`** (`Expr::AnyAll`). Element-wise comparison with
  Postgres's NULL and empty-array semantics. The empty-array short-circuit comes before
  the NULL-LHS check (`ANY` over ∅ is false, `ALL` over ∅ is true, even for a NULL LHS),
  matching PG's `ExecEvalScalarArrayOp`.
- **`unnest(a, b, …)` in FROM** (`Plan::FunctionScan`, the same node a FROM-clause
  `generate_series` uses, see `docs/compatibility.md`). You get one column per array,
  zipped element-wise and NULL-padded to the longest, with `AS p(c1, c2)` column aliases.
  The node keeps its effective FROM name (the alias if there is one, otherwise `unnest`).
  Default column names follow Postgres's `chooseScalarFunctionAlias`. A **single**-array
  `FROM unnest(a) AS v` names its one column `v`. A multi-array call names **every**
  column `unnest`, so there a bare `unnest` reference is `42702` and the alias names the
  composite row. The item isn't lateral: an argument can't name an earlier `FROM` item's
  column (`EDGES.md` rows 312 and 313). Scope claimant-counting (e.g. `RETURNING`'s) and
  the executor's relation keys resolve against that name.

  `SELECT v.* FROM unnest(…) AS v` is `0A000` here, where Postgres expands it. That's a
  different walk from the qualified-column one, and it's only `0A000` where no base table
  is spelled `v`. Where one is, the walk falls through to the catalog and silently
  expands *that table's* columns. A bare `*` that isn't the projection's only item, over
  a non-join `FROM`, goes through the same walk and splits the same way. `EDGES.md` row 8.
- **Wire.** Text format is `{…}`. Binary format is the full Postgres array layout (ndim,
  flags, element OID, dim length+lbound, length-prefixed elements), so the differential
  harness, which uses the binary protocol, decodes both engines.

## Test coverage

- Engine cases in `crates/pglet_sql/tests/engine_arrays.rs`: roundtrip, `@>`/`<@`,
  `ANY`, `unnest` single + zip, `array::text`.
- Differential fixtures vs real PG (`tests/fixtures/array/*`): text[]/int[]/uuid[]
  roundtrip incl. empty + NULL, `@>`, `<@`, `ANY`, `unnest` single + zip, `array::text`.

### Harness note (array result decoding)

The differential harness **decodes array result columns** in `pglet_tests`'
`column_value_to_json`, with one arm per array type pglet declares. `int8[]` and `uuid[]`
elements come back as strings (matching node-postgres), so bare array columns compare
directly. A type with no arm falls to `UndecodedType`, whose `accepts()` is `false`, so
it's a loud client error instead of a `null`. That's one of the reasons the enum fixtures
cast to `text` ([enums.md](enums.md) § Harness note).

## Parameter binding (`$N::text[]`)

The corpus covers arrays as bound params in four shapes: `refs @> $1`
(`tests/fixtures/params/array_contains.sql`), `id = ANY($1::uuid[])`
(`tests/fixtures/params/any_uuid_array.sql`), `refs @> ARRAY[$1]::text[]`
(`tests/fixtures/spine/cell/cell_list_refs_guard.sql`), and
`unnest($1::text[], $2::uuid[])` (`tests/fixtures/params/unnest_pairs.sql`).
Prepared-statement parameter binding ([param_binding.md](param_binding.md)) decodes a
bound `Vec<Uuid>`/`Vec<String>` into a `Value::Array` (the inverse of the binary array
encoder) and infers the array type from context, so a bound array works with the same
operators a literal one does. Those four, plus the rest of `tests/fixtures/params/*` and
`tests/fixtures/spine/*`, run against real Postgres.

## GIN-accelerated `@>`

`CREATE INDEX … USING gin(col)` builds an inverted index over an array or JSONB column,
with one entry per decomposed element or JSONB key+value, in the same B+tree keyspace.
The planner sends a top-level `col @> <const>` conjunct to a `GinScan`, which intersects
the query keys' posting lists. The probe constant can be a literal, a `$N` parameter, or
a **correlated outer-scope** column (a `NOT EXISTS (… c.refs @> ARRAY[f.hash]::text[])`,
as in `tests/fixtures/spine/fact/orphan_facts_list.sql`), resolved per execution.

An array's entries are its elements' own keys, so a `jsonb[]` element matches by
`jsonb` equality (`ARRAY['{"a":1.0}'::jsonb]` finds a row holding `{"a": 1}`). A probe
whose type isn't the column's, like `numeric[] @> ARRAY[1]`, which has no operator,
isn't looked up at all. The scan passes every row to the recheck, and the recheck
decides the result for that pair ([indexes.md](indexes.md) § A container key). The type
that counts is the probe value's. An `ARRAY[…]` over `timestamptz` or `varchar` elements
is a `timestamp[]` / `text[]` value, so it's declined against a `timestamptz[]` /
`varchar[]` column. There the recheck refuses it with `42883`, while 18.6 returns a
result (`EDGES.md` row 25).

Partial GIN indexes (`… WHERE deleted_at IS NULL`) are used when the query implies their
predicate. The index is **lossy** (JSONB containment, NULL array elements), so `GinScan`
returns a superset of candidates and is always wrapped in a recheck `Filter` that
re-applies the real `@>`. That keeps results exact. See `docs/storage.md` § Known
bottlenecks (the in-transaction GIN probe) and the `tests/fixtures/index/gin_*`
differential fixtures. `deno task bench:gin` measures the speedup over the scan-based
path.

Detection covers a **top-level AND conjunct** and also an `OR` of containments. A
`GinScan` has `probes` (the `@>` constants to decompose) and `guards` (booleans that
don't depend on the scan). A top-level `OR` qualifies when every disjunct is either a
same-column `@>` containment (it becomes a `probes` entry, and posting lists are
**unioned**) or a scan-independent boolean guard (it becomes a `guards` entry). Neither a
probe nor a guard can contain a volatile call (`random()`, `gen_random_uuid()`,
`clock_timestamp()`). Evaluated once, it would give one answer for the whole scan, so the
`OR` uses the scan instead.

At execution, any guard that evaluates true makes the `OR` always satisfied, which means
a full scan (the recheck `Filter` keeps it correct). A guard that **fails** does the same.
It's evaluated once, before any row, while the `OR` it came from evaluates it per row and
only when no earlier disjunct decides the row. So its error is left for the `Filter` to
raise for a row that reaches it, which is what the stripped build and 18.6 both do.

This handles two `OR` shapes: a multi-key probe (`col @> a OR col @> b`,
`tests/fixtures/spine/cell/audit_metadata_multikey.sql`) and a param-null guard
(`$p IS NULL OR col @> $p`, where a null-bound `$p` fires the guard;
`tests/fixtures/spine/cell/cell_list_refs_guard.sql` and its `_null` twin). Guards are
resolved at eval time, so a prepared statement keeps one immutable cached plan, with no
rewrite per binding. A disjunct that depends on the scan and isn't such a containment,
or probes spanning more than one column, fall back to the scan-based `@>`. That's where
the corpus's third `OR` shape ends up: `d @> … OR d @> … OR tags @> …`, spanning two
columns, in `tests/fixtures/jsonb/containment_bare_literal.sql`.

## Not covered (deferred)

- **`||` concatenation** (`array_cat` and the array/element forms). It's legal in
  Postgres but not implemented here, so it fails with `0A000` instead of stringifying
  both operands and joining them. Also missing: **`&&` overlap**, `array_agg`,
  subscripting (`a[1]`), `array_length`/`cardinality`, and `= ANY(subquery)` (only
  `ANY(array)` is built; `IN (SELECT …)` covers the subquery form). We'll add these as
  consumers need them.
- **Multi-dimensional arrays.** One dimension only, and both spellings refuse a second:
  the constructor (`ARRAY[ARRAY['a','b']]`) and a nested **literal**
  (`'{{a,b}}'::text[]`) are `0A000`, for every element type. The literal is first walked
  the way 18.6 walks it, converting its elements and comparing its sub-array lengths. So
  a literal 18.6 refuses gets 18.6's error (`'{{1,2},{3}}'::int[]` is `22P02`,
  `'{{x}}'::date[]` is `22007`), one with no elements is the empty array as it is there
  (`'{{}}'`), and only a literal 18.6 would build gets the `0A000`. Explicit bounds
  (`'[1:2]={a,b}'`) are refused with `22P02`, and a seventh nesting level is `0A000`
  where 18.6 gives `54000`. `EDGES.md` row 283.
- **Array functions.** `array_append`, `array_prepend` and `array_length` are among the
  ones we found to fail with `0A000` (`EDGES.md` row 44).
- **A constructor's type where an operator or a cast reads it.** `ARRAY[…]` over
  `timestamptz`, `varchar` or `name` elements is compared as `timestamp[]` / `text[]` by
  `@>`, `<@` and `=` (`EDGES.md` row 281). Under `ARRAY[…]::T[]` the constructor is
  resolved before the cast, so `ARRAY['NaN', 0]::float8[]` is `22P02` and `ARRAY['01',
  0]::text[]` is `{1,0}` (`EDGES.md` row 282). Casting the constructor to the column's
  array type, or using a typed element, gives the same result as 18.6.
- **Text-format fidelity for elements other than text/uuid** is best-effort (e.g. float
  and timestamp element spelling). Element quoting tests Rust's `is_whitespace`, a
  superset of the `scanner_isspace` set the literal reader skips. So an element holding a
  Unicode space is quoted where 18.6 prints it bare, and it reads back the same
  (`EDGES.md` row 294). Binary is the tested path, and the harness uses binary.
- **Enum arrays** (`cell_visibility[]`). An enum column is `TypeOid::Text` plus a catalog
  name, with no array OID, so `cell_visibility[]` can't be represented yet. It'll be added
  if a consumer needs it.
