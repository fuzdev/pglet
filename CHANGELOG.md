# pglet changelog

Covers `@fuzdev/pglet-wasm`, the npm package published from this repo, at the
`Cargo.toml [workspace.package]` version.

## 0.1.1

- fix: a `$N` compared with an aliased join column under `FOR SHARE` / `FOR UPDATE` is
  typed from the column, not `text`
- feat: `pg_column_size`, exact where pglet knows the stored bytes, `0A000` elsewhere

## 0.1.0

- first publish of `@fuzdev/pglet-wasm`: the in-memory wasm engine with the
  `create_pglet_db` adapter, a kysely dialect at `./kysely`, and drizzle support
  through its `node-postgres` driver
