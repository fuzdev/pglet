# INSERT cluster — named-column binding + DEFAULTs + SERIAL

`INSERT` binds values to columns by name, resolves the `DEFAULT` of each column you
leave out, and drives `SERIAL`/`BIGSERIAL` and identity sequences. The differential
fixtures under `tests/fixtures/insert/` guard it. § Gaps records this cluster's
divergences, and `EDGES.md` has the rest, including row 40 (on `MemoryStorage`, a failed
autocommit DML's partial effects stand). (Enums: [enums.md](enums.md); arrays:
[arrays.md](arrays.md).)

## Behavior

- INSERT binds values to columns **by name**, following the column list. Omitted columns
  take their `DEFAULT`. Binding is never positional.
- `INSERT … DEFAULT VALUES` is the form with no row source. Every column takes its own
  default, or SQL NULL if it has none. An all-NULL row succeeds; only a `NOT NULL` column
  with no default gives `23502`. It parses to **one empty `VALUES` tuple** and goes
  through the same widening to the table's width that an omitted column already gets, so
  it lowers to the same plan as its `VALUES (DEFAULT, …)` twin. `ON CONFLICT`,
  `RETURNING`, correlated `RETURNING` subqueries, `SERIAL` and identity columns all behave
  as they do there, and expression defaults evaluate per row (`DEFAULT nextval('s') + 10`
  over two inserts gives 11, 12). This form takes **no** column list and **no**
  `OVERRIDING`, because `gram.y`'s `insert_rest` allows neither, so both combinations are
  `42601` at the `DEFAULT` token.
- Catalog: `Column.default` (`ColumnDefault`, with its recognized closed forms plus an
  `Expr` catch-all) and `Schema.sequences`.
- These `DEFAULT`s resolve:
  - literals, including a **signed** numeric one (`DEFAULT -7` / `DEFAULT +1.5`), with
    the sign folded into the stored value
  - the five `now`-family spellings (`now()`, `transaction_timestamp()`,
    `statement_timestamp()`, `clock_timestamp()`, `CURRENT_TIMESTAMP`), `CURRENT_DATE`
    and `CURRENT_TIME`
  - `gen_random_uuid()` (with `uuid_generate_v4()` accepted as an alias)
  - `SERIAL`/`BIGSERIAL` (with an implicit `<table>_<col>_seq`)
  - **any other expression**, stored unevaluated on the `ColumnDefault::Expr` arm and
    evaluated per row. `DEFAULT 1+1` keeps its operator node, and
    `DEFAULT nextval('s') + 1` draws a fresh value on every insert, as on 18.6.
- `now()` and `gen_random_uuid()` are ordinary scalar functions, not just `DEFAULT`
  forms. `SELECT NOW()`, `WHERE t <= NOW()` and `VALUES (…, NOW())` all resolve.
- A `timestamp` value is accepted by a `timestamptz` column (`coerce_value`).

## Gaps

- **Not a gap, but the sharpest edge in this cluster.** In `ON CONFLICT DO UPDATE`, a
  bare column in the `SET` right-hand side (or the `WHERE`) is **ambiguous**, because the
  existing row and the proposed `EXCLUDED` row are both in scope. So the common increment
  idiom `SET n = n + 1` fails with `42702`, and you have to write `SET n = t.n + 1`.
  pglet **matches Postgres** here, error code included. It's listed because the idiom is
  common enough that the error looks like an engine limitation when it's correct
  behavior. See the ON CONFLICT row in [compatibility.md](compatibility.md).
- **Another connection's pending write, closed.** If another connection's open block has
  written an `INSERT`'s key, the `INSERT` waits for that block, as 18.6 does. It then
  runs against whatever that block committed or rolled back, `ON CONFLICT` actions
  included, and over a pending **delete** the insert succeeds. Past the session's
  `lock_timeout` the wait fails with `55P03`. The embedded `Database` can't wait, so it
  refuses with `55P03` right away (`EDGES.md` row 229). The full statement is
  [storage.md](storage.md) § Isolation Guarantees (`EDGES.md` rows 98, 101).
- **A duplicate target column, closed.** `INSERT INTO t (a, a) VALUES (1, 2)` is
  `42701 column "a" specified more than once`, as on Postgres. The column list is a set,
  on both the `VALUES` route and the `INSERT … SELECT` one. It's checked where
  `checkInsertTargets` checks it: before the arity check, the cells' conversions and the
  identity guard, and item by item, so a repeat wins over a *later* unknown name and loses
  to an earlier one. One ordering pglet does **not** share is the `INSERT … SELECT`
  **source query**: 18.6 transforms it after the list, and pglet lowers it before. The
  `COPY` and `SET`-list halves of the same rule, and what's left over, are in
  [names.md](names.md) § A statement's target column list is a set.
