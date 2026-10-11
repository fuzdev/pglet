# Errors on the wire — the structured `ErrorResponse` fields

A Postgres error is more than a string. Postgres's `send_message_to_frontend` (`elog.c`)
writes it as a sequence of typed fields: severity (`S`, and the unlocalized `V`),
SQLSTATE (`C`), primary message (`M`), then whichever of detail (`D`), hint (`H`),
context (`W`), schema/table/column/constraint (`s`/`t`/`c`/`n`) and statement position
(`P`) the error carries, and finally the C source location (`F`/`L`/`R`). Drivers expose
those by name. `pg`'s `pg-protocol` maps them onto `DatabaseError.detail` / `.hint` /
`.position` / `.schema` / `.table` / `.column` / `.constraint`, tokio-postgres exposes
all fifteen, and pglite exposes `pg`'s shape. If pglet doesn't send a field, the consumer
sees it as `undefined`.

## The rule

> **A client that reads a structured error field gets what 18.6 sends, and a field pglet
> cannot fill truthfully is omitted rather than invented.**

Both halves matter.

### One builder

Every field set is built in `pglet_core::error_fields`. `Error::wire_fields` handles an
error pglet raised, and `plain_wire_fields` (the same `S`/`V`/`C`/`M` prefix and nothing
after it) handles a message with no `Error` behind it. Every native-server message goes
through one of the two, and the wasm host reads the same accessors `wire_fields` is built
from:

- the native server's `extended_error` (and `error_response`, which wraps it) uses
  `wire_fields`,
- `error_with_code` / `fatal_with_code`, the raw-code sites for wire-framing errors, and
  `notice_response` use `plain_wire_fields`,
- `pglet_wasm`'s `pglet_error_to_js` reads `detail()` / `hint()` /
  `object_location()` / `position()`, the same accessors `wire_fields` reads, and sets
  `pg`'s property names.

A client comparing responses could spot a non-Postgres response from a missing `V` alone,
so `message_fields`, the prefix both builders share, always writes it.

### Omit rather than invent

`F`/`L`/`R` are the clearest case. pglet has no C source tree, so any value it put there
would describe a file, a line and a function that don't exist. They're never sent. The
same reasoning decides the harder cases:

- A `NOT NULL` violation raised **mid-assembly** has no whole row to describe, so it would
  send no `D`. That's why the check runs on the formed row (below).
- A `23505` on an **expression** index has no key-column name to derive and no declared
  type to render its value under, so the `Key (…)=(…)` **`D`** is dropped. `s`, `t` and
  `n` still go out, because the relation is true whatever the key looks like. The two
  facts are kept apart in the payload (`UniqueViolationDetail { table, key: Option<…> }`)
  so dropping one can't drop the other.
- A `23505` is never raised over a key that only another connection's *uncommitted* write
  holds. On the native server, the second writer waits for that block and is checked
  against what it committed (`EDGES.md` row 98). The embedded `Database` can't wait, so
  it refuses with `55P03` (row 229). Either way, its `DETAIL: Key (a)=(2) already
  exists.` names a committed row, as 18.6's does.

The inverse is also a rule: pglet knowing a name doesn't mean it should send it. 18.6's
`42703` sends no `t`/`c` on any of its four routes, `428C9` sends a detail and a hint but
no `s`/`t`/`c`, and `2BP01`, `42809`, `42P07`, `42710`, `42701`, `42803`, `42830`, a
foreign key's `42804` (with its detail), `42P10`, `42P16`, `22001`, `22003`, `22012`,
`22P02` and `25P02` send none of the four. We checked all of these on 18.6 with a raw v3
client, and the `42804` with `psql`'s verbose field display. pglet's `Error` variants
carry table and column names at several of those, but `Error::object_location` sends
them only where 18.6 calls `errtable*`. Its match arms are the full list:
`NotNullViolation`/`NotNullColumnHasNulls`, `CheckViolation`/`CheckConstraintNotSatisfied`,
`DuplicateIndexKey`/`UniqueViolation`, `ForeignKeyViolation`, `RestrictViolation`, and
`BtreeIndexRowTooLarge`, whose `n` is the **index**'s name, as
`_bt_check_third_page`'s `errtableconstraint(heap, index)` sends it. The two other
index-size `54000`s (GIN's item limit and `index_form_tuple`'s size field) send none of
the four, and neither does pglet's own page-bound refusal (all checked on 18.6 where 18.6
raises them). The B-tree refusal's `D` names the heap tuple the entry points to
(`Index row references tuple (0,2) in relation "t".`). pglet has no `ctid`, so it's
omitted.

