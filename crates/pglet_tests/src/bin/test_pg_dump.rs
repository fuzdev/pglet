//! Real `pg_dump` plain-text restore round-trip.
//!
//! The headline pg_dump-tolerance test: dump a small populated table from live
//! Postgres (`pg_dump`), then replay that dump **verbatim** into a freshly-spawned
//! `pglet_server` with `psql -f` (psql drives the `\restrict`/`\unrestrict`
//! meta-commands and the inline `COPY … FROM stdin` sub-protocol), and assert both
//! backends return identical rows. The dump exercises, end-to-end and over the wire:
//! the `SET …` / `SELECT pg_catalog.set_config(…)` preamble, `CREATE TABLE` — whose
//! column list carries pg_dump's own spelling of a sub-second precision,
//! `timestamp(3) without time zone`, the one place the *mid-name* modifier position
//! arrives as SQL pglet has to read back —,
//! `ALTER TABLE … OWNER TO`, `CREATE SEQUENCE`, `ALTER SEQUENCE … OWNED BY`,
//! `ALTER TABLE … ALTER COLUMN … SET DEFAULT nextval(…)`, `COPY … FROM stdin`,
//! `SELECT pg_catalog.setval(…)`, and `ALTER TABLE … ADD CONSTRAINT … PRIMARY KEY` —
//! over two tables, one of them named so that pg_dump quotes its sequence *inside* the
//! `regclass` literal (`nextval('public."Odd_pg_dump_a_seq"'::regclass)`), which is the
//! shape a real pg_dump restore actually delivers.
//!
//! Self-contained: builds + spawns the server (in-memory), runs against the live
//! Postgres oracle (`PGLET_PG_URL`, or `127.0.0.1:5432` with it unset), and
//! degrades gracefully (skip, exit 0) if Postgres
//! or the `pg_dump`/`psql` client binaries are unavailable. The restore runs under
//! `ON_ERROR_STOP=1`, so any statement pglet rejects fails the test loudly.

// Holds non-`Send` tokio-postgres futures across awaits on a single-threaded local
// runtime; they never cross a thread. `.expect()` surfaces setup failures loudly —
// right for a harness.
#![allow(clippy::future_not_send, clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;

use pglet_testing::server::{
    build_server, find_open_port, hard_kill, spawn_server, wait_for_ready,
};
use pglet_tests::{
    DYNAMIC_PORT_END, DbConfig, PGLET_TEST_PORT, POSTGRES_CONFIG, apply_conninfo, connect,
    has_binary, pglet_config, query_rows,
};
use tokio_postgres::Client;

/// The table the round-trip dumps and restores. A SERIAL primary key (so pg_dump
/// emits CREATE SEQUENCE + ALTER COLUMN SET DEFAULT + ADD CONSTRAINT PRIMARY KEY), a
/// NOT NULL text, a `text[]` array, and a nullable int — with a row carrying a NULL
/// array, to exercise the COPY `\N` path.
const TABLE: &str = "pg_dump_roundtrip";

/// A second table whose name is **not** a safe bare identifier, so its SERIAL
/// sequence's name is not either — and pg_dump therefore carries the identifier
/// quoting *inside* the `regclass` string literal
/// (`nextval('public."Odd_pg_dump_a_seq"'::regclass)`,
/// `pg_catalog.setval('public."Odd_pg_dump_a_seq"', 3, true)`).
///
/// This is the corpus cell that makes the restore path see a quoted literal at all.
/// pg_dump's posture is to quote when `fmtId` says a name needs it, so the *real*
/// restore path is where quoted literals actually arrive — and with a single
/// all-lowercase table here, neither polarity of reading them was reachable: a reader
/// that takes the literal's bytes verbatim resolves a relation *whose name contains the
/// quote characters*, which on pglet a `DEFAULT nextval('x')` silently auto-creates. The
/// symptom is not a failed restore but a **counter that restarts at 1**, which only an
/// assertion on the resumed value can see.
const QUOTED_TABLE: &str = "Odd_pg_dump";

