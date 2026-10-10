//! Large-dataset **soak** harness — sustained churn against a rich schema, with a
//! periodic hard-kill + in-process durability/consistency audit, plus a live
//! MVCC/vacuum-stats leg the wire can't reach.
//!
//! `test_crash` proves the durability *contract* on a trivial one-column table
//! under a tight kill loop; this drives a realistic schema (PK, UNIQUE, composite
//! FK with transitive `ON DELETE CASCADE`, CHECK, b-tree + partial + GIN indexes,
//! jsonb / text[] / numeric / timestamptz) under a weighted, growing workload and
//! asserts the engine stays *internally consistent and correct* the whole way —
//! not just that acked rows survive.
//!
//! ## Three legs over one engine
//!
//! **Leg A — server-driven churn + kill + post-kill in-process audit (primary).**
//! Spawns the real `pglet_server` over a `--data-dir` file and drives a weighted op
//! mix (INSERT/UPDATE/DELETE across the three tables, PK/range/GIN/aggregate reads,
//! `BEGIN…COMMIT` / `BEGIN…ROLLBACK` bursts, and abrupt mid-`BEGIN` socket drops)
//! over `tokio-postgres`, maintaining a **client-side ledger** (the expected live-PK
//! set per table, cascades applied) as the oracle. Continuous invariants run over the
//! wire (incl. a full account-set comparison that catches symmetric drift live);
//! periodically the server is `kill -9`ed and the data-dir is reopened **in-process**
//! to audit. Kills alternate between two modes:
//! - **quiescent** (state fully durable at kill time) — the *consistency* audit:
//!   index ↔ table consistency (every index entry resolves to a live row — no orphan;
//!   every predicate-satisfying live row has its entries — no missing; band count ==
//!   expected encoded-key count — no phantom; over b-tree, partial b-tree, GIN),
//!   durability == ledger (a lost acked-and-fsynced commit *or* a surviving
//!   uncommitted write both fail loud), and global referential integrity (no dup
//!   PK/UNIQUE, no FK orphan).
//! - **straddling** — the *atomicity* check: a large multi-row `INSERT` is fired on a
//!   separate connection and left un-awaited, then the kill lands after a seeded
//!   randomized delay (mirrors `test_crash` phase B, over the rich FK'd/indexed
//!   schema). On reopen the in-flight batch must recover **all-or-nothing** (never
//!   partial), the image must stay index-consistent, and the recovered accounts must
//!   bracket the ledger — the batch is *allowed* but only *required* if the client saw
//!   its ack (acked ⇒ durable). The ledger models it as an optional survivor and
//!   reconciles to the recovered outcome.
//!
//! Either way the store reopens clean, with sane `stats()`.
//!
//! **Leg B — in-process embed churn with LIVE `stats()` (snapshot/vacuum leg).**
//! The precise `active_snapshots` / vacuum invariants need a live handle the wire
//! doesn't expose, so a second segment assembles a file-backed `BtreeStorageWrapper`
//! directly and drives full-SQL churn through `execute_with_storage` (the same
//! executor entry the oracle's `TestDb` uses) — holding the wrapper so it can read
//! `stats()` and call `vacuum()` between ops. Autocommit writes land via
//! `apply_batch` (a CoW root swap with no metadata fsync), so the churn is fast and
//! its GC effects are the point. It asserts a committed / rolled-back transaction
//! releases its snapshot (`active_snapshots` returns to baseline), that an
//! **abandoned** transaction is the leaked-snapshot fingerprint (pins GC) and
//! recovers once released + vacuumed, and that under a bounded working set the free
//! list absorbs allocations so the data file stays bounded.
//!
//! **Leg C — re-migration / index-OID-desync soak (server-driven).** A long-lived
//! server is driven through many idempotent re-migration + same-named-index-swap
//! cycles (the fuz_app L2-reset shape: `DROP TABLE schema_version` → replay
//! `CREATE … IF NOT EXISTS` → same-named `DROP INDEX IF EXISTS x; CREATE INDEX IF NOT
//! EXISTS x …` swap pairs, each recreation burning a fresh OID → `TRUNCATE … CASCADE`
//! → re-seed), interleaved with index-OID-burn and table-id-reuse rollbacks. Every
//! cycle asserts the recreated indexes still BIND and ENFORCE over the wire
//! (partial-UNIQUE point read + duplicate-`23505`, GIN `@>`, b-tree range, total
//! count), then fires the **regression trigger proper**: a *within-transaction*
//! same-name swap on a dedicated highest-oid probe index (`BEGIN; DROP INDEX x; CREATE
//! INDEX x; COMMIT`/`ROLLBACK`) followed by an extended-protocol, index-accelerated
//! read of the recreated index. Under the reverted fix the in-txn CREATE reuses the
//! just-dropped (still-buffered) oid, so commit/rollback sweeps the shared band and the
//! read returns the wrong count / errors — the exact catalog↔storage desync (the
//! autocommit swaps above shed the band before reuse and so cannot reproduce it). This
//! makes Leg C a genuine regression-lock (RED when the index-OID-desync fix is reverted) under the
//! default `index_acceleration` build. Periodically the server is `kill -9`ed and
//! reopened in-process to audit that no desync ever accumulates — index↔table
//! consistency + referential integrity (reused from Leg A) plus a band↔catalog
//! **bijection** (no orphan index band whose id has no catalog owner) and a **distinct
//! storage index_id** check over the reopened catalog. One kill straddles an un-awaited
//! `CREATE INDEX` over a populated table (the half-built index must recover
//! all-or-nothing).
//!
//! Run: `deno task test:soak` (short default) / `deno task test:soak:long`.
//! Knobs: `PGLET_SOAK_SEED`, `PGLET_SOAK_ROWS`, `PGLET_SOAK_DURATION_SECS`,
//! `PGLET_SOAK_KILL_EVERY_SECS`, `PGLET_SOAK_CYCLES`, `PGLET_SOAK_KILL_CYCLES`,
//! `--long`, `--update`.

// Test-harness binary: `.expect()`/`panic!` surface setup failures loudly, the
// right behavior for a test runner rather than threading `Result` everywhere. The
// query futures hold non-`Send` bound-param state across awaits on a local runtime.
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::future_not_send)]

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pglet::{Database, StorageBackend};
use pglet_sql::catalog::Schema;
use pglet_sql::executor::{EvalEnv, SequenceSession, execute_with_storage};
use pglet_sql::types::Value;
use pglet_storage::Storage;
use pglet_storage::btree::{
    BtreeKey, BtreeStorageWrapper, CowBtree, DoubleMetaStore, FileBlockIo, PAGE_SIZE,
};
use pglet_testing::proc::read_status_kb;
use pglet_testing::rng::Xorshift;
use pglet_testing::server::{build_server, connect, hard_kill, spawn_ready};
use pglet_tests::{DYNAMIC_PORT_END, PGLET_TEST_PORT};
use tokio_postgres::{Client, SimpleQueryMessage};

/// The single data file `--data-dir` holds (mirrors `pglet::DATA_FILENAME`).
const DATA_FILE: &str = "data.pglet";
/// Churn steps run between loop-control checks (kill / sample / invariant gates).
const BATCH: u64 = 25;
/// Run the continuous wire invariants every this-many churn ops.
const INVARIANT_EVERY: u64 = 40;
/// Append a time-series recorder sample at least this often.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);
/// Leg B's base working set is capped here regardless of `PGLET_SOAK_ROWS`: it is the
/// snapshot/vacuum leg, and a pinned CoW churn on a huge tree balloons the temp data
/// file without adding coverage (the memory-vs-rows axis is Leg A's, at the full tier).
const LEG_B_MAX_ROWS: i64 = 10_000;

/// The workload schema — PK, UNIQUE, composite FK (transitive `ON DELETE CASCADE`),
/// CHECK, b-tree + partial b-tree + GIN indexes, jsonb/text[]/numeric/timestamptz.
/// Verified accepted by pglet (all three tables + the cascade) before landing.
const SCHEMA_SQL: &str = "\
CREATE TABLE account (id BIGINT PRIMARY KEY, email TEXT UNIQUE NOT NULL, \
  status TEXT NOT NULL CHECK (status IN ('active','suspended','closed')), \
  tags TEXT[], meta JSONB, created_at TIMESTAMPTZ NOT NULL);\n\
CREATE INDEX account_created_idx ON account (created_at);\n\
CREATE INDEX account_tags_gin ON account USING gin (tags);\n\
CREATE INDEX account_active_email_idx ON account (email) WHERE status = 'active';\n\
CREATE TABLE item (id BIGINT PRIMARY KEY, \
  account_id BIGINT NOT NULL REFERENCES account(id) ON DELETE CASCADE, \
  kind TEXT NOT NULL, amount NUMERIC(12,2) NOT NULL, doc JSONB, created_at TIMESTAMPTZ NOT NULL, \
  UNIQUE (id, account_id));\n\
CREATE INDEX item_account_idx ON item (account_id);\n\
CREATE INDEX item_doc_gin ON item USING gin (doc);\n\
CREATE TABLE item_label (item_id BIGINT NOT NULL, account_id BIGINT NOT NULL, label TEXT NOT NULL, \
  PRIMARY KEY (item_id, label), \
  FOREIGN KEY (item_id, account_id) REFERENCES item(id, account_id) ON DELETE CASCADE);";

const STATUSES: [&str; 3] = ["active", "suspended", "closed"];
const TAG_POOL: [&str; 6] = ["red", "blue", "green", "gold", "silver", "black"];
const LABEL_POOL: [&str; 6] = ["urgent", "later", "review", "done", "hold", "new"];
const KIND_POOL: [&str; 4] = ["alpha", "beta", "gamma", "delta"];

// ============================================================================
// Config
// ============================================================================

/// The run's tunables, resolved from env + `--long`/`--update` flags.
#[derive(Debug, Clone)]
struct Config {
    seed: u64,
    rows_target: u64,
    duration: Duration,
    kill_every: Duration,
    /// Leg C: how many idempotent-re-migration + index-swap reset cycles to run.
    reset_cycles: u64,
    /// Leg C: `kill -9` + in-process audit + respawn every this-many cycles (the final
    /// cycle always audits regardless). Clamped `>= 1` so `cycle % kill_every_cycles`
    /// never divides by zero.
    kill_every_cycles: u64,
    long: bool,
    update: bool,
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

impl Config {
    fn from_args() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let long = args.iter().any(|a| a == "--long");
        let update = args.iter().any(|a| a == "--update");

        // `--long`: sustained mode (larger tier, longer). Default: a few-second CI run.
        let (rows_default, dur_default, kill_default) = if long {
            (20_000, 60, 10)
        } else {
            (2_000, 6, 3)
        };
        // Leg C is cycle-bounded (not duration-bounded): more reset cycles under `--long`.
        let (cycles_default, kill_cycles_default) = if long { (200, 20) } else { (12, 4) };

        Self {
            seed: env_u64("PGLET_SOAK_SEED", 0x50AC_0F17_9E37_79B9),
            rows_target: env_u64("PGLET_SOAK_ROWS", rows_default),
            duration: Duration::from_secs(env_u64("PGLET_SOAK_DURATION_SECS", dur_default)),
            kill_every: Duration::from_secs(env_u64("PGLET_SOAK_KILL_EVERY_SECS", kill_default)),
            reset_cycles: env_u64("PGLET_SOAK_CYCLES", cycles_default).max(1),
            kill_every_cycles: env_u64("PGLET_SOAK_KILL_CYCLES", kill_cycles_default).max(1),
            long,
            update,
        }
    }
}

