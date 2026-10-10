# Errors on the wire — the structured `ErrorResponse` fields

An error is not a string. Postgres's `send_message_to_frontend` (`elog.c`) writes it as
a sequence of typed fields — severity (`S`, and the unlocalized `V`), SQLSTATE (`C`),
primary message (`M`), then whichever of detail (`D`), hint (`H`), context (`W`),
schema/table/column/constraint (`s`/`t`/`c`/`n`) and statement position (`P`) the error
carries, and finally the C source location (`F`/`L`/`R`). Drivers surface those by name:
`pg`'s `pg-protocol` maps them onto `DatabaseError.detail` / `.hint` / `.position` /
`.schema` / `.table` / `.column` / `.constraint`, tokio-postgres exposes all fifteen, and
pglite exposes `pg`'s shape. **A field pglet does not send is a field the consumer sees
as `undefined`.**

## The rule

> **A client that reads a structured error field gets what 18.6 sends, and a field pglet
> cannot fill truthfully is omitted rather than invented.**

Two halves, and both are load-bearing.

### One builder

Every field set is assembled in `pglet_core::error_fields`: `Error::wire_fields` for an
error pglet raised, and `plain_wire_fields` (the same `S`/`V`/`C`/`M` prefix and nothing
after it) for a message no `Error` stands behind. Each native-server message goes through
one of them, and the wasm host reads the accessors `wire_fields` is built from:

- the native server's `extended_error` (and `error_response`, which wraps it) takes
  `wire_fields`,
- `error_with_code` / `fatal_with_code`, the raw-code sites for wire-framing errors, and
  `notice_response` take `plain_wire_fields`,
- `pglet_wasm`'s `pglet_error_to_js` reads `detail()` / `hint()` /
  `object_location()` / `position()` — the same accessors `wire_fields` reads — and sets
  `pg`'s property names.

A client comparing responses can tell a response that did not come from Postgres by a
missing `V` alone, so `message_fields`, the prefix both builders share, always writes it.

### Omit rather than invent

`F`/`L`/`R` are the clearest case: pglet has no C source tree, so any value it put there
would be a claim about a file, a line and a function that do not exist. They are never
sent. The same reasoning decides the harder cases:

- A `NOT NULL` violation raised **mid-assembly** has no whole row to describe, so it
  sends no `D` — so the check runs on the formed row (below).
- A `23505` on an **expression** index has no derivable key-column name, and no declared
  type to render its value under, so the `Key (…)=(…)` **`D`** is dropped — `s`, `t` and
  `n` still go out, because the relation is truthful whatever the key looks like. The two
  facts are separated in the payload (`UniqueViolationDetail { table, key: Option<…> }`)
  so that dropping one cannot drop the other.
- A `23505` is never raised over a key only another connection's *uncommitted* write
  holds: the second writer waits for that block and is answered against what it
  committed (`EDGES.md` row 98) — on the native server; the embedded `Database`, which
  cannot wait, refuses it `55P03` (row 229) — so its `DETAIL: Key (a)=(2) already
  exists.` names a committed row, as 18.6's does.

And the inverse is equally a rule: **pglet holding a name is not a licence to send it.**
18.6's `42703` sends no `t`/`c` on any of its four routes, `428C9` sends a detail and a
hint but no `s`/`t`/`c`, and `2BP01`, `42809`, `42P07`, `42710`, `42701`, `42803`,
`42830`, a foreign key's `42804` (with its detail), `42P10`, `42P16`, `22001`, `22003`,
`22012`, `22P02` and `25P02` send none of the four — all measured on 18.6 with a raw v3
client, the `42804` with `psql`'s verbose field display. pglet's `Error` variants carry
table and column names at several of those; `Error::object_location` sends them at
18.6's `errtable*` positions only — its match arms are the enumeration
(`NotNullViolation`/`NotNullColumnHasNulls`, `CheckViolation`/`CheckConstraintNotSatisfied`,
`DuplicateIndexKey`/`UniqueViolation`, `ForeignKeyViolation`, `RestrictViolation`, and
`BtreeIndexRowTooLarge`, whose `n` is the **index**'s name, as
`_bt_check_third_page`'s `errtableconstraint(heap, index)` sends it). The two other
index-size `54000`s — GIN's item limit and `index_form_tuple`'s size field — send none
of the four, and nor does pglet's own page-bound refusal (all measured on 18.6 where
18.6 raises them). The B-tree refusal's `D` names the heap tuple the entry references
(`Index row references tuple (0,2) in relation "t".`), a `ctid` pglet does not have,
so it is omitted.

