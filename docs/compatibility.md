# SQL compatibility

What pglet supports, feature by feature, measured against PostgreSQL 18.6 by the
differential fixture harness. The table answers "does pglet support X?" in a line; the
feature's full record — what works, the measured divergences and the gaps — is under its
heading in [§ Details](#details), which the feature name links to. A one-line note is a
summary of that record, never a substitute for it.

The roadmap is [`PLAN.md`](../PLAN.md). The SQL a test suite commonly meets and pglet
refuses, and the differences it answers silently, are listed in plain words in the npm
package's README ([Known differences from PostgreSQL](../crates/pglet_wasm/npm/README.md#known-differences-from-postgresql)
and [Not supported yet](../crates/pglet_wasm/npm/README.md#not-supported-yet)).

**Status.** ✅ works, and its record names no open gap or divergence that ordinary SQL —
what an application, an ORM, a migration, a seed or a test suite writes — is likely to
meet; a residual it does name is exotic. ⚠️ works, with at least one open gap or
divergence ordinary SQL is likely to meet, each named in its record. 🔲 absent: refused loudly
wherever it is written. A status is a judgement over the record, so where the two seem to
disagree, the record wins.

## Summary

| Feature | Status | Note |
|---|---|---|
| [SELECT/INSERT/UPDATE/DELETE, WHERE, ORDER/LIMIT/OFFSET](#selectinsertupdatedelete-where-orderlimitoffset) | ⚠️ | The core statements and clauses; `ORDER BY` may name a column the select list leaves out, and `OFFSET` works without `LIMIT`; `FETCH FIRST` is `42601`. |
| [`UPDATE … [alias] SET … FROM other [a] WHERE …`](#update--alias-set--from-other-a-where-) | ⚠️ | Joins a source into an `UPDATE`; `SET`, `WHERE` and `RETURNING` resolve over target and source together. `DELETE … USING` is `42601`, as are an alias on a `DELETE`'s target (`DELETE FROM t AS x`) and `DELETE FROM ONLY t`. |
| [TRUNCATE](#truncate) | ✅ | Several tables, `CASCADE`, `RESTART` / `CONTINUE IDENTITY`; the tail clauses are accepted in either order, where PostgreSQL fixes it. |
| [`DROP SCHEMA` / `CREATE SCHEMA`](#drop-schema--create-schema) | ⚠️ | Only `public` is modelled: `DROP SCHEMA public CASCADE; CREATE SCHEMA public` resets the catalog, and another schema name is refused. |
| [`DROP TABLE` / `DROP INDEX` dependencies](#drop-table--drop-index-dependencies) | ⚠️ | Foreign-key and owned-sequence dependents block (`2BP01`) or cascade as in PostgreSQL; `ALTER TABLE … DROP CONSTRAINT` is `0A000`. |
| [Arithmetic (`+` `-` `*` `/` `%` `^`)](#arithmetic-------) | ⚠️ | Integer, float and `numeric` arithmetic at PostgreSQL's result types, `22003` on overflow, `22012` on division by zero; a `text` operand and unary `+` are under-checked. |
| [Numeric narrowing casts (`int8::int4`, `float8::int4`, …)](#numeric-narrowing-casts-int8int4-float8int4-) | ✅ | Out of range is `22003`, never a wrap; float to integer rounds half to even. |
| [INNER/LEFT/RIGHT/FULL / CROSS JOIN, comma-join, `JOIN … USING`](#innerleftrightfull--cross-join-comma-join-join--using) | ⚠️ | Every join kind as a nested loop, `USING` merging its columns; `NATURAL JOIN` is `0A000`, an `ON` naming a table out of its scope errors only once a row is evaluated, and a `USING` merge of differently-typed columns named bare is declared at the left side's type (`EDGES.md` row 309). |
| [Aggregates, GROUP BY, HAVING](#aggregates-group-by-having) | ⚠️ | `count`, `sum`, `avg`, `min`, `max` with `GROUP BY` and `HAVING`, under an expression (`round(avg(x), 2)`), at 18.6's result types — `avg` of an integer and `sum` of a `bigint` are exact `numeric` where the argument's integer type is certain (`EDGES.md` row 2). `array_agg`, `string_agg`, `bool_and` are `0A000`, `FILTER` `42601` (`EDGES.md` row 44). |
| [`COALESCE` / `CASE` common types](#coalesce--case-common-types) | ✅ | Branches resolve to PostgreSQL's common type and are cast to it, `42804` / `42846` where there is none; a branch the check cannot type can answer where PostgreSQL refuses (`EDGES.md` row 65). |
| [`IN` / `NOT IN` list common type](#in--not-in-list-common-type) | ⚠️ | A table's own column against written constants compares at PostgreSQL's common type; any other list compares item by item and can miss a `real` row (`EDGES.md` row 280). |
| [Subqueries (scalar/IN/derived)](#subqueries-scalarinderived) | ⚠️ | Scalar, `IN`, `EXISTS`, derived and correlated subqueries; `= ANY (SELECT …)` and `ALL (SELECT …)` are `42601`, a `WITH` or `VALUES` subquery `0A000`. |
| [CTEs (WITH)](#ctes-with) | ⚠️ | Plain, column-listed, data-modifying and `WITH RECURSIVE`; mutual recursion is `0A000`, and several data-modifying CTEs run in declaration order (`EDGES.md` rows 160, 162). |
| [Window functions](#window-functions) | ⚠️ | `row_number`, `rank`, `dense_rank` only; frames, `lag` / `lead` / `ntile` / `first_value` and aggregates over a window are refused. |
| [Transactions (BEGIN/COMMIT/ROLLBACK/SAVEPOINT)](#transactions-begincommitrollbacksavepoint) | ⚠️ | `BEGIN` / `COMMIT` / `ROLLBACK`, savepoints, isolation levels and PostgreSQL's aborted block, one writing block at a time; a multi-statement message is not one transaction; `READ ONLY` is `0A000`. |
| [JSONB `->`/`->>`, `@>`, key/path ops, `jsonb_build_object`](#jsonb-----keypath-ops-jsonb_build_object) | ⚠️ | `->`, `->>`, `#>`, `#>>`, `?`, `?\|`, `?&`, `@>`, `<@` and `jsonb_build_object`, containment GIN-accelerated; a `text` operand is accepted where PostgreSQL refuses, and jsonpath (`@?`, `@@`) is refused. |
| [`json` (as distinct from `jsonb`)](#json-as-distinct-from-jsonb) | 🔲 | `42704` wherever the type is named; a text-preserving `json` is unbuilt. |
| [ON CONFLICT](#on-conflict) | ⚠️ | `DO NOTHING` and `DO UPDATE … [WHERE]` with `EXCLUDED`, over column, expression and partial-index targets; no `ON CONSTRAINT`, and a target-less `DO UPDATE` silently runs as a plain `INSERT`. |
| [`CREATE`/`DROP INDEX CONCURRENTLY`](#createdrop-index-concurrently) | ✅ | `25001` inside a transaction block; otherwise built as the plain spelling builds it, blocking what it blocks. |
| [`CREATE`/`DROP EXTENSION`](#createdrop-extension) | ⚠️ | `pgcrypto`, `uuid-ossp` and `plpgsql` are accepted by name, their functions being builtins here; any other name is `0A000`. No extension catalog. |
| [CREATE INDEX](#create-index) | ⚠️ | B-tree and GIN, unique, partial and expression indexes, used for equality, ranges, single-key `ORDER BY` and `@>`; expression and enum keys never speed a query; `INCLUDE` is `42601`, `USING hash` `0A000`. |
| [Index entry size limits](#index-entry-size-limits) | ⚠️ | Each entry sized as PostgreSQL sizes it (`54000`); the over-refusals and silent residuals are `EDGES.md` rows. |
| [NOT NULL / CHECK constraints](#not-null--check-constraints) | ⚠️ | Enforced at write time (`23502` / `23514`); a `CHECK` is type-checked when it is created, and a quoted literal beside a non-text column in one is `42883` (`EDGES.md` row 196). |
| [PK / UNIQUE / FK constraints](#pk--unique--fk-constraints) | ⚠️ | Enforced, single and composite, with every `ON DELETE` / `ON UPDATE` action; no `DEFERRABLE`, and `MATCH FULL` / `PARTIAL` is `0A000`. |
| [INSERT column-list binding](#insert-column-list-binding) | ✅ | Binds by name, omitted columns take their `DEFAULT`, `DEFAULT VALUES` works, and a column named twice is `42701`. |
| [Arrays (`text[]`/`uuid[]`/…), `@>`/`<@`, `ANY`/`ALL`, `unnest`](#arrays-textuuid--anyall-unnest) | ⚠️ | One-dimensional arrays of every column type, `@>` / `<@`, `= ANY` / `ALL` and `unnest`; a nested array is `0A000`, a `timestamptz` or `varchar` constructor compares at the wrong type (`EDGES.md` row 281), and a mixed-type `ARRAY[…]` under a FROM `unnest` declares its first element's type (row 308). |
| [GIN indexes for arrays / JSONB `@>`](#gin-indexes-for-arrays--jsonb-) | ✅ | `USING gin` over arrays and `jsonb`, used for a top-level `@>` and for an `OR` of them over one column. |
| [ENUM types](#enum-types) | ⚠️ | `CREATE TYPE … AS ENUM`, ordered by member position, checked on write; sent as `text` on the wire, no enum array column, and `LIKE` and string functions read an enum as `text` (`EDGES.md` row 190). |
| [Sequences / SERIAL auto-increment](#sequences--serial-auto-increment) | ⚠️ | `SERIAL`, identity columns and named sequences; every draw is `+1`, silently, whatever `INCREMENT BY` / `MINVALUE` / `MAXVALUE` / `CYCLE` say (`EDGES.md` row 215). |
| [`DEFAULT` functions (`now()`, `gen_random_uuid()`)](#default-functions-now-gen_random_uuid) | ⚠️ | Literal, `now()`-family and `gen_random_uuid()` defaults, the `now`-family spellings kept distinct, each reading the instant its PostgreSQL namesake reads; outside a block a bound statement's `now()` is its `statement_timestamp()` (`EDGES.md` row 292). |
| [`starts_with(a, b)`](#starts_witha-b) | ✅ | A case-sensitive literal prefix over `text`; a non-text argument is `42883`, an enum excepted (`EDGES.md` row 190). |
| [`\|\|` string concat](#-string-concat) | ⚠️ | The text forms only; `bytea \|\| bytea`, `jsonb \|\| jsonb` and the array forms are `0A000`, and an uncast string or `$N` beside a `jsonb` or `bytea` side is joined as text, silently, and beside an array is refused (`42883`, or `0A000` over a `text[]`) (`EDGES.md` row 293). |
| [`LIKE … ESCAPE` + default `\` escape](#like--escape--default--escape) | ⚠️ | `\` is the default and the only accepted escape; a non-text operand is `42883`, a `bytea` beside an uncast pattern included, which PostgreSQL answers (`EDGES.md` row 262), `bytea LIKE bytea` is `0A000`, and `LIKE ANY (…)` is `42601`. |
| [`ILIKE`](#ilike) | ⚠️ | `LIKE` over Rust's full Unicode lowercasing, so a few characters fold otherwise than PostgreSQL's (`EDGES.md` row 264), and `ILIKE ANY (…)` is `42601`. |
| [Scalar string + misc functions](#scalar-string--misc-functions) | ⚠️ | The common string functions (`upper`, `lower`, `length`, `substring` and `substr` over `text` and `bytea`, `left`, `right`, `octet_length`, `md5`, `trim`, `position`, `replace`, `concat`, `lpad`, `split_part`, …), and `random()`; an unbuilt one is `0A000` (`format`, `repeat`). |
| [`date_trunc` / `extract` / `date_part`](#date_trunc--extract--date_part) | ✅ | `date_trunc`, `EXTRACT` and `date_part` over the temporal types at PostgreSQL's result types; an untyped `date_trunc` source answers where PostgreSQL refuses. |
| [`SELECT DISTINCT` / `DISTINCT ON`](#select-distinct--distinct-on) | ✅ | `DISTINCT` and `DISTINCT ON`, with PostgreSQL's `ORDER BY` match rule (`42P10`) and its `NULL` and `NaN` equality. |
| [`UNION` / `INTERSECT` / `EXCEPT` (+ `ALL`)](#union--intersect--except--all) | ✅ | `UNION`, `INTERSECT`, `EXCEPT`, with and without `ALL`, at PostgreSQL's precedence and common column types; a recursive term is typed by its own check. |
| [`CASE WHEN … END`](#case-when--end) | ✅ | Searched and simple forms; a non-boolean `WHEN` condition is `42804`. |
| [Boolean contexts (`WHERE`, `HAVING`, `JOIN … ON`, searched `CASE WHEN`, `AND`/`OR`/`NOT`, `CHECK`, a partial index's `WHERE`, `IS [NOT] TRUE\|FALSE\|UNKNOWN`)](#boolean-contexts-where-having-join--on-searched-case-when-andornot-check-a-partial-indexs-where-is-not-truefalseunknown) | ✅ | Each context resolves its operand to `boolean` as PostgreSQL does: any other type is `42804`, an unresolved literal is read as a boolean. |
| [`FOR UPDATE` / `FOR SHARE`](#for-update--for-share) | ⚠️ | Every lock strength is parsed; inside a block a locking read takes the single writer, coarser than a row lock. `SKIP LOCKED` and `NOWAIT` are `0A000`. |
| [`alias.*` qualified wildcard](#alias-qualified-wildcard) | ⚠️ | Expanded to that relation's columns, beside other items; over a `FROM`-clause `unnest(…)` item it is `0A000` or `42702`, or expands a same-named base table's column names (`42703` unless they match) (`EDGES.md` row 8). |
| [Non-sole `*` + `RETURNING` projection](#non-sole---returning-projection) | ✅ | `SELECT *, …` and `RETURNING *, …`, a correlated subquery in `RETURNING` included; PostgreSQL 18's `OLD` / `NEW` in `RETURNING` is `42P01`. |
| [`DO $$ … $$` anonymous block](#do----anonymous-block) | ⚠️ | `BEGIN … [EXCEPTION WHEN …] END` over plain SQL statements, an `EXCEPTION` block a real subtransaction; `DECLARE`, variables and control flow are refused. |
| [Keyword as identifier](#keyword-as-identifier) | ⚠️ | A keyword is reserved by PostgreSQL's own class for it, so `update`, `set`, `values` and the like are usable as names; the SQL value-function words (`user`, `current_date`, …) are accepted as a column name where PostgreSQL refuses them (`EDGES.md` row 75). |
| [Operator lexing + resolution](#operator-lexing--resolution) | ⚠️ | Lexed as PostgreSQL lexes an operator; an operator pglet lacks (`&`, `#-`, `@?`, `@@`, …) is `42883`, and `OPERATOR(…)` is not parsed. |
| [Identifier case-folding](#identifier-case-folding) | ✅ | Unquoted names fold to ASCII lower case where they are read and quoted names are exact, as on PostgreSQL over a multibyte (UTF-8) database encoding. |
| [INTERVAL](#interval) | ✅ | A full column type with PostgreSQL's three input styles, arithmetic and ordering; no `infinity`, and a field spec (`interval year`) is refused or ignored. |
| [DATE](#date) | ✅ | A full column type over `0001-01-01` to `9999-12-31` plus `infinity`; no BC dates. |
| [NUMERIC / DECIMAL](#numeric--decimal) | ✅ | Exact decimal with `NUMERIC(p,s)`, PostgreSQL's scale rules and exact `SUM` / `AVG`; a fractional exponent and numeric `Infinity` are refused. |
| [Session information functions](#session-information-functions) | ⚠️ | `version()`, `current_user` and the other identity spellings, `current_schema`, `current_setting`; `SET search_path` does not move their answers, and there are no roles. |
| [SQL typed constants (`DATE '2024-06-15'`, `NUMERIC(5,2) '1.5'`, `DOUBLE PRECISION '…'`)](#sql-typed-constants-date-2024-06-15-numeric52-15-double-precision-) | ✅ | `DATE '…'`, `NUMERIC(5,2) '…'` and the other typed-constant spellings, read as the matching cast. |
| [`x IS [NOT] TRUE / FALSE / UNKNOWN`](#x-is-not-true--false--unknown) | ✅ | PostgreSQL's boolean test, never `NULL`; a non-boolean operand is `42804`. |
| [TIME](#time) | ✅ | A full column type with `time(p)`; `timetz` is `42704`, a zone suffix on a time literal is `22007`, and `CURRENT_TIME` is `time`, not `timetz`. |
| [bytea](#bytea) | ⚠️ | A full column type, hex and escape input read as PostgreSQL reads them; `bytea \|\| bytea` and `bytea LIKE bytea` are `0A000`, and an uncast string beside a `bytea` under `\|\|` is joined as text (`EDGES.md` row 293). |
| [Real prepared statements / param type inference](#real-prepared-statements--param-type-inference) | ⚠️ | `$N` parameters typed from their context or from the client's declared types, reported in `ParameterDescription`; a `$N` used twice is not unified, and one beside arithmetic (`SET n = n + $1`) is left untyped, so a text-format bind of it is `42804`. |
| [`to_char` / `AT TIME ZONE`](#to_char--at-time-zone) | ⚠️ | The ISO-8601 `to_char` tokens over a timestamp, and `AT TIME ZONE 'UTC'`; another zone or an unknown token is refused. |
| [`timestamptz` text wire output](#timestamptz-text-wire-output) | ✅ | Rendered with the `+00` offset, so a text-protocol driver reads the right instant. |
| [`ErrorResponse` structured fields](#errorresponse-structured-fields) | ⚠️ | SQLSTATE, message, detail, hint, position and the table / column / constraint fields where PostgreSQL sends them; a field pglet cannot fill truthfully is omitted, and some are not sent yet. |
| [`jsonb_build_object`, `IS [NOT] DISTINCT FROM`](#jsonb_build_object-is-not-distinct-from) | ⚠️ | `jsonb_build_object` and the null-safe `IS [NOT] DISTINCT FROM`. A timestamp or array value in `jsonb_build_object` is rendered as its text output, not as 18.6 renders it (`EDGES.md` row 306). |
| [Scalar `EXISTS` / `NOT EXISTS`](#scalar-exists--not-exists) | ⚠️ | Plain and correlated `EXISTS` / `NOT EXISTS`; the sub-plan runs to its end, so a row past the first match that raises is an error where PostgreSQL answers (`EDGES.md` row 320). |
| [`INSERT … SELECT` + `unnest($N)` in a SELECT projection](#insert--select--unnestn-in-a-select-projection) | ⚠️ | `INSERT … SELECT`, and `unnest($1)` in a `SELECT` with no `FROM`; `unnest(…)` in the select list of a query with a `FROM` is `0A000`. |
| [`generate_series` in `FROM`](#generate_series-in-from) | ⚠️ | The `integer` and `bigint` series over constant and parameter arguments, up to 1 000 000 rows (`54000` past it); the `numeric` and timestamp series, an argument naming a column (the lateral form) or holding a subquery, and the select-list form are refused (`EDGES.md` row 321). |
| [pg_catalog introspection (drizzle-kit push etc.)](#pg_catalog-introspection-drizzle-kit-push-etc) | ⚠️ | The catalogs and functions ORM introspection reads (`pg_class`, `pg_attribute`, `pg_type`, `pg_constraint`, `pg_index`, `pg_enum`, `pg_tables`, `pg_indexes`, …), several with a subset of their columns. |
| [`information_schema`](#information_schema) | ⚠️ | `tables`, `columns`, `table_constraints`, `key_column_usage`, `referential_constraints`, `constraint_column_usage`; a `char(n)` column reads as `character varying`. |
| [`ALTER TABLE … ADD CONSTRAINT`](#alter-table--add-constraint) | ⚠️ | `CHECK`, `PRIMARY KEY`, `UNIQUE` and `FOREIGN KEY`, each validated against the existing rows, then enforced; `DROP CONSTRAINT` is `0A000`. |
| [`CREATE SEQUENCE` / `ALTER SEQUENCE` / `GRANT` / `REVOKE` / `COMMENT ON` / `ALTER TABLE … OWNER TO`](#create-sequence--alter-sequence--grant--revoke--comment-on--alter-table--owner-to) | ⚠️ | `pg_dump` preamble tolerance: a named sequence honors `START WITH`, `RESTART` and `OWNED BY`; `GRANT`, `REVOKE`, `COMMENT ON` and `OWNER TO` succeed as no-ops. |
| [`SET` / `SHOW` / `DEALLOCATE` / `DISCARD` / `RESET`](#set--show--deallocate--discard--reset) | ⚠️ | `SET`, `SHOW`, `RESET`, `DEALLOCATE`, `DISCARD`; a setting pglet holds fixed (`TimeZone` `UTC`, `DateStyle`, …) takes only that value, and a `search_path` naming another schema is `0A000`. |
| [`COPY … FROM STDIN` / `TO STDOUT`](#copy--from-stdin--to-stdout) | ⚠️ | Text format over the wire and the embed API, through the same checks as `INSERT`; `csv` and `binary` are `0A000`, `COPY (query) TO` is `42601`. |
| [`pg_dump` plain-text restore](#pg_dump-plain-text-restore) | ⚠️ | A plain-text `pg_dump \| psql` replays, preamble and identity columns included; a database with an enum column does not restore. |
| [Native dump emitter](#native-dump-emitter) | ⚠️ | `dump_sql()` and `pglet_server dump` emit portable SQL; some states pglet's own statements build still fail to export or to reload (`EDGES.md`). |
| [LISTEN/NOTIFY](#listennotify) | 🔲 | `42601`: the keywords are unknown to the lexer. |
| [Object kinds the `CREATE` / `DROP` dispatch does not name](#object-kinds-the-create--drop-dispatch-does-not-name) | 🔲 | `CREATE` / `DROP` of a domain, a view, a materialized view, a trigger, a function or a temporary table is `0A000`. |

## Details

### SELECT/INSERT/UPDATE/DELETE, WHERE, ORDER/LIMIT/OFFSET

`ORDER BY` a non-projected base column is honored (sorts the pre-projection rows; an
output-only alias/position keeps sort-above); `OFFSET` works without `LIMIT`. A `LIMIT`
cuts the rows after every input row is evaluated, so an error in a row 18.6 never reads is
raised — as it is in a derived table under an outer `WHERE false` or joined with an empty
table (`EDGES.md` row 320). The full record: § SELECT / DML family — details, below.

#### SELECT / DML family — details

A **no-FROM** `SELECT` runs the same tail: its trailing `ORDER BY`/`LIMIT`/`OFFSET`
routes through `plan_no_from_order_limit` (which also *binds* its sort keys, a
`Plan::Sort` over a `Plan::Values` judging them only where a row reaches the eval-time
resolver) and its row-level locking clause through the `parse_for_locking` its sibling
parse arms call.
One divergence is left open there, and it is a plan-**shape** exclusion rather than a
leniency: a top-level sequence projection is dispatched on
`Plan::is_sequence_projection`, which matches a bare `Plan::Values`, so the tail's gates
run over one but the `Sort`/`Limit` is not built — `SELECT nextval('s') LIMIT 0` answers
a row here against 18.6's `(0 rows)` (which also leaves the counter unadvanced, where
each pglet spelling measured advances it), while `SELECT nextval('s') ORDER BY zzz` is
`42703` and `ORDER BY 5` `42P10` on both. It reaches a **referenced CTE body** as well
as the outermost select, and a projection that merely *carries* a sequence function
beside other expressions; the **write** spellings measured are each loud here, so the
wrong row count stays inside a read. The `DISTINCT`/`DISTINCT ON` quantifiers take the
same exclusion and it changes no answer there — a dedup over one row returns that row.
See `EDGES.md`. A positional `ORDER BY n` binds the Nth **output** column (out-of-range
→ `42P10`); mixed with a dropped-base sort key it's rewritten to that projection
expression so it still sorts correctly below the projection — and the rewritten
expression is **shielded when it is itself a bare integer literal**, which is also the
plan's spelling of a positional reference. Unshielded, the Sort below read that ordinal
against the *base* row: `SELECT 't', 5 FROM s ORDER BY 2, a` would be `42P10 position 5
is not in select list` over a one-column source, and `SELECT 2 AS z, b FROM s ORDER BY 1,
a` would return the rows in `b` order rather than `a` order, where each answers without
error on 18.6 (`select/order_by_below_projection_positional_key_is_a_value.sql`).

**Divergence:** two identical unqualified output names referenced by `ORDER
BY`/`DISTINCT ON` (`SELECT a, a … ORDER BY a`) fail loud `42702` where Postgres answers
— its `findTargetlistEntrySQL92` compares two matching target-list entries with
`equal()` and refuses only when they differ (`a AS x, b AS x … ORDER BY x`). The
**plan-time** gate compares the two matching entries' **expressions** rather than
counting their names, and the comparison is structural — the postcard bytes of the
`Expr`, which is what `equal()` → `datumIsEqual` is, and deliberately **not** pglet's
own `PartialEq`, under which `PgNumeric`'s `1.5 == 1.50` and `PgInterval`'s `INTERVAL
'30 days' == INTERVAL '1 mon'` would fold two entries 18.6 calls ambiguous into one
claimant and destroy rows. Read that as the members its fixtures hold, not as `equal()` in
general: the accept side is a wildcard's qualified reference beside a written bare one,
an `AS` label, a qualified/bare pair, a `GROUP BY` target list and a `DISTINCT ON` key
(`select/nested_sort_key_duplicate_output_entries_resolve.sql`), and the refuse side is
two different columns plus those two value folds
(`select/nested_sort_key_duplicate_output_entries_loud.sql`,
`select/nested_sort_key_numeric_display_scale_is_not_equal.sql`,
`select/nested_sort_key_interval_estimate_is_not_equal.sql`).

**Four causes make it refuse what 18.6 resolves** — a no-op cast on an
already-typed expression, an unknown-typed literal's cast that is not folded to a
constant, an `IN` list that is not normalized to `= ANY (ARRAY[…])`, and
`sole_relation_carrying`'s three declines — each loud (`42702`), with the measured
members listed in `EDGES.md`. (These four are the causes measured, not a claim that a
fifth cannot exist; a unary `-` over a numeric constant is not one, the parser folding it
into the constant, below.) Each is row-count-**independent at a level's own lowering** as
well, the sort keys sitting below the projection: measured over an empty relation, a
duplicated no-op-cast pair, a duplicated unknown-literal-cast pair, an `IN`-list beside
its `= ANY` form, and `sole_relation_carrying`'s declines over a `USING` join and a
catalog view are each `42702`, as they are seeded (a measured sweep, not a proof). An
`unnest` FROM item is not among those declines: a FROM-clause `unnest(…)` registers
columns-exhaustive, so its level is certain and `sole_relation_carrying` answers —
`SELECT u.x AS q, x AS q FROM unnest(ARRAY[]::int[]) AS u(x) ORDER BY q` is `(0 rows)`
on both engines (`select/unnest_duplicate_output_entries_bind_to_the_unnest.sql`), while
its non-empty spelling and its `GROUP BY q` twin stay `42702` here from resolvers this
binding does not reach. A failing bare key also outranks an unparsed trailing set-op
token, the third member of a loud precedence class: `SELECT a FROM ne ORDER BY bogus
UNION SELECT a FROM nf` is `42703` here against 18.6's `42601` at the `UNION`, beside the
positional (`42P10`) and qualified (`42P01`) spellings, which diverge the same way, with
`ORDER BY a` / `ORDER BY 1` / `LIMIT 1` in that position `42601` on both as the
controls. Beside them stands the **eval-time** resolver, which flags the name collision
on the row it is handed, so the divergence is row-count-dependent:
measured over `ja2(c, x)` holding two rows, `SELECT c, c FROM ja2 ORDER BY c` and
`SELECT DISTINCT ON (c) c, c FROM ja2` are `42702` here and answer on 18.6, while the
same statements over an empty relation answer `(0 rows)` on both. `GROUP BY` does not
share that shortcut — it compares expressions, per the Aggregates row below. `SET col =
DEFAULT` resolves the column's catalog default (incl. SERIAL/nextval) on UPDATE and `ON
CONFLICT DO UPDATE`, and a SET-RHS eval error propagates (not swallowed). Every right-hand
side of one `SET` list reads the target row the scan or the conflict probe found, never an
earlier assignment's result, and they run in column order (`EDGES.md` rows 243 and 244); a
`DO UPDATE` that meets a row its own statement wrote is refused `21000` (row 247). A negative
`LIMIT`/`OFFSET` fails loud (`2201W`/`2201X`) and an overflowing one `22003`; a
fractional count rounds **half-away-from-zero** (matching Postgres's `numeric→int8`
coercion — `LIMIT 2.5` → 3 rows), never silently to unbounded.

**Divergence — `LIMIT`/`OFFSET` does not share the parser's constant fold.**
`parse_limit_count` reads the sign off a raw `-`-then-`Number` token pair ahead of its
general dispatch, so it never sees the folded constant `parser::expr`'s `negate`
produces, and it rejects a fold 18.6 *does* perform: `LIMIT -0` and `LIMIT -0.4` are
`2201W` here against `(0 rows)` there and `OFFSET -0` is `2201X` here against every row
there (`doNegate` folds `-0` to `0`, and the `numeric→int8` round takes `-0.4` there
too), while `LIMIT - -1` — two folds — is `42601` here against one row there. Read the
**parenthesized** spellings as this clause's own grammar rather than as the fold: `LIMIT
-(1)` / `OFFSET -(1)` are `42601` here against `2201W`/`2201X` there, but so is the
unsigned `LIMIT (1)` against one row there, and the quoted `LIMIT '1'` / `OFFSET '0'`
likewise (`42601` here, read as the count there), because the clause takes a bare number /
`$N` / `ALL` / `NULL` and no expression at all. Every one of those is loud here, so none
is a wrong answer. Being read in the **parser** also puts this clause ahead of every
plan-time gate, which is observable wherever the two are wrong at once: `… UNION … ORDER
BY bogus+1 LIMIT -1` is `2201W` here against 18.6's `42703` for the key, since 18.6
resolves the sort keys in parse analysis and reaches the bound after — loud both ways,
wrong code, where the `LIMIT`-less spelling is `42703` on both.
Unrelated to the sign but in the same clause: `FETCH FIRST <n> ROWS ONLY` is unparsed
here (`42601`) and answers on 18.6. `ORDER BY` runs the plan-time coercion pass against
the same scope as the projection, so `ORDER BY (id = '<uuid>')` against a `uuid` column
resolves rather than raising `42883`; a bare string constant still takes Postgres's
SQL92 ordinal rule (`ORDER BY 'lit'` is not a sort key). `ORDER BY +1` is **not** an
ordinal — unary `+` stays an operator node (`Expr::UnaryPlus`), mirroring Postgres's
grammar, which folds only unary `-` into a constant — so it sorts by a constant and
preserves the scan order rather than sorting by output column 1. Its mirror
image is that `ORDER BY -1` **is** an ordinal, a negative one: `parser::expr`'s `negate`
folds `-` over a `Literal::Int`/`Literal::Numeric` into the constant (`doNegate`'s two
`A_Const` kinds), so `-1` reaches the bounds check as `42P10 position -1`, `-0` as
`42P10 position 0`, `- -99` as `42P10 position 99`, and `-1.5` as `42601 non-integer
constant`, while `-99::int` / `-abs(99)` / `-id` stay ordinary keys because none of the
three is a constant node. `GROUP BY` and `DISTINCT ON` fold with it, and a no-FROM
`SELECT` reaches the same bounds check through its routed sort/bound tail (the paragraph
at the head of this row): `SELECT 1 AS q ORDER BY -1` and its positive control `ORDER BY
5` are `42P10` on both (`select/no_from_order_by_negative_position_loud.sql`,
`select/no_from_order_by_bad_position_loud.sql`). Unfolded, every one of those spellings
would sort by a constant and **run** — including inside the filtering subquery of a
`DELETE`/`UPDATE`, a silent write; see `EDGES.md`.

**Divergence:** a set-op's trailing `ORDER BY` is outside the coercion pass; its keys
bind at their own lowering against the set-op's output names alone — **every
`Expr::Column` leaf of a key, not only a key that is one**, which is the namespace 18.6
uses for them too (`transformSetOperationStmt` makes the output columns visible as a
join RTE) — while the *expression* rule itself is still over-accepted, which 18.6
refuses `0A000 invalid UNION/INTERSECT/EXCEPT ORDER BY clause`: measured over two
one-column tables, `… UNION … ORDER BY -a` / `a+0` / `abs(a)` each sort here and are
`0A000` there, whereas `… ORDER BY bogus+1` is `42703` on both, since 18.6 resolves the
names before raising that error. Not *any* expression, and not `42601` — a parenthesized
column `ORDER BY (a)` and a cast `ORDER BY a::int` answer identically on both engines,
and the `42601` in this neighbourhood is `findTargetlistEntrySQL92`'s separate
non-integer-constant rule (`ORDER BY true` / `NULL` / `'x'`, `42601` on both engines,
with `ORDER BY -1` `42P10` on both). Sort **placement** is qualifier-aware, following
Postgres's `findTargetlistEntrySQL92` rule that the output-name match applies only to a
single-field reference: a **qualified** `ORDER BY` key always resolves against its named
relation (so `SELECT pa.x … JOIN pb … ORDER BY pb.x` sorts by `pb.x`, never by a
same-named projected `pa.x`), while an **unqualified** key matching an output name binds
to the output column — but only when the key *is* that name: a name **nested** inside a
key (`ORDER BY lbl+0`, `-lbl`) is an ordinary column reference resolved against the FROM
relations, as `findTargetlistEntrySQL92`'s one-field gate requires. Mixing the two in
one `ORDER BY` (`ORDER BY x, dropped_col`) sorts below the projection whenever every key
is readable from the base row. Under a `GROUP BY`, a key the select list drops — a
grouping column (`ORDER BY k`, `k*2`) or an aggregate (`ORDER BY count(*)`,
`sum(v)/count(*)`) — rides the Aggregate as a Postgres **resjunk** column that a
`Plan::JunkFilter` trims off above the Sort; an **ungrouped** base column is `42803`, as
in Postgres. See `EDGES.md` row 3 and `docs/coercion.md` § Where the plan-time coercion pass reaches.

### `UPDATE … [alias] SET … FROM other [a] WHERE …`

inner-joins a source into the UPDATE (target alias supported); each target row updated
at most once (first matching source row — PG leaves a multi-match unspecified); **`SET`,
`WHERE` and `RETURNING` all resolve over the one combined namespace** — the target under
its effective name plus the source under its own. Measured on 18.6 with `amb_t(id, v)`
the target and `amb_s(id, v, k)` the source aliased `s`, every one identical here:
`RETURNING *` → five columns (`id | v | id | v | k`, target first, then the source, in
FROM order), `RETURNING s.*` → the source's three, `RETURNING amb_t.*` → the target's
two, `RETURNING s.k` and bare `RETURNING k` → `7`, `RETURNING v` (both carry it) →
`42702`, `RETURNING amb_s.k` (the alias hides the table name) → `42P01`, `RETURNING
s.nosuch` → `42703`. Each answer is the same whether or not a row matches, and a
`Describe` on a statement matching nothing still describes all five columns. The
executor resolves a `RETURNING` item by the **`(relation, column)` pair** rather than by
a synthesized `relation.column` string, so a column a user quotes into existence with a
dot in it cannot answer for another relation's — `RETURNING "wt.v"` over a target
`wt(id, v)` and a source `sd s(id, "wt.v")` answers the source's column, and `RETURNING
"s.k"` over a target `qt(id, "s.k")` and a source `ws s(id, v, k)` answers the target's,
as 18.6 does both ways (a flat-string key would get both wrong, silently; the join scope
carries a collision of its own, by a different mechanism — `EDGES.md`). The plan-time
namespace is one derivation (`returning_relations`) over every one of the five node
kinds a source can lower to: base `Scan`, `VirtualScan`, `CteScan`, `SubqueryScan`,
`FunctionScan` — covering the *kind* is not counting the *claimants*, and over a catalog view
pglet counts and projects only the columns it emits (see `EDGES.md`). A `Scan` of a
table the catalog lacks registers **nothing**, so the executor's own `42P01` for the
missing source wins, as it does on 18.6. Its `DELETE … USING` counterpart is a
**parser** gap (`42601`), so pglet has one join-DML statement, not two; nothing in
`lower_returning` is UPDATE-specific, so closing that gap is passing the source in. The
join `WHERE` eval is correct-or-loud (an error propagates, not swallowed to a silent
0-row update). A `FROM`-clause `unnest(…)` is a source like the others throughout — for
the claimant count, for `SET`/`WHERE` type resolution, **and for a qualified
reference**: `UPDATE t SET … FROM unnest(ARRAY['x','y']) AS u WHERE u.u = 'x'` answers on
both engines, the executor deriving a relation name for the `FunctionScan` node. The same
executor arm reaches `execute_join`'s column map, so on the `SELECT` side `SELECT u.q
FROM jn JOIN unnest(ARRAY[1,2]) AS u(q) ON true` over a `jn` that has its own `q` answers
the unnest's `1, 2`, not `jn`'s value, and `SELECT 1 FROM jn, unnest(ARRAY[1,2]) AS t(q)
WHERE t.q = 1` answers its one row, each as 18.6 does — see `EDGES.md` for the measured
table. `SELECT v.* FROM unnest(…) AS v` is `0A000` here **where no base
table shares the alias's spelling** (the *`SELECT`* qualified-wildcard expansion is
keyed on the name and has no arm for an `unnest` item to reach) where 18.6 expands it and where
pglet expands that wildcard over a base table, a CTE, a derived table and a catalog view
alike — and where one *does* share it the same expansion is **silent**, answering that
table's columns against the unnest's relation (`EDGES.md`) — and where a `RETURNING u.*`
over the same source expands too

### TRUNCATE

multi-table + `CASCADE` (clears FK-dependents); `RESTART`/`CONTINUE IDENTITY`; an
FK-referenced table without `CASCADE` errors `0A000`. Clears rows directly (no FK `ON
DELETE` actions), like Postgres. `RESTART IDENTITY` returns the truncation closure's
**owned** sequences (`Sequence.owner`, Postgres's `getOwnedSequences`) to their `START
WITH` — not to zero, and not the set of sequences the columns' `nextval` defaults name,
which gets both directions wrong: it restarts a merely-referenced sequence Postgres
leaves alone and skips an owned one whose column carries no default. A `CASCADE`-pulled
child's sequences are in the set. **Divergence:** pglet's parser accepts the tail
clauses in either order, where Postgres fixes `RESTART IDENTITY` before `CASCADE`
(`42601`)

### `DROP SCHEMA` / `CREATE SCHEMA`

the migration/test reset idiom `DROP SCHEMA public CASCADE; CREATE SCHEMA public` —
pglet models one implicit `public` schema, so `DROP SCHEMA public CASCADE` clears the
whole catalog (tables/indexes/enums/sequences) and `CREATE SCHEMA public` is a no-op
success. A `public` holding a table, an enum or a sequence refuses a drop without
`CASCADE` (`2BP01`), keeping them all (one holding only an extension accepts it where
18.6 refuses — `EDGES.md` row 72); a non-`public` name is `3F000` (no-op under `IF
EXISTS`), and `CREATE SCHEMA <other>` `0A000` (only `public` is modeled)

### `DROP TABLE` / `DROP INDEX` dependencies

`DROP TABLE [CASCADE|RESTRICT]` tracks FK **and** sequence dependents through the shared
resolver in `executor/dependencies.rs`: a foreign key declared on **another** table and
referencing this one blocks a bare / `RESTRICT` drop with `2BP01`, and `CASCADE` sheds
**only that constraint** — the child table and its rows survive and are thereafter
unconstrained, matching Postgres 18.6. A table's own FKs (a self-reference included) die
with it and never block. A sequence the table (or, on `ALTER TABLE … DROP COLUMN`, the
column) **owns** goes with it — the AUTO edge — and a column *outside the deletion set*
whose `DEFAULT` draws from that sequence blocks with `2BP01`, `CASCADE` shedding that
column's default and leaving the column and its rows. The deletion set is what separates
them: a table's own columns are inside it (so `CREATE TABLE t (a SERIAL); DROP TABLE t`
succeeds), while a `DROP COLUMN`'s is column-scoped, so a **sibling** column drawing
from the dropped column's sequence blocks. Postgres lists every blocking dependent in
one message; pglet names one, foreign key first. Its order is neither by kind nor by the
dependent's own OID — `findDependentObjects` sorts each recursion level
OID-**descending** and appends depth-first, `reportDependentObjects` reads the array
back to front, so a column default (reached through the sequence) is placed by the
**sequence's** OID. Measured three ways on 18.6; the first two arrangements falsify "by
kind" and the first and third falsify "by the dependent's own OID" in either direction.
So there is no ordering rule to reproduce and the divergence is one of **completeness**.
`DROP INDEX` **refuses** the index behind a declared `PRIMARY KEY` / `UNIQUE` (`2BP01`,
with or without `CASCADE`), read from the catalog's persisted constraint marker; a
standalone `CREATE UNIQUE INDEX` still drops freely, which is Postgres's own split.
Dropping a constraint's index would silently un-enforce the key — duplicates written,
surviving a restart — *and* erase it from `dump_sql`, which classifies PK/UNIQUE from
that same marker. **Gaps:** `ALTER TABLE … DROP CONSTRAINT` is unimplemented (parsed and
refused `0A000` over a table that exists) — and with the `DROP INDEX` route refused, a
declared `PRIMARY KEY`/`UNIQUE` is unremovable short of `DROP TABLE`. Postgres's hint for the refusal names `DROP
CONSTRAINT`; pglet's message deliberately does not, since that statement is refused here
— it names dropping the table. A `CASCADE` emits no `NOTICE`, and `DROP INDEX` does
**not** yet block on an FK that depends on a *plain* unique index covering its
referenced columns (PG: `2BP01`); pglet's FK existence probe falls back to a scan when
the covering index is gone, so enforcement degrades in speed, not correctness

### Arithmetic (`+` `-` `*` `/` `%` `^`)

integer-domain (exact above 2^53), **fails loud** on overflow (`22003`) / division by
zero (`22012`); mixed-width promotes to the wider operand type, **in either order and
for all of `+ - * / %`** (`int2+int4` and `int4+int2` are both `int4`; anything with an
`int8` is `int8`) — Postgres registers a distinct operator per ordered pair with the
same `prorettype` (`int24pl`/`int42pl` → `int4`), and `%`, which has no mixed-width
operator, resolves through the implicit int2→int4→int8 casts to the same answer. The
rule is declared-type-only: the eval path already widens, so getting it wrong declared a
**narrower** type than the bytes sent (`int2` over a four-byte value) and tokio-postgres
answers `error deserializing column` rather than misreading — the same shape as the
`float4`/`sum(real)`/`COALESCE(int4,int8)` entries elsewhere in this table. One chain
(`planner::arith_result_type`: temporal → float/`NUMERIC` → integer width) serves both
declared-type walkers, and `arithmetic/int_mixed_width_promotes` pins every pair with
the wider-on-the-left direction as its control. A non-numeric operand errors. `%` modulo
(int + numeric, sign follows the dividend; **no float `%`** → `42883`; `%0` → `22012`).
`^` power (`float8` unless **both** operands resolve to numeric — Postgres registers
only `^(float8,float8)` and `^(numeric,numeric)`, so `real ^ real` is `double precision`
too, not `real`; `0^negative` / negative-base^fractional → `2201F`, float overflow →
`22003`; **left-assoc**, binds tighter than `*`, looser than unary `-` so `-2^2=4`).
`mod(a, b)` and `power(a, b)` / `pow(a, b)` are `%` and `^` spelled as calls — 18.6
registers each over its operator's signatures and body — so they evaluate, type and gate
their operands as the operators do, named after the function: the column is `mod` / `power` / `pow`, the
rendering `mod(a, 2)`, and an operand pair with no signature `42883 function mod(double
precision, integer) does not exist`. A bare string literal beside a number literal takes
the type the call resolves to (`mod(7, '3')` is `1`, `power('2', '3')` is `8` as
`double precision`), and two of them are `42725` for `mod`, which has four signatures that
fit them equally; beside a column, a parameter, a written cast (`mod(7::int8, '3')`) or any
other expression the literal stays `text`, as it does under the operators (`EDGES.md` row
262), and an untyped `$N` beside a number is not typed by the
call either (`docs/param_binding.md`). They inherit the operators' residuals: a `numeric`
raised to a non-integer `numeric` power is `0A000`, a bare `NULL` beside a `numeric`
declares `double precision` on the extended protocol (row 54), and the catalog rendering
omits 18.6's implicit `::double precision` on `power`'s arguments
(`docs/deparse.md` § What is recorded, not reproduced).
Unary `+` is value-preserving but **kept as a node** (`Expr::UnaryPlus`) so `+1` is not
structurally a constant — the SQL92 ordinal rules depend on it; **divergence:** its
operand is not type-gated, so `+'abc'::text` yields `abc` where PG raises `42883` and
`+'abc'` yields `abc` where PG raises `22P02`, and `WHERE indexed = +1` declines the index for a row filter
(slower, and identical rows wherever the two spellings evaluate the same leaves — a
*non-matching* probe starves a `DEFAULT`/`$N` beside it, so over an indexed `a` holding
`1`, `a = +2 AND DEFAULT IS NULL` is `42601` where `a = 2 AND DEFAULT IS NULL` answers
`(0 rows)`; `EDGES.md` row 56). Unary `-` is the other way round, through the fold:
`WHERE indexed = -1` is a bare constant, so index selection **offers** the index, with
rows identical to the row filter's over an encodable key wherever both evaluate the same
leaves (a probe matching no row
starves a `DEFAULT`/`$N` beside it — `(0 rows)` accelerated against `42601` for the
`DEFAULT` / pglet's own `22000` for the unbound `$N` stripped; `EDGES.md` row 56), and
the reason the negative-probe fixtures carry `@profile-lock`. **`float4 op float4 →
float4`, computed in f32** (Postgres's `float4_pl`/`_mi`/`_mul`/`_div`), so `0.1::real +
0.2::real` is `0.3` and not `0.30000000447034836`; the overflow/underflow/zero-divide
checks mirror the f64 path. `float4` mixed with int/numeric/float8 widens to
**`float8`**, which is what Postgres does (measured for every mixed pair, and
deliberately the *opposite* of the `COALESCE`/`CASE` common-type rule — see that row).
**Divergence (silent):** a `text` left operand (`tx + 1`, `tx + b`) answers over an
empty table where 18.6 refuses `42883` — declaring `int4`/`int8` on the simple-query
`RowDescription` against `Describe`'s `text` — and is pglet's own `42804` once a row
reaches it; a `float4` beside an untyped `NULL` (`f4 + NULL`) declares `701` on the
simple leg where 18.6 declares `700`, and `^` between a `numeric` and an untyped `NULL`
(`NULL ^ n`) declares `701` on `Describe` where 18.6 declares `1700` (`EDGES.md` row 54)

### Numeric narrowing casts (`int8::int4`, `float8::int4`, …)

**fail loud** on out-of-range (`22003`, like the arithmetic path) instead of silently
wrapping/saturating; float→int rounds ties-to-even (Postgres `rint`); float8→real
overflow (finite double → ±inf) errors. A `real` / `double precision` **read from text**, or
converted from `numeric`, is range-checked as `float4in` / `float8in` check it: a decimal
that overflows the type, or a nonzero one that underflows it to zero, is `22003 "…" is out
of range for type real`, never the `±Infinity` / `0` it rounds to (`EDGES.md` row 149). A
`numeric` operand beside a float in an operator is the residual: it converts without the
check (`1e400 > 1::float8` is `t` here, `22003` on 18.6)

### INNER/LEFT/RIGHT/FULL / CROSS JOIN, comma-join, `JOIN … USING`

nested-loop only. **`CROSS JOIN` / comma-join** (`FROM a, b`) → Cartesian (a left-deep
`Join ON true`). **`JOIN … USING (c…)`** desugars to `ON a.c = b.c AND …` and merges
each USING column into ONE output (`COALESCE(left.c, right.c)`, uniform across join
types) — merged columns first (USING order), then left-remaining, then right-remaining;
the raw `t1.c`/`t2.c` stay qualifiable, a bare `c`/`*` show the merged one; chained
USING composes. Merged from columns of **different types**, a bare `c` is declared at the
left side's type where 18.6 declares the common type — `integer` over a `bigint` value —
while `*` declares 18.6's (`EDGES.md` row 309). A `SELECT *` over any join is expanded at plan time to bare/merged
output columns.
**NATURAL JOIN** stays fail-loud (`0A000`). Aliased columns resolve through 3+ table
(left-deep) chains; the ON predicate is correct-or-loud (an eval error propagates).
Missing USING column → `42703`, ambiguous USING column → `42702`; an ambiguous
**unqualified** ref matching >1 relation (`SELECT num FROM a JOIN b ON a.num=b.num`, or
a bare column in an `ON`) is `42702` too — a tiered exact-then-suffix resolver shared
with the USING check, so a merged USING column (an exact bare match) stays unambiguous.
**`JOIN` binds tighter than comma** (matching PG): `FROM c, a JOIN b ON c.num=b.num`
fails loud `42P01` — an inner `JOIN`'s `ON`/`USING` reaches only its own chain, not a
sibling comma item (so `FROM t1, t2 JOIN t3 USING(num)`, all three carrying `num`, is
unambiguous). pglet resolves the `ON` at **eval time**, so the out-of-scope
reference surfaces once the inner join evaluates a row — an all-empty inner join returns
0 rows where PG rejects at plan time. A comparison in the `ON` condition runs the
plan-time coercion pass against the whole FROM clause's scope, so a bare literal against
a non-string column (`ON g.id = g2.id AND g2.id = '<uuid>'`) resolves rather than
raising `42883`. Because that scope covers every FROM relation rather than only the
join's own chain, resolution there is slightly more permissive than Postgres's —
costlessly, since coercion only inserts a cast and an out-of-chain reference is still
rejected (`42P01` qualified, `42703` bare, both matching Postgres). See `EDGES.md` row 3
and `docs/coercion.md` § Where the plan-time coercion pass reaches.

### Aggregates, GROUP BY, HAVING

`sum` over `smallint`/`integer` is an exact `int8` (loud `22003` past it); `sum(bigint)`,
and `sum`/`avg` over `numeric`, are **exact** `numeric`; and `avg` over `smallint`/
`integer`/`bigint` is the exact `numeric` quotient at 18.6's `select_div_scale` — `3`, `4`
and `6` average to `4.3333333333333333`, three `bigint` maxima sum to
`27670116110564327421` — as 18.6's `int8_avg` / `numeric_avg` answer. Where a row's
value is not of the argument's declared type — a branch pglet leaves unconverted
(`EDGES.md` row 152), or an operator over an enclosing query's column inside a
correlated subquery, which pglet declares at its other operand's type whatever the
column holds (row 2) —
an exact `sum`/`avg` reads an
integer of any width exactly, under an integer or a `numeric` declaration, and refuses
any other value `0A000` rather than skipping it. The `numeric` answers are given only where
the argument's integer type is **certain** — a table's column; a column of a derived
table, a `WITH` query, a `VALUES` list or a set operation whose defining expression is
itself certain; a `JOIN … USING` column merged from same-typed columns; a literal, a
cast, a scalar subquery whose own column is certain, a `date` less a `date`, and the
numeric-category arithmetic and the folds the coercion pass types over those, a `$N` or
an untyped `NULL` branch of a fold taking the fold's type (as an operand of arithmetic it
leaves the operator untyped)
([coercion.md](coercion.md) § A computed branch is typed where its type is certain). The pass decides it once and records it on the
aggregate, and every reader of the result type reads that record, so the two protocols'
descriptors and the accumulator agree on the aggregate's type. Over a relation's column
defined by a window function, a FROM `unnest` of an `ARRAY[…]` constructor, a
set-returning `unnest` in a select list, a bare reference to a `USING` merge of
different integer types, a recursive `WITH` query's column whose recursive term is an
untyped `NULL`, a `RETURNING` column of a data-modifying `WITH` query, a catalog view's
column, an enclosing query's column inside a correlated subquery (`(SELECT avg(t.i +
u.i) FROM u)`, and any fold over that subquery), a column a scalar subquery over one of
those defines, or a fold with one of those — the column, or a subquery over it — for a
branch or under one, `avg` stays `double precision` and `sum(bigint)` `bigint` —
measured members, listed in `EDGES.md` row 2 — and a cast (`avg(x::int)`) gives
PostgreSQL's answer. `AVG`
over `interval` is `interval` on both, and over `real`/`float8` is `float8` on both. **`sum(real)` accumulates in f32**,
not f64 — `pg_aggregate.dat` gives
it `aggtransfn => float4pl, aggtranstype => float4`, so every partial sum rounds to
`real` — ten `0.1::real`s sum to `1`, not f64's `1.0000000149011612` — and the value sent
is a `real`, matching the declared type. `avg(real)` is `double precision` on both
(Postgres's `float4_accum` transitions through float8), and `min`/`max`/`count` over
`real` agree with 18.6. A float `sum` is a chain of the `+` operator's steps
(`float4pl`/`float8pl`) from its first input — a sum of `-0`s is `-0`, and an overflow from
finite inputs is `22003` — and a float `avg` runs `float8_accum`, whose running sum of
squares is overflow-checked too (`avg` over `1e200` and `-1e200` is `22003` on both).
**A float or `interval` `sum`/`avg(DISTINCT …)` adds in sort order**: 18.6 sorts a
`DISTINCT` aggregate's input by the argument type's order and feeds the deduplicated values
in that order, and for those two types pglet adds them in that order too — a float's in
`float8_cmp_internal`'s order, `NaN` last — so `sum(DISTINCT x)` over `1e16`, `-1e16`, `1` is
`0` on both. An integer or `numeric` one is exact, so its order moves nothing; a float
aggregate over an expression on a column whose values carry mixed types still adds in
input order (`EDGES.md` row 152). A plain aggregate beside a `DISTINCT` one keeps input order where 18.6 may
presort it (`EDGES.md` row 297), and `min` over equal values of different spellings
answers the first where 18.6's scan answers the last (row 298). **`GROUP BY` groups by
the equality `DISTINCT` dedups by** — 18.6 groups and dedups by the type's one default
equality, so `{"a":1.0}` and `{"a":1}`, `ARRAY[1.0]` and `ARRAY[1.00]`, or
`ARRAY['1 mon'::interval]` and `ARRAY['30 days'::interval]` are one group — and a group
shows the spelling met first; groups come out in first-seen order, an order neither
engine promises. **An aggregate under an expression answers** —
`round(avg(x), 2)`, `CASE WHEN count(*) > 0 THEN … END`, `abs(sum(x))`, `concat('n=',
count(*))`, `jsonb_build_object('n', count(*))`, `greatest(max(a), 1)`, `HAVING
round(avg(b)) > 1`, `ORDER BY abs(sum(b))`: the grouping computes the aggregates and
evaluates the expressions above them through the ordinary scalar walk
([scope.md](scope.md) § A grouping query's aggregates are values the scalar walk reads by
node). The aggregates under an expression are computed before it is evaluated, whether
or not it reads their values, as 18.6 computes every aggregate that survives its
planner's constant folding — so `CASE WHEN count(*) > 0 THEN max(a) ELSE sum(1/b) END`
raises the `ELSE` sum's `22012`, and `coalesce(max(a), sum(1/b))` does too. An aggregate
is dropped with its branch only under a decider that is a constant by its shape — a
literal `true` / `false` / `NULL`, a comparison of two number literals (a simple
`CASE`'s operand against an arm's value included), `NOT` / `AND` / `OR` / an `IS` test
over those, and for a `COALESCE` an earlier number or boolean literal — so `CASE WHEN false THEN sum(1/b) END`, `CASE WHEN 1 = 1 THEN 0 ELSE sum(1/b)
END`, `coalesce(1, sum(1/b))` and `true OR sum(1/b) > 0` answer. A decider 18.6 folds
that is none of those keeps its aggregates here, which is loud where 18.6 answers and
the aggregate fails: `CASE WHEN length('abc') = 3 THEN 0 ELSE sum(1/b) END`, a bound
`$1` condition, `NULL::int + sum(1/b)`, `sum(1/b) > 0 OR true` (`EDGES.md` row 89's
residuals). An aggregate over a column or a literal keeps its operand's type check
where it is dropped, and where no group is computed — `true OR sum(text_col) > 0` is
`42883` as on 18.6, and so is `sum(text_col)` over a grouping that forms no group; an
operand that is computed, a bare `$N` or a `DEFAULT` is not checked there, and `sum` /
`avg` over `time` is refused (rows 89 and 307). A subquery in
a grouping query's expressions is still `0A000` (row 89), and so is an aggregate in a
position the grouping does not evaluate — a window function's `ORDER BY` or `PARTITION
BY`, a set-returning `unnest(ARRAY[max(x)])`, a `RETURNING` (row 4) — and in a `SELECT`
with no `FROM` (`SELECT count(*)`, `1` on 18.6; row 89). An
aggregate whose argument holds another aggregate — `avg(avg(i))`, `sum(b + max(b))` — is
refused at plan time with 18.6's `42803 aggregate function calls cannot be nested`. In a
top-level `SELECT` that comes after a missing column in its own argument or an earlier
select-list entry and before a later clause's error, as 18.6 orders them; inside a
subquery or a DML statement it outranks errors 18.6 raises first, and ahead of an outer
signature 18.6 cannot resolve (`avg(avg(tx))`) it answers where 18.6 says `42883`
(`EDGES.md` rows 302 and 2). pglet reads every aggregate inside a subquery as that
subquery's: it has no outer-level aggregates, so `(SELECT max(t.tx) FROM u)` over an
outer `t` answers once per outer row where 18.6 aggregates the outer query, and a pair
18.6 places at two levels is refused (row 303). A
non-grouped/non-aggregated projected column **fails loud at plan time** (`42803`), with
Postgres's primary-key functional-dependency case modeled (`SELECT pk, other … GROUP BY
pk` is allowed); validated over a single base table, lenient on JOINs — and
**qualifier-aware**, so a key naming an *enclosing* level's relation (`SELECT o.id,
(SELECT o.id FROM i … GROUP BY i.k)`) does not read as a local grouping key. **`GROUP
BY` keys take Postgres's `findTargetlistEntrySQL92` rules**: a bare integer is a 1-based
**ordinal** into the wildcard-expanded select list and resolves to that entry's
*expression* (`SELECT x+1 AS e … GROUP BY 1` groups by `x+1`; `0` or out of range →
`42P10`); any other bare constant → `42601`; a bare identifier prefers a **FROM-clause
column** over a same-named output alias — the precheck Postgres takes for `GROUP BY`
only, making the precedence the **inverse** of `ORDER BY`/`DISTINCT ON`, where the alias
wins — and only then searches the target list, where two entries carrying the name with
*different* expressions are `42702` (equal expressions are accepted, and a no-op cast is
stripped first, mirroring `coerce_type`'s elision; a widening cast still differs). A
matched entry containing an aggregate is `42803`, a window function `42P20` — the split
`checkTargetlistEntrySQL92` makes. `GROUP BY +1` is an ordinary constant key, not an
ordinal (see the SELECT row). An aggregated `SELECT *` expands to the table's columns
instead of reporting `42703` on the star. **Operand types are gated** — `SUM`/`AVG`
accept the numeric family plus `interval` (both exact for `interval`), anything else
`42883`; `MIN`/`MAX` accept every sortable type except `bool`, `jsonb`, and `uuid`,
which Postgres's aggregate catalog omits. The check prefers the argument's **static**
type where it is certainly knowable (a column, literal, cast, or bound param), matching
Postgres's parse-time resolution — so `min(bool_col)` errors on an empty table too,
rather than passing on a thin seed and failing once populated. A `GROUP BY <ordinal>`
under a JOIN groups by the target entry the ordinal names, like Postgres (the JOIN
grouping-*validity* check is skipped, so an ungrouped joined column is accepted rather
than `42803`). `HAVING` and `GROUP BY` run the plan-time coercion pass
against the same scope as the projection — resolved **together**, since the grouping
rule compares the three by structural equality and coercing one alone raises a spurious
`42803`. So `HAVING id = '<uuid>'` against a `uuid` column resolves rather than raising
`42883`. See `EDGES.md` row 3 and `docs/coercion.md` § Where the plan-time coercion pass
reaches. **Absent (fail loud):** `array_agg`, `string_agg`, `json_agg` / `jsonb_agg`,
`jsonb_object_agg`, `bool_and` / `bool_or` / `every`, `stddev` and the ordered-set
aggregates (`percentile_cont`, `mode`) are `0A000`, and an aggregate's `FILTER (WHERE …)`
or its own `ORDER BY` is `42601` (`EDGES.md` row 44).

### `COALESCE` / `CASE` common types

Both resolve Postgres's `select_common_type` over **all** branches (category +
preferred-type + implicit-cast rules) and **cast every branch** to the result, so the
declared type and the returned value agree on every row. That is a statement about a
branch set the pass **can** resolve. A computed branch is typed where its result is in the
numeric category and certain (`i + 0`, `f * 2`, `length(s)`, `abs(x)`, `EXTRACT`), so
`COALESCE(i, f + 0)` is `double precision` and answers `2.5` as PostgreSQL does
([coercion.md](coercion.md) § A computed branch is typed where its type is certain); a
`$N` or an untyped `NULL` branch takes the set's target, and the parameter is described
at it; a set
holding a computed branch outside those shapes keeps the target its written branches
give — or `double precision`, where one of its computed branches is certainly that and
the rest are numeric —
and one whose branches are *all* invisible to `concrete_branch_type` (a bare `NULL` beside
such an expression) yields no target and is left untouched, so `COALESCE`/`CASE`/`ARRAY` take the *first* known branch where this
row's fold would take the common type of all of them (`EDGES.md` row 65) — a branch the
two declared-type walkers agree on (row 54's multi-operand seam is closed). Three
consequences worth knowing, each measured against the
oracle: the fold **widens** rather than narrowing (`COALESCE(int8, int4)` is `bigint`
either way); **`float4` beats `int` and `numeric`** here (`COALESCE(int4, real)` is
`real` — `int4 → float4` is an implicit cast and the reverse is not), which is the
*opposite* of the arithmetic promotion (`int4 + real` is `double precision`) and the two
rules are deliberately separate; and **order is observable** when two types are mutually
implicit — `COALESCE(varchar, text)` is `varchar` while the reverse is `text`, and
**`CASE` feeds its `ELSE` branch in first** (`parse_expr.c` prepends `defresult`, "the
default result is considered the most significant type"), so `CASE … THEN text ELSE
varchar` is `varchar` and the swap is `text`. A decimal literal is concrete
(`COALESCE(int_col, 1.5)` → `numeric`). No common type fails loud **over a branch set
the gate can resolve**: **`42804`** across type categories (`COALESCE(int_col,
text_col)`) and **`42846`** within one category where the conversion doesn't exist
(`COALESCE(date_col, time_col)`, `COALESCE(time_col, timestamp_col)`). The qualifier is the same one the row opens with, and **which
reader supplies it is not one answer**: the gate hands `concrete_branch_type` — plus the
bare-scalar-subquery arm the pass cannot use, since the pass would have to *cast* to
what it reads (`docs/coercion.md`) — to all five constructs it covers, while the pass
reads `concrete_branch_type` only for `COALESCE`/`CASE`/`ARRAY` and `arg_known_type` for
`greatest`/`least`/`nullif`, and the second lacks the `Array` and `Alias` arms the first
carries. Where the two agree — the `COALESCE` family —
`select_common_type` folds only the branches that reader resolves, so **any** branch
invisible to it (any computed expression, and also an `INTERVAL` literal, parsed here to
an `Expr::Bound`) is dropped from the fold and never compared against a sibling, whether
or not the set has a concrete member. Where they differ — a variadic over one of those
two arms — the pass folds the concrete sibling alone and casts the argument it cannot
see to that target, and the gate then reads a `Cast` at an agreeing type:
`greatest(ARRAY[1,2], tx)` answers `{1,2}` and `nullif(set_config('x', tx, false), i)`
answers NULL where 18.6 refuses `42804`/`42883`, while the same `int[]` reached through
an arm both readers carry (`greatest(a, tx)` over an `INT[]` column, `greatest(a::int[],
tx)`) is `42804` on both. With **no** concrete member the fold is `[None, None]`, the
answer `Ok(None)`, and nothing is cast either — so `COALESCE(length('ab'), upper('7'))`
answers int4 `2` and `ARRAY[length('ab'), upper('7')]` answers `_int4` `{2,7}` where
18.6 refuses `42804`, and where a short-circuit parts the declaring branch from the
evaluated one the declared type and the value part with it, `CASE WHEN false THEN
length('ab') ELSE upper('x') END` answering `X` under `23`. With **a** concrete member
the pass casts the invisible branch to the siblings' target, which keeps the declaration
and the value together and leaves the missing refusal alone in far more ordinary
spellings — `COALESCE(i, upper(tx))` answers `7`, `COALESCE(upper('7'), 1)` answers int4
`7`, and `COALESCE(upper('x'), 1)` is `22P02` at eval where the cast fails, all against
18.6's `42804`. The control is a set with no invisible branch (`COALESCE(i, tx)`,
`42804` on both), and a concrete pair still fires the gate beside an invisible branch
(`COALESCE(i, tx, upper('7'))`), which is what makes the rule per-branch rather than
per-set. That is `EDGES.md` row **65**, and it closes with the same widening as the fold
above. The check reaches **every clause that plans an expression**: projection, `WHERE`,
`JOIN … ON`, `GROUP BY`, `ORDER BY`, `HAVING` (whose aggregate branches type by their
argument), the `INSERT … VALUES` list, the `UPDATE … SET` right-hand side, and **stored
expressions** (a `CHECK`/index expression with no common type is rejected at DDL, where
Postgres rejects a genuinely-unresolvable one). Extending the check to stored
expressions also **carries pglet's known literal-as-text false rejection** into these
constructs: a stored expression is checked *as parsed*, without the planner's
literal-coercion pass, so a bare literal types as `text` rather than staying Postgres's
`unknown` — so `CHECK (COALESCE(n, '5') > 0)`, `CHECK (CASE … ELSE d END >
'1999-01-01')`, and `CREATE INDEX ((COALESCE(i, '7')))`, like `CHECK (n = '5')`, are
rejected here and accepted there. Same cause, same workaround (the explicit cast), same
real fix — resolving a stored expression against its table's scope before recording it.
It runs in **one bottom-up left-to-right walk** with the operand rulebook rather than as
a second pass, so the *leftmost* error wins whichever family it belongs to: `d LIKE 'x%'
AND COALESCE(k, x) IS NULL` is the `LIKE`'s `42883` and the reverse spelling is the
`COALESCE`'s `42804`, matching Postgres both ways. The
`LIKE`/`||`/`->`/containment/`starts_with` operand gates run in that walk, at plan time,
so they fire on an empty table as Postgres does. `JOIN … USING (k)` over mismatched `k`
reports the operator (`42883`) because the USING columns are gated before the merged
`COALESCE`, and an `UPDATE` whose `WHERE` **and** `SET` are both ill-typed reports the
`WHERE` (`analyze.c` transforms it first). **Arrays unify element-wise**:
`COALESCE(int_array, bigint_array)` is `bigint[]` because `int4 → int8` is implicit,
exactly as Postgres's `COERCION_PATH_ARRAYCOERCE` resolves it — so `ARRAY[1] UNION
ARRAY[1::bigint]` is `bigint[]` too, on both build profiles.
**`GREATEST`/`LEAST` are checked statically too**, which matters more for them than for
the others: their eval-time fold drops NULL arguments first, so unchecked,
`GREATEST(int_col, text_col)` would answer `7` over a `(7, NULL)` row and nothing over an
empty table, where Postgres rejects the query outright — and a `CHECK (GREATEST(t, i) IS
NOT NULL)` would be recorded and then fail every write supplying both columns

### `IN` / `NOT IN` list common type

a plain column of a base table against two or more written constants (literals, or a
cast directly over one) is compared at 18.6's `select_common_type` over the column and
those items (`transformAExprIn`): a string literal is read as it and an item of another
type is cast to it where it is `real` / `double precision` — so `real_col IN (0.1, 1.5)`
matches the stored `0.1` and `i IN ('1.0', 2.5)` reads the string as `numeric`. A single
item and a set with no common type are an `=` each, as there. **Compared item by item
still** (`EDGES.md` row 280, silent): a list under any other left operand (an
expression, a constant, a subquery, a derived table's or an enclosing query's column, an
`UPDATE … FROM` source's column), a list holding a column, a subquery, a parameter or a
computed item (`real_col IN ($1, 0.1)` misses the `0.1` row; `real_col IN ($1,
0.1::real)` finds it), and a list inside a stored expression (`CHECK`, `DEFAULT`,
partial-index predicate, expression index key). `docs/coercion.md` § An `IN` list is
compared at one type; `tests/fixtures/in_list/`

### Subqueries (scalar/IN/derived)

correlated subqueries supported (eval-time outer-scope resolution via `EvalEnv` +
alias-aware scoping); the spine's correlated EXISTS run end-to-end. `IN (SELECT …)`
works in a `DELETE`/`UPDATE` `WHERE` too — the spine's subquery-keyed eviction `DELETE …
WHERE id IN (SELECT id … ORDER BY … OFFSET $n)`. **Absent (fail loud):** `= ANY (SELECT
…)` / `ALL (SELECT …)` (`42601`), a `WITH …` or `VALUES …` body as a scalar or `IN`
subquery (`0A000`)

### CTEs (WITH)

a CTE's `name(cols)` **column list** names the relation's columns on every reader —
positionally replacing the body's names, filling from the body where the list is short,
`42P10 WITH query "c" has N columns available but M columns specified` where it is long
(raised while the `WITH` clause is analysed, so an **unreferenced** CTE raises too),
never changing a type, and leaving the body's own names out of scope. A duplicated
*list* name is accepted (`gram.y` has no uniqueness rule) and `42702` only on reference;
a duplicated **CTE** name is `42712 WITH query name "c" specified more than once`, gated
in declaration order. `MATERIALIZED` / `NOT
MATERIALIZED` are accepted and change nothing: pglet evaluates a read-only CTE that does not
reference itself at each reference whatever the keyword says, so a body calling `random()`,
`gen_random_uuid()` or `clock_timestamp()` answers afresh at each reference where 18.6
evaluates it once — `MATERIALIZED` included (`EDGES.md` row 316).
**Divergence:** 18.6 analyses a CTE body *before* the arity check, so `WITH c(w,x) AS
(SELECT 1 FROM nosuchtbl) SELECT 1` is `42P01` there and this `42P10` here — pglet does
not analyse an unreferenced body at all (`EDGES.md`), so the ordering follows that gap.
A **bare `VALUES`** CTE body is `42601` (a `VALUES` statement is `0A000` — the
absent-but-loud row). The duplicate verdict and every `42P10` are **recorded and fired
after the whole list has parsed**, duplicate first — `transformWithClause` before
`analyzeCTE` — so a `42712` outranks an earlier declaration's arity error and a body's
own **syntax** error outranks both, as on 18.6. What is left, under a plain `WITH`, is a
body error pglet raises *during* the walk (a plan-time `42703`), which pre-empts the
`42712` 18.6 gives — a `WITH RECURSIVE` list re-parses for the syntax error and lets the
deferred verdicts outrank the rest: `EDGES.md` row 80. A **`RETURNING`-less**
data-modifying body that the outer query references is **accepted and its write
performed** (the reference yields a zero-column relation) where 18.6 refuses the whole
statement `0A000 WITH query "c" does not have a RETURNING clause` and writes nothing —
silent, `EDGES.md` row 61. **Data-modifying CTEs** (`WITH x AS (UPDATE/DELETE/INSERT …
RETURNING) … FROM x`) run once, their `RETURNING` rows exposed as the relation (a
`Plan::With` materializes them into the `EvalEnv`; a `CteScan` reads them) — including
`UPDATE … FROM other` bodies. **`WITH RECURSIVE`** resolves its list in **dependency
order** — every name is visible to every body, so a CTE reads one declared after it, and
a cycle of more than one item is `0A000 mutual recursion between WITH items is not
implemented` — and each genuinely recursive CTE, any number of them, is an entry of the
`Plan::With`, which stays on the lock-free MVCC path when nothing in it writes. Each
runs the semi-naive fixed-point loop: the working table binds only the previous
iteration's new rows, beside the finished relations of the entries before it; `UNION`
dedups whole-row against all accumulated (cyclic graphs terminate) / `UNION ALL` keeps
all with a 100k-iteration cap (`54000`), the non-recursive term's types are
authoritative (recursive rows cast per iteration; mismatch `42804`; an `unknown` literal
converted to an enum seed, `22P02` for a non-member), and the CTE name shadows a
same-named real table. A CTE that does not name itself is an ordinary CTE,
data-modifying ones included; an `INTERSECT` binds into the recursive term, and a body
wrapped whole in further parentheses is the same query. The statement after a `WITH`
list is a query or a data-modifying statement, as `gram.y` has it — any other is
`42601`, as are the `TABLE` and `MERGE` spellings pglet parses nowhere. Postgres's
`checkWellFormedRecursion` verdicts on a self-ref in the non-recursive term, more than
once, on an outer join's nullable side or inside a sub-link (`42P19`), a data-modifying
CTE naming itself (`42P19`), and `ORDER BY`/`OFFSET`/`LIMIT`/`FOR UPDATE` over the
recursive query (`0A000`, once those clauses have passed their grammar's `42601`) are
judged from a token pre-scan of the list before any body is analysed; the `… within
INTERSECT` / `… within EXCEPT` verdicts of a set operation inside the recursive term
(`42P19`) are judged as that body parses, behind the bodies resolved before it
(`EDGES.md` row 80). **Divergences (loud):** refused `0A000` before anything runs — a
body that writes beside a data-modifying CTE, a recursive CTE resolved after one, and
under `WITH RECURSIVE` a second data-modifying CTE or a body naming a table one can
write, itself or through a foreign key's `CASCADE` / `SET NULL`, each ahead of the
data-modifying CTE's own execution error (`EDGES.md` rows 160 and 162); a recursive CTE
whose non-recursive term is itself a set operation is `42P19`; a recursive CTE 18.6
skips or stops early — unreferenced, or read under a `LIMIT` — is evaluated to its fixed
point here, so a runaway one is `54000` and an unreferenced one that raises raises where
18.6 answers; a body leading with a parenthesized query that holds its whole `UNION`,
followed by a clause over it, is an error of that query's parse; and a clause over the
recursive query 18.6's grammar reads and pglet does not (`USING OPERATOR(…)`) is refused
under another code (`EDGES.md` row 161). A clause a parenthesized query already carries
is `42601` under either spelling, as `insertSelectOptions` refuses it — behind an
analysis error inside the parentheses or, under a plain `WITH`, in an earlier item, each
of which pglet reports where 18.6's raw parse reports the `42601` (`EDGES.md` rows 30
and 80). **Divergences (silent) a `WITH` adds, plain `WITH` only:** a part of the
statement that reads a table a data-modifying CTE writes sees the write, and several
data-modifying CTEs run in declaration order where 18.6's pull-driven order differs —
which decides which of two writers of one row takes it, and which serial values rows
take (`EDGES.md` rows 160 and 162). A data-modifying CTE also carries its statement's
own divergences, under either spelling — among them a `RETURNING` subquery reading the
statement's own write (`EDGES.md` row 164, a read under `WITH RECURSIVE`) and a key an
`ON DELETE SET NULL` nulls (row 163, a write). A CTE body carries the divergences of the
query it is, under either spelling — among them a parenthesized query's clause stacking,
a read, and a write where a data-modifying statement reads the CTE (`EDGES.md` row 167).
A FROM-clause derived-table self-ref is allowed (matching PG)

### Window functions

**ranking only** — `ROW_NUMBER()` / `RANK()` / `DENSE_RANK()` over `[PARTITION BY …]
[ORDER BY …]` (honoring `NULLS FIRST/LAST`). No window frames (`ROWS`/`RANGE`), no
offset/value functions (`lag`/`lead`/`ntile`/`first_value`/…), and no
**aggregate-over-window** (`SUM(x) OVER …`) — each unsupported (fail loud). An argument list
is refused as 18.6 refuses it: `row_number(1) OVER ()` is `42883`, `rank(a) OVER (…)` /
`dense_rank(a) OVER (…)` `42809 WITHIN GROUP is required …`, and `row_number(*) OVER ()`
answers on both (`EDGES.md` row 53). A trailing
`ORDER BY` key the window's output cannot answer — a qualified or nested name, an
expression, a window function — rides the Window as a Postgres **resjunk** column, the
Sort being unable to go below it

### Transactions (BEGIN/COMMIT/ROLLBACK/SAVEPOINT)

one writer at a time: a block takes the writer at its first write — re-based there on
what is committed — and holds it until it ends, and another connection's write waits for
it, bounded by `lock_timeout` (pglet's default `1s`; `55P03` past it); the embedded
`Database`, which cannot wait, refuses `55P03` instead, and refuses another logical
connection's transaction control while a block is open; the isolation level is honored —
a `REPEATABLE READ` / `SERIALIZABLE` block keeps its snapshot — noted by its first
statement that takes one (a planned statement other than transaction control, or a
`COPY`): that statement's start where it is the block's first write, else `BEGIN`'s —
and its first write is `40001` if the store moved past it (`EDGES.md` row 232) — and the
settings pglet acts on (`lock_timeout`, `default_transaction_isolation`) are
transactional as 18.6's GUCs are (`application_name` is not, row 236); no nested
`BEGIN`. **Savepoints are in** — `SAVEPOINT n`, `ROLLBACK TO [SAVEPOINT] n`, `RELEASE
[SAVEPOINT] n`, with Postgres's rules: `25P01` outside a block, `3B001` for a name the
block does not hold (and the block then aborts), a repeated name stacks and the most
recent wins, `ROLLBACK TO` is repeatable and destroys only the savepoints established
after it, and it is the one statement an aborted block admits. The **aborted block** is
one `pglet::TransactionSession` every leg drives — native server, wasm bindings and
embed facade — so a failed statement aborts the block on all of them — so does an error
at any of the native server's extended-protocol phases, a refused `Bind` or an unknown
statement at `Describe` included, and a binding failure at the embed facade's
`execute_bound` — the next statement is `25P02`, and the `COMMIT` reports tag `ROLLBACK`
and publishes nothing (the failed statement's partial effects included). It is gated at
Postgres's five phases (simple query, `Parse`, `Bind`, `Describe` ×2, `Execute`), and
the exit classifier reads the **lexer**, so `/* c */ COMMIT` ends the block. A `COMMIT`
that fails — the catalog's encoding refused `54001` — rolls the block back and ends it,
as a failing `COMMIT` does on 18.6; a `COMMIT` that meets a refused write ends rolled
back, or fails the store (`PLAN.md` § 3). Recorded, open: `EDGES.md` rows 211 and 216,
and the writer's own costs against 18.6 — unrelated writes serialize (228), the embed
refuses where it cannot wait (229), an aborted block holds the writer (230), a client
can wait on its own block (231), the coarse `40001` (232), no keepalive (233), an
unwinding holder (234); `pg_advisory_xact_lock` — both overloads, answered as a
top-level item of a `SELECT` with no `FROM` — takes the writer, so every key is one lock
and every write contends with it (250); a statement whose plan fails on the block's
uncommitted catalog waits for it on the native server (the embed refuses it `55P03`,
229), keyed on the plan rather than on 18.6's locks (238, 240), and a read whose plan
succeeds reads that catalog (221). **Divergences:** a multi-statement simple-`Query`
message runs per-statement autocommit — Postgres wraps it in one implicit transaction,
so an error mid-batch rolls the whole message back there but leaves earlier statements
committed here (`EDGES.md` rows 68 and 175), and an extended-protocol pipeline up to one
`Sync` does the same (`EDGES.md` row 141); an **inline** mode clause is parsed —
`gram.y`'s `transaction_mode_list`, any order and combination, comma optional — with
`ISOLATION LEVEL …` carried to the block, `[NOT] DEFERRABLE` accepted and inert and
`READ ONLY` refused loud (`0A000`, matching the `SET TRANSACTION READ ONLY` spelling),
since pglet has no read-only block and accepting it would let the write 18.6 answers
`25006` through silently; `START TRANSACTION` still reports command tag `BEGIN` where
18.6 reports `START TRANSACTION`; `AND CHAIN` is absent, and so are `ABORT` / `PREPARE
TRANSACTION`, which 18.6's aborted block admits and pglet's parser does not model. A
**nested `BEGIN`** is a no-op on both legs, matching 18.6's outcome, but 18.6 also emits
`WARNING: there is already a transaction in progress` and neither leg does (measured);
the npm adapter is deliberately stricter still and answers its own `25001` for a raw
overlapping `BEGIN`, since a single connection cannot give that caller its own
transaction — its ergonomic `transaction()` under an already-open block runs as a
savepoint instead. Every rollback — `ROLLBACK`, `ROLLBACK TO`, a caught `DO …
EXCEPTION`, a refused or aborted `COMMIT`, a disconnect — restores the level's catalog
and storage directory unconditionally, since nothing but the block writes while it holds
the writer; a block that never wrote restores nothing, keeping every other connection's
commits. Two residual aborted-block answers are in `EDGES.md`: a comment-only simple
query, and a parse-*analysis* error reported as `25P02` where 18.6 reports its own
`42601`

### JSONB `->`/`->>`, `@>`, key/path ops, `jsonb_build_object`

full `@>` containment + variadic `jsonb_build_object` (scan-based, or GIN-accelerated
for a top-level `data @> <const>` conjunct); **key/path operators** `?` (key/element
exists — incl. PG's top-level-scalar-string quirk), `?|`/`?&` (any/all keys over
`text[]`, NULL array elements skipped), `#>` (path → jsonb) / `#>>` (path → text) — a
NULL path element propagates to NULL, an empty path returns the whole doc, an array
index is strict/negative-from-end, a scalar mid-path → NULL. Their operands are gated
(`planner::legality`'s `json_operator_legality`): a pair 18.6 resolves no operator for
is `42883` except at the measured departures: a `text`/`varchar` operand is
accepted — parsed as JSON on the left, as an array literal on the right of
`?|`/`?&`/`#>`/`#>>` — so text that parses answers (`'{"a":1}'::text ? 'a'` is `t` here,
`42883` there) and text that does not is `22P02` on the left and `42846` on the right
(`'{}'::jsonb ?| 'a'::text`), both `42883` there; some untyped `NULL` pairs answer NULL
where 18.6 refuses `42725` (`EDGES.md` row 187); and an **enum** right operand is shown
as `text` and accepted (`j ? m` is `t` here, `42883` there — row 190), while an enum on
the left is parsed as JSON like `text`, so a label that is not JSON is `22P02` against
`42883`. `->`/`->>` take a **key operand Postgres registers** — `text` and `integer` (an
`int2` widens; `int8`, `numeric`, `float8`, and `bool` have no `(jsonb, …)` signature
and fail `42883`) — and a negative array index counts from the end. A bare `$N` key
infers as `text`, matching Postgres's `(jsonb, text)` preference; use `$N::int` for the
index form. A bare `$N`/string-literal right operand types/parses as `text`/`text[]`.
The jsonpath language (`@?`/`@@`) stays fail-loud (scope/size). **Numbers are exact** —
stored as `PgNumeric` (not `f64`), so a big integer / long decimal round-trips
losslessly and two beyond-`2^53` numbers stay distinct in `ORDER BY`/`DISTINCT`; number
equality is scale-insensitive (`1.0` = `1.00`, matching PG). **Duplicate object keys**
dedup last-wins (`{"a":1,"a":2}` → `{"a":2}`, matching `jsonb_in`), and a
`Value::Numeric`→jsonb becomes a JSON number (`NaN`/`±Infinity` → the JSON string
`"NaN"`/`"Infinity"`, per PG). Number input enforces the strict **JSON grammar**
(leading zeros / bare trailing-or-leading dot / leading `+` → `22P02`); a grammar-valid
number past the numeric weight/scale cap → `22003` (both `::jsonb` and `::numeric`).
**The reader refuses what 18.6's JSON lexer refuses, and the renderer escapes a key as
it escapes a value.** A raw control character from `0x01` to `0x1F` inside a JSON string
is `22P02`, where 18.6's `json_lex_string` raises it, and a raw NUL — which the wire
doors answer at the encoding scan before the reader is reached, leaving the in-process
ones — is `22021`, the code both engines were measured to give one. Whitespace is
exactly space, tab, LF and CR at every position the reader skips it, so any other
character standing where a token starts is `22P02`. Which error a string earns is its
**position**: the raw-character gate is interleaved with the escape decoding, so
`"\u0000a<TAB>b"` is `22P05` and `"a<TAB>b\u0000"` is `22P02`, and a string's body is
decoded before trailing junk is refused, so `["\u0000"<FF>]` keeps its `22P05`. An
object key goes through the one `escape_json_string` a string value goes through, so the
key's text is 18.6's at every renderer — the `::text`/`::varchar` casts, the value at
the wire, `->>`/`#>>` over a container, and the dump's `COPY` payload, whose restore
loads on both legs. **A binary-format `jsonb` parameter's first byte is its version**,
read unconditionally as `jsonb_recv` reads it, and only `1` is supported — anything else
is `XX000 unsupported jsonb version number <n>` over that byte's unsigned value, ahead
of the server-encoding scan. A `jsonb[]` element takes the same gate where the wire
array header names `jsonb`; the header's element OID is checked against the parameter's
own element type only where it names an array type pglet recognises (`42804`), so one
naming another scalar type pglet recognises decodes its elements as that type instead
and never reaches the gate, and one naming an unrecognised OID falls back to the
parameter's element type and does — which 18.6 matches at or above its built-in-OID
floor and refuses `42804` below it (`docs/param_binding.md`, `EDGES.md` row 140).
**`@>`/`<@` implement Postgres's full `JsonbDeepContains`** — object key/value
containment, **array subset** with membership (order and multiplicity irrelevant),
nested containers recursively, and the documented top-level exception that *an array may
contain a primitive* (`'["a","b"]' @> '"a"'`), which is one-directional (a scalar never
contains an array) and does **not** apply nested (`'{"a":[1,2]}' @> '{"a":1}'` is false,
since an object value's type must match). A container rhs element matches only a
container lhs element; kinds never cross. The GIN path stays sound —
`gin_decompose` is lossy but a superset, so the index still only picks candidates and
this is the exact recheck. An **uncast** literal operand (`data @> '{"k":1}'`, `tags @>
'{a}'`) resolves to the other operand's type through the plan-time coercion pass, as
Postgres resolves it from the operator signature. The anchor is any expression the pass
can type from the column scope plus the node — a column, a cast, a `->`/`#>` result, a
`jsonb_build_object(…)` call — not just a bare column, so `'{"a":1}'::jsonb @>
'{"a":1}'` works as readily as the column form; a scalar **subquery** operand is still
unresolved (it would need the sub-plan's output type) and fails loud. A non-container
operand is `42883` (`operator does not exist: text @> text`) — Postgres registers
containment over `jsonb @> jsonb` and `anyarray @> anyarray` only, and resolves
`anyarray` per operand rather than unifying, so `integer[] @> bigint[]` is `42883` too —
that refusal reads pglet's array type, and an `ARRAY[…]` over an enum is `text[]` here,
so `ARRAY[m] @> ARRAY[t]` is `t` where 18.6 is `42883 … e1[] @> text[]` (`EDGES.md` row
190). **Object key order is canonical and semantically insignificant**, as in Postgres:
every jsonb value is built through one canonicalization (keys deduplicated last-wins,
then sorted **by length, then bytewise** — `lengthCompareJsonbString`, so
`{"b":1,"aa":2,"c":3}` renders `{"b": 1, "c": 3, "aa": 2}`), so `'{"a":1,"b":2}' =
'{"b":2,"a":1}'` and `DISTINCT`/`GROUP BY`/join dedup fold them together.
`jsonb_build_object` goes through the same canonicalization, so it dedups (`{"a": 2}`)
like every other jsonb producer

### `json` (as distinct from `jsonb`)

**`42704`** at a column, `ADD COLUMN`, `CAST`/`::`, a typed constant and `json[]`, never
an alias for `jsonb`. 18.6 keeps the text verbatim (key order, whitespace,
duplicate keys), which no `jsonb` value holds; a text-preserving `json` stays unbuilt

### ON CONFLICT

a *specified* conflict target must match a unique/PK index (`42P10` if none) and is then
probed; the match is **set equality** on the key parts, as `infer_arbiter_indexes` makes
it — the plain columns compared as a set and the key expressions by membership both
ways, so order is irrelevant and a repeat collapses on either half, while a subset or a
superset still refuses (`docs/indexes.md`); a target-less `DO NOTHING` arbitrates
against every unique constraint; `DO NOTHING` / `DO UPDATE … [WHERE]` with `EXCLUDED`. A
`DO UPDATE` whose probe finds a row the same statement already inserted or updated is
`21000 ON CONFLICT DO UPDATE command cannot affect row a second time`, with 18.6's hint
— decided by the row's identity, not its key, and ahead of the action's `WHERE`
(`EDGES.md` row 247). Expression/partial-index targets supported; `ON CONSTRAINT name`
unsupported. A `DO UPDATE` action puts **both** the target relation and `excluded` in
scope, per Postgres — and since `excluded` carries the target's whole shape, a **bare**
reference to any target column is ambiguous (`42702`) in the `SET` right-hand side and
the `WHERE` alike, so the increment idiom must be spelled `SET n = t.n + 1`. (The `SET`
target on the left is the column being assigned, not a row reference, and stays bare;
the arbiter target's own `WHERE` sees only the target relation, so bare names are fine
there.) The `DO UPDATE` action is lowered in the planner, after the row source and
before `RETURNING`, so its right-hand sides run the same plan-time resolve-and-gate
every other clause gets — an ill-typed one is rejected even when the statement hits **no
conflict** and the action never evaluates. **Name** resolution is on that footing too:
the action's namespace is complete by construction (the target plus `excluded`, no
enclosing level), so its scope is closed and an unresolvable `SET` target / RHS / action
`WHERE` is `42703` — or `42P01` for a qualifier naming neither relation — before any row
is looked at. The action's own checks run in PG's order: every RHS, then every target
name, then the action `WHERE`, then the `GENERATED ALWAYS` guard. The `SET` list's
**duplicate-target** refusal (`42601`) is not one of them — it is a rewriter check, so
it runs after every clause of the statement **pglet lowers**, fires whether or not a row
conflicts, and beats the arbiter inference (`docs/names.md`). What that position orders
it against is pglet's own pipeline and nothing wider: a verdict pglet reaches only at
execution — a row-source cell's implicit conversion, a `VALUES` cell's bare name — is
answered *after* this gate, so `INSERT INTO u VALUES (5, 'notanint', true) ON CONFLICT
(a) DO UPDATE SET b = 1, b = 2` is `22P02` on 18.6 and `42601` here (the `nosuchcol`
spelling `42703` there and `42601` here) — the residual `docs/names.md` § What the
refusals do not carry records, with the zero-row measurement that keeps the gate where it
is. What it does not beat here, and does on 18.6,
is that same `GENERATED ALWAYS` guard. The **arbiter target** is neither resolved nor
gated (see `EDGES.md`, *Statement-level error order*). **Divergence:** the arbiter's
partial-index `WHERE` is matched by structural **equality** against the candidate
index's own predicate (`index.predicate != target.predicate` declines), where
`infer_arbiter_indexes` requires only that the index's predicate be **implied** by it —
so a non-partial index, whose predicate is empty and therefore implied by anything,
matches there and is unreachable here: `CREATE TABLE t (id INT PRIMARY KEY, n BIGINT);
INSERT INTO t (id) VALUES (1) ON CONFLICT (id) WHERE n > 0 DO NOTHING` inserts the row
on 18.6 and is `42P10` here. Loud, and the `$N` spelling of the same statement types its
parameter `{bigint}` on both, so the divergence is the matcher's rather than the
clause's. A quoted `"EXCLUDED".c` and a `bogusrel.c` are `42P01` as on PG, and
`excluded.nosuch` is `42703`: `EvalEnv::resolve_outer_slot` (`executor/expr/env.rs`) stops
at the frame that owns the qualifier and declines where none does, so neither reaches the `excluded`
frame by bare column name. **Divergence (silent-wrong):** a target-less `DO UPDATE` is
*not* rejected — PG errors at parse (`42601`, "requires inference specification or
constraint name"), but pglet silently falls through to a plain INSERT (a non-conflicting
row is written as if the clause were absent; a conflicting row raises `23505`, not
`42601`). **Cross-connection:** on the native server a key another connection's block
holds pending makes the statement wait for that block (bounded by `lock_timeout`; the
embedded `Database` refuses it `55P03`), and it then arbitrates over what the block
committed — after a commit `DO UPDATE` updates the row that became visible and `DO
NOTHING` skips it, after a rollback each inserts — as 18.6 answers after its wait.
`EDGES.md` row 98 (closed), and `docs/storage.md` on why a constraint is decided against
a state no other connection is moving

### `CREATE`/`DROP INDEX CONCURRENTLY`

parsed in `gram.y`'s keyword position; decides one thing, `25001` inside a transaction
block (`Plan::forbidden_in_transaction_block`, asked by both hosts and by the `DO` body
and handler). Otherwise the index is built as the plain spelling builds it (measured:
the same `indexdef`) and blocks what the plain spelling blocks: pglet has no
*table*-scoped lock for the concurrent form to weaken, and a build holds the
process-wide schema + storage pair for its whole duration, so under either spelling an
unrelated table's `INSERT` and a `SELECT` from it each return only once the build has
finished (measured). 18.6, by its own `CREATE INDEX` documentation, "locks out writes
(but not reads)" on the **indexed** table for a plain build and takes no lock preventing
an insert, update or delete for a concurrent one — measured both ways: during a plain
build an unrelated table's `INSERT` runs straight through while the indexed table's
waits, and under `CONCURRENTLY` the indexed table's runs straight through too. So pglet
is the stricter engine here. A write staged before the build in another connection's
open block is not invisible to it: the build takes the writer, and on the native server
waits for that block (`EDGES.md` row 103, closed) where the embedded `Database` refuses
it `55P03`.

### `CREATE`/`DROP EXTENSION`

full option tail parsed; **no extension catalog** — the functions they stand for are
builtins here, so only the *name* is decided (`0A000` / `42704`, plus `3F000` for the
schema). Residuals: `docs/ddl.md` § Extensions.

### CREATE INDEX

built + maintained; the name is optional (`CREATE [UNIQUE] INDEX [CONCURRENTLY] ON t
(…)`, never behind `IF NOT EXISTS`) and an absent one is chosen as 18.6 chooses it —
`<table>_<key part names>_idx`, cut to 63 bytes, a digit stepping around a taken
relation name (`docs/names.md` § An unnamed index's name; an expression part's name can
differ, `EDGES.md` row 277); the planner uses it for equality point-reads (`WHERE col =
<value>`), range scans (`<`/`>`/`BETWEEN` → bounded probe), single-key `ORDER BY
[ASC|DESC] [LIMIT]` over a NOT-NULL column (`Sort` elision + scan early-stop), and
**GIN** `@>` containment (`USING gin`, partial + correlated probe) for a top-level
conjunct **and under an `OR`** (same-column union + scan-independent guards). A simple
`UPDATE` / `DELETE` collects its target rows through the same equality probe where the
rest of its `WHERE` cannot fail, in row order (`docs/indexes.md` § A write's target rows); a
range, a GIN containment and an `UPDATE … FROM` still scan. No
composite / `NULLS FIRST|LAST` index use yet. Index *use* is behind the default-on
`index_acceleration` feature; the stripped (`--no-default-features`) build drops it —
the index is still built, enforced, and introspected, and a full `Filter(Scan)` replaces
the probe (rows *intended* to be byte-identical). **Exact-or-decline** probing upholds
it against an *inexact* probe (a coerced probe value substitutes only when exactly
equivalent; otherwise the planner declines the index and a scan runs — a silent loss of
acceleration, never a wrong row): `docs/indexes.md` § Index probes are exact-or-decline.
A `float4`/`float8` key is keyed in 18.6's `float8_ops` order — `-0` with `0`, every NaN
as one key above `Infinity` — on every index route, a GIN over a float array and a
foreign key into a float key included (`docs/indexes.md` § A float key; fixtures
`float_key/`). A `jsonb` and an array key are keyed in 18.6's `jsonb_ops` / `array_ops`
order, the key's byte order being the comparator's — on each route that builds a B-tree
over one (a unique or primary key, a partial unique one included, and a composite or
expression key) and as a GIN element (`jsonb[]`); string order within one is by bytes
(`EDGES.md` row 43), and the one shape refused is a **non-unique B-tree over a single
bare `jsonb` or array column**, `0A000` at the DDL, since the catalog has no place to
record its method beside the GIN the same shape derives (`docs/indexes.md` § A container
key; `EDGES.md` row 25; fixtures `container_key/`). **A measured exception stands
against that byte-identity** — a *starved* query-position `DEFAULT`/`$N` leaf over an
ordinary indexed `INT` — stated in full, with its reachable spellings and its symptom
per backend, in `docs/indexes.md` § Build profiles (`EDGES.md` row 56). An index
**predicate** and **key expression** are type-checked at `CREATE` against the shared
operand rulebook, so a gated operator (`WHERE data->'k' LIKE 'x%'`) fails there rather
than leaving a table that accepts DDL and rejects every write. A build that fails anyway
— no DDL gate reads a composite key's second segment, an expression key's result type,
or a key expression that raises for a stored row (`(10 / a)` over a zero is `22012` on
both engines, from the build's entry sizing ahead of the storage build) — leaves
**nothing**: no catalog `Index`, no storage index def, no OID moved, the table still
writable, and the same on the constraint routes (`docs/ddl.md` § A failed index build).
**Divergences:** an **expression index** (`ON t(LOWER(x))`) is built + UNIQUE-enforced
but never used for query *acceleration* — `WHERE LOWER(x) = …` always full-scans
(correctness-safe, just unoptimized); an **enum key column** is built and
PK/UNIQUE-enforced but declined for query *acceleration* — the B-tree key encoder orders
a label's bytes where the engine orders by member position, so `single_column_index_for`
declines it for its equality, range and `ORDER BY` `Sort`-elision consumers alike and a
`Filter(Scan)` runs (`docs/enums.md`, `docs/indexes.md`); and `CREATE INDEX` **modifiers
that do not reach the rendering** — a per-key sort direction and `NULLS` placement (`a
DESC NULLS LAST`), and an operator class (`text_pattern_ops`, `jsonb_path_ops`), are
parsed and discarded, and `NULLS NOT DISTINCT` is kept and **enforced** (a second NULL
row is `23505`, as on 18.6) but absent from `pg_indexes.indexdef`; these are in the fuz
spine's own shipped schema (`EDGES.md` row 92). **Over-accepts (silent):** spellings
18.6's grammar refuses build here — a `public.`-qualified index name, `IF NOT EXISTS
CONCURRENTLY ON`, a qualified key column `(t.a)`, an expression key without its
parentheses, a key of unknown type `((NULL))` (`EDGES.md` row 284) — and a key
expression or predicate calling a function 18.6 does not mark `IMMUTABLE` (`now()`,
`concat`, `timestamptz::text`) builds where 18.6 answers `42P17` (`EDGES.md` row 285).
**Gaps (loud, measured):** `ON ONLY t`, a key's `COLLATE`, `INCLUDE (…)`, `WITH (…)` and
`TABLESPACE` are `42601`; `USING hash` is `0A000`; and an index named `if` is `42601`,
the word being read as `IF NOT EXISTS`'s

### Index entry size limits

every entry sized as 18.6's `index_form_tuple` lays it out: `54000` at the size field,
the B-tree and the GIN item limits, in 18.6's order; loud over-refusals in `EDGES.md`
rows 191, 192; silent residuals rows 95, 104; orders rows 92, 43, 166, 194

### NOT NULL / CHECK constraints

enforced at write time (`23502`/`23514`); CHECK stored as compiled `Expr` IR in the
catalog (postcard-persisted, cloned per statement — not re-parsed), NULL predicate
passes (SQL 3-valued). The stored predicate is **type-checked at `CREATE`/`ALTER`** — a
gated operator on wrong-typed operands (`CHECK (int_col LIKE 'x%')`) fails there, where
Postgres reports it, rather than making the table unwritable. The check is static
inference plus a representative-row probe, and then the **boolean-context gate** on the
predicate as a whole, where `cookConstraint` puts `coerce_to_boolean`: `CHECK (a)` over
an `INT` column is `42804 argument of CHECK` at DDL on both engines, and an unresolved
literal is converted there rather than refused (`CHECK ('t')` is accepted, `CHECK
('abc')` is `22P02`) — the *Boolean contexts* row carries that rule. Static inference is
a strong filter, not a proof, so a value-dependent error can still surface at write time
(as in Postgres). **Divergence (loud):** a quoted literal compared with a column of another
type inside the predicate — `CHECK (n > '0')` over an `int`, `CHECK (d > '2000-01-01')`
over a `date` — is `42883` at the DDL, where 18.6 reads the literal as the column's type and
accepts; cast the literal (`EDGES.md` row 196; the *`COALESCE` / `CASE`* row carries the
wider stored-expression form)

### PK / UNIQUE / FK constraints

enforced — UNIQUE/PK `23505` (single-column, composite, partial, `NULLS NOT DISTINCT`,
expression-keyed; a `CREATE UNIQUE INDEX` over rows that already duplicate is refused at
**build** time under the index's own key equality — `23505 could not create unique index
"x"`, `D: Key (a)=(1) is duplicated.`, 18.6's field set — so `numeric` `1.5`/`1.50`
duplicate there as on write, and a duplicate pending on **another** connection is
committed or gone by the time the build scans, since the `CREATE UNIQUE INDEX` waits for
that block on the native server and is refused `55P03` on the embed (`EDGES.md` row 103,
closed); per-row/immediate, no `DEFERRABLE`; and held against another connection's
write, pending or committed: a pending write makes the later writer wait for its block
on the native server and answer by its outcome (the embed refuses it `55P03`), and a
`READ COMMITTED` block's first write re-bases it on what is committed, so a key
committed since its `BEGIN` is refused `23505` (`EDGES.md` rows 98 and 218, closed) — at
`REPEATABLE READ` / `SERIALIZABLE` that write is `40001` instead (row 232)); FK `23503`
(child-side) + **both `ON DELETE` and `ON UPDATE` actions** (CASCADE / SET NULL /
RESTRICT / NO ACTION), **single AND composite (multi-column)** — composite is MATCH
SIMPLE (any NULL FK column skips), matches the referenced columns against a parent
PK/UNIQUE **covering key order-independently** (`REFERENCES parent(y,x)` against
`PK(x,y)` works), and tuple-matches for cascade/SET-NULL. `ON UPDATE` fires on a
referenced-key change — an `UPDATE`'s, or an `ON CONFLICT DO UPDATE`'s after its writes
and its `RETURNING` — is **transitive** from one `ON UPDATE` action to the next (queued
as 18.6's after-row triggers and fired in order, a key that moves twice moving its
children twice; a key an `ON DELETE SET NULL` nulls starts none, `EDGES.md` row 163),
assigns an action's value through the child column's assignment conversion (type and
modifier) as 18.6's action `UPDATE` does — a `bigint` child of an `int` key stores an `int8`,
an `int` child of a `numeric` key stores `2.6` as `3`, a `varchar(3)` child refuses `'abcdef'`
`22001` — converted once, when the action's turn comes and before its child rows are looked
up (`fk/update_cascade_bigint_child_of_int_key_unique_loud.sql`,
`fk/update_cascade_varchar_child_too_long_no_child_loud.sql` and their neighbours), and checks each child row an action rewrites
as 18.6 checks the row the action writes — NOT NULL, CHECK, each index entry.
The queue fires once the statement's rows are written, so a later row's own `23514` /
`23502` / `23505` / `54000` outranks every foreign-key refusal and a row's own key is
checked before the next row's actions run; a rewritten child queues its own actions and
checks behind them — a check of every key it holds where the statement or an earlier action
wrote the row, and of each key the action changed where neither did — `NO ACTION` refusing unless a parent row now holds the old key and
`RESTRICT` outright, and a check of a version an action has since rewritten is skipped.
The cycles measured — tables referencing each other's keys, a table's two keys each
referenced by its own column, a row that is its own parent — end with 18.6's answer
(`fk/update_cascade_*_cycle.sql`, `fk/update_self_ref_two_unique_keys_cascade.sql`,
`fk/update_self_ref_row_is_its_own_parent.sql`). An `ON CONFLICT DO UPDATE`
fires its queue after projecting `RETURNING`, so a `RETURNING` error outranks an earlier
row's `23503` / `23001`, a `RETURNING` subquery reads the children as they stood before
the actions, and a row the statement inserts under a vacated key satisfies `NO ACTION`; a
plain `UPDATE` or `DELETE` runs its foreign-key work before `RETURNING` (`EDGES.md` row
168), and inside a `WITH` the actions run when the `INSERT` finishes (row 160). A `DELETE`
plans its actions read-only before it removes a row, so a violation aborts it before a
child moves.
Parent-side **RESTRICT → `23001`**, **NO ACTION → `23503`** (matching Postgres for
delete and update). A FK whose referenced columns have no covering unique key → `42830`
at CREATE (single-column and composite alike); an arity mismatch → `42830` — a
written list's in the parser, ahead of every verdict past the parse and of an `IF [NOT]
EXISTS` skip (`EDGES.md` row 275); a column either list names that its table lacks →
`42703`, ahead of the covering key, and on `ADD COLUMN` a self-reference finds the added
column; a column pair 18.6 cannot implement through the referenced key's B-tree operator
family (`text` or `numeric` referencing `int`, `int4[]` referencing `int8[]`, an enum
beside `text` or another enum) → `42804 foreign key constraint "…" cannot be
implemented` with 18.6's `D`, at each door where 18.6 raises it (`EDGES.md` row 195,
closed); a `MATCH FULL`/`PARTIAL` clause → `0A000`. **Exotic non-spine gaps:** a
`DELETE`'s mixed RESTRICT+CASCADE fan-in (its RESTRICT / NO ACTION decision reads the
cascade closure built so far, in BFS order, where 18.6 runs a per-row RI-trigger queue),
the firing order of the keys referencing one
table where one was added by `ALTER TABLE` (row 198), a self-referencing or cyclic
CASCADE over a referencing row an earlier statement of the transaction wrote (silently,
`EDGES.md` row 165), an action's conversion refused over a key no child row references,
which 18.6 refuses only under a custom plan (loud, row 200), and an `INSERT`'s or
upsert's child-side check as the row is written (loud, row 166) each
diverge. Both foreign-key checks read a state no other connection is moving: a pending
write makes the other writer wait for its block on the native server (the embed refuses
it `55P03`), and a write committed since a block's `BEGIN` is in the state the block's
first write re-bases it on (`EDGES.md` rows 100 and 101, closed). **Undocumented
edges:** `NULLS NOT DISTINCT` works only via `CREATE [UNIQUE] INDEX` — the
**inline/table-level** `UNIQUE … NULLS NOT DISTINCT` constraint (PG 15+) isn't parsed
(fails loud); a `UNIQUE`/`PK` key over an **array or jsonb column** is sized
uncompressed, so one 18.6 holds only compressed is refused `54000` (`EDGES.md` row 191);
and an FK accepts a **partial** unique index as its covering key (PG requires
non-partial) — over-accepting at CREATE and then spuriously rejecting `23503` a child
row whose parent falls outside the partial predicate; a `REFERENCES parent` with no
explicit column list against a parent that has **no primary key** is `42830` where PG
says `42704` (`there is no unique constraint matching given keys` vs `there is no
primary key for referenced table` — loud on both, different codes; `EDGES.md` row 263).
**A table has at most one PRIMARY KEY**, checked at all three mint sites (`CREATE
TABLE`, `ADD CONSTRAINT`, `ADD COLUMN`) with Postgres's own `42P16` — *not* a name
collision, since the two keys may be validly named, and *not* merged even when
identical. Two `UNIQUE`s over the same key columns **are** merged, as
`transformIndexConstraints` does (see the `ALTER TABLE … ADD CONSTRAINT` row).
**Repeated key columns, per constraint:** a repeated PK/UNIQUE key column is `42701`, a
repeated FK *referenced* column `42830`; `CREATE INDEX (a, a)` and an FK's *referencing*
list stay accepted. **Divergences (silent):** a referential action written twice (`ON
DELETE CASCADE ON DELETE SET NULL`) is accepted, the later one kept, where 18.6 answers
`42601` (`EDGES.md` row 289); and a declared constraint may take the name 18.6 mints for
a column's `NOT NULL` (`<table>_<column>_not_null`), which 18.6 refuses `42710` for a
`UNIQUE`, a foreign key or an `ADD CONSTRAINT` (`EDGES.md` row 290)

### INSERT column-list binding

binds by name; omitted columns take their DEFAULT. A row's length is checked as 18.6
checks it, row by row in list order after the row's cells are resolved: against the
first `VALUES` row (`42601 VALUES lists must all be the same length`), then against the
target list (`INSERT has more expressions than target columns`; `… more target columns
than expressions` under a column list — without one a short row takes its defaults), the
`INSERT … SELECT` source alike — save that a cell error pglet reaches only at execution
loses to the length, and an `INSERT … SELECT` source's error pglet reaches only after
the source's arity (such as an unknown relation, an operator pglet resolves only at eval
— in the target list or the `WHERE` — or a name with no `FROM` to bind against) loses to
that arity (`EDGES.md` rows 225, 226). The list is a **set**: a column named twice is
`42701`, resolved at the boundaries `checkInsertTargets` sets — ahead of the arity
check, the cells' conversions and the identity guard — and interleaved per item, so a
repeat outranks a *later* unknown name and yields to an earlier one (`docs/names.md`).
One boundary it does **not** inherit is the `INSERT … SELECT` **source query**, which
18.6 transforms after the list and pglet lowers before it, so the source's own error
wins here (`docs/names.md` § What the refusals do not carry). The row-less **`INSERT …
DEFAULT VALUES`** is the same widening with *every* column omitted — it parses to one
empty `VALUES` tuple, so it lowers to the identical `Plan::Insert` its `VALUES (DEFAULT,
…)` twin does and inherits `ON CONFLICT`, `RETURNING` (wildcard and correlated
subqueries included) and the identity rules unchanged. `gram.y` gives that alternative
neither a column list nor `OVERRIDING`, so `INSERT INTO t (a) DEFAULT VALUES` and
`INSERT INTO t OVERRIDING SYSTEM VALUE DEFAULT VALUES` are `42601` at the `DEFAULT`
token on both engines. **Divergence:** under `OVERRIDING USER VALUE` the identity
column's cell is replaced before it is read, so an ill-typed constant or an unknown name
there is accepted where 18.6 refuses, a `$N` there leaves the statement's parameters
(native server: `08P01` at `Bind`), and a `SELECT` source is `0A000` (`EDGES.md`
row 287)

### Arrays (`text[]`/`uuid[]`/…), `@>`/`<@`, `ANY`/`ALL`, `unnest`

real `Value::Array` + per-type array OIDs; scan-based (GIN-accelerated for a top-level
`@>` conjunct or an `OR`-union of same-column `@>`); bound `$N::text[]`/`$N::uuid[]`
params work (literal + bound). An array literal takes the **common type** of its
elements under Postgres's `select_common_type` (`ARRAY[1, 2.5]` → `numeric[]`, `ARRAY[1,
'2']` → `int[]`, `ARRAY[2::int4, 1::int8]` → `int8[]`, `ARRAY[1::int4, 2::real]` →
`real[]`), and every element is **cast** to it at plan time — so the declared array type
and the constructed value agree. All-NULL/all-string → `text[]`. A **FROM-clause
`unnest(…)`**'s column type is derived from the array **expression**, by one
`function_scan_column_types` both declared-type walkers read — never from the evaluated
`Value::Array`'s `element_type`, which cannot carry the tz/no-tz bit a `timestamptz`
needs (that type has no distinct value here; only a column `TypeOid` distinguishes it).
So `SELECT t FROM unnest(ARRAY['…'::timestamptz]) AS u(t)` declares `timestamptz` (1184)
on both legs, as 18.6 does, and the column-type-aware text encoder keeps the UTC `+00`
offset — derived from the value it would declare `timestamp` (1114) and drop the offset,
the silent-instant-shift class. The evaluated array's element type remains the fallback
where static inference declines. The item is not lateral: an argument naming an earlier
`FROM` item's column is `42703` — or, inside a subquery whose enclosing query has a
column of that name, reads that column — and an enclosing query's column written without
its table's own name declares the item's column `text`, or as another table's column
where the qualifier is also that table's name (`EDGES.md` rows 312 and 313).
That reading is the **first element's** type for an
`ARRAY[…]` constructor written in a statement's own `FROM`, whose elements the coercion
pass does not unify there: `unnest(ARRAY[1, 2.5])` declares `integer` over `2.5` where
18.6 declares `numeric` (`EDGES.md` row 308); a cast (`ARRAY[1, 2.5]::numeric[]`) gives
18.6's column. An aggregate in the array is `42803`, as on 18.6. An unnest's columns also reach the plan-time coercion
scope, so a `COALESCE` branch unifies against one: `SELECT COALESCE(NULL, v) FROM
unnest(ARRAY[1,2]) AS u(v)` is `int4` on both legs and both engines. Fixtures:
`types/unnest_timestamptz_declared_type.sql`,
`types/unnest_timestamptz_text.sql`, `select/unnest_coalesce_declared_type.sql`. A
**query-position** `unnest(<a non-array>)` is `42883` at plan time
(`planner::legality`'s `unnest_legality`, reached from the one
`check_expr_node_semantics` walk), matching 18.6 in code and in protocol phase (`Parse`)
and independent of the row count. The gate reads
`arg_known_type`, so an argument that reader does not type (`unnest(i + 0)`,
`unnest(length(tx))`, and the
`CASE`/`NULLIF`/`GREATEST`/`LEAST`/unary-`-`/`set_config(…)` spellings beside them)
still defers, and the two `unknown` spellings (a bare `NULL` or string literal) are left
alone because 18.6 answers `42725` there rather than `42883`. **Row-independence is the
closed half's alone**: every query-position residual is silent over zero rows and
`0A000` once a row reaches the leaf, so only the arity and DDL-position residuals are
loud at any row count. `EDGES.md` row 55 holds the four. A **modifier beside a `[]` is
the element's**: a canonical type name decomposes once — the `[]` off first, then the
`(` — so `varchar(3)[]` is an array of `varchar(3)` and its cap is enforced element-wise
on store and on an explicit cast, while a `(…)` the element refuses takes the whole
spelling down at either of the two **name-bearing** `typenameTypeMod` refusals pglet
reaches, which name different types: `int4(3)[]` is `42601 … for type "int4[]"` (the
**array**) and `'{ab}'::char(0)[]` / `'{ab}'::numeric(0)[]` are the **element's** own
`22023`, each as 18.6's is — while a third layer names neither, the `int4` coercion of a
`numeric` component reached only for a carrier and ahead of that carrier's own range
checks, `'{ab}'::numeric(2147483648)[]` being `22003` and `'{ab}'::numeric(1.5)[]`
`22P02`, each about `integer` on both engines (`docs/arrays.md`, `docs/coercion.md`;
`EDGES.md` row 120, closed, and row 124 for the carrier half). **Divergence (pglet-stricter): a nested
`ARRAY[ARRAY[…]]` fails loud `0A000`** — multi-dimensional arrays are a non-goal, and
the refusal stands where Postgres returns `{{1,2},{3,4}}`. The guard is both plan-time (a
statically-typed element) and eval-time (`ARRAY[(SELECT ARRAY[1,2])]`,
`ARRAY[array_col]`), so `CREATE INDEX … ((ARRAY[ARRAY[1,2]]))` is no
**accept-then-brick** — never accepted and then failing every INSERT `0A000`.
Postgres accepts all of these; the **ragged** form `ARRAY[ARRAY[1,2],ARRAY[3]]` it
rejects `2202E`, where pglet reports its own `0A000` (a code divergence — pglet rejects
all nesting before it can tell the shapes apart). The nesting verdict is reached
**after** the common-type fold, never before it, so an element set that merely fails to
unify reports Postgres's own code whether or not one element is an array —
`ARRAY[bool_col, int_array_col]` is `42804` and `ARRAY[int_array_col, text_array_col]`
is `42846`, not a nesting complaint. `x op ANY(array)` / `op ALL(array)` over an
**empty** array short-circuits (`ANY` → false, `ALL` → true) before the NULL-LHS check,
matching Postgres. **Divergences:** a constructor over `timestamptz` / `varchar` /
`name` elements is compared as `timestamp[]` / `text[]` by `@>`, `<@` and `=` — `42883`
beside a `timestamptz[]` or `varchar[]` column where 18.6 answers, and an answer beside
`text[]` where 18.6 is `42883` (`EDGES.md` row 281); `ARRAY[…]::T[]` resolves the
constructor before the cast (`ARRAY['NaN', 0]::float8[]` `22P02`, `ARRAY['01',
0]::text[]` silently `{1,0}`; row 282); and an array **literal** is read by 18.6's array
grammar — one it refuses (`'{a,,b}'`, `'{a{b}'`, `'{a}x'`) is `22P02`, with an element's
own input error ahead of a later structural one — while one 18.6 reads as
multidimensional or with explicit bounds is refused (`'{{a,b}}'` `0A000`,
`'[1:2]={a,b}'` `22P02`; row 283), and a Unicode space on a number, `uuid`, `date`,
`time` or `interval` element is trimmed by that element's reader where 18.6 refuses it
(row 294)

### GIN indexes for arrays / JSONB `@>`

`USING gin(col)` builds an inverted index, `jsonb[]` included (each element keyed by
`jsonb`'s equality); a probe of another type than the column's declines to the recheck;
the planner routes a top-level `col @> <const>` conjunct to a `GinScan` + recheck
(partial indexes + correlated/`$N` probe), and an `OR` of same-column `@>` disjuncts (+
scan-independent guards) to a unioned `GinScan` — the multi-key audit and
param-null-guard shapes

### ENUM types

`CREATE TYPE … AS ENUM`; **a table's name is taken** — its row type shares the type
namespace, so an enum over a table and a table or sequence over an enum are `42710`, and
`DROP TYPE <a table>` is `2BP01` (`EDGES.md` row 248); **labels must be distinct** (a
repeat is 18.6's own `23505` on `pg_enum_typid_label_index`, behind the type name's
`42710`; a label is a string literal, so `('A','a')` is two members); members validated
on an `INSERT`, `UPDATE` or `COPY` of the column, at a column `DEFAULT`'s cook on every
route, and on the value an `ADD COLUMN` back-fills (`EDGES.md` row 105, closed); stored
as TEXT and **reported on the wire as TEXT** (OID 25, where 18.6 sends the enum's own
OID — the type itself has a persisted catalog OID, visible in
`pg_type`/`pg_enum`); ordering is **member position** where the value's enum is known —
from a column, or from a scalar subquery's plan output column, through one walk over the
expression — and label bytes where it is not (a cast erases it where nothing beside the
cast declares an enum — `EDGES.md` row 104; so does a subquery projecting an expression,
row 107; and two *different* enums beside each other cancel to neither — though that
pair is refused before any order is asked for, row 115). A set operation's or a
`JOIN … USING` merge's column takes the enum **either** branch declares, as 18.6's
`select_common_type` does with an `unknown` literal — over the branch pairs 18.6 itself
unifies, the rest being refused at the gate below. An **`ARRAY[…]` expression** over
enum-typed elements orders by member position too — the walk answers at the element's
width and the comparator reads that list at every element, so `ARRAY[m] <
ARRAY['ok'::vis]`, `ORDER BY ARRAY[m]` and `'ok' > ANY(ARRAY[m])` all agree with 18.6,
in a `CHECK` and a partial-index predicate as much as in a read (`EDGES.md` row 111,
closed). A cast to an enum (`'x'::vis`, `CAST(… AS vis)`, `vis 'x'`, `s::vis`,
`$1::vis`) converts as 18.6's `enum_in` does, where 18.6 runs it — a literal at plan
time and in the DDL walks, an undeclared `$N` at `Bind` where the cast is the use that
types it, anything else on the value — so a label the enum lacks is `22P02` (`EDGES.md`
row 241, closed; `docs/enums.md` § A cast to an enum converts); a `CHECK` or `DEFAULT`
holding such a conversion is a dependent of the type (`DROP TYPE` `2BP01`, `CASCADE`
drops or clears it) and an index cannot hold one (`42P17`); the cast still carries no
enum **identity** for ordering and the gates below. An enum's **identity** reaches the
gates that decide whether two operands have a comparison operator or a common type — not
the jsonb key/path, `LIKE`-family, `starts_with`, `||` and array-containment gates or
the string functions row 190 measures, which are shown it as `text` — and those gates
model 18.6's `typcategory`: an enum beside a plain `text`/`varchar`/`name` side is
`42883 operator does not exist: vis > text` at the comparisons, `nullif`, `IN`,
`BETWEEN`, `op ANY`/`op ALL`, `IN (SELECT …)` and a `JOIN … USING` merge, and `42804
<construct> types vis and text cannot be matched` at
`greatest`/`least`/`COALESCE`/`CASE` and at every door that decides a common type — a
set operation, a FROM-clause `VALUES` list, a CTE's and a recursive CTE's terms, and a
DDL-time `CHECK` or partial index — each measured byte-identical to 18.6 (rows 114 and
115, closed); two **different** enums share category `E`, so the fold succeeds and the
*conversion* fails, `42846 <construct> could not convert type <b> to <a>`, while an enum
**array** is category `A` with its identity at the element, `42846 could not convert
type text[] to vis[]`. **Gaps:** an enum array **column** (`vis[]`) is `type "vis" is
not supported` at the column, the `ADD COLUMN` and both cast-target spellings, a uniform
loud over-rejection where 18.6 accepts; an enum **right** operand at a jsonb key/path
operator, an enum at a `LIKE`-family operator, `starts_with`, a string function row 190
measures, `||` beside an enum, `integer`, `numeric`, `boolean` or `jsonb`, or `@>`/`<@`
over an enum array answers over its label as `text` where 18.6 is `42883` (`j ? m`, `m
LIKE 'a'`, `upper(m)`, `m || 1`, `ARRAY[m] @> ARRAY[t]`), and it writes — at a
predicate, an `UPDATE … SET`, and a `CHECK` or index key expression that the dump emits
and stock 18.6 refuses (row 190); the residual silent over-**acceptances** of rows
114/115 exclude the `ARRAY[…]` **pair** and the mixed `op ANY`/`op ALL` array:
an array's identity is the unification of its **typed** elements', an `unknown` element
contributing nothing as `transformArrayExpr`'s `select_common_type` does not, so
`ARRAY[m] < ARRAY['ok']`, `m = ANY(ARRAY[s,'ok'])` and `m > ANY(ARRAY['ok'])` are all
`42883` on both — what survives there is a `NULL` beside a typed element, which this
engine casts to the fold's target itself and so cannot tell from a written `NULL::text`;
a sub-plan column the `IN (SELECT …)` reader cannot type (`m IN (SELECT $1)`), which
writes; a bound `$N` whose enum target a **plan** decides, which writes at the
set-operation door; and an operand a written `::text` cast leaves undecidable, since
that is the node `m::vis` itself produces (row 104, which also writes: `DELETE FROM u
WHERE m::vis > t` destroys a row against 18.6's `42883`); rows 107 and 112 write too, by
a wrong answer where 18.6 answers; a FROM-item `unnest`'s column carries no enum (row
112); a **written** cast of a member literal to the enum in a persisted expression — at
every position `docs/ddl.md`'s render probe runs at: a column `DEFAULT`, a `CHECK`, an
index predicate and an index **key** expression — dumps as `::text`, which stock 18.6
refuses under `42804` or `42883`, decided by the statement and not by the position (a
cast of any other operand dumps as the enum cast and restores), so the database does not
restore into real Postgres, where a bare-literal spelling at each of the four positions
replays clean; and the restore is loud per statement but **silent per process**, psql
exiting `0` after a refused `CREATE TABLE` has desynchronised the `COPY` framing of
tables after it (row 104); and a real `pg_dump` of a database with an enum column does
not restore into pglet, over **two** independent `42601`s — the `ALTER TYPE … OWNER TO`
the dump emits for the type, and the schema-qualified `m public.vis` column type (row
113); an enum sharing a builtin's name yields the name to the builtin where pglet has
that type (`int4`, `text` — as 18.6's search path does), leaving the enum reachable in a
type position only as `public.<name>`, which pglet does not parse (`DROP TYPE <name>`
still drops it here, row 118), and captures it where pglet has none (`oid`, `json`,
`regclass`: row 118)

### Sequences / SERIAL auto-increment

implicit per-column sequence; explicit `CREATE SEQUENCE <name> [START WITH n]` is also
accepted (the `pg_dump` form), with `nextval('<name>')` defaults resolving against it.
**Divergences (silent):** every draw is `+1` — `INCREMENT
BY`/`MINVALUE`/`MAXVALUE`/`CYCLE` are ignored (`EDGES.md` row 215) — and ids can be
re-issued after a reopen no save of the advance preceded (row 214). The reverse
direction too: a rolled-back `ALTER SEQUENCE` naming `START WITH`, `INCREMENT BY`,
`MINVALUE`, `MAXVALUE`, `CACHE`, `AS` or `NO CYCLE`, or an identity column's `ALTER
COLUMN … SET INCREMENT BY` — measured with values that move no draw — keeps the block's
draws, where 18.6 restores the old cell and hands them out again (row 222). The full
record: § Sequences / SERIAL — details, below.

#### Sequences / SERIAL — details

**`nextval`/`currval`/`setval` are callable as SQL expressions** — `nextval` advances +
returns; `setval(seq, n [, is_called])` actually advances the counter (persisted,
non-transactional — kills the `pg_dump`-restore SERIAL-collision landmine); `currval`
reports the session's last **explicit** `nextval`/`setval(is_called=true)` value
(`55000` if none this session; an embedded `Database` is one session for all its logical
connections, `EDGES.md` row 223) — a draw made by a column `DEFAULT`'s fill does **not**
register it, where 18.6's does: after `INSERT INTO t DEFAULT VALUES` (or an
omitted-column INSERT) over a SERIAL column, `currval('t_id_seq')` is `55000` here and
the drawn value there; an explicit `SELECT nextval(…)` then sets it on both, which is
the control identifying the DEFAULT-fill path rather than `currval` itself. Loud,
measured (`EDGES.md` row 265). A name no relation holds → `42P01`; a name held by a **table or an index** →
`42809` (`init_sequence`'s `validate_relation_kind`, which all three functions reach —
measured for `nextval`, `currval` and `setval` alike). Those two land on **opposite
sides of `setval`'s argument evaluation**, because Postgres resolves `'x'::regclass` at
parse analysis and only reaches `init_sequence` in the function body: `setval('<no such
relation>', 1/0)` is `42P01` while `setval('<a table>', 1/0)` is `22012` and `setval('<a
table>', 3)` is `42809` — so a single up-front kind gate answers two of the three wrong.

**`setval` is also STRICT**, which is a fourth position between the arguments and the
body: a NULL value or `is_called` skips the call and yields NULL, so `setval('<a
table>', NULL)` is NULL rather than `42809` and the counter does not move — while an
erroring argument beside the NULL still wins (`setval(s, 1/0, NULL)` is `22012`), since
a STRICT function evaluates all its arguments before the call is skipped. Strictness is
reached only by a call the **overload resolution** accepted, though, which is a *fifth*
position and sits ahead of all four: Postgres registers `setval(regclass, bigint)` and
`setval(regclass, bigint, boolean)` and resolves between them from the argument
**types** during parse analysis, before the name's `regclass` coercion. So a NULL is not
uniformly strict — `setval(s, NULL::bigint)` is NULL, while `setval(s, NULL::text)`,
`setval(s, 5, NULL::int)` and `setval(s, NULL, 1/0)` are all `42883`, as are the
non-NULL `setval(s, 5.0)` (numeric → bigint is assignment-only) and `setval(s, 5, 1)`
(integer is not implicitly boolean). Reading strictness off the *value* cannot see any
of that — `Value::Null` carries no type — so such a reader answers NULL to three calls
Postgres refuses outright, a loud rejection turned silent success. The gate reads the argument's **static** type and treats a bare literal as
Postgres's `unknown` (accepted, then coerced), which is what keeps `setval(s, '5')` out
of it. A **sixth** position is the name's own `SplitIdentifierString` reading — `42602
invalid name syntax`, for `'s x'` or an unterminated `"` — and it is the one pglet
places wrongly, so it is recorded here rather than presented as a fifth-of-five
ordering. Postgres reaches it at the name argument's `regclass` **coercion** (behind
overload resolution, ahead of argument evaluation) and, on the DDL path, inside
`AddRelationNewConstraints` once the heap exists; pglet raises it in the **parser** (a
column `DEFAULT`) and in the **planner** (an expression), which puts it ahead of
everything. Measured on 18.6: `setval('s x', NULL::text)` is `42883` there and `42602`
here, and on `CREATE TABLE` a malformed name outranks the already-existing table
(`42P07`), the unknown column type (`42704`) and the second primary key (`42P16`) — and
outranks a Postgres **success**, `CREATE TABLE IF NOT EXISTS <an existing table> (a INT
DEFAULT nextval('s x'))`, which is a NOTICE and a skip there and `42602` here: the
*wrong kind outranks a skip* shape again, in its loud-where-Postgres-succeeds direction.
One shape already agrees, and it bounds the misplacement rather than illustrating it: a
duplicate CHECK name on the same statement is `42602` on **both** (Postgres cooks
defaults before constraints). `ALTER TABLE IF EXISTS <no such table> ADD COLUMN b INT
DEFAULT nextval('s x')` is one more member of the misplacement: 18.6 skips it with a
`NOTICE`, and pglet's parser answers `42602` before the executor's skip is reached
(`EDGES.md` row 15).

**The misplacement is shared by the other two codes this reader raises**, which is why
the gate is not moved: the same statements carrying a name malformed another way behave
identically — `setval('noschema.sq', NULL::text)` is
`42883` there and `42P01` here, `nextval('a.b.c')` is `0A000` there (`cross-database
references are not implemented`) and `42601` here. So it is a property of pglet's name
reader across three SQLSTATEs, not of `42602` alone, and closing it means
moving name resolution later on both the DDL and the expression path — a precedence
change at two scales (`docs/ddl.md` § Which error wins; `EDGES.md` row 267). Handled at the statement level (a top-level `SELECT
nextval(…)` / `INSERT … VALUES (nextval(…))`), preserving pglet's read-only eval — a
sequence function in a nested position (WHERE, subquery, `… FROM t`) **fails loud** (PG
would advance it; `EDGES.md` row 266). A sequence is a **catalog record** (`{last_value, start, owner}`),
not a bare counter: `START WITH` persists so a
bare `ALTER SEQUENCE … RESTART` can return to it, and the `OWNED BY` column is recorded
(by an explicit clause and by every implicit SERIAL/identity sequence) and dumped. A
sequence is also **in the relation namespace** — `pg_class` holds tables, indexes and
sequences in one pool, so `CREATE SEQUENCE ns1; CREATE TABLE ns1 (…)` is `42P07` in
either polarity, `IF NOT EXISTS`/`CREATE INDEX IF NOT EXISTS` skip over it, and a
duplicate `CREATE SEQUENCE` is `42P07`, not a silent success that keeps the old counter
and drops the new `START WITH`. The **implicit** `<table>_<column>_seq`
is minted at execute time against that live namespace and **disambiguated** like every
other engine-chosen name (`t_a_seq`, `t_a_seq1`, …) — pushed by a table or an index of
that name, *not* by a same-named CHECK constraint (`ChooseRelationName(…, isconstraint =
false)`), and claimed **before** the table's constraint indexes, so `CREATE TABLE b1 (a
SERIAL, CONSTRAINT b1_a_seq PRIMARY KEY (a))` is `42P07`. `CREATE SEQUENCE` also honours
`RESTART [[WITH] n]` (`SeqOptElem` admits it there too): it moves the **counter** and
leaves the declared start alone, so `START WITH 10 RESTART WITH 5` hands out `5` and a
later bare `RESTART` returns to `10`.

**`ALTER SEQUENCE` acts on three options**: `START [WITH] n` moves the recorded
start, `RESTART [[WITH] n]` moves the counter (bare → back to the start, and a `START
WITH` in the same statement applies first whatever the written order), and `OWNED BY
{t.c | NONE}` sets/clears the owner — **except on an identity column's own sequence,
which refuses every `OWNED BY`** (`0A000`, `process_owned_by`'s `sequenceIsOwned(seq,
DEPENDENCY_INTERNAL, …)`): `NONE`, another table's column, a plain sibling column, and
the identity column *itself* are all refused, and the refusal sits **after** the owner
is resolved, so a missing owner table (`42P01`), a missing owner column (`42703`) and a
wrong-kind owner (`42809`) each outrank it. That refusal is what keeps the identity link
inseparable, and pglet needs it more than Postgres does: PG stores no `pg_attrdef` row
for an identity column at all, while pglet models the link as an ordinary `NextVal`
default — so detaching the edge would leave the sequence nobody's dependent, an ordinary
`DROP SEQUENCE` (or a `DROP TABLE`/`DROP COLUMN` of whatever the edge was moved to) would
then remove it, and the column's default would name nothing: every write `42P01`, and a dump
emitting `DEFAULT nextval(…)` with no `CREATE SEQUENCE`, which stock Postgres refuses
and pglet's own restore silently renumbers from 1. A RESTART leaves `currval` alone and
**is transactional** (`Plan::swapped_sequences`, which records the counter the cell held
before it), as is `TRUNCATE … RESTART IDENTITY` — the counter reset is undone by its own
`ROLLBACK`. Its target is
resolved even for a tolerated tail (`OWNER TO`, `RENAME TO`, `CACHE`, …), in
`sequence.c`'s order: wrong kind (`42809`) **before** the `IF EXISTS` skip — so `ALTER
SEQUENCE IF EXISTS <a table>` still errors — then `42P01`, and both ahead of the `OWNED
BY` column's `42703`. On `CREATE SEQUENCE` the same three invert: the `IF NOT EXISTS`
skip first (a bad `OWNED BY` is never resolved), then `42P07`, then the owner.

**`DROP SEQUENCE [IF EXISTS] <n> [CASCADE|RESTRICT]`** parses and removes the record,
under `RemoveRelations`' order: the wrong kind is `42809` **before** the `IF EXISTS`
skip (a table or an index named here errors either way), and only a name no relation
holds reaches `42P01`/the skip. Two dependency edges then decide it, and they are
different rules rather than one — an **identity** column's sequence is its INTERNAL
implementation and is refused `2BP01` *with* `CASCADE` as without
(`findDependentObjects` raises it at the outermost recursion level with no
`DropBehavior` test), while a plain column `DEFAULT nextval('s')` — what a `SERIAL`
carries — is the NORMAL `pg_attrdef` edge, `2BP01` unless `CASCADE`, which sheds **that
column's default** and leaves the column, its `NOT NULL` and its rows. Where both are
present the INTERNAL one wins. An owned sequence whose column carries no default has
neither edge and drops cleanly. Being `OWNED BY` an identity column is **not** the same
fact as being its sequence: an `ALTER SEQUENCE … OWNED BY` records only the AUTO edge,
so such a sequence drops normally on both engines. An **`OWNED BY` owner** is checked on
its own terms rather than the target's: an index or a sequence owner is `42809` under
`process_owned_by`'s own wording (`sequence cannot be owned by relation "x"`, not
`cannot open relation`), ahead of the owner column's `42703`. A user's `DEFAULT
nextval('x')` naming a table or an index is `42809` at **DDL time** on all three
registration routes (`CREATE TABLE`, `ADD COLUMN`, `SET DEFAULT`) — deliberately
stricter than Postgres, which resolves `'x'::regclass` at parse analysis and defers the
kind check to the first read; pglet has no regclass and auto-creates the sequence, so
accepting would put one name in `tables` and `sequences` at once and dump to a script
emitting both a `CREATE SEQUENCE` and a `CREATE TABLE` under it. That includes the table
the `CREATE TABLE` is **itself** creating (`CREATE TABLE e3 (b INT DEFAULT
nextval('e3'), c INT)`), which a gate reading only the catalog cannot see; Postgres cooks
the default once the heap exists, so `'e3'::regclass` binds the table there and every
write is `42809`. Because the check stands in for an error Postgres raises at the
**first write**, it is the *last* thing the statement tests: everything Postgres rejects
at DDL time outranks it. On `CREATE TABLE` that is the column type (`42704`), the key
columns (`42703`) and a repeated one among them (`42701`), the multiplicity (`42P16`),
a colliding declared `SEQUENCE NAME` on
any column (`42P07`), the relation's column-name set (a duplicate and a system-owned
name, each `42701`), the table's own `42P07`, and then the constraint naming block — a
duplicate CHECK name (`42710`), an ill-typed CHECK (`42883`), a taken constraint-index
name (`42P07`), a missing FK parent (`42P01`) and a repeated column in an FK's
*referenced* list (`42830`); on `ADD COLUMN` the same block, ahead of the backfill.
All measured on 18.6, from both sides: with the named relation present (so
Postgres's own cook step succeeds) each of those errors is what Postgres answers,
and with it *missing* the `42P01` the cook raises instead is beaten by the first
group while **beating** the naming block — so 18.6's own cook error marks the
boundary between the two groups rather than sitting in this gate's last slot.
Gating inside the column loop instead would put pglet's over-rejection ahead of real
Postgres errors it must lose to. The name a `DEFAULT nextval('x')` will register is also
**claimed** into the in-flight relation namespace at that point, so the statement's own
constraint indexes disambiguate around it (`t1_pkey` taken by the sequence → `t1_pkey1`)
exactly as they do in Postgres when the sequence already exists — without the claim
the index would be minted on top of it and the collision found only after the catalog
writes began, which a failing DDL statement must never reach (`docs/ddl.md` § A failed
DDL statement must leave the catalog untouched; § A gate that reads the catalog is blind
inside the statement that is building it). A sequence one of this statement's own
SERIAL/identity columns claimed is **not** re-registered by a sibling `DEFAULT
nextval('<that name>')` — which both engines accept — so the column's `START WITH` and
its ownership edge survive. A **declared** `SEQUENCE NAME <n>` on an identity column is
taken verbatim at all three implicit-sequence routes and refused (`42P07`) when the
relation namespace holds it — the declared half of the mint rule; pg_dump emits that
option, so discarding it would silently restart a restored identity counter at 1
whenever the dumped name is not the default `<t>_<c>_seq`. It loses to `IF NOT EXISTS`, to the
column type (`42704`) and to the key columns (`42703`), and beats the table's own
`42P07` and a duplicate CHECK name — all measured. A declared name equal to the **table
being created** is the one shape that does not read off that list, because the collision
is not the sequence's: the before-list creates the sequence, then
`heap_create_with_catalog` finds the name taken, so `CREATE TABLE a1 (id INT GENERATED
ALWAYS AS IDENTITY (SEQUENCE NAME a1))` is `42P07` against the *table* — under `IF NOT
EXISTS` too, whose skip fires only when the table itself already exists — and a *later*
column's collision with an already-taken name still wins over it. The gates read the
catalog, which does not hold the table under construction, so this one is decided by
testing the statement's in-flight names at the table's own `42P07`. The auto-name reads
the **committed catalog only**, never the names the same statement has claimed, so a
declared name colliding with a sibling SERIAL's auto-name is `42P07` in either written
order rather than a disambiguation.

**Divergences:** a sequence is not a readable relation — Postgres answers `SELECT
last_value FROM s` with a one-row `(last_value, log_cnt, is_called)`, pglet raises
`42809` naming the reason; every *write* shape over a
sequence is `42809` on both engines — INSERT/UPDATE/DELETE, `COPY` either direction,
`TRUNCATE`, `DROP TABLE`, every `ALTER TABLE` action, `CREATE INDEX … ON s`, `REFERENCES
s`, and `DROP INDEX s` (`"s" is not an index`, its own wording). The kind is decided on
a name that **resolves**, so it outranks a *skip* as well as an error: `DROP TABLE IF
EXISTS s`, `DROP INDEX IF EXISTS s` and `CREATE INDEX IF NOT EXISTS <a taken name> ON s`
all error rather than reporting a no-op success — the shapes where this class would be
silent, since the skip makes the statement *succeed*.

**The whole class is one rule over the whole relation namespace, not a sequence rule.**
`pg_class` pools tables, indexes and sequences, so every ordered pair of kinds is
reachable and each is `42809` on 18.6: `DROP TABLE [IF EXISTS] <an index>`, `DROP INDEX
[IF EXISTS] <a table>`, `TRUNCATE`/`SELECT`/`INSERT`/`UPDATE`/`DELETE`/`COPY`/`ALTER
TABLE`/`CREATE INDEX … ON`/`REFERENCES` over an index, and the sequence-side
equivalents. `Schema::relation_kind` is the single enumeration and
`Error::WrongObjectType` the single error, so the two directions cannot drift — a gate
reading `schema.sequences` alone would leave the two `IF EXISTS` spellings *silently
succeeding* and the index-side shapes reporting `42P01`/`42704`. These are **not**
covered, each for its own reason: `COMMENT ON …` (any target, any kind) parses as a
tolerated no-op, so nothing reaches the kind gate (see `EDGES.md` — the tolerated-no-op
family), and a bad *column list* over a sequence is
`42809` where PG says `42703`, because PG resolves the sequence's own `(last_value,
log_cnt, is_called)` descriptor at parse analysis and pglet models no such relation —
the read-side gap seen from the write side. The read-side model is the class's remaining
edge in the other direction too: `pg_class` hardcodes `relkind` `'r'` and lists **tables
only**, so a sequence or an index is absent from it while `CREATE TABLE <that name>`
answers `42P07` — the one-name-two-answers contradiction the `42809` gate keeps off the
write side, standing on the introspection surface (`pg_indexes` does
carry indexes). pglet models no `MINVALUE`/`MAXVALUE`, so `START WITH 0`, `setval(s, 0)`
and `RESTART WITH 0` are all accepted where PG raises `22003`/`22023`, and every draw
is `+1` — `INCREMENT BY`, `MINVALUE` / `MAXVALUE` and `CYCLE` are parsed and ignored
(`EDGES.md` row 215); `ALTER SEQUENCE …
RENAME TO` is accepted and does nothing; `lastval()` unimplemented (`0A000`); a **NULL
sequence name** — `nextval(NULL)`, `currval(NULL)`, `setval(NULL, 5)`, all NULL in
Postgres by strictness — is `0A000` here, because the parser takes the name as a string
literal rather than an expression; and `setval(s, '5')` is `42804` where PG coerces the
unknown literal and answers `5` (the literal-as-text gap, here at a function argument;
`setval(s, 'notanumber')` is the same `42804` where PG says `22P02`) — and the gap is
the same on the **third** argument, which the overload gate likewise lets past as an
unknown literal: `setval(s, 5, 't')` is `42804` where PG answers `5`, and `setval(s, 5,
'notabool')` is `42804` where PG says `22P02`. A **`::regclass`** wrap on the name is
pglet's own `0A000 not implemented: nextval() requires a string-literal sequence name`
in an *expression* (`SELECT nextval('s'::regclass)` → PG `1`): `parse_cast_target`
lowers the cast to a `StringFn` that `sequence_name_from_arg`'s look-through does not
match, so the refusal is the look-through's and not the type resolver's. The spelling
pg_dump emits — a column `DEFAULT` — is accepted, because `parse_nextval_arg` eats the
cast token without resolving it, and `'s'::text`/`'s'::varchar` work in an expression
too (PG coerces those to `regclass` at run time). The name argument's own cast **is** coerced
before the expression is discarded, so `nextval('s'::int)` is `22P02` on both engines
rather than pglet silently advancing the sequence.

**`GENERATED {ALWAYS|BY DEFAULT} AS IDENTITY`** reuses this machinery (implicit sequence
+ NOT NULL, `START WITH` seed, `OVERRIDING SYSTEM|USER VALUE`; `ALWAYS` rejects an
explicit non-`DEFAULT` INSERT/UPDATE/`ON CONFLICT` value → `428C9`; the pg_dump `ALTER …
ADD GENERATED …` form; a non-integer identity type → `22023`, `ADD` on an
already-identity column → `55000`, on a column not declared `NOT NULL` → `55000`, on a
column that has a default → `55000` (`DROP DEFAULT` first — the statements that turn a
`SERIAL` column into an identity one), `… AS (…) STORED` → `0A000`; **`ALTER COLUMN … SET
DEFAULT` / `DROP DEFAULT` on an identity column → `42601`** — `ATExecColumnDefault`'s
refusal, the other half of the `OWNED BY` one above, outranking the default's own
sequence resolution (`nextval('<no such sequence>')`, `nextval('<a table>')` — so it
also outranks pglet's own DDL-time `42809`), and beaten *among the executor's gates* by
the table's `42P01` and the column's `42703`.

**Some spellings never reach any executor gate at all**, so the precedence has to be
quantified over what the front end does with the statement rather than over the gates:
an **unrecognized function** in the DEFAULT is refused by the planner's own lowering
(`0A000` — `nosuchfn()`, where 18.6 answers `42601`), under `ALTER TABLE IF EXISTS` too.
`ONLY`, the `COLUMN`-less `ALTER id`, `IF EXISTS`, `-1`, `CURRENT_DATE`, `(1+1)`, `$1` and
the pg_dump `nextval('x'::regclass)` spelling all do reach the gate and answer `42601` on
both engines — the last two through the `ColumnDefault::Expr` catch-all, which the
classifier's recognized forms fall through to (a bare `$N` on a **non**-identity column
is `42P02` on both, the control identifying the identity gate rather than the parameter
reader).
The identity flag is persisted (`Column.identity`).

**An `ALTER TABLE … ADD COLUMN` carrying a `SERIAL` / identity / `DEFAULT nextval('x')`
stages its sequence behind the backfill validation**, so a statement that then fails
(`… CHECK (c > 100)` over two existing rows, `23514`) leaves no sequence and a free name,
as 18.6 does: the record is *staged* (a local `Sequence` the backfill draws from) and
inserted only inside the mutation block, which is the shape the route needs rather than
a reordering — the backfill has to draw from the sequence while the statement can still
be refused. A sequence the statement did **not** create is still advanced in place,
since `nextval` is not rolled back and walking a live counter back would re-issue issued
ids. An added `SERIAL` column is declared `NOT NULL`, as a `CREATE TABLE`'s is and as on
18.6, each row the table holds having drawn its value (`ddl/add_column_serial_*`).

**Divergence, measured and left:** how *far* it advances, and **only on the row-level
constraint checks**. Those 18.6 applies as it rewrites row by row, stopping at the
offending row, while pglet resolves the whole backfill and then validates it, so pglet
draws once per existing row — over four rows, a `CHECK` failing on row 1 leaves 18.6's
next `nextval` at `2` and pglet's at `5`, and failing on row 3 leaves 18.6's at `4` and
pglet's at `5` (the `NOT NULL` member of that family is unreachable over a
sequence-bearing default, which resolves to a non-NULL `int8`). The **FK and unique**
routes agree instead, because 18.6 validates those after the whole rewrite: a
`REFERENCES` whose first violation is row 3, and a `NUMERIC(3,-1) … UNIQUE` whose
backfill collides, each leave both engines' next `nextval` at `5`. Loud on neither, and
a gap in a counter that promises no gaplessness; no fixture pins it because a faithful
one would be red. `CREATE SEQUENCE` rolls back inside a transaction: a block that wrote restores its
catalog unconditionally.

### `DEFAULT` functions (`now()`, `gen_random_uuid()`)

literals, `now()`, `gen_random_uuid()`; the impure pair reads the OS clock/RNG natively
and the JS host on wasm (a wasm clock read answers inside the millisecond `Date.now()`
reports at that read, a microsecond past the read before it through the first five
hundred reads of one millisecond and equal to it past them, so a value stored at
precision 3 or finer stays in that millisecond; a host read answering `NaN` is refused
`22000` — `docs/temporal.md` § The transaction and statement clocks). **The five
`now`-family spellings are five distinct catalog defaults** (`ColumnDefault::{Now,
TransactionTimestamp, StatementTimestamp, ClockTimestamp, CurrentTimestamp}` over the
matching `BuiltinFn` arms), because 18.6 keeps them distinct in three observable ways:
`information_schema.columns.column_default` and `pg_dump` render each verbatim
(`CURRENT_TIMESTAMP` as the bare keyword, the rest as their own calls), the output
column name of a bare projection is the spelling (`SELECT clock_timestamp()` → column
`clock_timestamp`), and **`clock_timestamp()` alone is VOLATILE**, so an `ALTER TABLE …
ADD COLUMN` backfill re-reads the clock per row where the other four take one value
(18.6's, measured on one idle host, a count that turns on the host's speed: 3-4 distinct
instants over 5 rows, 6-11 over 20 and mostly 7, against a uniform 1 for `now()` /
`transaction_timestamp()` / `statement_timestamp()` / `CURRENT_TIMESTAMP`; a *constant*
default is not written into the rows at all but into `pg_attribute.attmissingval`, which
pglet does not model — it is perfectly **observable** on 18.6, where `SELECT
atthasmissing, attmissingval FROM pg_attribute` answers `t | {42}` for a `DEFAULT 42`
add-column; what makes it unreachable *here* is pglet's own catalog, whose
`pg_attribute` emits neither of those two columns). One variant for all five would cost
all three at once — the deparse, the column name, and a single instant backfilled where
18.6 writes many. Pinned by
`ddl/default_now_family_spellings.sql`,
`ddl/add_column_clock_timestamp_backfills_per_row.sql`,
`select/now_family_output_column_names.sql`, `executor::ddl::backfill_volatility_tests`
and `dump::column_default_round_trip`. **Which clock each reads is 18.6's**: `now()`,
`transaction_timestamp()` and `CURRENT_TIMESTAMP` answer the transaction's start — an
open block's, or the statement's own — `statement_timestamp()` the statement's, and
`clock_timestamp()` the wall clock at the call, so a multi-row `INSERT` under `DEFAULT
now()` writes one instant and `now()` stands still inside one `BEGIN`
(`docs/temporal.md` § The transaction and statement clocks; the `**/clock_*` fixtures,
and the embed, server, wasm and npm clock tests for the boundaries a fixture cannot
drive). The protocol residual is the extended protocol's: outside a block a bound
statement's `now()` equals its `statement_timestamp()` where 18.6's is earlier, and each
statement of a pipeline has its own `now()` (`EDGES.md` row 292); on wasm the reads of
one millisecond tie past the five-hundredth (`docs/temporal.md`). **The variants'
postcard tags are pinned** (`catalog::schema`'s
`column_default_postcard_variant_tags_are_pinned` and `planner::expr`'s
`builtin_fn_postcard_variant_tags_are_pinned`), so a variant is added by appending it and
no stored tag changes meaning.

### `starts_with(a, b)`

case-sensitive literal prefix (not LIKE wildcards). Postgres has exactly one signature,
`starts_with(text, text)`, so a non-text argument on either side fails loud `42883` —
short of an enum, whose label is read as `text` and answers (`EDGES.md` row 190)

### `||` string concat

pglet implements the **text** forms only — `text || text` and Postgres's `text ||
anynonarray` / `anynonarray || text` (the non-text side stringifies, matching PG where
the text side is typed; an untyped side — a bare string literal or an untyped `$N` —
beside an operand whose type has a `||` of its own stays `text` here, where 18.6 reads it
as that type: beside a `bytea` or a `jsonb` the two are joined as text, silently, where
18.6 joins the bytes or merges the objects, and beside an array it is `42883` (`0A000` over a `text[]`) where 18.6 appends — `EDGES.md` row 293). NULL-propagating (`a || NULL` → NULL). A `timestamptz` operand keeps its `+00`
offset through the concat, so the stringified instant doesn't shift. **Gap (loud, not
silent):** the three concatenations Postgres has and pglet doesn't evaluate — `bytea ||
bytea` (byte concat), `jsonb || jsonb` (merge), and the array forms (`array_cat`) — fail
`0A000`; an operand pair with no `||` signature at all (`int || int`, `int[] || text`)
fails `42883`, short of an enum, which is `text` to the rule and so answers beside an
enum, `integer`, `numeric`, `boolean` or `jsonb` side, which 18.6 refuses `42883` (`m ||
m` is `aa`, `m || 1` is `a1`; `EDGES.md` row 190)

### `LIKE … ESCAPE` + default `\` escape

`\` is the default escape (PG-compatible — `\%`/`\_` match literally); explicit `ESCAPE
'\'` accepted, any other escape char fails loud. Operands are gated to the string types
— a non-text operand (`int_col LIKE 'x'`, `1 LIKE '1'`) fails `42883` rather than
silently returning `false` (which made `NOT LIKE` true for every row) — short of an
enum, whose label is read as `text` and answers where 18.6 is `42883` (`EDGES.md` row
190). **Gap (loud):** `bytea ~~ bytea` / `!~~ bytea` exist in Postgres (byte-wise) and
are unimplemented here → `0A000`; there is no `bytea ~~*`, so case-insensitive bytea
stays `42883`. **Divergences (loud):** `ANY`/`ALL` on the right of a `LIKE`-family
operator (`'a' LIKE ANY(ARRAY['a','b'])`) is refused `42601` where 18.6 applies it per
element (`EDGES.md` row 186), and a chained `LIKE` is `42883` where 18.6's grammar
answers `42601` (row 185)

### `ILIKE`

case-insensitive `LIKE` — both operands folded through the same Unicode lowercasing as
`lower()`, so wildcard/`ESCAPE` semantics are identical to `LIKE` and a bound `$N`
pattern types text. The fold is Rust's `to_lowercase()`, a **full** (1→N),
context-sensitive Unicode case mapping, where Postgres's libc path is the **simple** 1→1
`towlower`. Measured on the oracle (libc provider, `en_US.UTF-8`, UTF8): `SELECT
'İstanbul' ILIKE 'istanbul'` is `f` here and `t` there, and `SELECT length(lower('İ'))`
is `2` here and `1` there (`EDGES.md` row 264). Operands are gated exactly as `LIKE`'s —
a non-text operand fails `42883`, an enum excepted (row 190); `LIKE`'s `ANY`/`ALL` and
chaining divergences apply here too (`EDGES.md` rows 185, 186)

### Scalar string + misc functions

`upper`, `length` (char count; byte count for `bytea`), `replace`, `concat`/`concat_ws`
(NULL-skipping, not NULL-propagating), `substring` (`… FROM n FOR m` special form +
comma form, PG's clip math, `22011` on negative length), `trim`/`ltrim`/`rtrim`/`btrim`
(`TRIM([BOTH|LEADING|TRAILING] [chars] FROM x)`, char-**set** membership),
`position`/`strpos` (`POSITION(a IN b)`, 1-based, arg-swap), `lpad`/`rpad` (char-based,
`54000` on an oversized length), `split_part` (negative-index-from-end, `22023` on 0),
`substr` (`substring`'s evaluation under its own name; both take a `bytea` and answer
`bytea`, counted in bytes), `left`/`right` (characters, a negative count dropping `|n|`
from the other end; `right(s, -2147483648)` answers the whole string, as the 18.6 oracle
does), `octet_length` (`int4` bytes of a `text` or `bytea`), `md5` (`text` or `bytea`, an
in-house RFC 1321 digest as lower-case hex). `left`, `right` and `substr` take their
counts as `int4` — an `int2` widens, an `int8` or `numeric` is `42883`, a string literal
is read as `int4` (`22P02` if it cannot be) — while `substring` still accepts an `int8`
there, which 18.6 refuses. `left` and `right` are TYPE_FUNC_NAME keywords and parse as
calls before a `(` (§ Keyword as identifier). Over a `char(n)` value `octet_length`
counts the unpadded string, `char(n)` being stored as `varchar` (`EDGES.md` row 95).
`random()` is a `double precision` in `[0, 1)` drawn afresh at each evaluation — per row, per
row of an `ADD COLUMN` backfill, per row an `unnest(…)` beside it emits, and as the GIN
selector declines it for a guard — from the OS RNG natively and `Math.random()` in the
wasm build; a read-only `WITH` query that does not reference itself and whose body calls it is evaluated at each reference, and an
uncorrelated subquery over it — `(SELECT random())` — at each row of the query around it,
where 18.6 draws once for each (`EDGES.md` rows 316 and 319); `setseed` is not built, so a sequence of draws cannot be repeated, and
`random(min, max)` is `42883`. An index over `random()` is `42P17`, as on 18.6.
`greatest`/`least` (NULL-skipping + `select_common_type` unification so the wire type
label matches the returned value; `42804`/`22P02` on incompatible args). **`nullif`
follows a different rulebook** — Postgres resolves the `=` operator and declares
*argument one's* type as the resolution left it (`parse_expr.c`: `opresulttype =
exprType(linitial(args))`), so `NULLIF(int4, int8)` is `integer` where `GREATEST` of the
same pair is `bigint`. Argument one survives whenever a registered `=` takes it on the
left (the integer family cross-compares, a float accepts any numeric, `varchar` has no
`=` of its own so it goes to `text`) and widens when none does (`NULLIF(int4, real)` →
`double precision`). The returned value is argument one at that same type, so declared
and returned agree for those pairs. They part where the plan types only the first
argument: the value's type is recomputed from the two values, so through the npm
package `nullif(n, $1)` and `nullif(n, n * $1)` over a `numeric` or `bigint` `n`, bound
a fractional JS number, hand `n` back as a `double precision` under its own declared
type (`EDGES.md` row 288).
Two consequences that look wrong and are not: `NULLIF(varchar_col, varchar_col)` is
**`text`** (varchar has no `=` of its own even against another varchar), and
`NULLIF(time_col, interval_col)` is `interval` — the one implicit cast crossing a
type-category line, which the operator rulebook bridges and `select_common_type` does
not (`COALESCE(time, interval)` is `42804`). An operand pair with no `=` at all is
**`42883`**, not the `42846` the common-type constructs report for the same pair. All
indexing 1-based, all length/position character-based; a non-text arg (short of an enum,
whose label is read as `text` and answers — `EDGES.md` row 190) / wrong arity fails loud
— `42883` through `string_call`/`variadic_call` (`upper('a','b')`), but `0A000` through
`single_arg`/`binary_call` (`lower('a','b')`, `starts_with('a')`) where 18.6 is
uniformly `42883` (the *Wrong-SQLSTATE-class* row). Special forms are `ColName`
keywords, paren-guarded so a bare `substring`/`trim`/`position` stays a usable column
name. **Divergence:** `upper`/`lower` use Rust's full, context-sensitive Unicode case
mapping, which expands special-casing chars where PG's locale/simple mapping doesn't
(`upper('straße')` → pglet `STRASSE`, PG `STRAßE`; `length(upper('ß'))` is `2` here and
`1` there) and lowercases a word-final capital sigma to the final sigma where PG gives
the plain one (`EDGES.md` row 264) — an accepted divergence, like `ILIKE`'s. **What is
not implemented is loud**, and the set is **open, not a list**: the `0A000 function not
supported: <name>` arm is the *default* arm of `planner::expr`'s `build_call` name
match, so every name the dispatch table does not hold reaches it — including a name that
exists nowhere (`nosuchfn(1)` is `0A000` here against 18.6's `42883`, the
*Wrong-SQLSTATE-class* row). What can be stated is the **measured members**:
`initcap`, `reverse`,
`translate`, `repeat`, `ascii`, `chr`, `to_hex`, `array_position`,
`format`, `age`, `quote_literal` and the numeric `to_char(numeric, text)` (the
`to_char(timestamp, text)` overload the spine issues is implemented) all answer `0A000`.
**`overlay` is both**:
the ordinary call spelling `overlay('abc', 'x', 2)` is `0A000` like any other
unknown name, while only the SQL-standard `overlay('abc' PLACING 'x' FROM 2)` form is
the parser's `42601`. None of them is silent, and none is a spine consumer's

### `date_trunc` / `extract` / `date_part`

`date_trunc(field, source)` (timestamp/timestamptz/interval; a `date` casts to
timestamptz, result `+00`) — cascade zeroes finer fields, `week`→ISO Monday,
quarter→first month, century/millennium no-year-0; `EXTRACT(field FROM source)` special
form + `extract(text, src)` / `date_part(text, src)` plain calls over
timestamp/timestamptz/date/time/interval. **`extract` returns `numeric` with PG's
per-field display scale** (second/epoch → scale 6, ms → 3, integer fields → 0, julian →
20-digit fractional); **`date_part` returns `float8`**.
dow(0=Sun)/isodow(1=Mon)/doy/isoyear/epoch (interval epoch = 365.25-day years + 30-day
remainder-months); the two-tier field split matches PG (an EXTRACT-only field like
`dow`/`epoch` on `date_trunc` → `22023`; a recognized-but-wrong-for-type field →
`0A000`). Differential-tested. **Divergences:**
`extract(timezone*/timezone_hour/timezone_minute FROM timestamptz)` → `0A000` where PG
gives `0` (pglet stores timestamptz as a plain `Value::Timestamp`, so eval can't
distinguish it — the offset is always 0 under UTC anyway); `date_trunc(field, time)` →
`42883` (no `time → interval` implicit cast); a truncation result before year 1 —
`decade` over years 1–9 — is `22008 timestamp out of range` where PG answers `0001-01-01
00:00:00 BC` (the date domain, `EDGES.md` row 142); the `EXTRACT(field FROM …)` **field
grammar is too wide** — `parse_extract`'s `is_field_word` admits
`TokenKind::Keyword(_)`, so any reserved word standing directly before `FROM` is taken
as the field and resolved at eval, where PG's `extract_arg` production is `IDENT |
YEAR_P | … | Sconst` and refuses it in bison: `EXTRACT(ALL FROM now())`, its `DISTINCT`
spelling, `EXTRACT(SELECT FROM now())` and `EXTRACT(TABLE FROM now())` are `42601` there
against `22023 unit "…" not recognized` here (loud both, and *only* the word directly
before `FROM` reaches it — `EXTRACT(ALL year FROM now())` is `42601` on both, from the
comma form's own expression parse). **And an untyped `date_trunc` source is an
over-acceptance, silent in one spelling and loud-but-miscoded in the others.** 18.6
registers three two-argument `date_trunc`s — over `timestamp`, `timestamptz` and
`interval` — so a source it cannot type resolves to none of them: `SELECT
date_trunc('day', NULL)` is `42725 function date_trunc(unknown, unknown) is not unique`
there and **answers** here — one NULL row declaring `timestamp` (1114) on **both** wire
legs, which is `date_trunc_result_type`'s no-source default rather than a walker split
(the two legs agree). `date_trunc(NULL, NULL)` and `date_trunc('day', COALESCE(NULL,
NULL))` are silent members too (1114 here; `42725` and `42883 function
date_trunc(unknown, text) does not exist` there). Two neighbouring spellings are **loud
under the wrong code** instead, both by the same cause — a bare literal types as `text`
here rather than staying Postgres's `unknown` (the general literal-typing gap): a
string-literal source (`date_trunc('day', '2026-05-04')`) and a **non-NULL** `$1` bound
as unknown are each `42883 function date_trunc does not exist for source type
text` here against that same `42725` there, while a `$1` bound **NULL** lands back
on the silent side at 1114. The control is a typed source, agreeing everywhere:
`date_trunc('day', CAST(NULL AS TIMESTAMPTZ))` is 1184 and the `TIMESTAMP` spelling 1114
on both engines. Same class as the NUMERIC row's `trunc(NULL)` — pglet models no
overload-ambiguity check — and no fixture can hold the silent member, since pglet
answers where 18.6 errors; `executor::walker_pins`'s cell-M test pins the 1114 answer and
says it pins the answer, not its rightness

### `SELECT DISTINCT` / `DISTINCT ON`

`DISTINCT`: whole-row dedup, NULL-equal (two NULLs not distinct) and float `NaN`-equal.
pglet gives float `NaN` a **total order** in the shared comparators, matching Postgres:
`NaN = NaN` is true and `NaN` sorts greatest, consistently across `=`, `<`/`>`, `ORDER
BY`, `MIN`/`MAX`, `DISTINCT`, window partitioning, joins, and FK/conflict matchers — and
across float widths (`1.5::float8 = 1.5::float4`). **`DISTINCT ON (…)`**: a real
`Plan::DistinctOn` — keeps the first row per ON-key group in `ORDER BY` order (the input
is sorted by the merged effective key, then adjacent-deduped). Enforces Postgres's
stateful rule that the ON expressions match the leftmost `ORDER BY` (a foreign sort key
before an unmatched ON expr → **`42P10`**); an unmatched ON expr is appended ASC. ON
keys accept a positional `(1)`, an output alias, and a qualified/bare column
(canonicalized to the same output column). The latest-row-per-group idiom (`DISTINCT ON
(a.id) a.id, b.note … ORDER BY a.id, b.created_at DESC`) works: the ON key is answered
by the projection while the trailing sort key is read from the pre-projection row.
`SELECT DISTINCT` additionally enforces Postgres's rule that every `ORDER BY` expression
appear in the select list (**`42P10`**), matched by expression equivalence — so `ORDER
BY b+a` against a projected `a+b` is rejected, as in Postgres. An ON key referencing a
column the projection **drops** (`DISTINCT ON (dept) name … ORDER BY dept, salary DESC`)
rides the projecting node as a Postgres **resjunk** column that a `Plan::JunkFilter`
trims off above the dedup. An ON key over an unexpanded bare `*` projection needs no
junk column — the star denotes every column the scope carries — and one naming nothing
is `42703`, on both engines and whether or not a row matches. **Divergence:** over a
**no-FROM** select the ON keys and a trailing `ORDER BY` are lowered by separate
functions and not merged, so the match rule is unenforced there — `DISTINCT ON (q)
unnest(ARRAY[2,1,2]) AS q, 5 AS r ORDER BY r, q` answers here against 18.6's `42P10`. At
a level no expression subquery encloses, a **no-FROM** select's ON keys bind at plan
time against its target list, so a key naming nothing is `42703` and a qualifier `42P01`,
at any row count and ahead of any write. **Divergence:** over a bare `*` projection a key naming
a column the level's scope does not **enumerate** is `0A000` — one rule, whose members
are the kinds of relation `build_column_scope` registers non-exhaustively (re-derive
them from the function rather than from a count) — and each
reaches it: a catalog-view column pglet does not
model (`DISTINCT ON (c.udt_name) * FROM information_schema.columns c ORDER BY
c.udt_name`, rows on 18.6); and a **relation the catalog lacks**, registered the same
empty way, so pglet's own `42P01` never runs (`SELECT DISTINCT ON (nosuch) * FROM
no_such_tbl ORDER BY nosuch` is `0A000` against `42P01` there, the non-star spelling
being `42P01` here too). Both loud. A **FROM-clause `unnest(…)`** is not a member: it
registers its *columns*, so a qualified key answers (`SELECT DISTINCT ON (u.u) * FROM
unnest(ARRAY[1,2,2]) AS u ORDER BY u.u` answers two rows on both engines,
`select/distinct_on_unnest_qualified_key_over_star.sql`), and registers them
columns-**exhaustive**, so a bogus key is `42703` on both (`SELECT DISTINCT ON (nosuch) *
FROM unnest(ARRAY[1,2,2]) AS u ORDER BY nosuch`)

### `UNION` / `INTERSECT` / `EXCEPT` (+ `ALL`)

a real `Plan::SetOp` node, chainable + parenthesizable (either operand). PG precedence —
**`INTERSECT` binds tighter** than `UNION`/`EXCEPT` (both left-assoc). Non-`ALL` dedups
whole-row **NULL-equal**; `UNION ALL` concatenates; **`INTERSECT ALL` =
`min(countL,countR)`**, **`EXCEPT ALL` = `max(0, countL−countR)`** per row-group
(Postgres's `nodeSetOp` multiset arithmetic); of equal rows that spell differently
(`1.0`, `1.00`) an `INTERSECT [ALL]` shows the left input's, where 18.6 shows the input
its plan reads first (`EDGES.md` row 298). Columns unify per position to a common
type (int-width widening, numeric promotion, `text`/`varchar`→`text`,
`date`/`timestamp`/`timestamptz`→widest, and a `name` side by its own `pg_cast` rows —
`name`/`text` folds to `name` written first and to `text` written second,
`varchar`/`name` to `name` either way, `name[]`/`text[]` element-wise to the same pair,
each measured agreeing with 18.6), cast **positionally** so the wire type label matches
the value; an **`unknown`** column — a bare string constant, a bare `NULL` or an
undeclared `$N` — contributes no type and takes the other side's, `text` where both
sides are unknown, and a constant is converted as the node is built, so a refused one is
`22P02` ahead of execution (`SELECT 1 UNION SELECT 'x'`), the same rule a FROM-clause
`VALUES` list applies per column; a sort, grouping, `DISTINCT` or window key naming the
entry — by position, by name, or by being the same expression — resolves it to `text`
first, as on 18.6; a side projecting an **enclosing** query's column takes that column's
type: built inside a subquery the node cannot see those levels, so the column is left
pending, the correlated re-walk unifies it against the level-chained scope and records
what with, and the statement is **parsed a second time** with those types — so no reader
of the plan that parse returns is handed a provisional type (`docs/scope.md`); result
column **names from the left**. A column-count mismatch fails loud (`42601`), an
incompatible type `42804`. A trailing `ORDER BY`/`LIMIT` binds to the combined result;
**positional `ORDER BY <n>`** (1-based, out-of-range `42P10` — `<clause> position <n> is
not in select list`, matching PG) is supported on a set-op result (name refs too;
permissive on expression keys). **Divergences:** a `WITH RECURSIVE` term with a
self-reference is typed by its own check rather than by this unification, so `WITH
RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT '2' FROM r WHERE false) …` is `42804` here
where 18.6 answers `integer`; a column waiting on an enclosing level raises its error
after the enclosing level's clauses rather than ahead of them, and where the **first**
parse's reader raises first that error is the statement's — `SELECT count(*) FROM t
WHERE EXISTS (SELECT 1 FROM (SELECT n EXCEPT SELECT 1) s WHERE n = '1.25')` over a
`numeric` `n` is `22P02` here against 18.6's row; a FROM-clause `VALUES` list inside a
subquery types an enclosing column `text` (`SELECT (SELECT x FROM (VALUES (i), ('7'))
s(x) LIMIT 1) FROM t` declares `text` where 18.6 declares `integer`, and a predicate
over it can delete a row 18.6 keeps — `EDGES.md` row 151)

### `CASE WHEN … END`

searched (`WHEN <bool>`) and simple (`CASE <operand> WHEN <value>`) forms; absent ELSE →
NULL. The **searched** form's condition is one of the boolean contexts in the row below,
so a non-boolean `WHEN` is `42804 argument of CASE/WHEN` rather than a silently-taken
`ELSE`, and an unresolved literal converts (`CASE WHEN 't' …` takes the arm); the
**simple** form's label is not, 18.6 expanding it to `<operand> = <value>` and coercing
the comparison's result.

### Boolean contexts (`WHERE`, `HAVING`, `JOIN … ON`, searched `CASE WHEN`, `AND`/`OR`/`NOT`, `CHECK`, a partial index's `WHERE`, `IS [NOT] TRUE|FALSE|UNKNOWN`)

each resolves its operand to `boolean` where the operand is read, through one plan-time
gate over `assignment_compatible(x, boolean)` and one eval-time resolver. Only `boolean`
and an unresolved literal resolve — `pg_cast`'s two rows into `boolean` are both
explicit-context — so every other type is `42804 argument of <CTX> must be type boolean,
not type <T>`, and an unresolved literal is **converted** through a `boolin` that is a
prefix grammar (`'tr'`, `'fals'`, `'ye'`, `'of'` are values; `'o'`, `'01'` are `22P02`).
The four connective contexts are gated **per argument**, interleaved with the walk. A
`$N` at a clause root, or at a node position `Expr::for_each_child_ctx` marks, infers
`boolean`, below an explicit cast's priority; an `IS [NOT] TRUE` operand is neither, and
stays OID 0 (`docs/param_binding.md`). Where each clause root's gate sits is that
statement's `transformStmt` order and the clauses disagree — `JOIN/ON` outranks the
target list, `HAVING` does not, a partial index's predicate outranks its own key columns
and its access method. **Not** a member: the `ON CONFLICT` **arbiter**'s inference
`WHERE`, which 18.6 transforms with no boolean coercion. **Residuals:** a predicate a
build without this gate persisted is decoded from the store and re-passes nothing, so
such a database answers `42804` at every write it guards; the action `WHERE`'s gate
pre-empts a `VALUES` cell's per-row verdict (`docs/names.md`); and 18.6 stores a stored
expression post-parse-analysis, so its catalog surfaces render `CHECK ('t')` as `CHECK
(true)`, `check_clause` as `true`, and a constant-true index predicate not at all, where
pglet renders each as written — a divergence of the stored-expression rendering, not of
this gate (`docs/deparse.md`). `AND`/`OR`
evaluate left to right and stop at a deciding left operand, as 18.6 does — `a = 99 AND
a/(a-1) = 1` over `a = 1` is `f` on both — so an operand pglet refuses only at
evaluation goes unrefused where the left decides (`EDGES.md` row 60's silencers; row 172
for the residuals, each measured). § *A boolean context is that same conversion* in
`docs/coercion.md`.

### `FOR UPDATE` / `FOR SHARE`

the `FOR {UPDATE|NO KEY UPDATE|SHARE|KEY SHARE} [OF table…]` grammar is parsed, and so
is `gram.y`'s other production `FOR READ ONLY` — which, being an *alternative* to the
item list and not an item, is accepted only as the clause's **first-and-only**
production: `FOR UPDATE FOR READ ONLY` and `FOR READ ONLY FOR UPDATE` are both `42601`,
on both engines — inside a block a locking read takes the writer (`Plan::LockRows`), so
on the native server a second block's `FOR UPDATE` waits for the first to end: coarser
than 18.6's row lock, the same mutual exclusion (the embedded `Database` refuses the
second block's `BEGIN` `55P03` while the first is open). An autocommit locking read
takes nothing on the native server's B+tree, where 18.6 waits on a row another block
holds; on `--storage memory`, which has no lock-free read path, it takes the writer for
its statement and waits for a written block even where 18.6 would not; and the embedded
`Database` takes the writer for it, refusing it `55P03` while another connection's block
holds the writer (`docs/storage.md` § Isolation Guarantees). The **wait policy is the
exception and is refused** (`0A000`): `SKIP LOCKED` and `NOWAIT` each name a behaviour
that needs a row lock to mean anything, and honoring them as no-ops would answer
neither: a locking read inside a block takes the writer (`Plan::takes_writer`), so on
the native server a `NOWAIT` inside a second block would wait for the first block —
`55P03` only once `lock_timeout` passes — where 18.6 answers `55P03` at once, an
autocommit one would take nothing on the B+tree and return the row, and a second `…
ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED` lane would queue behind the first block
rather than take the next row (on the embedded `Database` another connection's `BEGIN`
is refused `55P03` while a block is open), where 18.6 answers `55P03` at once and hands
the lanes rows 1 and 2. The refusal is **deferred to the end of the statement's parse**,
so an error 18.6 raises *during* the parse or the plan outranks it — a projection
`42703`, a `42601` from trailing junk, from the clause's own boundary, or from a `FOR
READ ONLY` in the wrong position. **The exception is an error pglet raises later than
18.6 does**: a missing relation is `42P01` at *execute* time here (`EDGES.md` rows
29/60), so `SELECT * FROM nosuchtbl FOR UPDATE NOWAIT` is this `0A000` and 18.6's
`42P01` — loud both ways, and the deferral cannot reach past a phase. It fires on every
route, the `DO $$ … $$` body's own sub-parser included. An unrecognized lock strength or
trailing garbage (`FOR XYZ`, `FOR UPDATE foobar`) fails loud (`42601`) instead of being
swallowed to the statement boundary. The clause's **semantic** refusals are still
skipped (`EDGES.md` row 30)

### `alias.*` qualified wildcard

expanded at lowering against the catalog to that relation's columns (its JOIN side);
mixed with other projection items. The expansion is keyed on the name, so over a `FROM`-clause
`unnest(…)` item it is `0A000` (or `42702`), and where a base table shares the item's alias it
expands that table's names instead, `42703` unless they match the item's own (`EDGES.md` row 8).

### Non-sole `*` + `RETURNING` projection

a `*` expands even when mixed with other items (`SELECT *, <expr>`, INSERT/UPDATE/DELETE
`RETURNING *, <expr>` — expanded at plan time); a `RETURNING` item may be a **correlated
scalar subquery** against the just-written row (the cell `RETURNING *, (SELECT COUNT(*)
… WHERE cell_id = cell.id) AS grant_count`), evaluated with full subquery context — and
over an `UPDATE … FROM`, correlating on a **source** column works too (`RETURNING
(SELECT count(*) FROM x WHERE x.k = s.k)`). `RETURNING` items run the same plan-time
resolution and gating as a `SELECT` projection, so their declared types match the values
sent: `RETURNING COALESCE(int4_col, int8_col)` declares `bigint` and sends 8 bytes on
every row — one width per column, which a binary client needs to decode it. Their
`Column` leaves are
**bound** at plan time against the clause's namespace — the DML target under its
effective name plus the relations an `UPDATE … FROM` introduces (`returning_relations`,
the derivation the wildcard expansion, the planner-declared type walk and the
parameter-type walk read; the executor — the executor-declared walk included — derives
the same relations from its own materialized rows, a second derivation pinned against
this one by fixture): an unknown qualifier is `42P01`, a name the namespace does not
carry is `42703`, a bare name two relations claim is `42702`, and all three answer the
same whether or not the statement produces a row. `RETURNING v LIKE 5` is the leaf's
`42702` and not the operator's `42883`, on both engines — the ambiguity is decided per
node in the walk that already decides operand legality, not in a pass of its own. The
relation walk covers all five kinds a `FROM` source can lower to (base `Scan`,
`VirtualScan`, `CteScan`, `SubqueryScan`, `FunctionScan`), so over targets that carry the name
(`rvt(table_name TEXT, x INT)`, `run_t(id INT, v TEXT)`), `FROM information_schema.tables
c … RETURNING table_name` and `FROM unnest(ARRAY['x','y']) AS v … RETURNING v` are each
`42702`, as on 18.6. An `unnest` item's column is named as `chooseScalarFunctionAlias`
names a single-function RTE's — after the alias, so `SELECT v FROM unnest(ARRAY[1,2]) AS
v` answers and `SELECT unnest FROM unnest(ARRAY[1,2]) AS v` is `42703`, on both engines (a
two-argument `unnest` keeps `unnest` for both columns on both engines, so a bare `unnest`
there is `42702`). And **covering the kind is not counting the claimants** — for a `VirtualScan`
the list is the column list pglet's synthesized view emits, a strict subset of 18.6's
for many of the catalog relations it models, so the verdict silently under-counts on
every column in the difference (`EDGES.md`). **Divergences:** **PG18's `OLD`/`NEW`
correlation names are unimplemented** — `UPDATE amb_t SET v='NEWVAL' RETURNING NEW.v,
OLD.v` answers `NEWVAL | TARGET` on 18.6 and is `42P01 "new"` here; an aggregate in
`RETURNING` is **row-count-dependent** where 18.6 is uniformly `42803` — `DELETE … WHERE
id=999 RETURNING count(*)` is accepted here and answers `(0 rows)`, and the same
statement over a matching row is `0A000` (the binding rule above is
row-count-independent; this gate is not, and it is a *different* gate); an
alias-hidden table name (`UPDATE t o … RETURNING t.x`) is `42P01` on both, with PG's
message reading *invalid reference to* rather than *missing*, and pglet **quotes** a
qualified name in its `42703` (`column "amb_t.nosuch"`) where PG leaves it bare; and a
whole-tree constant-cast check still runs ahead of the binding walk, so `RETURNING
nosuchcol + 'x'::int` is `22P02` here and `42703` there. **A missing gate, recorded
rather than closed:** there is no `42712`. `UPDATE amb_t SET id=1 FROM amb_s amb_t WHERE
amb_t.id=1` is 18.6's `42712 table name "amb_t" specified more than once` and is
accepted here — the `FROM` alias shadows the target's own name and the statement runs

### `DO $$ … $$` anonymous block

a **minimal** PL/pgSQL form — `BEGIN <stmts>; [EXCEPTION WHEN <cond> [OR …] THEN
<handler>;] END` — runs the body statements and swallows a SQLSTATE-matched error (the
idempotent `CREATE TYPE` migration idiom; a `CREATE TYPE` duplicate raises `42710
duplicate_object`). `DECLARE` / variables / control flow / an unrecognized condition
**fail loud** — not a general PL/pgSQL engine. A block carrying an `EXCEPTION` clause is
a **subtransaction**, as in Postgres: catching the error discards every write the body
made (an earlier completed statement, a partially applied multi-row one, and DDL alike),
while the handler's own writes — which run after the rollback and outside it — survive,
and a sequence the body advanced is not rewound. A `DO` that **fails** — no handler, an
unmatched one, or a handler that raises — is one statement rolled back whole, as on
18.6: every object its body or handler made is gone, every table it dropped is back with
its rows, and its error is the failing statement's (`EDGES.md` row 249). A counter cell
a statement inside a `DO` replaces — a `RESTART`, a `DROP SEQUENCE` with or without a
re-create, a `TRUNCATE … RESTART IDENTITY`, a `DROP TABLE` or `DROP COLUMN` taking an
owned sequence, in the body, the handler or a nested `DO` — is put back as the in-place
advances left it by whatever rollback takes the `DO` back: its own undo, an enclosing
`DO`'s, or the enclosing block's, which reads the cells a completed `DO` reports in its
result (`EDGES.md` row 213, closed). Every `DO` pays for a savepoint level and a catalog
snapshot, a successful one included (`docs/storage.md` § Known bottlenecks / scaling
limits). **Divergences:** transaction control inside the body is refused `0A000` rather
than silently no-op'd, which over-rejects `COMMIT`/`ROLLBACK` (accepted there) and
re-codes `ROLLBACK TO`'s `42601`.

### Keyword as identifier

reservation is **derived from each keyword's Postgres `kwlist.h` class**
(`Keyword::pg_class`), not hand-listed: a word is reserved iff it is not a valid `ColId`
(i.e. not UNRESERVED/COL_NAME), matching Postgres's bare-name acceptance wherever the
keyword table decides it (the divergences below are where something else does). So `update`/`set`/`by`/`values`/`between`/`begin`/… (UNRESERVED/COL_NAME
words) are usable as bare column/table names, while `is`/`like`/`left`/`join`/… (TYPE_FUNC_NAME) are rejected as bare columns —
which Postgres rejects too (`SELECT is FROM t` is a syntax error in both). The wider
`ColLabel` positions Postgres allows for *any* keyword — a qualified attribute
(`t.left`) and an `AS` label (`… AS select`) — are accepted; a bare (no-`AS`) keyword
label stays fail-loud (it would collide with clause markers `UPDATE t SET …`). A
live-Postgres sweep (`test_keywords`) asserts pglet's accept/reject matches PG for every
keyword in pglet's table at four positions: a bare select-list column, a qualified attribute, an
`AS` label and a `FROM` table name — and, for a TYPE_FUNC_NAME word, a fifth, the function
name before `(`: 18.6's `func_name` admits that class, so `left('ab', 1)` is a call (`a`)
and `join(1)` one that resolves no function. **Divergences:** the SQL value-function words (`user`,
`current_date`, `current_user`, …) written unquoted as a column definition's
name, an `INSERT` column-list entry or an `UPDATE`'s `SET` target are accepted, where 18.6
answers `42601` — and the column so created is then unreachable by its bare name (`EDGES.md`
row 75). `ISNULL` / `NOTNULL` are keywords in `kwlist.h`'s class (TYPE_FUNC_NAME), read as the
postfix null tests `x ISNULL` / `x NOTNULL` at `IS`'s precedence; as a function name
`isnull(1)` is a call on both engines, and one pglet does not build: `0A000` here against
18.6's `42883`, the *Wrong-SQLSTATE-class* row, as `like('a', 'a')` is `0A000` here and
answers there.

### Operator lexing + resolution

**an operator is lexed as `scan.l` lexes one**: every `op_chars` run is one token, cut
where a `/*` or `--` opens inside it and trimmed of a trailing `+`/`-` unless the run
holds a character SQL's own operators do not use (`1 =- 1` is `1 = -1`; `2 ^- 1` names
`^-`), with the `self` characters and the four two-character rules (`<= >= <> !=`) ahead
of it and `=>` its own token. A spelling pglet's grammar has a kind for keeps it; every
other is the generic `Op` 18.6 lexes, whose name is its text, resolved at the operator
band (`gram.y`'s `%left Op OPERATOR`) in both the infix and the prefix form. pglet
implements `= <> < <= > >= + - * / % ^ || -> ->> #> #>> ? ?| ?& @> <@ ~~ !~~ ~~* !~~*
^@` (and `~` / `!~`, which answer over a pattern of `^` followed by literal characters
and refuse any other pattern `0A000` as a row is compared, `EDGES.md` row 251) and
refuses every other name `42883 operator does not exist: <left> <op> <right>` at the
statement's **analysis**, with 18.6's own sentence — deferred to the statement's end, so
a grammar error still outranks it. **Divergences** (`EDGES.md` row 184): the operators
18.6 has and pglet does not (`&` `#` `|` `#-` `@?` `@@`, prefix `~` and `@`, `<->`, and
`U&"…"`, whose `&` is an operator here), `42725 operator is not unique` answered `42883`
here, a **column** operand reading `unknown` in the message, and the refusal outranking
an operand's own `42703`/`42P01`. `OPERATOR(schema.op)` is not parsed

### Identifier case-folding

**an unquoted identifier is ASCII-downcased where the parser reads it, and a
`"…"`-quoted one is exact** — Postgres's rule, applied by `lexer::Token::name` for an
identifier-position token (the parser's name-producing reads go through it) and by
`split_name_literal` for a name literal, so the comparisons after it are plain byte
comparisons. `Foo` / `foo` / `FOO` are one name and `"Foo"` is another; a **keyword**
used as a name folds too (`CREATE TABLE Sequence` creates `sequence`), since Postgres's
scanner downcases before classifying. The fold is **ASCII-width** — at the parser, and
at the post-parser sites that re-fold a catalog type key (`execute_create_type`,
`execute_drop_type`, `resolve_column_type`) — because the oracle folds `A`–`Z` and
nothing else: measured on it, and matching the source, where `scansup.c`'s
`downcase_identifier` folds a high-bit byte (through `tolower`) only when the database
encoding is **single-byte**, so a multibyte encoding takes only the ASCII arm. That is a
property of the oracle's encoding (UTF-8), not of Postgres in general — a single-byte
database is the case that could answer otherwise, and pglet models no per-database
encoding. Measured
with no quoting anywhere: `CREATE TYPE CAFÉ AS ENUM ('a'); CREATE TYPE café AS ENUM
('b')` registers two types on both engines (18.6's `pg_type` holds `cafÉ` and `café`),
and `CREATE TYPE ZÄP AS ENUM ('a'); CREATE TABLE zt (c zäp)` is `42704` on both. What
those pin is a post-parser site folding *wider* than the parser: a Unicode
`to_lowercase()` collapses the first pair to one key — the second `CREATE TYPE` is then
`42710`, the column binds to the first type, and inserting the second type's label
reports it invalid — and accepts the second pair silently. The value grammars are a
separate question and are not covered by this: `lower()`, `upper()`, `ILIKE` and
`parse_bool` all fold Unicode here. A **name literal** (`nextval`/`currval`/`setval`'s
`regclass` argument) folds by the same rule inside `SplitIdentifierString`'s grammar, so
the two name positions always name one relation. A quoted **value-function** spelling is
skipped rather than matched (`SELECT "current_timestamp"` is `42703` on both, while
`SELECT "now"()` names a real `pg_proc` entry and resolves on both), and the
contextual-word matchers take only an unquoted token. `tests/fixtures/identifier/*` pins
both polarities; `crates/pglet/tests/case_folding.rs` carries the cases a differential
fixture cannot express. **Residual divergences:** a *quoted
type name* does not parse (`42601` here, `42704 type "INT" does not exist` there) — and
because the reference side cannot spell one, an **ENUM name is ASCII-downcased again
where it is stored**, which at that width reaches only a quoted one (an unquoted `name`
arrives from `Token::name` already folded, so re-applying the same fold is a no-op on
it): `CREATE TYPE "Loud"` + `CREATE TABLE t (m loud)` is accepted here and `42704`
there. That fold rescues a name differing from a bare identifier only in ASCII case and
leaves the rest — `CREATE TYPE MÖÖD` stores `mÖÖd`, `CREATE TYPE Sequence` stores
`sequence`, and each makes the whole database unexportable (see `EDGES.md`); a **quoted
lower-case function name** pglet models as a function but 18.6 has only as grammar
resolves here and is `42883` there (`"coalesce"`, `"nullif"`, `"greatest"`, `"least"`,
`"trim"` measured; `"substring"`, `"count"(*)`, `"now"()`, `"upper"`, `"lower"`,
`"gen_random_uuid"()` agree, each naming a real `pg_proc` entry), so a **parenthesized**
quoted name is a `pg_proc` lookup there and pglet's function table does not record which
of its names `pg_proc` holds. `SqlValueFunction::from_name`'s arms are `current_date`,
`current_timestamp`, `current_time`, `localtime`, `localtimestamp`, `current_user`,
`session_user`, `user`, `current_role`, `current_catalog` and `current_schema`, and
dropping `parse_identifier_primary`'s `!quoted` guard resolves `SELECT
"current_timestamp"` here. The **non-temporal** value functions are modeled (see
*Session information functions* below), and their quoted spellings agree with
18.6: `SELECT "current_schema"`, `SELECT "current_user"`, `SELECT "user"` and `SELECT
"now"` are `42703` on both, since a quoted word is never a keyword; no `NAMEDATALEN`
truncation (self-consistent, so both name positions agree, and `name` (OID 19) as a
declared type adds no truncation by itself — nothing pglet declares `name` can exceed 63
bytes); and `CREATE SCHEMA "PUBLIC"` is pglet's documented `0A000` where 18.6 creates a
second schema — a **loud** divergence, not a silent no-op

### INTERVAL

real column type over an in-house 3-field `PgInterval { months: i32, days: i32, micros:
i64 }` (the Postgres layout). Three input grammars (verbose `1 year 2 mons`/`ago`,
SQL-standard `1-2`/`3 4:05:06`, ISO-8601 `P1Y2M3DT4H5M6S` with `M`-before/after-`T`) +
fractional-unit normalization; `postgres`-style output (signed pluralization,
`mon`/`mons`, `+`-force on a positive-after-negative group, zero → `00:00:00`); 16-byte
binary wire (time→day→month, OID 1186, array 1187); order-preserving index key.
**Comparison is Postgres's estimate** (`micros + (months*30 + days)*86400e6`, i128) — so
`INTERVAL '30 days' = INTERVAL '1 mon'`, a UNIQUE interval index dedups them, and ORDER
BY / GROUP BY / DISTINCT fold identically. Arithmetic: `interval ± interval`, unary `−`,
`interval ×/÷ number` (fractions cascade down, byte-faithful to
`interval_mul`/`interval_div`), `timestamp ± interval` (3-pass calendar with leap-aware
day-of-month clamp), `date + interval → timestamp`, `timestamp - timestamp → interval`
(via `justify_hours`). Numeric fields parse exact (i64, not f64); exponent forms
rejected. Differential-tested (`types/interval_*.sql`). **`interval(p)` sub-second
precision is applied** — `'1.23456 secs'::interval(3)` is `00:00:01.235` on both
engines, at a cast, at a column store and element-wise under a `[]`, rounding only the
`micros` field and leaving months and days alone (`EDGES.md` row 123). **Divergences:**
an `INTERVAL <fields>` **range** spec fails loud at a column and at the multi-token cast
spellings, while a single-field cast spec is swallowed as a column alias — `'1 year 2
mons'::interval year` answers `1 year 2 mons` here against 18.6's `1 year`. No
`infinity`/`-infinity`: the text spelling and the binary all-maximum / all-minimum field
sentinels 18.6 reads as them are both `22007` here, the binary one raised at `Bind`,
where it fails an open block (`EDGES.md` row 139); the same triples spelled as numbers
or reached by arithmetic are finite intervals here (`EDGES.md` row 143). SQLSTATEs match
PG: malformed → `22007`, a parse-time field overflow (`interval '999999999999 years'`) →
`22015`, an arithmetic overflow (`interval × n`, `timestamp ± interval`) → `22008`. Not
DST-aware — pglet is UTC-only, so a day is always 24 h

### DATE

real column type: `Value::Date` rendered `YYYY-MM-DD` (text) / i32 days-from-2000-01-01
(binary), B-tree-indexable (chronological order, enforced UNIQUE), full
`=`/`<`/`>`/range + `ORDER BY`, casts (text↔date, `timestamp::date` truncates the
time-of-day, date→timestamp midnight), a comparison literal coerces to date (`WHERE d =
'2024-06-15'`), and bound `$N::date` params (inferred from column context). Arithmetic:
`date ± int → date`, `date - date → int4`, `date ± interval → timestamp`; `CURRENT_DATE`
(value function) + `DEFAULT CURRENT_DATE`. Differential-tested vs Postgres
(`types/date_*.sql`, `types/current_date.sql`). Input accepts 1-or-2-digit month/day
(`2026-1-1`, like PG); a malformed spelling → `22007` (`invalid_datetime_format`), a
shaped-but-out-of-range field (`2026-13-01`, `2026-02-30`) → `22008` — both matching PG.
**Domain:** a `date`, `timestamp` or `timestamptz` value is `0001-01-01` through
`9999-12-31` (a `timestamptz` read in UTC), held at the producers the domain section
lists — text and binary input, `date ± integer`, `± interval`, `date + time`,
`date_trunc`, the rounding carries, the clock reads, the date/timestamp casts — and at
the doors: a `Value` bound through `execute_bound` and a write's stored value, a
`timestamp` brought to UTC and a whole microsecond at both, and the dump, which refuses
a stored **row** holding such a value or a timestamp stored off UTC — a stored
expression's text is not asked, so a `CHECK` or partial-index constant in year 0 over an
empty table is still accepted (`EDGES.md` row 51) (`docs/temporal.md` § The date and
timestamp domain). Where PG refuses too the answer is PG's at the members measured — a
year-0 field in a literal that parses is `22008 date/time field value out of range:
"…"`, and `date ± integer` / `± interval` past PG's own range are `22008 date out of
range` / `timestamp out of range` — and the section's table lists the measured members
where it is not. `± interval` checks its months and days passes against PG's own range
rather than the domain, so `date '9999-12-31' + interval '1 day -1 microsecond'` is a
value on both. **Divergences:** a leading-`-` year → pglet `22007` (pglet models no BC)
vs PG `22009` where PG reads the sign as a time zone displacement (`'-2026-06-15'`),
both `22007` at `'-0001-01-01'`; a result PG represents but the domain excludes — a `BC`
value, or a year past 9999 — is refused `22008 date out of range` / `timestamp out of
range` where PG answers it (`EDGES.md` row 142). **`infinity` / `-infinity`** for
`date`, `timestamp` and `timestamptz` — text, binary sentinels, order, index, casts,
arithmetic, `date_part`, `isfinite`, `COPY`, the dump — agree with PG at the members
`docs/temporal.md` § `infinity` and `-infinity` measures; it also names the divergences
measured, among them `0A000`s where PG answers an infinite `interval` or `numeric`, `±
interval` at the interval field triples PG reads as infinite (`EDGES.md` row 143), and
the spellings PG's tokenizer accepts beside a punctuation byte it treats as a delimiter
or beside `AD`/`BC`

### NUMERIC / DECIMAL

real column type over an **in-house base-10 bignum** (`PgNumeric`, dep-free): exact
arbitrary-precision decimal with a separate display scale and a `NaN` that sorts above
every finite value and equals itself. Text (`1.50` keeps its scale) / Postgres binary
(`numeric_send`, base-10000 regroup) / order-preserving B-tree key (indexable, enforced
UNIQUE — `1.5` collides `1.50`). Comparison + `ORDER BY` are exact (never via `f64`;
numeric-vs-int exact, numeric-vs-float via the numeric's double). Arithmetic `+ − * /`
with Postgres's per-operator scale rules (add/sub `max` scale, mul scale-sum, div
`select_div_scale`), unary `−`, and `SUM`/`AVG` exact. Casts text/int↔numeric (exact),
numeric↔float (lossy double), numeric→int (half-away, loud `22003` overflow).
`NUMERIC(p,s)` rounds to scale on store and precision-overflows loud (`22003`); the
scale may be **negative** (`numeric(5,-2)` rounds to hundreds). The **modifier itself**
is validated where `numerictypmodin` validates it rather than dropped when declined — a
precision outside `1 ..= 1000`, a scale outside `-1000 ..= 1000` and any arity but one
or two are `22023`, behind the `int4` coercion every component goes through first
(`22003` out of range, `22P02` not an integer), in 18.6's measured order (`EDGES.md` row
124, `docs/coercion.md` § A modifier the type refuses). Functions
`abs`/`ceil`/`ceiling`/`floor`/`round(x[,s])`/`trunc(x[,s])` are exact for a numeric
argument; on an **integer** argument `ceil`/`floor`/`round(x)`/`trunc(x)` resolve to the
Postgres `double precision` overload (result `float8`, coupled type+value — `ceil(5)` is
`5.0`, not `5`), while the two-arg `round(x,s)`/`trunc(x,s)` stay `numeric`; a
`CASE`/`COALESCE` int/float literal branch unifies to a numeric sibling. An
**unknown-typed** argument resolves to the numeric category's *preferred* type,
`float8`, for every member including `abs` — `typeconv.sgml`'s own worked example for
the rule — so `SELECT abs(NULL)` is `double precision` on both legs. Only `abs` makes
the rule observable, since it is the one member whose rule passes the argument type
through (`types/math_fn_unknown_arg_declared_types.sql`). **That holds for a
*directly*-`NULL` argument, not for the family** — the executor arm routes
through `Literal::infer_type`, so `Literal::Null` is the only argument shape it can call
untyped, and an argument that is merely **transitively** untyped still declares 25 on
the simple query against `Describe`'s 701: `abs(-NULL)`, `abs(CASE WHEN true THEN NULL
END)` and `abs(COALESCE(NULL, NULL))`, all three of which 18.6 refuses (`42725 operator
is not unique: - unknown`, then `42883 function abs(text) does not exist` twice). Silent
here and loud there, the value being NULL either way; recorded in `EDGES.md`.
`abs(NULL::int)` is the control — 23 on both legs and both engines. **Two further
residual divergences in the same neighbourhood, neither closed:** `trunc(NULL)` is
18.6's `42725 function trunc(unknown) is not unique` (its overload set spans more than
the numeric category) and resolves to `float8` here — pglet models no overload-ambiguity
check; and `abs('1')` / `round('1.5')` are `701` answering `1` / `2` on 18.6 and `0A000`
here, because a bare string literal types as `text` rather than staying Postgres's
`unknown` (the general literal-typing gap this repo records elsewhere), which also makes
their `Describe` declare `text`. Both loud. A decimal/exponent literal (or an
out-of-`i64` integer) is `NUMERIC` (Postgres constant typing), carrying exact scale.
Bound `$N::numeric` params. `NUMERIC(p,s)` typmod is enforced on **both** the column
form (store) and a **cast** (`x::numeric(p,s)` rounds to scale + `22003`,
context-insensitive) via the shared `apply_typmod`. A `numeric(p,s)[]` carries the
**element's** typmod and `apply_typmod` walks the elements, so such a column rounds
`'{1.25}'` to `{1.3}` and refuses a bare `1.25` with `42804`, as 18.6 does (`EDGES.md`
row 120, closed). `%` (modulo, exact truncating), `^` (integer-exponent via
repeated-squaring), `sqrt` (Newton + exact ½-ulp rounding), `sign`, and digit-group
underscores in a numeric **literal** (`1_000`) are supported; inside a string read as a
number (`'1_0'::numeric`, `'0x10'::int4`) an underscore or a radix prefix is `22P02`
where 18.6 answers (`EDGES.md` row 189). **Gap:** a **fractional** numeric exponent (`2
^ 0.5`) → `0A000` (the ln/exp bignum lift is deferred); numeric `Infinity`/`-Infinity`
still absent (loud `22P02`). Differential-tested vs Postgres (`types/numeric_*.sql`)

### Session information functions

`version()`, the four role spellings (`CURRENT_USER` / `SESSION_USER` / `USER` /
`CURRENT_ROLE`), `CURRENT_CATALOG` / `current_database()`, `current_schema` /
`current_schema()`, `current_schemas(bool)` and `current_setting(name [, missing_ok])` —
every spelling a Postgres client can send for the session's identity, at 18.6's declared
type. That takes `name` (OID 19) and `name[]` (1003) as declared types: shape-free (a
`Value::Text` plus the OID, binary = raw bytes with no NUL padding, exactly as `varchar`
is modeled), in Postgres's **string** type category — which is two predicates, not one:
`planner::legality`'s `is_string`, so `owner = current_user` over a `text` column and
`INSERT INTO t(txt) VALUES (current_user)` both resolve while `current_user = 1` stays
`42883`; and `planner::common_type`'s `type_category` plus `name`'s implicit `pg_cast`
rows (`name → text`, `text → name`, `varchar → name`), so a common-type fold over `name`
and a string sibling resolves `name` (19) as 18.6 does instead of refusing the pair, and
with **no column form** — `type_oid_from_name` maps neither, so `CREATE TABLE t (a
name)` and `'a'::name` stay `42704`. **The parse split is 18.6's**: the six keyword
forms take no argument list (`current_user()` is `42601` on both), except
`current_schema`, which is a TYPE_FUNC_NAME_KEYWORD *and* `pg_proc` 1402, so both
spellings resolve and `current_schema(1)` is `42883`; the non-keyword names stay column
references when written bare (`SELECT version` / `current_database` / `current_schemas`
/ `current_setting` is `42703` on both). Every arity is refused as 18.6 refuses it — the
check is what keeps an unimplemented arity from becoming an acceptance. Each form is
**persistable**, so `DEFAULT current_user` is stored as a `ColumnDefault::Expr` and
`pg_get_expr`-rendered back as the written spelling: the six keywords bare and
upper-case, `current_schema()` **quoted** (`"current_schema"()`, since the bare word
re-lexes as the keyword), `current_database()` / `version()` as calls — all measured,
and all round-tripped by `dump`'s `ROUND_TRIP_CORPUS`. **What each answers:** the native
server echoes the startup packet's `user` and `database`, which is what a
`trust`-authenticated Postgres answers and what makes the differential fixtures
byte-comparable; the embed facade and the wasm bindings carry no packet and answer
`pglet` (`crates/pglet/tests/session_information.rs`). `current_setting` reads the
**same** table `SHOW <name>` answers from, so it raises the identical `42704` for an
unknown name, `missing_ok = true` turns that into NULL, and it is strict in both
arguments. **Divergences:** `current_schema` / `current_schemas` /
`current_setting('search_path')` all read the fixed search path, so `SET search_path`
moves 18.6's answers and not pglet's (`EDGES.md` row 66); no roles, so `SET ROLE` cannot
split `current_user` from `session_user` (row 73); a bare string literal written for the
`boolean` argument is `42883` here where 18.6 coerces the `unknown` (row 74); a `name`
operand beside an **enum** column is refused as 18.6 refuses it, the identity
reaching the branch, comparison and common-type gates alike (row 114, closed);
`version()`'s banner is `PostgreSQL 18.6 (pglet) on <target>, 64-bit` — the release
agreeing with `SHOW server_version` by construction, the rest deliberately not 18.6's
build string; `current_setting('server_version')` is `18.6` against 18.6's `18.6 (Debian
…)` and `current_setting('TimeZone')` a fixed `UTC`, which no accepted `SET` moves (row
69). **Not modeled:** `pg_typeof` (its declared type is `regtype`, which is *not*
shape-free — binary is a 4-byte OID and text a type name — and its **value** is a static
type OID, the question pglet's two type walkers answer separately) and
`pg_backend_pid()` (a native-process fact the embed and wasm hosts have none of); both
stay loud, and `session/pg_{typeof,backend_pid}_is_not_modelled.sql` pin that.
Differential-tested end to end (`tests/fixtures/session/*`, both wire formats and both
declared-type legs). **Two unguarded surfaces, both recorded rather than closed:** the
value-function words (`SqlValueFunction::from_name`'s arms) are accepted unquoted in
every **name** position where 18.6 is `42601` — a `CREATE TABLE` column, an `INSERT`
column list, an `UPDATE`'s `SET` target, every word measured at each — and a column so
created is then unreachable by its bare spelling (`EDGES.md` row 75; `test_keywords`
cannot probe any of them because none is a lexer `Keyword`);
and `CREATE INDEX ix ON t ((current_user))` / `…
WHERE a = current_user` are accepted where 18.6 refuses `42P17 functions in index
{expression,predicate} must be marked IMMUTABLE`, joining
`now()`/`gen_random_uuid()`/`CURRENT_DATE` on the unguarded non-IMMUTABLE index surface
— and natively such a key depends on which connection inserted the row.

### SQL typed constants (`DATE '2024-06-15'`, `NUMERIC(5,2) '1.5'`, `DOUBLE PRECISION '…'`)

`gram.y`'s `AexprConst`, lowered to the cast node `'…'::type` is, so value, declared
type, a **spelled** typmod and deparse follow one path. The type name is read at
`ConstTypename`, not `Typename`: a parenless `char` / `character` carries no length
where a column or cast takes `char(1)`, so `char 'ab'` is `ab`. An identifier before a
string is a type name, never a column; and `float` before a `(` is a type name, never a
function call — a window the speculative read commits to. Residuals, that commit and the
`INTERVAL '…' <field>` qualifier: `docs/parser.md`.

### `x IS [NOT] TRUE / FALSE / UNKNOWN`

Postgres's `BooleanTest`, never NULL; operand gated `42804` by `planner/legality.rs`, an
`unknown` literal converted (`22P02`). Deparsed faithfully, so a stored `CHECK`
round-trips.

### TIME

real column type: `Value::Time` = µs-since-midnight (raw `i64`, so `24:00:00` is a
representable endpoint), `HH:MM:SS[.ffffff]` text (trailing-zeros trimmed) / `int64`
binary (OID 1083, array 1183) / plain big-endian index key (chronological, enforced
UNIQUE), full `=`/`<`/`>`/range + `ORDER BY`. Input accepts `HH:MM:SS[.f]`, `HH:MM`,
`HHMMSS`/`HHMM` (incl. a fractional suffix), and 12-hour `AM/PM`. Casts text↔time and
`timestamp::time` (truncates to time-of-day). Arithmetic: `time - time → interval`,
`time ± interval → time` (µs-only, **wraps mod 24 h** — days/months ignored), `date +
time → timestamp`. `CURRENT_TIME`/`LOCALTIME` value functions + `DEFAULT CURRENT_TIME`
return plain `time`. Differential-tested (`types/time_*.sql`). A malformed spelling →
`22007`, a well-formed but out-of-range field (`25:00:00`, `12:99`) → `22008` — both
matching PG. **Divergences:** a tz suffix on a time literal fails loud `22007` (PG
accepts-and-ignores it; pglet is UTC-only); `CURRENT_TIME`/`LOCALTIME` report as `time`
(OID 1083), not PG's `timetz` (1266) — so a bare `SELECT CURRENT_TIME` isn't directly
differential-testable (fixtures use deterministic predicates); `CURRENT_TIME(p)` parens
rejected `42601`. `TIMETZ` unimplemented (fails loud `42704`), with a modifier as
without one — `timetz(3)` and `time(3) with time zone` are the same `42704`, because the
canonical name carries the precision **last**. **`time(p)` sub-second precision is
applied**: `'03:04:05.6789'::time(3)` is `03:04:05.679` on both engines, and a rounding
carry reaches the `24:00:00` endpoint rather than wrapping (`EDGES.md` row 123)

### bytea

real `Value::Bytea` column type — text input read as 18.6's `byteain` reads it (the hex
format behind a lowercase `\x`, whitespace allowed between digit pairs; the escape
format otherwise — the text's bytes, `\\` and `\ooo` — with 18.6's `22023` for a bad or
unpaired hex digit and `22P02` for an invalid escape; `EDGES.md` row 279, closed, lists
the doors measured and the residuals: a default string the hex format refuses is
accepted by `CREATE TABLE` / `SET DEFAULT` and `22023` at each `INSERT` that takes it),
hex text output, full binary wire, byte-count `length()`, B-tree-indexable / UNIQUE.
`bytea || bytea` and `bytea LIKE bytea` are Postgres-legal and unimplemented → `0A000`,
and beside a bare string literal a `text` result for `||` (the `||` row; `EDGES.md` row
293) and `42883` for `LIKE`, where 18.6 reads the literal as `bytea` and answers (`EDGES.md` row 262)

### Real prepared statements / param type inference

`$N` typed from usage context (cast/column/`ANY`/`@>`/`IN`/`LIMIT`/`||`/`->` key),
decoded to a `Value`, bound into the plan; reported via `ParameterDescription`. A type
the client declares at `Parse` is the parameter's type for parse analysis, as on 18.6 —
operators, common-type resolution, the result column (`docs/param_binding.md`;
`EDGES.md` rows 145–146, closed). The full record: § Prepared statements / param
inference — details, below.

#### Prepared statements / param inference — details

Column context is alias-aware across a JOIN (a per-scope FROM alias→table map) and sees
through `DISTINCT`/`Window` wrapper nodes, so a join-ON array param under `SELECT
DISTINCT` types (`rg.scope_id = ANY($2)`, the spine's scoped `actor_search`) — **while
the alias names no real relation**: the catalog lookup runs ahead of that map, so a FROM
alias colliding with a table name reports *that table's* column type (see `EDGES.md`). A
`||` concat operand, a `LOWER($N)` argument, a `->`/`->>` key operand (typed `text`,
matching Postgres's `(jsonb, text)` preference — the index form needs an explicit
`$N::int`), a bare `$N` in an `UPDATE … SET` value (incl. a `CASE`/`COALESCE` result
branch, typed from the target column), a bare `$N` at a **boolean position** (typed
`bool` — the clause roots and the node positions `Expr::for_each_child_ctx` marks, the
`IS [NOT] TRUE` operand excepted), and a filter on a **virtual-table** column
(`WHERE table_name = $1` over `information_schema.*`, resolved via the catalog view's
column list) all type rather than reporting OID 0 — an OID-0 param trips a
binary-protocol client (tokio-postgres) into a type-introspection failure
(`Parameters`), which a text-protocol client tolerates.

A `$N` parse analysis leaves **without** a type is refused as 18.6 refuses it —
`42P18`, or `42P08` where a later occurrence types it — and a bare select-list or
`RETURNING` entry resolves to `text` (OID 25), an `INSERT … SELECT` source's to its
target column's type, as there
(`docs/param_binding.md` § A parameter left without a type is refused; residuals,
`EDGES.md` row 276).

**Gap:** Postgres never reports OID 0 for a used param; pglet still does wherever its
inference has no rule for a position 18.6 types, and decodes the value as text (the
native server) or by its JS runtime type (the wasm binding) — see
`docs/param_binding.md` § Deferred / known gaps.

**Gap: there is no parameter-type *unification*, so pglet never raises `42P08` and a
`$N` written twice keeps one of its facts and discards the other — the
strongest-priority one, and at *equal* priority the one that reached it first.**
Postgres deduces one type per parameter across the whole statement and refuses when the
deductions disagree; pglet's `record` keeps the strongest-priority fact per slot and has
no consistency check at all. It is wider than any one clause — the members
below are measured over `oc_probe(id INT PRIMARY KEY, v TEXT, n BIGINT)`, each accepted
here and refused there, and the refusals arrive under three different SQLSTATEs —
`42883`, `42P08`, `42804` — which is why no single one of them names the class: `… DO
UPDATE SET n = $1 RETURNING (v = $1)` is `42883 operator does not exist: text = bigint`
there and `{20}` here; `… DO UPDATE SET v = $1 WHERE oc_probe.n > $1` is `42883 operator
does not exist: bigint > text` there and `{25}` here; `… DO UPDATE SET v = $1, n = $1`
is the bare `42P08 inconsistent types deduced for parameter $1 — text versus bigint`
there and `{25}` here; and the `UPDATE` member, `UPDATE oc_probe SET n = $1 WHERE v =
$1`, is `42804 column "n" is of type bigint but expression is of type text` there and
`{20}` here. Loud on 18.6, silent here: the statement runs with one operand coerced to a
type the user never wrote. An `ON CONFLICT` clause is walked too, at every position it
parameterizes, which are these — the arbiter's key parts and its partial-index
predicate, the `DO UPDATE` `SET` list (whose bare `$N` takes the assignment-target rule,
`CASE`/`COALESCE` branches included), and the action's `WHERE`. The **action** walks
against its own namespace — the target plus `excluded` carrying the target's shape
(`planner::plan`'s `on_conflict_action_relations`, the `RelationColumns` twin of the
`ColumnScope` `lower_on_conflict` gates the same two positions against) — and not
against the catalog, because the catalog answer is right only until a table is *named*
`excluded`: with a real `excluded(id INT, n TEXT, v TEXT)` beside the target, `… DO
UPDATE SET v = 'z' WHERE excluded.n = $1` reported the shadow table's `{text}` against
18.6's `{bigint}`, and the bound statement was then a loud `42883 operator does not
exist: bigint = text` where 18.6 updates the row — a false rejection out of nothing but
an ordinary table name. The **arbiter** is deliberately outside that namespace, on
18.6's own authority rather than out of caution: `ON CONFLICT ((excluded.id)) DO UPDATE
…` and `ON CONFLICT (id) WHERE excluded.n > 0 DO UPDATE …` are both `42P01 invalid
reference to FROM-clause entry for table "excluded"` there. The clause is walked
**after** the row source and **before** `RETURNING` (`transformInsertStmt`'s order), and
that order is observable rather than cosmetic, because `record` keeps the first fact at
equal priority: on 18.6 `VALUES (1,$1) … WHERE n > $1` is `42883 bigint > text` and `…
DO UPDATE SET v = 'z' WHERE oc_probe.n = $1 RETURNING (v = $1)` is `42883 text = bigint`
with the caret on the `RETURNING`, each the collision reported from the *later* clause.
The `RETURNING` example has to put its collision in the action's **`WHERE`** and not in
its `SET` list, or it stops discriminating the order at all: `coerce_to_type` wraps a
`SET` right-hand side in an `Expr::Cast`, which records at the cast priority and
outranks the `RETURNING` context fact whichever clause is walked first — the same escape
this row names for the arbiter-and-`SET` collision below. Measured both ways here: `… DO
UPDATE SET n = $1 RETURNING (v = $1)` is `{20}` with the clause walked before
`RETURNING` **and** `{20}` with it walked after, while the `WHERE` spelling is `{20}`
before and `{25}` after.

**The residuals on this clause**, each
measured against 18.6 over `oc_probe(id INT PRIMARY KEY, v TEXT, n BIGINT)`. *The
arbiter key expression*: it counts its `$N` without typing it — `ON CONFLICT ((id + $1))
DO NOTHING` reports OID 0 here against 18.6's `{integer}` — because pglet infers no type
across an arithmetic operator; the explicit `((id + $1::int))` spelling types, and 18.6
answers `{integer}` for both. *The arithmetic assignment right-hand side*, which is the
same missing rule in the position that matters most, because it is the canonical
upsert-increment idiom: `ON CONFLICT (id) DO UPDATE SET n = oc_probe.n + $1` and its
`SET n = excluded.n + $1` spelling both report `{0}` here against 18.6's `{20}`, and the
`UPDATE oc_probe SET n = n + $1` twin does the same. It is not merely an
unhelpful OID — the parameter then decodes as text and the statement fails at execute,
`42804 type error: operator + requires numeric operands` (measured through a text-format
`Bind`), where 18.6 adds and returns. So of the upsert spellings, `SET n = <rel>.n +
$1::int8` binds and `SET n = <rel>.n + $1` does not; the bare `SET n = $1` / `SET v =
$1` assignment binds, as does a comparison against a typed sibling (`WHERE excluded.n =
$1`). *The arbiter-and-`SET` collision*: a `$N` written in **both** takes the `SET`
list's type, where 18.6 keeps the arbiter's — `ON CONFLICT ((id + $1)) DO UPDATE SET v =
$1` is `{integer}` there and `text` here, and `ON CONFLICT (id) WHERE n > $1 DO UPDATE
SET v = $1` is `{bigint}` there and `text` here. This is not a walk-order fact and
reordering the walk does not touch it — `coerce_to_type` wraps an assignment right-hand
side in a plain `Expr::Cast`, indistinguishable from a user cast, so it records at the
cast priority and outranks the arbiter's context fact whenever it runs. It is one member
of the no-unification gap above, whose `UPDATE` spelling is `UPDATE oc_probe SET v = $1
WHERE n = $1`. *The ungated arbiter qualifier*: the arbiter is neither resolved nor
gated (see the *ON CONFLICT* row), and what it types off is **whichever relation the
qualifier names**, falling back to the target only when the catalog holds nothing by
that name — so it is not "loose toward the target" but a lookup in the whole database,
a shape the `RETURNING` clause's top-level walk, bound to its own namespace, does not
have. The measured pair, over
an `oc_other(id INT, n SMALLINT)` sitting beside the target and never mentioned in the
statement: `ON CONFLICT (id) WHERE oc_other.n > $1 DO NOTHING` reports `{21}` —
`oc_other`'s `int2`, which is what identifies *that* relation as the answerer rather
than the target — while `ON CONFLICT (id) WHERE zzz.n > $1 DO NOTHING` reports `{20}`,
the target's own `n` reached through the fallback. 18.6 refuses both, `42P01 missing
FROM-clause entry`, at Parse. Same class as the FROM-alias entry `docs/param_binding.md`
records for a plain `SELECT`, in the one clause pglet resolves no names in at all; the
`((zzz.n + $1))` key-expression spelling plans here and answers `42P10` at execute where
18.6 is `42P01` at Parse. The action's own **top-level** positions have no such hole —
`lower_on_conflict` resolves both halves against a **closed** namespace holding only the
target and `excluded`, so a bogus qualifier and the quoted `"EXCLUDED"` alike are
`42P01` on both engines before any plan exists to infer over. *An `excluded` reference
inside a **subquery** in the action* is the hole one level in, and it is a typing hole
rather than a gating one: `Inference::walk_scoped` clears the clause namespace at every
subquery boundary, and where a cleared `RETURNING` scope degrades to a catalog lookup
that re-finds an outer relation which *is* a catalog table, `excluded` is a catalog
table under no spelling and nothing re-finds it. Over `oc_probe(id INT PRIMARY KEY, v
TEXT, n BIGINT)` and `oc_src(id INT, v TEXT)`, `… DO UPDATE SET v = 'z' WHERE EXISTS
(SELECT 1 FROM oc_src s WHERE s.id = $1 AND excluded.n = $2)` reports `{integer,0}` here
against 18.6's `{integer,bigint}` — and `{integer,text}` with a real `excluded(id INT, n
TEXT, v TEXT)` beside the target, which 18.6 ignores. The control is the same comparison
written directly in the action's `WHERE`: `{bigint}` on both engines, shadow table or
not.

**Masked**, not harmless: at a conflicting key the action's `EXISTS` is pglet's own
`0A000 … requires execution context` at execute and the statement fails there, writing
nothing; at a free key the action is not evaluated at all and the `INSERT` succeeds on a
`VALUES` row `$2` does not reach. The wrong OID reaches a `Describe` reply and a `Bind`
decode, and no row carries a value read under it.

**No fixture can pin it**, and for a reason that outlives this shape: the harness
compares output-column declared types and never reads `ParameterDescription` at all, so
a parameter OID is visible to it only through a `Bind` that fails — and at a conflicting
key this statement does not survive execute on pglet to be compared. *And the all-unknown
`CASE`/`COALESCE` against a non-text column*, which is not this
clause's: it types from that column here (`SET n = COALESCE($1, $2)` → `{bigint,bigint}`)
where 18.6 folds the unknowns to text and refuses the statement `42804`; the `UPDATE`
spelling diverges identically

### `to_char` / `AT TIME ZONE`

the spine's ISO-8601 projection (`to_char(ts AT TIME ZONE 'UTC',
'YYYY-MM-DD"T"HH24:MI:SS"Z"')`); tokens YYYY/MM/DD/HH24/MI/SS + quoted literals, non-UTC
zone / unknown token fails loud

### `timestamptz` text wire output

renders with the UTC `+00` offset (`2024-06-15 12:30:45.123+00`); a plain `timestamp`
(no tz) has none. The offset is load-bearing for **text-protocol** drivers
(node-postgres, the fuz_app matrix) — an offset-less value is reparsed in the client's
local zone, silently shifting the instant. pglet has no session zone (always UTC), so
this is ≡ Postgres at `TimeZone=UTC`; the binary path is offset-correct. A
`timestamptz[]` element carries it too, rendered at the declared element type, and a
binary array's header names that element type. A bare `$N` the client declared
`timestamptz` is a `timestamptz` column with it. Input splits a spelling into fields as
`ParseDateTime` does and reads a numeric offset as `DecodeTimezone` does: an hour
outside `0 ..= 15`, or a minute or second outside `0 ..= 59`, is `22009`
(`docs/temporal.md`)

### `ErrorResponse` structured fields

every error goes out through **one** builder (`Error::wire_fields`) shared by the native
server's three send sites and the wasm host, carrying `S`/`V`/`C`/`M` plus
`D`/`H`/`s`/`t`/`c`/`n`/`P` where 18.6 does — byte-identical on `23502`, `23505`,
`23514` (both routes), `23503` (both directions), `23001`, `42809` (three of 18.6's five
wordings), `428C9`, `42601`'s `P` (a **1-based codepoint** index), and `2BP01` on three
of its five shapes — `DROP TABLE` with a dependent FK, `DROP COLUMN` with a dependent
sequence default, and `DROP SEQUENCE` for an identity column. The other two diverge on
purpose or by gap: `DROP INDEX` over a constraint's index sends a **deliberately
different** `H` (18.6 suggests `ALTER TABLE … DROP CONSTRAINT`, which pglet refuses),
and a non-`CASCADE` `DROP SCHEMA public` sends **no `D`** where 18.6 lists every
dependent. A field pglet cannot fill truthfully is **omitted**, never invented:
`F`/`L`/`R` always (no C source tree), the `Key (…)` description on an expression index,
the `Failing row` on a gate that runs before the row exists. Holding a name is not a
licence to send one — `t`/`c`/`n` go out at the five variants 18.6's `errtable*` macros
fill and nowhere else. The wasm host sets `pg`'s property names
(`.detail`/`.hint`/`.position`/`.schema`/`.table`/`.column`/`.constraint`), which is
what pglite exposes. **Gaps:** `P` outside `Error::position`'s arms, the near-name `H`
on `42703` and the escape `H` on a malformed Unicode escape's `22025`, the `D`+`W` pair
on a jsonb `22P02` and the `D`+`W` on its `22P05`, the `W` on COPY's and a bound
parameter's errors, the `D` on a `numeric` `22003`, the `DROP SCHEMA` dependent list,
the `D` on a `CREATE TYPE … AS ENUM` repeated label, and the `WARNING`/`25P01` notice
for `COMMIT`/`ROLLBACK` outside a block (`EDGES.md` row 76; the notice channel carries a
message string with no severity or SQLSTATE of its own). `docs/errors.md`

### `jsonb_build_object`, `IS [NOT] DISTINCT FROM`

spine `cell_list`/audit; `IS [NOT] DISTINCT FROM` is null-safe (always boolean).
`jsonb_build_object` renders a boolean, number, text, `NULL` and `jsonb` value as its
JSON kind and every other value as a JSON string of its text output — 18.6's rendering
for a `date`, `time`, `interval` and `uuid`, and not for a `timestamptz` / `timestamp`
(no `T`, no offset) or an array (a string, where 18.6 emits a JSON array): `EDGES.md`
row 306.

### Scalar `EXISTS` / `NOT EXISTS`

non-correlated (`query_has_fact`, incl. no-FROM projection) and **correlated** —
orphan-fact `NOT EXISTS` and cell-authz `EXISTS (… g.cell_id = c.id)` both run
end-to-end (eval-time outer-scope resolution via `EvalEnv` + alias-aware scoping). The
sub-plan runs to its end rather than stopping at its first row, so an error in a later row
is raised where 18.6 answers (`EDGES.md` row 320).

### `INSERT … SELECT` + `unnest($N)` in a SELECT projection

`put_fact_refs` end-to-end; projection-`unnest` row-expands (no-FROM); the native parser
handles the no-FROM `… SELECT … ON CONFLICT` shape directly. **Divergence: the no-FROM
restriction is a real one** — only that shape lowers to `Plan::ProjectUnnest`; with a
`FROM` the expression stays in a plain `Plan::Project` and is `0A000 set-returning
unnest() is only supported in a SELECT projection` as soon as a row reaches it, for an
**array** argument as much as any other, where 18.6 expands. Measured over `mwa(arr
INT[])` holding `ARRAY[1,2]`: `SELECT unnest(arr) FROM mwa` and `SELECT
unnest(ARRAY[1,2]) FROM mwa` are `1`/`2` there and `0A000` here, as are the CTE and
`VALUES`-derived-table spellings; empty, both answer `(0 rows)`. Loud, so no fixture is
red over it (`EDGES.md` row 55's bullet). A **non-array** argument does not reach
this refusal — it is `42883` at plan time, matching 18.6

### `generate_series` in `FROM`

`FROM generate_series(start, stop[, step]) [AS] g[(col)]` is a FROM item on the node the
FROM-clause `unnest` uses (`Plan::FunctionScan`). **What answers:** the `integer` and
`bigint` overloads, resolved as 18.6 resolves them over every argument-type combination
measured — two or three arguments of `smallint`, `integer`, `bigint` or unknown type, the
wider of `integer` and `bigint` written choosing, and a string literal, a bare `NULL` or a
bare `$N` taking the overload's type (`generate_series(1, '3')`, `generate_series(1, $1)`
inferring `integer`, `bigint` beside a `bigint`); `42883` for no signature, and `42725`
where neither is written (two unknowns, two `smallint`s). The rows
are 18.6's: a negative step, an empty series, a series ending at the type's edge rather
than wrapping, a NULL argument as zero rows ahead of a zero step's `22023`. The relation and
column names follow `chooseScalarFunctionAlias` (`generate_series` unaliased, the alias
under `AS g`, the listed name under `g(i)`), `g.*` and a non-sole `*` expand the one
column, and the column's declared type is the overload's on both wire formats. An
argument is evaluated once per execution, and a plan `execute_bound` holds keeps it as an
expression, so a bound value is read each run.

**The bound.** A series longer than **1 000 000 rows** is refused `54000` from its evaluated
arguments before a row is built: a FROM item's rows are materialized before the node above
reads them, as 18.6's tuplestore materializes them, but without its spill to disk.

**What is refused** (`EDGES.md` row 321, with the measured members): the `numeric` and
timestamp series (`0A000`); an argument naming a column — an earlier `FROM` item's, which
18.6 reads row by row, or an enclosing query's — or holding a subquery (`0A000`), the item
being evaluated once with no row; an argument whose type the certainty reader does not
give (`$1 + 1`, `nextval(…)`: `0A000`); `LATERAL`, `WITH ORDINALITY` and `ROWS FROM`
(`42601`); and `generate_series` in the select list or any other expression position
(`0A000`). A zero step, and a series past the bound, are refused under a `WHERE false` or
`LIMIT 0` that 18.6 does not run the series beneath (row 320); two items sharing a name are
row 322's; a `$N` compared with the series' column takes no type from it (row 288).
Fixtures: `generate_series/`; the bound, the refusals' codes and the plan cache:
`crates/pglet/tests/generate_series.rs`, `crates/pglet/src/plan_cache.rs`.

### pg_catalog introspection (drizzle-kit push etc.)

`pg_constraint` (22 of 28 columns, all five `contype` kinds incl. `n`) + `pg_index` (15
of 21) + `pg_enum` + `pg_get_constraintdef(oid)` + `::regclass` + `col_description(oid,
int4)` (always NULL: `COMMENT ON` is a no-op, `EDGES.md` row 15) +
`pg_get_serial_sequence(text, text)` + `has_schema_privilege(text, text)` +
`quote_ident(text)` (18.6's keyword table, not pglet's lexer) — the calls kysely's
introspector makes (`EDGES.md` row 252); `pg_class` (9 of 34, with `relkind` `r` and `i`
rows), `pg_namespace` (`pg_catalog` + `public`), `pg_attribute` (12 of 25), `pg_type`
(10 of 32: every scalar pglet models **plus** its array type and every user enum;
`char` / `oid` / `name` carry 18.6's real `typelem` / `typarray` rather than a `0` that
would claim they have no array type) partial; `pg_tables` / `pg_indexes` complete
(`indexdef` re-emits an expression key and a partial index's `WHERE`, as
`pg_get_indexdef` does; `hastriggers` is computed from the FK set on **both** sides,
which is exactly 18.6's value for a trigger-less engine). `pg_constraint` / `pg_index` /
`pg_enum` / `pg_tables` / `pg_indexes` bind exhaustively, so an unmodelled column is
`42703` rather than a fabricated zero-row `text`. Not modelled, each for want of a type or
model pglet lacks (`int2vector`, `oidvector`, `pg_node_tree`, an opclass and collation
model): `pg_constraint`'s `conbin`, `conpfeqop`, `conppeqop`, `conffeqop`,
`confdelsetcols` and `conexclop`, and `pg_index`'s `indkey`, `indcollation`, `indclass`,
`indoption`, `indexprs` and `indpred`. Of the `reg*` types only `regclass` is modelled, as
a cast: the parser's intercept matches `regclass` alone, and every other spelling falls to
the type-name table and is `42704` (`regtype`, `regproc`, `regnamespace`, `regrole` and
`regoperator` measured). `conkey` for a CHECK is the
predicate's **walk** order, as `pull_var_clause` yields it (`CHECK (qty > 1 AND pid >
0)` is `{3,2}`, not `{2,3}`). `oid` and `"char"` are declaration types over `int4` /
`text`, so the operand rulebook keeps them out of arithmetic and out of the math
functions (`conrelid + 1`, `- conrelid`, `abs(conrelid)` are all `42883`, as on 18.6,
and so are the same three reached **through a scalar subquery** and **through an
aggregate** — `max(conrelid) + 1`, `- max(conrelid)` and `abs(max(conrelid))`) while
leaving their comparisons; an unknown literal coerced to `"char"` truncates. A rendered `AND`/`OR`
chain is N-ary, as `pg_get_constraintdef` spells it, and an `IN` list renders as the
`ScalarArrayOpExpr`/`OR`-chain node `transformAExprIn` builds for it, in all five of its
shapes (`docs/deparse.md`; `EDGES.md` row 88 closed, row 90 the live residual); the fold
and assignment contexts are pglet-stricter (row 87). `pg_indexes.indexdef` sends its key
columns through `quote_ident`, whose **keyword half is not 18.6's**: a column named
`position` reads `(parent_id, "position")` on both engines, while one named `action`
reads `(parent_id, "action")` here against 18.6's bare `(parent_id, action)`, and
`pg_get_constraintdef` parts the same way (`CHECK (("action" > 0))` against `CHECK
((action > 0))`) — measured. The predicate, both directions of the disagreement and how
to re-run its extent are `EDGES.md` row 85 (f), which owns the rule. **Divergences:** an
index's own name and its table's are interpolated **raw** into `indexdef` where 18.6
renders both through `quote_identifier`, so every name 18.6 quotes reads bare here, and
what that costs is not one thing — text that does not parse at some names, text that
resolves a *different* relation at others, and at others only a spelling a schema differ
compares (`EDGES.md` row 122); a sequence gets no `relkind = 'S'` row (pglet's
`Sequence` carries no OID); `pg_constraint.oid` is synthesized positionally, so it
shifts when a constraint is added or dropped; `public`'s namespace OID is hardcoded
2200, where 18.6's is per-database; an enum column's `attlen` follows 18.6's
representation (4) rather than pglet's text storage

### `information_schema`

`tables` (one row per user table, `table_type` always `BASE TABLE`) + `columns` (with
`column_default` rendered to Postgres's spelling — `gen_random_uuid()` / `now()` /
`'success'::text` / `nextval(…)`, short of a sequence name that needs quoting, which is
written bare inside the literal: `EDGES.md` row 286) + `table_constraints` (CHECK /
FOREIGN KEY / **PRIMARY KEY** / **UNIQUE**) + `key_column_usage` /
`referential_constraints` (`delete_rule` from `ON DELETE`, `update_rule` from `ON
UPDATE`) / `constraint_column_usage`, all matched on the schema-qualified name so a bare
user table isn't shadowed. The **PK/UNIQUE rows** come from the catalog `Index`'s
constraint marker (`Uniqueness::constraint`, persisted): an
index created for a declared `PRIMARY KEY` / `UNIQUE` *is* the constraint, under the
same name, while a standalone `CREATE UNIQUE INDEX` is absent — Postgres's own split. A
user-declared `CONSTRAINT <name>` is reported under **that** name (all three spellings:
table-level, column-level, `ADD CONSTRAINT`), since the constraint's index is minted
under it; a column-level `CONSTRAINT <n> CHECK (…)` likewise keeps its name.
`key_column_usage` lists a PK/UNIQUE's key columns in
**constraint key order** with `position_in_unique_constraint` NULL (that column pairs an
FK's columns with the referenced key); `constraint_column_usage` lists them as the
constraint's own table+columns. `referential_constraints` stays FK-only — it has no
other meaning. A bound `$N` filter on a virtual-table column (`WHERE table_name = $1`,
the spine's `auth_tables_exist` probe) types from the catalog view's column list (text),
so a binary-protocol client does not see OID 0. `sequences` (every sequence that is
not an identity column's — 18.6's INTERNAL-`pg_depend` exclusion, read here off the
owner column's identity flag) and `check_constraints` (CHECK predicates plus the `NOT
NULL`s as `<col> IS NOT NULL`) are modelled, and `table_constraints` /
`constraint_column_usage` carry the `NOT NULL` and CHECK rows. `columns` carries
`udt_name` (an enum column reports its type's name with `data_type = USER-DEFINED`),
`is_identity` / `identity_generation` / `is_generated`, `character_maximum_length`,
`numeric_precision` / `_radix` / `_scale` and `datetime_precision` (a fact about the
**type**: `6` for a bare `timestamp`, `0` for a `date`, NULL for an array column and for
every non-temporal one); an identity column's `column_default` is SQL NULL, as 18.6's
is. `column_default` is rendered by the deparser's **catalog mode**, which prints what
`pg_get_expr` prints for the tree 18.6's parse analysis would have stored rather than a
faithful rendering of pglet's own — the cast folds, the constant labels, the
`unknown`→operand-type resolution, and `transformAExprIn`'s `IN` rewrite, all of
`docs/deparse.md`. So `DEFAULT '5'::text` reads `'5'::text` (not `('5')::text`),
`DEFAULT -1` reads `'-1'::integer`, `DEFAULT gen_random_uuid()::uuid` reads
`gen_random_uuid()`, and a string literal takes the **column's** type name — the enum's
own for an enum column, `character varying` for a `varchar(n)` one.
**Divergences:** `columns` reports a `char(n)` column as `character varying` — and
that is a type-system divergence, not a description one (`EDGES.md` row 95) — and
**renumbers** later columns' `ordinal_position` on a mid-table `DROP COLUMN` (PG
tombstones the gap); every `information_schema` column pglet models as a **string** is
declared `text`, where 18.6's `RowDescription` carries the domain's **base** type —
`check_constraints` is `[19, 19, 19, 1043]` there against `[25, 25, 25, 25]` here, so
pglet reports neither the domain nor the type under it. The **numeric** columns are not
in that class and agree: across the modelled views, the integer columns declare `int4`
(23) on both engines — `columns`' `ordinal_position` / `character_maximum_length` /
`numeric_precision` / `numeric_precision_radix` / `numeric_scale` /
`datetime_precision`, `key_column_usage`'s `ordinal_position` /
`position_in_unique_constraint`, and `sequences`' three `numeric_*` (measured over the
wire); and the four rendering classes `docs/deparse.md` § What is recorded, not
reproduced names — a nested implicit coercion, `CASE`'s indentation, an output
function's normalisation, and an enum literal outside a `DEFAULT` (`EDGES.md` rows
90/91/93/94)

### `ALTER TABLE … ADD CONSTRAINT`

`CHECK` / `PRIMARY KEY` / `UNIQUE` / `FOREIGN KEY` (single and composite) — each
validates existing rows, then records + enforces it (visible in `table_constraints`).
The full record: § ALTER TABLE … ADD CONSTRAINT — details, below.

#### ALTER TABLE … ADD CONSTRAINT — details

**A *declared* constraint name is unique per table across all four kinds**, matching
Postgres's one unique index on `pg_constraint (conrelid, conname)`: a `CHECK` / `FOREIGN
KEY` / `PRIMARY KEY` / `UNIQUE` name already taken by *any* of the four on that table
fails loud `42710` (`constraint "<n>" for relation "<t>" already exists`) — on the
inline `CREATE TABLE` spellings (table-level and column-level) as well as `ADD
CONSTRAINT` and `ADD COLUMN`. An **auto**-generated name is never gated, only
disambiguated (see the mint rule below). Two same-named FKs on one table are what the
gate exists to prevent — a later `DROP COLUMN` would shed **both** — but the gate alone
does not make that state unreachable, so the DROP-time resolver keys on `(table,
position)`, never on a name. An auto-name never meets the declared-name gate: `ALTER
TABLE t ADD FOREIGN KEY (a) REFERENCES p(id)` issued twice builds `t_a_fkey` +
`t_a_fkey1`, as Postgres does, and `ADD COLUMN … REFERENCES` and an unnamed CHECK claim
their names in the same pools as every other route. The gate spans the kinds, so
`CONSTRAINT n CHECK (…), CONSTRAINT n PRIMARY KEY (…)` is `42710` — a duplicated
`(table_name, constraint_name)`, unique by construction in Postgres, would be
undetectable to an introspecting client and dump to SQL stock Postgres refuses (`constraint
"n" for relation "t" already exists`). A PK/UNIQUE also occupies the **relation** namespace,
because its index carries its name, and `index_create` tests that one **first**: a
collision with a table or index is `42P07` (`relation "<n>" already exists`) and only
then is the constraint namespace consulted for `42710`. A CHECK/FK is in the constraint
namespace *only*, so a CHECK named after a plain `CREATE INDEX` is accepted on both
engines. The order across kinds is Postgres's too: the CHECKs' names are resolved first,
then the constraint indexes, then each FK in source order with its name checked
**before** its parent relation, so a missing parent on the first FK beats a duplicate
name on the second and a duplicate name beats a missing parent on the same FK. `PRIMARY
KEY (cols)` / `UNIQUE (cols)` / single-column `FOREIGN KEY … REFERENCES …` via `ADD
CONSTRAINT` **work** (the `pg_dump` restore shape): they validate existing rows
(`23505`/`23502`/`23503`) then create the same enforced unique index / FK that an inline
constraint creates; the `ONLY` modifier is accepted. The **declared constraint name is
kept** and becomes the index's name (Postgres names a constraint's index after the
constraint), so `information_schema`, `pg_indexes`, the `23505` message, `DROP INDEX`'s
`2BP01`, and the dump all agree on it; the `<table>_pkey` / `<table>_<cols>_key`
convention is only the default when no name is declared, and is not *read*
anywhere (`Schema::primary_key_index` asks the marker instead).

**A declared name is gated; an auto-name is disambiguated** — Postgres never lets one of
its own generated names fail a statement, so when `<table>_<cols>_key` is taken
`ChooseRelationName` moves a digit onto the label (`t_a_key1`, `t_a_key2`, …), every
candidate cut to 63 bytes as `makeObjectName` cuts it (`docs/names.md` § An unnamed index's
name), and `CREATE INDEX
t_a_key ON t (b); ALTER TABLE t ADD UNIQUE (a);` succeeds on both engines. The candidate
must clear the relation namespace, the table's constraint namespace, **and** every
constraint name in the schema (`ChooseRelationName`'s `isconstraint` pass, which
finishes with `ConstraintNameExists` — stricter than `pg_constraint`'s own index because
"the SQL standard says that constraint names should be unique within schemas").

**A CHECK's or FK's auto-name follows the same asymmetry through a different function**,
`ChooseConstraintName`, and the two search *different* spaces — a distinction that is
invisible in the SQL and silently changes which names collide if read backwards.
`ChooseConstraintName` scans `pg_constraint (conname, connamespace)`: schema-wide, all
four kinds, and **only** `pg_constraint`. So a `UNIQUE` named `t_a_check` on **another**
table pushes this table's next auto-named CHECK to `t_a_check1`, while a plain `CREATE
INDEX t_a_fkey` — a relation, not a constraint — does not push an FK's auto-name at all.
The **declared**-name gate is the per-relation `ConstraintNameIsUsed` ("deliberately not
the same test as `ChooseConstraintName` uses"). All of it goes through one
`ConstraintNames` gate at **execute** time — `mint_constraint` for CHECK and FK,
`mint_constraint_index` for a PK/UNIQUE name that is also its index's — and the
split is by *side* rather than by kind: the parse-side shapes (`planner::ForeignKeyDef`,
`planner::CheckConstraintDef`) carry `Option<String>` so declared-vs-auto is
representable, the catalog-side shapes (`ForeignKey`, `CheckConstraint`) carry a plain
`String` so nothing unnamed can reach the store, and the minted name is **persisted**.
A name is never reconstructed from the table: `ChooseConstraintName` searches the whole
schema across all four kinds, so `CREATE TABLE q (a INT, CONSTRAINT r_a_check CHECK (a >
0)); CREATE TABLE r (a INT, CHECK (a > 0));` names the second check `r_a_check1`, where a
search of the table's own preceding checks alone would answer `r_a_check` and dump two
`ADD CONSTRAINT`s under one name.

**The `CREATE TABLE` element order is part of the naming contract**, since a name is
minted from a position in a list: Postgres walks `stmt->tableElts` once
(`transformCreateStmt`), routing a column def and a table constraint onto the *same*
`cxt.ixconstraints`/`ckconstraints`/`fkconstraints` lists, so the constraints fold in
**written** order across the two spellings. Routing them to separate sinks and
concatenating would pair a persisted auto-name with the wrong specification — `CREATE
TABLE ro (FOREIGN KEY (a) REFERENCES p (id) ON DELETE CASCADE, a INT REFERENCES p (id) ON
DELETE SET NULL)` gives `ro_a_fkey` CASCADE on both engines. A
name-only query cannot see it; the fixture reads the `delete_rule` alongside.

**Which kind mints first differs by route**, and neither order follows from the other:
`CREATE TABLE` names its CHECKs, then its constraint indexes, then its FKs
(`DefineRelation` attaches the CHECKs as the heap is created), while `ALTER TABLE … ADD
COLUMN` names its **index first**, then its CHECKs, then its FK. The `ADD COLUMN` order
is a *reschedule*, not the pass numbering — read off the `AlterTablePass` enum it comes
out backwards, since the passes run ascending and `AT_PASS_ADD_CONSTR` (6) is **ahead**
of `AT_PASS_ADD_INDEX` (9). Nothing final happens in pass 6: `transformAlterTableStmt`
schedules the inline `UNIQUE` straight into pass 9 as an `AT_AddIndex`, while the CHECK
and the FK arrive in pass 6 as `AT_AddConstraint`s that `ATParseTransformCmd` only
re-files into `AT_PASS_ADD_OTHERCONSTR` (10) — CHECKs ahead of FKs because
`transformAlterTableStmt` appends `ckconstraints` before `fkconstraints`. The order is
observable only when the declared name is the one the other kind would have
auto-generated, and *that* is the case worth stating precisely: with a generic name the
two routes agree and both succeed (`ADD COLUMN c INT CONSTRAINT n UNIQUE CHECK (c > 0)`
yields `n` + `t_c_check` on both engines, and so does the `CREATE TABLE` spelling). Make
the declared name collide with the CHECK's auto base — `CONSTRAINT t_c_check` — and they
split: `ADD COLUMN` names the index `t_c_check` and the CHECK yields to it as
`t_c_check1`, while `CREATE TABLE t (c INT CONSTRAINT t_c_check UNIQUE CHECK (c > 0))`
names the CHECK `t_c_check` first and the declared UNIQUE then collides, `42710`.
Redundant `PRIMARY KEY`/`UNIQUE` specifications **within one statement** are merged
rather than rejected, reproducing `transformIndexConstraints`: `CREATE TABLE t (a INT
UNIQUE, UNIQUE (a))` and `a INT PRIMARY KEY UNIQUE` each build one index, the primary
key is kept in preference to a UNIQUE it absorbs, and a named duplicate transfers its
name onto an unnamed keeper (`a INT PRIMARY KEY, CONSTRAINT myu UNIQUE (a)` → one
PRIMARY KEY called `myu`). Key **order** is part of the specification, so `UNIQUE (a,b)`
and `UNIQUE (b,a)` stay two indexes; and the merge is scoped to one statement, so two
separate `ADD UNIQUE (a)` statements build `t_a_key` + `t_a_key1` — Postgres's scope as
well ("XXX in ALTER TABLE case, it'd be nice to look for duplicate pre-existing indexes,
too").

**Composite FK via `ADD CONSTRAINT` works** (multi-column `FOREIGN KEY (a,b)
REFERENCES parent (x,y)`). `ADD COLUMN [IF NOT EXISTS]` (+ inline FK) and `DROP COLUMN
[CASCADE|RESTRICT]` also supported.

**`DROP COLUMN` tracks dependent constraints** (one resolver in
`executor/dependencies.rs`, shared with `DROP TABLE`, reproducing Postgres's `pg_depend`
edge kinds): the table's own `CHECK`s naming the column and its own child-side `FOREIGN
KEY`s over it are **auto-dropped** (an AUTO edge — no `CASCADE`), while an FK that
*references* `table.column` — from another table **or from this one**, since the
referenced-column edge is NORMAL either way — raises `2BP01` unless `CASCADE`. Where one
constraint sits on both sides of the column (`FOREIGN KEY (id) REFERENCES t(id)`) the
AUTO edge wins and the drop proceeds, as in Postgres. The dependency check runs
**before** the index position-drift guard, so an FK-referenced column reports `2BP01`
(the real cause) rather than the guard's `0A000`; the guard itself stays (`0A000` if an
index keys on the dropped column or one after it — pglet keys indexes by row position
and doesn't rebuild them on drop).

**Divergences:** a *referenced* column always carries a covering unique index, so `DROP
COLUMN … CASCADE` on one still fails the drift guard `0A000` where PG succeeds (pglet
cannot rebuild the index the constraint's removal would orphan); `ALTER TABLE … DROP
CONSTRAINT` is unimplemented — parsed, and refused `0A000` over a table that exists
(`42P01` over a missing one, the skip under `IF EXISTS`) — which is also why a
constraint index's `2BP01` names dropping the table rather than PG's hint; a
column-level `CONSTRAINT <n> NOT NULL` drops its name, where PG 15+ materializes it
as a named CHECK; `ALTER TABLE IF EXISTS <t> …` performs the action over a relation that
exists and skips with 18.6's `NOTICE` over a name none holds (`EDGES.md` row 15 keeps its
residuals); `ADD COLUMN … NOT NULL DEFAULT NULL` is
`23502` like PG — the gate reads the resolved backfill values rather than whether a
DEFAULT is present, and `DEFAULT NULL` is normalized to *no* default (18.6 stores no
`pg_attrdef` row for a NULL constant) — as are `ADD COLUMN … NOT NULL` with no default,
`ALTER COLUMN … SET NOT NULL` over NULL rows, and `COPY` of a `\N`. `ALTER COLUMN … ADD
GENERATED … AS IDENTITY`, which reads no row, declares nothing: it is refused `55000`
over a column not already declared `NOT NULL`, whatever its rows hold, as on 18.6
(`ddl/add_identity_*`); `SET NOT NULL` first is the way in. A store in which an earlier
build accepted it over a NULL still holds that row, which the accelerated profile's
`ORDER BY` drops (`EDGES.md` row 38).

### `CREATE SEQUENCE` / `ALTER SEQUENCE` / `GRANT` / `REVOKE` / `COMMENT ON` / `ALTER TABLE … OWNER TO`

`pg_dump`-preamble tolerance: `CREATE SEQUENCE` registers a named sequence (honoring
`START WITH`, `RESTART` and `OWNED BY`; `INCREMENT BY`, `MINVALUE`/`MAXVALUE` and
`CYCLE` are parsed and ignored, every draw `+1` — `EDGES.md` row 215, silent) and `ALTER
SEQUENCE` honors `START WITH`/`RESTART`/`OWNED BY` while tolerating the rest of its tail
(see the Sequences row); `GRANT`/`REVOKE`/`COMMENT ON`/`OWNER TO` parse + succeed as
no-ops (pglet models no roles/comments). `SELECT pg_catalog.set_config(…)` parses
(schema-qualified call) and value-echoes; **`setval(…)` advances the sequence** (see the
Sequences row). `ALTER TABLE … ALTER COLUMN … SET/DROP DEFAULT` adjusts the catalog
default

### `SET` / `SHOW` / `DEALLOCATE` / `DISCARD` / `RESET`

`DEALLOCATE [PREPARE] {name|ALL}` drops the session's named prepared statements (a
`ColId` name; `26000` if absent; `ALL` spares the unnamed slot; non-transactional;
`25P02` in an aborted block). `DISCARD {ALL|PLANS|SEQUENCES|TEMP|TEMPORARY}` reports
18.6's tag for each; `ALL` is `DEALLOCATE ALL` + `RESET ALL` + the rest and is `25001`
in a transaction block; `SEQUENCES` clears the session `currval` cache; `PLANS`/`TEMP`
are no-ops. `RESET` takes the whole `reset_rest` (`ALL`, a dotted name, three fixed
spellings — one `0A000` on both) and reports tag `RESET`. Those three classify through
the lexer, so a comment-prefixed spelling works; `SET`/`SHOW` are `0A000` (`EDGES.md`
row 23). **`SET` arguments pglet decides are read on every route that writes them** —
the statement and `set_config`, one decider each: a `search_path` naming a schema pglet
lacks is `0A000` where 18.6 accepts it (one naming none, `pg_dump`'s `''` preamble, is
accepted — row 66), and a non-UTF-8 `client_encoding` likewise, `SET NAMES` included.
**A parameter whose value pglet holds fixed takes only that value**: `TimeZone` (`UTC`),
`DateStyle` (`ISO, MDY`), `IntervalStyle` (`postgres`) and `standard_conforming_strings`
(`on`) are `0A000` for any other, through `SET`, `SET TIME ZONE`, `set_config` and the
native server's startup packet, where 18.6 honors it; a reset is accepted, and
`set_config` of `application_name` or `lock_timeout`, which that function cannot move,
is `0A000` — row 69. A transaction-mode `SET` asking `READ ONLY` is `0A000`, matching
`BEGIN`. All five are intercepted **per statement** of a simple-query batch, on the
native server and in the embedded `Database::execute` — so `SET TIME ZONE 'UTC'; INSERT
INTO t VALUES (1)` runs the `INSERT` there and a mid-batch `SHOW` answers its row in
order — while the wasm bindings, and so the npm package, send every query string through
`Database::execute_bound`, which takes one statement and refuses that string `42601`
(`EDGES.md` row 270); an extended-protocol `Parse` of `SET x; INSERT …` is `42601` like
any other multi-statement Parse, since that route bypasses the parser's own
single-statement check. `SHOW server_version`/`server_version_num` answer
`18.6`/`180006` — **the release the differential corpus is asserted against: the dialect
target, not feature completeness** — from the one table the startup `ParameterStatus`
block renders and the server re-reports on change (`GUC_REPORT`). Incl. the isolation
level — `SET TRANSACTION ISOLATION LEVEL` (`25001` after the block's first statement
that takes a snapshot — not after a `SET`, `SHOW` or `SAVEPOINT` — or in a savepoint, as
on 18.6; `DEALLOCATE` and `DISCARD` take none here, row 237, nor does an
extended-protocol `Parse`, row 239), `SET SESSION CHARACTERISTICS`,
`default_transaction_isolation` and the startup parameter — honored and reported by
`SHOW transaction_isolation` / `default_transaction_isolation`; a `REPEATABLE READ` /
`SERIALIZABLE` block's first write is refused `40001` if the store moved past its
snapshot — coarser than 18.6 (`docs/storage.md` § Isolation Guarantees, `EDGES.md` row
232), and the `set_config` spelling naming either of those levels is `0A000`; `SHOW`
reports the modeled params and `SHOW ALL` lists them; an unmodeled parameter **fails
loud** (`42704`) rather than answering an empty result — Postgres's answer for a name it
does not know, and a divergence for every one it knows and pglet does not model (`SET
work_mem = '64MB'` answers `SET`, then `SHOW work_mem` `42704`, measured). Three of
those are timeouts whose `SET` pglet accepts and does not act on — `statement_timeout`,
`idle_in_transaction_session_timeout` and `transaction_timeout` — and that non-effect is
`EDGES.md` row 224. `lock_timeout` is modelled: `SET`, `SET LOCAL`, `RESET`, the startup
parameter, `SHOW` and `current_setting` read and write it (the `set_config` spelling
sets nothing, row 224) with 18.6's units, range and `22023`s (a list included),
transactionally — a `SET` in a block is taken back by its `ROLLBACK`, one after a
savepoint by a `ROLLBACK TO` it — and it bounds a statement's wait for the writer — its
default is pglet's `1s`, where 18.6's is `0`. **Divergences:** the `SHOW` parameter name
is read off the raw SQL text rather than through the lexer (`parse_show_name` splits on
whitespace and downcases), so a `"…"`-quoted spelling keeps its quote characters and
misses — `SHOW "search_path"` is `42704 unrecognized configuration parameter
""search_path""` here and reports the value on 18.6, where `VariableShowStmt` takes a
`ColId` and the scanner has already stripped them (`SHOW search_path` / `SHOW
SEARCH_PATH` agree on both). And `EDGES.md` rows 66, 67, 68, 69, 70, 224

### `COPY … FROM STDIN` / `TO STDOUT`

**text format** (the `pg_dump`/`psql` default) over the wire (the copy-in/copy-out
sub-protocol, simple **and** extended — Flush/Sync are ignored mid-copy-in, matching
Postgres) and via the embed API (`Database::copy_in`/`copy_out`). TAB-delimited, `\N`
null, full backslash-escape table (octal/hex on input); `COPY FROM` runs the **same**
validation/constraint/FK/index path as INSERT and ingests untrusted bulk input
**no-panic**, loud at what it checks (wrong field count `22P04`, a NUL or illegal UTF-8
in the input or a de-escaped field `22021`, bad value `22P02`, `\N` into NOT NULL
`23502`, unknown option `42601`). `FORMAT csv`/`binary` fail loud (`0A000`). The column
list is a **set** in both directions — `CopyGetAttnums` serves `TO` and `FROM` alike, so
a column named twice is `42701` on either, interleaved per item against the unknown
name's `42703` (`docs/names.md`). **Gaps:** `COPY (query) TO STDOUT` (copy a SELECT's
output) is absent (`42601`); a `\r`/`\n` `DELIMITER` → `0A000` where PG splits (CR/LF
`22023`, multi-byte `0A000`); the `HEADER` option takes a boolean only, so `HEADER
MATCH` (which checks the header line against the column list) is `42601 invalid value
for boolean option "header": MATCH` here where 18.6 accepts it and ingests the row —
measured in **both** the default text format and `FORMAT csv`, a loud over-rejection
either way. **Divergences in `COPY … FROM STDIN` input, some of them silent writes**
(`EDGES.md`): a `\.` after data on its line, which 18.6 refuses as a marker not alone on
its line and pglet reads with the rest of its line (row 132); line endings pglet never
compares with one another, splitting on LF alone (row 134); and a `CopyFail` message
carrying an illegal UTF-8 byte, answered `57014` with `U+FFFD` in the byte's place where
18.6 answers `22021` (row 130). `test_copy` compares its fixtures against real Postgres
— a `@copy-to`'s bytes byte-for-byte, a `@copy-from`'s `@verify` rows, and a refusal's
SQLSTATE — and holds none of those three rows, each row's bullet saying why

### `pg_dump` plain-text restore

a real `pg_dump … | psql -f` replays into pglet: the `SET`/`SELECT
pg_catalog.set_config(…)` preamble, `CREATE SEQUENCE`, `ALTER TABLE [ONLY] … OWNER TO`,
`ALTER SEQUENCE … OWNED BY`, `ALTER TABLE … ALTER COLUMN … SET DEFAULT nextval(…)`,
`ALTER TABLE … ALTER COLUMN … ADD GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY ( … )`
(the form pg_dump emits for every identity column — it creates the sequence under the
declared `SEQUENCE NAME` and records the ownership edge, over a column the dump's
`CREATE TABLE` declared `NOT NULL`, with `INCREMENT BY` / `NO MINVALUE` / `NO MAXVALUE` / `CACHE` / `AS <type>` parsed and
ignored), `GRANT`/`REVOKE`/`COMMENT ON` (parsed + no-op where semantically inert), the
inline `COPY … FROM stdin` data block, and `ALTER TABLE … ADD CONSTRAINT … PRIMARY KEY`
(enforcing). An **unsupported column type still fails loud** (`42704`) — tolerance never
swallows a type error. A **sub-second precision** arrives in pg_dump's own mid-name
spelling (`ts timestamp(3) without time zone`, `iv interval(3)`) and is read back at the
same width, which the round-trip asserts on the *catalog* as well as the rows: pg_dump's
`COPY` data is already rounded, so `datetime_precision` is the only observable a dropped
`(p)` moves. Round-trip differential vs live Postgres (`test_pg_dump`). **Divergence: a
dump of a database holding an *enum column* restores nothing but the type, over two
independent refusals.** The first is `ALTER TYPE public.mood OWNER TO postgres`, which
`pg_dump` emits for the user type and `parse_alter` refuses because its dispatch takes
`SEQUENCE` or `TABLE` only (`42601`; the `ALTER TABLE … OWNER TO` spelling beside it
succeeds). The second is the column type: `pg_dump` writes a user type qualified where
it writes a built-in bare (`m public.mood` beside `id integer`, measured —
`format_type.c` special-cases the built-in OIDs above the branch that qualifies, and the
`search_path = ''` preamble makes every user type non-visible) and the parser's type
position takes one bare identifier, so the column type is `42601` — a **supported** type
failing under the wrong code — and the `CREATE TABLE` takes its `COPY` block, its
`PRIMARY KEY` and everything after it down with it. Loud per statement and **silent per
process**: without `ON_ERROR_STOP` (psql's default) psql exits 0 over a database holding
the type and nothing else. Measured against the same table with a `TEXT` column, dumped
from its own `CREATE DATABASE`-fresh database: that control carries neither statement
and restores its rows, exit 0 either way. Closing it takes both members.
`test_pg_dump`'s schema builds no enum column;
`EDGES.md` row 113. **The other direction does not run: `pg_dump` cannot read a pglet
server** — its `SELECT pg_catalog.pg_is_in_recovery()` is `0A000` and it exits `1`
(`EDGES.md` row 44); `test_pg_dump` dumps *from* Postgres, so it does not reach this,
and a pglet store's export is `pglet_server dump`

### Native dump emitter

`Database::dump_sql()` (embed) + `pglet_server dump --data-dir <dir>` walk the catalog →
portable `CREATE TABLE …; COPY … FROM stdin; …; ALTER TABLE … ADD CONSTRAINT …` SQL. A
column's declared modifier rides out with it — a `numeric(p,s)`'s scale, a
`varchar(n)`'s length and a `timestamp(3)`/`interval(3)`'s sub-second precision — since
each is **re-applied by the restore**, so dumping any of them bare changes stored values
on reload without saying so; the round-trip's instrument for the last is a *write*
through the restored catalog, the rows alone being blind to it (they are already rounded
at the source). What works is the **emitter**. Whether that SQL then loads into **both**
stock Postgres and back into pglet is not closed: a database pglet's own accepted
statements build — a catalog state its DDL reaches, or a row value written into one of
its tables — can still fail to export at all, or dump to SQL one leg refuses, against
the dump invariant `docs/sequences.md` states over the catalog and the rows. Those are
`EDGES.md`'s rows, not this one's — a type name the emitter cannot spell, where `dump`
exits `1` having emitted nothing (row 32); a zero-column relation, refused on **both**
legs (row 64); a constant cast inside a stored expression (row 51); an invalid enum
label as a column `DEFAULT`, which the dump renders `DEFAULT 'zz'` and 18.6 refuses at
`CREATE TABLE` with `22P02` (row 47); a written enum cast at any persisted-expression
position (row 104); a `CHECK` or partial index over two different enum columns (row
115); a NUL in SQL text handed to an in-process door, the dump refusing some stored NUL
row values with `22021` and writing a NUL in catalog text raw, which a `psql -f` restore
into 18.6 exits `0` short of (row 133); the same NUL inside a **jsonb** string, which
the reader refuses wherever jsonb text is read but a jsonb-producing expression that
**wraps** a `Value::Text` rather than parsing it as jsonb text still reaches —
`jsonb_build_object`, over a value ordinary SQL through `Database::execute` stored as
readily as a bound one — and a bound `Value::Jsonb` beside it, a store the dump
**refuses to export** (`22021`, `refuse_unreadable_values`) rather than spell the
NUL jsonb's `\u0000` escape, which 18.6's `COPY` refuses `22P05`, restoring the table
with none of its rows at exit `0` (row 133); and, in a store written by an earlier
build, a NULL left in a column `ALTER COLUMN … ADD GENERATED … AS IDENTITY` was accepted
over — refused `55000` now — whose dump recreates the column `NOT NULL` and whose `COPY`
both legs refuse `23502`, the table restoring with none of its rows (row 38). The full
record: § Native dump emitter — details, below.

#### Native dump emitter — details

Cross-engine reloadability is only ever as good as the catalog states the engine refuses
to reach: two constraints sharing a name on one table, re-emitted faithfully, would load
back into pglet and be refused `42710` by stock Postgres, so a self-reload alone cannot
see such a state. The data section reuses the `COPY TO` text encoder; constraints and
indexes come after the data, pg_dump-style. A
**plain sequence emits three statements at three points**, because its start, its owner,
and its counter are three separate facts: `CREATE SEQUENCE … [START WITH <start>]` in
the schema block, `ALTER SEQUENCE … OWNED BY t.c` **after** the tables (it names one, so
it cannot live in the sequence block), and `SELECT setval('<n>', <last_value + 1>,
false)` after the data. An **identity** sequence emits **two**, and shares neither of
the first: one `ALTER TABLE … ALTER COLUMN … ADD GENERATED { ALWAYS | BY DEFAULT } AS
IDENTITY (SEQUENCE NAME <n> [MINVALUE <m>] [START WITH <s>])` after its table — which
creates the sequence and records the ownership edge in one statement, over the column
its `CREATE TABLE` declared `NOT NULL`, so a `CREATE SEQUENCE` beside it is `42P07` on the restore and an `ALTER
SEQUENCE … OWNED BY` beside it `0A000` — plus the same trailing `setval`. Rendering an
identity column as an ordinary `nextval`-defaulted one is the class of defect **rows
cannot witness**: the two hold identical values, so a round-trip comparing rows stays
green while `GENERATED ALWAYS` stops rejecting user-supplied ids (`428C9`) and all three
refusals tying the column to its sequence (`ALTER COLUMN … DROP DEFAULT` `42601`, `ALTER
SEQUENCE … OWNED BY` `0A000`, `DROP SEQUENCE` `2BP01`) vanish across pglet's *own*
round-trip, on both restored copies. The guard is therefore a refusal set asserted on
the source and on both legs, beside the rows. Only the *auto*-nameable options are
omitted (`INCREMENT BY` / `NO MAXVALUE` / `CACHE` / `AS <type>`, which pg_dump spells
and pglet models none of); `SEQUENCE NAME` is emitted **always**, including when it
equals the `<table>_<column>_seq` the restore would mint, because the two coincide in
the common case and a dump that leaves it out is right until the first database whose
identity sequence is called something else — at which point the trailing `setval` names
a sequence the restored catalog lacks and the counter silently restarts at 1.

**No `CREATE TABLE` names a sequence**: an identity sequence is created after its table,
so an inline `DEFAULT nextval(…)` cannot rely on every sequence existing before every
table, and *every* `nextval` default is hoisted into an
`ALTER TABLE … ALTER COLUMN … SET DEFAULT` after the whole schema block. That removes
the ordering requirement rather than patching the two ordinary catalogs that walk
through the hole — a sibling column of the same table drawing from its identity
sequence, and a column of a table earlier in name order drawing from a later table's,
both accepted by both engines. pg_dump reaches the same statement from the other side:
it prefers the inline form and falls back to this one when its dependency sort finds the
loop an owned sequence creates, which is why it hoists a `SERIAL`'s default too. Two
renderings are conditional, and both exist because a *catalog state pglet's own DDL could
reach* would otherwise emit SQL that does not load. An `OWNED BY` whose owner the
catalog cannot resolve is **skipped**: a sequence outliving its owner would otherwise
name a relation or column that does not exist — refused by stock Postgres and, for the
column routes, by pglet's own restore. Skipping rather than failing loud is deliberate:
such a sequence is semantically unowned, and an `Err` there would brick the export of
the whole database to preserve an edge pointing at nothing.

**The routes known to reach that state are shut** — a dropped table and a dropped SERIAL
column take their owned sequence with them (`executor::dependencies`' AUTO edge), and an
`ADD COLUMN … SERIAL` that fails after its sequence would have been registered registers
nothing, because the registration is staged behind the backfill validation. So the
branch has no live SQL seed, and both its guards (`pglet`'s
`a_dangling_sequence_owner_is_not_emitted` and `test_dump`'s `seed_sequence_edge_cases`)
build the state on the catalog by hand, asserting each of those routes from the other
side — the sequence is *gone* — since **a guard whose only seed is a defect stops testing
the emitter once the defect is fixed**.
And a start or resume point below `1` — reachable because pglet models no
`MINVALUE` — emits the explicit `MINVALUE` that floor needs plus an explicit `START
WITH` (given a minimum, `init_params` seeds an omitted `seqstart` from it). The floor is
a property of the *values*, not of the statement that declares them, so it travels into
the identity option list too (`GENERATED ALWAYS AS IDENTITY (START WITH 0)` and
`setval(<an identity sequence>, -5)` are both accepted by pglet, and 18.6 answers `START
value (0) cannot be less than MINVALUE (1)` without it); `MINVALUE` is
parsed-and-ignored there on pglet, so it is inert on the self-restore and load-bearing
on the Postgres one. A single folded `CREATE SEQUENCE … START WITH <counter + 1>` would
restore the resume point and silently lose the declared start, so a restored database
would answer a bare `RESTART` differently from the one it came from. The `is_called
= false` spelling is required rather than stylistic: the `true` form is `setval(n,
last_value, true)`, and for a never-advanced sequence `last_value` is `start - 1`, which
stock Postgres range-rejects (`22003`).

**A sequence name reaches the script in two grammars and both must name the same
relation**: three sites are identifier positions (`CREATE SEQUENCE`, `ALTER SEQUENCE …
OWNED BY`, `SEQUENCE NAME`) and two are string literals inside a `regclass` cast
(`nextval('<n>'::regclass)`, `setval('<n>', …)`). A literal is read back by
`SplitIdentifierString`, which **downcases** an unquoted part and rejects interior
whitespace, so a name that is not a safe bare identifier carries its *identifier*
quoting inside the string (`nextval('"Odd_a_seq"'::regclass)`) — pg_dump's own `fmtId`
posture of quoting only when needed. The *string* side is itself **two** grammars rather
than one: `quote_string_literal` emits the escape-string `E'…'` form for any name
containing a backslash (the one byte whose meaning depends on
`standard_conforming_strings`), so `CREATE SEQUENCE "a\b"` dumps as
`nextval(E'"a\\b"'::regclass)` and `setval(E'"a\\b"', …)` while the identifier positions
take that backslash raw. Any reader of the emitted script has to know all three — a check
scanning for `nextval('` alone passes **vacuously** over every `E'` default, on correct
dumps and orphaned ones alike. Emitting the raw name would be the **partial** failure
shape: an ordinary `CREATE TABLE "Odd" (a SERIAL, v TEXT)` would dump to a script where
stock Postgres takes the table and every row and drops only the column default and the
counter, so the identity silently renumbers from 1 into a collision, while a reader
taking the literal's bytes verbatim restores with zero errors (a DDL `DEFAULT
nextval('x')` auto-creates `x`, so the wrong name mints a second sequence). The reader
applies the same grammar, including its **downcase of an unquoted part** — the same fold
`Token::name` applies in an identifier position, so `CREATE SEQUENCE MySeq` and
`nextval('MYSEQ')` name one relation while `nextval('"MySeq"')` names another.

The deliberate divergences, which `reduce_name_literal` lists: there is no
`NAMEDATALEN` truncation, where `SplitIdentifierString` truncates every part, quoted
included (self-consistent here, since the lexer does not truncate either); `\v` is
missing from the scanner's space set, so `nextval(E'\x0Bs')` resolves `s` on 18.6 and is
`42P01` here while the neighbouring `\x0C` resolves on both; and an all-digit literal —
**or a lone `-`** — is a name here and an **OID** to `regclassin`, which runs
`parseDashOrOid` (named for both inputs) ahead of `stringToQualifiedNameList`
(`nextval('12345')` → `XX000 could not open relation with OID 12345` and `nextval('-')`
→ `XX000 could not open relation with OID 0` there, `42P01` here for each — unreachable
from a dump, since `quote_name_literal` quotes any name that is not a safe bare
identifier and neither a digit string nor `-` is one).

**A part's quoting decides whether it folds, in the reader as everywhere else**:
`nextval('"PUBLIC".s')`, `setval('"PUBLIC".sq', 7)` and `DEFAULT nextval('"PUBLIC".s2')`
are `42P01`, as on 18.6, where unquoting the parts while discarding that bit would advance
a sequence, move a counter and auto-create one. Whether an
index re-emits as `ADD CONSTRAINT … PRIMARY KEY|UNIQUE` or as `CREATE [UNIQUE] INDEX` is
read from the catalog's **constraint marker**, not re-derived from the index's name — a
deliberately standalone `CREATE UNIQUE INDEX t_pkey` stays an index instead of being
upgraded into a constraint on restore, and a declared `CONSTRAINT my_pk PRIMARY KEY`
re-emits under **that** name rather than the `<table>_pkey` convention. An unrenderable
type/default/expr **fails loud** rather than emitting wrong SQL, and the expression
renderer is **exhaustive by construction** — every unrenderable `Expr` variant declines
by name, so a new variant is a compile error there instead of silently inheriting a
catch-all (it reached the renderer by being *persisted*, and one unrenderable expression
aborts the dump of the **whole database**, not just its table). The scalar-function
families all render (`MathFn`, `StringFn`, `VariadicFn`, `Extract`, `Builtin`,
`jsonb_build_object`, `= ANY`/`ALL`), each emitting the spelling that **re-parses to the
identical variant** rather than a merely equivalent one — a stored expression is
compared structurally by the partial-index implication check, so rendering `POSITION(sub
IN str)` as `strpos(str, sub)` would restore as a different `Expr` and silently change
which queries the restored index serves. A corpus-driven test asserts that round-trip
property over every renderable variant and render arm.

**Divergence:** a stored expression Postgres would itself have rejected at DDL time does
not reload into stock Postgres — pglet does not enforce IMMUTABLE for an index
expression or partial predicate (Postgres: `42P17`), so a dumped `CREATE INDEX …
((concat(a, b)))` or `… WHERE ts < now()` is refused there; and an `extract()` whose
field is a non-literal expression (pglet accepts one, Postgres's `extract_arg` grammar
cannot spell one) renders back to pglet's comma form. Both reload into pglet.
Differential vs Postgres + self-reload (`test_dump`)

### LISTEN/NOTIFY

`42601` — a syntax error, not the `0A000` the `CREATE`/`DROP` dispatch gives, because
the keywords are unknown to the lexer (the fuz realtime layer uses app-level notify, not
LISTEN/NOTIFY)

### Object kinds the `CREATE` / `DROP` dispatch does not name

each dispatch (`parser/ddl.rs`) keys on the token after the keyword and refuses
everything else with `0A000 not implemented: only CREATE TABLE / INDEX / SEQUENCE / TYPE
/ SCHEMA / EXTENSION are supported`, or the `DROP` twin. Measured members: `CREATE
DOMAIN`; `DROP DOMAIN`, `IF EXISTS` included (the arm is chosen before the `IF EXISTS`
tail is read); `CREATE VIEW` plain, `OR REPLACE`, `MATERIALIZED` and `TEMP`; `DROP
VIEW`, `IF EXISTS` and `MATERIALIZED` included; `DROP TRIGGER`; `DROP FUNCTION`; and
`CREATE TEMP TABLE`, which shows the arm keys on the token rather than the object kind.
A column declared with a domain's name is a separate refusal from the type resolver,
`42704`. `PLAN.md` § Non-goals lists `domain` among the types absent by omission and
gives it `42704` — that is the *usage* code; the DDL statement's own code is the `0A000`
above. `EDGES.md` row 99

## Known divergences (`EDGES.md`)

`EDGES.md` is the ledger: one stably-numbered row per recorded divergence
the differential harness does not hold (struck when closed, grouped for
triage), each backed by a full bullet. A reference of the form “row 47”
indexes that file.