## What decides a field

| field | rule |
| --- | --- |
| `S` / `V` | the sending host's severity, in both slots, always. They're identical on an unlocalized server, which pglet is. |
| `C` | `Error::sqlstate()`, unchanged. |
| `M` | the variant's `Display`. |
| `D` | `Error::detail()`, per variant, `Option`. |
| `H` | `Error::hint()`, per variant, `Option`. |
| `s` | the constant `IMPLICIT_SCHEMA` (`public`), sent **exactly when `t` is**. pglet models one schema. 18.6 fills `s` and `t` through one `errtable*` macro and never sends `s` alone. |
| `t` / `c` / `n` | `Error::object_location()`. |
| `P` | `Error::position()`, a **1-based Unicode codepoint** index (below). |
| `W`, `p`, `q`, `d`, `F`, `L`, `R` | never sent. |

Field **order** matches 18.6's `send_message_to_frontend`: `S V C M D H W s t c d n P`.

## `P` counts characters

18.6's `P` is a 1-based **Unicode codepoint** index into the statement text. It doesn't
count bytes or UTF-16 units. For example, `SELECT 'ααααα' AS a, FROM t` reports `P=22`
where the byte offset is 27, and `SELECT '👍👍👍' AS a, FROM t` reports `P=20` (one per
emoji, not two units or four bytes). `pglet_core::codepoint_position` does the
conversion, and every `P` pglet sends comes from it. The two `Error` constructors that
take a position (`syntax_at`, `syntax_sentence_at`) and `parser::type_name`'s two
`FloatPrecision*` variants get their position from nowhere else. `parser/stream.rs` has
three callers:

- `TokenStream::codepoint_pos`, for the current token,
- `TokenStream::token_position`, for a token already **consumed**. That's where
  `opt_float`'s `22023` points, since the number is read before it can be judged.
- `lex_error_to_error`, which a lexical failure reaches from `parser::stream`'s `pull`,
  `parser::split_statements` and the session-command readers.

Those readers (`catalog::commands`) also call it directly for their `at or near "<tok>"`
arms.

**Scope.** These fill `P`:

- the `42601`s that reach `TokenStream` (every parse error the statement grammar raises),
- a lexical failure raised from `split_statements` before any `TokenStream` exists,
- the session-command readers' `at or near "<tok>"` arms, which have a real token offset,
- the only non-`42601` that fills it at all (`Error::position`'s arms are the full list):
  a `FLOAT(n)` outside 1–53, whose `22023` points at the number as
  `parser_errposition(@2)` does. That's `P=34` for
  `SELECT 0.1234567890123456::float(0)`, `P=26` for `CREATE TABLE zq (a float(0))`, and
  `P=30` for `CREATE SEQUENCE zsq AS FLOAT(0)`, each checked on the raw wire against
  18.6.

`P` is **not** filled by the `Error::syntax` producers that carry no token. On 18.6,
`COPY pv FROM STDIN (FORMAT bogus)` reports `P=21`,
`SELECT a FROM pv UNION SELECT a,b FROM pv` `P=31`,
`CREATE TABLE pw (a INT DEFAULT DEFAULT)` `P=32`, and `SELECT * FROM a.b.c.d` `P=15`.
pglet sends none. Nor is it filled by the readers' `syntax error at end of input` arm.
There, 18.6 counts one past the **raw** statement's last character, but
`split_statements` has already trimmed the slice the reader sees (`'DEALLOCATE '` is
`P=12` in 18.6, against a trimmed length of 11 in pglet).

The set of routes where 18.6 fills a `P` and pglet sends none is **open** and has no
known total. It grows as we find more, and `EDGES.md` row 76 lists them. Most would need
a span carried through `Plan`/`Expr`, which have none: `42P01`, `42703`, `42883`,
`22P02`, `42702`, `42804`, `42803`, `42P02`, and
`0A000 cannot use subquery in check constraint`, each checked.

A `PRIMARY KEY`/`UNIQUE` **key list naming a column twice** is on the list for the same
reason. 18.6's `P` is the `Constraint` node's location (`transformIndexConstraint`'s
`parser_errposition(cxt->pstate, constraint->location)`). That's the `CONSTRAINT`
keyword when one is declared and the `PRIMARY`/`UNIQUE` keyword otherwise (`P=24` at
`CREATE TABLE t (a int, PRIMARY KEY (a, a))`, `P=20` at
`ALTER TABLE tt ADD CONSTRAINT c UNIQUE (a, a)`). We checked it at `CREATE TABLE` and
`ALTER TABLE … ADD`, for `PRIMARY KEY` and `UNIQUE`, each named and unnamed. pglet's own
`UniqueConstraint` reaches `executor::ddl`'s `validate_key_columns` with no location.
The **SQLSTATE doesn't decide whether a `P` is sent**. Each `42701` route we checked alongside it sends no
`P` on either engine: a repeated column name at `CREATE TABLE` (`MergeAttributes`), and a
system column name at `CREATE TABLE` (`CheckAttributeNamesTypes`) and at `ADD COLUMN`
(`check_for_column_name_collision`). The foreign key referenced-list repeat's `42830`
(`transformFkeyCheckAttrs`) sends no `P` on either engine either.

An **unrecognized type name** and a **carrier's type-modifier refusal** are on the list
for a *different* reason. Both are raised inside `executor::type_resolution`'s
`resolve_type_name`, a resolver that takes a type *name* and no span, so none of its four
callers passes one down. The **name** refusal is `42704` at each of those callers:
`1::nosuchtype`, `nosuchtype 'x'`, `CREATE TABLE … (a nosuchtype)`,
`ADD COLUMN … nosuchtype`, `CAST(1 AS nosuchtype)`, and the `nosuchtype[]` spelling of
each except the typed constant, whose grammar takes no `[]` (`nosuchtype[] 'x'` is
`42601` on both engines). We checked that every one of those `42704`s has a caret on
18.6 and none in pglet. The **modifier** refusal comes under the `22023` itself
(`'ab'::char(0)`, `1.5::numeric(0)`) and under the `int4` coercion of a `numeric`
component before it (`22003` at `1.5::numeric(2147483648)`, `22P02` at
`1::numeric(1.5)`), and each of those likewise has a caret on 18.6 and none in pglet. The SQLSTATE doesn't decide it in
either direction. `opt_float`'s two refusals share the `22023` and do fill a `P`. And the
list's own `22P02` is the one a **value's** input function raises (`'x'::int`): the same
`Error` variant, but from converting a value rather than resolving a type name. Two of
the four callers are in the **parser** (`parser::expr`'s cast target and typed constant),
where a cursor is available and could be passed down if we ever wanted the `P`.

A minority of the fixtures that record a `42601` differ on *which token* the two parsers
stop at:

- 18.6 stops at the `*` in `count(ALL *)`, and pglet at the closing paren.
- 18.6 stops at the terminator in `SELECT is`, and pglet at the keyword.
- 18.6's lexer stops at the `ICONST` in a `timestamp(3.5)` / `float(1.5)` / `char(3.5)` /
  `varchar(3.5)` modifier (spellings of the windows `opt_precision` serves). pglet stops
  one token later at the `)`, because it consumed the number before validating it.

One more (`identifier/quoted_owned_by_none_loud`) differs because 18.6 sends no `P` at
all. Each carries a justified `-- @no-error-fields:`, so to rebuild the list, grep for
that directive across the fixtures that record a `42601`. A count here would go stale
without failing anything.

## Where the payloads live

`Error` is 72 bytes against a `MAX_ERROR_SIZE` of 80, and that budget sits below a
debug-stack cliff we measured, so inline `detail`/`hint` fields aren't possible. The
approach is to **derive what the variant already holds, and box only what it has to
add.**

- Derived, no new bytes: `23502`'s `s`/`t`/`c`, `23514`'s `s`/`t`/`n`, `2BP01`'s
  detail and hint, `42809`'s hint and detail, `428C9`'s detail and hint, and
  `0A000 cannot change ownership of identity sequence`'s detail.
- Boxed, because the variant needed more data: `FailingRow` on `NotNullViolation` and
  `CheckViolation`, `UniqueViolationDetail` on `UniqueViolation` and `DuplicateIndexKey`,
  `ForeignKeyViolationDetail` on `ForeignKeyViolation` and `RestrictViolation`, and
  `ForeignKeyTypeMismatch` on `ForeignKeyCannotBeImplemented`.

## The two value renderers

Both live in `pglet_core::error_fields`, and both render values raw, with no quoting or
escaping. So a text value containing `, ` looks the same as a field boundary. That's what
the oracle does, and pglet copies it rather than improving on it.

**`Failing row contains (…)`** comes from `ExecBuildSlotValueDescription` (`execMain.c`),
and is used by `23502` and the write-path `23514`:

- NULL renders as the bare word `null`, and an empty string renders as **nothing** between
  the separators (`(2, , null)`).
- Values go through the column type's output function under the **declared** type, so a
  `timestamptz` carries `+00` and a `numeric(4,2)` has its typmod applied.
- Each field is cut to **64 bytes** at a character boundary and then gets `...`.
  Exactly 64 bytes is **not** cut (`if (vallen <= maxfieldlen)`), but 65 is. 100 × `é`
  (200 bytes) renders as 32 × `é` + `...`.

**`Key (…)=(…)`** comes from `BuildIndexValueDescription` (`genam.c`), and is used by
`23505`, `23503`, `23001` and the index-build `23505`. Names and values are each joined
with `, ` in their own parens, NULL is `null` (you can reach that through
`UNIQUE NULLS NOT DISTINCT`), and it's **never truncated**, because that builder gets no
`maxfieldlen`.

## The not-null check is row-scoped

18.6 checks `NOT NULL` in `ExecConstraints`, *after* the tuple is formed, and reports the
whole tuple. pglet's `enforce_not_null_row` runs after assembly on every write path:

- directly in the INSERT loop and in both `UPDATE` collect passes (`executor/dml.rs`),
- through `enforce_row_checks` for `write_updated_row`,
- inside `enforce_row_constraints`, for the upsert `DO UPDATE` branch and `COPY`,
- over each row a foreign-key action rewrites, through `enforce_row_checks` for an
  `UPDATE`'s actions and `ActionState::rewrite` for a `DELETE`'s, including a `SET NULL`
  onto a `NOT NULL` column.

`validate_column` keeps only the enum check.

The order matches Postgres. A later column's input-function failure (`22P02`, `22001`, a
non-member enum label) wins over an earlier column's null, because 18.6 evaluates the
whole target list before it checks any constraints.

## One SQLSTATE, two errors

On 18.6, three SQLSTATEs can come from two different routes, with **different messages
and different field sets**. pglet gives each route its own variant, because the
write-path wording on the validation route would name a row the statement never wrote:

| code | write path (`ExecConstraints` / `_bt_check_unique` / `ri_ReportViolation`) | validation scan (`ATRewriteTable` / index build) |
| --- | --- | --- |
| `23502` | `null value in column "c" of relation "t" violates not-null constraint`, `D` = the failing row | `column "c" of relation "t" contains null values`, **no `D`** — `Error::NotNullColumnHasNulls` |
| `23514` | `new row for relation "t" violates check constraint "k"`, `D` = the failing row | `check constraint "k" of relation "t" is violated by some row`, **no `D`** — `Error::CheckConstraintNotSatisfied` |
| `23505` | `duplicate key value violates unique constraint "k"`, `D` = `Key (…)=(…) already exists.` | `could not create unique index "k"`, `D` = `Key (…)=(…) is **duplicated**.` — `Error::DuplicateIndexKey` |

`42809` splits the same way, but by **wording** rather than by route data. 18.6 has five
sentences for a relation of the wrong kind, and pglet uses three of them:

- `DropErrorMsgWrongType` (`Error::WrongObjectType`, `route: Drop`): `"x" is not a table`
  plus `Use DROP INDEX to remove an index.` in `H`, based on the kind the name *actually*
  refers to.
- `truncate_check_rel` (`route: Truncate`): the **same message**, and no hint at all.
  That's the only thing the route field decides, and it exists because the two differ.
- `validate_relation_kind` (`Error::WrongRelationKind`): `cannot open relation "x"` plus
  `This operation is not supported for indexes.` in `D`. This covers the scan / DML /
  `COPY` / `CREATE INDEX … ON` / `REFERENCES`-an-index route.
- `process_owned_by` (`Error::SequenceOwnerWrongKind`): its own message, the same detail.

The ones pglet doesn't use are `EDGES.md` row 77, and they don't differ in the same way:

- `referenced relation "x" is not a table` for a sequence FK parent, and `cannot change
  sequence "x"` for `INSERT`/`UPDATE`/`DELETE` on one. 18.6 sends **no `D`** on either,
  so pglet's `cannot open relation "x"` *plus* a detail differs in both fields. Its
  fixtures record that with a justified `-- @no-error-fields:`.
- `ALTER action ADD COLUMN cannot be performed on relation "x"` for `ALTER TABLE`. Here
  18.6 sends **the same `D`** pglet does, byte for byte, and only `M` differs.

The SQLSTATE agrees on every route. (A **non-superuser** gets
`42501 permission denied for sequence` for `INSERT`/`DELETE`, even the sequence's own
*owner*, because a sequence has no INSERT/DELETE ACL bit and `aclcheck_error` fires before
the relkind check. The harness connects as the superuser `postgres`, which bypasses the
ACL, so this corpus compares against `42809`. We checked both ways.)

## `2BP01` drops pglet's quoting

The dependency errors are the one place pglet leaves identifiers **bare**, because
`reportDependentObjects` does, and all three of its strings are compared against the
oracle byte for byte: `cannot drop table p_dep because other objects depend on it` /
`constraint c_dep_a_fkey on table c_dep depends on table p_dep` / `Use DROP ... CASCADE
to drop the dependent objects too.` `DropTargetName::unquoted` is the only renderer for them.

The one deliberate divergence here is `DROP INDEX` over a constraint's index. 18.6's
hint suggests `ALTER TABLE … DROP CONSTRAINT`, which pglet doesn't implement (it's
refused with `0A000`), so pointing a reader at it would be worse than diverging. Its
fixtures carry the waiver.

