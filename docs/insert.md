# INSERT cluster — named-column binding + DEFAULTs + SERIAL

`INSERT` binds values to columns by name, resolves each omitted column's
`DEFAULT`, and drives `SERIAL`/`BIGSERIAL` and identity sequences. The
differential fixtures under `tests/fixtures/insert/` are the guard; § Gaps
records this cluster's divergences, and `EDGES.md` carries the rest, including row 40
(a failed autocommit DML's partial effects stand on `MemoryStorage`). (Enums: [enums.md](enums.md); arrays: [arrays.md](arrays.md).)

## Behavior

- INSERT binds values to columns **by name** (honoring the column list); omitted
  columns take their `DEFAULT` — never positional binding.
- `INSERT … DEFAULT VALUES` is the row-less source: every column takes its own
  default, or SQL NULL where it has none (an all-NULL row is a success, not `23502`;
  a `NOT NULL` column with no default is the `23502`). It parses to **one empty
  `VALUES` tuple** and rides the same table-width widening an omitted column already
  took, so it lowers to the identical plan its `VALUES (DEFAULT, …)` twin does —
  `ON CONFLICT`, `RETURNING`, correlated `RETURNING` subqueries, `SERIAL` and
  identity columns all behave as they do there, and expression defaults evaluate
  per row (`DEFAULT nextval('s') + 10` over two inserts gives 11, 12). The
  alternative carries **no** column list and **no** `OVERRIDING` — `gram.y`'s
  `insert_rest` gives it neither — so both pairings are `42601` at the `DEFAULT`
  token.
- Catalog: `Column.default` (`ColumnDefault` — its recognized closed forms plus an
  `Expr` catch-all) + `Schema.sequences`.
- `DEFAULT`s resolve: literals — including a **signed** numeric one, `DEFAULT -7` /
  `DEFAULT +1.5`, with the sign folded into the stored value — the five `now`-family
  spellings (`now()`,
  `transaction_timestamp()`, `statement_timestamp()`, `clock_timestamp()`,
  `CURRENT_TIMESTAMP`), `CURRENT_DATE`, `CURRENT_TIME`, `gen_random_uuid()`
  (`uuid_generate_v4()` accepted as an alias), `SERIAL`/`BIGSERIAL` (implicit
  `<table>_<col>_seq`), and **any other expression**, stored unevaluated on the
  `ColumnDefault::Expr` arm and evaluated per row — `DEFAULT 1+1` keeps its operator
  node and `DEFAULT nextval('s') + 1` draws a fresh value on every insert, as on 18.6.
- `now()` / `gen_random_uuid()` are ordinary scalar functions, not only `DEFAULT`
  forms — `SELECT NOW()`, `WHERE t <= NOW()` and `VALUES (…, NOW())` all resolve.
- A `timestamp` value satisfies a `timestamptz` column (`coerce_value`).

## Gaps

- **Not a gap, but the sharpest edge in this cluster:** in `ON CONFLICT DO UPDATE`, a
  bare column in the `SET` right-hand side (or the `WHERE`) is **ambiguous** — the
  existing row and the proposed `EXCLUDED` row are both in scope — so the natural
  increment idiom `SET n = n + 1` fails `42702` and must be spelled `SET n = t.n + 1`.
  pglet **matches Postgres** here, error code included; it is listed because the idiom
  is common enough that the rejection reads like an engine limitation rather than the
  conformance it is. See the ON CONFLICT row in [compatibility.md](compatibility.md).
- **Another connection's pending write, closed.** An `INSERT` whose key another
  connection's open block has written waits for that block, as 18.6 waits, and then
  answers against what it committed or took back — the `ON CONFLICT` actions included,
  and over a pending **delete** the insert lands. Past the session's `lock_timeout` the
  wait answers `55P03`; the embedded `Database`, which cannot wait, refuses it `55P03` at
  once (`EDGES.md` row 229). [storage.md](storage.md) § Isolation Guarantees is the
  statement (`EDGES.md` rows 98, 101).
- **A duplicate target column, closed.** `INSERT INTO t (a, a) VALUES (1, 2)` is
  `42701 column "a" specified more than once`, as on Postgres. The column list is a set, on the
  `VALUES` route and the `INSERT … SELECT` one, resolved at the boundaries
  `checkInsertTargets` sets — ahead of the arity check, the cells' conversions and the
  identity guard — and interleaved per item, so a repeat outranks a *later* unknown name
  and yields to an earlier one. One boundary it does **not** inherit is the
  `INSERT … SELECT` **source query**, which 18.6 transforms after the list and pglet
  lowers before it. The `COPY` and `SET`-list halves of the same invariant, and
  the residuals it leaves, are [names.md](names.md) § A statement's target column list
  is a set.
- **Row lengths, closed.** A `VALUES` list whose rows differ in length, and a row longer or
  shorter than its target list, are 18.6's `42601` — `VALUES lists must all be the same
  length`, `INSERT has more
  expressions than target columns`, `INSERT has more target columns than expressions` — and
  an earlier row's error outranks a later row's length, as `transformInsertStmt` lowers the
  rows. Save one class: an error 18.6 raises in a cell's or an `INSERT … SELECT` source's
  parse analysis, and pglet only after the length comparison — in any row up to and
  including the refused one, a one-row `INSERT` included — answers the `42601` here
  (`VALUES (1, 2), (1 + '1x')` is `22P02` there). `EDGES.md` rows 225 and 226 carry the
  measurements and that loud residual.
- **An `ON CONFLICT DO UPDATE` meeting its own row, closed.** A statement whose rows propose
  one key twice is `21000 ON CONFLICT DO UPDATE command cannot affect row a second time`,
  by the row's identity and
  ahead of the action's `WHERE` (`EDGES.md` row 247).
- **Assignment casts, closed.** Postgres coerces every `INSERT` cell, `INSERT … SELECT`
  source column, `UPDATE … SET` and `ON CONFLICT … DO UPDATE SET` right-hand side with
  `COERCION_ASSIGNMENT` and refuses `42804` when no such cast exists — a **wider** class
  than `pg_cast`'s explicit-only rows, since a pair with no `pg_cast` entry at all is
  refused too. So `INSERT INTO t (b) VALUES (1)` over `t(b BOOL)` is `42804` while
  `SELECT 1::bool` still answers `t`, and `text → integer` is refused beside it. The
  gate is a plan-time one at all four sites, and its accepted set is four rules — the same type, an implicit/assignment
  `pg_cast` row, I/O coercion into a string-category *target* (so every type assigns
  into `text`/`varchar` and the reverse never does), and element-wise for two arrays.
  A **bare** string constant is Postgres's `unknown` and is exempt, so
  `INSERT INTO t (b) VALUES ('t')` stays ordinary SQL — but only while it is bare: a
  cast, a `CASE`, a `COALESCE`, an `ARRAY[…]` or an `ORDER BY` naming it all resolve it
  to `text` first, on both engines. Residuals are recorded in `EDGES.md`.

## Related capabilities

- **explicit sequences.** `CREATE SEQUENCE` is supported, alongside the implicit SERIAL
  sequence and `DEFAULT nextval('name')` extraction. A sequence is a **catalog record**
  (`{last_value, start, owner}`), not a bare counter: `start` backs
  `ALTER SEQUENCE … RESTART`, and `owner` backs the DROP-dependency AUTO edge and
  `TRUNCATE … RESTART IDENTITY`. `ALTER SEQUENCE` and `DROP SEQUENCE` parse and act.
  `nextval`/`currval`/`setval` are also
  callable as ordinary expressions (a top-level `SELECT nextval('s')` / `INSERT … VALUES
  (nextval('s'))`), with `setval` advancing and persisting the counter — see the Sequences row in [compatibility.md](compatibility.md).
- **identity columns.** `GENERATED {ALWAYS|BY DEFAULT} AS IDENTITY` reuses the SERIAL
  machinery (`Column.identity` + a `NextVal` default, implicit NOT NULL); `ALWAYS`
  rejects an explicit non-`DEFAULT` value (`428C9`) unless `OVERRIDING SYSTEM VALUE`,
  and `OVERRIDING USER VALUE` forces the sequence value — see the Sequences row in
  [compatibility.md](compatibility.md).
- **constraint enforcement** — NOT NULL (`23502`), CHECK (`23514`), PK/UNIQUE
  (`23505`, via auto-created enforced unique indexes — single-column, composite,
  partial, expression, `NULLS NOT DISTINCT`) and FK (`23503` — single **and
  composite**, with both `ON DELETE` and `ON UPDATE` actions) are enforced across
  the write paths. Separate from this cluster (see the PK / UNIQUE / FK row in
  [compatibility.md](compatibility.md)).