/// `QUOTED_TABLE` as pg_dump and psql must read it — quoted, since it is mixed case.
const QUOTED_TABLE_SQL: &str = "\"Odd_pg_dump\"";

/// A third table carrying what one page cannot: a row of 100 000 characters, which
/// pglet stores in an overflow chain, under a `CHECK` whose 6 000-character literal
/// makes a catalog entry longer than a page.
const LARGE_TABLE: &str = "pg_dump_large";

/// A fourth table keyed on containers: a `jsonb` primary key, a `text[]` unique key, a
/// composite unique key with an `int[]` part and a unique expression index over a
/// `jsonb` value — which pg_dump restores as `ADD CONSTRAINT` / `CREATE UNIQUE INDEX`
/// statements over rows the `COPY` already loaded, so each index is **built** over data.
const CONTAINER_TABLE: &str = "pg_dump_container";

/// A 60-byte table name, under which every constraint name 18.6 chooses is cut to 63
/// bytes: pg_dump writes each chosen name out, and the restore keeps them as written.
const LONG_TABLE: &str = "pg_dump_long_ttttttttttttttttttttttttttttttttttttttttttttttt";

/// A table whose `CHECK`s, index and defaults call the call-spelled string, digest and
/// arithmetic functions, so the restore reads 18.6's own rendering of each —
/// `"left"(t, 1)` keyword-quoted, `power((a)::double precision, (2)::double precision)`
/// with its implicit casts written out, `substr('abc'::text, 2)`.
const FN_TABLE: &str = "pg_dump_fns";

/// `len` letters, deterministic in `seed` — no `repeat()` on the pglet side to build
/// them with, and a string pglz cannot shrink either.
fn letters(seed: u64, len: usize) -> String {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            char::from(b'a' + (x % 26) as u8)
        })
        .collect()
}

