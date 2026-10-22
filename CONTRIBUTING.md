# Contributing to pglet

See [fuz.dev/contributing](https://www.fuz.dev/contributing).

The [Code of Conduct](https://github.com/fuzdev/.github/blob/main/CODE_OF_CONDUCT.md)
applies to all Fuz projects.

## Before you run the tests: they drop a schema in the database they connect to

**Every task that runs the `test_fixtures`, `test_dump` or `test_copy` binary against the
oracle resets it**, by issuing `DROP SCHEMA public CASCADE; CREATE SCHEMA public` against
the database those binaries connect to — the `postgres` database on `127.0.0.1:5432`
unless `PGLET_PG_URL` names another. Anything you keep in that database's `public` schema
is destroyed. Among the gates those are `test:fixtures`, `test:fixtures:profile-lock`,
`test:dump` and `test:copy` (read the set off `deno.json` rather than memorising it).
`--profile-lock` only narrows the fixture list, so
`cargo xtask gates --only test:fixtures:profile-lock`, the obvious cheap subset, drops the
schema too, and so does the non-gate `deno task test:fixtures:update`. `test:pg-dump` drops
tables of fixed names in the same database.

**The knob is `PGLET_PG_URL`.** Set it to a libpq `postgres://` URL (or a `key=value`
conninfo string) naming a Postgres you are willing to lose the `public` schema of. The
cleanest target is a disposable *cluster* — a container, or a second cluster on another
port — because that keeps the default `postgres`/`postgres` names the fixture corpus
records (see the limit below):

```sh
PGLET_PG_URL=postgres://postgres@127.0.0.1:5555/postgres cargo xtask gates
```

A scratch database inside your own cluster works too, at the cost of the two fixtures
named below:

```sh
createdb pglet_scratch
PGLET_PG_URL=postgres://postgres@127.0.0.1:5432/pglet_scratch cargo xtask gates
```

Left unset, the harness connects exactly where it always has. A value that does not parse,
or that omits the user or the database name, fails loud naming the variable — it does not
fall back to the default, because a typo silently dropping the `public` schema of your
`postgres` database is the accident this knob exists to prevent.

**The knob's one measured limit is the session identity.** A few fixtures select
`current_user` / `current_database()` and compare against a snapshot holding the default
spellings, so an oracle whose **user or database name** is not `postgres` reports those
fixtures' expected files as stale — measured: `session/session_database_identity` and
`session/default_session_functions_are_stored`, against a scratch database, with the other
Postgres gates green. Moving the host, the port or the whole cluster (a container, or a
second cluster on another port) costs nothing. `test_fixtures` says so at startup, and
**refuses `--update`** outright when either name has moved — regenerating there would commit
your machine's spelling into the shared corpus.

## The Postgres oracle

The differential harness compares pglet against a real Postgres, so the gates that compare
against it need one running:

- **Postgres 18.x**, by default on `127.0.0.1:5432` — the default oracle connection and
  every pglet server the tests spawn are addressed as `127.0.0.1`, not `localhost`
  (`crates/pglet_tests/src/lib.rs`). The browser harness is the exception: its local HTTP
  server on 8001 is reached over `localhost`.
- A **superuser** and a **database** it may reset, reachable over TCP — by default the
  `postgres` superuser and the `postgres` database, or whatever `PGLET_PG_URL` names.
- **No password prompt.** The default connection string carries no password field
  (`trust` auth, or a `~/.pgpass` entry). A password *may* be spelled in `PGLET_PG_URL`,
  and it reaches the `tokio-postgres` clients and the `psql`/`pg_dump` shell-outs, which
  take the conninfo positionally — but **not** `SimpleProbe`, the hand-rolled frontend
  that reads the simple-query declared types, which speaks trust auth only. Against a
  server that demands authentication it fails loudly, naming the variable and the method
  it was asked for, and takes the fixture harness's text-format leg with it. So: a
  password-free connection for this user, whatever the URL.

`PGLET_PG_URL` is the repo's one oracle knob and every *harness* site that reaches the
oracle reads it. What does not read it, by name: the pglet servers the tests spawn (a
dynamically chosen loopback port, any credentials accepted); `cargo xtask measure`, whose
quiet-box measurement set makes an administrative connection to `127.0.0.1:5432` from its
own constant to create and drop its own scratch database; and `benches/exit_proof.ts`,
which has a **second** URL knob of its own, `PGLET_EXITPROOF_PG_URL` — opt-in, naming the
real Postgres its production-ceiling leg runs against and whose database it drops and
creates, and set by `cargo xtask measure` to that run's own scratch database.

No shell-out call site sets `PGPASSWORD` itself: a password lives in one place, the
conninfo the config renders. If your oracle authenticates by password rather than by
`trust`, spell that password in `PGLET_PG_URL` — `psql` and `pg_dump` are given no other.

The other ports in play: 5433 for the dev server (`cargo run -p pglet_server`),
5434–5499 for servers the tests spawn, and 8001 for the browser tests.

## Running the checks

- `deno task check` — the hermetic bar: clippy with `-D warnings`, a
  `--no-default-features` `cargo check`, clippy over `pglet` and `pglet_wasm` for the
  wasm target with the wasm build's features, default, stripped and with the opt-in
  `opfs` feature, plus a host clippy of `pglet_wasm` with `opfs` on, its tests included
  (`check:wasm-features`), `cargo fmt --check`, the rustdoc
  broken-link check (`check:docs`, run with `--document-private-items` because most of
  the engine is private items; `private_intra_doc_links` and `redundant_explicit_links`
  stay at warn by choice), `deno check` of the release scripts (`check:scripts`, which
  nothing else typechecks before a release runs them), and the release audit. Needs no
  Postgres and runs anywhere.
  The narrower `cargo clippy --workspace` you are likelier to type covers neither tests
  nor benches and fails on nothing, so it passes locally where `check` does not.
- `cargo xtask gates` — the full gate set, in a fixed order. Run it
  **from the repository root**: the harness builds and then execs
  `target/debug/pglet_server` by relative path. It takes an exclusive lock, so a
  second concurrent run refuses, naming the holder's pid where the lock file records it,
  and it writes each task's output to a per-task log under `target/gates/`, printing each
  task's status and elapsed time in its closing summary.
  `--check` compares the run's counters against `tests/gates_baseline.txt`.

The gate set's prerequisites, read off `deno.json`'s task bodies: `deno` throughout
(every gate is a `deno task`) and `cargo`, with `check` also wanting the `clippy` and
`rustfmt` rustup components and the `wasm32-unknown-unknown` target; **Rust 1.99.1**,
which `rust-toolchain.toml` pins together with those two components and that target — the
wasm size gate is calibrated against that toolchain, so a different one moves `size:check`
(`Cargo.toml`'s `rust-version`, 1.89, is the declared floor, not the toolchain the gates
run); a live Postgres for the gates that compare against it (`test:fixtures`,
`test:fixtures:profile-lock`, `test:dump`, `test:pg-dump`, `test:copy`,
`test:keywords`); `wasm-pack` for the two wasm gates, which downloads `wasm-opt` into
its cache on the first `wasm:build`; `gzip` for
`size:check`, which measures the gzipped wasm on the `--check` path too and **throws**
rather than skipping without it; `git` for `size:check --update` **only**, which stamps
the commit into the baseline it writes; and the `psql`/`pg_dump` client binaries
(`test:dump` runs `psql`, `test:pg-dump` both), which those two gates **skip, exit 0**
without. A cold clone also fetches cargo's registry dependencies and deno's jsr/npm
imports.

Under `--check` a gate that skipped is **red**: a counter the baseline records and the
run did not report is a failure, because a gate that ran nothing proves nothing — which
is what makes a missing `psql` a loud hole rather than a quiet one.

`deno task test:npm` sits outside the gate set; beside `wasm-pack` it needs `node` and
`npm` at or above the package's `engines` floor (`scripts/npm_metadata.ts`) — its Node leg
refuses to run below it — and it packs and installs the package offline, in the OS temp
directory.

`.github/workflows/check.yml` runs the same three tiers on every push and pull
request — the hermetic bar, the published wasm/npm package, and the six differential
gates against a `postgres:18.6` service. It invokes the `deno task` bodies rather than
re-listing their steps, so what CI runs is what you run.

A warm run (cargo cache populated, `pkg/` built) takes minutes, not seconds — `test:wasm`
rebuilds the wasm and runs a suite that spawns child Deno processes for its depth probes;
read the real figure off the run's closing summary. A first run on a fresh clone
additionally pays a full debug build of the workspace plus a release wasm build, and no
figure for that is recorded.

## Releasing

`@fuzdev/pglet-wasm` is released by the maintainer with `deno task publish:npm`
(`scripts/publish.ts`, over the `scripts/release/` modules). The version is `Cargo.toml`'s
`[workspace.package].version`. Between releases, changes are noted under `CHANGELOG.md`'s
`## Unreleased`, whose next line declares the release's level (`<!-- bump: patch -->`; raise it
to `minor` or `major` when a change calls for it).

- `deno task publish:npm --bump <level>` — a dry run, which changes nothing. Its preflight
  refuses outright on a missing tool (`node` below the package's `engines` floor, `wasm-pack`,
  `gzip`, `cargo`), a failed registry lookup, or a target version already on npm, and warns on
  everything a wetrun refuses: an empty or missing `## Unreleased`, a `--bump` missing or
  disagreeing with the marker, a dirty tree, a branch other than `main`, a `HEAD` that is not
  exactly `origin/main` (after a `git fetch origin main`), an existing `v<version>` tag (locally
  or on `origin`) and a failing `npm whoami`. It then runs `deno task check`,
  `deno task test:rust`, `deno task test:wasm`, `deno task test:npm` (which builds
  `crates/pglet_wasm/pkg-npm` and ends in the packed-tarball smoke over that same directory),
  checks the built `name@version`, runs the size gate on the wasm it built, and runs
  `npm publish --dry-run` in `pkg-npm`. None of its gates needs Postgres; the differential gates
  are CI's and `cargo xtask gates`'.
- `deno task publish:npm --wetrun --bump <level>` — the release. The preflight's warnings
  refuse before anything changes. It then bumps `Cargo.toml`, moves both lockfiles' workspace
  members (the root's and `fuzz/`'s), stamps `## Unreleased` as `## <version>` under a fresh
  `## Unreleased`, runs the same gates over the bumped checkout, refuses if the checkout changed
  while they ran, runs `npm publish` (streamed, so npm's browser auth reaches you), commits
  exactly those files as `chore: release v<version>`, creates the annotated tag
  `v<version>`, and runs `git push --follow-tags origin main`.
- `--no-git` stops after the publish and prints the commit, tag and push instead.
- **Resuming.** The bump writes `.publish-in-progress` (gitignored), which is removed only once
  the push lands. If a wetrun fails after the bump, fix the cause and run
  `deno task publish:npm --wetrun`: it resumes the bumped version, accepting a tree dirty with
  only the release files, or one release commit ahead of `origin/main`. Every later step is
  idempotent, and a version already on npm skips the gates and the publish and only finishes
  git. To abandon a release instead, discard the release files' changes and delete the
  sentinel.
- `--bump` takes `patch`, `minor` or `major`; a prerelease version has no level.

Any other argument is refused — `--` included, so pass the flags straight after the task
name.