/// A scratch data-dir under the system temp dir, removed on drop (no `tempfile` dep).
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("pglet_soak_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create scratch data dir");
        Self { path }
    }

    fn data_file(&self) -> PathBuf {
        self.path.join(DATA_FILE)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// ============================================================================
// Client-side ledger (the durability oracle)
// ============================================================================

/// A set of live PKs supporting O(1) random pick + O(1) remove (Vec + id→index map).
#[derive(Debug, Default)]
struct IdSet {
    ids: Vec<i64>,
    pos: HashMap<i64, usize>,
}

impl IdSet {
    fn insert(&mut self, id: i64) {
        if self.pos.contains_key(&id) {
            return;
        }
        self.pos.insert(id, self.ids.len());
        self.ids.push(id);
    }

    fn remove(&mut self, id: i64) -> bool {
        let Some(idx) = self.pos.remove(&id) else {
            return false;
        };
        let last = self.ids.pop().expect("nonempty on removal");
        if idx < self.ids.len() {
            self.ids[idx] = last;
            self.pos.insert(last, idx);
        }
        true
    }

    const fn len(&self) -> usize {
        self.ids.len()
    }

    fn first(&self) -> Option<i64> {
        self.ids.first().copied()
    }

    fn pick(&self, rng: &mut Xorshift) -> Option<i64> {
        if self.ids.is_empty() {
            None
        } else {
            Some(self.ids[(rng.below(self.ids.len() as u64)) as usize])
        }
    }
}

/// The expected live state per table, updated per *acked* op with cascades applied.
/// This is the ground truth the post-kill recovered state is checked against.
#[derive(Debug, Default)]
struct Ledger {
    accounts: IdSet,
    account_items: HashMap<i64, HashSet<i64>>,
    items: IdSet,
    item_account: HashMap<i64, i64>,
    item_labels: HashMap<i64, HashSet<String>>,
}

impl Ledger {
    fn add_account(&mut self, id: i64) {
        self.accounts.insert(id);
        self.account_items.entry(id).or_default();
    }

    fn del_account(&mut self, id: i64) {
        if !self.accounts.remove(id) {
            return;
        }
        // Transitive cascade: the account's items and their labels go with it.
        if let Some(items) = self.account_items.remove(&id) {
            for item in items {
                self.items.remove(item);
                self.item_account.remove(&item);
                self.item_labels.remove(&item);
            }
        }
    }

    fn add_item(&mut self, id: i64, account: i64) {
        self.items.insert(id);
        self.item_account.insert(id, account);
        self.item_labels.entry(id).or_default();
        self.account_items.entry(account).or_default().insert(id);
    }

    fn del_item(&mut self, id: i64) {
        if !self.items.remove(id) {
            return;
        }
        if let Some(acct) = self.item_account.remove(&id)
            && let Some(set) = self.account_items.get_mut(&acct)
        {
            set.remove(&id);
        }
        self.item_labels.remove(&id);
    }

    fn add_label(&mut self, item: i64, label: String) {
        self.item_labels.entry(item).or_default().insert(label);
    }

    fn has_label(&self, item: i64, label: &str) -> bool {
        self.item_labels
            .get(&item)
            .is_some_and(|s| s.contains(label))
    }

    const fn account_count(&self) -> usize {
        self.accounts.len()
    }

    const fn item_count(&self) -> usize {
        self.items.len()
    }

    fn label_count(&self) -> usize {
        self.item_labels.values().map(HashSet::len).sum()
    }

    /// The expected recovered account PK set (for the durability audit).
    fn account_set(&self) -> HashSet<i64> {
        self.accounts.ids.iter().copied().collect()
    }

    fn item_set(&self) -> HashSet<i64> {
        self.items.ids.iter().copied().collect()
    }

    /// The expected recovered `(item_id, label)` PK set.
    fn label_set(&self) -> HashSet<(i64, String)> {
        self.item_labels
            .iter()
            .flat_map(|(item, labels)| labels.iter().map(move |l| (*item, l.clone())))
            .collect()
    }
}

/// The soak driver state carried across the churn loop.
struct Soak {
    rng: Xorshift,
    ledger: Ledger,
    next_account_id: i64,
    next_item_id: i64,
    ops: u64,
}

impl Soak {
    fn new(seed: u64) -> Self {
        Self {
            rng: Xorshift::new(seed),
            ledger: Ledger::default(),
            next_account_id: 1,
            // Item ids in a disjoint band from account ids — purely for legibility.
            next_item_id: 1_000_000_000,
            ops: 0,
        }
    }

    const fn fresh_account(&mut self) -> i64 {
        let id = self.next_account_id;
        self.next_account_id += 1;
        id
    }

    const fn fresh_item(&mut self) -> i64 {
        let id = self.next_item_id;
        self.next_item_id += 1;
        id
    }
}

// ============================================================================
// SQL value generators (shared by the async wire leg and the sync embed leg)
// ============================================================================

/// A deterministic `timestamptz` literal (`YYYY-MM-DD HH:MM:SS+00`) seeded by `seed`.
fn ts_literal(seed: i64) -> String {
    let base = 1_700_000_000i64;
    let secs = base + seed.rem_euclid(60_000_000);
    let fmt = time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]:[second]+00");
    time::OffsetDateTime::from_unix_timestamp(secs)
        .ok()
        .and_then(|dt| dt.format(&fmt).ok())
        .unwrap_or_else(|| "2024-01-01 00:00:00+00".to_string())
}

/// A `text[]` literal of 1–3 distinct tags from the pool.
fn tags_literal(rng: &mut Xorshift) -> String {
    let n = 1 + rng.below(3);
    let mut picked: Vec<&str> = Vec::new();
    for _ in 0..n {
        let t = TAG_POOL[(rng.below(TAG_POOL.len() as u64)) as usize];
        if !picked.contains(&t) {
            picked.push(t);
        }
    }
    let inner = picked
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(",");
    format!("ARRAY[{inner}]::text[]")
}

/// A parenthesized `account` VALUES tuple (so a batch load can join many into one
/// multi-row `INSERT` — collapsing per-row round-trips, which dominate wall time).
fn account_tuple(id: i64, rng: &mut Xorshift) -> String {
    let email = format!("u{id}@ex.com");
    let status = STATUSES[(rng.below(3)) as usize];
    let tags = if rng.below(4) == 0 {
        "NULL".to_string()
    } else {
        tags_literal(rng)
    };
    let kind = KIND_POOL[(rng.below(KIND_POOL.len() as u64)) as usize];
    let meta = if rng.below(4) == 0 {
        "NULL".to_string()
    } else {
        format!("'{{\"kind\":\"{kind}\",\"n\":{id}}}'::jsonb")
    };
    let ts = ts_literal(id);
    format!("({id}, '{email}', '{status}', {tags}, {meta}, '{ts}')")
}

fn insert_account_sql(id: i64, rng: &mut Xorshift) -> String {
    format!("INSERT INTO account VALUES {}", account_tuple(id, rng))
}

/// A parenthesized `item` VALUES tuple.
fn item_tuple(id: i64, account: i64, rng: &mut Xorshift) -> String {
    let kind = KIND_POOL[(rng.below(KIND_POOL.len() as u64)) as usize];
    let tag = TAG_POOL[(rng.below(TAG_POOL.len() as u64)) as usize];
    let amount = format!("{}.{:02}", id.rem_euclid(100_000), id.rem_euclid(100));
    let doc = format!("'{{\"kind\":\"{kind}\",\"tag\":\"{tag}\"}}'::jsonb");
    let ts = ts_literal(id);
    format!("({id}, {account}, '{kind}', {amount}, {doc}, '{ts}')")
}

fn insert_item_sql(id: i64, account: i64, rng: &mut Xorshift) -> String {
    format!("INSERT INTO item VALUES {}", item_tuple(id, account, rng))
}

// ============================================================================
// Wire helpers (Leg A, over tokio-postgres)
// ============================================================================

/// Run a statement over the simple-query protocol; a wire error is a hard finding.
async fn run(c: &Client, sql: &str) -> Result<(), String> {
    c.simple_query(sql)
        .await
        .map(|_| ())
        .map_err(|e| format!("{sql} :: {e}"))
}

/// Read the first column of the first row as an `i64` (for `count(*)`).
async fn scalar_i64(c: &Client, sql: &str) -> Result<i64, String> {
    let msgs = c
        .simple_query(sql)
        .await
        .map_err(|e| format!("{sql} :: {e}"))?;
    for m in msgs {
        if let SimpleQueryMessage::Row(r) = m {
            return r
                .get(0)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| format!("{sql}: unparseable scalar"));
        }
    }
    Err(format!("{sql}: no row"))
}

/// Assert `sql` fails with exactly `code` (a deliberately-failing probe).
async fn expect_sqlstate(c: &Client, sql: &str, code: &str) -> Result<(), String> {
    match c.simple_query(sql).await {
        Ok(_) => Err(format!(
            "expected SQLSTATE {code} but statement succeeded: {sql}"
        )),
        Err(e) => {
            let got = e.as_db_error().map(|d| d.code().code().to_string());
            if got.as_deref() == Some(code) {
                Ok(())
            } else {
                Err(format!("expected SQLSTATE {code}, got {got:?}: {sql}"))
            }
        }
    }
}

// ============================================================================
// Churn ops (Leg A)
// ============================================================================

async fn op_insert_account(s: &mut Soak, c: &Client) -> Result<(), String> {
    let id = s.fresh_account();
    let sql = insert_account_sql(id, &mut s.rng);
    run(c, &sql).await?;
    s.ledger.add_account(id);
    Ok(())
}

async fn op_insert_item(s: &mut Soak, c: &Client) -> Result<(), String> {
    let Some(account) = s.ledger.accounts.pick(&mut s.rng) else {
        return Ok(()); // no account to reference yet
    };
    let id = s.fresh_item();
    let sql = insert_item_sql(id, account, &mut s.rng);
    run(c, &sql).await?;
    s.ledger.add_item(id, account);
    Ok(())
}

async fn op_insert_label(s: &mut Soak, c: &Client) -> Result<(), String> {
    let Some(item) = s.ledger.items.pick(&mut s.rng) else {
        return Ok(());
    };
    let label = LABEL_POOL[(s.rng.below(LABEL_POOL.len() as u64)) as usize];
    if s.ledger.has_label(item, label) {
        return Ok(()); // avoid a self-inflicted dup-PK
    }
    let account = *s
        .ledger
        .item_account
        .get(&item)
        .expect("item has an owning account");
    let sql = format!("INSERT INTO item_label VALUES ({item}, {account}, '{label}')");
    run(c, &sql).await?;
    s.ledger.add_label(item, label.to_string());
    Ok(())
}

async fn op_update_account(s: &mut Soak, c: &Client) -> Result<(), String> {
    let Some(id) = s.ledger.accounts.pick(&mut s.rng) else {
        return Ok(());
    };
    // Touch an indexed column (created_at + the partial-index-driving status) and a
    // non-indexed one (meta) — exercises b-tree + partial-index maintenance.
    let status = STATUSES[(s.rng.below(3)) as usize];
    let ts = ts_literal(s.ops as i64 + id);
    let sql = format!(
        "UPDATE account SET status='{status}', created_at='{ts}', meta='{{\"u\":{}}}'::jsonb WHERE id={id}",
        s.ops
    );
    run(c, &sql).await // PK set unchanged → ledger untouched
}

async fn op_update_item(s: &mut Soak, c: &Client) -> Result<(), String> {
    let Some(id) = s.ledger.items.pick(&mut s.rng) else {
        return Ok(());
    };
    // Touch the GIN-indexed doc + non-indexed amount/kind.
    let kind = KIND_POOL[(s.rng.below(KIND_POOL.len() as u64)) as usize];
    let tag = TAG_POOL[(s.rng.below(TAG_POOL.len() as u64)) as usize];
    let amount = format!("{}.{:02}", s.ops.rem_euclid(100_000), s.ops.rem_euclid(100));
    let doc = format!("'{{\"kind\":\"{kind}\",\"tag\":\"{tag}\"}}'::jsonb");
    let sql = format!("UPDATE item SET amount={amount}, kind='{kind}', doc={doc} WHERE id={id}");
    run(c, &sql).await
}

async fn op_delete_account(s: &mut Soak, c: &Client) -> Result<(), String> {
    let Some(id) = s.ledger.accounts.pick(&mut s.rng) else {
        return Ok(());
    };
    run(c, &format!("DELETE FROM account WHERE id={id}")).await?;
    s.ledger.del_account(id); // transitive cascade in the ledger too
    Ok(())
}

async fn op_delete_item(s: &mut Soak, c: &Client) -> Result<(), String> {
    let Some(id) = s.ledger.items.pick(&mut s.rng) else {
        return Ok(());
    };
    run(c, &format!("DELETE FROM item WHERE id={id}")).await?;
    s.ledger.del_item(id);
    Ok(())
}

async fn op_select_point(s: &mut Soak, c: &Client) -> Result<(), String> {
    let Some(id) = s.ledger.accounts.pick(&mut s.rng) else {
        return Ok(());
    };
    run(
        c,
        &format!("SELECT id, email, status FROM account WHERE id={id}"),
    )
    .await
}

async fn op_select_range(s: &mut Soak, c: &Client) -> Result<(), String> {
    let lo = ts_literal(s.rng.below(60_000_000) as i64);
    let hi = ts_literal(s.rng.below(60_000_000) as i64 + 30_000_000);
    run(
        c,
        &format!("SELECT count(*) FROM account WHERE created_at >= '{lo}' AND created_at < '{hi}'"),
    )
    .await
}

async fn op_select_gin(s: &mut Soak, c: &Client) -> Result<(), String> {
    let tag = TAG_POOL[(s.rng.below(TAG_POOL.len() as u64)) as usize];
    let kind = KIND_POOL[(s.rng.below(KIND_POOL.len() as u64)) as usize];
    run(
        c,
        &format!("SELECT count(*) FROM account WHERE tags @> ARRAY['{tag}']::text[]"),
    )
    .await?;
    run(
        c,
        &format!("SELECT count(*) FROM item WHERE doc @> '{{\"kind\":\"{kind}\"}}'::jsonb"),
    )
    .await
}

/// A whole-table aggregate — deliberately materializes the table (the memory cliff).
async fn op_aggregate(_s: &mut Soak, c: &Client) -> Result<(), String> {
    run(c, "SELECT count(*), sum(amount) FROM item").await
}

/// A `BEGIN … inserts … COMMIT` burst; ledger updated only on the acked COMMIT.
async fn op_txn_commit(s: &mut Soak, c: &Client) -> Result<(), String> {
    run(c, "BEGIN").await?;
    let n = 2 + s.rng.below(3);
    let mut staged = Vec::new();
    for _ in 0..n {
        let id = s.fresh_account();
        let sql = insert_account_sql(id, &mut s.rng);
        if let Err(e) = run(c, &sql).await {
            let _ = run(c, "ROLLBACK").await;
            return Err(format!("txn-commit burst insert failed: {e}"));
        }
        staged.push(id);
    }
    run(c, "COMMIT").await?;
    for id in staged {
        s.ledger.add_account(id);
    }
    Ok(())
}

/// A `BEGIN … inserts … ROLLBACK` burst; the ledger is never touched (discarded).
async fn op_txn_rollback(s: &mut Soak, c: &Client) -> Result<(), String> {
    run(c, "BEGIN").await?;
    let n = 2 + s.rng.below(3);
    for _ in 0..n {
        let id = s.fresh_account();
        let sql = insert_account_sql(id, &mut s.rng);
        if let Err(e) = run(c, &sql).await {
            let _ = run(c, "ROLLBACK").await;
            return Err(format!("txn-rollback burst insert failed: {e}"));
        }
    }
    run(c, "ROLLBACK").await
}

/// Abrupt mid-`BEGIN` socket drop: open a fresh conn,
/// `BEGIN`, write, then drop the client without committing. The write is
/// uncommitted, so it must never persist — the ledger is untouched.
async fn op_midbegin_drop(s: &mut Soak, port: u16) -> Result<(), String> {
    let drop_client = connect(port).await;
    if run(&drop_client, "BEGIN").await.is_err() {
        return Ok(());
    }
    let id = s.fresh_account();
    let sql = insert_account_sql(id, &mut s.rng);
    let _ = run(&drop_client, &sql).await;
    drop(drop_client); // socket drop → server rolls the txn back
    Ok(())
}