## What decides a field

| field | rule |
| --- | --- |
| `S` / `V` | the sending host's severity, both slots, always. Identical on an unlocalized server, which pglet is. |
| `C` | `Error::sqlstate()`, unchanged. |
| `M` | the variant's `Display`. |
| `D` | `Error::detail()` — per variant, `Option`. |
| `H` | `Error::hint()` — per variant, `Option`. |
| `s` | the constant `IMPLICIT_SCHEMA` (`public`), emitted **iff `t` is**. pglet models one schema; 18.6 fills `s` and `t` through one `errtable*` macro and never sends `s` alone. |
| `t` / `c` / `n` | `Error::object_location()`. |
| `P` | `Error::position()` — a **1-based Unicode codepoint** index (below). |
| `W`, `p`, `q`, `d`, `F`, `L`, `R` | never sent. |

Field **order** is 18.6's, from `send_message_to_frontend`: `S V C M D H W s t c d n P`.

## `P` counts characters

18.6's `P` is a 1-based **Unicode codepoint** index into the statement text — not bytes
and not UTF-16 units. Measured: `SELECT 'ααααα' AS a, FROM t` reports `P=22` where the
byte offset is 27, and `SELECT '👍👍👍' AS a, FROM t` reports `P=20` (one per emoji, not
two units and not four bytes). `pglet_core::codepoint_position` is the one conversion,
and every `P` pglet sends is one of its results — the two `Error` constructors that take
a position (`syntax_at`, `syntax_sentence_at`) and `parser::type_name`'s two
`FloatPrecision*` variants are fed by nothing else. `parser/stream.rs` holds three
callers: `TokenStream::codepoint_pos` (the current token), `TokenStream::token_position`
(a token already **consumed** — where `opt_float`'s `22023` points, since the number is
read before it can be judged), and `lex_error_to_error`, which a lexical failure reaches
from `parser::stream`'s `pull`, `parser::split_statements` and the session-command readers.
Those readers (`catalog::commands`) also call it directly, for their
`at or near "<tok>"` arms.