## How the harness sees it

The differential harness compares the structured fields **live**, pglet against Postgres,
in the same run. They're never written to `.expected.json`, the same as the declared
column types.

Scoring is deliberately asymmetric:

- If pglet **omits** a field 18.6 sends, that counts as agreement. Omitting is the
  intended behavior, and failing on it would make the correct answer look the same as a
  wrong one.
- If pglet sends a field 18.6 omits, or sends it differently, that's a **failure**. That
  kind of invention is what this comparison is there to catch.

That leaves two blind spots. First, **the harness can't see pglet dropping a field**,
because an omission scores as agreement. Second, the comparison only runs in the
`(pglet Error, recorded Error)` arm. On the **pglet-stricter** fixtures (an
`@expect-error` fixture whose recorded `.expected.json` holds a success shape, where
Postgres *accepts* the statement and any pglet error passes) there's no oracle field set
to compare against, so an **invention** there is invisible too. Both gaps are covered by
other tests: unit tests on the builder (`pglet_core::error_fields`), executor-level tests
that check the payload is actually filled (`pglet_sql`'s `engine_constraints`), raw-field
tests for each send site (`pglet_server`'s `encoding::tests`), and the wasm property-name
tests (`tests-web/npm/dx_test.ts`).

Two directives handle the rest (`tests/README.md` has the grammar):

- `-- @no-error-fields: <why>` waives the comparison for a justified divergence. Like the
  other opt-outs, it needs a justification.
- `-- @expect-message` locks the primary message to Postgres's, byte for byte. It's
  opt-in rather than on by default because `M` differs on a long list tracked separately.
  Turning it on by default would fail all of those at once, and the directive would end
  up as a waiver on half the corpus. As opt-in, it works as a **lock** instead.

## Known gaps

`EDGES.md` rows 76 and 77 have the list we've measured. In short:

- `P` on a SQLSTATE that `Error::position` has no arm for: everything outside the
  token-bearing `42601`s and `opt_float`'s two `22023`s, per § `P` counts characters
  above.
- The `D` on a non-`CASCADE` `DROP SCHEMA public` over a non-empty schema. 18.6 lists
  each dependent object on its own line (`table t depends on schema public\n…`) and pglet
  sends none. pglet raises `DependentObjectsExist` from a check that the schema is empty
  rather than from a list of dependents, so there's no list to render.
- The near-name `H` on `42703`, the no-match `H` on `42883`, and the escape `H` on a
  malformed Unicode escape's `22025`.
- The `D`+`W` pair on a jsonb `22P02`, and the `D` and `W` on a jsonb `22P05`.
- The `W` on COPY's errors and on a bound parameter's errors.
- The `D` (and 18.6's `numeric field overflow` message) on a `numeric` `22003`.
- 18.6's `character(n)` spelling on a `char(n)` `22001`.
- The `D` on a `CREATE TYPE … AS ENUM` **repeated label**. The refusal is an
  `Error::UniqueViolation` with `key: None`, so no detail payload is built at all. The
  payload that would carry the key description also carries the relation, which brings
  `s` along with it, and its value here is the single `IMPLICIT_SCHEMA` (`docs/enums.md`
  § An enum's labels are a set).
- The `WARNING`/`25P01` `NoticeResponse` 18.6 sends for `COMMIT`/`ROLLBACK` outside a
  transaction block. pglet's notice channel carries only a message string, with no
  severity or SQLSTATE of its own, so fixing that means changing the notice type rather
  than this builder.