/// Bulk-load the working set to `target` using one **multi-row** `INSERT` per chunk
/// (one round-trip + one commit per chunk — per-row round-trips otherwise dominate
/// wall time): `target` accounts, then ~`target` items over them, then ~`target/2`
/// labels. Each acked chunk is folded into the ledger. This grows the tier the churn
/// loop then holds ~steady, so the kill/audit cycles run over a real dataset.
async fn bulk_grow(s: &mut Soak, c: &Client, target: u64) -> Result<(), String> {
    const CHUNK: u64 = 500;

    // Accounts.
    let mut remaining = target;
    while remaining > 0 {
        let n = CHUNK.min(remaining);
        let mut tuples = Vec::with_capacity(n as usize);
        let mut staged = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let id = s.fresh_account();
            tuples.push(account_tuple(id, &mut s.rng));
            staged.push(id);
        }
        run(
            c,
            &format!("INSERT INTO account VALUES {}", tuples.join(",")),
        )
        .await?;
        for id in staged {
            s.ledger.add_account(id);
        }
        remaining -= n;
    }

    // Items over existing (committed) accounts.
    remaining = target;
    while remaining > 0 {
        let n = CHUNK.min(remaining);
        let mut tuples = Vec::with_capacity(n as usize);
        let mut staged: Vec<(i64, i64)> = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let Some(account) = s.ledger.accounts.pick(&mut s.rng) else {
                break;
            };
            let id = s.fresh_item();
            tuples.push(item_tuple(id, account, &mut s.rng));
            staged.push((id, account));
        }
        if tuples.is_empty() {
            break;
        }
        run(c, &format!("INSERT INTO item VALUES {}", tuples.join(","))).await?;
        for (id, account) in staged {
            s.ledger.add_item(id, account);
        }
        remaining -= n;
    }

    // Labels over existing items (deduping within the chunk to avoid a self-inflicted
    // dup PK; the whole chunk is one atomic multi-row INSERT).
    remaining = target / 2;
    while remaining > 0 {
        let n = CHUNK.min(remaining);
        let mut tuples = Vec::new();
        let mut staged: Vec<(i64, String)> = Vec::new();
        for _ in 0..n {
            let Some(item) = s.ledger.items.pick(&mut s.rng) else {
                break;
            };
            let label = LABEL_POOL[(s.rng.below(LABEL_POOL.len() as u64)) as usize];
            if s.ledger.has_label(item, label)
                || staged.iter().any(|(i, l)| *i == item && l == label)
            {
                continue;
            }
            let account = *s.ledger.item_account.get(&item).expect("item has account");
            tuples.push(format!("({item}, {account}, '{label}')"));
            staged.push((item, label.to_string()));
        }
        if tuples.is_empty() {
            remaining = remaining.saturating_sub(n);
            continue;
        }
        run(
            c,
            &format!("INSERT INTO item_label VALUES {}", tuples.join(",")),
        )
        .await?;
        for (item, label) in staged {
            s.ledger.add_label(item, label);
        }
        remaining -= n;
    }

    Ok(())
}

/// One weighted churn step. The mix shifts by phase: while below the working-set
/// target the dataset grows (insert-heavy); at/above it holds ~steady (insert ≈
/// delete). Returns `Err` on any *unexpected* wire failure.
async fn churn_step(s: &mut Soak, c: &Client, port: u16, target: u64) -> Result<(), String> {
    let growing = (s.ledger.account_count() as u64) < target;
    let r = s.rng.below(100);
    if growing {
        match r {
            0..=34 => op_insert_account(s, c).await,
            35..=59 => op_insert_item(s, c).await,
            60..=71 => op_insert_label(s, c).await,
            72..=79 => op_update_account(s, c).await,
            80..=85 => op_update_item(s, c).await,
            86..=89 => op_select_point(s, c).await,
            90..=92 => op_select_range(s, c).await,
            93..=95 => op_select_gin(s, c).await,
            96..=97 => op_aggregate(s, c).await,
            98 => op_txn_commit(s, c).await,
            _ => op_midbegin_drop(s, port).await,
        }
    } else {
        match r {
            0..=11 => op_insert_account(s, c).await,
            12..=23 => op_insert_item(s, c).await,
            24..=29 => op_insert_label(s, c).await,
            30..=41 => op_delete_account(s, c).await,
            42..=53 => op_delete_item(s, c).await,
            54..=65 => op_update_account(s, c).await,
            66..=73 => op_update_item(s, c).await,
            74..=79 => op_select_point(s, c).await,
            80..=84 => op_select_range(s, c).await,
            85..=89 => op_select_gin(s, c).await,
            90..=93 => op_aggregate(s, c).await,
            94..=96 => op_txn_commit(s, c).await,
            97..=98 => op_txn_rollback(s, c).await,
            _ => op_midbegin_drop(s, port).await,
        }
    }
}

/// Continuous wire invariants: sampled counts match the ledger, read-your-writes
/// holds, and each constraint class fails with its exact SQLSTATE.
async fn run_invariants(c: &Client, s: &Soak) -> Result<(), String> {
    // Full account SET comparison over the wire (not just count(*)): a symmetric drift
    // — one row vanishing while another duplicates — nets the same count yet a
    // different set, so a count check would pass live and only surface at the next
    // kill/audit. `SELECT id … ORDER BY id` returns the ids sorted; compare to the
    // sorted ledger set. (One round-trip; item/label keep the cheaper count check.)
    let msgs = c
        .simple_query("SELECT id FROM account ORDER BY id")
        .await
        .map_err(|e| format!("account id scan: {e}"))?;
    let mut recovered: Vec<i64> = Vec::new();
    for m in msgs {
        if let SimpleQueryMessage::Row(r) = m {
            let id = r
                .get(0)
                .and_then(|v| v.parse::<i64>().ok())
                .ok_or_else(|| "account id: unparseable".to_string())?;
            recovered.push(id);
        }
    }
    let mut ledger_ids: Vec<i64> = s.ledger.accounts.ids.clone();
    ledger_ids.sort_unstable();
    if recovered != ledger_ids {
        return Err(format!(
            "account set drift: server has {} ids, ledger {} (symmetric drift or duplicate)",
            recovered.len(),
            ledger_ids.len()
        ));
    }
    let it = scalar_i64(c, "SELECT count(*) FROM item").await?;
    if it != s.ledger.item_count() as i64 {
        return Err(format!(
            "item count drift: server {it} != ledger {}",
            s.ledger.item_count()
        ));
    }
    let lb = scalar_i64(c, "SELECT count(*) FROM item_label").await?;
    if lb != s.ledger.label_count() as i64 {
        return Err(format!(
            "item_label count drift: server {lb} != ledger {}",
            s.ledger.label_count()
        ));
    }

    if let Some(id) = s.ledger.accounts.first() {
        // Read-your-writes: an acked-live account is present.
        let n = scalar_i64(c, &format!("SELECT count(*) FROM account WHERE id={id}")).await?;
        if n != 1 {
            return Err(format!(
                "read-your-writes: account {id} not found (got {n})"
            ));
        }
        // Duplicate PK on an existing id → 23505 (email/status are valid, so the PK trips).
        expect_sqlstate(
            c,
            &format!(
                "INSERT INTO account VALUES ({id}, 'dup@x.com', 'active', NULL, NULL, '2024-01-01 00:00:00+00')"
            ),
            "23505",
        )
        .await?;
    }
    // FK orphan (a non-existent account) → 23503.
    expect_sqlstate(
        c,
        "INSERT INTO item VALUES (777777777, 888888888, 'k', 1.00, NULL, '2024-01-01 00:00:00+00')",
        "23503",
    )
    .await?;
    // CHECK violation (bad status; a fresh id/email so only the CHECK trips) → 23514.
    let bogus = s.next_account_id + 7;
    expect_sqlstate(
        c,
        &format!(
            "INSERT INTO account VALUES ({bogus}, 'chk@x.com', 'bogus', NULL, NULL, '2024-01-01 00:00:00+00')"
        ),
        "23514",
    )
    .await?;
    Ok(())
}

// ============================================================================
// Error classification + in-process audit (Leg A, post-kill)
// ============================================================================

/// A soak finding, classified so an unattended run never reads a harness/environment
/// flake as an engine bug (or vice-versa).
enum SoakError {
    /// A genuine durability / consistency / atomicity / invariant violation — a bug
    /// in the engine under test. Exits `1`.
    Invariant(String),
    /// A setup / spawn / reopen / port failure — the harness or environment, not the
    /// engine. Exits `2`.
    Infra(String),
}

impl std::fmt::Display for SoakError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invariant(m) | Self::Infra(m) => f.write_str(m),
        }
    }
}

/// A `String` finding defaults to an invariant violation; infra failures are wrapped
/// explicitly (`SoakError::Infra`). Lets the legs `?`-propagate the internal
/// String-returning checks straight into a classified error.
impl From<String> for SoakError {
    fn from(m: String) -> Self {
        Self::Invariant(m)
    }
}

/// An in-flight (fired-but-unresolved-at-kill) multi-row account `INSERT`, for the
/// straddling-kill atomicity check. Its rows are **optional** survivors — *allowed*
/// but not *required* — unless the client saw the ack (`acked`), in which case they
/// MUST have survived. Either way, recovery must be all-or-nothing (never partial).
struct InFlight {
    accounts: HashSet<i64>,
    acked: bool,
}

/// The stats snapshot the audit records for the recorder / summary.
#[derive(Debug, Clone, Copy)]
struct AuditStats {
    active_snapshots: usize,
    pending_gc_pages: usize,
    free_list_size: usize,
    account_rows: usize,
    item_rows: usize,
    label_rows: usize,
    /// `Some(true)` if the in-flight batch fully survived recovery, `Some(false)` if
    /// it was fully rolled back, `None` for a quiescent (non-straddling) kill.
    inflight_survived: Option<bool>,
}

const fn as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Int8(n) => Some(*n),
        _ => None,
    }
}