**Scope.** `P` is filled by the `42601`s that reach `TokenStream` (every parse error the
statement grammar raises), by a lexical failure raised from `split_statements` ahead of
any `TokenStream`, by the session-command readers' `at or near "<tok>"` arms,
which have a real token offset, and by the one non-`42601` that fills it at all
(`Error::position`'s arms are the enumeration) — a `FLOAT(n)` outside 1–53, whose `22023`
points at the number as `parser_errposition(@2)` does (`P=34` for `SELECT 0.1234567890123456::float(0)`, `P=26` for
`CREATE TABLE zq (a float(0))`, `P=30` for `CREATE SEQUENCE zsq AS FLOAT(0)`, each
measured on the raw wire against 18.6). It is **not** filled by the `Error::syntax` producers
that carry no token — measured on 18.6, `COPY pv FROM STDIN (FORMAT bogus)` reports
`P=21`, `SELECT a FROM pv UNION SELECT a,b FROM pv` `P=31`, `CREATE TABLE pw (a INT
DEFAULT DEFAULT)` `P=32`, `SELECT * FROM a.b.c.d` `P=15`, and pglet sends none — nor by
the readers' `syntax error at end of input` arm, where 18.6 counts one past the **raw**
statement's last character and `split_statements` has already trimmed the slice the reader
sees (`'DEALLOCATE '` is `P=12` there against a trimmed length of 11 here). The
set of routes measured so far where 18.6 fills a `P` and pglet sends none is **open and
carries no total** — it grows by measurement, and `EDGES.md` row 76 enumerates it.
Most would need a span threaded through `Plan`/`Expr`, which carry none: `42P01`, `42703`,
`42883`, `22P02`, `42702`, `42804`, `42803`, `42P02`, and
`0A000 cannot use subquery in check constraint`, each measured. A `PRIMARY KEY`/`UNIQUE`
**key list naming a column twice** joins them on that same reason: 18.6's `P` is the
`Constraint` node's location (`transformIndexConstraint`'s
`parser_errposition(cxt->pstate, constraint->location)`) — the `CONSTRAINT` keyword where
one is declared and the `PRIMARY`/`UNIQUE` keyword otherwise (`P=24` at
`CREATE TABLE t (a int, PRIMARY KEY (a, a))`, `P=20` at
`ALTER TABLE tt ADD CONSTRAINT c UNIQUE (a, a)`), measured at `CREATE TABLE` and
`ALTER TABLE … ADD`, `PRIMARY KEY` and `UNIQUE`, each named and unnamed — and pglet's own
`UniqueConstraint` reaches `executor::ddl`'s `validate_key_columns` carrying no location.
Its **SQLSTATE is not the predicate**: each `42701` route measured beside it sends no `P`
on either engine — a repeated column name at `CREATE TABLE` (`MergeAttributes`), a system
column name at `CREATE TABLE` (`CheckAttributeNamesTypes`) and at `ADD COLUMN`
(`check_for_column_name_collision`) — as does the foreign key referenced-list repeat's
`42830` (`transformFkeyCheckAttrs`). An **unrecognized type name** and a **carrier's
type-modifier refusal** sit in the list with a *different* reason: both are raised inside
`executor::type_resolution`'s `resolve_type_name`, a resolver that takes a type *name* and
no span, so none of its four callers hands one down. The **name** refusal is `42704` at
each of those callers — `1::nosuchtype`, `nosuchtype 'x'`,
`CREATE TABLE … (a nosuchtype)`, `ADD COLUMN … nosuchtype`, `CAST(1 AS nosuchtype)`, and
the `nosuchtype[]` spelling of each but the typed constant, whose grammar takes no `[]`
(`nosuchtype[] 'x'` is `42601` on both engines) — every one of those `42704`s under a
caret on 18.6 and none here, measured. The **modifier** refusal arrives under the `22023`
itself (`'ab'::char(0)`, `1.5::numeric(0)`) and under the `int4` coercion of a `numeric`
component ahead of it — `22003` at `1.5::numeric(2147483648)`, `22P02` at
`1::numeric(1.5)` — each of those likewise. The SQLSTATE is the predicate in neither
direction: `opt_float`'s two refusals share the `22023` and do fill a `P`, and the list's
own `22P02` is the one a **value's** input function raises (`'x'::int`) — the same `Error`
variant, from a value's conversion rather than from a type name's resolution. Two of the
four callers are the **parser** (`parser::expr`'s cast target and typed constant), where a
cursor is in hand and could thread one if the `P` were ever wanted.
A minority of the fixtures recording a `42601` diverge on *which token* the two parsers
stop at (18.6 at the `*` in `count(ALL *)`, pglet at the closing paren; 18.6 at the
terminator in `SELECT is`, pglet at the keyword; 18.6's lexer at the `ICONST` in a
`timestamp(3.5)` / `float(1.5)` / `char(3.5)` / `varchar(3.5)` modifier — spellings of the
windows `opt_precision` serves — pglet one token later at the `)`, having consumed the
number before validating it), and one (`identifier/quoted_owned_by_none_loud`) diverges
because 18.6 sends no `P` at all. Each carries a justified `-- @no-error-fields:`, so
grepping that directive over the `42601`-recording fixtures is how to re-derive the
list — a count here would rot without reddening anything.

## Where the payloads live

`Error` is 72 bytes against a `MAX_ERROR_SIZE` of 80, and that budget sits below a
measured debug-stack cliff — inline `detail`/`hint` fields are impossible. So the rule
is: **derive what the variant already holds, and box only what it must gain.**

- Derived, zero new bytes: `23502`'s `s`/`t`/`c`, `23514`'s `s`/`t`/`n`, `2BP01`'s
  detail and hint, `42809`'s hint and detail, `428C9`'s detail and hint,
  `0A000 cannot change ownership of identity sequence`'s detail.
- Boxed, because the variant had to gain data: `FailingRow` on `NotNullViolation` and
  `CheckViolation`, `UniqueViolationDetail` on `UniqueViolation` and `DuplicateIndexKey`,
  `ForeignKeyViolationDetail` on `ForeignKeyViolation` and `RestrictViolation`,
  `ForeignKeyTypeMismatch` on `ForeignKeyCannotBeImplemented`.

## The two value renderers

Both live in `pglet_core::error_fields` and both render raw — no quoting, no escaping —
so a text value containing `, ` is indistinguishable from a field boundary. That is the
oracle's behaviour, reproduced rather than improved on.

**`Failing row contains (…)`** — `ExecBuildSlotValueDescription` (`execMain.c`), used by
`23502` and the write-path `23514`:

- NULL renders as the bare word `null`; an empty string renders as **nothing** between
  the separators (`(2, , null)`).
- Values are the column type's output function under the **declared** type, so a
  `timestamptz` carries `+00` and a `numeric(4,2)` is typmod-applied.
- Each field is clipped to **64 bytes** at a character boundary and then gets `...`.
  Exactly 64 bytes is **not** ellipsized (`if (vallen <= maxfieldlen)`); 65 is. 100 × `é`
  (200 bytes) renders as 32 × `é` + `...`.

**`Key (…)=(…)`** — `BuildIndexValueDescription` (`genam.c`), used by `23505`, `23503`,
`23001` and the index-build `23505`: names and values each `, `-joined in their own
parens, NULL as `null` (reachable through `UNIQUE NULLS NOT DISTINCT`), and **never
truncated** — that builder is passed no `maxfieldlen`.

## The not-null check is row-scoped

18.6 checks `NOT NULL` in `ExecConstraints`, *after* the tuple is formed, and reports the
whole tuple. `enforce_not_null_row` runs after assembly at every write path — explicitly in the
INSERT loop and in both `UPDATE` collect passes (`executor/dml.rs`), through
`enforce_row_checks` for `write_updated_row` and, inside `enforce_row_constraints`, the upsert
`DO UPDATE` branch and `COPY`, and over each row a foreign-key action rewrites (through
`enforce_row_checks` for an `UPDATE`'s actions and `ActionState::rewrite` for a
`DELETE`'s, a `SET NULL` onto a `NOT NULL` column included) — and
`validate_column` keeps the enum check alone.

The order is Postgres's: a later column's input-function failure — `22P02`,
`22001`, a non-member enum label — outranks an earlier column's null, because 18.6
evaluates the whole target list before it constrains anything.

## One SQLSTATE, two errors

Three SQLSTATEs are reached by two different routes on 18.6, with **different messages
and different field sets**, and pglet answers each route with its own variant — the
write-path wording on the validation route would name a row the statement never wrote:

| code | write path (`ExecConstraints` / `_bt_check_unique` / `ri_ReportViolation`) | validation scan (`ATRewriteTable` / index build) |
| --- | --- | --- |
| `23502` | `null value in column "c" of relation "t" violates not-null constraint`, `D` = the failing row | `column "c" of relation "t" contains null values`, **no `D`** — `Error::NotNullColumnHasNulls` |
| `23514` | `new row for relation "t" violates check constraint "k"`, `D` = the failing row | `check constraint "k" of relation "t" is violated by some row`, **no `D`** — `Error::CheckConstraintNotSatisfied` |
| `23505` | `duplicate key value violates unique constraint "k"`, `D` = `Key (…)=(…) already exists.` | `could not create unique index "k"`, `D` = `Key (…)=(…) is **duplicated**.` — `Error::DuplicateIndexKey` |

`42809` splits the same way, by **wording** rather than by route data — 18.6 has five
sentences for a wrong-kind relation and pglet spells three of them:

- `DropErrorMsgWrongType` (`Error::WrongObjectType`, `route: Drop`): `"x" is not a table`
  plus `Use DROP INDEX to remove an index.` in `H`, keyed on the kind the name *actually*
  holds.
- `truncate_check_rel` (`route: Truncate`): the **same message**, and no hint at all.
  That is the only thing the route field decides, and it decides it because the two
  differ.
- `validate_relation_kind` (`Error::WrongRelationKind`): `cannot open relation "x"` plus
  `This operation is not supported for indexes.` in `D` — the scan / DML / `COPY` /
  `CREATE INDEX … ON` / `REFERENCES`-an-index route.
- `process_owned_by` (`Error::SequenceOwnerWrongKind`): its own message, the same detail.

The ones it does **not** spell are `EDGES.md` row 77, and they do not diverge alike:

- `referenced relation "x" is not a table` for a sequence FK parent, and `cannot change
  sequence "x"` for `INSERT`/`UPDATE`/`DELETE` on one — 18.6 sends **no `D`** on either,
  so pglet's `cannot open relation "x"` *plus* a detail diverges in both fields. Its
  fixtures record it with a justified `-- @no-error-fields:`.
- `ALTER action ADD COLUMN cannot be performed on relation "x"` for `ALTER TABLE` — here
  18.6 sends **the same `D`** pglet does, byte for byte, and only `M` diverges.

The SQLSTATE agrees on every route. (`42501 permission denied for sequence` is what a
**non-superuser** gets for `INSERT`/`DELETE` — the sequence's own *owner* included, since
a sequence carries no INSERT/DELETE ACL bit and `aclcheck_error` fires before the relkind
check. The harness connects as the superuser `postgres`, which bypasses the ACL, so
`42809` is what this corpus compares against — measured both ways.)

## `2BP01` drops pglet's quoting

The dependency family is the one place pglet leaves identifiers **bare**, because
`reportDependentObjects` does and all three of its strings are compared against the
oracle byte for byte: `cannot drop table p_dep because other objects depend on it` /
`constraint c_dep_a_fkey on table c_dep depends on table p_dep` / `Use DROP ... CASCADE
to drop the dependent objects too.` `DropTargetName::unquoted` is the one renderer.

The single deliberate divergence in the family is `DROP INDEX` over a constraint's index:
18.6's hint suggests `ALTER TABLE … DROP CONSTRAINT`, which pglet does not implement (it is refused `0A000`), so
pointing a reader at it would be worse than diverging. Its fixtures carry the waiver.

## How the harness sees it

The differential harness compares the structured fields **live**, pglet against Postgres,
in the same run — they are never written to `.expected.json`, exactly as the declared
column types are not.

The scoring rule is asymmetric, and the asymmetry is the design:

- pglet **omits** a field 18.6 sends → agreement. Omission is the declared posture, and
  scoring it red would make the honest answer indistinguishable from a wrong one.
- pglet sends a field 18.6 omits, or sends it differently → **failure**. That is the
  invention this comparison exists to catch.

Its blind spot follows directly, in **two** directions. The first: **the harness cannot
see pglet dropping a field**, because an omission scores as agreement. The second: the
comparison runs only in the `(pglet Error, recorded Error)` arm, so on the
**pglet-stricter** fixtures — an `@expect-error` fixture whose recorded `.expected.json` holds a success shape, where Postgres *accepts* the statement and any pglet error
passes — there is no oracle field set to compare at all, and an **invention** on one of
those is invisible too. Both halves are guarded by unit tests on the builder (`pglet_core::error_fields`), executor-level
tests that assert the payload is actually filled (`pglet_sql`'s `engine_constraints`),
raw-field tests per send site (`pglet_server`'s `encoding::tests`), and the wasm
property-name tests (`tests-web/npm/dx_test.ts`).

Two directives carry the rest (`tests/README.md` has the grammar):

- `-- @no-error-fields: <why>` waives the comparison for a justified divergence.
  Justification required, like the other opt-outs.
- `-- @expect-message` locks the primary message to Postgres's, byte for byte. Opt-in
  rather than default-on: `M` diverges on a long, separately-tracked list, so a
  default-on comparison would redden them all at once and the directive would become a
  waiver worn by half the corpus. Opt-in makes it a **lock** instead.

## Known gaps

`EDGES.md` rows 76 and 77 carry the measured list. In short: `P` on a SQLSTATE
`Error::position` has no arm for — everything outside the token-bearing `42601`s and
`opt_float`'s two `22023`s, per § `P` counts characters above; the
`D` on a non-`CASCADE` `DROP SCHEMA public` over a non-empty schema, where 18.6 lists
every dependent object on its own line (`table t depends on schema public\n…`) and pglet
sends none — it raises `DependentObjectsExist` from a schema-emptiness test rather than
from an enumeration, so there is no list to render; the
near-name `H` on `42703`, the no-match `H` on `42883`, and the escape `H` on a
malformed Unicode escape's `22025`; the
`D`+`W` pair on a jsonb `22P02`, and the `D` and `W` on a jsonb `22P05`; the `W` on COPY's
and a bound parameter's errors; the `D` (and 18.6's
`numeric field overflow` message) on a `numeric` `22003`; 18.6's `character(n)` spelling
on a `char(n)` `22001`; the `D` on a `CREATE TYPE … AS ENUM` **repeated label**, where the
refusal is an `Error::UniqueViolation` with `key: None` — no detail payload is built at
all, and the payload that would carry the key description carries the relation with it,
which drags `s`, whose value here is the single `IMPLICIT_SCHEMA` (`docs/enums.md`
§ An enum's labels are a set); and the `WARNING`/`25P01` `NoticeResponse` 18.6 emits for
`COMMIT`/`ROLLBACK` outside a transaction block — pglet's notice channel carries a
message string only, with no severity or SQLSTATE of its own, so that one is a change to
the notice type rather than to this builder.
