# SQL compatibility

What pglet supports, feature by feature, checked against PostgreSQL 18.6 by the
differential fixture harness. The table answers "does pglet support X?" in one line. Each
feature's full record (what works, the divergences we measured, and the gaps) is under its
heading in [§ Details](#details), linked from the feature name. A one-line note summarizes
that record and doesn't replace it.

The roadmap is [`PLAN.md`](../PLAN.md). The npm package's README lists, in plain words,
the SQL a test suite commonly hits that pglet refuses, and the differences it gives
silently ([Known differences from PostgreSQL](../crates/pglet_wasm/npm/README.md#known-differences-from-postgresql)
and [Not supported yet](../crates/pglet_wasm/npm/README.md#not-supported-yet)).

**Status.** ✅ works, and its record names no open gap or divergence that ordinary SQL
(what an application, an ORM, a migration, a seed or a test suite writes) is likely to
hit. Any residual it does name is exotic. ⚠️ works, with at least one open gap or
divergence that ordinary SQL is likely to hit, each named in its record. 🔲 absent: refused
loudly wherever it's written. A status is a judgement made from the record, so if the two
seem to disagree, the record wins.

## Summary

| Feature | Status | Note |
|---|---|---|
| [SELECT/INSERT/UPDATE/DELETE, WHERE, ORDER/LIMIT/OFFSET](#selectinsertupdatedelete-where-orderlimitoffset) | ⚠️ | The core statements and clauses; `ORDER BY` may name a column the select list leaves out, and `OFFSET` works without `LIMIT`; `FETCH FIRST` is `42601`. |
| [`UPDATE … [alias] SET … FROM other [a] WHERE …`](#update--alias-set--from-other-a-where-) | ⚠️ | Joins a source into an `UPDATE`; `SET`, `WHERE` and `RETURNING` resolve over target and source together. `DELETE … USING` is `42601`, as are an alias on a `DELETE`'s target (`DELETE FROM t AS x`) and `DELETE FROM ONLY t`. |
| [TRUNCATE](#truncate) | ✅ | Several tables, `CASCADE`, `RESTART` / `CONTINUE IDENTITY`; the tail clauses are accepted in either order, where PostgreSQL requires one order. |
| [`DROP SCHEMA` / `CREATE SCHEMA`](#drop-schema--create-schema) | ⚠️ | Only `public` is modelled: `DROP SCHEMA public CASCADE; CREATE SCHEMA public` resets the catalog, and another schema name is refused. |
| [`DROP TABLE` / `DROP INDEX` dependencies](#drop-table--drop-index-dependencies) | ⚠️ | Foreign-key and owned-sequence dependents block (`2BP01`) or cascade as in PostgreSQL; `ALTER TABLE … DROP CONSTRAINT` is `0A000`. |
| [Arithmetic (`+` `-` `*` `/` `%` `^`)](#arithmetic-------) | ⚠️ | Integer, float and `numeric` arithmetic at PostgreSQL's result types, `22003` on overflow, `22012` on division by zero; a `text` operand and unary `+` are under-checked. |
| [Numeric narrowing casts (`int8::int4`, `float8::int4`, …)](#numeric-narrowing-casts-int8int4-float8int4-) | ✅ | Out of range is `22003`, never a wrap; float to integer rounds half to even. |
| [INNER/LEFT/RIGHT/FULL / CROSS JOIN, comma-join, `JOIN … USING`](#innerleftrightfull--cross-join-comma-join-join--using) | ⚠️ | Every join kind, run as a nested loop, with `USING` merging its columns; `NATURAL JOIN` is `0A000`, an `ON` naming a table out of its scope errors only once a row is evaluated, and a `USING` merge of differently-typed columns named bare is declared at the left side's type (`EDGES.md` row 309). |
| [Aggregates, GROUP BY, HAVING](#aggregates-group-by-having) | ⚠️ | `count`, `sum`, `avg`, `min`, `max` with `GROUP BY` and `HAVING`, under an expression (`round(avg(x), 2)`), at 18.6's result types — `avg` of an integer and `sum` of a `bigint` are exact `numeric` where the argument's integer type is certain (`EDGES.md` row 2). `array_agg`, `string_agg`, `bool_and` are `0A000`, `FILTER` `42601` (`EDGES.md` row 44). |
| [`COALESCE` / `CASE` common types](#coalesce--case-common-types) | ✅ | Branches resolve to PostgreSQL's common type and are cast to it, `42804` / `42846` where there is none; a branch the check can't type can succeed where PostgreSQL refuses (`EDGES.md` row 65). |
| [`IN` / `NOT IN` list common type](#in--not-in-list-common-type) | ⚠️ | A table's own column against written constants compares at PostgreSQL's common type; any other list compares item by item and can miss a `real` row (`EDGES.md` row 280). |
| [Subqueries (scalar/IN/derived)](#subqueries-scalarinderived) | ⚠️ | Scalar, `IN`, `EXISTS`, derived and correlated subqueries; `= ANY (SELECT …)` and `ALL (SELECT …)` are `42601`, a `WITH` or `VALUES` subquery `0A000`. |
| [CTEs (WITH)](#ctes-with) | ⚠️ | Plain, column-listed, data-modifying and `WITH RECURSIVE`; mutual recursion is `0A000`, and several data-modifying CTEs run in declaration order (`EDGES.md` rows 160, 162). |
| [Window functions](#window-functions) | ⚠️ | `row_number`, `rank`, `dense_rank` only; frames, `lag` / `lead` / `ntile` / `first_value` and aggregates over a window are refused. |
| [Transactions (BEGIN/COMMIT/ROLLBACK/SAVEPOINT)](#transactions-begincommitrollbacksavepoint) | ⚠️ | `BEGIN` / `COMMIT` / `ROLLBACK`, savepoints, isolation levels and PostgreSQL's aborted block, one writing block at a time; a multi-statement message is not one transaction; `READ ONLY` is `0A000`. |
| [JSONB `->`/`->>`, `@>`, key/path ops, `jsonb_build_object`](#jsonb-----keypath-ops-jsonb_build_object) | ⚠️ | `->`, `->>`, `#>`, `#>>`, `?`, `?\|`, `?&`, `@>`, `<@` and `jsonb_build_object`, containment GIN-accelerated; a `text` operand is accepted where PostgreSQL refuses, and jsonpath (`@?`, `@@`) is refused. |
| [`json` (as distinct from `jsonb`)](#json-as-distinct-from-jsonb) | 🔲 | `42704` wherever the type is named; a text-preserving `json` is unbuilt. |
| [ON CONFLICT](#on-conflict) | ⚠️ | `DO NOTHING` and `DO UPDATE … [WHERE]` with `EXCLUDED`, over column, expression and partial-index targets; no `ON CONSTRAINT`, and a target-less `DO UPDATE` silently runs as a plain `INSERT`. |
| [`CREATE`/`DROP INDEX CONCURRENTLY`](#createdrop-index-concurrently) | ✅ | `25001` inside a transaction block; otherwise built like the plain form, and it blocks what that blocks. |
| [`CREATE`/`DROP EXTENSION`](#createdrop-extension) | ⚠️ | `pgcrypto`, `uuid-ossp` and `plpgsql` are accepted by name, since their functions are builtins here; any other name is `0A000`. No extension catalog. |
| [CREATE INDEX](#create-index) | ⚠️ | B-tree and GIN, unique, partial and expression indexes, used for equality, ranges, single-key `ORDER BY` and `@>`; expression and enum keys never speed a query; `INCLUDE` is `42601`, `USING hash` `0A000`. |
| [Index entry size limits](#index-entry-size-limits) | ⚠️ | Each entry is sized the way PostgreSQL sizes it (`54000`); the over-refusals and silent residuals are `EDGES.md` rows. |
| [NOT NULL / CHECK constraints](#not-null--check-constraints) | ⚠️ | Enforced at write time (`23502` / `23514`); a `CHECK` is type-checked when it's created, and a quoted literal beside a non-text column in one is `42883` (`EDGES.md` row 196). |
| [PK / UNIQUE / FK constraints](#pk--unique--fk-constraints) | ⚠️ | Enforced, single and composite, with every `ON DELETE` / `ON UPDATE` action; no `DEFERRABLE`, and `MATCH FULL` / `PARTIAL` is `0A000`. |
| [INSERT column-list binding](#insert-column-list-binding) | ✅ | Binds by name, omitted columns take their `DEFAULT`, `DEFAULT VALUES` works, and a column named twice is `42701`. |
| [Arrays (`text[]`/`uuid[]`/…), `@>`/`<@`, `ANY`/`ALL`, `unnest`](#arrays-textuuid--anyall-unnest) | ⚠️ | One-dimensional arrays of every column type, `@>` / `<@`, `= ANY` / `ALL` and `unnest`; a nested array is `0A000`, a `timestamptz` or `varchar` constructor compares at the wrong type (`EDGES.md` row 281), and a mixed-type `ARRAY[…]` under a FROM `unnest` declares its first element's type (row 308). |
| [GIN indexes for arrays / JSONB `@>`](#gin-indexes-for-arrays--jsonb-) | ✅ | `USING gin` over arrays and `jsonb`, used for a top-level `@>` and for an `OR` of them over one column. |
| [ENUM types](#enum-types) | ⚠️ | `CREATE TYPE … AS ENUM`, ordered by member position, checked on write; sent as `text` on the wire, no enum array column, and `LIKE` and string functions read an enum as `text` (`EDGES.md` row 190). |
| [Sequences / SERIAL auto-increment](#sequences--serial-auto-increment) | ⚠️ | `SERIAL`, identity columns and named sequences; every draw is `+1`, silently, whatever `INCREMENT BY` / `MINVALUE` / `MAXVALUE` / `CYCLE` say (`EDGES.md` row 215). |
| [`DEFAULT` functions (`now()`, `gen_random_uuid()`)](#default-functions-now-gen_random_uuid) | ⚠️ | Literal, `now()`-family and `gen_random_uuid()` defaults, the `now`-family spellings kept distinct, each reading the instant its PostgreSQL namesake reads; outside a block a bound statement's `now()` is its `statement_timestamp()` (`EDGES.md` row 292). |
| [`starts_with(a, b)`](#starts_witha-b) | ✅ | A case-sensitive literal prefix test over `text`; a non-text argument is `42883`, an enum excepted (`EDGES.md` row 190). |
| [`\|\|` string concat](#-string-concat) | ⚠️ | The text forms only; `bytea \|\| bytea`, `jsonb \|\| jsonb` and the array forms are `0A000`, and an uncast string or `$N` beside a `jsonb` or `bytea` side is joined as text, silently, and beside an array is refused (`42883`, or `0A000` over a `text[]`) (`EDGES.md` row 293). |
| [`LIKE … ESCAPE` + default `\` escape](#like--escape--default--escape) | ⚠️ | `\` is the default and the only accepted escape; a non-text operand is `42883`, a `bytea` beside an uncast pattern included, which PostgreSQL accepts (`EDGES.md` row 262), `bytea LIKE bytea` is `0A000`, and `LIKE ANY (…)` is `42601`. |
| [`ILIKE`](#ilike) | ⚠️ | `LIKE` over Rust's full Unicode lowercasing, so a few characters fold differently from PostgreSQL (`EDGES.md` row 264), and `ILIKE ANY (…)` is `42601`. |
| [Scalar string + misc functions](#scalar-string--misc-functions) | ⚠️ | The common string functions (`upper`, `lower`, `length`, `substring` and `substr` over `text` and `bytea`, `left`, `right`, `octet_length`, `md5`, `trim`, `position`, `replace`, `concat`, `lpad`, `split_part`, …), and `random()`; an unbuilt one is `0A000` (`format`, `repeat`). |
| [`date_trunc` / `extract` / `date_part`](#date_trunc--extract--date_part) | ✅ | `date_trunc`, `EXTRACT` and `date_part` over the temporal types at PostgreSQL's result types; an untyped `date_trunc` source succeeds where PostgreSQL refuses. |
| [`SELECT DISTINCT` / `DISTINCT ON`](#select-distinct--distinct-on) | ✅ | `DISTINCT` and `DISTINCT ON`, with PostgreSQL's `ORDER BY` match rule (`42P10`) and its `NULL` and `NaN` equality. |
| [`UNION` / `INTERSECT` / `EXCEPT` (+ `ALL`)](#union--intersect--except--all) | ✅ | `UNION`, `INTERSECT`, `EXCEPT`, with and without `ALL`, at PostgreSQL's precedence and common column types; a recursive term is typed by a separate check. |
| [`CASE WHEN … END`](#case-when--end) | ✅ | Searched and simple forms; a non-boolean `WHEN` condition is `42804`. |
| [Boolean contexts (`WHERE`, `HAVING`, `JOIN … ON`, searched `CASE WHEN`, `AND`/`OR`/`NOT`, `CHECK`, a partial index's `WHERE`, `IS [NOT] TRUE\|FALSE\|UNKNOWN`)](#boolean-contexts-where-having-join--on-searched-case-when-andornot-check-a-partial-indexs-where-is-not-truefalseunknown) | ✅ | Each context resolves its operand to `boolean` as PostgreSQL does: any other type is `42804`, an unresolved literal is read as a boolean. |
| [`FOR UPDATE` / `FOR SHARE`](#for-update--for-share) | ⚠️ | Every lock strength is parsed; inside a block a locking read takes the single writer, which is coarser than a row lock. `SKIP LOCKED` and `NOWAIT` are `0A000`. |
| [`alias.*` qualified wildcard](#alias-qualified-wildcard) | ⚠️ | Expanded to that relation's columns, beside other items; over a `FROM`-clause `unnest(…)` item it is `0A000` or `42702`, or expands a same-named base table's column names (`42703` unless they match) (`EDGES.md` row 8). |
| [Non-sole `*` + `RETURNING` projection](#non-sole---returning-projection) | ✅ | `SELECT *, …` and `RETURNING *, …`, a correlated subquery in `RETURNING` included; PostgreSQL 18's `OLD` / `NEW` in `RETURNING` is `42P01`. |
| [`DO $$ … $$` anonymous block](#do----anonymous-block) | ⚠️ | `BEGIN … [EXCEPTION WHEN …] END` over plain SQL statements, an `EXCEPTION` block a real subtransaction; `DECLARE`, variables and control flow are refused. |
| [Keyword as identifier](#keyword-as-identifier) | ⚠️ | Each keyword is reserved according to PostgreSQL's class for it, so `update`, `set`, `values` and the like are usable as names; the SQL value-function words (`user`, `current_date`, …) are accepted as a column name where PostgreSQL refuses them (`EDGES.md` row 75). |
| [Operator lexing + resolution](#operator-lexing--resolution) | ⚠️ | Operators are lexed the way PostgreSQL lexes them; an operator pglet lacks (`&`, `#-`, `@?`, `@@`, …) is `42883`, and `OPERATOR(…)` is not parsed. |
| [Identifier case-folding](#identifier-case-folding) | ✅ | Unquoted names fold to ASCII lower case where they're read, and quoted names are exact, as on PostgreSQL over a multibyte (UTF-8) database encoding. |
| [INTERVAL](#interval) | ✅ | A full column type with PostgreSQL's three input styles, arithmetic and ordering; no `infinity`, and a field spec (`interval year`) is refused or ignored. |
| [DATE](#date) | ✅ | A full column type over `0001-01-01` to `9999-12-31` plus `infinity`; no BC dates. |
| [NUMERIC / DECIMAL](#numeric--decimal) | ✅ | Exact decimal with `NUMERIC(p,s)`, PostgreSQL's scale rules and exact `SUM` / `AVG`; a fractional exponent and numeric `Infinity` are refused. |
| [Session information functions](#session-information-functions) | ⚠️ | `version()`, `current_user` and the other identity spellings, `current_schema`, `current_setting`; `SET search_path` doesn't change what they return, and there are no roles. |
| [SQL typed constants (`DATE '2024-06-15'`, `NUMERIC(5,2) '1.5'`, `DOUBLE PRECISION '…'`)](#sql-typed-constants-date-2024-06-15-numeric52-15-double-precision-) | ✅ | `DATE '…'`, `NUMERIC(5,2) '…'` and the other typed-constant spellings, read as the matching cast. |
| [`x IS [NOT] TRUE / FALSE / UNKNOWN`](#x-is-not-true--false--unknown) | ✅ | PostgreSQL's boolean test, never `NULL`; a non-boolean operand is `42804`. |
| [TIME](#time) | ✅ | A full column type with `time(p)`; `timetz` is `42704`, a zone suffix on a time literal is `22007`, and `CURRENT_TIME` is `time`, not `timetz`. |
| [bytea](#bytea) | ⚠️ | A full column type, hex and escape input read as PostgreSQL reads them; `bytea \|\| bytea` and `bytea LIKE bytea` are `0A000`, and an uncast string beside a `bytea` under `\|\|` is joined as text (`EDGES.md` row 293). |
| [Real prepared statements / param type inference](#real-prepared-statements--param-type-inference) | ⚠️ | `$N` parameters typed from their context or from the client's declared types, reported in `ParameterDescription`; a `$N` used twice is not unified, and one beside arithmetic (`SET n = n + $1`) is left untyped, so a text-format bind of it is `42804`. |
| [`to_char` / `AT TIME ZONE`](#to_char--at-time-zone) | ⚠️ | The ISO-8601 `to_char` tokens over a timestamp, and `AT TIME ZONE 'UTC'`; another zone or an unknown token is refused. |
| [`timestamptz` text wire output](#timestamptz-text-wire-output) | ✅ | Rendered with the `+00` offset, so a text-protocol driver reads the right instant. |
| [`ErrorResponse` structured fields](#errorresponse-structured-fields) | ⚠️ | SQLSTATE, message, detail, hint, position and the table / column / constraint fields where PostgreSQL sends them; a field pglet can't fill truthfully is omitted, and some are not sent yet. |
| [`jsonb_build_object`, `IS [NOT] DISTINCT FROM`](#jsonb_build_object-is-not-distinct-from) | ⚠️ | `jsonb_build_object` and the null-safe `IS [NOT] DISTINCT FROM`. A timestamp or array value in `jsonb_build_object` is rendered as its text output, not as 18.6 renders it (`EDGES.md` row 306). |
| [Scalar `EXISTS` / `NOT EXISTS`](#scalar-exists--not-exists) | ⚠️ | Plain and correlated `EXISTS` / `NOT EXISTS`; the sub-plan runs to the end, so a row past the first match that raises an error fails the query where PostgreSQL succeeds (`EDGES.md` row 320). |
| [`INSERT … SELECT` + `unnest($N)` in a SELECT projection](#insert--select--unnestn-in-a-select-projection) | ⚠️ | `INSERT … SELECT`, and `unnest($1)` in a `SELECT` with no `FROM`; `unnest(…)` in the select list of a query with a `FROM` is `0A000`. |
| [`generate_series` in `FROM`](#generate_series-in-from) | ⚠️ | The `integer` and `bigint` series over constant and parameter arguments, up to 1 000 000 rows (`54000` past it); the `numeric` and timestamp series, an argument naming a column (the lateral form) or holding a subquery, and the select-list form are refused (`EDGES.md` row 321). |
| [pg_catalog introspection (drizzle-kit push etc.)](#pg_catalog-introspection-drizzle-kit-push-etc) | ⚠️ | The catalogs and functions ORM introspection reads (`pg_class`, `pg_attribute`, `pg_type`, `pg_constraint`, `pg_index`, `pg_enum`, `pg_tables`, `pg_indexes`, …), several with a subset of their columns. |
| [`information_schema`](#information_schema) | ⚠️ | `tables`, `columns`, `table_constraints`, `key_column_usage`, `referential_constraints`, `constraint_column_usage`; a `char(n)` column reads as `character varying`. |
| [`ALTER TABLE … ADD CONSTRAINT`](#alter-table--add-constraint) | ⚠️ | `CHECK`, `PRIMARY KEY`, `UNIQUE` and `FOREIGN KEY`, each validated against the existing rows, then enforced; `DROP CONSTRAINT` is `0A000`. |
| [`CREATE SEQUENCE` / `ALTER SEQUENCE` / `GRANT` / `REVOKE` / `COMMENT ON` / `ALTER TABLE … OWNER TO`](#create-sequence--alter-sequence--grant--revoke--comment-on--alter-table--owner-to) | ⚠️ | Enough to get through a `pg_dump` preamble: a named sequence honors `START WITH`, `RESTART` and `OWNED BY`; `GRANT`, `REVOKE`, `COMMENT ON` and `OWNER TO` succeed as no-ops. |
| [`SET` / `SHOW` / `DEALLOCATE` / `DISCARD` / `RESET`](#set--show--deallocate--discard--reset) | ⚠️ | `SET`, `SHOW`, `RESET`, `DEALLOCATE`, `DISCARD`; a setting pglet holds fixed (`TimeZone` `UTC`, `DateStyle`, …) takes only that value, and a `search_path` naming another schema is `0A000`. |
| [`COPY … FROM STDIN` / `TO STDOUT`](#copy--from-stdin--to-stdout) | ⚠️ | Text format over the wire and the embed API, through the same checks as `INSERT`; `csv` and `binary` are `0A000`, `COPY (query) TO` is `42601`. |
| [`pg_dump` plain-text restore](#pg_dump-plain-text-restore) | ⚠️ | A plain-text `pg_dump \| psql` replays, preamble and identity columns included; a database with an enum column doesn't restore. |
| [Native dump emitter](#native-dump-emitter) | ⚠️ | `dump_sql()` and `pglet_server dump` emit portable SQL; some states pglet's own statements build still fail to export or to reload (`EDGES.md`). |
| [LISTEN/NOTIFY](#listennotify) | 🔲 | `42601`: the keywords are unknown to the lexer. |
| [Object kinds the `CREATE` / `DROP` dispatch does not name](#object-kinds-the-create--drop-dispatch-does-not-name) | 🔲 | `CREATE` / `DROP` of a domain, a view, a materialized view, a trigger, a function or a temporary table is `0A000`. |

## Details

### SELECT/INSERT/UPDATE/DELETE, WHERE, ORDER/LIMIT/OFFSET

`ORDER BY` a base column that isn't projected works. It sorts the rows before projection,
and an output-only alias or position still sorts above the projection. `OFFSET` works
without `LIMIT`. A `LIMIT` cuts the rows after every input row is evaluated, so an error in
a row 18.6 never reads is still raised. The same happens in a derived table under an outer
`WHERE false` or joined with an empty table (`EDGES.md` row 320). The full record is in
§ SELECT / DML family — details, below.

#### SELECT / DML family — details

A **no-FROM** `SELECT` runs the same tail. Its trailing `ORDER BY`/`LIMIT`/`OFFSET` goes
through `plan_no_from_order_limit`, which also *binds* its sort keys (a `Plan::Sort` over
a `Plan::Values` only checks them when a row reaches the eval-time resolver). Its row-level
locking clause goes through the same `parse_for_locking` its sibling parse arms call.

One divergence is still open there, and it comes from the plan's **shape**, not from
leniency. A top-level sequence projection is dispatched on `Plan::is_sequence_projection`,
which matches a bare `Plan::Values`. So the tail's checks run over it, but the
`Sort`/`Limit` isn't built. `SELECT nextval('s') LIMIT 0` returns a row here, where 18.6
returns `(0 rows)`. 18.6 also leaves the counter where it was, while every pglet spelling
we tested advances it. Meanwhile `SELECT nextval('s') ORDER BY zzz` is `42703` and
`ORDER BY 5` is `42P10` on both. This affects a **referenced CTE body** as well as the
outermost select, and a projection that just *carries* a sequence function next to other
expressions. Each **write** spelling we tested errors loudly here, so the wrong row count
stays inside a read. The `DISTINCT`/`DISTINCT ON` quantifiers have the same exclusion, and
it changes no result there, since deduplicating one row returns that row. See `EDGES.md`.

A positional `ORDER BY n` binds the Nth **output** column (out of range is `42P10`). When
it's mixed with a sort key on a dropped base column, it's rewritten to that projection
expression so it still sorts correctly below the projection. The rewritten expression is
**shielded when it's a bare integer literal itself**, because that's also how the plan
spells a positional reference. Without the shield, the Sort below read that number as an
ordinal against the *base* row. `SELECT 't', 5 FROM s ORDER BY 2, a` would be
`42P10 position 5 is not in select list` over a one-column source, and
`SELECT 2 AS z, b FROM s ORDER BY 1, a` would return the rows in `b` order instead of `a`
order. Both run without error on 18.6
(`select/order_by_below_projection_positional_key_is_a_value.sql`).

**Divergence:** two identical unqualified output names referenced by `ORDER
BY`/`DISTINCT ON` (`SELECT a, a … ORDER BY a`) fail loudly with `42702` where Postgres
returns rows. Postgres's `findTargetlistEntrySQL92` compares the two matching target-list
entries with `equal()` and refuses only when they differ (`a AS x, b AS x … ORDER BY x`).
The **plan-time** check compares the two matching entries' **expressions** instead of
counting names. The comparison is structural: it compares the postcard bytes of the
`Expr`, which is what `equal()` → `datumIsEqual` does. It deliberately doesn't use pglet's
own `PartialEq`, because under that `PgNumeric`'s `1.5 == 1.50` and `PgInterval`'s
`INTERVAL '30 days' == INTERVAL '1 mon'` would merge two entries that 18.6 calls ambiguous
into one and lose rows. This only covers the cases its fixtures hold, not `equal()` in
general. On the accept side: a wildcard's qualified reference next to a written bare one,
an `AS` label, a qualified/bare pair, a `GROUP BY` target list and a `DISTINCT ON` key
(`select/nested_sort_key_duplicate_output_entries_resolve.sql`). On the refuse side: two
different columns, plus those two value folds
(`select/nested_sort_key_duplicate_output_entries_loud.sql`,
`select/nested_sort_key_numeric_display_scale_is_not_equal.sql`,
`select/nested_sort_key_interval_estimate_is_not_equal.sql`).

**Four causes make it refuse what 18.6 resolves:** a no-op cast on an expression that's
already typed, an unknown-typed literal's cast that isn't folded to a constant, an `IN`
list that isn't normalized to `= ANY (ARRAY[…])`, and `sole_relation_carrying`'s three
declines. Each one errors loudly (`42702`), and the cases we measured are listed in
`EDGES.md`. These are the four causes we found, which doesn't rule out a fifth. A unary
`-` over a numeric constant isn't one, since the parser folds it into the constant (see
below). Each one also errors regardless of row count at a level's own lowering, because
the sort keys sit below the projection. Over an empty relation, a duplicated no-op-cast
pair, a duplicated unknown-literal-cast pair, an `IN` list next to its `= ANY` form, and
`sole_relation_carrying`'s declines over a `USING` join and a catalog view are each
`42702`, just as they are with rows (a sweep, not a proof).

An `unnest` FROM item isn't one of those declines. A FROM-clause `unnest(…)` registers
its full column list, so its level is certain and `sole_relation_carrying` resolves it.
`SELECT u.x AS q, x AS q FROM unnest(ARRAY[]::int[]) AS u(x) ORDER BY q` is `(0 rows)` on
both engines (`select/unnest_duplicate_output_entries_bind_to_the_unnest.sql`). Its
non-empty spelling and its `GROUP BY q` twin still give `42702` here, from resolvers this
binding doesn't reach.

A failing bare key also takes precedence over an unparsed trailing set-op token, the third
member of a loud precedence class. `SELECT a FROM ne ORDER BY bogus UNION SELECT a FROM nf`
is `42703` here, where 18.6 gives `42601` at the `UNION`. The positional (`42P10`) and
qualified (`42P01`) spellings diverge the same way. As controls, `ORDER BY a` /
`ORDER BY 1` / `LIMIT 1` in that position are `42601` on both.

Next to those is the **eval-time** resolver, which flags the name collision on the row
it's given, so this divergence depends on the row count. Over `ja2(c, x)` holding two
rows, `SELECT c, c FROM ja2 ORDER BY c` and `SELECT DISTINCT ON (c) c, c FROM ja2` are
`42702` here and return rows on 18.6. The same statements over an empty relation return
`(0 rows)` on both. `GROUP BY` doesn't take that shortcut. It compares expressions, as the
Aggregates row below describes.

`SET col = DEFAULT` resolves the column's catalog default (including SERIAL/nextval) on
UPDATE and `ON CONFLICT DO UPDATE`, and an eval error on a SET right-hand side propagates
instead of being swallowed. Every right-hand side in one `SET` list reads the target row
that the scan or the conflict probe found, never an earlier assignment's result, and they
run in column order (`EDGES.md` rows 243 and 244). A `DO UPDATE` that hits a row its own
statement wrote is refused with `21000` (row 247). A negative `LIMIT`/`OFFSET` fails loudly
(`2201W`/`2201X`) and an overflowing one gives `22003`. A fractional count rounds
**half away from zero**, matching Postgres's `numeric→int8` coercion (`LIMIT 2.5` gives 3
rows), and never silently becomes unbounded.

**Divergence: `LIMIT`/`OFFSET` doesn't share the parser's constant fold.**
`parse_limit_count` reads the sign off a raw `-`-then-`Number` token pair before its
general dispatch. So it never sees the folded constant that `parser::expr`'s `negate`
produces, and it rejects a fold that 18.6 *does* perform. `LIMIT -0` and `LIMIT -0.4` are
`2201W` here against `(0 rows)` there, and `OFFSET -0` is `2201X` here against every row
there. (`doNegate` folds `-0` to `0`, and the `numeric→int8` round takes `-0.4` there
too.) `LIMIT - -1`, which is two folds, is `42601` here against one row there.

The **parenthesized** spellings come from this clause's own grammar, not the fold.
`LIMIT -(1)` / `OFFSET -(1)` are `42601` here against `2201W`/`2201X` there, but so is the
unsigned `LIMIT (1)` against one row there, and likewise the quoted `LIMIT '1'` /
`OFFSET '0'` (`42601` here, read as the count there). That's because the clause takes a
bare number, `$N`, `ALL` or `NULL`, and no expression at all. Every one of those errors
loudly here, so none gives a wrong result.

Because it's read in the **parser**, this clause is also checked before every plan-time
check. You can see that wherever both are wrong at once: `… UNION … ORDER BY bogus+1
LIMIT -1` is `2201W` here, where 18.6 gives `42703` for the key, since 18.6 resolves the
sort keys during parse analysis and checks the bound after. Both error, with different
codes. The spelling without `LIMIT` is `42703` on both.

Unrelated to the sign, but in the same clause: `FETCH FIRST <n> ROWS ONLY` isn't parsed
here (`42601`) and works on 18.6.

`ORDER BY` runs the plan-time coercion pass against the same scope as the projection, so
`ORDER BY (id = '<uuid>')` against a `uuid` column resolves instead of raising `42883`. A
bare string constant still follows Postgres's SQL92 ordinal rule (`ORDER BY 'lit'` isn't a
sort key). `ORDER BY +1` is **not** an ordinal. Unary `+` stays an operator node
(`Expr::UnaryPlus`), as in Postgres's grammar, which folds only unary `-` into a constant.
So it sorts by a constant and keeps the scan order instead of sorting by output column 1.

The flip side is that `ORDER BY -1` **is** an ordinal, a negative one. `parser::expr`'s
`negate` folds `-` over a `Literal::Int`/`Literal::Numeric` into the constant (`doNegate`'s
two `A_Const` kinds). So `-1` reaches the bounds check as `42P10 position -1`, `-0` as
`42P10 position 0`, `- -99` as `42P10 position 99`, and `-1.5` as
`42601 non-integer constant`. `-99::int` / `-abs(99)` / `-id` stay ordinary keys, because
none of the three is a constant node. `GROUP BY` and `DISTINCT ON` fold the same way, and a
no-FROM `SELECT` reaches the same bounds check through its sort/bound tail (the paragraph
at the top of this row). `SELECT 1 AS q ORDER BY -1` and its positive control
`ORDER BY 5` are `42P10` on both (`select/no_from_order_by_negative_position_loud.sql`,
`select/no_from_order_by_bad_position_loud.sql`). Without the fold, every one of those
spellings would sort by a constant and **run**, including inside the filtering subquery of
a `DELETE`/`UPDATE`, which would be a silent write. See `EDGES.md`.

**Divergence:** a set-op's trailing `ORDER BY` is outside the coercion pass. Its keys bind
at their own lowering against only the set-op's output names. That applies to **every
`Expr::Column` leaf of a key, not just a key that is a column**, and it's the namespace
18.6 uses for them too (`transformSetOperationStmt` makes the output columns visible as a
join RTE). The *expression* rule itself still accepts too much, where 18.6 refuses with
`0A000 invalid UNION/INTERSECT/EXCEPT ORDER BY clause`. Over two one-column tables,
`… UNION … ORDER BY -a` / `a+0` / `abs(a)` each sort here and are `0A000` there, while
`… ORDER BY bogus+1` is `42703` on both, since 18.6 resolves the names before raising that
error. It isn't *every* expression, and it isn't `42601`. A parenthesized column
`ORDER BY (a)` and a cast `ORDER BY a::int` give the same result on both engines. The
`42601` nearby is `findTargetlistEntrySQL92`'s separate non-integer-constant rule
(`ORDER BY true` / `NULL` / `'x'`, `42601` on both engines, with `ORDER BY -1` `42P10` on
both).

Where the sort goes depends on qualifiers, following Postgres's `findTargetlistEntrySQL92`
rule that the output-name match only applies to a single-field reference. A **qualified**
`ORDER BY` key always resolves against the relation it names, so
`SELECT pa.x … JOIN pb … ORDER BY pb.x` sorts by `pb.x`, never by a projected `pa.x` with
the same name. An **unqualified** key that matches an output name binds to the output
column, but only when the key *is* that name. A name **nested** inside a key
(`ORDER BY lbl+0`, `-lbl`) is an ordinary column reference resolved against the FROM
relations, as `findTargetlistEntrySQL92`'s one-field check requires. Mixing the two in one
`ORDER BY` (`ORDER BY x, dropped_col`) sorts below the projection whenever every key can be
read from the base row. Under a `GROUP BY`, a key the select list drops, either a grouping
column (`ORDER BY k`, `k*2`) or an aggregate (`ORDER BY count(*)`, `sum(v)/count(*)`), is
carried on the Aggregate as a Postgres **resjunk** column, which a `Plan::JunkFilter` trims
off above the Sort. An **ungrouped** base column is `42803`, as in Postgres. See `EDGES.md`
row 3 and `docs/coercion.md` § Where the plan-time coercion pass reaches.

### `UPDATE … [alias] SET … FROM other [a] WHERE …`

Inner-joins a source into the UPDATE (a target alias is supported). Each target row is
updated at most once, by the first matching source row (PG leaves a multi-match
unspecified). **`SET`, `WHERE` and `RETURNING` all resolve over one combined namespace**:
the target under its effective name plus the source under its own. We checked this on 18.6
with `amb_t(id, v)` as the target and `amb_s(id, v, k)` as the source aliased `s`, and
every result is the same here:

- `RETURNING *` gives five columns (`id | v | id | v | k`: target first, then the source,
  in FROM order).
- `RETURNING s.*` gives the source's three, and `RETURNING amb_t.*` the target's two.
- `RETURNING s.k` and a bare `RETURNING k` give `7`.
- `RETURNING v` (both carry it) is `42702`.
- `RETURNING amb_s.k` (the alias hides the table name) is `42P01`.
- `RETURNING s.nosuch` is `42703`.

Each result is the same whether or not a row matches, and a `Describe` on a statement that
matches nothing still describes all five columns.

The executor resolves a `RETURNING` item by the **`(relation, column)` pair**, not by a
built-up `relation.column` string. So a column a user names with a quoted dot in it can't
stand in for another relation's column. `RETURNING "wt.v"` over a target `wt(id, v)` and a
source `sd s(id, "wt.v")` returns the source's column, and `RETURNING "s.k"` over a target
`qt(id, "s.k")` and a source `ws s(id, v, k)` returns the target's, as 18.6 does both ways.
A flat-string key would get both wrong, silently. (The join scope has a collision of its
own, by a different mechanism; see `EDGES.md`.)

The plan-time namespace comes from one function (`returning_relations`) that covers every
one of the five node kinds a source can lower to: base `Scan`, `VirtualScan`, `CteScan`,
`SubqueryScan` and `FunctionScan`. Covering every *kind* isn't the same as counting every
*claimant*, and over a catalog view pglet only counts and projects the columns it emits
(see `EDGES.md`). A `Scan` of a table the catalog doesn't have registers **nothing**, so
the executor's own `42P01` for the missing source wins, as it does on 18.6.

The `DELETE … USING` counterpart is a **parser** gap (`42601`), so pglet has one join-DML
statement, not two. Nothing in `lower_returning` is UPDATE-specific, so closing that gap
means passing the source in. The join `WHERE` eval is correct-or-loud: an error
propagates instead of being swallowed into a silent 0-row update.

A `FROM`-clause `unnest(…)` is a source like the others throughout: for the claimant
count, for `SET`/`WHERE` type resolution, **and for a qualified reference**.
`UPDATE t SET … FROM unnest(ARRAY['x','y']) AS u WHERE u.u = 'x'` works on both engines,
with the executor deriving a relation name for the `FunctionScan` node. The same executor
arm reaches `execute_join`'s column map. So on the `SELECT` side,
`SELECT u.q FROM jn JOIN unnest(ARRAY[1,2]) AS u(q) ON true` over a `jn` with its own `q`
returns the unnest's `1, 2`, not `jn`'s value, and
`SELECT 1 FROM jn, unnest(ARRAY[1,2]) AS t(q) WHERE t.q = 1` returns its one row, each as
18.6 does. See `EDGES.md` for the table of cases.

`SELECT v.* FROM unnest(…) AS v` is `0A000` here **when no base table has the same name as
the alias**. The *`SELECT`* qualified-wildcard expansion is keyed on the name and has no
arm for an `unnest` item. 18.6 expands it, and pglet expands that wildcard over a base
table, a CTE, a derived table and a catalog view alike. When a base table *does* share the
name, the same expansion is **silent**: it returns that table's columns against the
unnest's relation (`EDGES.md`). A `RETURNING u.*` over the same source expands too.

### TRUNCATE

Multiple tables plus `CASCADE` (which clears FK dependents), and `RESTART`/`CONTINUE
IDENTITY`. An FK-referenced table without `CASCADE` errors with `0A000`. Like Postgres, it
clears rows directly, with no FK `ON DELETE` actions. `RESTART IDENTITY` resets the
**owned** sequences of everything being truncated (`Sequence.owner`, Postgres's
`getOwnedSequences`) to their `START WITH`. It doesn't reset them to zero, and it doesn't
use the set of sequences that the columns' `nextval` defaults name. That set gets it wrong
both ways: it would restart a sequence that's only referenced, which Postgres leaves
alone, and skip an owned one whose column has no default. A child pulled in by `CASCADE`
has its sequences included. **Divergence:** pglet's parser accepts the tail clauses in
either order, where Postgres requires `RESTART IDENTITY` before `CASCADE` (`42601`).

### `DROP SCHEMA` / `CREATE SCHEMA`

This covers the migration/test reset idiom `DROP SCHEMA public CASCADE; CREATE SCHEMA
public`. pglet models one implicit `public` schema, so `DROP SCHEMA public CASCADE` clears
the whole catalog (tables, indexes, enums, sequences) and `CREATE SCHEMA public` is a no-op
that succeeds. If `public` holds a table, an enum or a sequence, a drop without `CASCADE`
is refused (`2BP01`) and everything is kept. (If it holds only an extension, pglet accepts
the drop where 18.6 refuses; `EDGES.md` row 72.) A non-`public` name is `3F000` (a no-op
under `IF EXISTS`), and `CREATE SCHEMA <other>` is `0A000` (only `public` is modeled).

### `DROP TABLE` / `DROP INDEX` dependencies

`DROP TABLE [CASCADE|RESTRICT]` tracks FK **and** sequence dependents through the shared
resolver in `executor/dependencies.rs`. A foreign key declared on **another** table that
references this one blocks a bare or `RESTRICT` drop with `2BP01`. `CASCADE` drops
**only that constraint**: the child table and its rows survive, unconstrained from then on,
matching Postgres 18.6. A table's own FKs (including a self-reference) go with it and never
block.

A sequence the table **owns** goes with it (or, on `ALTER TABLE … DROP COLUMN`, a sequence
the column owns). That's the AUTO edge. A column *outside the set being deleted* whose
`DEFAULT` draws from that sequence blocks with `2BP01`, and `CASCADE` drops that column's
default and keeps the column and its rows. The deletion set is what tells them apart. A
table's own columns are inside it (so `CREATE TABLE t (a SERIAL); DROP TABLE t`
succeeds), while a `DROP COLUMN`'s set is just the column, so a **sibling** column drawing
from the dropped column's sequence blocks.

Postgres lists every blocking dependent in one message. pglet names one, foreign key
first. Postgres's order is neither by kind nor by the dependent's own OID.
`findDependentObjects` sorts each recursion level by OID **descending** and appends depth
first, and `reportDependentObjects` reads the array back to front, so a column default
(reached through the sequence) is placed by the **sequence's** OID. We checked three
arrangements on 18.6. The first two rule out "by kind", and the first and third rule out
"by the dependent's own OID" in either direction. So there's no ordering rule to
reproduce, and the divergence is only about **completeness**.

`DROP INDEX` **refuses** the index behind a declared `PRIMARY KEY` / `UNIQUE` (`2BP01`,
with or without `CASCADE`), read from the catalog's persisted constraint marker. A
standalone `CREATE UNIQUE INDEX` can still be dropped freely, which is the same split
Postgres makes. Dropping a constraint's index would silently stop enforcing the key
(duplicates get written and survive a restart) *and* remove it from `dump_sql`, which
classifies PK/UNIQUE from that same marker.

**Gaps:** `ALTER TABLE … DROP CONSTRAINT` isn't implemented (it's parsed and refused with
`0A000` over a table that exists). With the `DROP INDEX` route refused too, a declared
`PRIMARY KEY`/`UNIQUE` can't be removed short of `DROP TABLE`. Postgres's hint for the
refusal names `DROP CONSTRAINT`. pglet's message deliberately doesn't, since that statement
is refused here, and suggests dropping the table instead. A `CASCADE` emits no `NOTICE`.
`DROP INDEX` does **not** yet block on an FK that depends on a *plain* unique index covering
its referenced columns (PG gives `2BP01`). pglet's FK existence probe falls back to a scan
when the covering index is gone, so enforcement gets slower but stays correct.

### Arithmetic (`+` `-` `*` `/` `%` `^`)

Integer arithmetic stays in the integer domain (exact above 2^53) and **fails loudly** on
overflow (`22003`) or division by zero (`22012`). Mixed widths promote to the wider operand
type, **in either order and for all of `+ - * / %`**: `int2+int4` and `int4+int2` are both
`int4`, and anything with an `int8` is `int8`. Postgres registers a separate operator for
each ordered pair, with the same `prorettype` (`int24pl`/`int42pl` → `int4`). `%` has no
mixed-width operator, and resolves through the implicit int2→int4→int8 casts to the same
result. The rule only affects the declared type, since the eval path already widens.
Getting it wrong declared a **narrower** type than the bytes sent (`int2` over a four-byte
value), and tokio-postgres reports `error deserializing column` instead of misreading.
That's the same shape as the `float4`/`sum(real)`/`COALESCE(int4,int8)` entries elsewhere
in this table. One chain (`planner::arith_result_type`: temporal → float/`NUMERIC` →
integer width) serves both declared-type walkers, and `arithmetic/int_mixed_width_promotes`
pins every pair, with the wider-on-the-left direction as its control. A non-numeric operand
errors.

`%` is modulo over integers and numeric, with the sign following the dividend. There's
**no float `%`** (`42883`), and `%0` is `22012`. `^` is power. It returns `float8` unless
**both** operands resolve to numeric, because Postgres only registers `^(float8,float8)`
and `^(numeric,numeric)`, so `real ^ real` is `double precision` too, not `real`.
`0^negative` and a negative base to a fractional power are `2201F`, and float overflow is
`22003`. `^` is **left-associative** and binds tighter than `*` but looser than unary `-`,
so `-2^2=4`.

`mod(a, b)` and `power(a, b)` / `pow(a, b)` are `%` and `^` written as calls. 18.6
registers each with its operator's signatures and body, so they evaluate, type and check
their operands the way the operators do, but named after the function. The column is
`mod` / `power` / `pow`, the rendering is `mod(a, 2)`, and an operand pair with no
signature is `42883 function mod(double precision, integer) does not exist`. A bare string
literal next to a number literal takes the type the call resolves to (`mod(7, '3')` is `1`,
and `power('2', '3')` is `8` as `double precision`). Two of them are `42725` for `mod`,
which has four signatures that fit them equally well. Next to a column, a parameter, a
written cast (`mod(7::int8, '3')`) or any other expression, the literal stays `text`, as it
does under the operators (`EDGES.md` row 262). An untyped `$N` next to a number isn't typed
by the call either (`docs/param_binding.md`). They inherit the operators' residuals: a
`numeric` raised to a non-integer `numeric` power is `0A000`, a bare `NULL` next to a
`numeric` declares `double precision` on the extended protocol (row 54), and the catalog
rendering leaves out 18.6's implicit `::double precision` on `power`'s arguments
(`docs/deparse.md` § What is recorded, not reproduced).

Unary `+` keeps the value but is **kept as a node** (`Expr::UnaryPlus`), so `+1` isn't
structurally a constant. The SQL92 ordinal rules depend on that. **Divergence:** its
operand isn't type-checked, so `+'abc'::text` gives `abc` where PG raises `42883`, and
`+'abc'` gives `abc` where PG raises `22P02`. Also, `WHERE indexed = +1` skips the index
for a row filter. That's slower, and gives the same rows wherever the two spellings
evaluate the same leaves. A *non-matching* probe leaves a `DEFAULT`/`$N` next to it unevaluated,
since no row reaches it (the starved-leaf exception, `docs/indexes.md`), so over an indexed `a` holding `1`, `a = +2 AND DEFAULT IS NULL` is `42601` where
`a = 2 AND DEFAULT IS NULL` gives `(0 rows)` (`EDGES.md` row 56).

Unary `-` works the other way, through the fold. `WHERE indexed = -1` is a bare constant,
so index selection **offers** the index. Over an encodable key the rows match the row
filter's wherever both evaluate the same leaves. A probe that matches no row leaves a
`DEFAULT`/`$N` next to it unevaluated (the starved-leaf exception): `(0 rows)` with acceleration, against `42601` for the `DEFAULT`
or pglet's own `22000` for the unbound `$N` in the stripped build (`EDGES.md` row 56). That's
why the negative-probe fixtures carry `@profile-lock`.

**`float4 op float4` is `float4`, computed in f32** (Postgres's `float4_pl`/`_mi`/`_mul`/
`_div`), so `0.1::real + 0.2::real` is `0.3` and not `0.30000000447034836`. The
overflow, underflow and divide-by-zero checks mirror the f64 path. `float4` mixed with
int, numeric or float8 widens to **`float8`**, which is what Postgres does. We checked
every mixed pair. It's deliberately the *opposite* of the `COALESCE`/`CASE` common-type
rule (see that row).

**Divergence (silent):** a `text` left operand (`tx + 1`, `tx + b`) returns over an empty
table where 18.6 refuses with `42883`. It declares `int4`/`int8` in the simple-query
`RowDescription` while `Describe` says `text`, and it's pglet's own `42804` once a row
reaches it. A `float4` next to an untyped `NULL` (`f4 + NULL`) declares `701` on the simple
leg where 18.6 declares `700`, and `^` between a `numeric` and an untyped `NULL`
(`NULL ^ n`) declares `701` on `Describe` where 18.6 declares `1700` (`EDGES.md` row 54).

### Numeric narrowing casts (`int8::int4`, `float8::int4`, …)

These **fail loudly** when out of range (`22003`, like the arithmetic path) instead of
silently wrapping or saturating. Float to int rounds ties to even (Postgres `rint`).
float8 to real overflow (a finite double becoming ±inf) errors. A `real` or
`double precision` **read from text**, or converted from `numeric`, is range-checked the
way `float4in` / `float8in` check it. A decimal that overflows the type, or a nonzero one
that underflows it to zero, is `22003 "…" is out of range for type real`, never the
`±Infinity` / `0` it rounds to (`EDGES.md` row 149). The residual is a `numeric` operand
next to a float in an operator: it converts without the check (`1e400 > 1::float8` is `t`
here and `22003` on 18.6).

### INNER/LEFT/RIGHT/FULL / CROSS JOIN, comma-join, `JOIN … USING`

Joins are nested-loop only. **`CROSS JOIN` and comma-join** (`FROM a, b`) give the
Cartesian product (a left-deep `Join ON true`). **`JOIN … USING (c…)`** desugars to
`ON a.c = b.c AND …` and merges each USING column into ONE output
(`COALESCE(left.c, right.c)`, the same for every join type). The merged columns come first
(in USING order), then the rest of the left side, then the rest of the right. The raw
`t1.c`/`t2.c` can still be qualified, a bare `c` or `*` shows the merged one, and chained
USING composes. When the merged columns have **different types**, a bare `c` is declared at
the left side's type where 18.6 declares the common type (`integer` over a `bigint`
value), while `*` declares 18.6's (`EDGES.md` row 309). A `SELECT *` over any join is
expanded at plan time to the bare and merged output columns.

**NATURAL JOIN** stays fail-loud (`0A000`). Aliased columns resolve through chains of three
or more tables (left-deep), and the ON predicate is correct-or-loud (an eval error
propagates). A missing USING column is `42703`, and an ambiguous one is `42702`. An
ambiguous **unqualified** reference that matches more than one relation
(`SELECT num FROM a JOIN b ON a.num=b.num`, or a bare column in an `ON`) is `42702` too.
That uses a tiered exact-then-suffix resolver shared with the USING check, so a merged
USING column (an exact bare match) stays unambiguous.

**`JOIN` binds tighter than comma**, as in PG: `FROM c, a JOIN b ON c.num=b.num` fails
loudly with `42P01`. An inner `JOIN`'s `ON`/`USING` only reaches its own chain, not a
sibling comma item, so `FROM t1, t2 JOIN t3 USING(num)`, with all three carrying `num`, is
unambiguous. pglet resolves the `ON` at **eval time**, so the out-of-scope reference only
shows up once the inner join evaluates a row. An inner join with no rows returns 0 rows
where PG rejects it at plan time.

A comparison in the `ON` condition runs the plan-time coercion pass against the whole FROM
clause's scope, so a bare literal against a non-string column
(`ON g.id = g2.id AND g2.id = '<uuid>'`) resolves instead of raising `42883`. Because that
scope covers every FROM relation and not just the join's own chain, resolution there is a
little more permissive than Postgres's. That costs nothing, since coercion only inserts a
cast and an out-of-chain reference is still rejected (`42P01` qualified, `42703` bare,
both matching Postgres). See `EDGES.md` row 3 and `docs/coercion.md` § Where the plan-time
coercion pass reaches.

### Aggregates, GROUP BY, HAVING

`sum` over `smallint`/`integer` is an exact `int8` (a loud `22003` past it). `sum(bigint)`,
and `sum`/`avg` over `numeric`, are **exact** `numeric`. `avg` over `smallint`/`integer`/
`bigint` is the exact `numeric` quotient at 18.6's `select_div_scale`: `3`, `4` and `6`
average to `4.3333333333333333`, and three `bigint` maxima sum to `27670116110564327421`,
as 18.6's `int8_avg` / `numeric_avg` give.

Sometimes a row's value isn't of the argument's declared type. That happens with a branch
pglet leaves unconverted (`EDGES.md` row 152), or with an operator over an enclosing
query's column inside a correlated subquery, which pglet declares at its other operand's
type whatever the column holds (row 2). In those cases an exact `sum`/`avg` reads an integer
of any width exactly, under an integer or a `numeric` declaration, and refuses any other
value with `0A000` instead of skipping it.

The `numeric` results are only given where the argument's integer type is **certain**.
That covers: a table's column; a column of a derived table, a `WITH` query, a `VALUES`
list or a set operation whose defining expression is itself certain; a `JOIN … USING`
column merged from columns of the same type; a literal, a cast, a scalar subquery whose
own column is certain, a `date` minus a `date`, and the numeric-category arithmetic and
folds the coercion pass types over those. A `$N` or an untyped `NULL` branch of a fold
takes the fold's type (as an operand of arithmetic it leaves the operator untyped)
([coercion.md](coercion.md) § A computed branch is typed where its type is certain). The
pass decides this once and records it on the aggregate, and everything that reads the
result type reads that record, so both protocols' descriptors and the accumulator agree on
the aggregate's type.

In these cases `avg` stays `double precision` and `sum(bigint)` stays `bigint`: a
relation's column defined by a window function, a FROM `unnest` of an `ARRAY[…]`
constructor, a set-returning `unnest` in a select list, a bare reference to a `USING` merge
of different integer types, a recursive `WITH` query's column whose recursive term is an
untyped `NULL`, a `RETURNING` column of a data-modifying `WITH` query, a catalog view's
column, an enclosing query's column inside a correlated subquery
(`(SELECT avg(t.i + u.i) FROM u)`, and any fold over that subquery), a column defined by a
scalar subquery over one of those, or a fold with one of those (the column, or a subquery
over it) as a branch or under one. These are the cases we measured, listed in `EDGES.md`
row 2. A cast (`avg(x::int)`) gives PostgreSQL's result.

`AVG` over `interval` is `interval` on both, and over `real`/`float8` it's `float8` on
both. **`sum(real)` accumulates in f32**, not f64. `pg_aggregate.dat` gives it
`aggtransfn => float4pl, aggtranstype => float4`, so every partial sum rounds to `real`:
ten `0.1::real`s sum to `1`, not f64's `1.0000000149011612`, and the value sent is a
`real`, matching the declared type. `avg(real)` is `double precision` on both (Postgres's
`float4_accum` goes through float8), and `min`/`max`/`count` over `real` agree with 18.6.
A float `sum` is a chain of the `+` operator's steps (`float4pl`/`float8pl`) from its first
input, so a sum of `-0`s is `-0` and an overflow from finite inputs is `22003`. A float
`avg` runs `float8_accum`, whose running sum of squares is overflow-checked too (`avg` over
`1e200` and `-1e200` is `22003` on both).

**A float or `interval` `sum`/`avg(DISTINCT …)` adds in sort order.** 18.6 sorts a
`DISTINCT` aggregate's input by the argument type's order and feeds the deduplicated values
in that order. For those two types pglet adds them in that order too, a float's in
`float8_cmp_internal`'s order with `NaN` last, so `sum(DISTINCT x)` over `1e16`, `-1e16`,
`1` is `0` on both. An integer or `numeric` one is exact, so the order doesn't matter. A
float aggregate over an expression on a column whose values have mixed types still adds in
input order (`EDGES.md` row 152). A plain aggregate next to a `DISTINCT` one keeps input
order where 18.6 may presort it (`EDGES.md` row 297), and `min` over equal values with
different spellings returns the first where 18.6's scan returns the last (row 298).

**`GROUP BY` groups by the same equality `DISTINCT` dedups by.** 18.6 groups and dedups
by the type's single default equality, so `{"a":1.0}` and `{"a":1}`, `ARRAY[1.0]` and
`ARRAY[1.00]`, or `ARRAY['1 mon'::interval]` and `ARRAY['30 days'::interval]` are one
group. A group shows the spelling seen first. Groups come out in first-seen order, which
neither engine promises.

**An aggregate under an expression works**: `round(avg(x), 2)`,
`CASE WHEN count(*) > 0 THEN … END`, `abs(sum(x))`, `concat('n=', count(*))`,
`jsonb_build_object('n', count(*))`, `greatest(max(a), 1)`, `HAVING round(avg(b)) > 1`,
`ORDER BY abs(sum(b))`. The grouping computes the aggregates and evaluates the expressions
above them through the ordinary scalar walk ([scope.md](scope.md) § A grouping query's
aggregates are values the scalar walk reads by node). The aggregates under an expression
are computed before it's evaluated, whether or not it reads their values, just as 18.6
computes every aggregate that survives its planner's constant folding. So
`CASE WHEN count(*) > 0 THEN max(a) ELSE sum(1/b) END` raises the `ELSE` sum's `22012`, and
so does `coalesce(max(a), sum(1/b))`.

An aggregate is only dropped with its branch when the deciding expression is a constant by
its shape: a literal `true` / `false` / `NULL`, a comparison of two number literals
(including a simple `CASE`'s operand against an arm's value), `NOT` / `AND` / `OR` / an
`IS` test over those, and for a `COALESCE`, an earlier number or boolean literal. So
`CASE WHEN false THEN sum(1/b) END`, `CASE WHEN 1 = 1 THEN 0 ELSE sum(1/b) END`,
`coalesce(1, sum(1/b))` and `true OR sum(1/b) > 0` all return a result. A decider that 18.6
folds but isn't one of those keeps its aggregates here, which errors where 18.6 returns a
result and the aggregate fails: `CASE WHEN length('abc') = 3 THEN 0 ELSE sum(1/b) END`, a
bound `$1` condition, `NULL::int + sum(1/b)`, `sum(1/b) > 0 OR true` (`EDGES.md` row 89's
residuals).

An aggregate over a column or a literal keeps its operand's type check where it's dropped,
and where no group is computed: `true OR sum(text_col) > 0` is `42883` as on 18.6, and so
is `sum(text_col)` over a grouping that forms no group. An operand that's computed, a bare
`$N` or a `DEFAULT` isn't checked there, and `sum` / `avg` over `time` is refused (rows 89
and 307). A subquery in a grouping query's expressions is still `0A000` (row 89). So is an
aggregate in a position the grouping doesn't evaluate (a window function's `ORDER BY` or
`PARTITION BY`, a set-returning `unnest(ARRAY[max(x)])`, a `RETURNING` (row 4)), and one in
a `SELECT` with no `FROM` (`SELECT count(*)`, `1` on 18.6; row 89).

An aggregate whose argument holds another aggregate (`avg(avg(i))`, `sum(b + max(b))`) is
refused at plan time with 18.6's `42803 aggregate function calls cannot be nested`. In a
top-level `SELECT` that error comes after a missing column in its own argument or an
earlier select-list entry, and before a later clause's error, as 18.6 orders them. Inside a
subquery or a DML statement it wins over errors 18.6 raises first, and in front of an outer
signature 18.6 can't resolve (`avg(avg(tx))`) it returns a result where 18.6 says `42883`
(`EDGES.md` rows 302 and 2). pglet treats every aggregate inside a subquery as belonging
to that subquery. It has no outer-level aggregates, so `(SELECT max(t.tx) FROM u)` over an
outer `t` gives one result per outer row where 18.6 aggregates the outer query, and a pair
that 18.6 places at two levels is refused (row 303).

A projected column that's neither grouped nor aggregated **fails loudly at plan time**
(`42803`). Postgres's primary-key functional-dependency case is modeled
(`SELECT pk, other … GROUP BY pk` is allowed). This is validated over a single base table
and lenient on JOINs. It's also **qualifier-aware**, so a key naming an *enclosing* level's
relation (`SELECT o.id, (SELECT o.id FROM i … GROUP BY i.k)`) doesn't count as a local
grouping key.

**`GROUP BY` keys follow Postgres's `findTargetlistEntrySQL92` rules.** A bare integer is a
1-based **ordinal** into the wildcard-expanded select list, and resolves to that entry's
*expression* (`SELECT x+1 AS e … GROUP BY 1` groups by `x+1`; `0` or out of range is
`42P10`). Any other bare constant is `42601`. A bare identifier prefers a **FROM-clause
column** over an output alias with the same name. That's a precheck Postgres does for
`GROUP BY` only, which makes the precedence the **inverse** of `ORDER BY`/`DISTINCT ON`,
where the alias wins. Only then does it search the target list, where two entries with the
name and *different* expressions are `42702`. Equal expressions are accepted, and a no-op
cast is stripped first, as `coerce_type`'s elision does (a widening cast still counts as
different). A matched entry containing an aggregate is `42803`, and one containing a
window function is `42P20`, the same split `checkTargetlistEntrySQL92` makes. `GROUP BY +1`
is an ordinary constant key, not an ordinal (see the SELECT row). An aggregated `SELECT *`
expands to the table's columns instead of reporting `42703` on the star.

**Operand types are checked.** `SUM`/`AVG` accept the numeric family plus `interval` (both
exact for `interval`), and anything else is `42883`. `MIN`/`MAX` accept every sortable type
except `bool`, `jsonb`, and `uuid`, which Postgres's aggregate catalog leaves out. The check
prefers the argument's **static** type when it's known for certain (a column, literal,
cast, or bound param), matching Postgres's parse-time resolution. So `min(bool_col)` errors
on an empty table too, instead of passing on a small seed and failing once the table has
data. A `GROUP BY <ordinal>` under a JOIN groups by the target entry the ordinal names, like
Postgres (the JOIN grouping-*validity* check is skipped, so an ungrouped joined column is
accepted instead of giving `42803`).

`HAVING` and `GROUP BY` run the plan-time coercion pass against the same scope as the
projection. They're resolved **together**, since the grouping rule compares the three by
structural equality, and coercing one alone raises a spurious `42803`. So
`HAVING id = '<uuid>'` against a `uuid` column resolves instead of raising `42883`. See
`EDGES.md` row 3 and `docs/coercion.md` § Where the plan-time coercion pass reaches.

**Absent (fail loudly):** `array_agg`, `string_agg`, `json_agg` / `jsonb_agg`,
`jsonb_object_agg`, `bool_and` / `bool_or` / `every`, `stddev` and the ordered-set
aggregates (`percentile_cont`, `mode`) are `0A000`, and an aggregate's `FILTER (WHERE …)`
or its own `ORDER BY` is `42601` (`EDGES.md` row 44).

### `COALESCE` / `CASE` common types

Both resolve Postgres's `select_common_type` over **all** branches (category,
preferred-type and implicit-cast rules) and **cast every branch** to the result, so the
declared type and the returned value agree on every row. That holds for a branch set the
pass **can** resolve. A computed branch is typed when its result is in the numeric
category and certain (`i + 0`, `f * 2`, `length(s)`, `abs(x)`, `EXTRACT`), so
`COALESCE(i, f + 0)` is `double precision` and returns `2.5` as PostgreSQL does
([coercion.md](coercion.md) § A computed branch is typed where its type is certain). A
`$N` or an untyped `NULL` branch takes the set's target, and the parameter is described
at that type.

A set holding a computed branch outside those shapes keeps the target its written branches
give, or `double precision` where one of its computed branches is certainly that and the
rest are numeric. A set whose branches are *all* invisible to `concrete_branch_type` (a
bare `NULL` next to such an expression) gets no target and is left alone. So
`COALESCE`/`CASE`/`ARRAY` take the *first* known branch where this row's fold would take
the common type of all of them (`EDGES.md` row 65). The two declared-type walkers agree on
that branch (row 54's multi-operand gap is closed).

Three consequences worth knowing, each checked against the oracle:

- The fold **widens** instead of narrowing (`COALESCE(int8, int4)` is `bigint` either way).
- **`float4` beats `int` and `numeric`** here (`COALESCE(int4, real)` is `real`, because
  `int4 → float4` is an implicit cast and the reverse isn't). That's the *opposite* of the
  arithmetic promotion (`int4 + real` is `double precision`), and the two rules are
  deliberately separate.
- **Order matters** when two types are mutually implicit. `COALESCE(varchar, text)` is
  `varchar` while the reverse is `text`. **`CASE` feeds its `ELSE` branch in first**
  (`parse_expr.c` prepends `defresult`, "the default result is considered the most
  significant type"), so `CASE … THEN text ELSE varchar` is `varchar` and the swap is
  `text`.

A decimal literal is concrete (`COALESCE(int_col, 1.5)` is `numeric`). Having no common
type fails loudly **over a branch set the check can resolve**: **`42804`** across type
categories (`COALESCE(int_col, text_col)`) and **`42846`** within one category where the
conversion doesn't exist (`COALESCE(date_col, time_col)`, `COALESCE(time_col,
timestamp_col)`).

That qualifier is the same one this row opens with, and **different readers supply it in
different places**. The check passes `concrete_branch_type` (plus a bare-scalar-subquery
arm the pass can't use, since the pass would have to *cast* to what it reads;
`docs/coercion.md`) to all five constructs it covers. The pass reads `concrete_branch_type`
only for `COALESCE`/`CASE`/`ARRAY`, and `arg_known_type` for `greatest`/`least`/`nullif`,
and the second lacks the `Array` and `Alias` arms the first has.

Where the two agree (the `COALESCE` family), `select_common_type` folds only the branches
that reader resolves. So **any** branch it can't see (any computed expression, and also an
`INTERVAL` literal, which is parsed here to an `Expr::Bound`) is dropped from the fold and
never compared against a sibling, whether or not the set has a concrete member. Where they
differ (a variadic over one of those two arms), the pass folds the concrete sibling alone
and casts the argument it can't see to that target, and the check then reads a `Cast` at a
matching type. `greatest(ARRAY[1,2], tx)` returns `{1,2}` and
`nullif(set_config('x', tx, false), i)` returns NULL where 18.6 refuses with
`42804`/`42883`. The same `int[]` reached through an arm both readers have
(`greatest(a, tx)` over an `INT[]` column, `greatest(a::int[], tx)`) is `42804` on both.

With **no** concrete member the fold is `[None, None]`, the result is `Ok(None)`, and
nothing is cast either. So `COALESCE(length('ab'), upper('7'))` returns int4 `2` and
`ARRAY[length('ab'), upper('7')]` returns `_int4` `{2,7}` where 18.6 refuses with `42804`.
Where a short-circuit separates the branch that sets the type from the one that's
evaluated, the declared type and the value separate with it:
`CASE WHEN false THEN length('ab') ELSE upper('x') END` returns `X` under `23`.

With **a** concrete member, the pass casts the invisible branch to the siblings' target.
That keeps the declaration and the value together, but doesn't restore the missing refusal, in far
more ordinary spellings: `COALESCE(i, upper(tx))` returns `7`, `COALESCE(upper('7'), 1)`
returns int4 `7`, and `COALESCE(upper('x'), 1)` is `22P02` at eval where the cast fails,
all against 18.6's `42804`. The control is a set with no invisible branch
(`COALESCE(i, tx)`, `42804` on both). A concrete pair still triggers the check next to an
invisible branch (`COALESCE(i, tx, upper('7'))`), which is what makes the rule per-branch
and not per-set. That's `EDGES.md` row **65**, and it closes with the same widening as the
fold above.

The check covers **every clause that plans an expression**: projection, `WHERE`,
`JOIN … ON`, `GROUP BY`, `ORDER BY`, `HAVING` (whose aggregate branches are typed by their
argument), the `INSERT … VALUES` list, the `UPDATE … SET` right-hand side, and **stored
expressions**. A `CHECK`/index expression with no common type is rejected at DDL, where
Postgres rejects one that can't be resolved at all. Extending the check to stored
expressions also **brings pglet's known literal-as-text false rejection** into these
constructs. A stored expression is checked *as parsed*, without the planner's
literal-coercion pass, so a bare literal is typed as `text` instead of staying Postgres's
`unknown`. So `CHECK (COALESCE(n, '5') > 0)`, `CHECK (CASE … ELSE d END > '1999-01-01')`,
and `CREATE INDEX ((COALESCE(i, '7')))`, like `CHECK (n = '5')`, are rejected here and
accepted there. Same cause, same workaround (the explicit cast), and same real fix:
resolving a stored expression against its table's scope before recording it.

It runs in **one bottom-up, left-to-right walk** together with the operand rulebook, not
as a second pass, so the *leftmost* error wins whichever family it belongs to:
`d LIKE 'x%' AND COALESCE(k, x) IS NULL` gives the `LIKE`'s `42883`, and the reverse
spelling gives the `COALESCE`'s `42804`, matching Postgres both ways. The
`LIKE`/`||`/`->`/containment/`starts_with` operand checks run in that walk, at plan time,
so they fire on an empty table as Postgres does. `JOIN … USING (k)` over mismatched `k`
reports the operator (`42883`), because the USING columns are checked before the merged
`COALESCE`. An `UPDATE` whose `WHERE` **and** `SET` are both ill-typed reports the `WHERE`
(`analyze.c` transforms it first).

**Arrays unify element-wise.** `COALESCE(int_array, bigint_array)` is `bigint[]` because
`int4 → int8` is implicit, exactly as Postgres's `COERCION_PATH_ARRAYCOERCE` resolves it.
So `ARRAY[1] UNION ARRAY[1::bigint]` is `bigint[]` too, on both build profiles.

**`GREATEST`/`LEAST` are checked statically too**, which matters more for them than for
the others. Their eval-time fold drops NULL arguments first, so without the check,
`GREATEST(int_col, text_col)` would return `7` over a `(7, NULL)` row and nothing over an
empty table, where Postgres rejects the query outright. A
`CHECK (GREATEST(t, i) IS NOT NULL)` would be recorded and then fail every write that
supplies both columns.

### `IN` / `NOT IN` list common type

A plain column of a base table against two or more written constants (literals, or a cast
directly over one) is compared at 18.6's `select_common_type` over the column and those
items (`transformAExprIn`). A string literal is read as that type, and an item of another
type is cast to it when it's `real` / `double precision`. So `real_col IN (0.1, 1.5)`
matches the stored `0.1`, and `i IN ('1.0', 2.5)` reads the string as `numeric`. A single
item, and a set with no common type, each compare as one `=` per item, as in 18.6.

**Still compared item by item** (`EDGES.md` row 280, silent): a list under any other left
operand (an expression, a constant, a subquery, a derived table's or an enclosing query's
column, an `UPDATE … FROM` source's column); a list holding a column, a subquery, a
parameter or a computed item (`real_col IN ($1, 0.1)` misses the `0.1` row;
`real_col IN ($1, 0.1::real)` finds it); and a list inside a stored expression (`CHECK`,
`DEFAULT`, partial-index predicate, expression index key). See `docs/coercion.md` § An `IN`
list is compared at one type, and `tests/fixtures/in_list/`.

### Subqueries (scalar/IN/derived)

Correlated subqueries are supported (outer-scope names resolve at eval time via `EvalEnv`,
with alias-aware scoping), and the spine's correlated EXISTS queries run end to end.
`IN (SELECT …)` works in a `DELETE`/`UPDATE` `WHERE` too, as in the spine's
subquery-keyed eviction `DELETE … WHERE id IN (SELECT id … ORDER BY … OFFSET $n)`.
**Absent (fail loudly):** `= ANY (SELECT …)` / `ALL (SELECT …)` (`42601`), and a `WITH …`
or `VALUES …` body as a scalar or `IN` subquery (`0A000`).

### CTEs (WITH)

A CTE's `name(cols)` **column list** names the relation's columns for every reader. It
replaces the body's names by position, fills in from the body when the list is short, and
gives `42P10 WITH query "c" has N columns available but M columns specified` when it's
long. That error is raised while the `WITH` clause is analysed, so an **unreferenced** CTE
raises it too. The list never changes a type, and the body's own names go out of scope. A
duplicated *list* name is accepted (`gram.y` has no uniqueness rule) and only gives
`42702` when referenced. A duplicated **CTE** name is
`42712 WITH query name "c" specified more than once`, checked in declaration order.

`MATERIALIZED` / `NOT MATERIALIZED` are accepted and change nothing. pglet evaluates a
read-only CTE that doesn't reference itself at each reference, whatever the keyword says.
So a body calling `random()`, `gen_random_uuid()` or `clock_timestamp()` gives a fresh
result at each reference where 18.6 evaluates it once, with `MATERIALIZED` too (`EDGES.md`
row 316).

**Divergence:** 18.6 analyses a CTE body *before* the arity check, so
`WITH c(w,x) AS (SELECT 1 FROM nosuchtbl) SELECT 1` is `42P01` there and this `42P10`
here. pglet doesn't analyse an unreferenced body at all (`EDGES.md`), so the order follows
from that gap. A **bare `VALUES`** CTE body is `42601` (a `VALUES` statement is `0A000`,
the absent-but-loud row).

The duplicate verdict and every `42P10` are **recorded and raised after the whole list
has parsed**, duplicate first (`transformWithClause` before `analyzeCTE`). So a `42712`
wins over an earlier declaration's arity error, and a body's own **syntax** error wins
over both, as on 18.6. What's left, under a plain `WITH`, is a body error pglet raises
*during* the walk (a plan-time `42703`), which comes before the `42712` 18.6 gives. A
`WITH RECURSIVE` list re-parses for the syntax error and lets the deferred verdicts win
over the rest (`EDGES.md` row 80).

A **`RETURNING`-less** data-modifying body that the outer query references is **accepted
and its write performed** (the reference gives a zero-column relation), where 18.6 refuses
the whole statement with `0A000 WITH query "c" does not have a RETURNING clause` and writes
nothing. This is silent (`EDGES.md` row 61).

**Data-modifying CTEs** (`WITH x AS (UPDATE/DELETE/INSERT … RETURNING) … FROM x`) run
once, and their `RETURNING` rows are exposed as the relation (a `Plan::With` materializes
them into the `EvalEnv`, and a `CteScan` reads them). That includes `UPDATE … FROM other`
bodies.

**`WITH RECURSIVE`** resolves its list in **dependency order**. Every name is visible to
every body, so a CTE can read one declared after it, and a cycle of more than one item is
`0A000 mutual recursion between WITH items is not implemented`. Each self-referencing (recursive)
CTE, however many there are, is an entry of the `Plan::With`, which stays on the
lock-free MVCC path when nothing in it writes. Each one runs the semi-naive fixed-point
loop: the working table binds only the previous iteration's new rows, next to the finished
relations of the entries before it. `UNION` dedups whole rows against everything
accumulated so far, so cyclic graphs terminate. `UNION ALL` keeps everything, with a
100k-iteration cap (`54000`). The non-recursive term's types win: recursive rows are cast
each iteration, a mismatch is `42804`, and an `unknown` literal is converted to an enum
seed (`22P02` for a non-member). The CTE name shadows a real table with the same name.

A CTE that doesn't name itself is an ordinary CTE, data-modifying ones included. An
`INTERSECT` binds into the recursive term, and a body wrapped whole in more parentheses is
the same query. The statement after a `WITH` list is a query or a data-modifying
statement, as in `gram.y`. Anything else is `42601`, as are the `TABLE` and `MERGE`
spellings, which pglet doesn't parse anywhere.

Postgres's `checkWellFormedRecursion` verdicts are judged from a token pre-scan of the
list before any body is analysed. Those are: a self-reference in the non-recursive term,
more than once, on an outer join's nullable side or inside a sub-link (`42P19`); a
data-modifying CTE naming itself (`42P19`); and `ORDER BY`/`OFFSET`/`LIMIT`/`FOR UPDATE`
over the recursive query (`0A000`, once those clauses have passed their grammar's `42601`).
The `… within INTERSECT` / `… within EXCEPT` verdicts for a set operation inside the
recursive term (`42P19`) are judged as that body parses, after the bodies resolved before
it (`EDGES.md` row 80).

**Divergences (loud):**

- These are refused with `0A000` before anything runs: a body that writes next to a
  data-modifying CTE, a recursive CTE resolved after one, and under `WITH RECURSIVE` a
  second data-modifying CTE or a body naming a table one can write (directly, or through a
  foreign key's `CASCADE` / `SET NULL`). Each is raised before the data-modifying CTE's own
  execution error (`EDGES.md` rows 160 and 162).
- A recursive CTE whose non-recursive term is itself a set operation is `42P19`.
- A recursive CTE that 18.6 skips or stops early (unreferenced, or read under a `LIMIT`) is
  evaluated to its fixed point here. So a runaway one is `54000`, and an unreferenced one
  that raises an error raises it where 18.6 returns a result.
- A body that leads with a parenthesized query holding its whole `UNION`, followed by a
  clause over it, gets an error from that query's parse.
- A clause over the recursive query that 18.6's grammar reads and pglet doesn't
  (`USING OPERATOR(…)`) is refused with a different code (`EDGES.md` row 161).

A clause that a parenthesized query already carries is `42601` under either spelling, as
`insertSelectOptions` refuses it. If there's an analysis error inside the parentheses, or
under a plain `WITH` in an earlier item, pglet reports that error where 18.6's raw parse
reports the `42601` (`EDGES.md` rows 30 and 80).

**Silent divergences a `WITH` adds, plain `WITH` only:** a part of the statement that
reads a table a data-modifying CTE writes sees the write. Several data-modifying CTEs run
in declaration order, where 18.6's pull-driven order differs. That decides which of two
writers of one row gets it, and which serial values rows get (`EDGES.md` rows 160 and
162). A data-modifying CTE also carries its statement's own divergences, under either
spelling. Among them are a `RETURNING` subquery reading the statement's own write
(`EDGES.md` row 164, a read under `WITH RECURSIVE`) and a key that an
`ON DELETE SET NULL` nulls (row 163, a write). A CTE body carries the divergences of the
query it is, under either spelling. Among them are a parenthesized query's clause stacking,
a read, and a write where a data-modifying statement reads the CTE (`EDGES.md` row 167).
A FROM-clause derived-table self-reference is allowed (matching PG).

### Window functions

**Ranking only**: `ROW_NUMBER()` / `RANK()` / `DENSE_RANK()` over `[PARTITION BY …]
[ORDER BY …]` (honoring `NULLS FIRST/LAST`). There are no window frames (`ROWS`/`RANGE`),
no offset/value functions (`lag`/`lead`/`ntile`/`first_value`/…), and no
**aggregate over a window** (`SUM(x) OVER …`). Each of those is unsupported and fails
loudly. An argument list is refused the way 18.6 refuses it: `row_number(1) OVER ()` is
`42883`, `rank(a) OVER (…)` / `dense_rank(a) OVER (…)` are
`42809 WITHIN GROUP is required …`, and `row_number(*) OVER ()` works on both (`EDGES.md`
row 53). A trailing `ORDER BY` key that the window's output can't supply (a qualified or
nested name, an expression, a window function) is carried on the Window as a Postgres
**resjunk** column, because the Sort can't go below it.

### Transactions (BEGIN/COMMIT/ROLLBACK/SAVEPOINT)

One writer at a time. A block takes the writer at its first write, re-based at that point
on what's committed, and holds it until the block ends. Another connection's write waits
for it, bounded by `lock_timeout` (pglet's default is `1s`, then `55P03`). The embedded
`Database` can't wait, so it refuses with `55P03` instead, and it refuses another logical
connection's transaction control while a block is open.

The isolation level is honored: a `REPEATABLE READ` / `SERIALIZABLE` block keeps its
snapshot. The snapshot is taken by the block's first statement that takes one (a planned
statement other than transaction control, or a `COPY`). It's that statement's start if
it's the block's first write, otherwise `BEGIN`'s. The block's first write is `40001` if
the store moved past it (`EDGES.md` row 232). The settings pglet acts on (`lock_timeout`,
`default_transaction_isolation`) are transactional, as 18.6's GUCs are (`application_name`
isn't, row 236). There's no nested `BEGIN`.

**Savepoints work**: `SAVEPOINT n`, `ROLLBACK TO [SAVEPOINT] n`, `RELEASE [SAVEPOINT] n`,
with Postgres's rules. Outside a block it's `25P01`. A name the block doesn't hold is
`3B001` (and the block then aborts). A repeated name stacks and the most recent wins.
`ROLLBACK TO` can be repeated and destroys only the savepoints set after it, and it's the
only statement an aborted block accepts.

The **aborted block** is one `pglet::TransactionSession` that every leg drives (native
server, wasm bindings and embed facade), so a failed statement aborts the block on all of
them. So does an error at any of the native server's extended-protocol phases (including a
refused `Bind` or an unknown statement at `Describe`), and a binding failure at the embed
facade's `execute_bound`. The next statement is `25P02`, and the `COMMIT` reports tag
`ROLLBACK` and publishes nothing (including the failed statement's partial effects). The
check runs at Postgres's five phases (simple query, `Parse`, `Bind`, `Describe` ×2,
`Execute`), and the exit classifier reads the **lexer**, so `/* c */ COMMIT` ends the
block. A `COMMIT` that fails (the catalog's encoding refused with `54001`) rolls the block
back and ends it, as a failing `COMMIT` does on 18.6. A `COMMIT` that hits a refused write
ends rolled back, or fails the store (`PLAN.md` § 3).

Recorded and open: `EDGES.md` rows 211 and 216, and the writer's own costs against 18.6.
Those are: unrelated writes serialize (228), the embed refuses where it can't wait (229),
an aborted block holds the writer (230), a client can wait on its own block (231), the
coarse `40001` (232), no keepalive (233), and an unwinding holder (234).
`pg_advisory_xact_lock` (both overloads, supported as a top-level item of a `SELECT` with
no `FROM`) takes the writer, so every key is one lock and every write contends with it
(250). A statement whose plan fails on the block's uncommitted catalog waits for it on the
native server (the embed refuses it with `55P03`, 229). That's keyed on the plan, not on
18.6's locks (238, 240), and a read whose plan succeeds reads that catalog (221).

**Divergences:**

- A multi-statement simple-`Query` message runs each statement in autocommit. Postgres
  wraps it in one implicit transaction, so an error mid-batch rolls back the whole message
  there but leaves earlier statements committed here (`EDGES.md` rows 68 and 175). An
  extended-protocol pipeline up to one `Sync` does the same (`EDGES.md` row 141).
- An **inline** mode clause is parsed (`gram.y`'s `transaction_mode_list`, in any order and
  combination, comma optional). `ISOLATION LEVEL …` is carried to the block,
  `[NOT] DEFERRABLE` is accepted and does nothing, and `READ ONLY` is refused loudly
  (`0A000`, matching the `SET TRANSACTION READ ONLY` spelling). pglet has no read-only
  block, and accepting it would silently let through the write that 18.6 refuses with
  `25006`.
- `START TRANSACTION` still reports command tag `BEGIN` where 18.6 reports
  `START TRANSACTION`.
- `AND CHAIN` is absent, and so are `ABORT` / `PREPARE TRANSACTION`, which 18.6's aborted
  block accepts and pglet's parser doesn't model.
- A **nested `BEGIN`** is a no-op on both legs, matching 18.6's outcome, but 18.6 also
  emits `WARNING: there is already a transaction in progress` and neither leg does. The npm
  adapter is deliberately stricter: it gives its own `25001` for a raw overlapping `BEGIN`,
  since a single connection can't give that caller its own transaction. Its ergonomic
  `transaction()` under an already-open block runs as a savepoint instead.

Every rollback (`ROLLBACK`, `ROLLBACK TO`, a caught `DO … EXCEPTION`, a refused or aborted
`COMMIT`, a disconnect) restores the level's catalog and storage directory
unconditionally, since nothing but the block writes while it holds the writer. A block that
never wrote restores nothing, which keeps every other connection's commits. Two leftover
aborted-block cases are in `EDGES.md`: a comment-only simple query, and a parse-*analysis*
error reported as `25P02` where 18.6 reports its own `42601`.

### JSONB `->`/`->>`, `@>`, key/path ops, `jsonb_build_object`

Full `@>` containment and variadic `jsonb_build_object` (scan-based, or GIN-accelerated
for a top-level `data @> <const>` conjunct). The **key/path operators** are `?` (key or
element exists, including PG's top-level-scalar-string quirk), `?|`/`?&` (any/all keys
over `text[]`, skipping NULL array elements), `#>` (path to jsonb) and `#>>` (path to
text). A NULL path element gives NULL, an empty path returns the whole document, an array
index is strict and negative counts from the end, and a scalar mid-path gives NULL.

Their operands are checked (`planner::legality`'s `json_operator_legality`). A pair that
18.6 resolves no operator for is `42883`, except for these known departures:

- A `text`/`varchar` operand is accepted. It's parsed as JSON on the left, and as an array
  literal on the right of `?|`/`?&`/`#>`/`#>>`. So text that parses works
  (`'{"a":1}'::text ? 'a'` is `t` here, `42883` there), and text that doesn't is `22P02` on
  the left and `42846` on the right (`'{}'::jsonb ?| 'a'::text`), both `42883` there.
- Some untyped `NULL` pairs give NULL where 18.6 refuses with `42725` (`EDGES.md` row 187).
- An **enum** right operand is treated as `text` and accepted (`j ? m` is `t` here, `42883`
  there; row 190). An enum on the left is parsed as JSON like `text`, so a label that isn't
  JSON is `22P02` against `42883`.

`->`/`->>` take a **key operand Postgres registers**: `text` and `integer`. An `int2`
widens, and `int8`, `numeric`, `float8` and `bool` have no `(jsonb, …)` signature and fail
with `42883`. A negative array index counts from the end. A bare `$N` key is inferred as
`text`, matching Postgres's `(jsonb, text)` preference, so use `$N::int` for the index
form. A bare `$N` or string-literal right operand is typed or parsed as `text`/`text[]`.
The jsonpath language (`@?`/`@@`) stays fail-loud (for scope and size).

**Numbers are exact.** They're stored as `PgNumeric` (not `f64`), so a big integer or long
decimal round-trips losslessly, and two numbers beyond `2^53` stay distinct in
`ORDER BY`/`DISTINCT`. Number equality ignores scale (`1.0` = `1.00`, matching PG).
**Duplicate object keys** dedup last-wins (`{"a":1,"a":2}` becomes `{"a":2}`, matching
`jsonb_in`), and a `Value::Numeric` converted to jsonb becomes a JSON number
(`NaN`/`±Infinity` become the JSON string `"NaN"`/`"Infinity"`, as in PG). Number input
follows the strict **JSON grammar** (leading zeros, a bare trailing or leading dot, or a
leading `+` are `22P02`). A grammar-valid number past the numeric weight/scale cap is
`22003` (for both `::jsonb` and `::numeric`).

**The reader refuses what 18.6's JSON lexer refuses, and the renderer escapes a key the
same way it escapes a value.** A raw control character from `0x01` to `0x1F` inside a JSON
string is `22P02`, where 18.6's `json_lex_string` raises it. A raw NUL is `22021`, the code
both engines give for one. (The wire entry points catch a NUL at the encoding scan before
the reader is reached, which leaves the in-process ones.) Whitespace is exactly space,
tab, LF and CR at every position the reader skips it, so any other character where a token
starts is `22P02`. Which error a string gets depends on **position**. The raw-character
check is interleaved with escape decoding, so `"\u0000a<TAB>b"` is `22P05` and
`"a<TAB>b\u0000"` is `22P02`. A string's body is decoded before trailing junk is refused,
so `["\u0000"<FF>]` keeps its `22P05`. An object key goes through the same
`escape_json_string` a string value does, so the key's text matches 18.6 at every
renderer: the `::text`/`::varchar` casts, the value on the wire, `->>`/`#>>` over a
container, and the dump's `COPY` payload, whose restore loads on both legs.

**A binary-format `jsonb` parameter's first byte is its version.** It's always read, as
`jsonb_recv` reads it, and only `1` is supported. Anything else is
`XX000 unsupported jsonb version number <n>` over that byte's unsigned value, before the
server-encoding scan. A `jsonb[]` element gets the same check when the wire array header
names `jsonb`. The header's element OID is only checked against the parameter's own
element type when it names an array type pglet recognises (`42804`). So a header naming
another scalar type pglet recognises decodes its elements as that type instead and never
reaches the check. A header naming an unrecognised OID falls back to the parameter's
element type and does reach it. 18.6 matches that at or above its built-in-OID floor and
refuses with `42804` below it (`docs/param_binding.md`, `EDGES.md` row 140).

**`@>`/`<@` implement Postgres's full `JsonbDeepContains`**: object key/value
containment, **array subset** by membership (order and multiplicity don't matter), nested
containers recursively, and the documented top-level exception that *an array may contain
a primitive* (`'["a","b"]' @> '"a"'`). That exception only goes one way (a scalar never
contains an array) and does **not** apply when nested (`'{"a":[1,2]}' @> '{"a":1}'` is
false, since an object value's type must match). A container element on the right only
matches a container element on the left; kinds never cross. The GIN path stays sound:
`gin_decompose` is lossy but gives a superset, so the index only picks candidates, and this
is the exact recheck.

An **uncast** literal operand (`data @> '{"k":1}'`, `tags @> '{a}'`) resolves to the
other operand's type through the plan-time coercion pass, as Postgres resolves it from the
operator signature. The anchor can be any expression the pass can type from the column
scope plus the node (a column, a cast, a `->`/`#>` result, a `jsonb_build_object(…)`
call), not just a bare column, so `'{"a":1}'::jsonb @> '{"a":1}'` works as well as the
column form. A scalar **subquery** operand is still unresolved (it would need the
sub-plan's output type) and fails loudly.

A non-container operand is `42883` (`operator does not exist: text @> text`). Postgres
registers containment only over `jsonb @> jsonb` and `anyarray @> anyarray`, and resolves
`anyarray` per operand without unifying, so `integer[] @> bigint[]` is `42883` too. That
refusal reads pglet's array type, and an `ARRAY[…]` over an enum is `text[]` here, so
`ARRAY[m] @> ARRAY[t]` is `t` where 18.6 gives `42883 … e1[] @> text[]` (`EDGES.md` row
190).

**Object key order is canonical and has no meaning**, as in Postgres. Every jsonb value is
built through one canonicalization: keys deduplicated last-wins, then sorted **by length,
then bytewise** (`lengthCompareJsonbString`, so `{"b":1,"aa":2,"c":3}` renders
`{"b": 1, "c": 3, "aa": 2}`). So `'{"a":1,"b":2}' = '{"b":2,"a":1}'`, and
`DISTINCT`/`GROUP BY`/join dedup treat them as equal. `jsonb_build_object` goes through
the same canonicalization, so it dedups (`{"a": 2}`) like every other jsonb producer.

### `json` (as distinct from `jsonb`)

**`42704`** at a column, `ADD COLUMN`, `CAST`/`::`, a typed constant and `json[]`. It's
never an alias for `jsonb`. 18.6 keeps the text verbatim (key order, whitespace, duplicate
keys), which no `jsonb` value holds. A text-preserving `json` isn't built yet.

### ON CONFLICT

A *specified* conflict target must match a unique/PK index (`42P10` if none), which is
then probed. The match is **set equality** on the key parts, as `infer_arbiter_indexes`
does it: the plain columns are compared as a set and the key expressions by membership
both ways. So order doesn't matter and a repeat collapses on either half, while a subset
or a superset is still refused (`docs/indexes.md`). A target-less `DO NOTHING` arbitrates
against every unique constraint. `DO NOTHING` and `DO UPDATE … [WHERE]` with `EXCLUDED`
both work. A `DO UPDATE` whose probe finds a row the same statement already inserted or
updated is `21000 ON CONFLICT DO UPDATE command cannot affect row a second time`, with
18.6's hint. That's decided by the row's identity, not its key, and before the action's
`WHERE` (`EDGES.md` row 247). Expression and partial-index targets are supported;
`ON CONSTRAINT name` isn't.

A `DO UPDATE` action puts **both** the target relation and `excluded` in scope, as
Postgres does. Since `excluded` has the target's whole shape, a **bare** reference to any
target column is ambiguous (`42702`) in the `SET` right-hand side and in the `WHERE`, so
the increment idiom has to be written `SET n = t.n + 1`. (The `SET` target on the left is
the column being assigned, not a row reference, and stays bare. The arbiter target's own
`WHERE` only sees the target relation, so bare names are fine there.)

The `DO UPDATE` action is lowered in the planner, after the row source and before
`RETURNING`, so its right-hand sides go through the same plan-time resolve-and-check as
every other clause. An ill-typed one is rejected even when the statement hits **no
conflict** and the action never runs. **Name** resolution works the same way. The action's
namespace is complete by construction (the target plus `excluded`, with no enclosing
level), so its scope is closed, and an unresolvable `SET` target, right-hand side or action
`WHERE` is `42703` (or `42P01` for a qualifier naming neither relation) before any row is
looked at. The action's own checks run in PG's order: every right-hand side, then every
target name, then the action `WHERE`, then the `GENERATED ALWAYS` guard.

The `SET` list's **duplicate-target** refusal (`42601`) isn't one of those. It's a
rewriter check, so it runs after every clause of the statement that **pglet lowers**,
fires whether or not a row conflicts, and wins over the arbiter inference
(`docs/names.md`). That position only orders it against pglet's own pipeline, nothing
wider. A verdict pglet only reaches at execution (a row-source cell's implicit conversion,
a `VALUES` cell's bare name) comes *after* this check. So
`INSERT INTO u VALUES (5, 'notanint', true) ON CONFLICT (a) DO UPDATE SET b = 1, b = 2` is
`22P02` on 18.6 and `42601` here (the `nosuchcol` spelling is `42703` there and `42601`
here). That residual is recorded in `docs/names.md` § What the refusals do not carry,
along with the zero-row measurement that keeps the check where it is. What it doesn't win
over here, but does on 18.6, is that same `GENERATED ALWAYS` guard. The **arbiter target**
is neither resolved nor checked (see `EDGES.md`, *Statement-level error order*).

**Divergence:** the arbiter's partial-index `WHERE` is matched by structural **equality**
against the candidate index's own predicate (`index.predicate != target.predicate`
declines), where `infer_arbiter_indexes` only requires that the index's predicate be
**implied** by it. So a non-partial index, whose predicate is empty and so implied by
anything, matches there and can't be reached here:
`CREATE TABLE t (id INT PRIMARY KEY, n BIGINT); INSERT INTO t (id) VALUES (1) ON CONFLICT
(id) WHERE n > 0 DO NOTHING` inserts the row on 18.6 and is `42P10` here. It's loud, and
the `$N` spelling of the same statement types its parameter `{bigint}` on both, so the
divergence is in the matcher, not the clause.

A quoted `"EXCLUDED".c` and a `bogusrel.c` are `42P01` as on PG, and `excluded.nosuch` is
`42703`. `EvalEnv::resolve_outer_slot` (`executor/expr/env.rs`) stops at the frame that
owns the qualifier and declines when none does, so neither reaches the `excluded` frame by
bare column name.

**Divergence (silent and wrong):** a target-less `DO UPDATE` is *not* rejected. PG errors
at parse (`42601`, "requires inference specification or constraint name"), but pglet
silently falls through to a plain INSERT. A non-conflicting row is written as if the clause
weren't there, and a conflicting row raises `23505`, not `42601`.

**Across connections:** on the native server, a key that another connection's block holds
pending makes the statement wait for that block (bounded by `lock_timeout`; the embedded
`Database` refuses it with `55P03`). It then arbitrates over what the block committed.
After a commit, `DO UPDATE` updates the row that became visible and `DO NOTHING` skips it,
and after a rollback each one inserts, as 18.6 does after its wait. See `EDGES.md` row 98
(closed), and `docs/storage.md` for why a constraint is decided against a state no other
connection is changing.

### `CREATE`/`DROP INDEX CONCURRENTLY`

Parsed in `gram.y`'s keyword position. It decides one thing: `25001` inside a transaction
block (`Plan::forbidden_in_transaction_block`, checked by both hosts and by the `DO` body
and handler). Otherwise the index is built the same way as the plain form (the same
`indexdef`) and blocks what the plain form blocks. pglet has no *table*-scoped lock for the
concurrent form to weaken, and a build holds the process-wide schema + storage pair for
its whole duration. So under either spelling, an unrelated table's `INSERT` and a `SELECT`
from it each only return once the build has finished (we checked). 18.6, according to its
own `CREATE INDEX` documentation, "locks out writes (but not reads)" on the **indexed**
table for a plain build, and takes no lock that prevents an insert, update or delete for a
concurrent one. We checked both: during a plain build an unrelated table's `INSERT` goes
straight through while the indexed table's waits, and under `CONCURRENTLY` the indexed
table's goes straight through too. So pglet is the stricter engine here. A write staged
before the build in another connection's open block isn't invisible to it. The build
takes the writer, and on the native server waits for that block (`EDGES.md` row 103,
closed), where the embedded `Database` refuses it with `55P03`.

### `CREATE`/`DROP EXTENSION`

The full option tail is parsed. There's **no extension catalog**: the functions these
extensions provide are builtins here, so only the *name* is checked (`0A000` / `42704`,
plus `3F000` for the schema). Residuals: `docs/ddl.md` § Extensions.

### CREATE INDEX

Built and maintained. The name is optional (`CREATE [UNIQUE] INDEX [CONCURRENTLY] ON t
(…)`, never behind `IF NOT EXISTS`), and a missing one is chosen the way 18.6 chooses it:
`<table>_<key part names>_idx`, cut to 63 bytes, with a digit added to step around a taken
relation name (`docs/names.md` § An unnamed index's name; an expression part's name can
differ, `EDGES.md` row 277).

The planner uses an index for:

- equality point reads (`WHERE col = <value>`),
- range scans (`<`/`>`/`BETWEEN` become a bounded probe),
- single-key `ORDER BY [ASC|DESC] [LIMIT]` over a NOT NULL column (the `Sort` is elided and
  the scan stops early),
- **GIN** `@>` containment (`USING gin`, partial + correlated probe), for a top-level
  conjunct **and under an `OR`** (same-column union + scan-independent guards).

A simple `UPDATE` / `DELETE` collects its target rows through the same equality probe when
the rest of its `WHERE` can't fail, in row order (`docs/indexes.md` § A write's target
rows). A range, a GIN containment and an `UPDATE … FROM` still scan. There's no
composite / `NULLS FIRST|LAST` index use yet.

Index *use* is behind the default-on `index_acceleration` feature. The stripped
(`--no-default-features`) build drops it: the index is still built, enforced, and
introspected, and a full `Filter(Scan)` replaces the probe (the rows are *intended* to be
byte-identical). **Exact-or-decline** probing keeps that true against an *inexact* probe.
A coerced probe value only substitutes when it's exactly equivalent. Otherwise the planner
declines the index and a scan runs, which silently loses the speedup but never gives a
wrong row (`docs/indexes.md` § Index probes are exact-or-decline).

A `float4`/`float8` key is keyed in 18.6's `float8_ops` order (`-0` with `0`, and every
NaN as one key above `Infinity`) on every index route, including a GIN over a float array
and a foreign key into a float key (`docs/indexes.md` § A float key; fixtures
`float_key/`). A `jsonb` and an array key are keyed in 18.6's `jsonb_ops` / `array_ops`
order, with the key's byte order matching the comparator. That holds on each route that
builds a B-tree over one (a unique or primary key, including a partial unique one, and a
composite or expression key) and as a GIN element (`jsonb[]`). String order within one is
by bytes (`EDGES.md` row 43). The one shape refused is a **non-unique B-tree over a single
bare `jsonb` or array column**, `0A000` at the DDL, since the catalog has nowhere to record
its method next to the GIN that the same shape derives (`docs/indexes.md` § A container
key; `EDGES.md` row 25; fixtures `container_key/`).

**A measured exception to that byte-identity**: a *starved* query-position
`DEFAULT`/`$N` leaf (one an index probe matching no row leaves unevaluated), over an ordinary indexed `INT`. It's stated
in full, with the spellings that reach it and its symptom per backend, in
`docs/indexes.md` § Build profiles (`EDGES.md` row 56).

An index **predicate** and **key expression** are type-checked at `CREATE` against the
shared operand rulebook, so a checked operator (`WHERE data->'k' LIKE 'x%'`) fails there
instead of leaving a table that accepts DDL and rejects every write. A build can still
fail: no DDL check reads a composite key's second segment, an expression key's result
type, or a key expression that raises for a stored row (`(10 / a)` over a zero is `22012`
on both engines, from the build's entry sizing before the storage build). Such a failed
build leaves **nothing** behind: no catalog `Index`, no storage index def, no OID moved,
and the table still writable. The constraint routes behave the same way (`docs/ddl.md`
§ A failed index build).

**Divergences:**

- An **expression index** (`ON t(LOWER(x))`) is built and UNIQUE-enforced but never used
  to speed up queries. `WHERE LOWER(x) = …` always does a full scan (safe, just
  unoptimized).
- An **enum key column** is built and PK/UNIQUE-enforced but not used to speed up queries.
  The B-tree key encoder orders a label's bytes, while the engine orders by member
  position, so `single_column_index_for` declines it for its equality, range and
  `ORDER BY` `Sort`-elision consumers alike, and a `Filter(Scan)` runs (`docs/enums.md`,
  `docs/indexes.md`).
- `CREATE INDEX` **modifiers that don't reach the rendering**: a per-key sort direction
  and `NULLS` placement (`a DESC NULLS LAST`) and an operator class (`text_pattern_ops`,
  `jsonb_path_ops`) are parsed and discarded. `NULLS NOT DISTINCT` is kept and
  **enforced** (a second NULL row is `23505`, as on 18.6) but missing from
  `pg_indexes.indexdef`. These appear in the fuz spine's own shipped schema (`EDGES.md`
  row 92).

**Over-accepts (silent):** spellings that 18.6's grammar refuses are built here: a
`public.`-qualified index name, `IF NOT EXISTS CONCURRENTLY ON`, a qualified key column
`(t.a)`, an expression key without its parentheses, and a key of unknown type `((NULL))`
(`EDGES.md` row 284). A key expression or predicate calling a function 18.6 doesn't mark
`IMMUTABLE` (`now()`, `concat`, `timestamptz::text`) builds where 18.6 gives `42P17`
(`EDGES.md` row 285).

**Gaps (loud, checked):** `ON ONLY t`, a key's `COLLATE`, `INCLUDE (…)`, `WITH (…)` and
`TABLESPACE` are `42601`. `USING hash` is `0A000`. An index named `if` is `42601`, because
the word is read as the start of `IF NOT EXISTS`.

### Index entry size limits

Every entry is sized the way 18.6's `index_form_tuple` lays it out: `54000` at the size
field, the B-tree item limit and the GIN item limit, in 18.6's order. Loud over-refusals are
in `EDGES.md` rows 191 and 192, silent residuals in rows 95 and 104, and ordering
differences in rows 92, 43, 166 and 194.

### NOT NULL / CHECK constraints

Enforced at write time (`23502`/`23514`). A CHECK is stored as compiled `Expr` IR in the
catalog (persisted with postcard and cloned per statement, not re-parsed), and a NULL
predicate passes (SQL three-valued logic). The stored predicate is **type-checked at
`CREATE`/`ALTER`**, so a checked operator on wrong-typed operands
(`CHECK (int_col LIKE 'x%')`) fails there, where Postgres reports it, instead of making the
table unwritable. The check is static inference plus a representative-row probe, followed
by the **boolean-context check** on the predicate as a whole, where `cookConstraint` puts
`coerce_to_boolean`. `CHECK (a)` over an `INT` column is `42804 argument of CHECK` at DDL
on both engines, and an unresolved literal is converted there instead of refused
(`CHECK ('t')` is accepted, `CHECK ('abc')` is `22P02`). The *Boolean contexts* row has
that rule. Static inference is a strong filter, not a proof, so a value-dependent error can
still show up at write time (as in Postgres).

**Divergence (loud):** a quoted literal compared with a column of another type inside the
predicate (`CHECK (n > '0')` over an `int`, `CHECK (d > '2000-01-01')` over a `date`) is
`42883` at the DDL, where 18.6 reads the literal as the column's type and accepts it. Cast
the literal (`EDGES.md` row 196; the *`COALESCE` / `CASE`* row covers the wider
stored-expression form).

### PK / UNIQUE / FK constraints

**UNIQUE/PK** are enforced with `23505` (single-column, composite, partial,
`NULLS NOT DISTINCT`, and expression-keyed). A `CREATE UNIQUE INDEX` over rows that are
already duplicated is refused at **build** time under the index's own key equality:
`23505 could not create unique index "x"`, `D: Key (a)=(1) is duplicated.`, with 18.6's
field set. So `numeric` `1.5`/`1.50` count as duplicates there, as they do on write. A
duplicate pending on **another** connection is committed or gone by the time the build
scans, because the `CREATE UNIQUE INDEX` waits for that block on the native server and is
refused with `55P03` on the embed (`EDGES.md` row 103, closed). Checks are per row and
immediate, with no `DEFERRABLE`. The key is also held against another connection's write,
pending or committed. A pending write makes the later writer wait for its block on the
native server and then succeed or fail based on its outcome (the embed refuses it with
`55P03`). A `READ COMMITTED` block's first write re-bases it on what's committed, so a key
committed since its `BEGIN` is refused with `23505` (`EDGES.md` rows 98 and 218, closed).
At `REPEATABLE READ` / `SERIALIZABLE` that write is `40001` instead (row 232).

**FK** gives `23503` on the child side, with **both `ON DELETE` and `ON UPDATE` actions**
(CASCADE / SET NULL / RESTRICT / NO ACTION), **single AND composite (multi-column)**. A
composite key is MATCH SIMPLE (any NULL FK column skips the check). It matches the
referenced columns against a parent PK/UNIQUE **covering key regardless of order**
(`REFERENCES parent(y,x)` against `PK(x,y)` works), and matches whole tuples for cascade
and SET NULL.

`ON UPDATE` fires on a change to a referenced key, from an `UPDATE`, or from an
`ON CONFLICT DO UPDATE` after its writes and its `RETURNING`. It's **transitive** from one
`ON UPDATE` action to the next: actions are queued like 18.6's after-row triggers and
fired in order, so a key that moves twice moves its children twice. (A key that an
`ON DELETE SET NULL` nulls starts none; `EDGES.md` row 163.) An action's value is assigned
through the child column's assignment conversion (type and modifier), as 18.6's action
`UPDATE` does it. A `bigint` child of an `int` key stores an `int8`, an `int` child of a
`numeric` key stores `2.6` as `3`, and a `varchar(3)` child refuses `'abcdef'` with
`22001`. The value is converted once, when the action's turn comes and before its child
rows are looked up (`fk/update_cascade_bigint_child_of_int_key_unique_loud.sql`,
`fk/update_cascade_varchar_child_too_long_no_child_loud.sql` and the fixtures next to
them). Each child row an action rewrites is checked the way 18.6 checks the row the action
writes: NOT NULL, CHECK, and each index entry.

The queue fires once the statement's rows are written, so a later row's own `23514` /
`23502` / `23505` / `54000` wins over every foreign-key refusal, and a row's own key is
checked before the next row's actions run. A rewritten child queues its own actions and
the checks behind them. If the statement or an earlier action wrote the row, every key it
holds is checked. If neither did, only each key the action changed is checked. `NO ACTION`
refuses unless a parent row now holds the old key, `RESTRICT` refuses outright, and a check
of a version that an action has since rewritten is skipped. The cycles we tested (tables
referencing each other's keys, a table's two keys each referenced by its own column, a row
that is its own parent) end with 18.6's result (`fk/update_cascade_*_cycle.sql`,
`fk/update_self_ref_two_unique_keys_cascade.sql`,
`fk/update_self_ref_row_is_its_own_parent.sql`).

An `ON CONFLICT DO UPDATE` fires its queue after projecting `RETURNING`. So a `RETURNING`
error wins over an earlier row's `23503` / `23001`, a `RETURNING` subquery reads the
children as they were before the actions, and a row the statement inserts under a vacated
key satisfies `NO ACTION`. A plain `UPDATE` or `DELETE` runs its foreign-key work before
`RETURNING` (`EDGES.md` row 168), and inside a `WITH` the actions run when the `INSERT`
finishes (row 160). A `DELETE` plans its actions read-only before it removes a row, so a
violation aborts it before any child moves.

On the parent side, **RESTRICT gives `23001`** and **NO ACTION gives `23503`** (matching
Postgres for delete and update). Other refusals:

- An FK whose referenced columns have no covering unique key is `42830` at CREATE
  (single-column and composite alike).
- An arity mismatch is `42830`. A written list's mismatch is caught in the parser, before
  every verdict past the parse and before an `IF [NOT] EXISTS` skip (`EDGES.md` row 275).
- A column either list names that its table doesn't have is `42703`, before the covering
  key check. On `ADD COLUMN`, a self-reference finds the added column.
- A column pair that 18.6 can't implement through the referenced key's B-tree operator
  family (`text` or `numeric` referencing `int`, `int4[]` referencing `int8[]`, an enum
  against `text` or another enum) is `42804 foreign key constraint "…" cannot be
  implemented`, with 18.6's `D`, at each entry point where 18.6 raises it (`EDGES.md` row
  195, closed).
- A `MATCH FULL`/`PARTIAL` clause is `0A000`.

**Exotic non-spine gaps.** Each of these diverges:

- a `DELETE`'s mixed RESTRICT+CASCADE fan-in (its RESTRICT / NO ACTION decision reads the
  cascade closure built so far, in BFS order, where 18.6 runs a per-row RI-trigger queue);
- the firing order of the keys referencing one table, when one was added by `ALTER TABLE`
  (row 198);
- a self-referencing or cyclic CASCADE over a referencing row that an earlier statement of
  the transaction wrote (silent, `EDGES.md` row 165);
- an action's conversion refused over a key no child row references, which 18.6 refuses
  only under a custom plan (loud, row 200);
- an `INSERT`'s or upsert's child-side check as the row is written (loud, row 166).

Both foreign-key checks read a state no other connection is changing. A pending write makes
the other writer wait for its block on the native server (the embed refuses it with
`55P03`), and a write committed since a block's `BEGIN` is in the state the block's first
write re-bases it on (`EDGES.md` rows 100 and 101, closed).

**Undocumented edges:**

- `NULLS NOT DISTINCT` only works via `CREATE [UNIQUE] INDEX`. The **inline/table-level**
  `UNIQUE … NULLS NOT DISTINCT` constraint (PG 15+) isn't parsed (fails loudly).
- A `UNIQUE`/`PK` key over an **array or jsonb column** is sized uncompressed, so one that
  18.6 can only hold compressed is refused with `54000` (`EDGES.md` row 191).
- An FK accepts a **partial** unique index as its covering key (PG requires a non-partial
  one). That over-accepts at CREATE and then wrongly rejects, with `23503`, a child row
  whose parent falls outside the partial predicate.
- A `REFERENCES parent` with no explicit column list, against a parent that has **no
  primary key**, is `42830` where PG says `42704` (`there is no unique constraint matching
  given keys` vs `there is no primary key for referenced table`). Both are loud, with
  different codes (`EDGES.md` row 263).

**A table has at most one PRIMARY KEY.** That's checked at all three places one can be
created (`CREATE TABLE`, `ADD CONSTRAINT`, `ADD COLUMN`) with Postgres's own `42P16`. It
isn't treated as a name collision, since both keys can have valid names, and two keys
aren't merged even when identical. Two `UNIQUE`s over the same key columns **are** merged,
as `transformIndexConstraints` does (see the `ALTER TABLE … ADD CONSTRAINT` row).

**Repeated key columns, per constraint:** a repeated PK/UNIQUE key column is `42701`, and a
repeated FK *referenced* column is `42830`. `CREATE INDEX (a, a)` and an FK's
*referencing* list are still accepted.

**Divergences (silent):** a referential action written twice
(`ON DELETE CASCADE ON DELETE SET NULL`) is accepted and the later one kept, where 18.6
gives `42601` (`EDGES.md` row 289). A declared constraint can take the name 18.6 generates
for a column's `NOT NULL` (`<table>_<column>_not_null`), which 18.6 refuses with `42710`
for a `UNIQUE`, a foreign key or an `ADD CONSTRAINT` (`EDGES.md` row 290).

### INSERT column-list binding

Binds by name, and omitted columns take their DEFAULT. A row's length is checked the way
18.6 checks it, row by row in list order after the row's cells are resolved. It's checked
first against the first `VALUES` row (`42601 VALUES lists must all be the same length`),
then against the target list (`INSERT has more expressions than target columns`, or
`… more target columns than expressions` under a column list; without one, a short row
takes its defaults). The `INSERT … SELECT` source is checked the same way. There are
two exceptions: a cell error pglet only reaches at execution loses to the length check, and an
`INSERT … SELECT` source's error that pglet only reaches after the source's arity (such as
an unknown relation, an operator pglet only resolves at eval, in the target list or the
`WHERE`, or a name with no `FROM` to bind against) loses to that arity check (`EDGES.md`
rows 225, 226).

The list is a **set**: a column named twice is `42701`. That's checked at the points
`checkInsertTargets` sets (before the arity check, the cells' conversions and the identity
guard), interleaved per item, so a repeat wins over a *later* unknown name and loses to an
earlier one (`docs/names.md`). One boundary it does **not** share is the `INSERT … SELECT`
**source query**. 18.6 transforms it after the list, and pglet lowers it before, so the
source's own error wins here (`docs/names.md` § What the refusals do not carry).

The row-less **`INSERT … DEFAULT VALUES`** is the same thing with *every* column omitted.
It parses to one empty `VALUES` tuple, so it lowers to the same `Plan::Insert` as its
`VALUES (DEFAULT, …)` twin and gets `ON CONFLICT`, `RETURNING` (including wildcard and
correlated subqueries) and the identity rules unchanged. `gram.y` gives that alternative
neither a column list nor `OVERRIDING`, so `INSERT INTO t (a) DEFAULT VALUES` and
`INSERT INTO t OVERRIDING SYSTEM VALUE DEFAULT VALUES` are `42601` at the `DEFAULT` token
on both engines.

**Divergence:** under `OVERRIDING USER VALUE`, the identity column's cell is replaced
before it's read. So an ill-typed constant or an unknown name there is accepted where 18.6
refuses, a `$N` there drops out of the statement's parameters (native server: `08P01` at
`Bind`), and a `SELECT` source is `0A000` (`EDGES.md` row 287).

### Arrays (`text[]`/`uuid[]`/…), `@>`/`<@`, `ANY`/`ALL`, `unnest`

A real `Value::Array` with per-type array OIDs. Operations are scan-based (GIN-accelerated
for a top-level `@>` conjunct or an `OR`-union of same-column `@>`). Bound
`$N::text[]`/`$N::uuid[]` params work (literal and bound). An array literal takes the
**common type** of its elements under Postgres's `select_common_type`
(`ARRAY[1, 2.5]` is `numeric[]`, `ARRAY[1, '2']` is `int[]`, `ARRAY[2::int4, 1::int8]` is
`int8[]`, `ARRAY[1::int4, 2::real]` is `real[]`), and every element is **cast** to it at
plan time, so the declared array type and the constructed value agree. All-NULL or
all-string gives `text[]`.

A **FROM-clause `unnest(…)`**'s column type comes from the array **expression**, through
one `function_scan_column_types` that both declared-type walkers read. It never comes from
the evaluated `Value::Array`'s `element_type`, which can't carry the tz/no-tz bit a
`timestamptz` needs (that type has no distinct value here; only a column `TypeOid` tells
them apart). So `SELECT t FROM unnest(ARRAY['…'::timestamptz]) AS u(t)` declares
`timestamptz` (1184) on both legs, as 18.6 does, and the column-type-aware text encoder
keeps the UTC `+00` offset. Derived from the value, it would declare `timestamp` (1114) and
drop the offset, which is the silent-instant-shift problem. The evaluated array's element
type is still the fallback where static inference declines.

The item isn't lateral. An argument naming an earlier `FROM` item's column is `42703`, or,
inside a subquery whose enclosing query has a column with that name, reads that column. An
enclosing query's column written without its table's own name declares the item's column
`text`, or as another table's column when the qualifier is also that table's name
(`EDGES.md` rows 312 and 313).

That type comes from the **first element** for an `ARRAY[…]` constructor written in a
statement's own `FROM`, because the coercion pass doesn't unify its elements there.
`unnest(ARRAY[1, 2.5])` declares `integer` over `2.5` where 18.6 declares `numeric`
(`EDGES.md` row 308). A cast (`ARRAY[1, 2.5]::numeric[]`) gives 18.6's column type. An
aggregate in the array is `42803`, as on 18.6. An unnest's columns also reach the
plan-time coercion scope, so a `COALESCE` branch unifies against one:
`SELECT COALESCE(NULL, v) FROM unnest(ARRAY[1,2]) AS u(v)` is `int4` on both legs and both
engines. Fixtures: `types/unnest_timestamptz_declared_type.sql`,
`types/unnest_timestamptz_text.sql`, `select/unnest_coalesce_declared_type.sql`.

A **query-position** `unnest(<a non-array>)` is `42883` at plan time
(`planner::legality`'s `unnest_legality`, reached from the single
`check_expr_node_semantics` walk). That matches 18.6 in code and in protocol phase
(`Parse`), regardless of row count. The check reads `arg_known_type`, so an argument that
reader doesn't type (`unnest(i + 0)`, `unnest(length(tx))`, and the
`CASE`/`NULLIF`/`GREATEST`/`LEAST`/unary-`-`/`set_config(…)` spellings next to them) is
still deferred. The two `unknown` spellings (a bare `NULL` or string literal) are left
alone, because 18.6 gives `42725` there, not `42883`. **Only the closed half is
independent of row count**: every query-position residual is silent over zero rows and
`0A000` once a row reaches the leaf, so only the arity and DDL-position residuals error at
any row count. `EDGES.md` row 55 holds the four.

**A modifier next to a `[]` belongs to the element.** A canonical type name is decomposed
once, the `[]` taken off first and then the `(`. So `varchar(3)[]` is an array of
`varchar(3)`, and its cap is enforced element by element on store and on an explicit cast.
A `(…)` the element refuses takes the whole spelling down at one of the two
**name-bearing** `typenameTypeMod` refusals pglet reaches, which name different types.
`int4(3)[]` is `42601 … for type "int4[]"` (the **array**), and `'{ab}'::char(0)[]` /
`'{ab}'::numeric(0)[]` are the **element's** own `22023`, each matching 18.6. A third
layer names neither: the `int4` coercion of a `numeric` component, reached only for a
type that carries a modifier and before that type's own range checks.
`'{ab}'::numeric(2147483648)[]` is `22003` and `'{ab}'::numeric(1.5)[]` is `22P02`, each
about `integer`, on both engines (`docs/arrays.md`, `docs/coercion.md`; `EDGES.md` row 120,
closed, and row 124 for the carrier half).

**Divergence (pglet is stricter): a nested `ARRAY[ARRAY[…]]` fails loudly with `0A000`.**
Multi-dimensional arrays are a non-goal, so this is refused where Postgres returns
`{{1,2},{3,4}}`. The guard runs at plan time (a statically typed element) and at eval time
(`ARRAY[(SELECT ARRAY[1,2])]`, `ARRAY[array_col]`), so `CREATE INDEX … ((ARRAY[ARRAY[1,2]]))`
is never accepted and then left failing every INSERT with `0A000`. Postgres accepts all of
these. It rejects the **ragged** form `ARRAY[ARRAY[1,2],ARRAY[3]]` with `2202E`, where
pglet reports its own `0A000` (a code divergence, since pglet rejects all nesting before it
can tell the shapes apart). The nesting check runs **after** the common-type fold, never
before, so an element set that just fails to unify reports Postgres's own code whether or
not one element is an array: `ARRAY[bool_col, int_array_col]` is `42804` and
`ARRAY[int_array_col, text_array_col]` is `42846`, not a nesting complaint.

`x op ANY(array)` / `op ALL(array)` over an **empty** array short-circuits (`ANY` gives
false, `ALL` gives true) before the NULL left-hand side check, matching Postgres.

**Divergences:**

- A constructor over `timestamptz` / `varchar` / `name` elements is compared as
  `timestamp[]` / `text[]` by `@>`, `<@` and `=`. That's `42883` next to a `timestamptz[]`
  or `varchar[]` column where 18.6 returns a result, and a result next to `text[]` where
  18.6 gives `42883` (`EDGES.md` row 281).
- `ARRAY[…]::T[]` resolves the constructor before the cast (`ARRAY['NaN', 0]::float8[]` is
  `22P02`, and `ARRAY['01', 0]::text[]` is silently `{1,0}`; row 282).
- An array **literal** is read by 18.6's array grammar. One it refuses (`'{a,,b}'`,
  `'{a{b}'`, `'{a}x'`) is `22P02`, with an element's own input error before a later
  structural one. One that 18.6 reads as multidimensional or with explicit bounds is
  refused (`'{{a,b}}'` is `0A000`, `'[1:2]={a,b}'` is `22P02`; row 283).
- A Unicode space on a number, `uuid`, `date`, `time` or `interval` element is trimmed by
  that element's reader where 18.6 refuses it (row 294).

### GIN indexes for arrays / JSONB `@>`

`USING gin(col)` builds an inverted index, including over `jsonb[]` (each element keyed by
`jsonb`'s equality). A probe of a different type from the column's falls back to the
recheck. The planner sends a top-level `col @> <const>` conjunct to a `GinScan` + recheck
(partial indexes + correlated/`$N` probe), and an `OR` of same-column `@>` disjuncts (+
scan-independent guards) to a unioned `GinScan`. Those are the multi-key audit and
param-null-guard shapes.

### ENUM types

`CREATE TYPE … AS ENUM`.

- **A table's name is taken.** Its row type shares the type namespace, so an enum over a
  table, and a table or sequence over an enum, are `42710`, and `DROP TYPE <a table>` is
  `2BP01` (`EDGES.md` row 248).
- **Labels must be distinct.** A repeat is 18.6's own `23505` on
  `pg_enum_typid_label_index`, after the type name's `42710`. A label is a string literal,
  so `('A','a')` is two members.
- Members are validated on an `INSERT`, `UPDATE` or `COPY` of the column, when a column
  `DEFAULT` is cooked on every route, and on the value an `ADD COLUMN` back-fills
  (`EDGES.md` row 105, closed).
- Values are stored as TEXT and **reported on the wire as TEXT** (OID 25, where 18.6 sends
  the enum's own OID). The type itself has a persisted catalog OID, visible in
  `pg_type`/`pg_enum`.

Ordering is by **member position** when the value's enum is known, and by label bytes when
it isn't. It's known from a column, or from a scalar subquery's plan output column, through
one walk over the expression. A cast erases it when nothing next to the cast declares an
enum (`EDGES.md` row 104), and so does a subquery projecting an expression (row 107). Two
*different* enums next to each other cancel out to neither, though that pair is refused
before any order is needed (row 115). A set operation's or a `JOIN … USING` merge's column
takes the enum that **either** branch declares, as 18.6's `select_common_type` does with an
`unknown` literal. That covers the branch pairs 18.6 itself unifies, and the rest are
refused at the check described below.

An **`ARRAY[…]` expression** over enum-typed elements orders by member position too. The
walk works at the element level and the comparator reads that list at every element, so
`ARRAY[m] < ARRAY['ok'::vis]`, `ORDER BY ARRAY[m]` and `'ok' > ANY(ARRAY[m])` all agree
with 18.6, in a `CHECK` and a partial-index predicate as well as in a read (`EDGES.md` row
111, closed).

A cast to an enum (`'x'::vis`, `CAST(… AS vis)`, `vis 'x'`, `s::vis`, `$1::vis`) converts
the way 18.6's `enum_in` does, at the point where 18.6 runs it: a literal at plan time and
in the DDL walks, an undeclared `$N` at `Bind` when the cast is the use that types it, and
anything else on the value. So a label the enum doesn't have is `22P02` (`EDGES.md` row
241, closed; `docs/enums.md` § A cast to an enum converts). A `CHECK` or `DEFAULT` holding
such a conversion depends on the type (`DROP TYPE` is `2BP01`, and `CASCADE` drops or
clears it), and an index can't hold one (`42P17`). The cast still carries no enum
**identity** for ordering or for the checks below.

An enum's **identity** reaches the checks that decide whether two operands have a
comparison operator or a common type. It doesn't reach the jsonb key/path, `LIKE`-family,
`starts_with`, `||` and array-containment checks, or the string functions row 190 covers,
which see it as `text`. The checks it does reach model 18.6's `typcategory`. An enum next to a plain
`text`/`varchar`/`name` side is `42883 operator does not exist: vis > text` at the
comparisons, `nullif`, `IN`, `BETWEEN`, `op ANY`/`op ALL`, `IN (SELECT …)` and a
`JOIN … USING` merge. It's `42804 <construct> types vis and text cannot be matched` at
`greatest`/`least`/`COALESCE`/`CASE` and everywhere a common type is decided: a set
operation, a FROM-clause `VALUES` list, a CTE's and a recursive CTE's terms, and a DDL-time
`CHECK` or partial index. Each was checked byte-identical to 18.6 (rows 114 and 115,
closed). Two **different** enums share category `E`, so the fold succeeds and the
*conversion* fails with `42846 <construct> could not convert type <b> to <a>`. An enum
**array** is category `A` with its identity at the element:
`42846 could not convert type text[] to vis[]`.

**Gaps:**

- An enum array **column** (`vis[]`) is `type "vis" is not supported` at the column, at
  `ADD COLUMN` and at both cast-target spellings. It's loud everywhere, and over-rejects
  where 18.6 accepts.
- An enum **right** operand at a jsonb key/path operator, an enum at a `LIKE`-family
  operator, `starts_with`, a string function row 190 covers, `||` next to an enum,
  `integer`, `numeric`, `boolean` or `jsonb`, or `@>`/`<@` over an enum array, all work on
  the label as `text` where 18.6 gives `42883` (`j ? m`, `m LIKE 'a'`, `upper(m)`,
  `m || 1`, `ARRAY[m] @> ARRAY[t]`). These also write: at a predicate, an `UPDATE … SET`,
  and a `CHECK` or index key expression that the dump emits and stock 18.6 refuses (row
  190).
- The leftover silent over-**acceptances** of rows 114/115 don't include the `ARRAY[…]`
  **pair** or the mixed `op ANY`/`op ALL` array. An array's identity is the unification of
  its **typed** elements' identities, and an `unknown` element contributes nothing, as in
  `transformArrayExpr`'s `select_common_type`. So `ARRAY[m] < ARRAY['ok']`,
  `m = ANY(ARRAY[s,'ok'])` and `m > ANY(ARRAY['ok'])` are all `42883` on both. What's left
  there is:
  - a `NULL` next to a typed element, which this engine casts to the fold's target itself
    and so can't tell apart from a written `NULL::text`;
  - a sub-plan column the `IN (SELECT …)` reader can't type (`m IN (SELECT $1)`), which
    writes;
  - a bound `$N` whose enum target is decided by a **plan**, which writes at the
    set-operation check;
  - an operand that a written `::text` cast leaves undecidable, since that's the same node
    `m::vis` itself produces (row 104, which also writes:
    `DELETE FROM u WHERE m::vis > t` destroys a row against 18.6's `42883`).
- Rows 107 and 112 write too, by giving a wrong result where 18.6 gives a right one. A
  FROM-item `unnest`'s column carries no enum (row 112).
- A **written** cast of a member literal to the enum in a persisted expression, at every
  position `docs/ddl.md`'s render probe runs at (a column `DEFAULT`, a `CHECK`, an index
  predicate and an index **key** expression), dumps as `::text`. Stock 18.6 refuses that
  with `42804` or `42883`, depending on the statement rather than the position. (A cast of
  any other operand dumps as the enum cast and restores.) So the database doesn't restore
  into real Postgres, while a bare-literal spelling at each of the four positions replays
  cleanly. The restore errors per statement but is **silent per process**: psql exits `0`
  after a refused `CREATE TABLE` has thrown off the `COPY` framing of the tables after it
  (row 104).
- A real `pg_dump` of a database with an enum column doesn't restore into pglet, because of
  **two** independent `42601`s: the `ALTER TYPE … OWNER TO` the dump emits for the type,
  and the schema-qualified `m public.vis` column type (row 113).
- An enum with the same name as a builtin gives up the name to the builtin when pglet has
  that type (`int4`, `text`, as 18.6's search path does). The enum is then only reachable
  in a type position as `public.<name>`, which pglet doesn't parse (`DROP TYPE <name>` still
  drops it here, row 118). When pglet doesn't have the builtin (`oid`, `json`,
  `regclass`), the enum takes the name (row 118).

### Sequences / SERIAL auto-increment

An implicit sequence per column. An explicit `CREATE SEQUENCE <name> [START WITH n]` is
also accepted (the `pg_dump` form), and `nextval('<name>')` defaults resolve against it.
**Divergences (silent):** every draw is `+1`, ignoring `INCREMENT BY`/`MINVALUE`/
`MAXVALUE`/`CYCLE` (`EDGES.md` row 215), and ids can be re-issued after a reopen if the
advance wasn't saved first (row 214). It goes the other way too: a rolled-back
`ALTER SEQUENCE` naming `START WITH`, `INCREMENT BY`, `MINVALUE`, `MAXVALUE`, `CACHE`, `AS`
or `NO CYCLE`, or an identity column's `ALTER COLUMN … SET INCREMENT BY` (tested with
values that don't change any draw), keeps the block's draws, where 18.6 restores the old
cell and hands them out again (row 222). The full record is in § Sequences / SERIAL —
details, below.

#### Sequences / SERIAL — details

**`nextval`/`currval`/`setval` can be called as SQL expressions.** `nextval` advances and
returns. `setval(seq, n [, is_called])` moves the counter (persisted, not
transactional), which removes the SERIAL-collision trap after a `pg_dump` restore.
`currval` reports the session's last **explicit** `nextval`/`setval(is_called=true)` value
(`55000` if there's none this session; an embedded `Database` is one session for all its
logical connections, `EDGES.md` row 223). A draw made by filling a column `DEFAULT` does
**not** set it, where 18.6's does. After `INSERT INTO t DEFAULT VALUES` (or an INSERT that
omits the column) over a SERIAL column, `currval('t_id_seq')` is `55000` here and the drawn
value there. An explicit `SELECT nextval(…)` then sets it on both, which is the control
showing the problem is the DEFAULT-fill path and not `currval` itself. Loud (`EDGES.md` row
265).

A name no relation holds is `42P01`. A name held by a **table or an index** is `42809`
(`init_sequence`'s `validate_relation_kind`, which all three functions reach; we checked
`nextval`, `currval` and `setval`). Those two fall on **opposite sides of `setval`'s
argument evaluation**, because Postgres resolves `'x'::regclass` during parse analysis and
only reaches `init_sequence` in the function body. `setval('<no such relation>', 1/0)` is
`42P01`, while `setval('<a table>', 1/0)` is `22012` and `setval('<a table>', 3)` is
`42809`. So a single kind check up front would get two of the three wrong.

**`setval` is also STRICT**, which is a fourth position between the arguments and the
body. A NULL value or `is_called` skips the call and returns NULL, so
`setval('<a table>', NULL)` is NULL instead of `42809` and the counter doesn't move. An
erroring argument next to the NULL still wins (`setval(s, 1/0, NULL)` is `22012`), since a
STRICT function evaluates all its arguments before the call is skipped.

Strictness only applies to a call that **overload resolution** accepted, which is a
*fifth* position, ahead of all four. Postgres registers `setval(regclass, bigint)` and
`setval(regclass, bigint, boolean)`, and picks between them from the argument **types**
during parse analysis, before the name's `regclass` coercion. So a NULL isn't always
strict: `setval(s, NULL::bigint)` is NULL, while `setval(s, NULL::text)`,
`setval(s, 5, NULL::int)` and `setval(s, NULL, 1/0)` are all `42883`. So are the non-NULL
`setval(s, 5.0)` (numeric to bigint is assignment-only) and `setval(s, 5, 1)` (integer
isn't implicitly boolean). Reading strictness off the *value* can't see any of that, since
`Value::Null` carries no type. A reader like that would return NULL for three calls
Postgres refuses outright, turning a loud rejection into a silent success. The check reads
the argument's **static** type and treats a bare literal as Postgres's `unknown`
(accepted, then coerced), which is what keeps `setval(s, '5')` out of it.

A **sixth** position is the name's own `SplitIdentifierString` reading: `42602 invalid
name syntax`, for `'s x'` or an unterminated `"`. That's the one pglet puts in the wrong
place, so it's recorded here rather than presented as part of a clean ordering. Postgres
reaches it at the name argument's `regclass` **coercion** (after overload resolution,
before argument evaluation) and, on the DDL path, inside `AddRelationNewConstraints` once
the heap exists. pglet raises it in the **parser** (for a column `DEFAULT`) and in the
**planner** (for an expression), which puts it ahead of everything. Checked against 18.6:
`setval('s x', NULL::text)` is `42883` there and `42602` here. On `CREATE TABLE`, a
malformed name wins over the already-existing table (`42P07`), the unknown column type
(`42704`) and the second primary key (`42P16`). It even wins over a Postgres **success**:
`CREATE TABLE IF NOT EXISTS <an existing table> (a INT DEFAULT nextval('s x'))` is a
NOTICE and a skip there and `42602` here. That's the *wrong kind wins over a skip* pattern
again, in the direction where pglet errors and Postgres succeeds.

One case already agrees, and it limits the misplacement rather than illustrating it: a
duplicate CHECK name in the same statement is `42602` on **both** (Postgres cooks defaults
before constraints). `ALTER TABLE IF EXISTS <no such table> ADD COLUMN b INT DEFAULT
nextval('s x')` is one more case of the misplacement: 18.6 skips it with a `NOTICE`, and
pglet's parser gives `42602` before the executor's skip is reached (`EDGES.md` row 15).

**The other two codes this reader raises are misplaced the same way**, which is why the
check isn't moved. The same statements with a name malformed another way behave the same:
`setval('noschema.sq', NULL::text)` is `42883` there and `42P01` here, and
`nextval('a.b.c')` is `0A000` there (`cross-database references are not implemented`) and
`42601` here. So it's a property of pglet's name reader across three SQLSTATEs, not of
`42602` alone. Fixing it means moving name resolution later on both the DDL and the
expression path, which is a precedence change at two scales (`docs/ddl.md` § Which error
wins; `EDGES.md` row 267).

Sequence calls are handled at the statement level (a top-level `SELECT nextval(…)` /
`INSERT … VALUES (nextval(…))`), which keeps pglet's eval read-only. A sequence function in
a nested position (WHERE, subquery, `… FROM t`) **fails loudly** (PG would advance it;
`EDGES.md` row 266).

A sequence is a **catalog record** (`{last_value, start, owner}`), not just a counter.
`START WITH` persists, so a bare `ALTER SEQUENCE … RESTART` can return to it, and the
`OWNED BY` column is recorded (by an explicit clause and by every implicit SERIAL/identity
sequence) and dumped. A sequence is also **in the relation namespace**. `pg_class` holds
tables, indexes and sequences in one pool, so `CREATE SEQUENCE ns1; CREATE TABLE ns1 (…)`
is `42P07` in either order, `IF NOT EXISTS`/`CREATE INDEX IF NOT EXISTS` skip over it, and
a duplicate `CREATE SEQUENCE` is `42P07`. It isn't a silent success that keeps the old
counter and drops the new `START WITH`.

The **implicit** `<table>_<column>_seq` is named at execute time against that live
namespace and **disambiguated** like every other name the engine picks (`t_a_seq`,
`t_a_seq1`, …). It's pushed aside by a table or an index with that name, *not* by a CHECK
constraint with that name (`ChooseRelationName(…, isconstraint = false)`), and it's
claimed **before** the table's constraint indexes, so
`CREATE TABLE b1 (a SERIAL, CONSTRAINT b1_a_seq PRIMARY KEY (a))` is `42P07`.
`CREATE SEQUENCE` also honours `RESTART [[WITH] n]` (`SeqOptElem` allows it there too). It
moves the **counter** and leaves the declared start alone, so
`START WITH 10 RESTART WITH 5` hands out `5`, and a later bare `RESTART` returns to `10`.

**`ALTER SEQUENCE` acts on three options.** `START [WITH] n` moves the recorded start.
`RESTART [[WITH] n]` moves the counter (bare, it goes back to the start, and a
`START WITH` in the same statement applies first whatever the written order).
`OWNED BY {t.c | NONE}` sets or clears the owner, **except on an identity column's own
sequence, which refuses every `OWNED BY`** (`0A000`, `process_owned_by`'s
`sequenceIsOwned(seq, DEPENDENCY_INTERNAL, …)`). `NONE`, another table's column, a plain
sibling column, and the identity column *itself* are all refused. The refusal comes
**after** the owner is resolved, so a missing owner table (`42P01`), a missing owner
column (`42703`) and an owner of the wrong kind (`42809`) each win over it.

That refusal is what keeps the identity link from being separated, and pglet needs it more
than Postgres does. PG stores no `pg_attrdef` row for an identity column at all, while
pglet models the link as an ordinary `NextVal` default. So detaching the edge would leave
the sequence as nobody's dependent. An ordinary `DROP SEQUENCE` (or a
`DROP TABLE`/`DROP COLUMN` of whatever the edge was moved to) would then remove it, and the
column's default would name nothing. Every write would be `42P01`, and a dump would emit
`DEFAULT nextval(…)` with no `CREATE SEQUENCE`, which stock Postgres refuses and pglet's
own restore silently renumbers from 1.

A RESTART leaves `currval` alone and **is transactional** (`Plan::swapped_sequences`,
which records the counter the cell held before it), as is `TRUNCATE … RESTART IDENTITY`:
the counter reset is undone by its own `ROLLBACK`. The target is resolved even for a
tolerated tail (`OWNER TO`, `RENAME TO`, `CACHE`, …), in `sequence.c`'s order: wrong kind
(`42809`) **before** the `IF EXISTS` skip, so `ALTER SEQUENCE IF EXISTS <a table>` still
errors, then `42P01`, and both before the `OWNED BY` column's `42703`. On
`CREATE SEQUENCE` the same three are reversed: the `IF NOT EXISTS` skip first (a bad
`OWNED BY` is never resolved), then `42P07`, then the owner.

**`DROP SEQUENCE [IF EXISTS] <n> [CASCADE|RESTRICT]`** parses and removes the record, in
`RemoveRelations`' order. The wrong kind is `42809` **before** the `IF EXISTS` skip (a
table or an index named here errors either way), and only a name no relation holds reaches
`42P01` or the skip. Two dependency edges then decide it, and they're two different rules:

- An **identity** column's sequence is its INTERNAL implementation, and is refused with
  `2BP01` *with* `CASCADE` as well as without (`findDependentObjects` raises it at the
  outermost recursion level without testing `DropBehavior`).
- A plain column `DEFAULT nextval('s')`, which is what a `SERIAL` has, is the NORMAL
  `pg_attrdef` edge: `2BP01` unless `CASCADE`, which drops **that column's default** and
  keeps the column, its `NOT NULL` and its rows.

When both are present, the INTERNAL one wins. An owned sequence whose column has no default
has neither edge and drops cleanly. Being `OWNED BY` an identity column **isn't** the same
thing as being its sequence: an `ALTER SEQUENCE … OWNED BY` only records the AUTO edge, so
such a sequence drops normally on both engines. An **`OWNED BY` owner** is checked on its
own terms, not the target's. An index or a sequence owner is `42809`, with
`process_owned_by`'s own wording (`sequence cannot be owned by relation "x"`, not
`cannot open relation`), before the owner column's `42703`.

A user's `DEFAULT nextval('x')` naming a table or an index is `42809` at **DDL time** on
all three registration routes (`CREATE TABLE`, `ADD COLUMN`, `SET DEFAULT`). That's
deliberately stricter than Postgres, which resolves `'x'::regclass` during parse analysis
and leaves the kind check until the first read. pglet has no regclass and auto-creates the
sequence, so accepting it would put one name in `tables` and `sequences` at once and dump
to a script with both a `CREATE SEQUENCE` and a `CREATE TABLE` under that name. That
includes the table the `CREATE TABLE` is **itself** creating
(`CREATE TABLE e3 (b INT DEFAULT nextval('e3'), c INT)`), which a check that only reads the
catalog can't see. Postgres cooks the default once the heap exists, so `'e3'::regclass`
binds the table there and every write is `42809`.

Because the check replaces an error Postgres raises at the **first write**, it's the
*last* thing the statement tests, and everything Postgres rejects at DDL time wins over it.
On `CREATE TABLE` that's the column type (`42704`), the key columns (`42703`) and a
repeated one among them (`42701`), the multiplicity (`42P16`), a colliding declared
`SEQUENCE NAME` on any column (`42P07`), the relation's column-name set (a duplicate and a
system-owned name, each `42701`), the table's own `42P07`, and then the constraint naming
block: a duplicate CHECK name (`42710`), an ill-typed CHECK (`42883`), a taken
constraint-index name (`42P07`), a missing FK parent (`42P01`) and a repeated column in an
FK's *referenced* list (`42830`). On `ADD COLUMN` it's the same block, before the backfill.
We checked all of this on 18.6 from both sides. With the named relation present (so
Postgres's own cook step succeeds), each of those errors is what Postgres gives. With it
*missing*, the `42P01` the cook raises instead loses to the first group and **wins** over
the naming block. So 18.6's own cook error marks the boundary between the two groups
instead of sitting in this check's last slot. Checking inside the column loop instead would
put pglet's over-rejection ahead of real Postgres errors it has to lose to.

The name a `DEFAULT nextval('x')` will register is also **claimed** in the in-flight
relation namespace at that point, so the statement's own constraint indexes pick names
around it (`t1_pkey` taken by the sequence gives `t1_pkey1`), exactly as they do in
Postgres when the sequence already exists. Without the claim, the index would be named on
top of it and the collision only found after the catalog writes began, which a failing DDL
statement must never reach (`docs/ddl.md` § A failed DDL statement must leave the catalog
untouched; § A gate that reads the catalog is blind inside the statement that is building
it). A sequence that one of this statement's own SERIAL/identity columns claimed is **not**
re-registered by a sibling `DEFAULT nextval('<that name>')` (which both engines accept), so
the column's `START WITH` and its ownership edge survive.

A **declared** `SEQUENCE NAME <n>` on an identity column is used as written at all three
implicit-sequence routes, and refused (`42P07`) when the relation namespace already holds
it. That's the declared half of the naming rule. pg_dump emits that option, so dropping it
would silently restart a restored identity counter at 1 whenever the dumped name isn't the
default `<t>_<c>_seq`. It loses to `IF NOT EXISTS`, to the column type (`42704`) and to the
key columns (`42703`), and wins over the table's own `42P07` and a duplicate CHECK name (all
checked). A declared name equal to the **table being created** is the one case that doesn't
follow that list, because the collision isn't the sequence's. The steps before the table
create the sequence, then `heap_create_with_catalog` finds the name taken, so
`CREATE TABLE a1 (id INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME a1))` is `42P07`
against the *table*. That's true under `IF NOT EXISTS` too, whose skip only fires when the
table itself already exists, and a *later* column's collision with a name that's already
taken still wins over it. The checks read the catalog, which doesn't hold the table under
construction, so this one is decided by testing the statement's in-flight names at the
table's own `42P07`. The auto-name only reads the **committed catalog**, never the names
the same statement has claimed, so a declared name that collides with a sibling SERIAL's
auto-name is `42P07` in either written order instead of being disambiguated.

**Divergences:** a sequence isn't a readable relation. Postgres answers
`SELECT last_value FROM s` with a one-row `(last_value, log_cnt, is_called)`, and pglet
raises `42809` naming the reason. Every *write* shape over a sequence is `42809` on both
engines: INSERT/UPDATE/DELETE, `COPY` in either direction, `TRUNCATE`, `DROP TABLE`, every
`ALTER TABLE` action, `CREATE INDEX … ON s`, `REFERENCES s`, and `DROP INDEX s`
(`"s" is not an index`, its own wording). The kind is decided on a name that **resolves**,
so it wins over a *skip* as well as an error. `DROP TABLE IF EXISTS s`,
`DROP INDEX IF EXISTS s` and `CREATE INDEX IF NOT EXISTS <a taken name> ON s` all error
instead of reporting a no-op success. Those are the cases where this class would be silent,
since the skip makes the statement *succeed*.

**The whole class is one rule over the whole relation namespace, not a sequence rule.**
`pg_class` pools tables, indexes and sequences, so every ordered pair of kinds is reachable
and each is `42809` on 18.6: `DROP TABLE [IF EXISTS] <an index>`,
`DROP INDEX [IF EXISTS] <a table>`, `TRUNCATE`/`SELECT`/`INSERT`/`UPDATE`/`DELETE`/`COPY`/
`ALTER TABLE`/`CREATE INDEX … ON`/`REFERENCES` over an index, and the sequence-side
equivalents. `Schema::relation_kind` is the single enumeration and `Error::WrongObjectType`
the single error, so the two directions can't drift apart. A check that only read
`schema.sequences` would leave the two `IF EXISTS` spellings *silently succeeding*, and the
index-side cases reporting `42P01`/`42704`.

These are **not** covered, each for its own reason. `COMMENT ON …` (any target, any kind)
parses as a tolerated no-op, so nothing reaches the kind check (see `EDGES.md`, the
tolerated-no-op family). A bad *column list* over a sequence is `42809` where PG says
`42703`, because PG resolves the sequence's own `(last_value, log_cnt, is_called)`
descriptor during parse analysis and pglet doesn't model that relation. That's the
read-side gap seen from the write side. The read-side model is also the class's remaining
gap in the other direction. `pg_class` hardcodes `relkind` `'r'` and lists **tables only**,
so a sequence or an index is missing from it, while `CREATE TABLE <that name>` gives
`42P07`. That's the same one-name-two-answers contradiction the `42809` check keeps off the
write side, showing up on the introspection surface (`pg_indexes` does carry indexes).

More divergences:

- pglet doesn't model `MINVALUE`/`MAXVALUE`, so `START WITH 0`, `setval(s, 0)` and
  `RESTART WITH 0` are all accepted where PG raises `22003`/`22023`. Every draw is `+1`;
  `INCREMENT BY`, `MINVALUE` / `MAXVALUE` and `CYCLE` are parsed and ignored (`EDGES.md`
  row 215).
- `ALTER SEQUENCE … RENAME TO` is accepted and does nothing.
- `lastval()` isn't implemented (`0A000`).
- A **NULL sequence name** (`nextval(NULL)`, `currval(NULL)`, `setval(NULL, 5)`, all NULL in
  Postgres because they're strict) is `0A000` here, because the parser takes the name as a
  string literal, not an expression.
- `setval(s, '5')` is `42804` where PG coerces the unknown literal and returns `5`. That's
  the literal-as-text gap, here at a function argument; `setval(s, 'notanumber')` is the
  same `42804` where PG says `22P02`. The gap is the same on the **third** argument, which
  the overload check also lets through as an unknown literal: `setval(s, 5, 't')` is
  `42804` where PG returns `5`, and `setval(s, 5, 'notabool')` is `42804` where PG says
  `22P02`.
- A **`::regclass`** cast on the name is pglet's own
  `0A000 not implemented: nextval() requires a string-literal sequence name` in an
  *expression* (`SELECT nextval('s'::regclass)` gives `1` on PG). `parse_cast_target`
  lowers the cast to a `StringFn`, which `sequence_name_from_arg`'s look-through doesn't
  match, so the refusal comes from the look-through and not the type resolver. The
  spelling pg_dump emits, a column `DEFAULT`, is accepted, because `parse_nextval_arg`
  consumes the cast token without resolving it. `'s'::text`/`'s'::varchar` work in an
  expression too (PG coerces those to `regclass` at run time). The name argument's own cast
  **is** coerced before the expression is discarded, so `nextval('s'::int)` is `22P02` on
  both engines, instead of pglet silently advancing the sequence.

**`GENERATED {ALWAYS|BY DEFAULT} AS IDENTITY`** reuses this machinery: an implicit
sequence + NOT NULL, a `START WITH` seed, and `OVERRIDING SYSTEM|USER VALUE`. `ALWAYS`
rejects an explicit non-`DEFAULT` INSERT/UPDATE/`ON CONFLICT` value with `428C9`. The
pg_dump `ALTER … ADD GENERATED …` form works. Other refusals:

- a non-integer identity type is `22023`;
- `ADD` on a column that's already identity is `55000`, as is `ADD` on a column not
  declared `NOT NULL`, and on a column that has a default (`DROP DEFAULT` first; those are
  the statements that turn a `SERIAL` column into an identity one);
- `… AS (…) STORED` is `0A000`;
- **`ALTER COLUMN … SET DEFAULT` / `DROP DEFAULT` on an identity column is `42601`.**
  That's `ATExecColumnDefault`'s refusal, the other half of the `OWNED BY` one above. It
  wins over the default's own sequence resolution (`nextval('<no such sequence>')`,
  `nextval('<a table>')`, so it also wins over pglet's own DDL-time `42809`), and *among
  the executor's checks* it loses to the table's `42P01` and the column's `42703`.

**Some spellings never reach any executor check at all**, so the precedence has to be
stated in terms of what the front end does with the statement, not in terms of the
checks. An **unrecognized function** in the DEFAULT is refused by the planner's own
lowering (`0A000` for `nosuchfn()`, where 18.6 gives `42601`), under
`ALTER TABLE IF EXISTS` too. `ONLY`, the `COLUMN`-less `ALTER id`, `IF EXISTS`, `-1`,
`CURRENT_DATE`, `(1+1)`, `$1` and the pg_dump `nextval('x'::regclass)` spelling all do
reach the check and give `42601` on both engines. The last two get there through the
`ColumnDefault::Expr` catch-all, which the classifier's recognized forms fall through to.
(A bare `$N` on a **non**-identity column is `42P02` on both, the control showing it's the
identity check and not the parameter reader.) The identity flag is persisted
(`Column.identity`).

**An `ALTER TABLE … ADD COLUMN` with a `SERIAL` / identity / `DEFAULT nextval('x')`
stages its sequence behind the backfill validation.** So a statement that then fails
(`… CHECK (c > 100)` over two existing rows, `23514`) leaves no sequence and a free name,
as 18.6 does. The record is *staged* (a local `Sequence` the backfill draws from) and only
inserted inside the mutation block. The route needs that shape, not just a reordering,
because the backfill has to draw from the sequence while the statement can still be
refused. A sequence the statement did **not** create is still advanced in place, since
`nextval` isn't rolled back and walking a live counter back would re-issue ids that were
already handed out. An added `SERIAL` column is declared `NOT NULL`, like a
`CREATE TABLE`'s and as on 18.6, with each row in the table having drawn its value
(`ddl/add_column_serial_*`).

**Divergence, measured and left as is:** how *far* it advances, and **only for the
row-level constraint checks**. 18.6 applies those as it rewrites row by row, stopping at
the offending row, while pglet resolves the whole backfill and then validates it, so pglet
draws once per existing row. Over four rows, a `CHECK` failing on row 1 leaves 18.6's next
`nextval` at `2` and pglet's at `5`, and failing on row 3 leaves 18.6's at `4` and pglet's
at `5`. (The `NOT NULL` member of that family can't be reached over a sequence-bearing
default, which resolves to a non-NULL `int8`.) The **FK and unique** routes agree, because
18.6 validates those after the whole rewrite: a `REFERENCES` whose first violation is row
3, and a `NUMERIC(3,-1) … UNIQUE` whose backfill collides, each leave both engines' next
`nextval` at `5`. Neither engine reports the difference, and it's a gap in a counter that never promised to
be gapless. No fixture pins it, because a faithful one would fail. `CREATE SEQUENCE` rolls
back inside a transaction: a block that wrote restores its catalog unconditionally.

### `DEFAULT` functions (`now()`, `gen_random_uuid()`)

Literals, `now()` and `gen_random_uuid()`. The impure pair reads the OS clock/RNG natively
and the JS host on wasm. A wasm clock read falls inside the millisecond `Date.now()`
reports at that read: it's a microsecond past the previous read for the first five hundred
reads of one millisecond, and equal to it after that, so a value stored at precision 3 or
finer stays in that millisecond. A host read that returns `NaN` is refused with `22000`
(`docs/temporal.md` § The transaction and statement clocks).

**The five `now`-family spellings are five distinct catalog defaults**
(`ColumnDefault::{Now, TransactionTimestamp, StatementTimestamp, ClockTimestamp,
CurrentTimestamp}` over the matching `BuiltinFn` arms), because 18.6 keeps them distinct
in three visible ways:

- `information_schema.columns.column_default` and `pg_dump` render each as written
  (`CURRENT_TIMESTAMP` as the bare keyword, the rest as their own calls).
- The output column name of a bare projection is the spelling
  (`SELECT clock_timestamp()` gives column `clock_timestamp`).
- **`clock_timestamp()` alone is VOLATILE**, so an `ALTER TABLE … ADD COLUMN` backfill
  re-reads the clock per row, where the other four take one value. On 18.6, measured on one
  idle host (the count depends on the host's speed), that gave 3-4 distinct instants over 5
  rows and 6-11 over 20 (mostly 7), against a uniform 1 for `now()` /
  `transaction_timestamp()` / `statement_timestamp()` / `CURRENT_TIMESTAMP`. A *constant*
  default isn't written into the rows at all but into `pg_attribute.attmissingval`, which
  pglet doesn't model. That's fully **visible** on 18.6, where
  `SELECT atthasmissing, attmissingval FROM pg_attribute` gives `t | {42}` for a
  `DEFAULT 42` add-column. What makes it unreachable *here* is pglet's own catalog, whose
  `pg_attribute` has neither of those two columns.

Using one variant for all five would lose all three at once: the deparse, the column name,
and the backfill (one instant where 18.6 writes many). Pinned by
`ddl/default_now_family_spellings.sql`,
`ddl/add_column_clock_timestamp_backfills_per_row.sql`,
`select/now_family_output_column_names.sql`, `executor::ddl::backfill_volatility_tests`
and `dump::column_default_round_trip`.

**Each one reads the same clock as in 18.6.** `now()`, `transaction_timestamp()` and
`CURRENT_TIMESTAMP` give the transaction's start (an open block's, or the statement's
own). `statement_timestamp()` gives the statement's start, and `clock_timestamp()` the wall
clock at the call. So a multi-row `INSERT` under `DEFAULT now()` writes one instant, and
`now()` doesn't change inside one `BEGIN` (`docs/temporal.md` § The transaction and statement
clocks; the `**/clock_*` fixtures, and the embed, server, wasm and npm clock tests for the
boundaries a fixture can't drive). The protocol residual is on the extended protocol:
outside a block, a bound statement's `now()` equals its `statement_timestamp()` where
18.6's is earlier, and each statement of a pipeline has its own `now()` (`EDGES.md` row
292). On wasm, reads within one millisecond tie after the five-hundredth
(`docs/temporal.md`). **The variants' postcard tags are pinned** (`catalog::schema`'s
`column_default_postcard_variant_tags_are_pinned` and `planner::expr`'s
`builtin_fn_postcard_variant_tags_are_pinned`), so a variant is added by appending it, and
no stored tag changes meaning.

### `starts_with(a, b)`

A case-sensitive literal prefix test (not LIKE wildcards). Postgres has exactly one
signature, `starts_with(text, text)`, so a non-text argument on either side fails loudly
with `42883`. The exception is an enum, whose label is read as `text` and works
(`EDGES.md` row 190).

### `||` string concat

pglet only implements the **text** forms: `text || text` and Postgres's
`text || anynonarray` / `anynonarray || text`. The non-text side is turned into a string,
matching PG when the text side is typed. An untyped side (a bare string literal or an
untyped `$N`) next to an operand whose type has its own `||` stays `text` here, where 18.6
reads it as that type. Next to a `bytea` or a `jsonb`, the two are silently joined as text,
where 18.6 joins the bytes or merges the objects. Next to an array it's `42883` (`0A000`
over a `text[]`) where 18.6 appends (`EDGES.md` row 293). NULL propagates (`a || NULL` is
NULL). A `timestamptz` operand keeps its `+00` offset through the concat, so the instant
doesn't shift when it's turned into a string.

**Gap (loud, not silent):** the three concatenations Postgres has and pglet doesn't
evaluate, `bytea || bytea` (byte concat), `jsonb || jsonb` (merge), and the array forms
(`array_cat`), fail with `0A000`. An operand pair with no `||` signature at all
(`int || int`, `int[] || text`) fails with `42883`. The exception is an enum, which the
rule treats as `text`, so it works next to an enum, `integer`, `numeric`, `boolean` or
`jsonb` side, which 18.6 refuses with `42883` (`m || m` is `aa`, `m || 1` is `a1`;
`EDGES.md` row 190).

### `LIKE … ESCAPE` + default `\` escape

`\` is the default escape (PG-compatible: `\%`/`\_` match literally). An explicit
`ESCAPE '\'` is accepted, and any other escape character fails loudly. Operands are
checked to be string types. A non-text operand (`int_col LIKE 'x'`, `1 LIKE '1'`) fails
with `42883` instead of silently returning `false` (which would make `NOT LIKE` true for
every row). The exception is an enum, whose label is read as `text` and works where 18.6
gives `42883` (`EDGES.md` row 190). **Gap (loud):** `bytea ~~ bytea` / `!~~ bytea` exist in
Postgres (byte-wise) and aren't implemented here (`0A000`). There's no `bytea ~~*`, so
case-insensitive bytea stays `42883`. **Divergences (loud):** `ANY`/`ALL` on the right of
a `LIKE`-family operator (`'a' LIKE ANY(ARRAY['a','b'])`) is refused with `42601` where
18.6 applies it per element (`EDGES.md` row 186), and a chained `LIKE` is `42883` where
18.6's grammar gives `42601` (row 185).

### `ILIKE`

Case-insensitive `LIKE`. Both operands are folded through the same Unicode lowercasing as
`lower()`, so wildcard/`ESCAPE` behavior is the same as `LIKE`, and a bound `$N` pattern is
typed as text. The fold is Rust's `to_lowercase()`, a **full** (1→N), context-sensitive
Unicode case mapping, where Postgres's libc path is the **simple** 1→1 `towlower`. On the
oracle (libc provider, `en_US.UTF-8`, UTF8), `SELECT 'İstanbul' ILIKE 'istanbul'` is `f`
here and `t` there, and `SELECT length(lower('İ'))` is `2` here and `1` there (`EDGES.md`
row 264). Operands are checked exactly like `LIKE`'s: a non-text operand fails with
`42883`, except an enum (row 190). `LIKE`'s `ANY`/`ALL` and chaining divergences apply here
too (`EDGES.md` rows 185, 186).

### Scalar string + misc functions

- `upper`, `length` (character count; byte count for `bytea`), `replace`.
- `concat`/`concat_ws` (skip NULLs instead of propagating them).
- `substring` (the `… FROM n FOR m` special form and the comma form, PG's clipping math,
  `22011` on a negative length).
- `trim`/`ltrim`/`rtrim`/`btrim` (`TRIM([BOTH|LEADING|TRAILING] [chars] FROM x)`, matching
  against a character **set**).
- `position`/`strpos` (`POSITION(a IN b)`, 1-based, arguments swapped).
- `lpad`/`rpad` (character-based, `54000` on an oversized length).
- `split_part` (a negative index counts from the end, `22023` on 0).
- `substr` (`substring`'s evaluation under its own name; both take a `bytea` and return
  `bytea`, counted in bytes).
- `left`/`right` (characters; a negative count drops `|n|` from the other end;
  `right(s, -2147483648)` returns the whole string, as the 18.6 oracle does).
- `octet_length` (`int4` bytes of a `text` or `bytea`).
- `md5` (`text` or `bytea`, an in-house RFC 1321 digest as lower-case hex).

`left`, `right` and `substr` take their counts as `int4`: an `int2` widens, an `int8` or
`numeric` is `42883`, and a string literal is read as `int4` (`22P02` if it can't be).
`substring` still accepts an `int8` there, which 18.6 refuses. `left` and `right` are
TYPE_FUNC_NAME keywords and parse as calls before a `(` (§ Keyword as identifier). Over a
`char(n)` value, `octet_length` counts the unpadded string, since `char(n)` is stored as
`varchar` (`EDGES.md` row 95).

`random()` is a `double precision` in `[0, 1)`, drawn fresh at each evaluation: per row,
per row of an `ADD COLUMN` backfill, per row an `unnest(…)` next to it emits, and when the
GIN selector declines it for a guard. It comes from the OS RNG natively and from
`Math.random()` in the wasm build. A read-only `WITH` query that doesn't reference itself
and whose body calls it is evaluated at each reference, and an uncorrelated subquery over
it (`(SELECT random())`) is evaluated at each row of the query around it, where 18.6 draws
once for each (`EDGES.md` rows 316 and 319). `setseed` isn't built, so a sequence of draws
can't be repeated, and `random(min, max)` is `42883`. An index over `random()` is `42P17`,
as on 18.6.

`greatest`/`least` skip NULLs and unify with `select_common_type`, so the wire type label
matches the returned value (`42804`/`22P02` on incompatible arguments). **`nullif` follows
a different rulebook.** Postgres resolves the `=` operator and declares *argument one's*
type as the resolution left it (`parse_expr.c`: `opresulttype =
exprType(linitial(args))`), so `NULLIF(int4, int8)` is `integer` where `GREATEST` of the
same pair is `bigint`. Argument one keeps its type whenever a registered `=` takes it on
the left (the integer family compares across widths, a float accepts any numeric, and
`varchar` has no `=` of its own so it goes to `text`), and widens when none does
(`NULLIF(int4, real)` is `double precision`). The returned value is argument one at that
same type, so the declared and returned types agree for those pairs. They differ when the
plan only types the first argument, because the value's type is recomputed from the two
values. So through the npm package, `nullif(n, $1)` and `nullif(n, n * $1)` over a
`numeric` or `bigint` `n`, bound to a fractional JS number, return `n` as a
`double precision` under its own declared type (`EDGES.md` row 288).

Two consequences that look wrong but aren't: `NULLIF(varchar_col, varchar_col)` is
**`text`** (varchar has no `=` of its own, even against another varchar), and
`NULLIF(time_col, interval_col)` is `interval`. That's the one implicit cast that crosses a
type-category line, which the operator rulebook bridges and `select_common_type` doesn't
(`COALESCE(time, interval)` is `42804`). An operand pair with no `=` at all is **`42883`**,
not the `42846` the common-type constructs report for the same pair.

All indexing is 1-based, and all lengths and positions are character-based. A non-text
argument (except an enum, whose label is read as `text` and works; `EDGES.md` row 190) or
the wrong arity fails loudly: `42883` through `string_call`/`variadic_call`
(`upper('a','b')`), but `0A000` through `single_arg`/`binary_call` (`lower('a','b')`,
`starts_with('a')`) where 18.6 is always `42883` (the *Wrong-SQLSTATE-class* row). The
special forms are `ColName` keywords, guarded by the parenthesis, so a bare
`substring`/`trim`/`position` can still be used as a column name.

**Divergence:** `upper`/`lower` use Rust's full, context-sensitive Unicode case mapping.
That expands special-casing characters where PG's locale/simple mapping doesn't
(`upper('straße')` gives `STRASSE` in pglet and `STRAßE` in PG; `length(upper('ß'))` is
`2` here and `1` there), and lowercases a word-final capital sigma to the final sigma where
PG gives the plain one (`EDGES.md` row 264). This is an accepted divergence, like
`ILIKE`'s.

**What isn't implemented fails loudly**, and that set is **open-ended, not a fixed list**.
The `0A000 function not supported: <name>` arm is the *default* arm of `planner::expr`'s
`build_call` name match, so every name the dispatch table doesn't hold reaches it,
including a name that doesn't exist anywhere (`nosuchfn(1)` is `0A000` here against 18.6's
`42883`, the *Wrong-SQLSTATE-class* row). What we can list is the ones we checked:
`initcap`, `reverse`, `translate`, `repeat`, `ascii`, `chr`, `to_hex`, `array_position`,
`format`, `age`, `quote_literal` and the numeric `to_char(numeric, text)` all give `0A000`
(the `to_char(timestamp, text)` overload the spine issues is implemented). **`overlay` is
both**: the ordinary call `overlay('abc', 'x', 2)` is `0A000` like any other unknown name,
while only the SQL-standard `overlay('abc' PLACING 'x' FROM 2)` form gives the parser's
`42601`. None of them is silent, and none is used by a spine consumer.

### `date_trunc` / `extract` / `date_part`

`date_trunc(field, source)` works over timestamp/timestamptz/interval (a `date` casts to
timestamptz, result `+00`). It zeroes every finer field, `week` goes to the ISO Monday,
quarter to its first month, and century/millennium have no year 0. `EXTRACT(field FROM
source)` (the special form) and the plain calls `extract(text, src)` / `date_part(text,
src)` work over timestamp/timestamptz/date/time/interval. **`extract` returns `numeric`
with PG's per-field display scale** (second/epoch get scale 6, ms 3, integer fields 0, and
julian a 20-digit fraction); **`date_part` returns `float8`**. Supported fields include
dow (0=Sun), isodow (1=Mon), doy, isoyear and epoch (an interval's epoch uses 365.25-day
years plus 30-day months for the remainder). The two-tier field split matches PG: an
EXTRACT-only field like `dow`/`epoch` on `date_trunc` is `22023`, and a recognized field
that's wrong for the type is `0A000`. All differential-tested.

**Divergences:**

- `extract(timezone*/timezone_hour/timezone_minute FROM timestamptz)` is `0A000` where PG
  gives `0`. pglet stores timestamptz as a plain `Value::Timestamp`, so eval can't tell
  them apart (the offset is always 0 under UTC anyway).
- `date_trunc(field, time)` is `42883` (there's no implicit `time → interval` cast).
- A truncation result before year 1 (`decade` over years 1–9) is
  `22008 timestamp out of range` where PG gives `0001-01-01 00:00:00 BC` (the date domain,
  `EDGES.md` row 142).
- The `EXTRACT(field FROM …)` **field grammar is too wide.** `parse_extract`'s
  `is_field_word` accepts `TokenKind::Keyword(_)`, so any reserved word directly before
  `FROM` is taken as the field and resolved at eval. PG's `extract_arg` production is
  `IDENT | YEAR_P | … | Sconst` and refuses it in bison. `EXTRACT(ALL FROM now())`, its
  `DISTINCT` spelling, `EXTRACT(SELECT FROM now())` and `EXTRACT(TABLE FROM now())` are
  `42601` there against `22023 unit "…" not recognized` here. Both error, and *only* the
  word directly before `FROM` is affected: `EXTRACT(ALL year FROM now())` is `42601` on
  both, from the comma form's own expression parse.

**An untyped `date_trunc` source is also over-accepted: silently in one spelling, and with
the wrong error code in the others.** 18.6 registers three two-argument `date_trunc`s (over
`timestamp`, `timestamptz` and `interval`), so a source it can't type matches none of them.
`SELECT date_trunc('day', NULL)` is `42725 function date_trunc(unknown, unknown) is not
unique` there and **returns a result** here: one NULL row declaring `timestamp` (1114) on
**both** wire legs. That's `date_trunc_result_type`'s default when there's no source, not
a walker split (the two legs agree). `date_trunc(NULL, NULL)` and
`date_trunc('day', COALESCE(NULL, NULL))` are silent cases too (1114 here; `42725` and
`42883 function date_trunc(unknown, text) does not exist` there).

Two nearby spellings **error with the wrong code** instead, both for the same reason: a
bare literal is typed as `text` here instead of staying Postgres's `unknown` (the general
literal-typing gap). A string-literal source (`date_trunc('day', '2026-05-04')`) and a
**non-NULL** `$1` bound as unknown are each
`42883 function date_trunc does not exist for source type text` here against that same
`42725` there, while a `$1` bound to **NULL** goes back to the silent side at 1114. The
control is a typed source, which agrees everywhere:
`date_trunc('day', CAST(NULL AS TIMESTAMPTZ))` is 1184 and the `TIMESTAMP` spelling 1114
on both engines. It's the same class as the NUMERIC row's `trunc(NULL)` (pglet doesn't
model an overload-ambiguity check), and no fixture can hold the silent case, since pglet
returns a result where 18.6 errors. `executor::walker_pins`'s cell-M test pins the 1114
result, and says it pins the result, not its correctness.

### `SELECT DISTINCT` / `DISTINCT ON`

`DISTINCT` dedups whole rows, treating NULLs as equal (two NULLs aren't distinct) and float
`NaN`s as equal. pglet gives float `NaN` a **total order** in the shared comparators,
matching Postgres: `NaN = NaN` is true and `NaN` sorts greatest, consistently across `=`,
`<`/`>`, `ORDER BY`, `MIN`/`MAX`, `DISTINCT`, window partitioning, joins, and FK/conflict
matchers, and across float widths (`1.5::float8 = 1.5::float4`).

**`DISTINCT ON (…)`** is a real `Plan::DistinctOn`. It keeps the first row per ON-key group
in `ORDER BY` order (the input is sorted by the merged effective key, then adjacent rows
are deduplicated). It enforces Postgres's rule that the ON expressions match the leftmost
`ORDER BY` (a different sort key before an unmatched ON expression is **`42P10`**), and an
unmatched ON expression is appended ASC. ON keys accept a positional `(1)`, an output
alias, and a qualified or bare column (all mapped to the same output column). The
latest-row-per-group idiom
(`DISTINCT ON (a.id) a.id, b.note … ORDER BY a.id, b.created_at DESC`) works: the ON key
comes from the projection while the trailing sort key is read from the row before
projection.

`SELECT DISTINCT` also enforces Postgres's rule that every `ORDER BY` expression appears
in the select list (**`42P10`**), matched by expression equivalence. So `ORDER BY b+a`
against a projected `a+b` is rejected, as in Postgres. An ON key referencing a column the
projection **drops** (`DISTINCT ON (dept) name … ORDER BY dept, salary DESC`) is carried on
the projecting node as a Postgres **resjunk** column, which a `Plan::JunkFilter` trims off
above the dedup. An ON key over an unexpanded bare `*` projection needs no junk column (the
star stands for every column in scope), and one naming nothing is `42703`, on both engines
and whether or not a row matches.

**Divergence:** over a **no-FROM** select, the ON keys and a trailing `ORDER BY` are
lowered by separate functions and not merged, so the match rule isn't enforced there.
`DISTINCT ON (q) unnest(ARRAY[2,1,2]) AS q, 5 AS r ORDER BY r, q` returns rows here
against 18.6's `42P10`. At a level that no expression subquery encloses, a **no-FROM**
select's ON keys bind at plan time against its target list, so a key naming nothing is
`42703` and a bad qualifier is `42P01`, at any row count and before any write.

**Divergence:** over a bare `*` projection, a key naming a column that the level's scope
doesn't **enumerate** is `0A000`. That's one rule, and what falls under it is the kinds of
relation `build_column_scope` registers non-exhaustively (re-derive them from the function,
not from a count). Each of these reaches it:

- a catalog-view column pglet doesn't model
  (`DISTINCT ON (c.udt_name) * FROM information_schema.columns c ORDER BY c.udt_name`,
  rows on 18.6);
- a **relation the catalog doesn't have**, registered the same empty way, so pglet's own
  `42P01` never runs (`SELECT DISTINCT ON (nosuch) * FROM no_such_tbl ORDER BY nosuch` is
  `0A000` against `42P01` there; the non-star spelling is `42P01` here too).

Both are loud. A **FROM-clause `unnest(…)`** isn't one of them. It registers its
*columns*, so a qualified key works
(`SELECT DISTINCT ON (u.u) * FROM unnest(ARRAY[1,2,2]) AS u ORDER BY u.u` returns two rows
on both engines, `select/distinct_on_unnest_qualified_key_over_star.sql`), and it
registers its full column list, so a bogus key is `42703` on both
(`SELECT DISTINCT ON (nosuch) * FROM unnest(ARRAY[1,2,2]) AS u ORDER BY nosuch`).

### `UNION` / `INTERSECT` / `EXCEPT` (+ `ALL`)

A real `Plan::SetOp` node, chainable and parenthesizable (either operand). PG precedence:
**`INTERSECT` binds tighter** than `UNION`/`EXCEPT` (both left-associative). Non-`ALL`
dedups whole rows, treating **NULLs as equal**. `UNION ALL` concatenates.
**`INTERSECT ALL` gives `min(countL,countR)`** and **`EXCEPT ALL` gives
`max(0, countL−countR)`** per row group (Postgres's `nodeSetOp` multiset arithmetic). Of
equal rows spelled differently (`1.0`, `1.00`), an `INTERSECT [ALL]` shows the left
input's, where 18.6 shows the input its plan reads first (`EDGES.md` row 298).

Columns unify per position to a common type: integer widening, numeric promotion,
`text`/`varchar` to `text`, `date`/`timestamp`/`timestamptz` to the widest, and a `name`
side by its own `pg_cast` rows (`name`/`text` folds to `name` when `name` is written first
and to `text` when it's second, `varchar`/`name` to `name` either way, and
`name[]`/`text[]` element-wise to the same pair, each checked against 18.6). Each column
is cast **by position**, so the wire type label matches the value.

An **`unknown`** column (a bare string constant, a bare `NULL` or an undeclared `$N`)
contributes no type and takes the other side's, or `text` when both sides are unknown. A
constant is converted as the node is built, so a refused one is `22P02` before execution
(`SELECT 1 UNION SELECT 'x'`), the same rule a FROM-clause `VALUES` list applies per
column. A sort, grouping, `DISTINCT` or window key that names the entry (by position, by
name, or by being the same expression) resolves it to `text` first, as on 18.6.

A side that projects an **enclosing** query's column takes that column's type. Built inside
a subquery, the node can't see those levels, so the column is left pending. The correlated
re-walk unifies it against the level-chained scope and records the result, and the
statement is **parsed a second time** with those types, so nothing reading the plan that
parse returns gets a provisional type (`docs/scope.md`). Result column **names come from
the left**.

A column-count mismatch fails loudly (`42601`), and an incompatible type is `42804`. A
trailing `ORDER BY`/`LIMIT` binds to the combined result. **Positional `ORDER BY <n>`**
(1-based; out of range is `42P10`, `<clause> position <n> is not in select list`, matching
PG) works on a set-op result (name references too; permissive on expression keys).

**Divergences:**

- A `WITH RECURSIVE` term with a self-reference is typed by its own check, not by this
  unification, so
  `WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT '2' FROM r WHERE false) …` is `42804`
  here where 18.6 returns `integer`.
- A column waiting on an enclosing level raises its error after the enclosing level's
  clauses instead of before them, and where the **first** parse's reader raises first, that
  error is the statement's.
  `SELECT count(*) FROM t WHERE EXISTS (SELECT 1 FROM (SELECT n EXCEPT SELECT 1) s WHERE n = '1.25')`
  over a `numeric` `n` is `22P02` here against 18.6's row.
- A FROM-clause `VALUES` list inside a subquery types an enclosing column as `text`.
  `SELECT (SELECT x FROM (VALUES (i), ('7')) s(x) LIMIT 1) FROM t` declares `text` where
  18.6 declares `integer`, and a predicate over it can delete a row 18.6 keeps (`EDGES.md`
  row 151).

### `CASE WHEN … END`

Searched (`WHEN <bool>`) and simple (`CASE <operand> WHEN <value>`) forms; a missing ELSE
gives NULL. The **searched** form's condition is one of the boolean contexts in the row
below, so a non-boolean `WHEN` is `42804 argument of CASE/WHEN` instead of silently taking
the `ELSE`, and an unresolved literal is converted (`CASE WHEN 't' …` takes the arm). The
**simple** form's label isn't a boolean context: 18.6 expands it to `<operand> = <value>`
and coerces the comparison's result.

### Boolean contexts (`WHERE`, `HAVING`, `JOIN … ON`, searched `CASE WHEN`, `AND`/`OR`/`NOT`, `CHECK`, a partial index's `WHERE`, `IS [NOT] TRUE|FALSE|UNKNOWN`)

Each one resolves its operand to `boolean` where the operand is read, through one
plan-time check over `assignment_compatible(x, boolean)` and one eval-time resolver. Only
`boolean` and an unresolved literal resolve (`pg_cast`'s two rows into `boolean` are both
explicit-context), so every other type is `42804 argument of <CTX> must be type boolean,
not type <T>`. An unresolved literal is **converted** through a `boolin` that accepts
prefixes (`'tr'`, `'fals'`, `'ye'`, `'of'` are values; `'o'` and `'01'` are `22P02`).

The four connective contexts are checked **per argument**, interleaved with the walk. A
`$N` at a clause root, or at a node position that `Expr::for_each_child_ctx` marks, is
inferred as `boolean`, below an explicit cast's priority. An `IS [NOT] TRUE` operand is
neither, and stays OID 0 (`docs/param_binding.md`). Where each clause root's check sits
follows that statement's `transformStmt` order, and the clauses differ: `JOIN/ON` wins
over the target list, `HAVING` doesn't, and a partial index's predicate wins over its own
key columns and its access method. The `ON CONFLICT` **arbiter**'s inference `WHERE`
**isn't** one of these contexts, since 18.6 transforms it with no boolean coercion.

**Residuals:**

- A predicate persisted by a build without this check is decoded from the store and isn't
  re-checked, so such a database gives `42804` at every write it guards.
- The action `WHERE`'s check comes before a `VALUES` cell's per-row verdict
  (`docs/names.md`).
- 18.6 stores a stored expression after parse analysis, so its catalog surfaces render
  `CHECK ('t')` as `CHECK (true)`, `check_clause` as `true`, and a constant-true index
  predicate not at all, where pglet renders each as written. That's a divergence in the
  stored-expression rendering, not in this check (`docs/deparse.md`).

`AND`/`OR` evaluate left to right and stop at a deciding left operand, as 18.6 does
(`a = 99 AND a/(a-1) = 1` over `a = 1` is `f` on both). So an operand that pglet only
refuses at evaluation isn't refused when the left side decides (`EDGES.md` row 60's
silencers; row 172 for the residuals, each measured). See § *A boolean context is that
same conversion* in `docs/coercion.md`.

### `FOR UPDATE` / `FOR SHARE`

The `FOR {UPDATE|NO KEY UPDATE|SHARE|KEY SHARE} [OF table…]` grammar is parsed, and so is
`gram.y`'s other production, `FOR READ ONLY`. Since that's an *alternative* to the item
list and not an item, it's only accepted as the clause's **first and only** production:
`FOR UPDATE FOR READ ONLY` and `FOR READ ONLY FOR UPDATE` are both `42601`, on both
engines.

Inside a block, a locking read takes the writer (`Plan::LockRows`), so on the native
server a second block's `FOR UPDATE` waits for the first to end. That's coarser than 18.6's
row lock, with the same mutual exclusion (the embedded `Database` refuses the second
block's `BEGIN` with `55P03` while the first is open). An autocommit locking read takes
nothing on the native server's B+tree, where 18.6 waits on a row another block holds. On
`--storage memory`, which has no lock-free read path, it takes the writer for its
statement and waits for a block that has written, even where 18.6 wouldn't. The embedded
`Database` takes the writer for it, and refuses it with `55P03` while another connection's
block holds the writer (`docs/storage.md` § Isolation Guarantees).

**The wait policy is the exception, and is refused** (`0A000`). `SKIP LOCKED` and `NOWAIT`
each describe a behavior that needs a row lock to mean anything, and treating them as
no-ops would match neither. A locking read inside a block takes the writer
(`Plan::takes_writer`), so on the native server a `NOWAIT` inside a second block would wait
for the first block (`55P03` only once `lock_timeout` passes) where 18.6 gives `55P03` at
once. An autocommit one would take nothing on the B+tree and return the row. A second
`… ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED` worker would queue behind the first block
instead of taking the next row (on the embedded `Database`, another connection's `BEGIN` is
refused with `55P03` while a block is open), where 18.6 gives `55P03` at once and hands the
workers rows 1 and 2.

The refusal is **deferred to the end of the statement's parse**, so an error 18.6 raises
*during* the parse or the plan wins over it: a projection `42703`, or a `42601` from
trailing junk, from the clause's own boundary, or from a `FOR READ ONLY` in the wrong
position. **The exception is an error pglet raises later than 18.6 does.** A missing
relation is `42P01` at *execute* time here (`EDGES.md` rows 29/60), so
`SELECT * FROM nosuchtbl FOR UPDATE NOWAIT` is this `0A000` here and `42P01` on 18.6. Both
error, and the deferral can't reach past a phase. The refusal fires on every route,
including the `DO $$ … $$` body's own sub-parser. An unrecognized lock strength or trailing
garbage (`FOR XYZ`, `FOR UPDATE foobar`) fails loudly (`42601`) instead of being swallowed
up to the statement boundary. The clause's **semantic** refusals are still skipped
(`EDGES.md` row 30).

### `alias.*` qualified wildcard

Expanded at lowering, against the catalog, to that relation's columns (its JOIN side), and
can be mixed with other projection items. The expansion is keyed on the name, so over a
`FROM`-clause `unnest(…)` item it's `0A000` (or `42702`). When a base table shares the
item's alias, it expands that table's names instead, which is `42703` unless they match the
item's own (`EDGES.md` row 8).

### Non-sole `*` + `RETURNING` projection

A `*` expands even when mixed with other items (`SELECT *, <expr>`, and INSERT/UPDATE/DELETE
`RETURNING *, <expr>`, expanded at plan time). A `RETURNING` item can be a **correlated
scalar subquery** against the row just written (the cell
`RETURNING *, (SELECT COUNT(*) … WHERE cell_id = cell.id) AS grant_count`), evaluated with
full subquery context. Over an `UPDATE … FROM`, correlating on a **source** column works too
(`RETURNING (SELECT count(*) FROM x WHERE x.k = s.k)`). `RETURNING` items go through the
same plan-time resolution and checks as a `SELECT` projection, so their declared types match
the values sent: `RETURNING COALESCE(int4_col, int8_col)` declares `bigint` and sends 8
bytes on every row. A binary client needs that one width per column to decode it.

Their `Column` leaves are **bound** at plan time against the clause's namespace: the DML
target under its effective name, plus the relations an `UPDATE … FROM` brings in. That
namespace comes from `returning_relations`, which the wildcard expansion, the
planner-declared type walk and the parameter-type walk all read. The executor (including
the executor-declared walk) derives the same relations from its own materialized rows, a
second derivation pinned against the first by fixture. An unknown qualifier is `42P01`, a
name the namespace doesn't have is `42703`, a bare name two relations claim is `42702`, and
all three are the same whether or not the statement produces a row. `RETURNING v LIKE 5`
gives the leaf's `42702`, not the operator's `42883`, on both engines, because ambiguity is
decided per node in the same walk that decides operand legality, not in a separate pass.

The relation walk covers all five kinds a `FROM` source can lower to (base `Scan`,
`VirtualScan`, `CteScan`, `SubqueryScan`, `FunctionScan`). So over targets that have the
name (`rvt(table_name TEXT, x INT)`, `run_t(id INT, v TEXT)`),
`FROM information_schema.tables c … RETURNING table_name` and
`FROM unnest(ARRAY['x','y']) AS v … RETURNING v` are each `42702`, as on 18.6. An `unnest`
item's column is named the way `chooseScalarFunctionAlias` names a single-function RTE's
column: after the alias. So `SELECT v FROM unnest(ARRAY[1,2]) AS v` works and
`SELECT unnest FROM unnest(ARRAY[1,2]) AS v` is `42703`, on both engines. (A two-argument
`unnest` keeps `unnest` for both columns on both engines, so a bare `unnest` there is
`42702`.) **Covering every kind isn't the same as counting every claimant.** For a
`VirtualScan` the list is the column list pglet's synthesized view emits, which for many of
the catalog relations it models is a strict subset of 18.6's. So the verdict silently
undercounts on every column in the difference (`EDGES.md`).

**Divergences:**

- **PG18's `OLD`/`NEW` correlation names aren't implemented.**
  `UPDATE amb_t SET v='NEWVAL' RETURNING NEW.v, OLD.v` returns `NEWVAL | TARGET` on 18.6
  and is `42P01 "new"` here.
- An aggregate in `RETURNING` **depends on the row count** where 18.6 always gives
  `42803`. `DELETE … WHERE id=999 RETURNING count(*)` is accepted here and returns
  `(0 rows)`, and the same statement over a matching row is `0A000`. (The binding rule
  above doesn't depend on row count. This check does, and it's a *different* check.)
- An alias-hidden table name (`UPDATE t o … RETURNING t.x`) is `42P01` on both, but PG's
  message says *invalid reference to* instead of *missing*. pglet also **quotes** a
  qualified name in its `42703` (`column "amb_t.nosuch"`) where PG leaves it bare.
- A whole-tree constant-cast check still runs before the binding walk, so
  `RETURNING nosuchcol + 'x'::int` is `22P02` here and `42703` there.

**A missing check, recorded rather than fixed:** there's no `42712`.
`UPDATE amb_t SET id=1 FROM amb_s amb_t WHERE amb_t.id=1` is 18.6's
`42712 table name "amb_t" specified more than once`, and is accepted here: the `FROM`
alias shadows the target's own name and the statement runs.

### `DO $$ … $$` anonymous block

A **minimal** PL/pgSQL form, `BEGIN <stmts>; [EXCEPTION WHEN <cond> [OR …] THEN
<handler>;] END`, runs the body statements and swallows an error whose SQLSTATE matches.
That covers the idempotent `CREATE TYPE` migration idiom (a duplicate `CREATE TYPE` raises
`42710 duplicate_object`). `DECLARE`, variables, control flow and an unrecognized condition
**fail loudly**; it isn't a general PL/pgSQL engine.

A block with an `EXCEPTION` clause is a **subtransaction**, as in Postgres. Catching the
error discards every write the body made (an earlier completed statement, a partly applied
multi-row one, and DDL alike). The handler's own writes, which run after the rollback and
outside it, survive, and a sequence the body advanced isn't rewound. A `DO` that **fails**
(no handler, an unmatched one, or a handler that raises) is one statement rolled back
whole, as on 18.6: every object its body or handler made is gone, every table it dropped is
back with its rows, and its error is the failing statement's (`EDGES.md` row 249).

A counter cell that a statement inside a `DO` replaces (a `RESTART`, a `DROP SEQUENCE` with
or without a re-create, a `TRUNCATE … RESTART IDENTITY`, or a `DROP TABLE` or
`DROP COLUMN` that takes an owned sequence, in the body, the handler or a nested `DO`) is
restored to where the in-place advances left it, by whichever rollback undoes the `DO`:
its own undo, an enclosing `DO`'s, or the enclosing block's, which reads the cells a
completed `DO` reports in its result (`EDGES.md` row 213, closed). Every `DO` pays for a
savepoint level and a catalog snapshot, including a successful one (`docs/storage.md`
§ Known bottlenecks / scaling limits). **Divergences:** transaction control inside the
body is refused with `0A000` instead of silently doing nothing. That over-rejects
`COMMIT`/`ROLLBACK` (accepted there) and changes the code of `ROLLBACK TO`'s `42601`.

### Keyword as identifier

Which words are reserved is **derived from each keyword's Postgres `kwlist.h` class**
(`Keyword::pg_class`), not listed by hand. A word is reserved if and only if it isn't a
valid `ColId` (that is, not UNRESERVED/COL_NAME), matching Postgres's bare-name acceptance
wherever the keyword table decides it (the divergences below are where something else
does). So `update`/`set`/`by`/`values`/`between`/`begin`/… (UNRESERVED/COL_NAME words) can
be used as bare column or table names, while `is`/`like`/`left`/`join`/… (TYPE_FUNC_NAME)
are rejected as bare columns, which Postgres also rejects (`SELECT is FROM t` is a syntax
error in both). The wider `ColLabel` positions where Postgres allows *any* keyword, a
qualified attribute (`t.left`) and an `AS` label (`… AS select`), are accepted. A bare
keyword label without `AS` still fails loudly (it would collide with clause markers, as in
`UPDATE t SET …`).

A live-Postgres sweep (`test_keywords`) checks that pglet accepts and rejects the same as
PG for every keyword in pglet's table at four positions: a bare select-list column, a
qualified attribute, an `AS` label and a `FROM` table name. For a TYPE_FUNC_NAME word it
checks a fifth, the function name before `(`. 18.6's `func_name` accepts that class, so
`left('ab', 1)` is a call (`a`) and `join(1)` is a call that resolves no function.

**Divergences:** the SQL value-function words (`user`, `current_date`, `current_user`, …)
written unquoted as a column definition's name, an `INSERT` column-list entry or an
`UPDATE`'s `SET` target are accepted, where 18.6 gives `42601`. The column created that way
then can't be reached by its bare name (`EDGES.md` row 75). `ISNULL` / `NOTNULL` are
keywords in `kwlist.h`'s TYPE_FUNC_NAME class, read as the postfix null tests `x ISNULL` /
`x NOTNULL` at `IS`'s precedence. As a function name, `isnull(1)` is a call on both
engines, and one pglet doesn't build: `0A000` here against 18.6's `42883`, the
*Wrong-SQLSTATE-class* row, just as `like('a', 'a')` is `0A000` here and works there.

### Operator lexing + resolution

**Operators are lexed the way `scan.l` lexes them.** Every `op_chars` run is one token. It's
cut where a `/*` or `--` starts inside it, and trimmed of a trailing `+`/`-` unless the run
holds a character SQL's own operators don't use (`1 =- 1` is `1 = -1`; `2 ^- 1` names
`^-`). The `self` characters and the four two-character rules (`<= >= <> !=`) come first,
and `=>` is its own token. A spelling that pglet's grammar has a kind for keeps it. Every
other spelling is the generic `Op` 18.6 lexes, whose name is its text, resolved at the
operator precedence level (`gram.y`'s `%left Op OPERATOR`) in both the infix and prefix
form.

pglet implements `= <> < <= > >= + - * / % ^ || -> ->> #> #>> ? ?| ?& @> <@ ~~ !~~ ~~* !~~*
^@`, plus `~` / `!~`, which work over a pattern of `^` followed by literal characters and
refuse any other pattern with `0A000` as a row is compared (`EDGES.md` row 251). It refuses
every other name with `42883 operator does not exist: <left> <op> <right>` at the
statement's **analysis**, with 18.6's own wording. That's deferred to the end of the
statement, so a grammar error still wins over it.

**Divergences** (`EDGES.md` row 184): the operators 18.6 has and pglet doesn't (`&` `#`
`|` `#-` `@?` `@@`, prefix `~` and `@`, `<->`, and `U&"…"`, whose `&` is an operator here);
`42725 operator is not unique` given as `42883` here; a **column** operand shown as
`unknown` in the message; and the refusal winning over an operand's own `42703`/`42P01`.
`OPERATOR(schema.op)` isn't parsed.

### Identifier case-folding

**An unquoted identifier is ASCII-downcased where the parser reads it, and a
`"…"`-quoted one is exact.** That's Postgres's rule, applied by `lexer::Token::name` for a
token in identifier position (the parser's name-producing reads go through it) and by
`split_name_literal` for a name literal, so every comparison after that is a plain byte
comparison. `Foo` / `foo` / `FOO` are one name and `"Foo"` is another. A **keyword** used
as a name folds too (`CREATE TABLE Sequence` creates `sequence`), since Postgres's scanner
downcases before classifying.

The fold only covers **ASCII**, both in the parser and at the post-parser sites that
re-fold a catalog type key (`execute_create_type`, `execute_drop_type`,
`resolve_column_type`), because the oracle folds `A`–`Z` and nothing else. We checked that
on the oracle, and it matches the source: `scansup.c`'s `downcase_identifier` folds a
high-bit byte (through `tolower`) only when the database encoding is **single-byte**, so a
multibyte encoding only takes the ASCII branch. That's a property of the oracle's encoding
(UTF-8), not of Postgres in general. A single-byte database is the case that could behave
differently, and pglet doesn't model a per-database encoding.

With no quoting anywhere, `CREATE TYPE CAFÉ AS ENUM ('a'); CREATE TYPE café AS ENUM ('b')`
registers two types on both engines (18.6's `pg_type` holds `cafÉ` and `café`), and
`CREATE TYPE ZÄP AS ENUM ('a'); CREATE TABLE zt (c zäp)` is `42704` on both. Those cases
guard against a post-parser site that folds *wider* than the parser. A Unicode
`to_lowercase()` would merge the first pair into one key (the second `CREATE TYPE` would
then be `42710`, the column would bind to the first type, and inserting the second type's
label would report it invalid), and would silently accept the second pair. The value
grammars are a separate question and aren't covered by this: `lower()`, `upper()`, `ILIKE`
and `parse_bool` all fold Unicode here.

A **name literal** (`nextval`/`currval`/`setval`'s `regclass` argument) folds by the same
rule inside `SplitIdentifierString`'s grammar, so the two name positions always name the
same relation. A quoted **value-function** spelling is skipped, not matched
(`SELECT "current_timestamp"` is `42703` on both, while `SELECT "now"()` names a real
`pg_proc` entry and resolves on both), and the contextual-word matchers only take an
unquoted token. `tests/fixtures/identifier/*` pins both directions, and
`crates/pglet/tests/case_folding.rs` covers the cases a differential fixture can't express.

**Residual divergences:**

- A *quoted type name* doesn't parse (`42601` here, `42704 type "INT" does not exist`
  there). Because the reference side can't spell one, an **ENUM name is ASCII-downcased
  again where it's stored**. At that width this only affects a quoted name (an unquoted
  `name` arrives from `Token::name` already folded, so folding it again does nothing).
  `CREATE TYPE "Loud"` + `CREATE TABLE t (m loud)` is accepted here and `42704` there. That
  fold rescues a name that differs from a bare identifier only in ASCII case and leaves the
  rest: `CREATE TYPE MÖÖD` stores `mÖÖd`, `CREATE TYPE Sequence` stores `sequence`, and
  each makes the whole database unexportable (see `EDGES.md`).
- A **quoted lower-case function name** that pglet models as a function, but 18.6 only has
  as grammar, resolves here and is `42883` there (`"coalesce"`, `"nullif"`, `"greatest"`,
  `"least"`, `"trim"` checked; `"substring"`, `"count"(*)`, `"now"()`, `"upper"`,
  `"lower"`, `"gen_random_uuid"()` agree, each naming a real `pg_proc` entry). A
  **parenthesized** quoted name is a `pg_proc` lookup there, and pglet's function table
  doesn't record which of its names `pg_proc` holds. `SqlValueFunction::from_name`'s arms
  are `current_date`, `current_timestamp`, `current_time`, `localtime`, `localtimestamp`,
  `current_user`, `session_user`, `user`, `current_role`, `current_catalog` and
  `current_schema`, and dropping `parse_identifier_primary`'s `!quoted` guard would make
  `SELECT "current_timestamp"` resolve here. The **non-temporal** value functions are
  modeled (see *Session information functions* below), and their quoted spellings agree
  with 18.6: `SELECT "current_schema"`, `SELECT "current_user"`, `SELECT "user"` and
  `SELECT "now"` are `42703` on both, since a quoted word is never a keyword.
- There's no `NAMEDATALEN` truncation. That's self-consistent, so both name positions
  agree, and `name` (OID 19) as a declared type doesn't add truncation by itself, since
  nothing pglet declares as `name` can exceed 63 bytes.
- `CREATE SCHEMA "PUBLIC"` is pglet's documented `0A000` where 18.6 creates a second
  schema. It's a **loud** divergence, not a silent no-op.

### INTERVAL

A real column type over an in-house three-field
`PgInterval { months: i32, days: i32, micros: i64 }` (the Postgres layout). It supports
three input grammars (verbose `1 year 2 mons`/`ago`, SQL-standard `1-2`/`3 4:05:06`, and
ISO-8601 `P1Y2M3DT4H5M6S` with `M` before or after `T`), plus normalization of fractional
units. Output is `postgres` style (signed pluralization, `mon`/`mons`, a forced `+` on a
positive group after a negative one, and zero as `00:00:00`). The binary wire format is 16
bytes (time, then day, then month; OID 1186, array 1187), and the index key preserves
order.

**Comparison uses Postgres's estimate** (`micros + (months*30 + days)*86400e6`, in i128).
So `INTERVAL '30 days' = INTERVAL '1 mon'`, a UNIQUE interval index treats them as
duplicates, and ORDER BY / GROUP BY / DISTINCT treat them the same way. Arithmetic:
`interval ± interval`, unary `−`, `interval ×/÷ number` (fractions cascade down, matching
`interval_mul`/`interval_div` byte for byte), `timestamp ± interval` (a three-pass calendar
step with a leap-aware day-of-month clamp), `date + interval → timestamp`, and
`timestamp - timestamp → interval` (via `justify_hours`). Numeric fields parse exactly
(i64, not f64), and exponent forms are rejected. Differential-tested
(`types/interval_*.sql`).

**`interval(p)` sub-second precision is applied.** `'1.23456 secs'::interval(3)` is
`00:00:01.235` on both engines, at a cast, at a column store and element-wise under a `[]`.
It only rounds the `micros` field and leaves months and days alone (`EDGES.md` row 123).

**Divergences:** an `INTERVAL <fields>` **range** spec fails loudly at a column and at the
multi-token cast spellings, while a single-field cast spec is swallowed as a column alias:
`'1 year 2 mons'::interval year` returns `1 year 2 mons` here against 18.6's `1 year`.
There's no `infinity`/`-infinity`. The text spelling, and the binary all-maximum /
all-minimum field sentinels that 18.6 reads as infinity, are both `22007` here. The binary
one is raised at `Bind`, where it fails an open block (`EDGES.md` row 139). The same
triples spelled as numbers or reached by arithmetic are finite intervals here (`EDGES.md`
row 143). SQLSTATEs match PG: malformed input is `22007`, a parse-time field overflow
(`interval '999999999999 years'`) is `22015`, and an arithmetic overflow (`interval × n`,
`timestamp ± interval`) is `22008`. It isn't DST-aware: pglet is UTC-only, so a day is
always 24 h.

### DATE

A real column type: `Value::Date`, rendered as `YYYY-MM-DD` in text and as i32 days from
2000-01-01 in binary. It's B-tree-indexable (chronological order, enforced UNIQUE), with
full `=`/`<`/`>`/range and `ORDER BY`, and casts (text↔date, `timestamp::date` truncates the
time of day, date→timestamp gives midnight). A comparison literal coerces to date
(`WHERE d = '2024-06-15'`), and bound `$N::date` params work (inferred from column
context). Arithmetic: `date ± int → date`, `date - date → int4`, and
`date ± interval → timestamp`. `CURRENT_DATE` (a value function) and
`DEFAULT CURRENT_DATE` work. Differential-tested against Postgres (`types/date_*.sql`,
`types/current_date.sql`). Input accepts a one- or two-digit month and day (`2026-1-1`,
like PG). A malformed spelling is `22007` (`invalid_datetime_format`), and a well-shaped
but out-of-range field (`2026-13-01`, `2026-02-30`) is `22008`, both matching PG.

**Domain:** a `date`, `timestamp` or `timestamptz` value is `0001-01-01` through
`9999-12-31` (a `timestamptz` read in UTC). That's enforced at the producers the domain
section lists (text and binary input, `date ± integer`, `± interval`, `date + time`,
`date_trunc`, the rounding carries, the clock reads, the date/timestamp casts) and at the
entry points: a `Value` bound through `execute_bound` and a write's stored value (with a
`timestamp` brought to UTC and a whole microsecond at both), and the dump. The dump refuses
a stored **row** holding such a value, or a timestamp stored off UTC. A stored expression's
text isn't checked, so a `CHECK` or partial-index constant in year 0 over an empty table is
still accepted (`EDGES.md` row 51) (`docs/temporal.md` § The date and timestamp domain).
Where PG refuses too, the error matches PG in the cases we measured: a year-0 field in a
literal that parses is `22008 date/time field value out of range: "…"`, and
`date ± integer` / `± interval` past PG's own range are `22008 date out of range` /
`timestamp out of range`. The section's table lists the measured cases where it doesn't.
`± interval` checks its months and days passes against PG's own range, not the domain, so
`date '9999-12-31' + interval '1 day -1 microsecond'` is a value on both.

**Divergences:** a year with a leading `-` gives pglet `22007` (pglet doesn't model BC),
against PG's `22009`, where PG reads the sign as a time zone displacement
(`'-2026-06-15'`). Both give `22007` at `'-0001-01-01'`. A result PG can represent but the
domain excludes (a `BC` value, or a year past 9999) is refused with
`22008 date out of range` / `timestamp out of range` where PG returns it (`EDGES.md` row
142). **`infinity` / `-infinity`** for `date`, `timestamp` and `timestamptz` (text, binary
sentinels, order, index, casts, arithmetic, `date_part`, `isfinite`, `COPY`, the dump)
agree with PG in the cases `docs/temporal.md` § `infinity` and `-infinity` covers. That
section also names the measured divergences, among them `0A000`s where PG returns an
infinite `interval` or `numeric`, `± interval` at the interval field triples PG reads as
infinite (`EDGES.md` row 143), and the spellings PG's tokenizer accepts next to a
punctuation byte it treats as a delimiter, or next to `AD`/`BC`.

### NUMERIC / DECIMAL

A real column type over an **in-house base-10 bignum** (`PgNumeric`, no dependencies):
exact arbitrary-precision decimal with a separate display scale, and a `NaN` that sorts
above every finite value and equals itself. Text (`1.50` keeps its scale), Postgres binary
(`numeric_send`, base-10000 regrouping) and an order-preserving B-tree key (indexable,
with enforced UNIQUE, so `1.5` collides with `1.50`). Comparison and `ORDER BY` are exact
(never via `f64`; numeric vs int is exact, numeric vs float goes through the numeric's
double). Arithmetic `+ − * /` follows Postgres's per-operator scale rules (add/sub take the
`max` scale, mul the sum of scales, div `select_div_scale`), plus unary `−`, and `SUM`/`AVG`
are exact. Casts: text/int↔numeric (exact), numeric↔float (lossy double), and numeric→int
(half away from zero, a loud `22003` on overflow).

`NUMERIC(p,s)` rounds to scale on store and fails loudly on precision overflow (`22003`).
The scale can be **negative** (`numeric(5,-2)` rounds to hundreds). The **modifier itself**
is validated where `numerictypmodin` validates it, instead of being dropped when declined.
A precision outside `1 ..= 1000`, a scale outside `-1000 ..= 1000` and any number of
arguments other than one or two are `22023`, after the `int4` coercion every component goes
through first (`22003` out of range, `22P02` not an integer), in the order measured on 18.6
(`EDGES.md` row 124, `docs/coercion.md` § A modifier the type refuses).

The functions `abs`/`ceil`/`ceiling`/`floor`/`round(x[,s])`/`trunc(x[,s])` are exact for a
numeric argument. On an **integer** argument, `ceil`/`floor`/`round(x)`/`trunc(x)` resolve
to the Postgres `double precision` overload (result `float8`, type and value together, so
`ceil(5)` is `5.0`, not `5`), while the two-argument `round(x,s)`/`trunc(x,s)` stay
`numeric`. A `CASE`/`COALESCE` int/float literal branch unifies to a numeric sibling. An
**unknown-typed** argument resolves to the numeric category's *preferred* type, `float8`,
for every member including `abs` (this is `typeconv.sgml`'s own worked example for the
rule), so `SELECT abs(NULL)` is `double precision` on both legs. Only `abs` makes the rule
visible, since it's the one member whose rule passes the argument type through
(`types/math_fn_unknown_arg_declared_types.sql`).

**That holds for an argument that's *directly* `NULL`, not for the whole family.** The
executor arm goes through `Literal::infer_type`, so `Literal::Null` is the only argument
shape it can call untyped. An argument that's only **indirectly** untyped still declares 25
on the simple query, against `Describe`'s 701: `abs(-NULL)`,
`abs(CASE WHEN true THEN NULL END)` and `abs(COALESCE(NULL, NULL))`, all three of which
18.6 refuses (`42725 operator is not unique: - unknown`, then
`42883 function abs(text) does not exist` twice). It's silent here and loud there, and the
value is NULL either way; recorded in `EDGES.md`. `abs(NULL::int)` is the control: 23 on
both legs and both engines.

**Two more residual divergences nearby, neither closed:** `trunc(NULL)` is 18.6's
`42725 function trunc(unknown) is not unique` (its overload set spans more than the numeric
category) and resolves to `float8` here, since pglet doesn't model an overload-ambiguity
check. And `abs('1')` / `round('1.5')` are `701` returning `1` / `2` on 18.6 and `0A000`
here, because a bare string literal is typed as `text` instead of staying Postgres's
`unknown` (the general literal-typing gap recorded elsewhere in this repo), which also
makes their `Describe` declare `text`. Both are loud.

A decimal or exponent literal (or an integer outside `i64`) is `NUMERIC` (Postgres
constant typing), carrying its exact scale. Bound `$N::numeric` params work. The
`NUMERIC(p,s)` typmod is enforced on **both** the column form (store) and a **cast**
(`x::numeric(p,s)` rounds to scale and gives `22003`, regardless of context) via the
shared `apply_typmod`. A `numeric(p,s)[]` carries the **element's** typmod and
`apply_typmod` walks the elements, so such a column rounds `'{1.25}'` to `{1.3}` and
refuses a bare `1.25` with `42804`, as 18.6 does (`EDGES.md` row 120, closed). `%` (modulo,
exact, truncating), `^` (integer exponent via repeated squaring), `sqrt` (Newton's method
with exact ½-ulp rounding), `sign`, and digit-group underscores in a numeric **literal**
(`1_000`) are supported. Inside a string read as a number (`'1_0'::numeric`,
`'0x10'::int4`), an underscore or a radix prefix is `22P02` where 18.6 accepts it
(`EDGES.md` row 189). **Gap:** a **fractional** numeric exponent (`2 ^ 0.5`) is `0A000`
(the ln/exp bignum work is deferred), and numeric `Infinity`/`-Infinity` are still missing
(a loud `22P02`). Differential-tested against Postgres (`types/numeric_*.sql`).

### Session information functions

`version()`, the four role spellings (`CURRENT_USER` / `SESSION_USER` / `USER` /
`CURRENT_ROLE`), `CURRENT_CATALOG` / `current_database()`, `current_schema` /
`current_schema()`, `current_schemas(bool)` and `current_setting(name [, missing_ok])`.
That's every spelling a Postgres client can send for the session's identity, at 18.6's
declared type. It needs `name` (OID 19) and `name[]` (1003) as declared types. They have
no special shape (a `Value::Text` plus the OID, and binary is the raw bytes with no NUL
padding, exactly how `varchar` is modeled), and they're in Postgres's **string** type
category. That's two predicates, not one:

- `planner::legality`'s `is_string`, so `owner = current_user` over a `text` column and
  `INSERT INTO t(txt) VALUES (current_user)` both resolve, while `current_user = 1` stays
  `42883`;
- `planner::common_type`'s `type_category` plus `name`'s implicit `pg_cast` rows
  (`name → text`, `text → name`, `varchar → name`), so a common-type fold over `name` and a
  string sibling resolves to `name` (19) as 18.6 does, instead of refusing the pair.

They have **no column form**: `type_oid_from_name` maps neither, so
`CREATE TABLE t (a name)` and `'a'::name` stay `42704`.

**The parse split matches 18.6.** The six keyword forms take no argument list
(`current_user()` is `42601` on both), except `current_schema`, which is a
TYPE_FUNC_NAME_KEYWORD *and* `pg_proc` 1402, so both spellings resolve and
`current_schema(1)` is `42883`. The non-keyword names stay column references when written
bare (`SELECT version` / `current_database` / `current_schemas` / `current_setting` is
`42703` on both). Every arity is refused the way 18.6 refuses it, and that check is what
keeps an unimplemented arity from being accepted. Each form is **persistable**, so
`DEFAULT current_user` is stored as a `ColumnDefault::Expr` and rendered by `pg_get_expr`
as written: the six keywords bare and upper-case, `current_schema()` **quoted**
(`"current_schema"()`, since the bare word would re-lex as the keyword), and
`current_database()` / `version()` as calls. All of that was checked, and all of it
round-trips through `dump`'s `ROUND_TRIP_CORPUS`.

**What each returns:** the native server echoes the startup packet's `user` and
`database`, which is what a `trust`-authenticated Postgres returns and what makes the
differential fixtures byte-comparable. The embed facade and the wasm bindings have no
packet and return `pglet` (`crates/pglet/tests/session_information.rs`).
`current_setting` reads the **same** table `SHOW <name>` reads from, so it raises the same
`42704` for an unknown name, `missing_ok = true` turns that into NULL, and it's strict in
both arguments.

**Divergences:**

- `current_schema` / `current_schemas` / `current_setting('search_path')` all read the
  fixed search path, so `SET search_path` changes 18.6's results and not pglet's
  (`EDGES.md` row 66).
- There are no roles, so `SET ROLE` can't separate `current_user` from `session_user`
  (row 73).
- A bare string literal written for the `boolean` argument is `42883` here where 18.6
  coerces the `unknown` (row 74).
- A `name` operand next to an **enum** column is refused as 18.6 refuses it, with the
  identity reaching the branch, comparison and common-type checks alike (row 114, closed).
- `version()`'s banner is `PostgreSQL 18.6 (pglet) on <target>, 64-bit`. The release
  agrees with `SHOW server_version` by construction, and the rest is deliberately not
  18.6's build string. `current_setting('server_version')` is `18.6` against 18.6's
  `18.6 (Debian …)`, and `current_setting('TimeZone')` is a fixed `UTC` that no accepted
  `SET` changes (row 69).

**Not modeled:** `pg_typeof` (its declared type is `regtype`, which *does* have a special
shape: binary is a 4-byte OID and text a type name. Its **value** is a static type OID,
which pglet's two type walkers work out separately) and `pg_backend_pid()` (a fact about a
native process, which the embed and wasm hosts don't have). Both stay loud, and
`session/pg_{typeof,backend_pid}_is_not_modelled.sql` pin that. Differential-tested end to
end (`tests/fixtures/session/*`, both wire formats and both declared-type legs).

**Two unguarded surfaces, both recorded rather than closed:**

- The value-function words (`SqlValueFunction::from_name`'s arms) are accepted unquoted in
  every **name** position where 18.6 gives `42601`: a `CREATE TABLE` column, an `INSERT`
  column list, an `UPDATE`'s `SET` target, every word checked at each. A column created
  that way then can't be reached by its bare spelling (`EDGES.md` row 75; `test_keywords`
  can't probe any of them because none is a lexer `Keyword`).
- `CREATE INDEX ix ON t ((current_user))` / `… WHERE a = current_user` are accepted where
  18.6 refuses with `42P17 functions in index {expression,predicate} must be marked
  IMMUTABLE`. They join `now()`/`gen_random_uuid()`/`CURRENT_DATE` on the unguarded
  non-IMMUTABLE index surface, and natively such a key depends on which connection
  inserted the row.

### SQL typed constants (`DATE '2024-06-15'`, `NUMERIC(5,2) '1.5'`, `DOUBLE PRECISION '…'`)

`gram.y`'s `AexprConst`, lowered to the same cast node `'…'::type` gives, so the value,
declared type, a **written** typmod and the deparse all follow one path. The type name is
read at `ConstTypename`, not `Typename`: a parenless `char` / `character` carries no
length, where a column or cast takes `char(1)`, so `char 'ab'` is `ab`. An identifier before
a string is a type name, never a column, and `float` before a `(` is a type name, never a
function call. That's a point where the speculative read commits. Residuals, that commit
and the `INTERVAL '…' <field>` qualifier: `docs/parser.md`.

### `x IS [NOT] TRUE / FALSE / UNKNOWN`

Postgres's `BooleanTest`, never NULL. The operand is checked by `planner/legality.rs`
(`42804`), and an `unknown` literal is converted (`22P02`). It deparses faithfully, so a
stored `CHECK` round-trips.

### TIME

A real column type: `Value::Time` is microseconds since midnight (a raw `i64`, so
`24:00:00` is a representable endpoint). Text is `HH:MM:SS[.ffffff]` (trailing zeros
trimmed), binary is `int64` (OID 1083, array 1183), and the index key is plain big-endian
(chronological, enforced UNIQUE), with full `=`/`<`/`>`/range and `ORDER BY`. Input accepts
`HH:MM:SS[.f]`, `HH:MM`, `HHMMSS`/`HHMM` (including a fractional suffix), and 12-hour
`AM/PM`. Casts: text↔time and `timestamp::time` (truncates to the time of day).
Arithmetic: `time - time → interval`, `time ± interval → time` (microseconds only, it
**wraps mod 24 h**, and days/months are ignored), and `date + time → timestamp`.
`CURRENT_TIME`/`LOCALTIME` value functions and `DEFAULT CURRENT_TIME` return plain `time`.
Differential-tested (`types/time_*.sql`). A malformed spelling is `22007`, and a
well-formed but out-of-range field (`25:00:00`, `12:99`) is `22008`, both matching PG.

**Divergences:** a time zone suffix on a time literal fails loudly with `22007` (PG accepts
and ignores it; pglet is UTC-only). `CURRENT_TIME`/`LOCALTIME` report as `time` (OID 1083),
not PG's `timetz` (1266), so a bare `SELECT CURRENT_TIME` can't be directly
differential-tested (the fixtures use deterministic predicates). `CURRENT_TIME(p)` with
parentheses is rejected with `42601`. `TIMETZ` isn't implemented (fails loudly with
`42704`), with or without a modifier: `timetz(3)` and `time(3) with time zone` are the same
`42704`, because the canonical name puts the precision **last**. **`time(p)` sub-second
precision is applied:** `'03:04:05.6789'::time(3)` is `03:04:05.679` on both engines, and a
rounding carry reaches the `24:00:00` endpoint instead of wrapping (`EDGES.md` row 123).

### bytea

A real `Value::Bytea` column type. Text input is read the way 18.6's `byteain` reads it:
the hex format after a lowercase `\x` (whitespace allowed between digit pairs), and the
escape format otherwise (the text's bytes, `\\` and `\ooo`), with 18.6's `22023` for a bad
or unpaired hex digit and `22P02` for an invalid escape. `EDGES.md` row 279 (closed) lists
the entry points we checked and the residuals: a default string the hex format refuses is
accepted by `CREATE TABLE` / `SET DEFAULT` and gives `22023` at each `INSERT` that uses it.
Output is hex text, with full binary wire support, a byte-count `length()`, and B-tree
indexing / UNIQUE. `bytea || bytea` and `bytea LIKE bytea` are legal in Postgres and not
implemented here (`0A000`). Next to a bare string literal, `||` gives a `text` result (the
`||` row; `EDGES.md` row 293) and `LIKE` gives `42883`, where 18.6 reads the literal as
`bytea` and returns a result (`EDGES.md` row 262).

### Real prepared statements / param type inference

`$N` is typed from how it's used (cast/column/`ANY`/`@>`/`IN`/`LIMIT`/`||`/`->` key),
decoded to a `Value`, bound into the plan, and reported via `ParameterDescription`. A type
the client declares at `Parse` is the parameter's type for parse analysis, as on 18.6:
operators, common-type resolution, and the result column (`docs/param_binding.md`;
`EDGES.md` rows 145–146, closed). The full record is in § Prepared statements / param
inference — details, below.

#### Prepared statements / param inference — details

Column context is alias-aware across a JOIN (a per-scope map from FROM alias to table) and
sees through `DISTINCT`/`Window` wrapper nodes, so a join-ON array param under
`SELECT DISTINCT` gets typed (`rg.scope_id = ANY($2)`, the spine's scoped
`actor_search`). That only holds **while the alias names no real relation**: the catalog
lookup runs before that map, so a FROM alias with the same name as a table reports *that
table's* column type (see `EDGES.md`).

These all get typed instead of reporting OID 0:

- a `||` concat operand;
- a `LOWER($N)` argument;
- a `->`/`->>` key operand (typed `text`, matching Postgres's `(jsonb, text)` preference;
  the index form needs an explicit `$N::int`);
- a bare `$N` in an `UPDATE … SET` value (including a `CASE`/`COALESCE` result branch,
  typed from the target column);
- a bare `$N` at a **boolean position** (typed `bool`: the clause roots and the node
  positions `Expr::for_each_child_ctx` marks, except the `IS [NOT] TRUE` operand);
- a filter on a **virtual-table** column (`WHERE table_name = $1` over
  `information_schema.*`, resolved via the catalog view's column list).

An OID-0 param makes a binary-protocol client (tokio-postgres) fail type introspection
(`Parameters`), though a text-protocol client tolerates it.

A `$N` that parse analysis leaves **without** a type is refused the way 18.6 refuses it:
`42P18`, or `42P08` where a later occurrence types it. A bare select-list or `RETURNING`
entry resolves to `text` (OID 25), and an `INSERT … SELECT` source's resolves to its target
column's type, as in 18.6 (`docs/param_binding.md` § A parameter left without a type is
refused; residuals, `EDGES.md` row 276).

**Gap:** Postgres never reports OID 0 for a parameter that's used. pglet still does
wherever its inference has no rule for a position 18.6 types, and decodes the value as text
(the native server) or by its JS runtime type (the wasm binding). See
`docs/param_binding.md` § Deferred / known gaps.

**Gap: there's no parameter-type *unification*, so pglet never raises `42P08`, and a `$N`
written twice keeps one of its types and drops the other: the highest-priority one, and at
*equal* priority the one that got there first.** Postgres deduces one type per parameter
across the whole statement and refuses when the deductions disagree. pglet's `record`
keeps the highest-priority fact per slot and has no consistency check at all. It's wider
than any one clause. The cases below were measured over
`oc_probe(id INT PRIMARY KEY, v TEXT, n BIGINT)`, each accepted here and refused there, and
the refusals come under three different SQLSTATEs (`42883`, `42P08`, `42804`), which is why
no single one of them names the class:

- `… DO UPDATE SET n = $1 RETURNING (v = $1)` is
  `42883 operator does not exist: text = bigint` there and `{20}` here.
- `… DO UPDATE SET v = $1 WHERE oc_probe.n > $1` is
  `42883 operator does not exist: bigint > text` there and `{25}` here.
- `… DO UPDATE SET v = $1, n = $1` is the bare
  `42P08 inconsistent types deduced for parameter $1 — text versus bigint` there and
  `{25}` here.
- The `UPDATE` case, `UPDATE oc_probe SET n = $1 WHERE v = $1`, is
  `42804 column "n" is of type bigint but expression is of type text` there and `{20}`
  here.

They're loud on 18.6 and silent here: the statement runs with one operand coerced to a type
the user never wrote.

An `ON CONFLICT` clause is walked too, at every position it parameterizes: the arbiter's
key parts and its partial-index predicate, the `DO UPDATE` `SET` list (whose bare `$N`
follows the assignment-target rule, `CASE`/`COALESCE` branches included), and the action's
`WHERE`. The **action** is walked against its own namespace, the target plus `excluded`
with the target's shape (`planner::plan`'s `on_conflict_action_relations`, the
`RelationColumns` twin of the `ColumnScope` that `lower_on_conflict` checks the same two
positions against). It isn't walked against the catalog, because the catalog answer is
only right until a table is *named* `excluded`. With a real
`excluded(id INT, n TEXT, v TEXT)` next to the target,
`… DO UPDATE SET v = 'z' WHERE excluded.n = $1` reported the shadow table's `{text}`
against 18.6's `{bigint}`, and the bound statement was then a loud
`42883 operator does not exist: bigint = text` where 18.6 updates the row. That's a false
rejection caused by nothing but an ordinary table name. The **arbiter** is deliberately
outside that namespace, because 18.6 does the same, not out of caution:
`ON CONFLICT ((excluded.id)) DO UPDATE …` and
`ON CONFLICT (id) WHERE excluded.n > 0 DO UPDATE …` are both
`42P01 invalid reference to FROM-clause entry for table "excluded"` there.

The clause is walked **after** the row source and **before** `RETURNING`
(`transformInsertStmt`'s order). That order has visible effects, because `record` keeps the
first fact at equal priority. On 18.6, `VALUES (1,$1) … WHERE n > $1` is
`42883 bigint > text`, and
`… DO UPDATE SET v = 'z' WHERE oc_probe.n = $1 RETURNING (v = $1)` is
`42883 text = bigint` with the caret on the `RETURNING`, each collision reported from the
*later* clause. The `RETURNING` example has to put its collision in the action's
**`WHERE`**, not its `SET` list, or it stops showing the order at all. `coerce_to_type`
wraps a `SET` right-hand side in an `Expr::Cast`, which records at the cast priority and
wins over the `RETURNING` context fact whichever clause is walked first. That's the same
escape this row describes for the arbiter-and-`SET` collision below. We checked both ways
here: `… DO UPDATE SET n = $1 RETURNING (v = $1)` is `{20}` with the clause walked before
`RETURNING` **and** `{20}` with it walked after, while the `WHERE` spelling is `{20}` before
and `{25}` after.

**The residuals on this clause**, each checked against 18.6 over
`oc_probe(id INT PRIMARY KEY, v TEXT, n BIGINT)`:

*The arbiter key expression.* It counts its `$N` without typing it:
`ON CONFLICT ((id + $1)) DO NOTHING` reports OID 0 here against 18.6's `{integer}`,
because pglet infers no type across an arithmetic operator. The explicit
`((id + $1::int))` spelling gets typed, and 18.6 gives `{integer}` for both.

*The arithmetic assignment right-hand side.* This is the same missing rule in the position
that matters most, because it's the standard upsert-increment idiom:
`ON CONFLICT (id) DO UPDATE SET n = oc_probe.n + $1` and its `SET n = excluded.n + $1`
spelling both report `{0}` here against 18.6's `{20}`, and the
`UPDATE oc_probe SET n = n + $1` twin does the same. It's more than an unhelpful OID: the
parameter then decodes as text and the statement fails at execute with
`42804 type error: operator + requires numeric operands` (checked through a text-format
`Bind`), where 18.6 adds and returns. So of the upsert spellings,
`SET n = <rel>.n + $1::int8` binds and `SET n = <rel>.n + $1` doesn't. The bare
`SET n = $1` / `SET v = $1` assignment binds, and so does a comparison against a typed
sibling (`WHERE excluded.n = $1`).

*The arbiter-and-`SET` collision.* A `$N` written in **both** takes the `SET` list's type,
where 18.6 keeps the arbiter's: `ON CONFLICT ((id + $1)) DO UPDATE SET v = $1` is
`{integer}` there and `text` here, and `ON CONFLICT (id) WHERE n > $1 DO UPDATE SET v = $1`
is `{bigint}` there and `text` here. This isn't about walk order, and reordering the walk
doesn't affect it. `coerce_to_type` wraps an assignment right-hand side in a plain
`Expr::Cast`, which looks the same as a user cast, so it records at the cast priority and
wins over the arbiter's context fact whenever it runs. It's one case of the no-unification
gap above, whose `UPDATE` spelling is `UPDATE oc_probe SET v = $1 WHERE n = $1`.

*The unchecked arbiter qualifier.* The arbiter is neither resolved nor checked (see the
*ON CONFLICT* row), and it types off **whichever relation the qualifier names**, falling
back to the target only when the catalog has nothing by that name. So it's a lookup across
the whole database, not just the target, which the `RETURNING` clause's
top-level walk, bound to its own namespace, doesn't do. The pair we measured uses an
`oc_other(id INT, n SMALLINT)` next to the target that the statement never mentions:
`ON CONFLICT (id) WHERE oc_other.n > $1 DO NOTHING` reports `{21}`, `oc_other`'s `int2`,
which shows that *that* relation answered rather than the target, while
`ON CONFLICT (id) WHERE zzz.n > $1 DO NOTHING` reports `{20}`, the target's own `n` reached
through the fallback. 18.6 refuses both with `42P01 missing FROM-clause entry`, at Parse.
It's the same class as the FROM-alias entry `docs/param_binding.md` records for a plain
`SELECT`, in the one clause where pglet resolves no names at all. The `((zzz.n + $1))`
key-expression spelling plans here and gives `42P10` at execute, where 18.6 gives `42P01`
at Parse.

The action's own **top-level** positions don't have that hole. `lower_on_conflict`
resolves both halves against a **closed** namespace holding only the target and `excluded`,
so a bogus qualifier and the quoted `"EXCLUDED"` are both `42P01` on both engines, before
any plan exists to infer over.

*An `excluded` reference inside a **subquery** in the action* is the same hole one level
in, and it's a typing hole, not a checking one. `Inference::walk_scoped` clears the clause
namespace at every subquery boundary. A cleared `RETURNING` scope falls back to a catalog
lookup that finds an outer relation again when it *is* a catalog table, but `excluded`
isn't a catalog table under any spelling, so nothing finds it again. Over
`oc_probe(id INT PRIMARY KEY, v TEXT, n BIGINT)` and `oc_src(id INT, v TEXT)`,
`… DO UPDATE SET v = 'z' WHERE EXISTS (SELECT 1 FROM oc_src s WHERE s.id = $1 AND excluded.n = $2)`
reports `{integer,0}` here against 18.6's `{integer,bigint}`, and `{integer,text}` with a
real `excluded(id INT, n TEXT, v TEXT)` next to the target, which 18.6 ignores. The control
is the same comparison written directly in the action's `WHERE`: `{bigint}` on both
engines, with or without the shadow table.

**Masked, not harmless:** at a conflicting key the action's `EXISTS` gives pglet's own
`0A000 … requires execution context` at execute, and the statement fails there, writing
nothing. At a free key the action isn't evaluated at all, and the `INSERT` succeeds on a
`VALUES` row that `$2` doesn't reach. The wrong OID reaches a `Describe` reply and a `Bind`
decode, and no row holds a value read under it.

**No fixture can pin it**, and for a reason that goes beyond this case: the harness
compares output-column declared types and never reads `ParameterDescription` at all, so it
can only see a parameter OID through a `Bind` that fails. At a conflicting key this
statement doesn't survive execute on pglet to be compared.

*The all-unknown `CASE`/`COALESCE` against a non-text column* isn't specific to this
clause. It's typed from that column here (`SET n = COALESCE($1, $2)` gives
`{bigint,bigint}`), where 18.6 folds the unknowns to text and refuses the statement with
`42804`. The `UPDATE` spelling diverges the same way.

### `to_char` / `AT TIME ZONE`

The spine's ISO-8601 projection
(`to_char(ts AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"')`). The tokens
YYYY/MM/DD/HH24/MI/SS and quoted literals work. A non-UTC zone or an unknown token fails
loudly.

### `timestamptz` text wire output

Rendered with the UTC `+00` offset (`2024-06-15 12:30:45.123+00`). A plain `timestamp`
(no tz) has none. **Text-protocol** drivers (node-postgres, the fuz_app matrix) depend on
the offset: a value without one is reparsed in the client's local zone, which silently
shifts the instant. pglet has no session zone (it's always UTC), so this is the same as
Postgres at `TimeZone=UTC`, and the binary path is offset-correct. A `timestamptz[]`
element carries it too, rendered at the declared element type, and a binary array's header
names that element type. A bare `$N` the client declared as `timestamptz` behaves like a
`timestamptz` column here too. Input splits a spelling into fields the way `ParseDateTime`
does, and reads a numeric offset the way `DecodeTimezone` does: an hour outside
`0 ..= 15`, or a minute or second outside `0 ..= 59`, is `22009` (`docs/temporal.md`).

### `ErrorResponse` structured fields

Every error goes out through **one** builder (`Error::wire_fields`), shared by the native
server's three send sites and the wasm host. It carries `S`/`V`/`C`/`M` plus
`D`/`H`/`s`/`t`/`c`/`n`/`P` where 18.6 does. It's byte-identical on `23502`, `23505`,
`23514` (both routes), `23503` (both directions), `23001`, `42809` (three of 18.6's five
wordings), `428C9`, `42601`'s `P` (a **1-based codepoint** index), and `2BP01` on three of
its five shapes: `DROP TABLE` with a dependent FK, `DROP COLUMN` with a dependent sequence
default, and `DROP SEQUENCE` for an identity column. The other two differ on purpose or
because of a gap. `DROP INDEX` over a constraint's index sends a **deliberately different**
`H` (18.6 suggests `ALTER TABLE … DROP CONSTRAINT`, which pglet refuses), and a
non-`CASCADE` `DROP SCHEMA public` sends **no `D`** where 18.6 lists every dependent.

A field pglet can't fill truthfully is **omitted**, never made up: `F`/`L`/`R` always
(there's no C source tree), the `Key (…)` description on an expression index, and the
`Failing row` on a check that runs before the row exists. Knowing a name isn't a reason to
send it: `t`/`c`/`n` go out only at the five variants 18.6's `errtable*` macros fill. The
wasm host sets `pg`'s property names
(`.detail`/`.hint`/`.position`/`.schema`/`.table`/`.column`/`.constraint`), which is what
pglite exposes.

**Gaps:** `P` outside `Error::position`'s arms, the near-name `H` on `42703` and the escape
`H` on a malformed Unicode escape's `22025`, the `D`+`W` pair on a jsonb `22P02` and the
`D`+`W` on its `22P05`, the `W` on COPY's and a bound parameter's errors, the `D` on a
`numeric` `22003`, the `DROP SCHEMA` dependent list, the `D` on a `CREATE TYPE … AS ENUM`
repeated label, and the `WARNING`/`25P01` notice for `COMMIT`/`ROLLBACK` outside a block
(`EDGES.md` row 76; the notice channel carries a message string with no severity or
SQLSTATE of its own). See `docs/errors.md`.

### `jsonb_build_object`, `IS [NOT] DISTINCT FROM`

Used by the spine's `cell_list`/audit. `IS [NOT] DISTINCT FROM` is null-safe (always
boolean). `jsonb_build_object` renders a boolean, number, text, `NULL` and `jsonb` value as
its JSON kind, and every other value as a JSON string of its text output. That matches
18.6's rendering for a `date`, `time`, `interval` and `uuid`, but not for a `timestamptz` /
`timestamp` (no `T`, no offset) or an array (a string, where 18.6 emits a JSON array):
`EDGES.md` row 306.

### Scalar `EXISTS` / `NOT EXISTS`

Non-correlated (`query_has_fact`, including a no-FROM projection) and **correlated**: the
orphan-fact `NOT EXISTS` and the cell-authz `EXISTS (… g.cell_id = c.id)` both run end to
end (outer-scope names resolve at eval time via `EvalEnv`, with alias-aware scoping). The
sub-plan runs to the end instead of stopping at its first row, so an error in a later row
is raised where 18.6 returns a result (`EDGES.md` row 320).

### `INSERT … SELECT` + `unnest($N)` in a SELECT projection

`put_fact_refs` runs end to end, a projection `unnest` expands into rows (with no FROM),
and the native parser handles the no-FROM `… SELECT … ON CONFLICT` shape directly.
**Divergence: the no-FROM restriction is real.** Only that shape lowers to
`Plan::ProjectUnnest`. With a `FROM`, the expression stays in a plain `Plan::Project` and is
`0A000 set-returning unnest() is only supported in a SELECT projection` as soon as a row
reaches it, for an **array** argument as much as any other, where 18.6 expands it. Over
`mwa(arr INT[])` holding `ARRAY[1,2]`, `SELECT unnest(arr) FROM mwa` and
`SELECT unnest(ARRAY[1,2]) FROM mwa` give `1`/`2` there and `0A000` here, and so do the CTE
and `VALUES`-derived-table spellings. With an empty table, both give `(0 rows)`. It's loud,
so no fixture fails over it (`EDGES.md` row 55's bullet). A **non-array** argument doesn't
reach this refusal: it's `42883` at plan time, matching 18.6.

### `generate_series` in `FROM`

`FROM generate_series(start, stop[, step]) [AS] g[(col)]` is a FROM item on the same node
the FROM-clause `unnest` uses (`Plan::FunctionScan`).

**What works:** the `integer` and `bigint` overloads, resolved the way 18.6 resolves them
over every argument-type combination we tested. That's two or three arguments of
`smallint`, `integer`, `bigint` or unknown type, where the wider of `integer` and `bigint`
that's written decides, and a string literal, a bare `NULL` or a bare `$N` takes the
overload's type (`generate_series(1, '3')`, `generate_series(1, $1)` inferring `integer`,
and `bigint` next to a `bigint`). It's `42883` when no signature fits, and `42725` when
neither type is written (two unknowns, two `smallint`s). The rows match 18.6: a negative
step, an empty series, a series that ends at the type's edge instead of wrapping, and a
NULL argument giving zero rows before a zero step's `22023`. The relation and column names
follow `chooseScalarFunctionAlias` (`generate_series` unaliased, the alias under `AS g`, the
listed name under `g(i)`), `g.*` and a non-sole `*` expand the one column, and the column's
declared type is the overload's on both wire formats. An argument is evaluated once per
execution, and a plan that `execute_bound` holds keeps it as an expression, so a bound
value is read on each run.

**The limit.** A series longer than **1 000 000 rows** is refused with `54000`, based on its
evaluated arguments, before any row is built. A FROM item's rows are materialized before
the node above reads them, as 18.6's tuplestore materializes them, but without spilling to
disk.

**What's refused** (`EDGES.md` row 321, with the measured cases):

- the `numeric` and timestamp series (`0A000`);
- an argument naming a column (an earlier `FROM` item's, which 18.6 reads row by row, or
  an enclosing query's) or holding a subquery (`0A000`), since the item is evaluated once
  with no row;
- an argument whose type the certainty reader doesn't give (`$1 + 1`, `nextval(…)`:
  `0A000`);
- `LATERAL`, `WITH ORDINALITY` and `ROWS FROM` (`42601`);
- `generate_series` in the select list or any other expression position (`0A000`).

A zero step, and a series past the limit, are refused under a `WHERE false` or `LIMIT 0`
that 18.6 doesn't run the series under (row 320). Two items with the same name are row
322's, and a `$N` compared with the series' column takes no type from it (row 288).
Fixtures: `generate_series/`; for the limit, the refusals' codes and the plan cache:
`crates/pglet/tests/generate_series.rs`, `crates/pglet/src/plan_cache.rs`.

### pg_catalog introspection (drizzle-kit push etc.)

These are the calls kysely's introspector makes (`EDGES.md` row 252): `pg_constraint` (22
of 28 columns, all five `contype` kinds including `n`), `pg_index` (15 of 21), `pg_enum`,
`pg_get_constraintdef(oid)`, `::regclass`, `col_description(oid, int4)` (always NULL, since
`COMMENT ON` is a no-op; `EDGES.md` row 15), `pg_get_serial_sequence(text, text)`,
`has_schema_privilege(text, text)` and `quote_ident(text)` (18.6's keyword table, not
pglet's lexer).

Partial: `pg_class` (9 of 34, with `relkind` `r` and `i` rows), `pg_namespace`
(`pg_catalog` + `public`), `pg_attribute` (12 of 25), and `pg_type` (10 of 32: every scalar
pglet models **plus** its array type and every user enum; `char` / `oid` / `name` carry
18.6's real `typelem` / `typarray` instead of a `0` that would claim they have no array
type). Complete: `pg_tables` / `pg_indexes` (`indexdef` re-emits an expression key and a
partial index's `WHERE`, as `pg_get_indexdef` does; `hastriggers` is computed from the FK
set on **both** sides, which is exactly 18.6's value for an engine without triggers).
`pg_constraint` / `pg_index` / `pg_enum` / `pg_tables` / `pg_indexes` bind their full
column list, so an unmodelled column is `42703` instead of a made-up zero-row `text`.

Not modelled, each because pglet lacks a type or model it needs (`int2vector`,
`oidvector`, `pg_node_tree`, an opclass and collation model): `pg_constraint`'s `conbin`,
`conpfeqop`, `conppeqop`, `conffeqop`, `confdelsetcols` and `conexclop`, and `pg_index`'s
`indkey`, `indcollation`, `indclass`, `indoption`, `indexprs` and `indpred`. Of the `reg*`
types only `regclass` is modelled, as a cast. The parser's intercept only matches
`regclass`, and every other spelling falls through to the type-name table and is `42704`
(`regtype`, `regproc`, `regnamespace`, `regrole` and `regoperator` checked).

`conkey` for a CHECK is in the predicate's **walk** order, as `pull_var_clause` yields it
(`CHECK (qty > 1 AND pid > 0)` is `{3,2}`, not `{2,3}`). `oid` and `"char"` are declared
types over `int4` / `text`, so the operand rulebook keeps them out of arithmetic and the
math functions while still allowing their comparisons. `conrelid + 1`, `- conrelid` and
`abs(conrelid)` are all `42883`, as on 18.6, and so are the same three reached **through a
scalar subquery** and **through an aggregate** (`max(conrelid) + 1`, `- max(conrelid)` and
`abs(max(conrelid))`). An unknown literal coerced to `"char"` is truncated. A rendered
`AND`/`OR` chain is N-ary, as `pg_get_constraintdef` writes it, and an `IN` list renders as
the `ScalarArrayOpExpr`/`OR`-chain node that `transformAExprIn` builds for it, in all five
of its shapes (`docs/deparse.md`; `EDGES.md` row 88 closed, row 90 the live residual). The
fold and assignment contexts are stricter in pglet (row 87).

`pg_indexes.indexdef` sends its key columns through `quote_ident`, whose **keyword half
doesn't match 18.6**. A column named `position` reads `(parent_id, "position")` on both
engines, while one named `action` reads `(parent_id, "action")` here against 18.6's bare
`(parent_id, action)`, and `pg_get_constraintdef` differs the same way
(`CHECK (("action" > 0))` against `CHECK ((action > 0))`). The predicate, both directions of
the disagreement, and how to re-measure its extent are in `EDGES.md` row 85 (f), which
owns the rule.

**Divergences:**

- An index's own name and its table's name are inserted **raw** into `indexdef`, where 18.6
  renders both through `quote_identifier`, so every name 18.6 quotes reads bare here. The
  cost varies: text that doesn't parse for some names, text that resolves a *different*
  relation for others, and for others only a spelling difference that a schema diff would
  flag (`EDGES.md` row 122).
- A sequence gets no `relkind = 'S'` row (pglet's `Sequence` has no OID).
- `pg_constraint.oid` is synthesized by position, so it shifts when a constraint is added
  or dropped.
- `public`'s namespace OID is hardcoded to 2200, where 18.6's is per database.
- An enum column's `attlen` follows 18.6's representation (4), not pglet's text storage.

### `information_schema`

`tables` (one row per user table, `table_type` always `BASE TABLE`), `columns` (with
`column_default` rendered in Postgres's spelling: `gen_random_uuid()` / `now()` /
`'success'::text` / `nextval(…)`, except that a sequence name needing quotes is written
bare inside the literal; `EDGES.md` row 286), `table_constraints` (CHECK / FOREIGN KEY /
**PRIMARY KEY** / **UNIQUE**), `key_column_usage`, `referential_constraints` (`delete_rule`
from `ON DELETE`, `update_rule` from `ON UPDATE`) and `constraint_column_usage`. All are
matched on the schema-qualified name, so a user table with the same bare name isn't
shadowed.

The **PK/UNIQUE rows** come from the catalog `Index`'s constraint marker
(`Uniqueness::constraint`, persisted). An index created for a declared `PRIMARY KEY` /
`UNIQUE` *is* the constraint, under the same name, while a standalone
`CREATE UNIQUE INDEX` is left out, which is the same split Postgres makes. A user-declared
`CONSTRAINT <name>` is reported under **that** name (all three spellings: table-level,
column-level, `ADD CONSTRAINT`), since the constraint's index is named after it. A
column-level `CONSTRAINT <n> CHECK (…)` keeps its name too. `key_column_usage` lists a
PK/UNIQUE's key columns in **constraint key order**, with `position_in_unique_constraint`
NULL (that column pairs an FK's columns with the referenced key). `constraint_column_usage`
lists them as the constraint's own table and columns. `referential_constraints` stays
FK-only, since it has no other meaning. A bound `$N` filter on a virtual-table column
(`WHERE table_name = $1`, the spine's `auth_tables_exist` probe) is typed from the catalog
view's column list (text), so a binary-protocol client doesn't see OID 0.

`sequences` (every sequence that isn't an identity column's, which is 18.6's
INTERNAL-`pg_depend` exclusion, read here from the owner column's identity flag) and
`check_constraints` (CHECK predicates plus the `NOT NULL`s as `<col> IS NOT NULL`) are
modelled, and `table_constraints` / `constraint_column_usage` include the `NOT NULL` and
CHECK rows. `columns` has `udt_name` (an enum column reports its type's name with
`data_type = USER-DEFINED`), `is_identity` / `identity_generation` / `is_generated`,
`character_maximum_length`, `numeric_precision` / `_radix` / `_scale` and
`datetime_precision` (a fact about the **type**: `6` for a bare `timestamp`, `0` for a
`date`, NULL for an array column and for every non-temporal one). An identity column's
`column_default` is SQL NULL, as 18.6's is.

`column_default` is rendered by the deparser's **catalog mode**, which prints what
`pg_get_expr` prints for the tree 18.6's parse analysis would have stored, not a faithful
rendering of pglet's own tree. That covers the cast folds, the constant labels, the
`unknown`-to-operand-type resolution, and `transformAExprIn`'s `IN` rewrite, all described
in `docs/deparse.md`. So `DEFAULT '5'::text` reads `'5'::text` (not `('5')::text`),
`DEFAULT -1` reads `'-1'::integer`, `DEFAULT gen_random_uuid()::uuid` reads
`gen_random_uuid()`, and a string literal takes the **column's** type name: the enum's own
for an enum column, and `character varying` for a `varchar(n)` one.

**Divergences:**

- `columns` reports a `char(n)` column as `character varying`. That's a type-system
  divergence, not a description one (`EDGES.md` row 95).
- It **renumbers** later columns' `ordinal_position` after a mid-table `DROP COLUMN` (PG
  leaves the gap).
- Every `information_schema` column pglet models as a **string** is declared `text`, where
  18.6's `RowDescription` carries the domain's **base** type. `check_constraints` is
  `[19, 19, 19, 1043]` there against `[25, 25, 25, 25]` here, so pglet reports neither the
  domain nor the type under it. The **numeric** columns aren't affected and agree: across
  the modelled views, the integer columns declare `int4` (23) on both engines (`columns`'
  `ordinal_position` / `character_maximum_length` / `numeric_precision` /
  `numeric_precision_radix` / `numeric_scale` / `datetime_precision`,
  `key_column_usage`'s `ordinal_position` / `position_in_unique_constraint`, and
  `sequences`' three `numeric_*`, checked over the wire).
- The four rendering classes named in `docs/deparse.md` § What is recorded, not
  reproduced: a nested implicit coercion, `CASE`'s indentation, an output function's
  normalisation, and an enum literal outside a `DEFAULT` (`EDGES.md` rows 90/91/93/94).

### `ALTER TABLE … ADD CONSTRAINT`

`CHECK` / `PRIMARY KEY` / `UNIQUE` / `FOREIGN KEY` (single and composite). Each one
validates the existing rows, then records and enforces the constraint (visible in
`table_constraints`). The full record is in § ALTER TABLE … ADD CONSTRAINT — details,
below.

#### ALTER TABLE … ADD CONSTRAINT — details

**A *declared* constraint name is unique per table across all four kinds**, matching
Postgres's single unique index on `pg_constraint (conrelid, conname)`. A `CHECK` /
`FOREIGN KEY` / `PRIMARY KEY` / `UNIQUE` name already taken by *any* of the four on that
table fails loudly with `42710` (`constraint "<n>" for relation "<t>" already exists`).
That applies to the inline `CREATE TABLE` spellings (table-level and column-level) as well
as `ADD CONSTRAINT` and `ADD COLUMN`. An **auto**-generated name is never refused, only
disambiguated (see the naming rule below).

Two FKs with the same name on one table are what this check exists to prevent, since a
later `DROP COLUMN` would drop **both**. The check alone doesn't make that state
impossible, though, so the DROP-time resolver keys on `(table, position)`, never on a
name. An auto-name never hits the declared-name check:
`ALTER TABLE t ADD FOREIGN KEY (a) REFERENCES p(id)` issued twice builds `t_a_fkey` +
`t_a_fkey1`, as Postgres does, and `ADD COLUMN … REFERENCES` and an unnamed CHECK claim
their names in the same pools as every other route. The check spans the kinds, so
`CONSTRAINT n CHECK (…), CONSTRAINT n PRIMARY KEY (…)` is `42710`. A duplicated
`(table_name, constraint_name)`, which is unique by construction in Postgres, would be
invisible to an introspecting client, and would dump to SQL that stock Postgres refuses
(`constraint "n" for relation "t" already exists`).

A PK/UNIQUE also occupies the **relation** namespace, because its index carries its name,
and `index_create` checks that one **first**. A collision with a table or index is `42P07`
(`relation "<n>" already exists`), and only then is the constraint namespace checked for
`42710`. A CHECK/FK is *only* in the constraint namespace, so a CHECK named after a plain
`CREATE INDEX` is accepted on both engines. The order across kinds matches Postgres too:
the CHECKs' names are resolved first, then the constraint indexes, then each FK in source
order, with its name checked **before** its parent relation. So a missing parent on the
first FK wins over a duplicate name on the second, and a duplicate name wins over a missing
parent on the same FK.

`PRIMARY KEY (cols)` / `UNIQUE (cols)` / single-column `FOREIGN KEY … REFERENCES …` via
`ADD CONSTRAINT` **work** (the `pg_dump` restore shape). They validate the existing rows
(`23505`/`23502`/`23503`), then create the same enforced unique index or FK that an inline
constraint creates. The `ONLY` modifier is accepted. The **declared constraint name is
kept** and becomes the index's name (Postgres names a constraint's index after the
constraint), so `information_schema`, `pg_indexes`, the `23505` message, `DROP INDEX`'s
`2BP01`, and the dump all agree on it. The `<table>_pkey` / `<table>_<cols>_key`
convention is only the default when no name is declared, and nothing *reads* it
(`Schema::primary_key_index` checks the marker instead).

**A declared name is checked; an auto-name is disambiguated.** Postgres never lets one of
its own generated names fail a statement, so when `<table>_<cols>_key` is taken,
`ChooseRelationName` adds a digit to the label (`t_a_key1`, `t_a_key2`, …), with every
candidate cut to 63 bytes the way `makeObjectName` cuts it (`docs/names.md` § An unnamed
index's name). `CREATE INDEX t_a_key ON t (b); ALTER TABLE t ADD UNIQUE (a);` succeeds on
both engines. The candidate has to be free in the relation namespace, the table's
constraint namespace, **and** among every constraint name in the schema
(`ChooseRelationName`'s `isconstraint` pass, which ends with `ConstraintNameExists`. That's
stricter than `pg_constraint`'s own index, because "the SQL standard says that constraint
names should be unique within schemas").

**A CHECK's or FK's auto-name follows the same split through a different function**,
`ChooseConstraintName`, and the two search *different* spaces. That difference can't be
seen in the SQL, and silently changes which names collide if you get it backwards.
`ChooseConstraintName` scans `pg_constraint (conname, connamespace)`: schema-wide, all four
kinds, and **only** `pg_constraint`. So a `UNIQUE` named `t_a_check` on **another** table
pushes this table's next auto-named CHECK to `t_a_check1`, while a plain
`CREATE INDEX t_a_fkey` (a relation, not a constraint) doesn't push an FK's auto-name at
all. The **declared**-name check is the per-relation `ConstraintNameIsUsed` ("deliberately
not the same test as `ChooseConstraintName` uses").

All of it goes through one `ConstraintNames` check at **execute** time: `mint_constraint`
for CHECK and FK, and `mint_constraint_index` for a PK/UNIQUE name that's also its index's.
The split is by *side*, not by kind. The parse-side shapes (`planner::ForeignKeyDef`,
`planner::CheckConstraintDef`) carry `Option<String>`, so declared vs auto can be
represented. The catalog-side shapes (`ForeignKey`, `CheckConstraint`) carry a plain
`String`, so nothing unnamed can reach the store, and the generated name is **persisted**.
A name is never reconstructed from the table. `ChooseConstraintName` searches the whole
schema across all four kinds, so
`CREATE TABLE q (a INT, CONSTRAINT r_a_check CHECK (a > 0)); CREATE TABLE r (a INT, CHECK (a > 0));`
names the second check `r_a_check1`. Searching only the table's own earlier checks would
give `r_a_check` and dump two `ADD CONSTRAINT`s under one name.

**The `CREATE TABLE` element order is part of the naming contract**, since a name is
generated from a position in a list. Postgres walks `stmt->tableElts` once
(`transformCreateStmt`), routing a column def and a table constraint onto the *same*
`cxt.ixconstraints`/`ckconstraints`/`fkconstraints` lists, so the constraints are collected
in **written** order across the two spellings. Routing them to separate lists and
concatenating would pair a persisted auto-name with the wrong definition:
`CREATE TABLE ro (FOREIGN KEY (a) REFERENCES p (id) ON DELETE CASCADE, a INT REFERENCES p (id) ON DELETE SET NULL)`
gives `ro_a_fkey` CASCADE on both engines. A query that only reads names can't see this,
so the fixture reads the `delete_rule` too.

**Which kind gets named first differs by route**, and neither order follows from the
other. `CREATE TABLE` names its CHECKs, then its constraint indexes, then its FKs
(`DefineRelation` attaches the CHECKs as the heap is created), while
`ALTER TABLE … ADD COLUMN` names its **index first**, then its CHECKs, then its FK. The
`ADD COLUMN` order comes from rescheduling, not from the pass numbers. Read off the
`AlterTablePass` enum it comes out backwards, since the passes run in ascending order and
`AT_PASS_ADD_CONSTR` (6) comes **before** `AT_PASS_ADD_INDEX` (9). Nothing final happens in
pass 6. `transformAlterTableStmt` schedules the inline `UNIQUE` straight into pass 9 as an
`AT_AddIndex`, while the CHECK and the FK arrive in pass 6 as `AT_AddConstraint`s that
`ATParseTransformCmd` only re-files into `AT_PASS_ADD_OTHERCONSTR` (10), CHECKs before FKs
because `transformAlterTableStmt` appends `ckconstraints` before `fkconstraints`.

The order is only visible when the declared name is the one the other kind would have
generated, and that's the case worth stating precisely. With a generic name the two routes
agree and both succeed (`ADD COLUMN c INT CONSTRAINT n UNIQUE CHECK (c > 0)` gives `n` +
`t_c_check` on both engines, and so does the `CREATE TABLE` spelling). Make the declared
name collide with the CHECK's auto base, `CONSTRAINT t_c_check`, and they split:
`ADD COLUMN` names the index `t_c_check` and the CHECK yields to it as `t_c_check1`, while
`CREATE TABLE t (c INT CONSTRAINT t_c_check UNIQUE CHECK (c > 0))` names the CHECK
`t_c_check` first, and the declared UNIQUE then collides with `42710`.

Redundant `PRIMARY KEY`/`UNIQUE` definitions **within one statement** are merged, not
rejected, as `transformIndexConstraints` does. `CREATE TABLE t (a INT UNIQUE, UNIQUE (a))`
and `a INT PRIMARY KEY UNIQUE` each build one index, the primary key is kept over a UNIQUE
it absorbs, and a named duplicate passes its name to an unnamed keeper
(`a INT PRIMARY KEY, CONSTRAINT myu UNIQUE (a)` gives one PRIMARY KEY called `myu`). Key
**order** is part of the definition, so `UNIQUE (a,b)` and `UNIQUE (b,a)` stay two indexes.
The merge only applies within one statement, so two separate `ADD UNIQUE (a)` statements
build `t_a_key` + `t_a_key1`. That's Postgres's scope as well ("XXX in ALTER TABLE case,
it'd be nice to look for duplicate pre-existing indexes, too").

**A composite FK via `ADD CONSTRAINT` works** (multi-column
`FOREIGN KEY (a,b) REFERENCES parent (x,y)`). `ADD COLUMN [IF NOT EXISTS]` (plus an inline
FK) and `DROP COLUMN [CASCADE|RESTRICT]` are also supported.

**`DROP COLUMN` tracks dependent constraints** (one resolver in
`executor/dependencies.rs`, shared with `DROP TABLE`, reproducing Postgres's `pg_depend`
edge kinds). The table's own `CHECK`s naming the column and its own child-side
`FOREIGN KEY`s over it are **auto-dropped** (an AUTO edge, no `CASCADE` needed). An FK that
*references* `table.column`, from another table **or from this one** (the
referenced-column edge is NORMAL either way), raises `2BP01` unless `CASCADE`. When one
constraint is on both sides of the column (`FOREIGN KEY (id) REFERENCES t(id)`), the AUTO
edge wins and the drop goes ahead, as in Postgres. The dependency check runs **before** the
index position-drift guard, so an FK-referenced column reports `2BP01` (the real cause)
instead of the guard's `0A000`. The guard itself stays (`0A000` if an index keys on the
dropped column or one after it, since pglet keys indexes by row position and doesn't
rebuild them on drop).

**Divergences:**

- A *referenced* column always has a covering unique index, so `DROP COLUMN … CASCADE` on
  one still fails the drift guard with `0A000` where PG succeeds (pglet can't rebuild the
  index the constraint's removal would orphan).
- `ALTER TABLE … DROP CONSTRAINT` isn't implemented. It's parsed and refused with `0A000`
  over a table that exists (`42P01` over a missing one, and the skip under `IF EXISTS`).
  That's also why a constraint index's `2BP01` suggests dropping the table instead of
  giving PG's hint.
- A column-level `CONSTRAINT <n> NOT NULL` loses its name, where PG 15+ materializes it as
  a named CHECK.
- `ALTER TABLE IF EXISTS <t> …` performs the action over a relation that exists, and skips
  with 18.6's `NOTICE` over a name nothing holds (`EDGES.md` row 15 keeps its residuals).

`ADD COLUMN … NOT NULL DEFAULT NULL` is `23502` like PG. The check reads the resolved
backfill values, not whether a DEFAULT is present, and `DEFAULT NULL` is normalized to *no*
default (18.6 stores no `pg_attrdef` row for a NULL constant). The same goes for
`ADD COLUMN … NOT NULL` with no default, `ALTER COLUMN … SET NOT NULL` over NULL rows, and
`COPY` of a `\N`. `ALTER COLUMN … ADD GENERATED … AS IDENTITY` reads no row and declares
nothing: it's refused with `55000` over a column not already declared `NOT NULL`, whatever
its rows hold, as on 18.6 (`ddl/add_identity_*`). `SET NOT NULL` first is the way in. A
store where an earlier build accepted it over a NULL still holds that row, which the
accelerated profile's `ORDER BY` drops (`EDGES.md` row 38).

### `CREATE SEQUENCE` / `ALTER SEQUENCE` / `GRANT` / `REVOKE` / `COMMENT ON` / `ALTER TABLE … OWNER TO`

Enough to get through a `pg_dump` preamble. `CREATE SEQUENCE` registers a named sequence
(honoring `START WITH`, `RESTART` and `OWNED BY`; `INCREMENT BY`, `MINVALUE`/`MAXVALUE` and
`CYCLE` are parsed and ignored, and every draw is `+1`; `EDGES.md` row 215, silent).
`ALTER SEQUENCE` honors `START WITH`/`RESTART`/`OWNED BY` and tolerates the rest of its
tail (see the Sequences row). `GRANT`/`REVOKE`/`COMMENT ON`/`OWNER TO` parse and succeed
as no-ops (pglet models no roles or comments). `SELECT pg_catalog.set_config(…)` parses
(a schema-qualified call) and echoes the value back. **`setval(…)` advances the
sequence** (see the Sequences row). `ALTER TABLE … ALTER COLUMN … SET/DROP DEFAULT`
changes the catalog default.

### `SET` / `SHOW` / `DEALLOCATE` / `DISCARD` / `RESET`

`DEALLOCATE [PREPARE] {name|ALL}` drops the session's named prepared statements (a `ColId`
name; `26000` if it doesn't exist; `ALL` leaves the unnamed slot alone; not transactional;
`25P02` in an aborted block). `DISCARD {ALL|PLANS|SEQUENCES|TEMP|TEMPORARY}` reports
18.6's tag for each. `ALL` is `DEALLOCATE ALL` + `RESET ALL` + the rest, and is `25001` in
a transaction block. `SEQUENCES` clears the session `currval` cache, and `PLANS`/`TEMP` are
no-ops. `RESET` takes the whole `reset_rest` (`ALL`, a dotted name, three fixed spellings,
one of which is `0A000` on both) and reports tag `RESET`. Those three are classified
through the lexer, so a spelling with a comment in front works; for `SET`/`SHOW` that's
`0A000` (`EDGES.md` row 23).

**The `SET` arguments pglet acts on are read on every route that writes them**: the
statement and `set_config`, with one decider each. A `search_path` naming a schema pglet
doesn't have is `0A000` where 18.6 accepts it (one naming none, `pg_dump`'s `''`
preamble, is accepted; row 66). A non-UTF-8 `client_encoding` is `0A000` too, `SET NAMES`
included. **A parameter whose value pglet holds fixed only takes that value**:
`TimeZone` (`UTC`), `DateStyle` (`ISO, MDY`), `IntervalStyle` (`postgres`) and
`standard_conforming_strings` (`on`) are `0A000` for any other value, through `SET`,
`SET TIME ZONE`, `set_config` and the native server's startup packet, where 18.6 honors
it. A reset is accepted, and `set_config` of `application_name` or `lock_timeout`, which
that function can't change, is `0A000` (row 69). A transaction-mode `SET` asking for
`READ ONLY` is `0A000`, matching `BEGIN`.

All five are intercepted **per statement** of a simple-query batch, on the native server
and in the embedded `Database::execute`. So `SET TIME ZONE 'UTC'; INSERT INTO t VALUES (1)`
runs the `INSERT` there, and a mid-batch `SHOW` returns its row in order. The wasm
bindings, and so the npm package, send every query string through
`Database::execute_bound`, which takes one statement and refuses that string with `42601`
(`EDGES.md` row 270). An extended-protocol `Parse` of `SET x; INSERT …` is `42601` like any
other multi-statement Parse, since that route skips the parser's own single-statement
check.

`SHOW server_version`/`server_version_num` give `18.6`/`180006`. **That's the release the
differential corpus is checked against: the dialect target, not a claim of feature
completeness.** It comes from the single table the startup `ParameterStatus` block renders,
which the server re-reports on change (`GUC_REPORT`). That includes the isolation level:
`SET TRANSACTION ISOLATION LEVEL` (`25001` after the block's first statement that takes a
snapshot, but not after a `SET`, `SHOW` or `SAVEPOINT`, or in a savepoint, as on 18.6;
`DEALLOCATE` and `DISCARD` take none here, row 237, and neither does an extended-protocol
`Parse`, row 239), `SET SESSION CHARACTERISTICS`, `default_transaction_isolation` and the
startup parameter. All are honored and reported by `SHOW transaction_isolation` /
`default_transaction_isolation`. A `REPEATABLE READ` / `SERIALIZABLE` block's first write
is refused with `40001` if the store moved past its snapshot, which is coarser than 18.6
(`docs/storage.md` § Isolation Guarantees, `EDGES.md` row 232). The `set_config` spelling
naming either of those levels is `0A000`.

`SHOW` reports the modeled params and `SHOW ALL` lists them. An unmodeled parameter **fails
loudly** (`42704`) instead of returning an empty result. That's Postgres's answer for a
name it doesn't know, and a divergence for every one it knows that pglet doesn't model
(`SET work_mem = '64MB'` gives `SET`, then `SHOW work_mem` gives `42704`). Three of those
are timeouts whose `SET` pglet accepts and doesn't act on: `statement_timeout`,
`idle_in_transaction_session_timeout` and `transaction_timeout`. That missing effect is
`EDGES.md` row 224. `lock_timeout` is modelled. `SET`, `SET LOCAL`, `RESET`, the startup
parameter, `SHOW` and `current_setting` read and write it (the `set_config` spelling sets
nothing, row 224) with 18.6's units, range and `22023`s (including for a list), and
transactionally: a `SET` in a block is undone by its `ROLLBACK`, and one after a savepoint
by a `ROLLBACK TO` that savepoint. It limits how long a statement waits for the writer.
Its default is pglet's `1s`, where 18.6's is `0`.

**Divergences:** the `SHOW` parameter name is read from the raw SQL text instead of
through the lexer (`parse_show_name` splits on whitespace and downcases), so a
`"…"`-quoted spelling keeps its quote characters and doesn't match. `SHOW "search_path"`
is `42704 unrecognized configuration parameter ""search_path""` here and reports the value
on 18.6, where `VariableShowStmt` takes a `ColId` and the scanner has already stripped the
quotes (`SHOW search_path` / `SHOW SEARCH_PATH` agree on both). See also `EDGES.md` rows
66, 67, 68, 69, 70, 224.

### `COPY … FROM STDIN` / `TO STDOUT`

**Text format** (the `pg_dump`/`psql` default) over the wire (the copy-in/copy-out
sub-protocol, simple **and** extended; Flush/Sync are ignored mid-copy-in, matching
Postgres) and via the embed API (`Database::copy_in`/`copy_out`). TAB-delimited, `\N` for
null, and the full backslash-escape table (octal/hex on input). `COPY FROM` runs the
**same** validation/constraint/FK/index path as INSERT, and takes untrusted bulk input
**without panicking**, failing loudly on what it checks: a wrong field count is `22P04`, a
NUL or illegal UTF-8 in the input or a de-escaped field is `22021`, a bad value is `22P02`,
`\N` into a NOT NULL column is `23502`, and an unknown option is `42601`.
`FORMAT csv`/`binary` fail loudly (`0A000`). The column list is a **set** in both
directions. `CopyGetAttnums` serves `TO` and `FROM` alike, so a column named twice is
`42701` on either, interleaved per item with the unknown name's `42703` (`docs/names.md`).

**Gaps:**

- `COPY (query) TO STDOUT` (copying a SELECT's output) is missing (`42601`).
- A `\r`/`\n` `DELIMITER` is `0A000` where PG splits (CR/LF `22023`, multi-byte `0A000`).
- The `HEADER` option only takes a boolean, so `HEADER MATCH` (which checks the header
  line against the column list) is `42601 invalid value for boolean option "header":
  MATCH` here, where 18.6 accepts it and ingests the row. That's the case in **both** the
  default text format and `FORMAT csv`, a loud over-rejection either way.

**Divergences in `COPY … FROM STDIN` input, some of them silent writes** (`EDGES.md`):

- a `\.` after data on its line, which 18.6 refuses as a marker not alone on its line and
  pglet reads along with the rest of its line (row 132);
- line endings, which pglet never compares with each other, splitting on LF alone (row
  134);
- a `CopyFail` message carrying an illegal UTF-8 byte, which gets `57014` with `U+FFFD` in
  the byte's place, where 18.6 gives `22021` (row 130).

`test_copy` compares its fixtures against real Postgres (a `@copy-to`'s bytes byte for
byte, a `@copy-from`'s `@verify` rows, and a refusal's SQLSTATE), and covers none of those
three rows. Each row's bullet says why.

### `pg_dump` plain-text restore

A real `pg_dump … | psql -f` replays into pglet. That covers:

- the `SET`/`SELECT pg_catalog.set_config(…)` preamble;
- `CREATE SEQUENCE`, `ALTER TABLE [ONLY] … OWNER TO`, `ALTER SEQUENCE … OWNED BY`, and
  `ALTER TABLE … ALTER COLUMN … SET DEFAULT nextval(…)`;
- `ALTER TABLE … ALTER COLUMN … ADD GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY ( … )`,
  the form pg_dump emits for every identity column. It creates the sequence under the
  declared `SEQUENCE NAME` and records the ownership edge, over a column the dump's
  `CREATE TABLE` declared `NOT NULL`, with `INCREMENT BY` / `NO MINVALUE` /
  `NO MAXVALUE` / `CACHE` / `AS <type>` parsed and ignored;
- `GRANT`/`REVOKE`/`COMMENT ON` (parsed and treated as no-ops where they have no effect);
- the inline `COPY … FROM stdin` data block;
- `ALTER TABLE … ADD CONSTRAINT … PRIMARY KEY` (enforcing).

An **unsupported column type still fails loudly** (`42704`); tolerance never swallows a
type error. A **sub-second precision** arrives in pg_dump's own mid-name spelling
(`ts timestamp(3) without time zone`, `iv interval(3)`) and is read back at the same width.
The round-trip checks that on the *catalog* as well as the rows, because pg_dump's `COPY`
data is already rounded, so `datetime_precision` is the only visible thing a dropped `(p)`
changes. It's a round-trip differential against live Postgres (`test_pg_dump`).

**Divergence: a dump of a database with an *enum column* restores nothing but the type,
because of two independent refusals.** The first is `ALTER TYPE public.mood OWNER TO
postgres`, which `pg_dump` emits for the user type and `parse_alter` refuses because its
dispatch only takes `SEQUENCE` or `TABLE` (`42601`; the `ALTER TABLE … OWNER TO` spelling
next to it succeeds). The second is the column type. `pg_dump` writes a user type
qualified and a built-in one bare (`m public.mood` next to `id integer`; `format_type.c`
special-cases the built-in OIDs before the branch that qualifies, and the
`search_path = ''` preamble makes every user type non-visible), and the parser's type
position only takes one bare identifier. So the column type is `42601`, a **supported**
type failing with the wrong code, and the `CREATE TABLE` takes its `COPY` block, its
`PRIMARY KEY` and everything after it down with it. It errors per statement but is
**silent per process**: without `ON_ERROR_STOP` (psql's default), psql exits 0 over a
database holding the type and nothing else. The control is the same table with a `TEXT`
column, dumped from a database fresh from its own `CREATE DATABASE`: it has neither
statement and restores its rows, with exit 0 either way. Fixing it needs both parts fixed.
`test_pg_dump`'s schema builds no enum column; `EDGES.md` row 113.

**The other direction doesn't work: `pg_dump` can't read a pglet server.** Its
`SELECT pg_catalog.pg_is_in_recovery()` is `0A000`, and it exits `1` (`EDGES.md` row 44).
`test_pg_dump` dumps *from* Postgres, so it doesn't hit this, and a pglet store is exported
with `pglet_server dump`.

### Native dump emitter

`Database::dump_sql()` (embed) and `pglet_server dump --data-dir <dir>` walk the catalog
and produce portable `CREATE TABLE …; COPY … FROM stdin; …; ALTER TABLE … ADD CONSTRAINT …`
SQL. A column's declared modifier goes out with it (a `numeric(p,s)`'s scale, a
`varchar(n)`'s length, and a `timestamp(3)`/`interval(3)`'s sub-second precision), since
each is **re-applied by the restore**, so dumping any of them bare would silently change
stored values on reload. For the last one, the round-trip checks with a *write* through the
restored catalog, since the rows alone can't show it (they're already rounded at the
source).

What works is the **emitter**. Whether its SQL then loads into **both** stock Postgres and
back into pglet isn't settled. A database built by pglet's own accepted statements (a
catalog state its DDL reaches, or a row value written into one of its tables) can still
fail to export at all, or dump to SQL that one leg refuses, against the dump invariant
`docs/sequences.md` states over the catalog and the rows. Those cases are rows in
`EDGES.md`, not here:

- a type name the emitter can't spell, where `dump` exits `1` having emitted nothing (row
  32);
- a zero-column relation, refused on **both** legs (row 64);
- a constant cast inside a stored expression (row 51);
- an invalid enum label as a column `DEFAULT`, which the dump renders as `DEFAULT 'zz'` and
  18.6 refuses at `CREATE TABLE` with `22P02` (row 47);
- a written enum cast at any persisted-expression position (row 104);
- a `CHECK` or partial index over two different enum columns (row 115);
- a NUL in SQL text passed to an in-process API: the dump refuses some stored NUL row
  values with `22021` and writes a NUL in catalog text raw, and a `psql -f` restore of that into
  18.6 exits `0` without restoring all of it (row 133);
- the same NUL inside a **jsonb** string. The reader refuses it wherever jsonb text is
  read, but a jsonb-producing expression that **wraps** a `Value::Text` instead of parsing
  it as jsonb text still gets it in (`jsonb_build_object`, over a value that ordinary SQL
  through `Database::execute` stores as easily as a bound one), and so does a bound
  `Value::Jsonb`. The dump **refuses to export** that store (`22021`,
  `refuse_unreadable_values`) instead of writing the NUL as jsonb's `\u0000` escape, which
  18.6's `COPY` refuses with `22P05`, restoring the table with none of its rows at exit `0`
  (row 133);
- in a store written by an earlier build, a NULL left in a column that
  `ALTER COLUMN … ADD GENERATED … AS IDENTITY` was accepted over (it's refused with
  `55000` now). Its dump recreates the column `NOT NULL`, both legs refuse its `COPY` with
  `23502`, and the table restores with none of its rows (row 38).

The full record is in § Native dump emitter — details, below.

#### Native dump emitter — details

Reloading into another engine only works as well as the catalog states the engine refuses
to reach. Two constraints with the same name on one table, re-emitted faithfully, would load
back into pglet and be refused with `42710` by stock Postgres, so reloading into pglet alone
can't catch such a state. The data section reuses the `COPY TO` text encoder, and
constraints and indexes come after the data, as in pg_dump.

A **plain sequence emits three statements at three points**, because its start, its owner,
and its counter are three separate facts: `CREATE SEQUENCE … [START WITH <start>]` in the
schema block, `ALTER SEQUENCE … OWNED BY t.c` **after** the tables (it names one, so it
can't go in the sequence block), and `SELECT setval('<n>', <last_value + 1>, false)` after
the data. An **identity** sequence emits **two**, and shares neither of the first two. One
is `ALTER TABLE … ALTER COLUMN … ADD GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY
(SEQUENCE NAME <n> [MINVALUE <m>] [START WITH <s>])` after its table. That creates the
sequence and records the ownership edge in one statement, over the column its
`CREATE TABLE` declared `NOT NULL`, so a `CREATE SEQUENCE` next to it would be `42P07` on
restore and an `ALTER SEQUENCE … OWNED BY` next to it `0A000`. The other is the same
trailing `setval`.

Rendering an identity column as an ordinary `nextval`-defaulted one is the kind of defect
**the rows can't show**. The two hold identical values, so a round-trip that compares rows
stays green while `GENERATED ALWAYS` stops rejecting user-supplied ids (`428C9`), and all
three refusals that tie the column to its sequence (`ALTER COLUMN … DROP DEFAULT` `42601`,
`ALTER SEQUENCE … OWNED BY` `0A000`, `DROP SEQUENCE` `2BP01`) disappear across pglet's
*own* round-trip, on both restored copies. So the guard is a set of refusals checked on the
source and on both legs, next to the rows. Only the *auto*-nameable options are left out
(`INCREMENT BY` / `NO MAXVALUE` / `CACHE` / `AS <type>`, which pg_dump writes and pglet
models none of). `SEQUENCE NAME` is **always** emitted, even when it equals the
`<table>_<column>_seq` the restore would generate. The two match in the common case, and a
dump that leaves it out is fine until the first database whose identity sequence has some
other name. At that point the trailing `setval` names a sequence the restored catalog
doesn't have, and the counter silently restarts at 1.

**No `CREATE TABLE` names a sequence.** An identity sequence is created after its table,
so an inline `DEFAULT nextval(…)` can't count on every sequence existing before every
table, and *every* `nextval` default is moved into an
`ALTER TABLE … ALTER COLUMN … SET DEFAULT` after the whole schema block. That removes the
ordering requirement instead of patching the two ordinary catalogs that would hit it: a
sibling column of the same table drawing from its identity sequence, and a column of a
table earlier in name order drawing from a later table's sequence, both accepted by both
engines. pg_dump gets to the same statement from the other side. It prefers the inline
form and falls back to this one when its dependency sort finds the loop an owned sequence
creates, which is why it moves a `SERIAL`'s default out too.

Two renderings are conditional, and both exist because a *catalog state pglet's own DDL
could reach* would otherwise emit SQL that doesn't load. An `OWNED BY` whose owner the
catalog can't resolve is **skipped**. A sequence that outlived its owner would otherwise
name a relation or column that doesn't exist, which stock Postgres refuses, and so does
pglet's own restore for the column routes. Skipping instead of failing loudly is
deliberate: such a sequence is effectively unowned, and an `Err` there would block the
export of the whole database just to keep an edge that points at nothing.

**The routes known to reach that state are closed.** A dropped table and a dropped SERIAL
column take their owned sequence with them (`executor::dependencies`' AUTO edge), and an
`ADD COLUMN … SERIAL` that fails after its sequence would have been registered registers
nothing, because the registration is staged behind the backfill validation. So nothing in
live SQL reaches the branch, and both its guards (`pglet`'s
`a_dangling_sequence_owner_is_not_emitted` and `test_dump`'s `seed_sequence_edge_cases`)
build the state on the catalog by hand, and check each of those routes from the other side
(the sequence is *gone*). **A guard whose only seed is a defect stops testing the emitter
once the defect is fixed.**

A start or resume point below `1`, which is reachable because pglet doesn't model
`MINVALUE`, emits the explicit `MINVALUE` that floor needs plus an explicit `START WITH`
(given a minimum, `init_params` seeds an omitted `seqstart` from it). The floor is a
property of the *values*, not of the statement that declares them, so it goes into the
identity option list too. `GENERATED ALWAYS AS IDENTITY (START WITH 0)` and
`setval(<an identity sequence>, -5)` are both accepted by pglet, and without it 18.6 gives
`START value (0) cannot be less than MINVALUE (1)`. `MINVALUE` is parsed and ignored there
on pglet, so it does nothing on the self-restore and matters on the Postgres one. A single
folded `CREATE SEQUENCE … START WITH <counter + 1>` would restore the resume point and
silently lose the declared start, so the restored database would handle a bare `RESTART`
differently from the original. The `is_called = false` spelling is required, not a style
choice. The `true` form is `setval(n, last_value, true)`, and for a sequence that never
advanced, `last_value` is `start - 1`, which stock Postgres rejects as out of range
(`22003`).

**A sequence name reaches the script in two grammars, and both must name the same
relation.** Three sites are identifier positions (`CREATE SEQUENCE`,
`ALTER SEQUENCE … OWNED BY`, `SEQUENCE NAME`), and two are string literals inside a
`regclass` cast (`nextval('<n>'::regclass)`, `setval('<n>', …)`). A literal is read back by
`SplitIdentifierString`, which **downcases** an unquoted part and rejects interior
whitespace. So a name that isn't a safe bare identifier carries its *identifier* quoting
inside the string (`nextval('"Odd_a_seq"'::regclass)`), which is pg_dump's own `fmtId`
approach of quoting only when needed. The *string* side is itself **two** grammars:
`quote_string_literal` emits the escape-string `E'…'` form for any name containing a
backslash (the one byte whose meaning depends on `standard_conforming_strings`), so
`CREATE SEQUENCE "a\b"` dumps as `nextval(E'"a\\b"'::regclass)` and `setval(E'"a\\b"', …)`,
while the identifier positions take that backslash raw. Anything reading the emitted script
has to know all three. A check that scans for `nextval('` alone passes **without testing
anything** over every `E'` default, on correct and orphaned dumps alike.

Emitting the raw name would fail **partially**. An ordinary
`CREATE TABLE "Odd" (a SERIAL, v TEXT)` would dump to a script where stock Postgres takes
the table and every row and drops only the column default and the counter, so the identity
silently renumbers from 1 into a collision. Meanwhile a reader taking the literal's bytes
verbatim would restore with zero errors (a DDL `DEFAULT nextval('x')` auto-creates `x`, so
the wrong name creates a second sequence). The reader applies the same grammar, including
its **downcase of an unquoted part**. That's the same fold `Token::name` applies in an
identifier position, so `CREATE SEQUENCE MySeq` and `nextval('MYSEQ')` name one relation,
while `nextval('"MySeq"')` names another.

The deliberate divergences, which `reduce_name_literal` lists:

- There's no `NAMEDATALEN` truncation, where `SplitIdentifierString` truncates every part,
  quoted ones included (self-consistent here, since the lexer doesn't truncate either).
- `\v` is missing from the scanner's space set, so `nextval(E'\x0Bs')` resolves `s` on 18.6
  and is `42P01` here, while the neighbouring `\x0C` resolves on both.
- An all-digit literal, **or a lone `-`**, is a name here and an **OID** to `regclassin`,
  which runs `parseDashOrOid` (named for both inputs) before `stringToQualifiedNameList`.
  `nextval('12345')` gives `XX000 could not open relation with OID 12345` and
  `nextval('-')` gives `XX000 could not open relation with OID 0` there, and each is
  `42P01` here. A dump can't produce either, since `quote_name_literal` quotes any name
  that isn't a safe bare identifier, and neither a digit string nor `-` is one.

**A part's quoting decides whether it folds, in the reader as everywhere else.**
`nextval('"PUBLIC".s')`, `setval('"PUBLIC".sq', 7)` and `DEFAULT nextval('"PUBLIC".s2')`
are `42P01`, as on 18.6. Unquoting the parts while discarding that bit would advance a
sequence, move a counter and auto-create one.

Whether an index is re-emitted as `ADD CONSTRAINT … PRIMARY KEY|UNIQUE` or as
`CREATE [UNIQUE] INDEX` is read from the catalog's **constraint marker**, not re-derived
from the index's name. A deliberately standalone `CREATE UNIQUE INDEX t_pkey` stays an
index instead of being upgraded into a constraint on restore, and a declared
`CONSTRAINT my_pk PRIMARY KEY` is re-emitted under **that** name, not the `<table>_pkey`
convention. An unrenderable type, default or expression **fails loudly** instead of
emitting wrong SQL. The expression renderer is **exhaustive by construction**: every
unrenderable `Expr` variant declines by name, so a new variant is a compile error there
instead of silently falling into a catch-all. (It reached the renderer by being
*persisted*, and one unrenderable expression aborts the dump of the **whole database**, not
just its table.)

The scalar-function families all render (`MathFn`, `StringFn`, `VariadicFn`, `Extract`,
`Builtin`, `jsonb_build_object`, `= ANY`/`ALL`), each emitting the spelling that
**re-parses to the identical variant**, not just an equivalent one. A stored expression is
compared structurally by the partial-index implication check, so rendering
`POSITION(sub IN str)` as `strpos(str, sub)` would restore as a different `Expr` and
silently change which queries the restored index serves. A corpus-driven test checks that
round-trip property over every renderable variant and render arm.

**Divergence:** a stored expression that Postgres would have rejected at DDL time doesn't
reload into stock Postgres. pglet doesn't enforce IMMUTABLE for an index expression or
partial predicate (Postgres gives `42P17`), so a dumped
`CREATE INDEX … ((concat(a, b)))` or `… WHERE ts < now()` is refused there. An `extract()`
whose field is a non-literal expression (pglet accepts one, and Postgres's `extract_arg`
grammar can't write one) renders back to pglet's comma form. Both reload into pglet.
Differential against Postgres plus self-reload (`test_dump`).

### LISTEN/NOTIFY

`42601`, a syntax error, not the `0A000` the `CREATE`/`DROP` dispatch gives, because the
lexer doesn't know the keywords (the fuz realtime layer uses app-level notify, not
LISTEN/NOTIFY).

### Object kinds the `CREATE` / `DROP` dispatch does not name

Each dispatch (`parser/ddl.rs`) keys on the token after the keyword and refuses everything
else with `0A000 not implemented: only CREATE TABLE / INDEX / SEQUENCE / TYPE / SCHEMA /
EXTENSION are supported`, or the `DROP` equivalent. Cases we checked: `CREATE DOMAIN`;
`DROP DOMAIN`, including `IF EXISTS` (the branch is chosen before the `IF EXISTS` tail is
read); `CREATE VIEW` plain, `OR REPLACE`, `MATERIALIZED` and `TEMP`; `DROP VIEW`,
including `IF EXISTS` and `MATERIALIZED`; `DROP TRIGGER`; `DROP FUNCTION`; and
`CREATE TEMP TABLE`, which shows the branch keys on the token, not the object kind. A
column declared with a domain's name is a separate refusal from the type resolver,
`42704`. `PLAN.md` § Non-goals lists `domain` among the types left out and gives it
`42704`. That's the code for *using* it; the DDL statement's own code is the `0A000` above.
See `EDGES.md` row 99.

## Known divergences (`EDGES.md`)

`EDGES.md` is the ledger: one row, with a stable number, per recorded divergence that the
differential harness doesn't cover (struck through when closed, grouped for triage), each
backed by a full bullet. A reference like "row 47" points into that file.