const fn as_text(v: &Value) -> Option<&str> {
    match v {
        Value::Text(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Reopen the killed store's data-dir **in-process** and run the full audit. A
/// failed reopen is an **infra** error; every check violation is an **invariant**
/// finding. `inflight` (when a straddling kill fired one) makes the audit assert the
/// in-flight batch recovered atomically (all-or-nothing) and reports its survival.
fn audit(data_dir: &Path, s: &Soak, inflight: Option<&InFlight>) -> Result<AuditStats, SoakError> {
    // `Database::open` reopens the double-meta store, recovers the catalog, AND
    // reattaches the persisted indexes — so `index_defs()` is populated for the
    // consistency check. The server holds no lock at the embed layer, and it is
    // dead here anyway. A failed reopen is treated as infra (a corrupt-image bug
    // would surface as a torn recovery below, which this harness never provokes —
    // kill -9 leaves a recoverable image by the double-meta contract).
    let db = Database::open(data_dir)
        .map_err(|e| SoakError::Infra(format!("reopen after kill: {e}")))?;
    let (_schema, backend) = db.into_parts();
    let StorageBackend::Btree(wrapper) = backend else {
        return Err(SoakError::Infra(
            "expected a B+tree backend after reopen".into(),
        ));
    };

    let stats = wrapper.stats();
    // Opens clean: the recovered stats must be sane (a wild free-list / pending-GC
    // count would signal a corrupt reopen).
    if stats.pending_gc_pages > stats.page_count.saturating_add(1)
        || stats.free_list_size > stats.page_count.saturating_add(1)
    {
        return Err(SoakError::Invariant(format!(
            "reopen stats look corrupt: pending_gc={} free_list={} page_count={}",
            stats.pending_gc_pages, stats.free_list_size, stats.page_count
        )));
    }

    check_index_consistency(&wrapper).map_err(SoakError::Invariant)?;
    let inflight_survived =
        check_durability(&wrapper, s, inflight).map_err(SoakError::Invariant)?;
    check_referential(&wrapper).map_err(SoakError::Invariant)?;

    let account_rows = wrapper
        .scan("account")
        .map_err(|e| SoakError::Invariant(e.to_string()))?
        .len();
    let item_rows = wrapper
        .scan("item")
        .map_err(|e| SoakError::Invariant(e.to_string()))?
        .len();
    let label_rows = wrapper
        .scan("item_label")
        .map_err(|e| SoakError::Invariant(e.to_string()))?
        .len();

    Ok(AuditStats {
        active_snapshots: stats.active_snapshots,
        pending_gc_pages: stats.pending_gc_pages,
        free_list_size: stats.free_list_size,
        account_rows,
        item_rows,
        label_rows,
        inflight_survived,
    })
    // `wrapper` drops here → the file handle closes before the server respawns.
}

/// Every index is consistent with its table: no orphan entry (→ dead row), no
/// missing entry (a predicate-satisfying live row without its keys), and no phantom
/// (the band entry count equals the expected encoded-key count). Covers b-tree,
/// partial b-tree, and GIN indexes uniformly.
fn check_index_consistency(w: &BtreeStorageWrapper) -> Result<(), String> {
    let root = w.inner().current_root();
    for (index_id, def) in w.index_defs() {
        let table = def.table_name.as_str();

        // ONE table scan → the live row-id set + the set of index entries the live rows
        // *should* produce, each as the fully-encoded `BtreeKey` bytes (the key repr). That
        // encoding is byte-identical to what the engine stores: maintenance inserts
        // `BtreeKey::index(index_id, encode_row_keys(row)[i], row_id)`, so reconstructing
        // the same `BtreeKey` and encoding it reproduces the band key exactly. A GIN row
        // that decomposes to a duplicate key collapses in the set — the b-tree stores one
        // entry for it too, so both sides agree. Keyed by key repr → row_id so a missing
        // entry can name its row. This is O(N) hashing, replacing the old O(N²) per-row
        // `index_scan_at_root` probe (each GIN probe returned O(N) row-ids).
        let rows = w.scan(table).map_err(|e| format!("scan {table}: {e}"))?;
        let live_rids: HashSet<u64> = rows.iter().map(|(rid, _)| *rid).collect();
        let mut expected: HashMap<Vec<u8>, u64> = HashMap::new();
        for (rid, row) in &rows {
            if def
                .row_in_index(row)
                .map_err(|e| format!("index {index_id}: row_in_index: {e}"))?
            {
                for enc in def
                    .keying
                    .encode_row_keys(row)
                    .map_err(|e| format!("index {index_id}: encode_row_keys: {e}"))?
                {
                    expected.insert(BtreeKey::index(*index_id, &enc, *rid).encode(), *rid);
                }
            }
        }

        // ONE band scan → the orphan check (every entry's row-id must be live) + the
        // band's own key-repr set (each band key encoded the same way).
        let entries = w
            .inner()
            .range_scan_at_root(
                root,
                &BtreeKey::index_table_start(*index_id),
                &BtreeKey::index_table_end(*index_id),
            )
            .map_err(|e| format!("index {index_id} band scan: {e}"))?;
        let mut band: HashSet<Vec<u8>> = HashSet::with_capacity(entries.len());
        for (key, _) in &entries {
            let rid = key.row_id();
            if !live_rids.contains(&rid) {
                return Err(format!(
                    "index {index_id} ({table}): orphan entry → dead row_id {rid}"
                ));
            }
            band.insert(key.encode());
        }

        // Set comparison (O(1) per element):
        // (a) MISSING — every expected entry (exact value AND row) must be in the band
        //     (subsumes the old per-key probe: a hit for `(enc, rid)` == membership here).
        for (key_repr, rid) in &expected {
            if !band.contains(key_repr) {
                return Err(format!(
                    "index {index_id} ({table}): live row {rid} missing its index entry"
                ));
            }
        }
        // (b) PHANTOM / count — the raw band count equals the expected set size. With
        //     (a) this is exact set equality: an extra band entry (a phantom, or an
        //     entry a stale partial-index update failed to shed) makes the band larger.
        if entries.len() != expected.len() {
            return Err(format!(
                "index {index_id} ({table}): band has {} entries, expected {} \
                 (phantom or missing entries)",
                entries.len(),
                expected.len()
            ));
        }
    }
    Ok(())
}

/// The recovered live-PK set per table brackets the ledger:
/// - **items / labels** are untouched by an in-flight *account* batch, so they must
///   equal the ledger exactly (a lost acked commit fails `⊇`, a phantom fails `⊆`).
/// - **accounts** must contain the committed ledger base (`⊇`) and nothing beyond
///   base ∪ in-flight (`⊆`). The in-flight batch is *optional* (allowed, not
///   required) unless it was acked — and it must recover **atomically**: fully
///   present or fully absent, never partial.
///
/// Returns `Some(true/false)` for the in-flight batch's survival (for the caller to
/// reconcile the ledger), or `None` for a quiescent kill.
fn check_durability(
    w: &BtreeStorageWrapper,
    s: &Soak,
    inflight: Option<&InFlight>,
) -> Result<Option<bool>, String> {
    let recovered_acc: HashSet<i64> = pk_set_i64(w, "account", 0)?;
    let recovered_item: HashSet<i64> = pk_set_i64(w, "item", 0)?;
    let recovered_lbl: HashSet<(i64, String)> = {
        let rows = w.scan("item_label").map_err(|e| e.to_string())?;
        let mut set = HashSet::new();
        for (_, row) in &rows {
            let item = as_i64(&row[0]).ok_or("item_label.item_id not int8")?;
            let label = as_text(&row[2])
                .ok_or("item_label.label not text")?
                .to_string();
            set.insert((item, label));
        }
        set
    };

    // Items / labels: the in-flight batch never touches them, so exact match holds.
    compare_sets("item", &recovered_item, &s.ledger.item_set())?;
    compare_label_sets(&recovered_lbl, &s.ledger.label_set())?;

    let base = s.ledger.account_set();
    let Some(inf) = inflight else {
        // Quiescent kill: accounts equal the ledger exactly, both directions.
        compare_sets("account", &recovered_acc, &base)?;
        return Ok(None);
    };

    // Straddling kill — atomicity: the in-flight batch is all-in or all-out.
    let present = inf
        .accounts
        .iter()
        .filter(|id| recovered_acc.contains(id))
        .count();
    let survived = if present == 0 {
        false
    } else if present == inf.accounts.len() {
        true
    } else {
        return Err(format!(
            "NON-ATOMIC recovery: in-flight account batch partially applied ({present} of {} rows survived a kill -9)",
            inf.accounts.len()
        ));
    };
    // Acked ⇒ durable: if the client saw the commit's ack, fsync #2 passed, so the
    // whole batch MUST have survived.
    if inf.acked && !survived {
        return Err(format!(
            "acked in-flight commit LOST after kill -9 ({} rows, all absent) — durability violation",
            inf.accounts.len()
        ));
    }
    // Durability: every committed ledger-base account must be present.
    for id in &base {
        if !recovered_acc.contains(id) {
            return Err(format!(
                "account {id}: acked+fsynced commit LOST after kill -9 (durability violation)"
            ));
        }
    }
    // No phantom: every recovered account is either committed-base or the in-flight batch.
    for id in &recovered_acc {
        if !base.contains(id) && !inf.accounts.contains(id) {
            return Err(format!(
                "account {id}: uncommitted/phantom row SURVIVED the kill (atomicity violation)"
            ));
        }
    }
    Ok(Some(survived))
}

fn compare_sets(
    table: &str,
    recovered: &HashSet<i64>,
    ledger: &HashSet<i64>,
) -> Result<(), String> {
    for id in ledger {
        if !recovered.contains(id) {
            return Err(format!(
                "{table} {id}: acked+fsynced commit LOST after kill -9 (durability violation)"
            ));
        }
    }
    for id in recovered {
        if !ledger.contains(id) {
            return Err(format!(
                "{table} {id}: uncommitted/phantom row SURVIVED the kill (atomicity violation)"
            ));
        }
    }
    Ok(())
}

fn compare_label_sets(
    recovered: &HashSet<(i64, String)>,
    ledger: &HashSet<(i64, String)>,
) -> Result<(), String> {
    for k in ledger {
        if !recovered.contains(k) {
            return Err(format!(
                "item_label {k:?}: acked+fsynced commit LOST after kill -9 (durability violation)"
            ));
        }
    }
    for k in recovered {
        if !ledger.contains(k) {
            return Err(format!(
                "item_label {k:?}: uncommitted/phantom row SURVIVED the kill (atomicity violation)"
            ));
        }
    }
    Ok(())
}

/// Global referential integrity via direct scans: no duplicate PK, no FK orphan.
fn check_referential(w: &BtreeStorageWrapper) -> Result<(), String> {
    let acc_ids = pk_set_i64(w, "account", 0)?;
    let account_rows = w.scan("account").map_err(|e| e.to_string())?;
    if account_rows.len() != acc_ids.len() {
        return Err("duplicate account PK detected".into());
    }

    let item_rows = w.scan("item").map_err(|e| e.to_string())?;
    let mut item_pk = HashSet::new();
    let mut item_account: HashMap<i64, i64> = HashMap::new();
    for (_, row) in &item_rows {
        let id = as_i64(&row[0]).ok_or("item.id not int8")?;
        let acct = as_i64(&row[1]).ok_or("item.account_id not int8")?;
        if !item_pk.insert(id) {
            return Err(format!("duplicate item PK {id}"));
        }
        if !acc_ids.contains(&acct) {
            return Err(format!("item {id}: FK orphan — account_id {acct} absent"));
        }
        item_account.insert(id, acct);
    }

    let label_rows = w.scan("item_label").map_err(|e| e.to_string())?;
    let mut label_pk = HashSet::new();
    for (_, row) in &label_rows {
        let item = as_i64(&row[0]).ok_or("item_label.item_id not int8")?;
        let acct = as_i64(&row[1]).ok_or("item_label.account_id not int8")?;
        let label = as_text(&row[2])
            .ok_or("item_label.label not text")?
            .to_string();
        if !label_pk.insert((item, label.clone())) {
            return Err(format!("duplicate item_label PK ({item},{label})"));
        }
        match item_account.get(&item) {
            Some(a) if *a == acct => {}
            _ => {
                return Err(format!(
                    "item_label ({item},{label}): composite FK orphan (item_id,account_id)=({item},{acct})"
                ));
            }
        }
    }
    Ok(())
}

/// The set of `i64` PKs at column `col` of `table` (via a direct storage scan).
fn pk_set_i64(w: &BtreeStorageWrapper, table: &str, col: usize) -> Result<HashSet<i64>, String> {
    let rows = w.scan(table).map_err(|e| format!("scan {table}: {e}"))?;
    let mut set = HashSet::new();
    for (_, row) in &rows {
        let v = row
            .get(col)
            .and_then(as_i64)
            .ok_or_else(|| format!("{table} col {col}: not int8"))?;
        set.insert(v);
    }
    Ok(set)
}

// ============================================================================
// Recorder
// ============================================================================

/// One time-series sample, appended (flushed) as an ndjson line under `target/`.
struct Sample {
    leg: &'static str,
    elapsed_secs: f64,
    data_file_bytes: u64,
    server_rss_kb: u64,
    server_rss_cur_kb: u64,
    ops_completed: u64,
    /// Live dataset row counts, so a memory-cliff RSS-vs-dataset-size analysis is
    /// derivable straight from the recorder (the RSS fields alone can't give it).
    account_rows: usize,
    item_rows: usize,
    label_rows: usize,
    active_snapshots: Option<usize>,
    pending_gc_pages: Option<usize>,
    free_list_size: Option<usize>,
}

impl Sample {
    fn to_json(&self) -> String {
        use std::fmt::Write as _;
        // `wall_ts` mirrors `elapsed_secs` on purpose — a real wall clock would break
        // reproducibility, and the recorder only needs a monotonic axis.
        let mut s = format!(
            "{{\"leg\":\"{}\",\"elapsed_secs\":{:.3},\"wall_ts\":{:.3},\
\"data_file_bytes\":{},\"server_rss_kb\":{},\"server_rss_cur_kb\":{},\"ops_completed\":{},\
\"account_rows\":{},\"item_rows\":{},\"label_rows\":{}",
            self.leg,
            self.elapsed_secs,
            self.elapsed_secs,
            self.data_file_bytes,
            self.server_rss_kb,
            self.server_rss_cur_kb,
            self.ops_completed,
            self.account_rows,
            self.item_rows,
            self.label_rows,
        );
        if let Some(v) = self.active_snapshots {
            let _ = write!(s, ",\"active_snapshots\":{v}");
        }
        if let Some(v) = self.pending_gc_pages {
            let _ = write!(s, ",\"pending_gc_pages\":{v}");
        }
        if let Some(v) = self.free_list_size {
            let _ = write!(s, ",\"free_list_size\":{v}");
        }
        s.push('}');
        s
    }
}

/// Appends samples to a gitignored ndjson file under `target/`, flushing each write
/// so an interrupted run still leaves data.
struct Recorder {
    file: Option<std::fs::File>,
    path: PathBuf,
}

impl Recorder {
    fn new() -> Self {
        let _ = std::fs::create_dir_all("target");
        let path = PathBuf::from("target/soak_samples.ndjson");
        let file = std::fs::File::create(&path).ok();
        Self { file, path }
    }

    fn record(&mut self, sample: &Sample) {
        if let Some(f) = self.file.as_mut() {
            let _ = writeln!(f, "{}", sample.to_json());
            let _ = f.flush();
        }
    }
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

// ============================================================================
// Leg A driver
// ============================================================================

/// The accumulated Leg A outcome, for the summary + results file.
#[derive(Debug, Default, Clone)]
struct LegAReport {
    kill_audit_cycles: u64,
    straddle_kills: u64,
    invariant_batches: u64,
    final_ops: u64,
    peak_data_file_bytes: u64,
    peak_server_rss_kb: u64,
    last_audit: Option<(usize, usize, usize)>, // (accounts, items, labels)
}

/// How many fresh accounts the straddling-kill in-flight `INSERT` carries — large
/// enough that its commit takes long enough to straddle a randomized kill.
const INFLIGHT_ROWS: usize = 400;

async fn leg_a(config: &Config, rec: &mut Recorder) -> Result<LegAReport, SoakError> {
    let scratch = Scratch::new("lega");
    let range = PGLET_TEST_PORT..DYNAMIC_PORT_END;

    let (mut server, mut port) = spawn_ready(range.clone(), Some(&scratch.path))
        .await
        .map_err(SoakError::Infra)?;
    let mut client = connect(port).await;
    run(&client, SCHEMA_SQL)
        .await
        .map_err(|e| SoakError::Invariant(format!("schema: {e}")))?;

    let mut s = Soak::new(config.seed);
    let mut report = LegAReport::default();

    // Grow the tier fast (batched), then baseline the continuous invariants over it.
    bulk_grow(&mut s, &client, config.rows_target).await?;
    run_invariants(&client, &s).await?;
    report.invariant_batches += 1;
    eprintln!(
        "[soak] bulk-grew tier: accounts={} items={} labels={}",
        s.ledger.account_count(),
        s.ledger.item_count(),
        s.ledger.label_count()
    );

    let start = Instant::now();
    let mut last_kill = Instant::now();
    let mut last_sample = Instant::now();
    let mut last_invariant_ops = 0u64;

    // Churn until the duration elapses, killing + auditing on the interval; then do
    // one final kill + audit so the very last state is always validated.
    loop {
        for _ in 0..BATCH {
            churn_step(&mut s, &client, port, config.rows_target).await?;
            s.ops += 1;
        }

        if s.ops - last_invariant_ops >= INVARIANT_EVERY {
            run_invariants(&client, &s).await?;
            report.invariant_batches += 1;
            last_invariant_ops = s.ops;
        }

        if last_sample.elapsed() >= SAMPLE_INTERVAL {
            let df = file_len(&scratch.data_file());
            let rss = read_status_kb(Some(server.id()), "VmHWM").unwrap_or(0);
            let cur = read_status_kb(Some(server.id()), "VmRSS").unwrap_or(0);
            report.peak_data_file_bytes = report.peak_data_file_bytes.max(df);
            report.peak_server_rss_kb = report.peak_server_rss_kb.max(rss);
            rec.record(&Sample {
                leg: "a",
                elapsed_secs: start.elapsed().as_secs_f64(),
                data_file_bytes: df,
                server_rss_kb: rss,
                server_rss_cur_kb: cur,
                ops_completed: s.ops,
                account_rows: s.ledger.account_count(),
                item_rows: s.ledger.item_count(),
                label_rows: s.ledger.label_count(),
                active_snapshots: None,
                pending_gc_pages: None,
                free_list_size: None,
            });
            last_sample = Instant::now();
        }

        let time_up = start.elapsed() >= config.duration;
        if last_kill.elapsed() >= config.kill_every || time_up {
            // Validate over the wire one last time while the state is still QUIESCENT
            // (before any in-flight write), so a drift is attributed to churn.
            run_invariants(&client, &s).await?;
            report.invariant_batches += 1;
            last_invariant_ops = s.ops;

            // Alternate: even cycles STRADDLE an in-flight commit (atomicity), odd ones
            // are quiescent (consistency + durability == ledger). The default's first
            // kill straddles, guaranteeing >= 1 straddling kill per run.
            let straddle = report.kill_audit_cycles % 2 == 0;

            // A straddling kill fires a LARGE multi-row account INSERT on a SEPARATE
            // connection WITHOUT awaiting it, then kills after a seeded random delay so
            // the kill lands both before and after the commit point across seeds
            // (mirrors `test_crash` phase B, over the rich FK'd/indexed schema).
            let inflight = if straddle {
                let mut ids = Vec::with_capacity(INFLIGHT_ROWS);
                let mut tuples = Vec::with_capacity(INFLIGHT_ROWS);
                for _ in 0..INFLIGHT_ROWS {
                    let id = s.fresh_account();
                    tuples.push(account_tuple(id, &mut s.rng));
                    ids.push(id);
                }
                let sql = format!("INSERT INTO account VALUES {}", tuples.join(","));
                // Small base so the statement is reliably in flight; a spread wide
                // enough to land BOTH before the commit point (kill mid-apply → the
                // batch recovers fully absent) AND after it (acked → fully present),
                // so both atomicity branches get exercised across seeds/cycles. The
                // window is generous on purpose — the commit is fsync-bound and the
                // 400-row apply over three indexes is not instantaneous.
                let delay_ms = 2 + s.rng.below(400);
                let inflight_client = connect(port).await;
                let handle = tokio::spawn(async move { inflight_client.execute(&sql, &[]).await });
                Some((ids, handle, delay_ms))
            } else {
                None
            };

            drop(client);

            let (inflight_ids, acked) = if let Some((ids, handle, delay_ms)) = inflight {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                hard_kill(server);
                // Did the client read the commit's ack before the process died? The
                // server sends CommandComplete only past fsync #2, so acked ⇒ durable.
                let acked = matches!(handle.await, Ok(Ok(_)));
                (Some(ids), acked)
            } else {
                hard_kill(server);
                (None, false)
            };

            let inflight_arg = inflight_ids.as_ref().map(|ids| InFlight {
                accounts: ids.iter().copied().collect(),
                acked,
            });

            let audit_start = Instant::now();
            let stats = audit(&scratch.path, &s, inflight_arg.as_ref())?;
            let audit_ms = audit_start.elapsed().as_millis();
            report.kill_audit_cycles += 1;
            if straddle {
                report.straddle_kills += 1;
            }

            // Reconcile: if the in-flight batch survived, it is now committed — fold it
            // into the ledger so subsequent audits match. Its atomicity / subset /
            // superset were already validated by `audit`.
            if let (Some(ids), Some(true)) = (&inflight_ids, stats.inflight_survived) {
                for id in ids {
                    s.ledger.add_account(*id);
                }
            }

            report.last_audit = Some((stats.account_rows, stats.item_rows, stats.label_rows));
            let mode = match (straddle, stats.inflight_survived) {
                (true, Some(true)) => format!("straddle: in-flight batch survived (acked={acked})"),
                (true, Some(false)) => {
                    format!("straddle: in-flight batch rolled back (acked={acked})")
                }
                (true, None) => "straddle".to_string(),
                (false, _) => "quiescent".to_string(),
            };
            eprintln!(
                "[soak] kill/audit #{} at {} ops [{mode}]: accounts={} items={} labels={} \
                 (audit={audit_ms}ms active_snapshots={} pending_gc={} free_list={})",
                report.kill_audit_cycles,
                s.ops,
                stats.account_rows,
                stats.item_rows,
                stats.label_rows,
                stats.active_snapshots,
                stats.pending_gc_pages,
                stats.free_list_size,
            );
            rec.record(&Sample {
                leg: "a",
                elapsed_secs: start.elapsed().as_secs_f64(),
                data_file_bytes: file_len(&scratch.data_file()),
                server_rss_kb: 0,
                server_rss_cur_kb: 0,
                ops_completed: s.ops,
                account_rows: stats.account_rows,
                item_rows: stats.item_rows,
                label_rows: stats.label_rows,
                active_snapshots: Some(stats.active_snapshots),
                pending_gc_pages: Some(stats.pending_gc_pages),
                free_list_size: Some(stats.free_list_size),
            });

            if time_up {
                report.final_ops = s.ops;
                return Ok(report);
            }

            let (srv, p) = spawn_ready(range.clone(), Some(&scratch.path))
                .await
                .map_err(SoakError::Infra)?;
            server = srv;
            port = p;
            client = connect(port).await;
            last_kill = Instant::now();
        }
    }
}

// ============================================================================
// Leg B driver (in-process embed, live stats)
// ============================================================================

#[derive(Debug, Default, Clone)]
struct LegBReport {
    pending_peak_pinned: usize,
    pending_after_vacuum: usize,
    free_list_after_vacuum: usize,
    file_after_vacuum_bytes: u64,
    file_after_churn_bytes: u64,
}

/// Assemble a file-backed `BtreeStorageWrapper` directly (mirrors `Database::open`'s
/// stack) so Leg B can drive `execute_with_storage` and hold the wrapper for live
/// `stats()`. Leg B always starts on a fresh file, so the schema comes back empty.
fn open_wrapper(path: &Path) -> Result<(BtreeStorageWrapper, Schema), String> {
    let file_io = FileBlockIo::open(path).map_err(|e| format!("open file block io: {e}"))?;
    let (store, metadata) =
        DoubleMetaStore::open(Box::new(file_io)).map_err(|e| format!("open double-meta: {e}"))?;
    let btree = CowBtree::with_store(Box::new(store), metadata.root, metadata.version, 100);
    let (wrapper, schema_bytes) =
        BtreeStorageWrapper::recover(std::sync::Arc::new(btree), metadata.next_table_id)
            .map_err(|e| format!("recover: {e}"))?;
    if !schema_bytes.is_empty() {
        return Err("leg-b expects a fresh store (recovered a non-empty schema)".into());
    }
    Ok((wrapper, Schema::new()))
}

/// Drive SQL through the executor entry the oracle's `TestDb` uses, statement by
/// statement against the live schema (so dependent DDL sees the prior statement).
/// Autocommit writes land via the wrapper's `apply_batch` — a CoW root swap **with no
/// metadata fsync** — so the churn is fast and its GC/vacuum effects are the point.
fn exec_sql(
    sql: &str,
    w: &mut BtreeStorageWrapper,
    schema: &mut Schema,
    session: &mut SequenceSession,
) -> Result<(), String> {
    for stmt in pglet_sql::parser::split_statements(sql).map_err(|e| format!("split {sql}: {e}"))? {
        if let Some(plan) = pglet_sql::parser::parse_single(stmt, schema)
            .map_err(|e| format!("parse {stmt}: {e}"))?
        {
            execute_with_storage(&plan, schema, w, EvalEnv::EMPTY, session)
                .map_err(|e| format!("exec {stmt}: {e}"))?;
        }
    }
    Ok(())
}

/// One in-process UPDATE (CoW page turnover) on an existing account.
fn update_account_inproc(
    w: &mut BtreeStorageWrapper,
    schema: &mut Schema,
    session: &mut SequenceSession,
    id: i64,
    salt: i64,
) -> Result<(), String> {
    let ts = ts_literal(id.wrapping_add(salt));
    let status = STATUSES[(salt.rem_euclid(3)) as usize];
    let sql = format!(
        "UPDATE account SET created_at='{ts}', status='{status}', meta='{{\"u\":{salt}}}'::jsonb WHERE id={id}"
    );
    exec_sql(&sql, w, schema, session)
}

fn leg_b(config: &Config, rec: &mut Recorder, base_secs: f64) -> Result<LegBReport, SoakError> {
    let scratch = Scratch::new("legb");
    let data_file = scratch.data_file();
    let (mut w, mut schema) = open_wrapper(&data_file).map_err(SoakError::Infra)?;
    let mut session = SequenceSession::new();
    exec_sql(SCHEMA_SQL, &mut w, &mut schema, &mut session)
        .map_err(|e| SoakError::Invariant(format!("leg-b schema: {e}")))?;

    let mut rng = Xorshift::new(config.seed ^ 0x0000_0000_0000_B0B0);
    // Cap the base working set (see `LEG_B_MAX_ROWS`): a pinned CoW churn on a huge tree
    // would balloon the temp data file without adding vacuum-invariant coverage.
    let target = (config.rows_target as i64).min(LEG_B_MAX_ROWS);
    // Bounded churn volume — enough to push pending-GC well past the ~100 auto-vacuum
    // threshold (each CoW update retires several pages; on a large tree even ~200
    // updates clear it), but capped so it doesn't scale with the tier: a pinned update
    // deep-copies its whole CoW path, so unbounded churn adds cost without adding
    // vacuum-invariant coverage.
    let churn: i64 = (target / 16).clamp(200, 500);
    let start = Instant::now();
    let mut report = LegBReport::default();

    // Leg B's working set is `target` accounts only (no items/labels), so the row-count
    // fields record that.
    let account_rows = target as usize;
    let sample = |rec: &mut Recorder, w: &BtreeStorageWrapper, ops: u64| {
        let st = w.stats();
        rec.record(&Sample {
            leg: "b",
            elapsed_secs: base_secs + start.elapsed().as_secs_f64(),
            data_file_bytes: file_len(&data_file),
            server_rss_kb: read_status_kb(None, "VmHWM").unwrap_or(0),
            server_rss_cur_kb: read_status_kb(None, "VmRSS").unwrap_or(0),
            ops_completed: ops,
            account_rows,
            item_rows: 0,
            label_rows: 0,
            active_snapshots: Some(st.active_snapshots),
            pending_gc_pages: Some(st.pending_gc_pages),
            free_list_size: Some(st.free_list_size),
        });
    };

    // B1 — load a bounded working set of accounts (autocommit, no fsync → fast).
    for id in 1..=target {
        let sql = insert_account_sql(id, &mut rng);
        exec_sql(&sql, &mut w, &mut schema, &mut session)
            .map_err(|e| SoakError::Invariant(format!("leg-b load {id}: {e}")))?;
    }
    sample(rec, &w, target as u64);

    // B2 — a committed and a rolled-back transaction each release their snapshot: after
    // each quiesces, active_snapshots must be back to baseline 0. (Driven directly on
    // the wrapper — `execute_with_storage` over the raw wrapper is the autocommit path.)
    w.begin_transaction(1)
        .map_err(|e| SoakError::Invariant(format!("b2 begin: {e}")))?;
    if w.stats().active_snapshots < 1 {
        return Err(SoakError::Invariant(
            "open transaction did not register a snapshot".to_string(),
        ));
    }
    w.commit_transaction(1)
        .map_err(|e| SoakError::Invariant(format!("b2 commit: {e}")))?;
    if w.stats().active_snapshots != 0 {
        return Err(SoakError::Invariant(format!(
            "snapshot leaked after a committed txn: active_snapshots={} (expected 0)",
            w.stats().active_snapshots
        )));
    }
    w.begin_transaction(2)
        .map_err(|e| SoakError::Invariant(format!("b2 begin2: {e}")))?;
    w.rollback_transaction(2)
        .map_err(|e| SoakError::Invariant(format!("b2 rollback: {e}")))?;
    if w.stats().active_snapshots != 0 {
        return Err(SoakError::Invariant(format!(
            "snapshot leaked after a rolled-back txn: active_snapshots={} (expected 0)",
            w.stats().active_snapshots
        )));
    }

    // B3 — the leaked-snapshot fingerprint: an ABANDONED transaction pins a version, so
    // GC can't reclaim while churn advances versions → pending_gc_pages climbs and
    // active_snapshots stays >= 1. This is EXPECTED (a pin legitimately blocks GC), so
    // it is recorded, not failed.
    w.begin_transaction(99)
        .map_err(|e| format!("abandon begin: {e}"))?;
    let mut pending_peak = 0usize;
    for i in 0..churn {
        let id = 1 + rng.below(target as u64) as i64;
        update_account_inproc(&mut w, &mut schema, &mut session, id, i)?;
        if i % 64 == 0 {
            let st = w.stats();
            pending_peak = pending_peak.max(st.pending_gc_pages);
            if st.active_snapshots < 1 {
                return Err(SoakError::Invariant(format!(
                    "abandoned txn's snapshot vanished mid-churn: active_snapshots={} (expected >= 1)",
                    st.active_snapshots
                )));
            }
            sample(rec, &w, target as u64 + i as u64);
        }
    }
    let st3 = w.stats();
    pending_peak = pending_peak.max(st3.pending_gc_pages);
    if st3.active_snapshots < 1 {
        return Err(SoakError::Invariant(format!(
            "abandoned txn released its snapshot on its own: active_snapshots={} (expected >= 1)",
            st3.active_snapshots
        )));
    }
    report.pending_peak_pinned = pending_peak;

    // The drain check below is only meaningful if the pin actually built pending-GC
    // above the ~100 vacuum threshold. Never a silent pass: if a too-small tier didn't
    // build pressure, fail loud (raise the tier) rather than skip the assertion.
    if pending_peak <= 100 {
        return Err(SoakError::Invariant(format!(
            "leg-b churn did not build pending-GC above the ~100 vacuum threshold (peak={pending_peak}); \
             the drain assertion would be vacuous — raise PGLET_SOAK_ROWS"
        )));
    }

    // B4 — release the pin + vacuum: active_snapshots must drain to 0 and the pinned
    // pending-GC pages must MATERIALLY reclaim (a correct engine drains to ~0). A stuck
    // active_snapshots here is the real leaked-snapshot bug; a pending-GC that barely
    // moves is a vacuum that isn't reclaiming.
    w.rollback_transaction(99)
        .map_err(|e| SoakError::Invariant(format!("abandon rollback: {e}")))?;
    let _ = w.vacuum();
    let st4 = w.stats();
    if st4.active_snapshots != 0 {
        return Err(SoakError::Invariant(format!(
            "leaked-snapshot fingerprint: active_snapshots={} after the abandoned txn was rolled back (expected 0)",
            st4.active_snapshots
        )));
    }
    if st4.pending_gc_pages >= pending_peak / 2 {
        return Err(SoakError::Invariant(format!(
            "pending-GC did not materially drain after the pin released: peak={pending_peak}, \
             after-vacuum={} (expected < peak/2; a correct engine drains to ~0)",
            st4.pending_gc_pages
        )));
    }
    if st4.free_list_size == 0 {
        return Err(SoakError::Invariant(format!(
            "free list is empty after vacuuming {pending_peak} pinned pages (no pages reclaimed for reuse)"
        )));
    }
    report.pending_after_vacuum = st4.pending_gc_pages;
    report.free_list_after_vacuum = st4.free_list_size;
    report.file_after_vacuum_bytes = file_len(&data_file);
    sample(rec, &w, target as u64 + churn as u64);

    // B5 — with no pin, an equal churn burst must be absorbed by the free list, so the
    // data file stays bounded (allocations reuse reclaimed pages, not extend the file).
    for i in 0..churn {
        let id = 1 + rng.below(target as u64) as i64;
        update_account_inproc(&mut w, &mut schema, &mut session, id, churn + i)?;
        if i % 128 == 0 {
            let _ = w.vacuum(); // replenish the free list from freshly-retired pages
        }
    }
    let _ = w.vacuum();
    let file_after_churn = file_len(&data_file);
    report.file_after_churn_bytes = file_after_churn;
    // Generous bound: bounded-working-set churn may need a little headroom, but must not
    // grow proportional to the op count (which would be tens of MB).
    let slack = 256 * PAGE_SIZE as u64;
    if file_after_churn > report.file_after_vacuum_bytes + slack {
        return Err(SoakError::Invariant(format!(
            "data file grew unbounded under bounded-working-set churn: {} → {} bytes (free list not absorbing allocations)",
            report.file_after_vacuum_bytes, file_after_churn
        )));
    }
    if w.stats().active_snapshots != 0 {
        return Err(SoakError::Invariant(format!(
            "snapshot leaked after the final unpinned churn: active_snapshots={} (expected 0)",
            w.stats().active_snapshots
        )));
    }
    sample(rec, &w, target as u64 + 2 * churn as u64);

    Ok(report)
}

// ============================================================================
// Leg C driver (server-driven re-migration / index-OID-desync soak)
// ============================================================================

/// The idempotent re-migration DDL — the rich schema issued `IF NOT EXISTS` (mostly
/// no-ops after cycle 1). The **swap-targeted** indexes (`account_active_email_idx`,
/// `account_created_idx`, `account_tags_gin`) are deliberately **not** here: they are
/// created only via the per-cycle drop/recreate swap, so every cycle genuinely cycles
/// their OID (a `CREATE … IF NOT EXISTS` that found the object would be a silent no-op
/// and prove nothing). Mirrors `SCHEMA_SQL`'s three tables + their PK/composite
/// FK/CHECK and the two non-swap indexes — but `email` carries **no** table-level
/// UNIQUE (only `NOT NULL`), so the recreated partial `account_active_email_idx` is the
/// **sole** active-email uniqueness enforcer and the per-cycle `23505` probe is
/// attributable to it (a table-level UNIQUE would shadow it). Seed emails are unique
/// per id, so the partial builds without collision.
const SCHEMA_SQL_IDEMPOTENT: &str = "\
CREATE TABLE IF NOT EXISTS account (id BIGINT PRIMARY KEY, email TEXT NOT NULL, \
  status TEXT NOT NULL CHECK (status IN ('active','suspended','closed')), \
  tags TEXT[], meta JSONB, created_at TIMESTAMPTZ NOT NULL);\n\
CREATE TABLE IF NOT EXISTS item (id BIGINT PRIMARY KEY, \
  account_id BIGINT NOT NULL REFERENCES account(id) ON DELETE CASCADE, \
  kind TEXT NOT NULL, amount NUMERIC(12,2) NOT NULL, doc JSONB, created_at TIMESTAMPTZ NOT NULL, \
  UNIQUE (id, account_id));\n\
CREATE INDEX IF NOT EXISTS item_account_idx ON item (account_id);\n\
CREATE INDEX IF NOT EXISTS item_doc_gin ON item USING gin (doc);\n\
CREATE TABLE IF NOT EXISTS item_label (item_id BIGINT NOT NULL, account_id BIGINT NOT NULL, label TEXT NOT NULL, \
  PRIMARY KEY (item_id, label), \
  FOREIGN KEY (item_id, account_id) REFERENCES item(id, account_id) ON DELETE CASCADE);";

/// The accumulated Leg C outcome, for the summary + results file.
#[derive(Debug, Default, Clone)]
struct LegCReport {
    reset_cycles: u64,
    index_swaps: u64,
    oid_burns: u64,
    table_id_burns: u64,
    correctness_batches: u64,
    /// Within-transaction same-name index swaps on a highest-oid probe (the storage
    /// index_id-reuse desync trigger — the actual regression the index-OID-desync fix closed).
    probe_swaps: u64,
    kill_audit_cycles: u64,
    straddle_kills: u64,
    /// Every distinct storage `index_id` ever observed live across the run (union of the
    /// reopened `schema.indexes` ids at each audit). Grows as swaps burn fresh OIDs —
    /// logged as evidence the swaps are not silent no-ops, not asserted on (a restart
    /// legitimately re-seeds `next_oid` from the live catalog max).
    distinct_index_ids_seen: usize,
    last_audit: Option<(usize, usize, usize)>, // (accounts, items, labels)
}

/// The per-cycle expected query answers (Leg C's small oracle), computed while the
/// deterministic seed set is built so the wire-correctness asserts are non-vacuous.
struct CycleLedger {
    account_count: i64,
    item_count: i64,
    label_count: i64,
    /// Accounts whose `tags` contain `'gold'` (the GIN `@>` oracle).
    gold_count: i64,
    /// The email of a seeded ACTIVE account — drives the partial-unique point read and
    /// the duplicate-active-email `23505` probe.
    known_active_email: String,
    created_lo: String,
    created_hi: String,
    /// Accounts whose `created_at` falls in `[created_lo, created_hi)` (the b-tree
    /// range oracle).
    created_in_range: i64,
}

/// The in-process audit result Leg C records per kill.
struct LegCAudit {
    account_rows: usize,
    item_rows: usize,
    label_rows: usize,
    active_snapshots: usize,
    pending_gc_pages: usize,
    free_list_size: usize,
    /// The reopened catalog's live storage index_ids (unioned into the run-wide set).
    index_ids: Vec<u32>,
}

/// Band↔catalog bijection: every physically-present index band has a live catalog
/// owner. The orphan-band case (a band whose index_id is absent from `index_defs()`)
/// is structurally invisible to `check_index_consistency` (it only scans catalog-known
/// ids) — it is the fingerprint of the OID-reuse desync (a dropped index whose band
/// wasn't swept, or a reused id whose leftover entries outlived their catalog def).
fn check_index_band_bijection(w: &BtreeStorageWrapper) -> Result<(), String> {
    let root = w.inner().current_root();
    let entries = w
        .inner()
        .range_scan_at_root(
            root,
            &BtreeKey::index_table_start(0),
            &BtreeKey::sys_catalog_start(),
        )
        .map_err(|e| format!("bijection band scan: {e}"))?;
    let catalog_ids: HashSet<u32> = w.index_defs().keys().copied().collect();
    for (key, _) in &entries {
        if let BtreeKey::Index { index_id, .. } = key
            && !catalog_ids.contains(index_id)
        {
            return Err(format!(
                "orphan index band: index_id {index_id} has storage entries but no catalog owner"
            ));
        }
    }
    Ok(())
}

/// Every catalog index NAME maps to a distinct storage `index_id` — two indexes
/// sharing one id is the OID-reuse desync (dropping one orphans the other).
fn check_distinct_index_ids(schema: &Schema) -> Result<(), String> {
    let mut seen = HashSet::new();
    for (name, idx) in &schema.indexes {
        if !seen.insert(idx.index_id()) {
            return Err(format!(
                "index {name} shares storage index_id {} with another index",
                idx.index_id()
            ));
        }
    }
    Ok(())
}

/// Reopen the killed Leg C store in-process and run the structural audit: index↔table
/// consistency + referential integrity (reused from Leg A) plus the two OID-desync
/// detectors. A failed reopen is infra; every check violation is an invariant finding.
/// No durability/ledger comparison — Leg C's per-cycle `TRUNCATE`+re-seed makes exact
/// row-set recovery uninteresting; the point is structural (no desync ever accretes).
fn audit_c(data_dir: &Path) -> Result<LegCAudit, SoakError> {
    let db = Database::open(data_dir)
        .map_err(|e| SoakError::Infra(format!("leg-c reopen after kill: {e}")))?;
    // KEEP the schema (Leg A discards it): the distinct-id check needs `schema.indexes`.
    let (schema, backend) = db.into_parts();
    let StorageBackend::Btree(wrapper) = backend else {
        return Err(SoakError::Infra(
            "leg-c: expected a B+tree backend after reopen".into(),
        ));
    };

    let stats = wrapper.stats();
    if stats.pending_gc_pages > stats.page_count.saturating_add(1)
        || stats.free_list_size > stats.page_count.saturating_add(1)
    {
        return Err(SoakError::Invariant(format!(
            "leg-c reopen stats look corrupt: pending_gc={} free_list={} page_count={}",
            stats.pending_gc_pages, stats.free_list_size, stats.page_count
        )));
    }

    check_index_consistency(&wrapper).map_err(SoakError::Invariant)?;
    check_referential(&wrapper).map_err(SoakError::Invariant)?;
    check_index_band_bijection(&wrapper).map_err(SoakError::Invariant)?;
    check_distinct_index_ids(&schema).map_err(SoakError::Invariant)?;

    let account_rows = wrapper
        .scan("account")
        .map_err(|e| SoakError::Invariant(e.to_string()))?
        .len();
    let item_rows = wrapper
        .scan("item")
        .map_err(|e| SoakError::Invariant(e.to_string()))?
        .len();
    let label_rows = wrapper
        .scan("item_label")
        .map_err(|e| SoakError::Invariant(e.to_string()))?
        .len();
    let index_ids: Vec<u32> = schema
        .indexes
        .values()
        .map(pglet_sql::catalog::Index::index_id)
        .collect();

    Ok(LegCAudit {
        account_rows,
        item_rows,
        label_rows,
        active_snapshots: stats.active_snapshots,
        pending_gc_pages: stats.pending_gc_pages,
        free_list_size: stats.free_list_size,
        index_ids,
    })
}

/// Seed a deterministic per-cycle set (after a `TRUNCATE`), returning the oracle for the
/// wire-correctness asserts. Status alternates so the partial index has both members
/// (even ids ACTIVE) and exclusions (odd ids suspended/closed); `'gold'` tags on every
/// third id feed the GIN oracle; `created_at = ts_literal(id)` is strictly increasing in
/// id (for these small ids), so an id-bounded range maps to a known count.
async fn seed_cycle(
    c: &Client,
    rng: &mut Xorshift,
    seed_accounts: i64,
) -> Result<CycleLedger, String> {
    let status_of = |id: i64| -> &'static str {
        if id % 2 == 0 {
            "active"
        } else if id % 4 == 1 {
            "suspended"
        } else {
            "closed"
        }
    };
    let mut tuples = Vec::with_capacity(seed_accounts as usize);
    let mut gold_count = 0i64;
    let mut known_active_email: Option<String> = None;
    for id in 1..=seed_accounts {
        let status = status_of(id);
        if status == "active" && known_active_email.is_none() {
            known_active_email = Some(format!("u{id}@ex.com"));
        }
        let tags = if id % 3 == 0 {
            gold_count += 1;
            "ARRAY['gold','red']::text[]"
        } else {
            "ARRAY['silver']::text[]"
        };
        let ts = ts_literal(id);
        tuples.push(format!(
            "({id}, 'u{id}@ex.com', '{status}', {tags}, '{{\"n\":{id}}}'::jsonb, '{ts}')"
        ));
    }
    run(
        c,
        &format!("INSERT INTO account VALUES {}", tuples.join(",")),
    )
    .await?;

    // Items over the even (active) accounts; a label on every 4th (so item/item_label
    // aren't empty — the referential audit and the FK cascade have something to chew on).
    let mut item_tuples = Vec::new();
    let mut label_tuples = Vec::new();
    let mut item_count = 0i64;
    let mut label_count = 0i64;
    let mut id = 2;
    while id <= seed_accounts {
        let item_id = 1_000_000 + id;
        let ts = ts_literal(item_id);
        let kind = KIND_POOL[(rng.below(KIND_POOL.len() as u64)) as usize];
        item_tuples.push(format!(
            "({item_id}, {id}, '{kind}', 10.00, '{{\"k\":{id}}}'::jsonb, '{ts}')"
        ));
        item_count += 1;
        if id % 4 == 0 {
            label_tuples.push(format!("({item_id}, {id}, 'review')"));
            label_count += 1;
        }
        id += 2;
    }
    if !item_tuples.is_empty() {
        run(
            c,
            &format!("INSERT INTO item VALUES {}", item_tuples.join(",")),
        )
        .await?;
    }
    if !label_tuples.is_empty() {
        run(
            c,
            &format!("INSERT INTO item_label VALUES {}", label_tuples.join(",")),
        )
        .await?;
    }

    let known_active_email = known_active_email.ok_or("leg-c seed: no active account produced")?;

    // The b-tree range oracle: ids in `[lo_id, hi_id)` (ts_literal is monotone in id).
    let lo_id = (seed_accounts / 4).max(1);
    let hi_id = (seed_accounts * 3 / 4).max(lo_id + 1);
    let created_in_range = (1..=seed_accounts)
        .filter(|&i| i >= lo_id && i < hi_id)
        .count() as i64;

    Ok(CycleLedger {
        account_count: seed_accounts,
        item_count,
        label_count,
        gold_count,
        known_active_email,
        created_lo: ts_literal(lo_id),
        created_hi: ts_literal(hi_id),
        created_in_range,
    })
}

/// The non-vacuous part: after the seed, assert the RECREATED indexes actually bind and
/// enforce — a dangling def or a desynced band would surface here (wrong count, a point
/// read that misses, or a duplicate that isn't rejected).
async fn assert_cycle_correctness(c: &Client, led: &CycleLedger) -> Result<(), String> {
    let total = scalar_i64(c, "SELECT count(*) FROM account").await?;
    if total != led.account_count {
        return Err(format!(
            "leg-c account count: server {total} != seeded {}",
            led.account_count
        ));
    }
    let items = scalar_i64(c, "SELECT count(*) FROM item").await?;
    if items != led.item_count {
        return Err(format!(
            "leg-c item count: server {items} != seeded {}",
            led.item_count
        ));
    }
    // Partial-index point read: exactly the one seeded active row. Under the default
    // `index_acceleration` build the planner serves this from the recreated
    // `account_active_email_idx`; the stripped profile full-scans (same answer, no
    // acceleration).
    let hit = scalar_i64(
        c,
        &format!(
            "SELECT count(*) FROM account WHERE status='active' AND email='{}'",
            led.known_active_email
        ),
    )
    .await?;
    if hit != 1 {
        return Err(format!(
            "leg-c partial-index point read: expected 1 row for active email {}, got {hit}",
            led.known_active_email
        ));
    }
    // A DUPLICATE active email must be rejected 23505 — with no table-level UNIQUE on
    // email, the recreated partial `account_active_email_idx` is the sole enforcer, so
    // this proves *it* is live/enforcing (not a dangling def). A fresh PK so the
    // uniqueness, not the PK, is what trips.
    expect_sqlstate(
        c,
        &format!(
            "INSERT INTO account VALUES (990000000, '{}', 'active', ARRAY['gold']::text[], NULL, '2024-01-01 00:00:00+00')",
            led.known_active_email
        ),
        "23505",
    )
    .await?;
    // GIN `@>` containment — served by the recreated `account_tags_gin` under the
    // default `index_acceleration` build (a full-scan recheck otherwise, same answer).
    let gold = scalar_i64(
        c,
        "SELECT count(*) FROM account WHERE tags @> ARRAY['gold']::text[]",
    )
    .await?;
    if gold != led.gold_count {
        return Err(format!(
            "leg-c GIN @> gold count: server {gold} != seeded {}",
            led.gold_count
        ));
    }
    // B-tree range — served by the recreated `account_created_idx` under acceleration.
    let in_range = scalar_i64(
        c,
        &format!(
            "SELECT count(*) FROM account WHERE created_at >= '{}' AND created_at < '{}'",
            led.created_lo, led.created_hi
        ),
    )
    .await?;
    if in_range != led.created_in_range {
        return Err(format!(
            "leg-c range count: server {in_range} != expected {}",
            led.created_in_range
        ));
    }
    Ok(())
}

/// Bulk-load `[id_lo, id_hi)` accounts (reusing `account_tuple`) so a straddle cycle has
/// a populated table for the un-awaited `CREATE INDEX` to build over.
async fn bulk_load_accounts(
    c: &Client,
    rng: &mut Xorshift,
    id_lo: i64,
    id_hi: i64,
) -> Result<(), String> {
    const CHUNK: i64 = 500;
    let mut id = id_lo;
    while id < id_hi {
        let end = (id + CHUNK).min(id_hi);
        let mut tuples = Vec::with_capacity((end - id) as usize);
        while id < end {
            tuples.push(account_tuple(id, rng));
            id += 1;
        }
        run(
            c,
            &format!("INSERT INTO account VALUES {}", tuples.join(",")),
        )
        .await?;
    }
    Ok(())
}

/// Trigger the exact catalog↔storage index-OID desync the fix closed — the actual
/// regression, not just its structural shadow.
///
/// The autocommit swaps above **cannot** reproduce it: an autocommit `DROP INDEX x`
/// fully commits — shedding its band (`delete_index_band`) and freeing the oid —
/// *before* the autocommit `CREATE INDEX x` mints a new one, so even under the reverted
/// `max+1` bug the freed oid is reused but the old band is already gone → no two live
/// indexes ever share a band. The desync needs a **within-one-transaction** same-name
/// swap on the **highest-oid** object:
///
/// - `BEGIN; DROP INDEX x` — buffered (`drop_index_in_tx`): the band + def stay live.
/// - `CREATE INDEX x` — applied immediately (read-your-own-writes). Under the reverted
///   bug `generate_object_oid` = `max+1`; since the just-dropped `x` was the highest oid
///   (a fresh probe created last → its oid *is* the current max), the CREATE **reuses
///   that same oid** → the new index's entries land in the *dropped* index's band.
/// - `COMMIT` applies the buffered `delete_index_band` **first** → sweeps the reused
///   band (including the freshly-built entries); or `ROLLBACK`'s `undo_created_indexes`
///   sheds the reused band. Either way the catalog index is left with an empty/desynced
///   storage band.
///
/// Under the fix (`Schema::next_oid` monotonic) the recreation gets a fresh higher oid,
/// so the buffered drop/undo touches only the OLD band and the new one survives.
///
/// The detection is an index-accelerated probe **prepared over the extended protocol**
/// (the plan-cache + `IndexScan` path the fuz_app "index not found" rode): a desynced
/// band makes it return the wrong count or error loud. This needs the default
/// `index_acceleration` build — the stripped profile full-scans and would mask a
/// desynced band. The prepare precedes the swap so a cached plan references the pre-swap
/// index (also exercising the rollback-generation cache path the fix's other prong
/// guards). Both commit (even cycles) and rollback (odd) variants run.
async fn probe_swap_desync_trigger(client: &Client, cycle: u64) -> Result<(), String> {
    // Fresh probe each cycle, created LAST, so `probe_k_idx` is the highest-oid object
    // (under the bug its oid *is* `max+1` = the current max — the reuse precondition).
    run(client, "DROP TABLE IF EXISTS probe").await?;
    run(client, "CREATE TABLE probe (id INT PRIMARY KEY, k INT)").await?;
    run(client, "CREATE INDEX probe_k_idx ON probe (k)").await?;
    run(
        client,
        "INSERT INTO probe VALUES (1, 7), (2, 7), (3, 9), (4, 7)",
    )
    .await?;
    let expected_k7: i64 = 3;

    // Prepare the index-accelerated probe (extended protocol → plan cache) BEFORE the
    // swap, so a cached plan references the pre-swap index.
    let stmt = client
        .prepare("SELECT count(*) FROM probe WHERE k = $1")
        .await
        .map_err(|e| format!("probe prepare: {e}"))?;

    // The within-txn same-name swap — commit on even cycles, rollback on odd (both
    // desync under the bug; a probe insert perturbs the tree so the band is non-empty).
    run(client, "BEGIN").await?;
    run(client, "DROP INDEX probe_k_idx").await?;
    run(client, "CREATE INDEX probe_k_idx ON probe (k)").await?;
    if cycle.is_multiple_of(2) {
        run(client, "COMMIT").await?;
    } else {
        run(client, "ROLLBACK").await?;
    }

    // Re-execute the SAME prepared statement (extended protocol → plan cache + the
    // IndexScan on the recreated index). A swept/desynced band returns the wrong count
    // or errors loud (e.g. "index not found") — either is the desync finding. Surface
    // the server-side message + SQLSTATE so the finding is legible, not a bare "db error".
    let rows = client.query(&stmt, &[&7i32]).await.map_err(|e| {
        let detail = e.as_db_error().map_or_else(
            || e.to_string(),
            |db| format!("{} (SQLSTATE {})", db.message(), db.code().code()),
        );
        format!(
            "index-OID-desync: probe recreated-index query errored (the index-OID-desync regression): {detail}"
        )
    })?;
    let got: i64 = rows
        .first()
        .ok_or("probe recreated-index query returned no row")?
        .try_get(0)
        .map_err(|e| format!("probe count decode: {e}"))?;
    if got != expected_k7 {
        return Err(format!(
            "index-OID-desync: probe count via recreated probe_k_idx = {got}, expected \
             {expected_k7} — the recreated index's storage band was swept (catalog/storage \
             desync; the index-OID-desync regression)"
        ));
    }

    run(client, "DROP TABLE probe").await?; // dropped before any kill → never audited
    Ok(())
}

/// The Leg C driver: one long-lived server churned through `reset_cycles` idempotent
/// re-migration + index-swap cycles, with periodic `kill -9` → in-process desync audit →
/// respawn (the respawn exercises the restart `next_oid` re-seed).
async fn leg_c(
    config: &Config,
    rec: &mut Recorder,
    base_secs: f64,
) -> Result<LegCReport, SoakError> {
    let scratch = Scratch::new("legc");
    let range = PGLET_TEST_PORT..DYNAMIC_PORT_END;
    let (mut server, mut port) = spawn_ready(range.clone(), Some(&scratch.path))
        .await
        .map_err(SoakError::Infra)?;
    let mut client = connect(port).await;

    let mut rng = Xorshift::new(config.seed ^ 0x0000_0000_0000_C0DE);
    let mut report = LegCReport::default();
    // Every distinct storage index_id ever seen live (logged as swap-burn evidence).
    let mut all_index_ids: HashSet<u32> = HashSet::new();

    let seed_accounts: i64 = if config.long { 200 } else { 80 };
    // A straddle cycle bulk-loads this many extra accounts so the un-awaited CREATE INDEX
    // build is slow enough to actually straddle the kill window.
    let straddle_extra: i64 = if config.long { 20_000 } else { 4_000 };

    let start = Instant::now();
    let mut last_sample = Instant::now();

    for cycle in 1..=config.reset_cycles {
        // 1. Idempotent re-migration over LIVE prior tables/rows/indexes — all must
        //    succeed. `schema_version` is dropped+recreated (the L2-reset shape); the
        //    rest is `CREATE … IF NOT EXISTS` (mostly no-ops after cycle 1).
        run(&client, "DROP TABLE IF EXISTS schema_version").await?;
        run(
            &client,
            "CREATE TABLE IF NOT EXISTS schema_version(version INT)",
        )
        .await?;
        run(&client, SCHEMA_SQL_IDEMPOTENT).await?;

        // The same-named index swaps — the core stressor. Each recreation allocates a
        // fresh OID; a surfaced `index not found` / catalog error is the desync's
        // fail-loud symptom → propagates as an Invariant (via `?`).
        run(&client, "DROP INDEX IF EXISTS account_active_email_idx").await?;
        run(
            &client,
            "CREATE UNIQUE INDEX IF NOT EXISTS account_active_email_idx ON account (email) WHERE status = 'active'",
        )
        .await?;
        run(&client, "DROP INDEX IF EXISTS account_created_idx").await?;
        run(
            &client,
            "CREATE INDEX IF NOT EXISTS account_created_idx ON account (created_at)",
        )
        .await?;
        run(&client, "DROP INDEX IF EXISTS account_tags_gin").await?;
        run(
            &client,
            "CREATE INDEX IF NOT EXISTS account_tags_gin ON account USING gin (tags)",
        )
        .await?;
        report.index_swaps += 3;

        // 2. Rollback axes (interleaved; the suffix varies so names never collide).
        //    (a) index-OID burn: the band must be shed, the OID stays burned (monotonic).
        if cycle % 2 == 0 {
            run(&client, "BEGIN").await?;
            let r = run(
                &client,
                &format!("CREATE INDEX tmp_idx_{cycle} ON account (created_at)"),
            )
            .await;
            run(&client, "ROLLBACK").await?;
            r?;
            report.oid_burns += 1;
        }
        //    (b) table_id-reuse footgun: `next_table_id` IS restored on rollback — safe
        //        only because `undo_created_indexes` sheds the implicit pkey index first.
        if cycle % 3 == 0 {
            run(&client, "BEGIN").await?;
            let r = run(
                &client,
                &format!("CREATE TABLE tmp_tbl_{cycle} (id BIGINT PRIMARY KEY)"),
            )
            .await;
            run(&client, "ROLLBACK").await?;
            r?;
            report.table_id_burns += 1;
        }
        //    (c) occasionally COMMIT an index then drop it (a real build + real
        //        range-delete — the band must be fully swept, no orphan on reopen).
        if cycle % 5 == 0 {
            run(&client, "BEGIN").await?;
            run(
                &client,
                &format!("CREATE INDEX committed_idx_{cycle} ON item (created_at)"),
            )
            .await?;
            run(&client, "COMMIT").await?;
            run(&client, &format!("DROP INDEX committed_idx_{cycle}")).await?;
        }

        // 3. Per-test reset + deterministic seed.
        run(&client, "TRUNCATE account, item, item_label CASCADE").await?;
        let led = seed_cycle(&client, &mut rng, seed_accounts).await?;

        // 4. Wire-correctness — the recreated indexes must BIND and ENFORCE.
        assert_cycle_correctness(&client, &led).await?;
        report.correctness_batches += 1;

        // 4b. The desync trigger proper — a within-txn same-name swap on a highest-oid
        //     probe + an extended-protocol accelerated read. Goes RED under the reverted
        //     fix; the structural audits alone stay green (the autocommit
        //     swaps above don't reproduce the bug).
        probe_swap_desync_trigger(&client, cycle).await?;
        report.probe_swaps += 1;

        if last_sample.elapsed() >= SAMPLE_INTERVAL {
            rec.record(&Sample {
                leg: "c",
                elapsed_secs: base_secs + start.elapsed().as_secs_f64(),
                data_file_bytes: file_len(&scratch.data_file()),
                server_rss_kb: read_status_kb(Some(server.id()), "VmHWM").unwrap_or(0),
                server_rss_cur_kb: read_status_kb(Some(server.id()), "VmRSS").unwrap_or(0),
                ops_completed: cycle,
                account_rows: led.account_count as usize,
                item_rows: led.item_count as usize,
                label_rows: led.label_count as usize,
                active_snapshots: None,
                pending_gc_pages: None,
                free_list_size: None,
            });
            last_sample = Instant::now();
        }

        // 5. Periodic kill → in-process desync audit → respawn (the final cycle always
        //    audits). The respawn exercises the restart `next_oid` re-seed.
        let is_final = cycle == config.reset_cycles;
        if cycle % config.kill_every_cycles == 0 || is_final {
            // Guarantee >= 1 kill-mid-swap straddle: alternate, and force the straddle on
            // the final kill if none has straddled yet.
            let straddle =
                report.kill_audit_cycles % 2 == 1 || (is_final && report.straddle_kills == 0);

            if straddle {
                // Populate the table so the un-awaited CREATE INDEX build is slow enough
                // to straddle the kill, then fire it on a SEPARATE connection and kill
                // after a seeded random delay. On reopen the half-built index must be
                // all-or-nothing (the bijection must hold — no orphan band, no reused-id
                // collision with an un-swept band).
                let lo = seed_accounts + 1;
                let hi = seed_accounts + 1 + straddle_extra;
                bulk_load_accounts(&client, &mut rng, lo, hi).await?;
                let big_sql = format!("CREATE INDEX straddle_idx_{cycle} ON account (email)");
                let straddle_client = connect(port).await;
                let handle =
                    tokio::spawn(async move { straddle_client.simple_query(&big_sql).await });
                let delay_ms = 2 + rng.below(40);
                drop(client);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                hard_kill(server);
                // Ack ignored — the build is optional; only its atomicity is required.
                let _ = handle.await;
                report.straddle_kills += 1;
            } else {
                drop(client);
                hard_kill(server);
            }

            let audit_start = Instant::now();
            let a = audit_c(&scratch.path)?;
            let audit_ms = audit_start.elapsed().as_millis();
            report.kill_audit_cycles += 1;
            for id in &a.index_ids {
                all_index_ids.insert(*id);
            }
            report.distinct_index_ids_seen = all_index_ids.len();
            report.last_audit = Some((a.account_rows, a.item_rows, a.label_rows));

            eprintln!(
                "[soak] leg-c kill/audit #{} at cycle {cycle} [{}]: accounts={} items={} labels={} \
                 catalog_indexes={} distinct_ids_seen={} \
                 (audit={audit_ms}ms active_snapshots={} pending_gc={} free_list={})",
                report.kill_audit_cycles,
                if straddle { "straddle" } else { "quiescent" },
                a.account_rows,
                a.item_rows,
                a.label_rows,
                a.index_ids.len(),
                report.distinct_index_ids_seen,
                a.active_snapshots,
                a.pending_gc_pages,
                a.free_list_size,
            );
            rec.record(&Sample {
                leg: "c",
                elapsed_secs: base_secs + start.elapsed().as_secs_f64(),
                data_file_bytes: file_len(&scratch.data_file()),
                server_rss_kb: 0,
                server_rss_cur_kb: 0,
                ops_completed: cycle,
                account_rows: a.account_rows,
                item_rows: a.item_rows,
                label_rows: a.label_rows,
                active_snapshots: Some(a.active_snapshots),
                pending_gc_pages: Some(a.pending_gc_pages),
                free_list_size: Some(a.free_list_size),
            });

            if is_final {
                report.reset_cycles = cycle;
                return Ok(report);
            }

            let (srv, p) = spawn_ready(range.clone(), Some(&scratch.path))
                .await
                .map_err(SoakError::Infra)?;
            server = srv;
            port = p;
            client = connect(port).await;
        }
    }

    // Reached only if `reset_cycles == 0` (the loop returns on the final cycle otherwise).
    report.reset_cycles = config.reset_cycles;
    Ok(report)
}

// ============================================================================
// Results file (--update) + main
// ============================================================================

fn git_commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map_or_else(
            || "unknown".to_string(),
            |o| String::from_utf8_lossy(&o.stdout).trim().to_string(),
        )
}

fn write_results(config: &Config, a: &LegAReport, b: &LegBReport, c: &LegCReport, verdict: &str) {
    let json = format!(
        "{{\n  \"scenario\": \"soak\",\n  \"note\": \"large-dataset soak: server churn + kill/audit (Leg A), in-process embed vacuum/snapshot stats (Leg B), and re-migration/index-OID-desync churn (Leg C). Watch-tier — compare to a prior local run.\",\n  \"meta\": {{\n    \"git_commit\": \"{}\",\n    \"seed\": {},\n    \"rows_target\": {},\n    \"duration_secs\": {},\n    \"kill_every_secs\": {},\n    \"reset_cycles\": {},\n    \"kill_every_cycles\": {},\n    \"long\": {}\n  }},\n  \"verdict\": \"{}\",\n  \"leg_a\": {{\n    \"kill_audit_cycles\": {},\n    \"invariant_batches\": {},\n    \"final_ops\": {},\n    \"peak_data_file_bytes\": {},\n    \"peak_server_rss_kb\": {}\n  }},\n  \"leg_b\": {{\n    \"pending_peak_pinned\": {},\n    \"pending_after_vacuum\": {},\n    \"free_list_after_vacuum\": {},\n    \"file_after_vacuum_bytes\": {},\n    \"file_after_churn_bytes\": {}\n  }},\n  \"leg_c\": {{\n    \"reset_cycles\": {},\n    \"index_swaps\": {},\n    \"probe_swaps\": {},\n    \"oid_burns\": {},\n    \"table_id_burns\": {},\n    \"kill_audit_cycles\": {},\n    \"straddle_kills\": {},\n    \"distinct_index_ids_seen\": {}\n  }}\n}}\n",
        git_commit(),
        config.seed,
        config.rows_target,
        config.duration.as_secs(),
        config.kill_every.as_secs(),
        config.reset_cycles,
        config.kill_every_cycles,
        config.long,
        verdict,
        a.kill_audit_cycles,
        a.invariant_batches,
        a.final_ops,
        a.peak_data_file_bytes,
        a.peak_server_rss_kb,
        b.pending_peak_pinned,
        b.pending_after_vacuum,
        b.free_list_after_vacuum,
        b.file_after_vacuum_bytes,
        b.file_after_churn_bytes,
        c.reset_cycles,
        c.index_swaps,
        c.probe_swaps,
        c.oid_burns,
        c.table_id_burns,
        c.kill_audit_cycles,
        c.straddle_kills,
        c.distinct_index_ids_seen,
    );
    let path = PathBuf::from("benches/soak_results.json");
    let _ = std::fs::create_dir_all("benches");
    match std::fs::write(&path, json) {
        Ok(()) => eprintln!("[soak] wrote {}", path.display()),
        Err(e) => eprintln!("[soak] WARNING: failed to write {}: {e}", path.display()),
    }
}

#[tokio::main]
async fn main() {
    let config = Config::from_args();

    eprintln!("[soak] Building pglet_server...");
    if !build_server() {
        eprintln!("[soak] Build failed!");
        std::process::exit(1);
    }

    eprintln!(
        "[soak] seed={:#x} rows_target={} duration={}s kill_every={}s long={} \
         (override via PGLET_SOAK_SEED / PGLET_SOAK_ROWS / PGLET_SOAK_DURATION_SECS / PGLET_SOAK_KILL_EVERY_SECS)",
        config.seed,
        config.rows_target,
        config.duration.as_secs(),
        config.kill_every.as_secs(),
        config.long,
    );
    println!("=== pglet soak suite ===\n");

    let mut rec = Recorder::new();
    // A real engine violation exits 1; a harness/environment (infra) failure exits 2 —
    // so an unattended run never confuses a spawn flake for a durability bug.
    let mut invariant_failures = 0;
    let mut infra_failures = 0;

    println!("--- Leg A: server churn + kill -9 + in-process post-kill audit ---");
    let leg_a_report = match leg_a(&config, &mut rec).await {
        Ok(r) => {
            println!(
                "✓ Leg A: {} kill/audit cycles ({} straddling), {} invariant batches, {} ops — all invariants held",
                r.kill_audit_cycles, r.straddle_kills, r.invariant_batches, r.final_ops
            );
            if let Some((acc, it, lb)) = r.last_audit {
                println!("  final recovered: accounts={acc} items={it} labels={lb}");
            }
            r
        }
        Err(SoakError::Invariant(e)) => {
            eprintln!("✗ INVARIANT (Leg A): {e}");
            invariant_failures += 1;
            LegAReport::default()
        }
        Err(SoakError::Infra(e)) => {
            eprintln!("⚠ INFRA (Leg A): {e}");
            infra_failures += 1;
            LegAReport::default()
        }
    };

    let base_secs = config.duration.as_secs_f64();
    println!("\n--- Leg B: in-process embed churn with live vacuum/snapshot stats ---");
    let leg_b_report = match leg_b(&config, &mut rec, base_secs) {
        Ok(r) => {
            println!(
                "✓ Leg B: pinned pending-GC peak={}, drained to {} after vacuum, free_list={}; \
                 data file {} → {} bytes under bounded churn",
                r.pending_peak_pinned,
                r.pending_after_vacuum,
                r.free_list_after_vacuum,
                r.file_after_vacuum_bytes,
                r.file_after_churn_bytes,
            );
            r
        }
        Err(SoakError::Invariant(e)) => {
            eprintln!("✗ INVARIANT (Leg B): {e}");
            invariant_failures += 1;
            LegBReport::default()
        }
        Err(SoakError::Infra(e)) => {
            eprintln!("⚠ INFRA (Leg B): {e}");
            infra_failures += 1;
            LegBReport::default()
        }
    };

    // Leg C's recorder samples sit after Leg A + Leg B on the (monotonic) elapsed axis.
    let base_secs_c = base_secs * 2.0;
    println!(
        "\n--- Leg C: re-migration / index-OID-desync soak (server-driven index swaps + kill/audit) ---"
    );
    let leg_c_report = match leg_c(&config, &mut rec, base_secs_c).await {
        Ok(r) => {
            println!(
                "✓ Leg C: {} reset cycles, {} index swaps, {} within-txn probe swaps, \
                 {} OID burns, {} table-id burns, {} correctness batches, \
                 {} kill/audit cycles ({} straddling); {} distinct storage index_ids observed \
                 — no OID desync",
                r.reset_cycles,
                r.index_swaps,
                r.probe_swaps,
                r.oid_burns,
                r.table_id_burns,
                r.correctness_batches,
                r.kill_audit_cycles,
                r.straddle_kills,
                r.distinct_index_ids_seen,
            );
            if let Some((acc, it, lb)) = r.last_audit {
                println!("  final recovered: accounts={acc} items={it} labels={lb}");
            }
            r
        }
        Err(SoakError::Invariant(e)) => {
            eprintln!("✗ INVARIANT (Leg C): {e}");
            invariant_failures += 1;
            LegCReport::default()
        }
        Err(SoakError::Infra(e)) => {
            eprintln!("⚠ INFRA (Leg C): {e}");
            infra_failures += 1;
            LegCReport::default()
        }
    };

    // A real violation takes precedence over an infra flake in the verdict + exit code.
    let verdict = if invariant_failures > 0 {
        "fail"
    } else if infra_failures > 0 {
        "infra"
    } else {
        "pass"
    };
    println!("\n=== Results ===");
    println!("samples recorded: {}", rec.path.display());
    println!(
        "verdict: {verdict} (invariant violations: {invariant_failures}, infra failures: {infra_failures})"
    );

    if config.update {
        write_results(
            &config,
            &leg_a_report,
            &leg_b_report,
            &leg_c_report,
            verdict,
        );
    } else {
        eprintln!(
            "[soak] (results not written — pass --update to refresh benches/soak_results.json)"
        );
    }

    if invariant_failures > 0 {
        std::process::exit(1);
    } else if infra_failures > 0 {
        std::process::exit(2);
    }
}
