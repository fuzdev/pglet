# pglet changelog

Covers `@fuzdev/pglet-wasm`, the npm package published from this repo, at the
`Cargo.toml [workspace.package]` version.

Pending changes go under `## Unreleased`, whose next line declares the release's level
(`<!-- bump: patch|minor|major -->`). `npm run publish:npm -- --wetrun --bump <level>`
requires `<level>` to repeat it and a non-empty section, then stamps the section as the
released version under a fresh `## Unreleased` declaring a patch.

## Unreleased
<!-- bump: patch -->

- feat: `WITH ORDINALITY` on a `FROM`-clause `unnest(…)` or `generate_series(…)`: a last
  `bigint` column numbering the rows from 1, named `ordinality` unless the alias list names
  it — `unnest($1::text[]) WITH ORDINALITY AS h(hash, ord) … ORDER BY h.ord` (was `42601`)
- feat: `to_regclass(text)`, answering the relation's name as `::regclass` does (`text`, where
  PostgreSQL answers a `regclass`), or NULL where no table, index or sequence answers;
  `to_regclass('public.t') IS NOT NULL` is the existence probe (was `0A000`)
- feat: a `SELECT` with no `FROM` takes a `WHERE`, `GROUP BY` or `HAVING` over its one row —
  `INSERT INTO t SELECT 1, 'x' WHERE NOT EXISTS (…)` writes its row or none (was `42601`);
  a bare `SELECT *` with no `FROM` is `42601`, as in PostgreSQL (was `0A000`)
- feat: `ALTER TABLE … ALTER COLUMN … DROP NOT NULL` (was `0A000`); a primary-key column is
  refused `42P16` and an identity column `42601`, as in PostgreSQL
- feat: `LATERAL (SELECT …)` in `FROM` — `CROSS JOIN LATERAL`, the comma form, `JOIN LATERAL …
  ON` and `LEFT JOIN LATERAL … ON`, the subquery naming the items before it and run once per
  row of them (was `42601`); `LATERAL` before a function or a `VALUES` list, or under a
  `RIGHT`/`FULL` join, is `0A000`
- feat: `FOR UPDATE NOWAIT` (and the other strengths') answers as a plain locking read does
  (was `0A000`); in a statement that writes, beside a locking item without it, or in a `DO`
  body it stays `0A000`
- feat: `FOR UPDATE SKIP LOCKED` (and the other strengths') answers as a plain locking read
  does (was `0A000`); the package has one connection, so nothing is ever skipped
- feat: `ALTER TABLE … RENAME TO` (was `0A000`): the table's rows, indexes, constraints,
  referencing foreign keys and owned sequences follow it, keeping their own names
- feat: `ALTER TABLE … RENAME [COLUMN] … TO …` (was `0A000`): the column's `CHECK`s, index
  keys and predicates, and foreign keys on either side follow it
- feat: `ALTER TABLE … ADD CHECK … NOT VALID` and `VALIDATE CONSTRAINT` (was `42601`); a
  store written by an earlier version is refused at open (store format `PGMW`)
- feat: an aggregate or a window function in a `SELECT` with no `FROM` — `SELECT count(*)`,
  `INSERT INTO t SELECT max(…)` — answers over one row (was `0A000`)
- fix: `'s'::regclass` names a sequence, as `to_regclass` does (was `42P01`)

## 0.2.1

- feat: `pg_advisory_xact_lock_shared`, over a `bigint` or two `integer`s, answering where
  `pg_advisory_xact_lock` does and taking the same lock. On the package's one connection
  every call is granted; across connections a shared and an exclusive holder of a key wait
  on each other as on Postgres, and two shared holders wait on each other too, where
  Postgres grants both. `pg_try_advisory_xact_lock_shared` is still refused `0A000`

## 0.2.0

- the package no longer claims Deno support: the README and the `db.js` and `index.d.ts`
  comments name Node and Bun as the runtimes the Node entry serves
- the statement-depth limit is 1 300 levels (was 2 000): a statement nested deeper is refused
  `54001`. Re-measured on Node's V8 under both tiers and on Bun's JavaScriptCore, the
  shortest stack edge (Bun's, 2 001 levels) left 2 000 too little margin; 1 300 is under
  two thirds of it
- feat: `ALTER TABLE … DROP CONSTRAINT [IF EXISTS]` for foreign keys and `CHECK`s, alone or
  in a list; a primary key, `UNIQUE` or `NOT NULL` constraint is still refused `0A000`
- fix: `ALTER TABLE t* …` (the inheritance star) no longer reports success and does nothing
- feat: session-level advisory locks — `pg_advisory_lock`, `pg_advisory_unlock` and
  `pg_advisory_unlock_all`, over a `bigint` or two `integer`s, counted per key and held until
  the last unlock or the session ends; an unlock of a key not held answers `false`. A call
  now answers in the statement's own select list over any `FROM`, `WHERE` or CTE, once per
  row, `pg_advisory_xact_lock` included. kysely 0.28.19's `Migrator`, whose stock Postgres
  adapter takes its lock this way, runs through `@fuzdev/pglet-wasm/kysely`. The `try_`
  forms are still refused `0A000`
- feat: `set_config('lock_timeout', value, is_local)` applies as `SET` / `SET LOCAL` do
  (was refused `0A000`)
- the dialect target is PostgreSQL 18.7 (was 18.6): `server_version` reports `18.7` and
  `server_version_num` `180007`, and `right(s, -2147483648)` answers the empty string, as
  18.7 does (was the whole string)

## 0.1.2

- fix: on Node, Deno and Bun, a `transaction()` called while another one's callback runs
  queues behind it instead of being rejected; only a call from inside a running callback is
  refused, as nested. `fork()` and `close()` called from outside a running callback no longer
  reject either: the fork branches after that transaction ends, and the close waits for it —
  and for any `db.query()` the callback still makes — before freeing. Called from inside one,
  `fork()` and `close()` still reject `25001` rather than deadlock. The browser entry, which has no
  `AsyncLocalStorage`, still refuses all three while any callback runs
- the license is now 0BSD (was MIT)

## 0.1.1

- fix: a `$N` compared with an aliased join column under `FOR SHARE` / `FOR UPDATE` is
  typed from the column, not `text`
- feat: `pg_column_size`, exact where pglet knows the stored bytes, `0A000` elsewhere

## 0.1.0

- first publish of `@fuzdev/pglet-wasm`: the in-memory wasm engine with the
  `create_pglet_db` adapter, a kysely dialect at `./kysely`, and drizzle support
  through its `node-postgres` driver