- **Row lengths, closed.** A `VALUES` list whose rows differ in length, and a row longer
  or shorter than its target list, give 18.6's `42601`: `VALUES lists must all be the
  same length`, `INSERT has more expressions than target columns`, or `INSERT has more
  target columns than expressions`. An earlier row's error wins over a later row's
  length, matching the order `transformInsertStmt` lowers the rows in. There's one
  exception: an error 18.6 raises during parse analysis of a cell or an
  `INSERT … SELECT` source, which pglet only raises after the length comparison. If that error is
  in any row up to and including the refused one (a one-row `INSERT` included), pglet
  gives the `42601` instead (`VALUES (1, 2), (1 + '1x')` is `22P02` on 18.6). `EDGES.md`
  rows 225 and 226 have the measurements and that loud residual.
- **An `ON CONFLICT DO UPDATE` meeting its own row, closed.** A statement whose rows
  propose the same key twice is
  `21000 ON CONFLICT DO UPDATE command cannot affect row a second time`, decided by the
  row's identity and before the action's `WHERE` (`EDGES.md` row 247).
- **Assignment casts, closed.** Postgres coerces every `INSERT` cell, `INSERT … SELECT`
  source column, `UPDATE … SET` and `ON CONFLICT … DO UPDATE SET` right-hand side with
  `COERCION_ASSIGNMENT`, and refuses with `42804` when no such cast exists. That refuses
  more than `pg_cast`'s explicit-only rows, since a pair with no `pg_cast` entry at all is
  refused too. So `INSERT INTO t (b) VALUES (1)` over `t(b BOOL)` is `42804` while
  `SELECT 1::bool` still returns `t`, and `text → integer` is refused as well. pglet
  checks this at plan time at all four sites, and accepts by four rules: the same type,
  an implicit/assignment `pg_cast` row, I/O coercion into a string-category *target*
  (so every type assigns into `text`/`varchar` and never the reverse), and element-wise
  for two arrays. A **bare** string constant is Postgres's `unknown` and is exempt, so
  `INSERT INTO t (b) VALUES ('t')` stays ordinary SQL. That only holds while it's bare: a
  cast, a `CASE`, a `COALESCE`, an `ARRAY[…]` or an `ORDER BY` naming it all resolve it
  to `text` first, on both engines. Residuals are recorded in `EDGES.md`.

## Related capabilities

- **explicit sequences.** `CREATE SEQUENCE` is supported, along with the implicit SERIAL
  sequence and `DEFAULT nextval('name')` extraction. A sequence is a **catalog record**
  (`{last_value, start, owner}`), not just a counter. `start` backs
  `ALTER SEQUENCE … RESTART`, and `owner` backs the DROP-dependency AUTO edge and
  `TRUNCATE … RESTART IDENTITY`. `ALTER SEQUENCE` and `DROP SEQUENCE` parse and work.
  `nextval`/`currval`/`setval` can also be called as ordinary expressions (a top-level
  `SELECT nextval('s')` / `INSERT … VALUES (nextval('s'))`), and `setval` advances and
  persists the counter. See the Sequences row in [compatibility.md](compatibility.md).
- **identity columns.** `GENERATED {ALWAYS|BY DEFAULT} AS IDENTITY` reuses the SERIAL
  machinery (`Column.identity` plus a `NextVal` default, with an implicit NOT NULL).
  `ALWAYS` rejects an explicit non-`DEFAULT` value (`428C9`) unless you give
  `OVERRIDING SYSTEM VALUE`, and `OVERRIDING USER VALUE` forces the sequence value. See
  the Sequences row in [compatibility.md](compatibility.md).
- **constraint enforcement.** NOT NULL (`23502`), CHECK (`23514`), PK/UNIQUE (`23505`,
  through auto-created enforced unique indexes: single-column, composite, partial,
  expression, `NULLS NOT DISTINCT`) and FK (`23503`, single **and composite**, with both
  `ON DELETE` and `ON UPDATE` actions) are enforced across the write paths. This is
  separate from this cluster (see the PK / UNIQUE / FK row in
  [compatibility.md](compatibility.md)).