/// Seed live Postgres with a fresh populated `TABLE` and return the workspace-root
/// path of the written dump file.
async fn seed_and_dump(pg: &Client, dump_path: &PathBuf) -> Result<(), String> {
    pg.simple_query(&format!("DROP TABLE IF EXISTS {TABLE} CASCADE"))
        .await
        .map_err(|e| format!("drop failed: {e}"))?;
    pg.simple_query(&format!(
        "CREATE TABLE {TABLE} (
            id serial PRIMARY KEY,
            name text NOT NULL,
            tags text[],
            score int,
            ts timestamp(3),
            iv interval(3),
            d date,
            tz timestamptz
        )"
    ))
    .await
    .map_err(|e| format!("create failed: {e}"))?;
    pg.simple_query(&format!(
        "INSERT INTO {TABLE} (name, tags, score, ts, iv, d, tz) VALUES
            ('alpha', '{{x,y}}', 1, '2020-01-02 03:04:05.6789', '1.23456 secs', 'infinity', '-infinity'),
            ('beta', '{{z}}', 2, '1999-12-31 23:59:59.9995', '-0.0015 secs', '-infinity', 'infinity'),
            ('gamma', NULL, 3, 'infinity', NULL, '2000-01-01', NULL)"
    ))
    .await
    .map_err(|e| format!("insert failed: {e}"))?;

    // Two indexes Postgres names itself: pg_dump writes the names it chose, and the
    // restore has to keep them.
    pg.simple_query(&format!(
        "CREATE INDEX ON {TABLE} (score, name); CREATE UNIQUE INDEX ON {TABLE} (lower(name))"
    ))
    .await
    .map_err(|e| format!("create index failed: {e}"))?;

    pg.simple_query(&format!("DROP TABLE IF EXISTS {QUOTED_TABLE_SQL} CASCADE"))
        .await
        .map_err(|e| format!("drop (quoted) failed: {e}"))?;
    pg.simple_query(&format!(
        "CREATE TABLE {QUOTED_TABLE_SQL} (a serial PRIMARY KEY, v text NOT NULL)"
    ))
    .await
    .map_err(|e| format!("create (quoted) failed: {e}"))?;
    pg.simple_query(&format!(
        "INSERT INTO {QUOTED_TABLE_SQL} (v) VALUES ('one'), ('two'), ('three')"
    ))
    .await
    .map_err(|e| format!("insert (quoted) failed: {e}"))?;

    pg.simple_query(&format!("DROP TABLE IF EXISTS {LARGE_TABLE} CASCADE"))
        .await
        .map_err(|e| format!("drop (large) failed: {e}"))?;
    pg.simple_query(&format!(
        "CREATE TABLE {LARGE_TABLE} (id int, body text CHECK (body <> '{}'))",
        letters(11, 6_000)
    ))
    .await
    .map_err(|e| format!("create (large) failed: {e}"))?;
    pg.simple_query(&format!(
        "INSERT INTO {LARGE_TABLE} VALUES (1, '{}'), (2, 'short')",
        letters(12, 100_000)
    ))
    .await
    .map_err(|e| format!("insert (large) failed: {e}"))?;

    pg.simple_query(&format!(
        "DROP TABLE IF EXISTS {CONTAINER_TABLE} CASCADE;
         CREATE TABLE {CONTAINER_TABLE} (
            v int,
            k jsonb PRIMARY KEY,
            tags text[] UNIQUE,
            tenant int,
            nums int[],
            UNIQUE (tenant, nums)
         );
         CREATE UNIQUE INDEX ON {CONTAINER_TABLE} ((k -> 0));
         INSERT INTO {CONTAINER_TABLE} (k, v, tags, tenant, nums) VALUES
            ('{{\"a\":1}}', 1, '{{x,y}}', 1, '{{1,NULL}}'),
            ('[1,2]', 2, '{{}}', 1, '{{}}'),
            ('\"s\"', 3, NULL, NULL, NULL),
            ('[]', 4, '{{x}}', 2, '{{1}}')"
    ))
    .await
    .map_err(|e| format!("create (container) failed: {e}"))?;

    pg.simple_query(&format!(
        "DROP TABLE IF EXISTS {LONG_TABLE} CASCADE;
         CREATE TABLE {LONG_TABLE} (
            a_column_whose_name_is_forty_bytes_long_x int UNIQUE CHECK (a_column_whose_name_is_forty_bytes_long_x > 0),
            a_column_whose_name_is_forty_bytes_long_y int UNIQUE REFERENCES {LONG_TABLE},
            id int PRIMARY KEY,
            CHECK (id > a_column_whose_name_is_forty_bytes_long_x)
         );
         INSERT INTO {LONG_TABLE} VALUES (1, 2, 2)"
    ))
    .await
    .map_err(|e| format!("create (long) failed: {e}"))?;

    pg.simple_query(&format!(
        "DROP TABLE IF EXISTS {FN_TABLE} CASCADE;
         CREATE TABLE {FN_TABLE} (
            id int PRIMARY KEY CHECK (mod(id, 2) = 1),
            t text CHECK (left(t, 1) <> 'z' AND right(t, 1) <> 'z'),
            b bytea CHECK (octet_length(b) < 10),
            a int CHECK (power(a, 2) < 1000 AND md5(t) <> ''),
            l text DEFAULT left('abc', 2),
            s text DEFAULT substr('abc', 2),
            r float8 DEFAULT random()
         );
         CREATE INDEX ON {FN_TABLE} (left(t, 2));
         CREATE INDEX ON {FN_TABLE} (md5(t)) WHERE octet_length(t) > 0;
         INSERT INTO {FN_TABLE} (id, t, b, a) VALUES (1, 'apple', '\\x0102', 3), (3, 'héllo', NULL, NULL)"
    ))
    .await
    .map_err(|e| format!("create (fns) failed: {e}"))?;

    // pg_dump the tables to the dump file. The quoted name needs its quotes inside the
    // `-t` pattern, or pg_dump folds it and matches nothing.
    let mut command = Command::new("pg_dump");
    command
        .args([
            "-t",
            TABLE,
            "-t",
            QUOTED_TABLE_SQL,
            "-t",
            LARGE_TABLE,
            "-t",
            CONTAINER_TABLE,
            "-t",
            LONG_TABLE,
            "-t",
            FN_TABLE,
            "-f",
        ])
        .arg(dump_path);
    apply_conninfo(&mut command, &POSTGRES_CONFIG);
    let output = command
        .output()
        .map_err(|e| format!("pg_dump spawn failed: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "pg_dump failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

/// Replay the dump into pglet via `psql -f` under `ON_ERROR_STOP=1`. Any statement
/// pglet rejects makes psql exit non-zero — a loud restore failure.
fn restore_into_pglet(pglet: &DbConfig, dump_path: &PathBuf) -> Result<(), String> {
    let mut command = Command::new("psql");
    command
        .args(["-v", "ON_ERROR_STOP=1", "-q", "-f"])
        .arg(dump_path);
    apply_conninfo(&mut command, pglet);
    let output = command
        .output()
        .map_err(|e| format!("psql spawn failed: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "psql restore into pglet failed (ON_ERROR_STOP):\n  stdout: {}\n  stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ))
    }
}

/// The full round-trip: seed+dump on Postgres, restore into pglet, compare rows, and
/// confirm the restored PRIMARY KEY enforces (a duplicate-id insert must be rejected).
async fn run_roundtrip(pg: &Client, pglet_client: &Client, pglet: &DbConfig) -> Result<(), String> {
    let dump_path = std::env::temp_dir().join("pglet_pg_dump_roundtrip.sql");

    seed_and_dump(pg, &dump_path).await?;
    restore_into_pglet(pglet, &dump_path)?;

    // Both backends must return identical rows.
    let select = format!("SELECT id, name, tags, score, ts, iv, d, tz FROM {TABLE} ORDER BY id");
    let pg_rows = query_rows(pg, &select).await?;
    let pglet_rows = query_rows(pglet_client, &select).await?;
    if pg_rows != pglet_rows {
        return Err(format!(
            "restored rows differ:\n  Postgres: {pg_rows:?}\n  pglet:    {pglet_rows:?}"
        ));
    }
    if pglet_rows.len() != 3 {
        return Err(format!(
            "expected 3 restored rows, got {}",
            pglet_rows.len()
        ));
    }

    // **Rows alone cannot see a dropped precision**: pg_dump's `COPY` data is already
    // rounded on the Postgres side, so a restore that read `timestamp(3) without time
    // zone` and threw the `(3)` away still matches every row above. The catalog is
    // where it shows — `datetime_precision` is `3` for the restored column and `6` for
    // a column that lost its modifier — so the shape is compared as well as the data.
    let precision_select = format!(
        "SELECT column_name, datetime_precision FROM information_schema.columns \
         WHERE table_name = '{TABLE}' ORDER BY ordinal_position"
    );
    let pg_precision = query_rows(pg, &precision_select).await?;
    let pglet_precision = query_rows(pglet_client, &precision_select).await?;
    if pg_precision != pglet_precision {
        return Err(format!(
            "restored datetime_precision differs:\n  Postgres: {pg_precision:?}\n  pglet:    \
             {pglet_precision:?}"
        ));
    }

    // The index names Postgres chose for its two unnamed `CREATE INDEX` statements come
    // back as the dump spelled them, beside the primary key's.
    let index_select = format!(
        "SELECT indexname::text AS indexname FROM pg_indexes WHERE tablename = '{TABLE}' ORDER BY 1"
    );
    let pg_indexes = query_rows(pg, &index_select).await?;
    let pglet_indexes = query_rows(pglet_client, &index_select).await?;
    if pg_indexes != pglet_indexes || pglet_indexes.len() != 3 {
        return Err(format!(
            "restored index names differ:\n  Postgres: {pg_indexes:?}\n  pglet:    {pglet_indexes:?}"
        ));
    }

    // ...and the same for the quoted table, whose sequence pg_dump names through a
    // *quoted* literal.
    let quoted_select = format!("SELECT a, v FROM {QUOTED_TABLE_SQL} ORDER BY a");
    let pg_quoted = query_rows(pg, &quoted_select).await?;
    let pglet_quoted = query_rows(pglet_client, &quoted_select).await?;
    if pg_quoted != pglet_quoted {
        return Err(format!(
            "restored rows differ for {QUOTED_TABLE}:\n  Postgres: {pg_quoted:?}\n  pglet:    \
             {pglet_quoted:?}"
        ));
    }

    // The long row comes back whole, and the long `CHECK` still refuses its literal.
    let large_select = format!("SELECT id, body FROM {LARGE_TABLE} ORDER BY id");
    let pg_large = query_rows(pg, &large_select).await?;
    let pglet_large = query_rows(pglet_client, &large_select).await?;
    if pg_large != pglet_large || pglet_large.len() != 2 {
        return Err(format!(
            "restored rows differ for {LARGE_TABLE} ({} rows on Postgres, {} on pglet)",
            pg_large.len(),
            pglet_large.len()
        ));
    }
    let refused = pglet_client
        .simple_query(&format!(
            "INSERT INTO {LARGE_TABLE} VALUES (3, '{}')",
            letters(11, 6_000)
        ))
        .await;
    match refused {
        Err(e) if e.as_db_error().is_some_and(|d| d.code().code() == "23514") => {}
        other => {
            return Err(format!(
                "the restored {LARGE_TABLE} CHECK did not refuse its own literal: {other:?}"
            ));
        }
    }

    // The names 18.6 chose under the 60-byte table name arrive as pg_dump wrote them.
    let long_select = "SELECT contype::text AS contype, conname::text AS conname FROM \
         pg_constraint WHERE conname LIKE 'pg\\_dump\\_long%' AND contype <> 'n' ORDER BY 1, 2";
    let pg_long = query_rows(pg, long_select).await?;
    let pglet_long = query_rows(pglet_client, long_select).await?;
    if pg_long != pglet_long || pglet_long.len() != 6 {
        return Err(format!(
            "restored constraint names differ for {LONG_TABLE}:\n  Postgres: {pg_long:?}\n  \
             pglet:    {pglet_long:?}"
        ));
    }

    // The functions table: its rows, its defaults applied by a fresh insert on each
    // engine, and its restored `mod` CHECK refusing an even key.
    let fn_select = format!(
        "SELECT id, t, b, a, l, s, r >= 0 AND r < 1 AS r_in_range, left(t, 2) AS l2, \
         md5(t) AS m FROM {FN_TABLE} ORDER BY id"
    );
    let insert_defaults = format!("INSERT INTO {FN_TABLE} (id, t) VALUES (5, 'kiwi')");
    for client in [pg, pglet_client] {
        client
            .simple_query(&insert_defaults)
            .await
            .map_err(|e| format!("insert into the restored {FN_TABLE} failed: {e}"))?;
    }
    let pg_fns = query_rows(pg, &fn_select).await?;
    let pglet_fns = query_rows(pglet_client, &fn_select).await?;
    if pg_fns != pglet_fns || pglet_fns.len() != 3 {
        return Err(format!(
            "restored {FN_TABLE} rows differ:\n  Postgres: {pg_fns:?}\n  pglet:    {pglet_fns:?}"
        ));
    }
    let even = pglet_client
        .simple_query(&format!("INSERT INTO {FN_TABLE} (id, t) VALUES (2, 'x')"))
        .await;
    match even {
        Err(e) if e.as_db_error().is_some_and(|d| d.code().code() == "23514") => {}
        other => {
            return Err(format!(
                "the restored {FN_TABLE} mod CHECK did not refuse an even key: {other:?}"
            ));
        }
    }

    // The container-keyed table: its rows in the `jsonb` primary key's order, the index
    // names pg_dump wrote, and each restored index refusing the key it already holds —
    // a `jsonb` number at another scale, the same tags, the same `(tenant, nums)`, the
    // same `k -> 0`.
    let container_select =
        format!("SELECT v, k::text AS key, tags, tenant, nums FROM {CONTAINER_TABLE} ORDER BY k");
    let pg_container = query_rows(pg, &container_select).await?;
    let pglet_container = query_rows(pglet_client, &container_select).await?;
    if pg_container != pglet_container || pglet_container.len() != 4 {
        return Err(format!(
            "restored rows differ for {CONTAINER_TABLE}:\n  Postgres: {pg_container:?}\n  pglet:    \
             {pglet_container:?}"
        ));
    }
    let container_indexes = format!(
        "SELECT indexname::text AS indexname, indexdef FROM pg_indexes \
         WHERE tablename = '{CONTAINER_TABLE}' ORDER BY 1"
    );
    let pg_indexes = query_rows(pg, &container_indexes).await?;
    let pglet_indexes = query_rows(pglet_client, &container_indexes).await?;
    if pg_indexes != pglet_indexes || pglet_indexes.len() != 4 {
        return Err(format!(
            "restored indexes differ for {CONTAINER_TABLE}:\n  Postgres: {pg_indexes:?}\n  pglet:    \
             {pglet_indexes:?}"
        ));
    }
    for duplicate in [
        "(k, v) VALUES ('{\"a\":1.0}', 9)",
        "(k, v, tags) VALUES ('9', 9, '{x,y}')",
        "(k, v, tenant, nums) VALUES ('9', 9, 1, '{1,NULL}')",
        "(k, v) VALUES ('[1,3]', 9)",
    ] {
        let refused = pglet_client
            .simple_query(&format!("INSERT INTO {CONTAINER_TABLE} {duplicate}"))
            .await;
        match refused {
            Err(e) if e.as_db_error().is_some_and(|d| d.code().code() == "23505") => {}
            other => {
                return Err(format!(
                    "the restored {CONTAINER_TABLE} took the duplicate `{duplicate}`: {other:?}"
                ));
            }
        }
    }

    // The counter resumes where the dump left it. Three rows were seeded, so the fourth
    // must be `4`.
    //
    // **This is the weak half of the pair and it is worth saying why**, because on its
    // own it proves less than it looks. A restore that reads a `regclass` literal as the
    // bytes between its quotes resolves a relation whose name *contains* the quote
    // characters — and both literal sites, the `SET DEFAULT nextval(…)` and the trailing
    // `pg_catalog.setval(…)`, go through that same reader. So they agree with each other
    // while both name a twin the `CREATE SEQUENCE` did not create (a DDL
    // `DEFAULT nextval('x')` auto-creates `x` on pglet), the twin is the one `setval`
    // advances, and the next insert answers `4` exactly as it should. Measured: this
    // assertion alone is **green** against that reader. Every probe spelled as a literal
    // follows the wrong path identically; only an **identifier**-position probe, which
    // the lexer resolves, can see the split — hence the `DROP SEQUENCE` below.
    let next = pglet_client
        .simple_query(&format!(
            "INSERT INTO {QUOTED_TABLE_SQL} (v) VALUES ('four') RETURNING a"
        ))
        .await
        .map_err(|e| format!("insert into the restored {QUOTED_TABLE} failed: {e}"))?;
    let issued = next
        .iter()
        .find_map(|m| match m {
            tokio_postgres::SimpleQueryMessage::Row(r) => r.get(0).map(ToString::to_string),
            _ => None,
        })
        .ok_or_else(|| format!("no RETURNING row from the restored {QUOTED_TABLE}"))?;
    if issued != "4" {
        return Err(format!(
            "the restored {QUOTED_TABLE} issued `{issued}` where the dumped counter says 4 — its \
             quoted `nextval`/`setval` literal named a different relation, so the identity \
             silently renumbers from 1"
        ));
    }

    // The discriminating half: which sequence the restored column's default actually
    // names, asked through an **identifier** position so the lexer resolves the quoting
    // rather than the literal reader. A restored `SERIAL` column's default is the NORMAL
    // `pg_attrdef` dependency edge, so dropping its sequence is `2BP01`. If the literal
    // named a quote-carrying twin instead, the real `CREATE SEQUENCE`d relation has no
    // dependent at all and this **succeeds** — the whole defect, visible in one statement
    // and invisible to every row and counter above it.
    let drop_seq = pglet_client
        .simple_query(&format!("DROP SEQUENCE \"{QUOTED_TABLE}_a_seq\""))
        .await;
    match drop_seq {
        Ok(_) => {
            return Err(format!(
                "DROP SEQUENCE \"{QUOTED_TABLE}_a_seq\" succeeded on the restored database — the \
                 restored column default names some other relation, so the sequence pg_dump \
                 created is nobody's dependent"
            ));
        }
        Err(e) => {
            let code = e.as_db_error().map(|d| d.code().code().to_string());
            if code.as_deref() != Some("2BP01") {
                return Err(format!(
                    "DROP SEQUENCE \"{QUOTED_TABLE}_a_seq\" errored but not with 2BP01: {e} (code \
                     {code:?})"
                ));
            }
        }
    }

    // The PRIMARY KEY added by `ADD CONSTRAINT` during restore must actually enforce:
    // a duplicate-id insert is rejected (proves it wasn't silently no-op'd).
    let dup = pglet_client
        .simple_query(&format!("INSERT INTO {TABLE} (id, name) VALUES (1, 'dup')"))
        .await;
    match dup {
        Ok(_) => Err("restored PRIMARY KEY did not enforce (duplicate id accepted)".to_string()),
        Err(e) => {
            let code = e.as_db_error().map(|d| d.code().code().to_string());
            if code.as_deref() == Some("23505") {
                Ok(())
            } else {
                Err(format!(
                    "duplicate insert errored but not with 23505: {e} (code {code:?})"
                ))
            }
        }
    }
}

#[tokio::main]
async fn main() {
    // Postgres is the ground truth; the client binaries drive the restore. Degrade
    // gracefully (skip, exit 0) when any is missing — matching the `test_copy` policy.
    if let Err(e) = pglet_tests::ensure_postgres().await {
        eprintln!("[pg_dump] {e}");
        eprintln!("[pg_dump] skipping round-trip (Postgres unavailable)");
        return;
    }
    if !has_binary("pg_dump") || !has_binary("psql") {
        eprintln!("[pg_dump] skipping round-trip (pg_dump/psql client not on PATH)");
        return;
    }

    eprintln!("[pg_dump] Building pglet_server...");
    if !build_server() {
        eprintln!("[pg_dump] build failed");
        std::process::exit(1);
    }
    let port = find_open_port(PGLET_TEST_PORT..DYNAMIC_PORT_END);
    eprintln!("[pg_dump] Spawning pglet_server on port {port} (in-memory)...");
    let server = spawn_server(port, None);
    if !wait_for_ready(port).await {
        eprintln!("[pg_dump] server failed to start");
        hard_kill(server);
        std::process::exit(1);
    }

    let pglet_config = pglet_config(port);
    let pg = connect(&POSTGRES_CONFIG)
        .await
        .expect("connect to Postgres");
    let pglet_client = connect(&pglet_config).await.expect("connect to pglet");

    println!("\n=== pg_dump Round-Trip ===\n");
    let outcome = run_roundtrip(&pg, &pglet_client, &pglet_config).await;

    eprintln!("[pg_dump] Shutting down server...");
    hard_kill(server);

    match outcome {
        Ok(()) => {
            println!("\x1b[32m✓\x1b[0m pg_dump restore round-trip ({TABLE})");
            println!("\n=== Results ===\nPassed: 1\nFailed: 0");
        }
        Err(e) => {
            println!("\x1b[31m✗\x1b[0m pg_dump restore round-trip: {e}");
            println!("\n=== Results ===\nPassed: 0\nFailed: 1");
            std::process::exit(1);
        }
    }
}
