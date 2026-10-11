//! Native `dump_sql()` portability round-trip.
//!
//! Builds a populated in-process pglet `Database` (embed), calls its native
//! `dump_sql()` emitter, and proves the dump is portable in two directions:
//!
//! 1. **Portability to Postgres** — `psql -f` the dump into a freshly-reset
//!    `public` schema on the live Postgres oracle (`PGLET_PG_URL`, or
//!    `127.0.0.1:5432` with it unset), `SELECT` the rows back,
//!    and assert they match the original pglet data.
//! 2. **Self round-trip** — `psql -f` the same dump into a freshly-spawned
//!    `pglet_server` (in-memory) and assert the reloaded rows match too.
//!
//! The single dump string drives both, so a passing run proves the emitted SQL is
//! the same portable artifact for stock Postgres and pglet. The schema exercises a
//! `SERIAL` PK, a `text[]`, an `ENUM`, a `NOT NULL` + `DEFAULT`, a `FOREIGN KEY`, a
//! NULL cell, a value needing COPY escaping (tab / newline / backslash), a
//! `timestamptz` (the UTC `+00` round-trip, restored into Postgres at a non-UTC session zone
//! and read back under `SET TimeZone='UTC'`), a `timestamptz[]` holding a finite element, and a
//! `jsonb` value.
//!
//! **Rows are not the whole of what a restore has to reproduce**, and the identity
//! columns are where the two came apart: the reloaded *catalog* must answer the same
//! DDL as the one dumped, and an identity column that restored as a plain
//! `nextval`-defaulted one satisfied every row comparison here while silently
//! dropping every guarantee it exists to provide. So the identity corpus is read back
//! twice — as rows (counters, starts, and the two ordering hazards) and as
//! **refusals** ([`IDENTITY_REFUSALS`]), which is the only instrument that can see a
//! `GENERATED ALWAYS` column that stopped rejecting user-supplied ids.
//!
//! It also carries an `expr_probe` table exercising every
//! renderable scalar-function family in a **stored expression** — CHECK
//! constraints, an expression index key, and a partial index predicate — restricted
//! to what stock Postgres accepts at each site (a CHECK takes a volatile function,
//! `CREATE INDEX` does not). The pglet-only stored-expression shapes are covered by
//! the in-process unit tests in `pglet`'s `dump` module instead.
//!
//! Self-contained: builds + spawns the server, runs against live Postgres, and
//! degrades gracefully (skip, exit 0) when Postgres or the `psql` client is absent.

// Holds non-`Send` tokio-postgres futures across awaits on a single-threaded local
// runtime; they never cross a thread. `.expect()` surfaces setup failures loudly —
// right for a harness.
#![allow(clippy::future_not_send, clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;

use pglet::{Database, ExecuteResult};
use pglet_sql::catalog::Sequence;
use pglet_sql::types::value_to_text_with_oid;
use pglet_testing::server::{
    build_server, find_open_port, hard_kill, spawn_server, wait_for_ready,
};
use pglet_tests::{
    DYNAMIC_PORT_END, DbConfig, PGLET_TEST_PORT, POSTGRES_CONFIG, apply_conninfo, connect,
    has_binary, pglet_config, query_rows,
};
use tokio_postgres::Client;

/// The `SELECT`s whose rows must agree across the original pglet data, the
/// Postgres restore, and the pglet self-restore.
const SELECT_AUTHOR: &str = "SELECT id, name FROM author ORDER BY id";
const SELECT_POST: &str =
    "SELECT id, author_id, title, state, tags, body, created, meta FROM post ORDER BY id";
const SELECT_EXPR: &str = "SELECT id, txt, num, ts, uu FROM expr_probe ORDER BY id";
/// The two sequences whose counter sits **below 1** — a range only pglet accepts, and
/// the one the restored `CREATE SEQUENCE` has to declare a `MINVALUE` for. Read
/// through `nextval` rather than a catalog view (pglet models no `pg_sequence`), which
/// also proves the restored *resume point* and not merely that the script parsed.
const SELECT_LOW_COUNTERS: &str = "SELECT nextval('low_start_seq'), nextval('neg_counter_seq')";
/// The restored **start** of a below-1 sequence, read the only way SQL can: a bare
/// `RESTART` goes to it. The resume point alone cannot see this — `setval(n, …,
/// false)` restores it whatever `start` the `CREATE SEQUENCE` declared — so without
/// this probe the forced explicit `START WITH` beside the emitted `MINVALUE` is
/// guarded by nothing but a string assertion. It is discriminating on the *Postgres*
/// leg: given a minimum and no `START WITH`, `init_params` seeds `seqstart` **from the
/// minimum**, so a dump that dropped the `START WITH 1` restores
/// `neg_counter_seq` with `start = -4` and this answers −4 instead of 1. Runs after
/// [`SELECT_LOW_COUNTERS`] because it moves the counter it reads.
const SELECT_RESTARTED_START: &str =
    "ALTER SEQUENCE neg_counter_seq RESTART; SELECT nextval('neg_counter_seq')";
/// The identity tables' rows. Weak on their own — a dump that renders every identity
/// column as an ordinary `nextval`-defaulted one restores these byte-identically —
/// but they pin the *values* the counter and start probes below then have to agree
/// with, and they are what the two ordering hazards break outright.
const SELECT_IDENT_ROWS: &str = "SELECT id, note FROM ident_always ORDER BY id";
const SELECT_IDENT_DEFAULT_ROWS: &str = "SELECT id, note FROM ident_default ORDER BY id";
const SELECT_IDENT_NAMED_ROWS: &str = "SELECT id, note FROM ident_named ORDER BY id";
const SELECT_IDENT_LOW_ROWS: &str = "SELECT id, note FROM ident_low ORDER BY id";
/// The two **ordering hazards** the identity rendering creates, read as rows: a
/// sibling column drawing from its own table's identity sequence, and a table earlier
/// in name order drawing from a later table's. Both are ordinary catalogs both engines
/// accept, and both restore only because every `nextval` default is hoisted out of its
/// `CREATE TABLE`.
const SELECT_IDENT_SIBLING: &str = "SELECT borrowed, id, note FROM ident_sibling ORDER BY id";
const SELECT_IDENT_BORROWER: &str =
    "SELECT borrowed, also_borrowed, expr_borrowed, note FROM ident_borrower";
/// Every identity sequence's **resume point**. Each is well clear of `1`, so a dump
/// that lost the counter cannot pass by coincidence.
const SELECT_IDENT_COUNTERS: &str = "SELECT nextval('ident_always_id_seq'), \
     nextval('ident_default_id_seq'), nextval('journal_counter'), \
     nextval('ident_low_id_seq'), nextval('sibling_counter')";
/// Every identity sequence's restored **start**, which the resume point cannot see:
/// `setval(…, false)` restores the counter whatever start the creating statement
/// declared, so only a bare `RESTART` reads it back. Discriminating on both legs —
/// drop the `START WITH` from the emitted identity option list and these answer
/// `1, 1, 0` instead. Runs after [`SELECT_IDENT_COUNTERS`] because it moves what that
/// reads.
const SELECT_IDENT_STARTS: &str = "ALTER SEQUENCE ident_default_id_seq RESTART; \
     ALTER SEQUENCE journal_counter RESTART; ALTER SEQUENCE ident_low_id_seq RESTART; \
     SELECT nextval('ident_default_id_seq'), nextval('journal_counter'), \
     nextval('ident_low_id_seq')";
/// The **accept** polarity, as a row: a `GENERATED BY DEFAULT` column still takes an
/// explicit id after the restore. Without it an over-correction that restored every
/// identity column as `ALWAYS` would satisfy every refusal in [`IDENTITY_REFUSALS`]
/// and pass green.
const SELECT_IDENT_ACCEPTS: &str =
    "INSERT INTO ident_default (id, note) VALUES (555, 'explicit') RETURNING id, note";
/// The rows of a table whose **name needs quoting**, so its implicit sequence's name
/// does too. Weak alone — they restore from the `COPY` whatever the sequence did — but
/// they pin the values the counter probe then has to be clear of, and a `CREATE TABLE`
/// / `COPY` that lost the quoting fails here rather than three statements later.
const SELECT_QUOTED_ROWS: &str = "SELECT id, note FROM \"Odd\" ORDER BY id";
/// An identity column whose **declared** `SEQUENCE NAME` contains a space. Its ids are
/// the only plain-SQL observable that the `ADD GENERATED` and the trailing `setval`
/// agreed about which relation they meant.
const SELECT_QUOTED_IDENT_ROWS: &str = "SELECT id, note FROM quoted_ident ORDER BY id";
/// An ordinary column whose hoisted `SET DEFAULT nextval(…)` names that spaced
/// sequence. Its one value (`702`) is what proves the *`nextval` literal* survived:
/// restored from a script whose literal read as a different relation, this column has
/// no default at all on Postgres and draws from a freshly minted sequence on pglet.
const SELECT_QUOTED_BORROWED: &str = "SELECT borrowed, note FROM quoted_borrower";
/// A sequence whose name contains a **`"`**, drawn from by an ordinary column. The
/// outer `quote_string_literal` escapes for the *string* grammar and the inner
/// `quote_ident` for the *identifier* grammar, and only a name carrying the identifier
/// grammar's own escape can tell the two apart: quote the name without doubling its
/// internal `"` and the literal closes early, naming `a`. Every other cell here passes
/// under that mutation.
const SELECT_QUOTED_DOUBLED: &str = "SELECT id, note FROM quoted_dq";
/// All three quoted-name sequences' **resume points**, read by their quoted names — the
/// discriminating probe for the `setval` literal, which is the *only* site an identity
/// sequence's name reaches. Each value is far from `1` and from its own start, so a
/// `setval` that landed on a different relation cannot answer these by coincidence.
/// Runs after the row probes because it moves what they read.
const SELECT_QUOTED_COUNTERS: &str = "SELECT nextval('\"Odd_id_seq\"'), \
     nextval('\"seq w space\"'), nextval('\"a\"\"b\"')";

/// The rows an **expression default** produced, on both sides of the emitter's hoist.
/// Rows are the right instrument here: a default hoisted when it should be inline (or
/// the reverse) restores as *no* default at all on one of the two engines, and the
/// next row written through it then differs.
const SELECT_EXPR_DEFAULT_HOIST: &str = "SELECT k, drawn FROM expr_default_hoist ORDER BY k";
const SELECT_EXPR_DEFAULT_INLINE: &str =
    "SELECT k, arith, cmp, fun, branch, arr, cast_ FROM expr_default_inline ORDER BY k";
/// The hoisted default's **resume point**, read after the row probe above: a restore
/// that dropped the `SET DEFAULT` or the `setval` answers a different value here while
/// every already-written row still matches.
const SELECT_EXPR_DEFAULT_RESUME: &str = "SELECT nextval('expr_default_seq')";

/// The temporal-precision table's stored rows. Like the identity rows, **weak on
/// their own**: the source's values are already rounded, so a dump that emitted a bare
/// `timestamp` for a `timestamp(3)` column restores every one of them byte-identically.
/// They pin the values [`SELECT_PRECISION_STORE`] then writes beside.
const SELECT_PRECISION_ROWS: &str = "SELECT id, a, b, c, d, e, f FROM precision_probe ORDER BY id";
/// **The instrument that can see a dropped precision**: a write through the *restored*
/// catalog. The literal carries four fractional digits at every column, so a restore
/// that lost the `(p)` stores it whole and this row differs — where the rows above and
/// the whole of `information_schema` at the source cannot tell the two catalogs apart.
/// Runs after [`SELECT_PRECISION_ROWS`] because it adds a row that select would read.
const SELECT_PRECISION_STORE: &str = "INSERT INTO precision_probe (id, a, b, c, d, e, f) VALUES \
     (9, '2020-01-02 03:04:05.6789', '2020-01-02 03:04:05.6789', '03:04:05.6789', \
     '1.23456 secs', '{2020-01-02 03:04:05.6789}', '{1.23456 secs}'); \
     SELECT a, b, c, d, e, f FROM precision_probe WHERE id = 9";
/// The restored **catalog shape** for the same table, read the way a client reads it.
/// The write probe above proves the precision is enforced; this proves it is also
/// *reported*. It cannot tell `g TIMESTAMP` from `h TIMESTAMP(6)`, though:
/// `datetime_precision` is a fact about the **type**, so it answers `6` at both — which
/// is [`SELECT_PRECISION_TYPMOD`]'s job.
const SELECT_PRECISION_CATALOG: &str = "SELECT column_name, data_type, datetime_precision \
     FROM information_schema.columns WHERE table_name = 'precision_probe' \
     ORDER BY ordinal_position";
/// **The one observable that separates a bare `timestamp` from a `timestamp(6)`** — and
/// therefore the whole reason the `g`/`h` pair is in [`build_source`]'s table.
/// `pg_attribute.atttypmod` is `-1` against `6`, and nothing else here can tell them
/// apart: [`SELECT_PRECISION_ROWS`] and [`SELECT_PRECISION_STORE`] do not name `g`/`h`
/// at all, and a write that did would round to the same microsecond at `p = 6` as at no
/// modifier; [`SELECT_PRECISION_CATALOG`] names them and reads the *type*, answering `6`
/// for both. Measured: dropping the emitted precision **only** at `p == 6` left the rest
/// of this round-trip green and the whole `cargo test` workspace green, and reddens both
/// restore legs here.
const SELECT_PRECISION_TYPMOD: &str = "SELECT a.attname, a.atttypmod FROM pg_attribute a \
     JOIN pg_class c ON c.oid = a.attrelid WHERE c.relname = 'precision_probe' \
     AND a.attnum > 0 ORDER BY a.attnum";

/// The stored `infinity` / `-infinity` of each type, beside a finite neighbour and a NULL,
/// in scalar and array columns, under a primary key, an index and a `CHECK` whose
/// constant is an infinity — the `COPY` text a restore has to read back as the value
/// stored, on both engines. `tzs` holds a finite `timestamptz[]` element, whose `COPY`
/// text needs its `+00`: the Postgres restore runs at a non-UTC session zone
/// ([`restore_via_psql`]), which would read an offset-less element as local time.
const SELECT_INFINITY_ROWS: &str =
    "SELECT id, d, ts, tz, ds, tzs FROM infinity_probe ORDER BY d, id";

/// A row of 100 000 characters (an overflow chain in the source) beside a short one,
/// under a `CHECK` whose 6 000-character literal is a catalog entry longer than a page;
/// the second `SELECT` is `1` only where that `CHECK` came back with its whole literal.
const SELECT_LARGE_ROWS: &str = "SELECT id, body FROM large_probe ORDER BY id";
const SELECT_LARGE_CHECK: &str = "SELECT count(*) FROM information_schema.check_constraints \
     WHERE constraint_name = 'large_probe_body_check' AND length(check_clause) > 6000";

/// A sequence and an enum of **one name**, twice over: a plain sequence an ordinary
/// column draws from, beside a column of the enum of its name, and a `SERIAL`'s sequence
/// beside an enum of *its* name that a later column of the owning table declares. Both
/// engines build the pair only sequence-first — `CREATE SEQUENCE` over an enum's name is
/// `42710` — so these restore only while the dump emits every `CREATE SEQUENCE` ahead of
/// every `CREATE TYPE`; emitted the other way round the restore stops at the first
/// sequence on both legs.
const SELECT_SHARED_NAME_ROWS: &str = "SELECT id, m::text AS m, k, s FROM shared_name_user \
     ORDER BY id";
/// Both shared-name sequences' **resume points**, read after the rows above: a restore
/// that created the type and lost the sequence has no relation for either literal.
const SELECT_SHARED_NAME_RESUME: &str =
    "SELECT nextval('shared_name'), nextval('shared_name_user_s_seq')";

/// The names of the indexes `CREATE INDEX ON …` minted, read back by name: the index the
/// first statement named was dropped, so the survivor carries a digit a restore could not
/// arrive at by choosing the name again — it holds only where the dump wrote the name the
/// catalog kept. The table's own name needs quoting, and so does every name minted from it.
const SELECT_UNNAMED_INDEXES: &str = "SELECT indexname::text AS indexname FROM pg_indexes \
     WHERE tablename = 'Unnamed Ix' ORDER BY 1";

/// The constraint names the engine chose over long table and column names, read back by
/// name. Each is cut to 18.6's 63 bytes where it is chosen, and several meet only after
/// the cut — so a dump that wrote them whole, or a chooser that cut them differently from
/// 18.6's, restores short there: 18.6 truncates the identifier it reads, and the second
/// `ADD CONSTRAINT` of a pair is refused while `psql` exits `0`.
const SELECT_LONG_CHOSEN_NAMES: &str = "SELECT contype::text AS contype, conname::text AS conname \
     FROM pg_constraint WHERE conname LIKE 'long\\_chosen%' AND contype <> 'n' ORDER BY 1, 2";

/// The `jsonb`-keyed parent's rows, read in its primary key's order.
const SELECT_CONTAINER_KEY_PARENT: &str = "SELECT v, k::text AS key FROM ck_parent ORDER BY k";

/// The child's rows: a `jsonb` foreign key, a `text[]` unique key, a composite key
/// with an `int[]` part.
const SELECT_CONTAINER_KEY_CHILD: &str =
    "SELECT id, k::text AS k, tags, tenant, nums FROM ck_child ORDER BY id";

/// The container-key indexes as the catalog renders them, access method included.
const SELECT_CONTAINER_KEY_INDEXES: &str = "SELECT indexname::text AS indexname, indexdef \
     FROM pg_indexes WHERE tablename IN ('ck_parent', 'ck_child', 'ck_gin') ORDER BY 1";

/// The GIN index over `jsonb[]` answers a containment probe by `jsonb`'s equality.
const SELECT_CONTAINER_KEY_GIN: &str =
    "SELECT id FROM ck_gin WHERE arr @> ARRAY['1.0'::jsonb] ORDER BY id";

/// The restored container-key indexes **enforce by the key**, which rows alone cannot
/// see: each statement below offers one row whose key a stored row already holds
/// — a `jsonb` number at another scale, the same tags, the same `(tenant, nums)`, the
/// same `k -> 0` — beside one that is new, and only the new one comes back.
const SELECT_CONTAINER_KEY_PKEY_HOLDS: &str = "INSERT INTO ck_parent (k, v) VALUES \
     ('{\"a\":1.0}', 90), ('[9]', 91) ON CONFLICT (k) DO NOTHING RETURNING v";
const SELECT_CONTAINER_KEY_UNIQUE_HOLDS: &str = "INSERT INTO ck_child (id, tags) VALUES \
     (90, '{x,y}'), (91, '{y,x}') ON CONFLICT (tags) DO NOTHING RETURNING id";
const SELECT_CONTAINER_KEY_COMPOSITE_HOLDS: &str = "INSERT INTO ck_child (id, tenant, nums) \
     VALUES (92, 1, '{1,NULL}'), (93, 2, '{1,NULL}') \
     ON CONFLICT (tenant, nums) DO NOTHING RETURNING id";
const SELECT_CONTAINER_KEY_EXPRESSION_HOLDS: &str = "INSERT INTO ck_child (id, k) VALUES \
     (94, '[1,2.0]'), (95, '[9]') ON CONFLICT ((k -> 0)) DO NOTHING RETURNING id";

/// The `bytea` table's stored rows — a value of each input spelling and a `bytea[]`.
/// **Weak on their own**, like the precision rows: a row travels as its `\x…` hex in the
/// `COPY` text whatever read it at the source, so a reader that stored other bytes for
/// `'abcd'` would restore them identically. [`SELECT_BYTEA_DEFAULT`] is the probe.
const SELECT_BYTEA_ROWS: &str = "SELECT id, b, bs, length(b) AS n FROM bytea_probe ORDER BY id";
/// A write through the *restored* defaults: the dump carries each `DEFAULT` as its
/// string, so the restored catalog reads it again — as hex or as the escape format —
/// and the row shows which. Runs after [`SELECT_BYTEA_ROWS`] because it adds a row.
const SELECT_BYTEA_DEFAULT: &str = "INSERT INTO bytea_probe (id) VALUES (90); \
     SELECT b, plain, escaped, hex FROM bytea_probe WHERE id = 90";

/// Every compared `SELECT`, in one list so adding a table is a one-line change.
const COMPARED_SELECTS: [&str; 43] = [
    SELECT_AUTHOR,
    SELECT_POST,
    SELECT_EXPR,
    SELECT_LOW_COUNTERS,
    SELECT_RESTARTED_START,
    SELECT_IDENT_ROWS,
    SELECT_IDENT_DEFAULT_ROWS,
    SELECT_IDENT_NAMED_ROWS,
    SELECT_IDENT_LOW_ROWS,
    SELECT_IDENT_SIBLING,
    SELECT_IDENT_BORROWER,
    SELECT_IDENT_COUNTERS,
    SELECT_IDENT_STARTS,
    SELECT_IDENT_ACCEPTS,
    SELECT_QUOTED_ROWS,
    SELECT_QUOTED_IDENT_ROWS,
    SELECT_QUOTED_BORROWED,
    SELECT_QUOTED_DOUBLED,
    SELECT_QUOTED_COUNTERS,
    SELECT_EXPR_DEFAULT_HOIST,
    SELECT_EXPR_DEFAULT_INLINE,
    SELECT_EXPR_DEFAULT_RESUME,
    SELECT_PRECISION_ROWS,
    SELECT_PRECISION_STORE,
    SELECT_PRECISION_CATALOG,
    SELECT_PRECISION_TYPMOD,
    SELECT_INFINITY_ROWS,
    SELECT_LARGE_ROWS,
    SELECT_LARGE_CHECK,
    SELECT_SHARED_NAME_ROWS,
    SELECT_SHARED_NAME_RESUME,
    SELECT_UNNAMED_INDEXES,
    SELECT_LONG_CHOSEN_NAMES,
    SELECT_CONTAINER_KEY_PARENT,
    SELECT_CONTAINER_KEY_CHILD,
    SELECT_CONTAINER_KEY_INDEXES,
    SELECT_CONTAINER_KEY_GIN,
    SELECT_CONTAINER_KEY_PKEY_HOLDS,
    SELECT_CONTAINER_KEY_UNIQUE_HOLDS,
    SELECT_CONTAINER_KEY_COMPOSITE_HOLDS,
    SELECT_CONTAINER_KEY_EXPRESSION_HOLDS,
    SELECT_BYTEA_ROWS,
    SELECT_BYTEA_DEFAULT,
];

/// A guarantee whose observable is a **refusal**, not a row.
///
/// Rows are the wrong instrument for the fact an identity rendering carries: an
/// identity column and a plain `nextval`-defaulted one hold the same values and
/// answer every `SELECT` identically. What separates them is what they *reject* —
/// which is also the whole of identity's usefulness, and which vanished silently
/// across pglet's own round-trip while every row still matched.
struct IdentityRefusal {
    /// The statement, run against the source and against each restored copy.
    sql: &'static str,
    /// The SQLSTATE both engines answer (measured on 18.6 with `VERBOSITY verbose`
    /// and on pglet). Asserting the code and not merely "some error" is what stops a
    /// typo'd table name from passing as a guarantee.
    sqlstate: &'static str,
}

/// The identity guarantees, stated over three of the corpus's identity columns.
///
/// The split across the three tables is deliberate. `ident_always` carries all four,
/// including the `428C9` that *is* `GENERATED ALWAYS`. `ident_default` carries the
/// three that are about **identity** rather than generation — a `BY DEFAULT` column
/// still refuses to have its default dropped, its sequence detached, or its sequence
/// dropped — and its accept polarity rides [`SELECT_IDENT_ACCEPTS`] instead, so the
/// pair pins both directions on one column. `ident_named` carries the two
/// sequence-side ones under the sequence's **declared** name, which is the only
/// plain-SQL observable that the name travelled as an identity sequence rather than
/// being re-minted as an ordinary one.
const IDENTITY_REFUSALS: [IdentityRefusal; 9] = [
    IdentityRefusal {
        sql: "INSERT INTO ident_always (id, note) VALUES (900, 'forced')",
        sqlstate: "428C9",
    },
    IdentityRefusal {
        sql: "ALTER TABLE ident_always ALTER COLUMN id DROP DEFAULT",
        sqlstate: "42601",
    },
    IdentityRefusal {
        sql: "ALTER SEQUENCE ident_always_id_seq OWNED BY NONE",
        sqlstate: "0A000",
    },
    IdentityRefusal {
        sql: "DROP SEQUENCE ident_always_id_seq CASCADE",
        sqlstate: "2BP01",
    },
    IdentityRefusal {
        sql: "ALTER TABLE ident_default ALTER COLUMN id DROP DEFAULT",
        sqlstate: "42601",
    },
    IdentityRefusal {
        sql: "ALTER SEQUENCE ident_default_id_seq OWNED BY NONE",
        sqlstate: "0A000",
    },
    IdentityRefusal {
        sql: "DROP SEQUENCE ident_default_id_seq CASCADE",
        sqlstate: "2BP01",
    },
    IdentityRefusal {
        sql: "ALTER SEQUENCE journal_counter OWNED BY NONE",
        sqlstate: "0A000",
    },
    IdentityRefusal {
        sql: "DROP SEQUENCE journal_counter CASCADE",
        sqlstate: "2BP01",
    },
];

/// Build the populated embed `Database`: an ENUM, a parent table with a `SERIAL`
/// PK, and a child table with a `FOREIGN KEY`, a `text[]`, a `NOT NULL` + `DEFAULT`
/// enum column, and rows carrying a NULL and a COPY-escaping value (tab, newline,
/// backslash).
///
/// It also seeds the **sequence states a dump can render unloadably**, which is why
/// the round-trip stayed green through a regression that broke it: the corpus simply
/// had no sequence outliving its owner and no counter below `1`. Those states are
/// reachable through ordinary accepted DDL (see [`seed_sequence_edge_cases`]), so the
/// dump invariant's catalog part they guard — *any catalog pglet's own accepted
/// statements build must dump to something that loads into stock Postgres and back into
/// pglet* — needs them present.
fn build_source() -> Database {
    let mut db = Database::with_btree();
    let setup = [
        "CREATE TYPE status AS ENUM ('active', 'inactive')",
        "CREATE TABLE author (id SERIAL PRIMARY KEY, name TEXT NOT NULL)",
        "CREATE TABLE post (
            id SERIAL PRIMARY KEY,
            author_id INT REFERENCES author(id) ON DELETE CASCADE,
            title TEXT NOT NULL DEFAULT 'untitled',
            state status NOT NULL DEFAULT 'active',
            tags TEXT[],
            body TEXT,
            created TIMESTAMPTZ,
            meta JSONB
        )",
        // Every type that carries a sub-second precision, scalar and array, plus the
        // two catalog shapes a restore can confuse with each other: a bare `timestamp`
        // (no modifier) and a `timestamp(6)` (a captured modifier whose rounding is a
        // no-op). The values are stored **rounded**, which is what makes
        // `SELECT_PRECISION_STORE`'s post-restore write — not these rows — the probe
        // that can see a dropped `(p)`; and the `g`/`h` pair is visible to exactly one
        // reader, `SELECT_PRECISION_TYPMOD`, because a `p = 6` rounding is a no-op and
        // `datetime_precision` answers `6` for both.
        "CREATE TABLE precision_probe (
            id INT PRIMARY KEY,
            a TIMESTAMP(3),
            b TIMESTAMPTZ(2),
            c TIME(1),
            d INTERVAL(4),
            e TIMESTAMP(3)[],
            f INTERVAL(3)[],
            g TIMESTAMP,
            h TIMESTAMP(6)
        )",
        "INSERT INTO precision_probe (id, a, b, c, d, e, f, g, h) VALUES \
            (1, '2020-01-02 03:04:05.6789', '2020-01-02 03:04:05.6789+00', '03:04:05.6789', \
             '1.23456 secs', '{2020-01-02 03:04:05.6789}', '{1.23456 secs}', \
             '2020-01-02 03:04:05.6789', '2020-01-02 03:04:05.6789'), \
            (2, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
        "INSERT INTO author (name) VALUES ('Ada'), ('Bob')",
        // Row 1: explicit values incl. a body needing COPY escaping (real tab +
        // newline + a literal backslash), a `timestamptz` (guards the UTC `+00`
        // round-trip), and a single-key `jsonb`. Row 2: DEFAULTs + NULLs.
        "INSERT INTO post (author_id, title, state, tags, body, created, meta) VALUES \
            (1, 'first', 'inactive', '{x,y}', 'a\tb\nc\\d', '2024-01-15 12:30:45+00', '{\"k\":\"v\"}'), \
            (2, DEFAULT, DEFAULT, NULL, NULL, NULL, NULL)",
        // Stored expressions across every renderable scalar-function family, at all
        // three sites. Restricted to what *stock Postgres* accepts at each site:
        // a CHECK takes any function (Postgres applies no volatility gate there, so
        // `now()` is fine), but `CREATE INDEX` requires IMMUTABLE for both key
        // expressions and predicates — so the index side stays IMMUTABLE-only.
        // Deliberately-Postgres-unloadable shapes (a volatile index predicate, the
        // pglet-only `extract(<expr>, source)` comma form) live in the in-process
        // `dump.rs` unit tests instead.
        "CREATE TABLE expr_probe (
            id INT PRIMARY KEY,
            txt TEXT,
            num NUMERIC(6,2),
            ts TIMESTAMP,
            uu UUID,
            a4 INT[],
            a8 BIGINT[],
            -- A **declared** constraint name that is also the *auto*-name base of the
            -- unnamed CHECK below (a two-column predicate names `<table>_check`). The
            -- dump used to number the unnamed checks by its own ordinal —
            -- `expr_probe_check`, `expr_probe_check2`, … — blind to the declared names
            -- it had already emitted, so this pair emitted `ADD CONSTRAINT
            -- expr_probe_check` twice and the dump stopped reloading into *either*
            -- engine. Without a declared-name CHECK on a table that also has unnamed
            -- ones, that collision is unreachable and the guard is vacuous.
            CONSTRAINT expr_probe_check CHECK (id > 0),
            -- Two *different* array types in one common-type construct. This is the
            -- shape whose over-rejection broke dump portability: a store written by an
            -- older binary carrying this CHECK still read and wrote fine, but its dump
            -- stopped restoring into pglet while the byte-identical dump restored into
            -- real Postgres. The `OR TRUE` keeps the constraint satisfiable for the
            -- all-NULL seed row while still forcing the branch types to resolve.
            CHECK (COALESCE(a4, a8) IS NOT NULL OR TRUE),
            -- `GREATEST` in a stored expression, now that it is statically checked —
            -- a well-typed one must still be accepted and must still round-trip.
            CHECK (GREATEST(id, 0) >= 0),
            CHECK (abs(id) < 1000),
            CHECK (sign(id) >= 0),
            CHECK (position('x' IN txt) >= 0),
            CHECK (substring(txt, 1, 2) <> 'zz'),
            CHECK (split_part(txt, ',', 1) <> 'zz'),
            CHECK (concat_ws('-', txt, 'z') <> ''),
            CHECK (nullif(id, -1) IS NOT NULL),
            -- A folded **negative constant** as a binary operator's operand. Unary `-`
            -- over a numeric constant folds into the constant at parse time (Postgres's
            -- `doNegate`), so what is persisted here is a plain negative literal rather
            -- than the `Expr::Negate` wrapper it used to be — and both the emitter's
            -- rendering of it and stock Postgres's acceptance of that rendering are on
            -- this path. `nullif(id, -1)` above carries the same constant through a
            -- *function argument*; this one carries it through `render_binary_op`.
            CHECK (id > -1),
            CHECK (jsonb_build_object('k', txt) IS NOT NULL),
            CHECK (id = ANY (ARRAY[1, 2, 3])),
            CHECK (extract(year FROM ts) > 1999),
            CHECK (date_part('year', ts) > 1999),
            CHECK (date_trunc('day', ts) IS NOT NULL),
            CHECK (round(num, 2) >= 0),
            CHECK (ts <= now()),
            CHECK (uu <> gen_random_uuid()),
            -- The call-spelled string, digest and arithmetic functions, each rendered
            -- under its own name — `left` / `right` as TYPE_FUNC_NAME keywords both
            -- engines read back as calls — and the volatile `random()`, which a CHECK
            -- may hold on both.
            CHECK (left(txt, 1) <> 'z'),
            CHECK (right(txt, -1) <> 'zz'),
            CHECK (substr(txt, 1, 2) <> 'zz'),
            CHECK (octet_length(txt) < 100),
            CHECK (md5(txt) <> ''),
            CHECK (mod(id, 1000) >= 0),
            CHECK (power(id, 2) >= 0),
            CHECK (pow(num, 2) >= 0),
            CHECK (random() < 2),
            -- A conversion to an enum over a non-constant operand, persisted naming the
            -- type it converts to: it must dump as that one cast, `(…)::status`, which
            -- both engines load behind the dump's `CREATE TYPE`.
            CHECK ((CASE WHEN id > 0 THEN 'active' END)::status IS NOT NULL OR TRUE)
        )",
        "CREATE INDEX expr_probe_upper ON expr_probe ((upper(txt)))",
        "CREATE INDEX expr_probe_abs ON expr_probe ((abs(id)))",
        "CREATE INDEX expr_probe_pos ON expr_probe ((position('x' IN txt)))",
        "CREATE INDEX expr_probe_live ON expr_probe (id) WHERE btrim(txt) <> ''",
        "CREATE INDEX expr_probe_left ON expr_probe ((left(txt, 2)))",
        "CREATE INDEX expr_probe_md5 ON expr_probe ((md5(txt)))",
        "CREATE INDEX expr_probe_mod ON expr_probe ((mod(id, 7))) WHERE octet_length(txt) > 0",
        "INSERT INTO expr_probe (id, txt, num, ts, uu) VALUES \
            (1, 'axb', 1.50, '2024-01-15 12:30:45', '00000000-0000-4000-8000-000000000001'), \
            (2, ' q ', 0, '2025-06-01 00:00:00', NULL), \
            (3, NULL, NULL, '2026-02-03 04:05:06', NULL)",
        "CREATE TABLE infinity_probe (
            id INT PRIMARY KEY,
            d DATE NOT NULL DEFAULT 'infinity' CHECK (d > '-infinity'::date),
            ts TIMESTAMP,
            tz TIMESTAMPTZ(3),
            ds DATE[],
            tzs TIMESTAMPTZ[]
        )",
        "CREATE INDEX infinity_probe_tz ON infinity_probe (tz)",
        "INSERT INTO infinity_probe (id, ts, tz, ds, tzs) VALUES \
            (1, 'infinity', '-infinity', '{infinity,-infinity,NULL}', \
             '{\"2000-01-01 00:00:00+05\",infinity,NULL}')",
        "INSERT INTO infinity_probe (id, d, ts, tz, ds) VALUES \
            (2, '9999-12-31', '-infinity', 'infinity', '{2000-01-01}'), \
            (3, '0001-01-01', NULL, NULL, NULL)",
    ];
    for sql in setup {
        db.execute(sql).expect("source setup");
    }
    seed_sequence_edge_cases(&mut db);
    seed_identity_columns(&mut db);
    seed_quoted_sequence_names(&mut db);
    seed_expression_defaults(&mut db);
    seed_large_values(&mut db);
    seed_shared_type_and_sequence_names(&mut db);
    seed_unnamed_indexes(&mut db);
    seed_long_chosen_names(&mut db);
    seed_container_keys(&mut db);
    seed_bytea(&mut db);
    db
}

/// A `bytea` table whose values and defaults are written as strings of each input
/// spelling — a run of hex digits with no prefix, text, whitespace, the escape format's
/// `\\` and `\ooo`, and the hex format with whitespace between its pairs.
fn seed_bytea(db: &mut Database) {
    for sql in [
        "CREATE TABLE bytea_probe (
            id INT PRIMARY KEY,
            b BYTEA DEFAULT 'beef',
            bs BYTEA[],
            plain BYTEA DEFAULT 'hello there',
            escaped BYTEA DEFAULT 'a\\\\b\\000\\377 ',
            hex BYTEA DEFAULT '\\x6162 63'
        )",
        "INSERT INTO bytea_probe (id, b, bs) VALUES \
            (1, 'abcd', '{abcd,\"\\\\x6162\",NULL}'), (2, '\\xabcd', '{}'), (3, ' ', NULL), \
            (4, '', '{\"\",\" \"}'), (5, 'a\\\\b\\000\\377', '{\"a\\\\\\\\b\"}'), \
            (6, '\\x6162 63', NULL), (7, 'h\u{e9}llo', NULL), (8, NULL, NULL), \
            (9, '\\x00010a0d5c22277f80ff', '{\"\\\\x00010a0d5c22277f80ff\"}')",
    ] {
        db.execute(sql).expect("bytea setup");
    }
}

/// The tables keyed on a `jsonb` or an array: a primary key, a foreign key into it, an
/// inline and a table-level `UNIQUE`, a unique expression index, a non-unique composite
/// one, and a GIN index over `jsonb[]` — each with rows in it, so the restore has to
/// build every one of them over data.
fn seed_container_keys(db: &mut Database) {
    for sql in [
        "CREATE TABLE ck_parent (v INT, k JSONB PRIMARY KEY)",
        "CREATE TABLE ck_child (id INT, k JSONB REFERENCES ck_parent, tags TEXT[] UNIQUE, \
         tenant INT, nums INT[], UNIQUE (tenant, nums))",
        "CREATE UNIQUE INDEX ck_child_first ON ck_child ((k -> 0))",
        "CREATE INDEX ck_child_nums ON ck_child (nums, id)",
        "CREATE TABLE ck_gin (id INT, arr JSONB[])",
        "CREATE INDEX ck_gin_ix ON ck_gin USING gin (arr)",
        "INSERT INTO ck_parent (k, v) VALUES ('{\"a\":1}', 1), ('[1,2]', 2), ('\"s\"', 3), \
         ('[]', 4), ('null', 5)",
        "INSERT INTO ck_child (id, k, tags, tenant, nums) VALUES \
         (1, '{\"a\":1.0}', '{x,y}', 1, '{1,NULL}'), (2, '[1,2]', '{}', 1, '{}'), \
         (3, NULL, NULL, NULL, NULL), (4, '\"s\"', '{x}', 2, '{1}')",
        "INSERT INTO ck_gin VALUES (1, ARRAY['1'::jsonb, '{\"a\":1}'::jsonb]), (2, '{}'), \
         (3, ARRAY['2'::jsonb]), (4, NULL)",
    ] {
        db.execute(sql).expect("container-key setup");
    }
}

/// The indexes [`SELECT_UNNAMED_INDEXES`] reads: three minted names, the first of them
/// dropped again.
fn seed_unnamed_indexes(db: &mut Database) {
    for sql in [
        "CREATE TABLE \"Unnamed Ix\" (b INT, a INT, t TEXT)",
        "CREATE INDEX ON \"Unnamed Ix\" (a)",
        "CREATE INDEX ON \"Unnamed Ix\" (a)",
        "CREATE UNIQUE INDEX ON \"Unnamed Ix\" (lower(t), a)",
        "DROP INDEX \"Unnamed Ix_a_idx\"",
        "INSERT INTO \"Unnamed Ix\" VALUES (1, 10, 'x'), (2, 20, 'y')",
    ] {
        db.execute(sql).expect("unnamed-index setup");
    }
}

/// The constraints [`SELECT_LONG_CHOSEN_NAMES`] reads: every kind, declared without a
/// name, over a 40-byte table with 50-byte and 41-byte columns, and over two 60-byte
/// tables whose names differ in their last byte.
fn seed_long_chosen_names(db: &mut Database) {
    let table = format!("long_chosen_{}", "t".repeat(28));
    let (e, d, c) = (
        format!("e{}", "o".repeat(49)),
        format!("d{}", "o".repeat(9)),
        format!("c{}f", "o".repeat(39)),
    );
    let pair = format!("long_chosen_pair_{}", "t".repeat(42));
    for sql in [
        "CREATE TABLE long_chosen_parent (id INT PRIMARY KEY)".to_string(),
        format!(
            "CREATE TABLE {table} ({d} INT, {e} INT, {c} INT REFERENCES long_chosen_parent, \
             UNIQUE ({e}), UNIQUE ({e}, {d}), CHECK ({e} > 0), CHECK ({e} > 1))"
        ),
        format!("ALTER TABLE {table} ADD FOREIGN KEY ({c}) REFERENCES long_chosen_parent"),
        format!("CREATE TABLE {pair}a (id INT PRIMARY KEY, v INT UNIQUE CHECK (v > 0))"),
        format!("CREATE TABLE {pair}b (id INT PRIMARY KEY, v INT UNIQUE CHECK (v > 0))"),
        "INSERT INTO long_chosen_parent VALUES (1)".to_string(),
        format!("INSERT INTO {table} VALUES (1, 2, 1)"),
        format!("INSERT INTO {pair}b VALUES (1, 2)"),
    ] {
        db.execute(&sql).expect("long-chosen-name setup");
    }
}

/// The shared-name pairs [`SELECT_SHARED_NAME_ROWS`] reads: each sequence created ahead
/// of the enum of its name, the only order either engine accepts.
fn seed_shared_type_and_sequence_names(db: &mut Database) {
    for sql in [
        "CREATE SEQUENCE shared_name START WITH 40",
        "CREATE TYPE shared_name AS ENUM ('p', 'q')",
        "CREATE TABLE shared_name_user (id INT PRIMARY KEY, m shared_name, \
         k INT DEFAULT nextval('shared_name'), s SERIAL)",
        "CREATE TYPE shared_name_user_s_seq AS ENUM ('x')",
        "ALTER TABLE shared_name_user ADD COLUMN t shared_name_user_s_seq",
        "INSERT INTO shared_name_user (id, m, t) VALUES (1, 'q', 'x'), (2, 'p', NULL)",
    ] {
        db.execute(sql).expect("shared-name setup");
    }
}

/// `len` letters, deterministic in `seed` — nothing a `repeat()` could build, and
/// nothing pglz could shrink.
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

/// Seed the rows and the constraint one page cannot hold: [`SELECT_LARGE_ROWS`]'s
/// 100 000-character row, stored in an overflow chain, and [`SELECT_LARGE_CHECK`]'s
/// 6 000-character `CHECK` literal, a catalog entry longer than a page. The dump
/// reads both back out of the store and both restores must take them.
fn seed_large_values(db: &mut Database) {
    db.execute(&format!(
        "CREATE TABLE large_probe (id INT PRIMARY KEY, body TEXT CHECK (body <> '{}'))",
        letters(21, 6_000)
    ))
    .expect("large_probe");
    db.execute(&format!(
        "INSERT INTO large_probe VALUES (1, '{}'), (2, 'short')",
        letters(22, 100_000)
    ))
    .expect("large_probe rows");
}

/// Seed the columns whose `DEFAULT` is a general **expression**, at both sides of the
/// emitter's hoist.
///
/// A `ColumnDefault::Expr` renders through the same `Expr` deparser a `CHECK` does,
/// but nothing above reaches it: every default in the corpus is one of the ten
/// recognized closed forms, so the arm would render nowhere and this guard would be
/// green because its corpus lacks the shape — the failure mode this file has already
/// paid for once with the sequence-owner rendering.
///
/// Two facts need the seed, and they are on opposite sides of one predicate. The
/// hoist is **"references a sequence"**, not "is a bare `nextval`": `DEFAULT
/// nextval('s') + 1` names `s` exactly as the bare spelling does, so leaving it
/// inline would emit a `CREATE TABLE` ahead of its own `CREATE SEQUENCE` and stock
/// Postgres would refuse the restore. The sequence-free expressions are the control —
/// they must stay **inline**, or a table with an ordinary arithmetic default grows a
/// spurious `ALTER TABLE … SET DEFAULT` and the two engines disagree about where the
/// default came from.
///
/// **A third cell exists for the sequence the statement itself mints**, and the
/// difference from the hoisted cell is the whole point: `expr_default_seq` is created
/// by an explicit `CREATE SEQUENCE` above, so the dump would emit one for it whatever
/// the registration did, while `expr_minted_seq` exists *only* because the
/// `CREATE TABLE`'s own default registered it — pglet's model of a `DEFAULT
/// nextval('x')` naming an absent sequence, which the bare spelling has always had.
/// The registration used to read the closed `ColumnDefault::NextVal` arm alone, so the
/// expression spelling registered **nothing**: the column then answered `42P01` at
/// every write, and the dump emitted its `SET DEFAULT nextval('expr_minted_seq') + 1`
/// with no `CREATE SEQUENCE` anywhere — the exact shape
/// [`check_every_nextval_has_a_sequence`] states, and which no seed could reach until
/// this one. The identity is what makes the invariant testable rather than merely
/// stated; the explicit-`CREATE SEQUENCE` cell cannot fail it.
///
/// The cell carries **two** nets and they fire in order. On the un-widened
/// registration the seed's own `INSERT` dies first (`SequenceNotFound`), because the
/// column is unwritable; drop that `INSERT` and
/// [`check_every_nextval_has_a_sequence`] fires instead, naming `expr_minted_seq` —
/// both measured. Keep the `INSERT`: the writability is the user-visible half, and
/// the check is the net behind it.
fn seed_expression_defaults(db: &mut Database) {
    for sql in [
        "CREATE SEQUENCE expr_default_seq",
        // The hoisted side: a sequence reference buried in an expression.
        "CREATE TABLE expr_default_hoist (
            k INT,
            drawn INT DEFAULT nextval('expr_default_seq') + 100
        )",
        // The auto-minted side: no `CREATE SEQUENCE` names `expr_minted_seq`, so the
        // dump can only emit one if the `CREATE TABLE`'s default registered it.
        "CREATE TABLE expr_minted_host (
            k INT,
            drawn INT DEFAULT nextval('expr_minted_seq') + 1
        )",
        "INSERT INTO expr_minted_host (k) VALUES (1), (2)",
        // The inline side, spanning the renderable families a default can reach:
        // arithmetic, a comparison (`b_expr` admits those), a function call, a `CASE`,
        // an array constructor, and a cast.
        "CREATE TABLE expr_default_inline (
            k INT,
            arith INT DEFAULT 6 * 7,
            cmp BOOL DEFAULT 5 = 4,
            fun TEXT DEFAULT upper('x'),
            branch INT DEFAULT CASE WHEN true THEN 1 ELSE 2 END,
            arr INT[] DEFAULT ARRAY[1, 2],
            cast_ TEXT DEFAULT (1 + 1)::text,
            lft TEXT DEFAULT left('abc', 2),
            sub TEXT DEFAULT substr('abc', 2),
            dig TEXT DEFAULT md5('a'),
            rem INT DEFAULT mod(7, 3),
            pw FLOAT8 DEFAULT power(2, 3),
            rnd FLOAT8 DEFAULT random()
        )",
        "INSERT INTO expr_default_hoist (k) VALUES (1), (2)",
        "INSERT INTO expr_default_inline (k) VALUES (1)",
    ] {
        db.execute(sql).expect("expression-default setup");
    }
}

/// Seed the sequences whose **names need quoting**, at both of the emitter's
/// string-literal name positions.
///
/// A sequence name reaches an emitted script at five sites. Three are *identifier*
/// positions (`CREATE SEQUENCE`, `ALTER SEQUENCE … OWNED BY`, `SEQUENCE NAME`) and two
/// are **string literals** inside a `regclass` argument (`nextval('…'::regclass)`,
/// `setval('…', …)`). The two grammars are not the same: a literal is read back by
/// Postgres's `SplitIdentifierString`, which **downcases** an unquoted part and
/// **rejects** interior whitespace, so a name that is not already a safe bare
/// identifier must carry its identifier quoting *inside* the string or the literal
/// names a different relation than the identifiers do.
///
/// The whole corpus was blind to that, because every sequence in it is called
/// something like `author_id_seq`, and for such a name the two spellings coincide.
/// The failure it hid is neither loud nor total: the restored table and all its rows
/// land, and only the column's default and the sequence's counter go missing — so on
/// Postgres the identity column silently renumbers from `1` into a collision.
///
/// Each cell answers a question no other answers:
///
/// - **`"Odd"`** — a `SERIAL` whose implicit sequence inherits the table's
///   **mixed case**. This is the *downcasing* failure, which is the silent one:
///   `nextval('Odd_id_seq')` is well-formed SQL naming `odd_id_seq`, so Postgres
///   reports only `relation "odd_id_seq" does not exist` at the two statements that
///   name it, and the restore carries on. It reaches **both** literal sites (the
///   hoisted `SET DEFAULT` and the `setval`).
/// - **`quoted_ident`** — an identity column with a **declared** `SEQUENCE NAME`
///   containing a space. This is the `invalid name syntax` failure, and it reaches the
///   `setval` site **only**: an identity column emits no `nextval` of its own, so a
///   fix applied to the `nextval` rendering alone leaves this one broken. The two
///   sites fail independently and one repair can miss one.
/// - **`quoted_borrower`** — an ordinary column drawing from that spaced sequence, so
///   the space name reaches the `nextval` site too. It also sorts *before*
///   `quoted_ident`, so its `CREATE TABLE` precedes the `ADD GENERATED` that creates
///   the sequence — the hoist is what makes that legal.
///
/// Both engines accept every statement here and answer every probe identically, which
/// is what makes this a differential guard rather than a pglet-only assertion.
fn seed_quoted_sequence_names(db: &mut Database) {
    for sql in [
        "CREATE TABLE \"Odd\" (id SERIAL PRIMARY KEY, note TEXT)",
        "CREATE TABLE quoted_ident (
            id INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME \"seq w space\" START WITH 700),
            note TEXT)",
        "CREATE TABLE quoted_borrower (
            borrowed INT DEFAULT nextval('\"seq w space\"'), note TEXT)",
        // A name carrying the *identifier* grammar's own escape, which is the only
        // shape that separates the two quotings: `quote_ident` doubles the `\"`, and
        // the outer string quoting must then leave that doubling intact.
        "CREATE SEQUENCE \"a\"\"b\" START WITH 60",
        "CREATE TABLE quoted_dq (id INT DEFAULT nextval('\"a\"\"b\"'), note TEXT)",
        "INSERT INTO \"Odd\" (note) VALUES ('a'), ('b')",
        "INSERT INTO quoted_ident (note) VALUES ('a'), ('b')",
        // Drawn after `quoted_ident`'s rows, so the borrowed value (702) is one
        // neither table could have produced alone.
        "INSERT INTO quoted_borrower (note) VALUES ('a')",
        "INSERT INTO quoted_dq (note) VALUES ('x')",
    ] {
        db.execute(sql).expect("quoted-name setup");
    }
    for name in ["Odd_id_seq", "seq w space", "a\"b"] {
        assert!(
            db.schema().sequences.contains_key(name),
            "the sequence must really be called {name:?} — a name the catalog folded \
             or trimmed is one whose two spellings coincide again, and this seed is \
             then testing nothing"
        );
    }
}

/// Seed the identity columns, whose *rendering* is the thing under test.
///
/// A catalog fact the emitter does not read is invisible to a round-trip that
/// compares rows, and identity was exactly that: an identity column and a plain
/// `nextval`-defaulted one hold identical values, so the whole corpus stayed green
/// while every identity guarantee — `GENERATED ALWAYS`'s `428C9` and the three
/// refusals that keep the column tied to its sequence — disappeared across pglet's
/// own dump. [`IDENTITY_REFUSALS`] is where that is read back; this is the catalog it
/// is read back from.
///
/// Every cell here answers a question no other cell can, and each was chosen against
/// a *plausible wrong emitter* rather than against the bug:
///
/// - `ident_always` / `ident_default` are the two **generations**. One alone cannot
///   see a rendering that hardcodes the other, and the pair is what makes an
///   over-correction (everything restored as `ALWAYS`) fail rather than look like a
///   stricter success.
/// - `ident_named` declares a **non-default** `SEQUENCE NAME`. With the usual
///   `<table>_<column>_seq` the restore re-mints the identical name for itself, so a
///   dump that drops the clause round-trips by coincidence — the same trap the parser
///   side hit, where discarding the declared name looked harmless until the two
///   differ.
/// - every table carries **rows**, so no restored counter is `1`; a corpus sitting at
///   the start on both the fixed and the broken build proves nothing about the
///   trailing `setval`.
/// - `ident_low` starts **below `1`**, which pglet accepts and stock Postgres will not
///   restore without an explicit floor. The floor now has to travel inside an identity
///   option list, which is a different statement from the `CREATE SEQUENCE` the
///   existing below-1 seeds cover.
/// - `ident_sibling` and `ident_borrower` are the two **ordering hazards** an identity
///   sequence's creation moving after its table creates: a sibling column of the same
///   table drawing from it, and a column of a table *earlier in name order* drawing
///   from it. Both are accepted by both engines and neither survives a dump whose
///   `CREATE TABLE` still names a sequence.
fn seed_identity_columns(db: &mut Database) {
    for sql in [
        "CREATE TABLE ident_always (id INT GENERATED ALWAYS AS IDENTITY, note TEXT)",
        "CREATE TABLE ident_default (
            id BIGINT GENERATED BY DEFAULT AS IDENTITY (START WITH 100), note TEXT)",
        "CREATE TABLE ident_named (
            id INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME journal_counter START WITH 500),
            note TEXT)",
        "CREATE TABLE ident_low (
            id INT GENERATED BY DEFAULT AS IDENTITY (START WITH 0), note TEXT)",
        // Hazard 1: `borrowed` draws from the identity sequence its own table's `id`
        // creates. Both engines accept it (the implicit sequence is created ahead of
        // the heap), and pg_dump breaks the resulting dependency loop the same way
        // this emitter does — by hoisting the default out of the `CREATE TABLE`.
        "CREATE TABLE ident_sibling (
            borrowed INT DEFAULT nextval('sibling_counter'),
            id INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME sibling_counter START WITH 300),
            note TEXT)",
        // Hazard 2: a *different* table, and `ident_borrower` sorts before
        // `ident_named` — so the dump emits its `CREATE TABLE` ahead of the
        // `ADD GENERATED` that creates the sequence it draws from. The name order is
        // load-bearing; renaming either table past the other retires the hazard
        // silently.
        // `also_borrowed` draws from an identity sequence at the **default** start,
        // whose emitted option list is therefore `(SEQUENCE NAME x)` and nothing else —
        // the one shape where the name butts against the closing paren. Every other
        // identity here carries a `START WITH`, so the name ends at whitespace and the
        // paren is never the terminator; without this cell a scanner that reads `x)` as
        // the created name registers a name nothing references, and the *real*
        // reference two statements later reports as an orphan.
        "CREATE TABLE ident_borrower (
            borrowed INT DEFAULT nextval('journal_counter'),
            also_borrowed INT DEFAULT nextval('ident_always_id_seq'),
            -- Hazard 2 through a general expression default. The emitter's hoist
            -- predicate is \"references a sequence\", not \"is a bare nextval\", and this
            -- is the cell that requires it: read as the bare form, this default stays
            -- inline in a CREATE TABLE emitted ahead of the ADD GENERATED that creates
            -- journal_counter, and stock Postgres refuses the restore. Every
            -- plain-sequence expression default in the corpus passes either way,
            -- because a plain sequence is created in the schema block ahead of the
            -- tables, so the hazard needs an identity sequence to be visible at all.
            expr_borrowed INT DEFAULT nextval('journal_counter') + 1000,
            note TEXT)",
        "INSERT INTO ident_always (note) VALUES ('a'), ('b')",
        "INSERT INTO ident_default (note) VALUES ('a'), ('b')",
        "INSERT INTO ident_named (note) VALUES ('a'), ('b')",
        "INSERT INTO ident_low (note) VALUES ('a'), ('b')",
        "INSERT INTO ident_sibling (note) VALUES ('a'), ('b')",
        // Drawn after `ident_named`'s rows, so `journal_counter` reads 503 rather than
        // a value either table could have produced alone.
        "INSERT INTO ident_borrower (note) VALUES ('a')",
    ] {
        db.execute(sql).expect("identity setup");
    }
    assert!(
        db.schema().sequences.contains_key("journal_counter"),
        "the declared SEQUENCE NAME must be the sequence's real name, or the \
         coincidence this corpus exists to break is back"
    );
    assert!(
        !db.schema().sequences.contains_key("ident_named_id_seq"),
        "a re-minted default name beside the declared one would make the restore's \
         setval land on whichever the emitter happened to pick"
    );
}

/// Seed the sequence states whose *rendering* is the thing under test.
///
/// Two families:
///
/// 1. **A sequence outliving its owner.** The `owner` names a relation or column the
///    catalog no longer has, and rendering it would emit an `ALTER SEQUENCE … OWNED
///    BY` that stock Postgres rejects — and that **pglet's own restore rejects too**.
///    This proves the dump does not turn the leak into an unloadable script.
///
///    **No route reaches the state any more**, so the seed is built on the catalog by
///    hand. The two `DROP` routes this used to seed with — a dropped table and a
///    dropped `SERIAL` column — stopped leaking when an owned sequence became
///    auto-dropped with its owner, and the third, an `ADD COLUMN … SERIAL` failing
///    *after* registering its sequence, stopped when that registration was staged
///    behind the backfill validation (`executor::ddl`'s `staged_sequences`). All three
///    are kept below as **live seeds all the same**, each asserted from the other side
///    — the sequence is *gone* — so a route that regressed to leaking fails here rather
///    than quietly becoming this test's real coverage with nothing saying so.
///    (Re-derived: `owned_sequences` removes every sequence whose `owner` names the
///    dropped table or column, so *no* `DROP` can leave one orphaned — the enumeration
///    holds, and it held through D7b's identity-detach bug too, whose routes all
///    auto-dropped the sequence rather than orphaning it. The enumeration is over
///    writers of the **owner edge**, so it also covers `CREATE SEQUENCE … OWNED BY` and
///    `ALTER SEQUENCE … OWNED BY`: both build the record through
///    `executor::ddl`'s `resolve_sequence_owner`, which raises `42809` / `42P01` /
///    `42703` before a `SequenceOwner` exists, so neither can record one that does not
///    resolve.)
///
///    The hand-built seed is the point rather than a shortcut: a guard whose only seed
///    is a defect stops testing the emitter the moment the defect is fixed — and goes
///    *green* rather than red, so nothing announces it. Closing the two `DROP` routes
///    took two of this guard's three seeds and left it looking covered; closing the
///    third took the last one. It is seeded in **both** arms of the owner pair — a gone
///    table and an existing table's gone column — because the emitter reads both, and a
///    single-arm seed leaves half its test unexercised.
///
///    **Its symmetric twin had no seed and no assertion**, which is how D7b's worst
///    half crossed the whole dump corpus: a *column default* naming a sequence the
///    catalog lacks. Renders like any other default, restores nowhere. Nothing here
///    could produce it, so nothing here looked for it —
///    [`check_every_nextval_has_a_sequence`] now states the property whether or not a
///    seed reaches it.
/// 2. **A counter at or below zero**, which pglet accepts (it models no `MINVALUE`)
///    and stock Postgres will not restore without an explicit floor.
///
/// Plus an `OWNED BY NONE` detach: its *rendering* is what this checks, while the
/// detach's plain-SQL observable is a fixture of its own (`TRUNCATE … RESTART
/// IDENTITY` restarts only owned sequences).
fn seed_sequence_edge_cases(db: &mut Database) {
    for sql in [
        // Two drop routes that must take their sequence with them, so the emitter is
        // exercised over a catalog where they are absent rather than orphaned.
        "CREATE TABLE dropped_host (a SERIAL, b INT)",
        "DROP TABLE dropped_host",
        "CREATE TABLE dropped_col (a SERIAL, b INT)",
        "ALTER TABLE dropped_col DROP COLUMN a",
        // The counter families.
        "CREATE SEQUENCE low_start_seq START WITH 0",
        "CREATE SEQUENCE neg_counter_seq",
        "SELECT setval('neg_counter_seq', -5)",
        // The detach.
        "CREATE TABLE detach_host (a SERIAL, b INT)",
        "ALTER SEQUENCE detach_host_a_seq OWNED BY NONE",
    ] {
        db.execute(sql).expect("sequence edge-case setup");
    }
    // The third closed route, seeded and asserted the same way: the statement is
    // *expected* to fail (the backfilled `c` violates its own CHECK), and its sequence
    // is staged behind that validation, so nothing is registered.
    db.execute("CREATE TABLE failed_add (x INT)")
        .expect("failed-add table");
    db.execute("INSERT INTO failed_add VALUES (1)")
        .expect("failed-add row");
    assert!(
        db.execute("ALTER TABLE failed_add ADD COLUMN c SERIAL CHECK (c > 100)")
            .is_err(),
        "the ADD COLUMN must fail for this seed to say anything"
    );
    for gone in [
        "dropped_host_a_seq",
        "dropped_col_a_seq",
        "failed_add_c_seq",
    ] {
        assert!(
            !db.schema().sequences.contains_key(gone),
            "{gone} must not survive its statement — otherwise it is a second, \
             unannounced source of the dangling owner the hand-built seed below builds"
        );
    }
    // The dangling owner itself, built on the catalog because no DDL reaches it — and
    // **both arms of it**, because the owner is a `(table, column)` pair and
    // `emit_sequence_owned_by` tests both halves. One names a table the schema does not
    // hold; the other names `failed_add`, which it does, and a column `c` it does not
    // (the `ADD COLUMN` above was refused). Without the second, narrowing the emitter's
    // test to the table half alone renders an `OWNED BY` no assertion here would catch.
    db.schema_mut().sequences.insert(
        "orphan_owner_seq".to_string(),
        Sequence::owned_by(1, "gone_table", "gone_column"),
    );
    db.schema_mut().sequences.insert(
        "orphan_column_seq".to_string(),
        Sequence::owned_by(1, "failed_add", "c"),
    );
}

/// The last statement's result of running `sql` on the embed `Database`.
fn embed_query(db: &mut Database, sql: &str) -> ExecuteResult {
    db.execute(sql)
        .expect("embed query")
        .pop()
        .expect("embed query returns a result")
}

/// Render an embed `ExecuteResult` to the `Option<String>` cell grid the
/// simple-query (text) protocol returns — each value through the column-type-aware
/// text encoder, NULL as `None`.
fn render_embed(result: &ExecuteResult) -> Vec<Vec<Option<String>>> {
    result
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(i, v)| value_to_text_with_oid(v, result.columns()[i].type_oid))
                .collect()
        })
        .collect()
}

/// Reset a backend's `public` schema (the `DROP SCHEMA public CASCADE; CREATE
/// SCHEMA public` idiom both backends support) so the restore starts clean.
async fn reset_schema(client: &Client) -> Result<(), String> {
    client
        .simple_query("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .await
        .map_err(|e| format!("schema reset failed: {e}"))?;
    Ok(())
}

/// Replay the dump into a backend via `psql -f` under `ON_ERROR_STOP=1`. Any
/// statement the backend rejects makes psql exit non-zero — a loud restore failure.
///
/// `session_zone` is the restore session's `TimeZone`, sent in the startup packet (`PGTZ`).
/// The Postgres leg runs at a zone that is not UTC, whatever the server's own `TimeZone`,
/// so a `timestamptz` the dump writes without its `+00` loads as a different instant rather
/// than agreeing by coincidence. pglet has one zone and refuses a startup that asks for
/// another, so its leg sends none.
fn restore_via_psql(
    config: &DbConfig,
    dump_path: &PathBuf,
    session_zone: Option<&str>,
) -> Result<(), String> {
    let mut command = Command::new("psql");
    if let Some(zone) = session_zone {
        command.env("PGTZ", zone);
    }
    command
        .args(["-v", "ON_ERROR_STOP=1", "-q", "-f"])
        .arg(dump_path);
    apply_conninfo(&mut command, config);
    let output = command
        .output()
        .map_err(|e| format!("psql spawn failed: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "psql restore failed (ON_ERROR_STOP):\n  stdout: {}\n  stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ))
    }
}

/// Assert every [`IDENTITY_REFUSALS`] statement is refused, under its SQLSTATE, by
/// the **source** embed `Database`.
///
/// Run before the dump is compared anywhere, because a corpus that does not itself
/// hold the guarantees cannot witness their loss: if `ident_always` were not really a
/// `GENERATED ALWAYS` column, every restored copy would agree with it and the guard
/// would be a tautology. Each statement is refused, so none of them mutates.
fn assert_source_refuses_identity_violations(db: &mut Database) -> Result<(), String> {
    for probe in &IDENTITY_REFUSALS {
        match db.execute(probe.sql) {
            Ok(_) => {
                return Err(format!(
                    "the source accepted `{}`, so the corpus does not hold the \
                     guarantee the restore is checked against",
                    probe.sql
                ));
            }
            Err(e) if e.sqlstate() == probe.sqlstate => {}
            Err(e) => {
                return Err(format!(
                    "the source refused `{}` under {} rather than {}: {e}",
                    probe.sql,
                    e.sqlstate(),
                    probe.sqlstate
                ));
            }
        }
    }
    Ok(())
}

/// Assert every [`IDENTITY_REFUSALS`] statement is refused, under its SQLSTATE, by a
/// restored copy — the observable that rows cannot carry.
///
/// Runs on **both** legs: Postgres makes it differential (the codes are 18.6's own),
/// and the pglet self-restore makes it a round-trip. A rendering that loses identity
/// fails here on both, since the restored column is then an ordinary
/// `nextval`-defaulted one that accepts every statement below.
async fn assert_backend_refuses_identity_violations(
    label: &str,
    client: &Client,
) -> Result<(), String> {
    for probe in &IDENTITY_REFUSALS {
        let Err(e) = client.simple_query(probe.sql).await else {
            return Err(format!(
                "{label}: `{}` succeeded — the restored column is not an identity \
                 column, so every guarantee identity exists to provide is gone while \
                 every row still matches",
                probe.sql
            ));
        };
        let code = e
            .code()
            .map_or("<none>", tokio_postgres::error::SqlState::code);
        if code != probe.sqlstate {
            return Err(format!(
                "{label}: `{}` was refused under {code} rather than {}: {e}",
                probe.sql, probe.sqlstate
            ));
        }
    }
    Ok(())
}

/// Restore `dump_path` into `config` and assert every compared table matches the
/// original `expected` rows (one entry per [`COMPARED_SELECTS`] query, same order),
/// then that the restored catalog still refuses every [`IDENTITY_REFUSALS`] statement.
async fn restore_and_compare(
    label: &str,
    config: &DbConfig,
    dump_path: &PathBuf,
    expected: &[Vec<Vec<Option<String>>>],
    session_zone: Option<&str>,
) -> Result<(), String> {
    restore_via_psql(config, dump_path, session_zone).map_err(|e| format!("{label}: {e}"))?;
    let client = connect(config)
        .await
        .map_err(|e| format!("connect to {label} failed: {e}"))?;

    // Pin the read session to UTC so the `timestamptz` column compares byte-for-byte:
    // pglet always renders UTC `+00`, while Postgres otherwise renders in the
    // server's local TimeZone. pglet is always UTC, and accepts a `SET` to the zone it has.
    client
        .simple_query("SET TimeZone='UTC'")
        .await
        .map_err(|e| format!("{label}: set timezone failed: {e}"))?;

    for (select, want) in COMPARED_SELECTS.iter().zip(expected) {
        let got = query_rows(&client, select)
            .await
            .map_err(|e| format!("{label}: {e}"))?;
        if got != *want {
            return Err(format!(
                "{label}: rows differ for `{select}`:\n  expected: {want:?}\n  got:      {got:?}"
            ));
        }
    }
    // After the rows, because these are refusals of statements that would otherwise
    // mutate what the rows read.
    assert_backend_refuses_identity_violations(label, &client).await
}

/// Every `nextval` default the script emits — in **either** SQL string grammar,
/// `nextval('x'::regclass)` and the escape-string `nextval(E'x'::regclass)` — must name
/// a sequence the same script creates, by a `CREATE SEQUENCE` **or** by an identity
/// column's `ADD GENERATED … (SEQUENCE NAME x …)`, where *names* the literal is read the
/// way a `regclass` literal is actually read and not as the bytes between the quotes.
///
/// Naming both grammars is not pedantry: `quote_string_literal` emits `E'…'` for any
/// name containing a backslash, and a scan for `nextval('` never enters its loop body
/// for one — so the check **passed vacuously** over that whole family, which is exactly
/// the family a backslash name reaches. A check that reports green by not looking is
/// worse than no check, because it also reports green on the orphan.
///
/// That last clause is doing real work. A name reaches the script in two different
/// grammars: an identifier position (`quote_ident`) and a string literal inside a
/// `regclass` cast (`SplitIdentifierString`, which downcases an unquoted part and
/// rejects interior whitespace). Comparing the literal's raw content against the
/// identifier's is comparing two spellings without reading either, and it matches
/// `CREATE SEQUENCE "Odd_id_seq"` to `nextval('Odd_id_seq')` — two *different*
/// relations, one of which no statement creates.
///
/// Reading both creation forms is not a widening of scope but the price of keeping
/// the one it had. An identity column emits neither a `CREATE SEQUENCE` nor a
/// `nextval` of its own, so on its own it is simply outside this check; what pulled it
/// back in is that an *ordinary* column may draw from an identity sequence (a sibling
/// of the same table, or a column of another table), and that default is emitted.
/// Reading only `CREATE SEQUENCE` would report those as orphans — the false positive
/// this check cannot distinguish from its finding.
///
/// **The narrow edge of its coverage**, stated plainly because an identity column
/// emits no `nextval` and so sits outside this check by default: an identity column
/// whose own sequence is missing from the catalog *is* caught, but only because the
/// emitter declines to render an identity whose sequence it cannot find and such a
/// column falls back to the ordinary hoisted default. That is a property of one
/// branch in the emitter rather than of the rendering as a whole. An
/// identity column whose sequence *is* present but whose ownership edge points
/// elsewhere is invisible here by construction, and deliberately so: the emitter
/// re-records the edge rather than reporting it.
///
/// The restore already proves this for anything it *reaches* — but only for a catalog
/// this corpus builds, and the corpus is where the hole was. A column default naming a
/// sequence the catalog lacks rendered exactly like an ordinary one: the script
/// carried the `DEFAULT nextval(…)` with no `CREATE SEQUENCE` for it, stock Postgres
/// refused the restore, and pglet's own restore accepted it and silently renumbered
/// the column from 1. Nothing here looked for that, because no seed produced it —
/// the *other* orphan direction (a sequence outliving its owner) had seeds and
/// assertions, and its symmetric twin had neither.
///
/// **It is not discriminating today, and that limits its worth.**
/// Every route into the state is now refused (`ALTER SEQUENCE … OWNED BY` and
/// `ALTER COLUMN … SET/DROP DEFAULT` on an identity column), so no catalog this
/// engine's own DDL can build reaches it — the same standing as
/// `a_missing_sequence_is_loud_rather_than_minted_at_one` in `executor::dml`, and kept
/// for the same reason: it converts the next removal route, or a corner of an existing
/// one, from a silent renumbering into a named failure. A store written by a build
/// *between* the identity feature and those gates can still hold the state (the magic
/// did not move), and this is the check that would name it.
///
/// **The `setval` half of the same property is structurally unfireable, and is
/// deliberately not checked** — recorded because reading only the `nextval` literals here
/// invites filing the `setval` ones as an open gap, and they are not one. The emitter's
/// sequence-position block walks `schema.sequences` whole; its `CREATE SEQUENCE` block
/// walks the same key set minus `identity_sequences`; and `identity_sequences` is built
/// by `filter_map(identity_spec)`, whose last field is `schema.sequences.get(sequence)?`
/// — so that set is a *subset* of the same keys. Every name a `setval` is emitted for is
/// therefore created by one of the two creating statements, for every catalog, and a
/// `setval` arm could never fire. What is genuinely at risk on that side is the
/// *spelling* — the identifier and literal grammars naming different relations — and it
/// is covered twice over: hermetically by
/// `dump::tests::a_name_needing_quotes_carries_its_quoting_inside_both_literals`, and
/// loudly by the restore itself.
///
/// **One known false positive, recorded rather than closed.** The scan anchors on the
/// substring `nextval(` anywhere in a statement line, *including inside a string value*,
/// so an ordinary `DEFAULT 'nextval(''x'')'` (or a CHECK against such a text) reduces to
/// the content `""`, which is no name, and this check fails loud over a correct dump.
/// Reachable from ordinary DDL, not a regression — the `nextval('` anchor it replaced had
/// it too — and absent from the corpus. The loud non-literal-argument arm below extends
/// the same surface by one shape (a text value spelling `nextval(` with a non-literal
/// argument, previously skipped). Closing it means tokenizing each statement line's SQL
/// string literals so the anchor is sought only outside them, and that trade is refused
/// on **direction**: this failure is loud and names itself, whereas a bug in a
/// hand-rolled string tokenizer would be silent, and a repair that can mask a finding is
/// worse than a false positive a reader can see in the message.
fn check_every_nextval_has_a_sequence(dump: &str) -> Result<(), String> {
    // The two sides spell a name differently, so both are normalized: `CREATE SEQUENCE`
    // emits it through `quote_ident` (double-quoted, internal `"` doubled, when it is
    // not a safe bare identifier) while the default emits it as a string literal
    // (single-quoted, internal `'` doubled). Comparing the raw tokens would fail
    // *spuriously* on the first sequence name needing either — a fail-loud check whose
    // false positive is indistinguishable from the real finding.
    // The identifier side has the literal side's problem, so it is read the same way: a
    // double-quoted name ends at the first `"` that is not the opening half of a doubled
    // `""`. Reading it by whitespace instead cuts `"a b"` — reachable from the engine's
    // own accepted DDL (`CREATE SEQUENCE "a b"`, measured through
    // `pglet_server dump`) — at `"a`, and the check then failed loud naming a sequence
    // the dump does create: the false positive it cannot distinguish from a finding.
    // A `;` *inside* a quoted name is not that shape, measured — the closing `"` shields
    // it from a trailing-`;` trim. `None` = unterminated.
    let read_ident = |rest: &str| {
        let Some(quoted) = rest.strip_prefix('"') else {
            // A bare identifier ends at whitespace, the statement's `;`, or the `)`
            // that closes an identity option list — `(SEQUENCE NAME s)` is the whole
            // list when the sequence starts at the default `1`, so without `)` the
            // name reads as `s)` and the check fails loud over a correct dump.
            let end = rest
                .find(|c: char| c.is_whitespace() || c == ';' || c == ')')
                .unwrap_or(rest.len());
            return Some(rest[..end].to_string());
        };
        let mut chars = quoted.chars().peekable();
        let mut name = String::new();
        while let Some(c) = chars.next() {
            if c != '"' {
                name.push(c);
                continue;
            }
            if chars.peek() == Some(&'"') {
                chars.next();
                name.push('"');
                continue;
            }
            return Some(name);
        }
        None
    };
    // A single-quoted literal ends at the first `'` that is *not* the opening half of a
    // doubled `''`, so the terminator cannot be found by scanning for the first quote:
    // on `a''b'::regclass)` that lands at index 1 and yields `a`, and unfolding `''`
    // afterwards then runs over a string that can no longer contain one. The only input
    // the doubling exists for was the one input the check could not read, and it failed
    // loud naming a sequence the dump never mentions.
    //
    // `escaped` selects the **second** grammar the emitter reaches: `E'…'`, which
    // `quote_string_literal` produces for any name containing a backslash (the one byte
    // whose meaning depends on `standard_conforming_strings`). There a `\` introduces an
    // escape, and the emitter produces exactly two — `\\` for a backslash and `''` for a
    // quote, since it doubles quotes in both grammars. Any *other* escape is one this
    // reader does not model, and it declines rather than guessing: a mis-decoded name
    // silently becomes either a spurious orphan report or a missed one, and both are
    // failures this check exists to prevent. `None` = unterminated or un-decodable.
    let unquote_literal = |rest: &str, escaped: bool| {
        let mut chars = rest.chars().peekable();
        let mut name = String::new();
        while let Some(c) = chars.next() {
            if escaped && c == '\\' {
                match chars.next() {
                    Some('\\') => name.push('\\'),
                    Some('\'') => name.push('\''),
                    _ => return None,
                }
                continue;
            }
            if c != '\'' {
                name.push(c);
                continue;
            }
            if chars.peek() == Some(&'\'') {
                chars.next();
                name.push('\'');
                continue;
            }
            return Some(name);
        }
        None
    };
    // Unfolding the SQL string grammar yields the literal's *content*, which is a name
    // in a **third** grammar — Postgres's `SplitIdentifierString`, the reader behind
    // `'…'::regclass`. It is neither of the two above: a part beginning `"` runs to the
    // first undoubled `"`, an unquoted part runs to a `.` or whitespace and is
    // **downcased**, and a second unquoted word or an unterminated `"` is no name at
    // all (`invalid name syntax`).
    //
    // Reading the content verbatim instead is what made this check blind to the defect
    // it is best placed to see. `CREATE SEQUENCE "Odd_id_seq"` beside
    // `nextval('Odd_id_seq'::regclass)` is two spellings of two *different* relations —
    // the literal names `odd_id_seq` — and a verbatim comparison matches them, reports
    // green, and the restore then loses the column's default and the sequence's counter
    // on stock Postgres while pglet's own restore accepts the script whole.
    //
    // This is now the **same** rule both engines apply — pglet folds an unquoted
    // identifier and an unquoted name-literal part alike, so the two readers agree by
    // construction rather than by this check being the stricter of them. It is kept as
    // an independent restatement: a regression that stopped folding one of pglet's two
    // name positions would make the emitted script name two different relations, and
    // this is the side that refuses. The downcase is a no-op on every *correct*
    // emission — `quote_ident` leaves a name bare only when it already matches
    // `^[a-z_][a-z0-9_]*$` — so it costs no false positive and is the whole
    // discriminator. `None` = not a well-formed name.
    // Both arms open `let mut name = String::new()`; they stay apart for the same reason
    // `split_name_literal` keeps them apart — quoted-verbatim vs unquoted-downcased is the
    // rule under test, not an incidental shared binding.
    #[allow(clippy::branches_sharing_code)]
    let read_name_literal = |content: &str| {
        let mut chars = content.chars().peekable();
        let is_space = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c');
        while chars.peek().copied().is_some_and(is_space) {
            chars.next();
        }
        chars.peek()?;
        let name = if chars.next_if_eq(&'"').is_some() {
            let mut name = String::new();
            loop {
                let c = chars.next()?; // mismatched quotes
                if c != '"' {
                    name.push(c);
                } else if chars.next_if_eq(&'"').is_some() {
                    name.push('"');
                } else {
                    break;
                }
            }
            name
        } else {
            let mut name = String::new();
            while let Some(&c) = chars.peek() {
                if c == '.' || is_space(c) {
                    break;
                }
                name.push(c);
                chars.next();
            }
            if name.is_empty() {
                return None;
            }
            // ASCII-width, the same fold `split_name_literal` applies. A Unicode
            // `to_lowercase()` here is a *different* rule from the one this claims to
            // restate, and reduces a non-ASCII-uppercase name to a spelling the engine
            // never produces.
            name.to_ascii_lowercase()
        };
        while chars.peek().copied().is_some_and(is_space) {
            chars.next();
        }
        // The emitter emits unqualified names only, so anything after the first name —
        // a `.` qualifier included — is itself the finding rather than something to
        // reduce away.
        if chars.next().is_some() {
            return None;
        }
        Some(name)
    };
    // Statement lines only, for **both** passes below. A `COPY … FROM stdin` block's
    // rows are *data*, and the two passes read a data line wrongly in opposite
    // directions: a value containing `nextval('` reads as a column default naming a
    // sequence nothing creates (a spurious failure), while a value beginning
    // `CREATE SEQUENCE ` registers a name nothing declares — which *masks* a real
    // finding, the direction a fail-loud check cannot report.
    let statement_lines = || {
        let mut in_copy_data = false;
        dump.lines().filter(move |line| {
            if in_copy_data {
                in_copy_data = line.trim_end() != "\\.";
                return false;
            }
            if line.trim_start().starts_with("COPY ") && line.trim_end().ends_with("FROM stdin;") {
                in_copy_data = true;
                return false;
            }
            true
        })
    };
    // Two statements create a sequence, and reading only the first is how this check
    // went blind to identity columns: an identity sequence has **no** `CREATE
    // SEQUENCE` at all — its `ADD GENERATED … (SEQUENCE NAME <n> …)` creates it — so a
    // hoisted default drawing from one (an ordinary catalog: a sibling column, or a
    // column of another table) would be reported as an orphan the dump never creates.
    let mut created: Vec<String> = Vec::new();
    for line in statement_lines() {
        let trimmed = line.trim();
        let rest = trimmed.strip_prefix("CREATE SEQUENCE ").or_else(|| {
            trimmed
                .find("AS IDENTITY (SEQUENCE NAME ")
                .map(|at| &trimmed[at + "AS IDENTITY (SEQUENCE NAME ".len()..])
        });
        let Some(rest) = rest else { continue };
        let Some(name) = read_ident(rest) else {
            return Err(format!(
                "unterminated sequence identifier in the dump:\n{dump}"
            ));
        };
        created.push(name);
    }
    for line in statement_lines() {
        // Both string grammars, found from the one `nextval(` anchor. Scanning for
        // `nextval('` instead misses `nextval(E'` entirely — the loop body never runs,
        // so the check passes over a backslash-named sequence whether or not the dump
        // creates it.
        //
        // The anchor is a bare literal, so it is case- and whitespace-sensitive: an
        // emitter mutation rendering `NEXTVAL('…')` or `nextval ('…')` reads as no
        // nextval at all and this check reports **green** — the masking direction. It is
        // asymmetric with the `created` side above, whose prefixes are equally literal
        // but whose drift *empties* `created` and therefore fails loud on the first
        // default it can no longer match. Disclosed rather than closed; widening the
        // anchor is its own decision.
        for (offset, _) in line.match_indices("nextval(") {
            let after = &line[offset + "nextval(".len()..];
            let (rest, escaped) = if let Some(rest) = after.strip_prefix("E'") {
                (rest, true)
            } else if let Some(rest) = after.strip_prefix('\'') {
                (rest, false)
            } else {
                // An argument that is not a string literal is the **same class of
                // unknown** as the unmodelled escape below — a `nextval` this reader
                // cannot resolve to a name — and it takes the same answer: decline
                // loudly. Skipping it silently was the masking direction, and its guard
                // ("the emitter produces no such spelling") is a claim about an
                // enumeration rather than a property. The claim holds today
                // (`render_default`'s `NextVal` arm is the only producer and always goes
                // through `quote_name_literal`), but it is exactly the mutation this
                // check exists to survive that breaks it: a `quote_name_literal` that
                // stops quoting emits `nextval(Odd_a_seq::regclass)`, which the skip
                // reads as no `nextval` at all and reports **green** over a dump whose
                // restore loses every default and every counter.
                return Err(format!(
                    "the dump defaults a column from a nextval() whose argument is not a \
                     string literal, so this check cannot resolve it to a sequence name — \
                     every one the emitter renders goes through `quote_name_literal`, so \
                     this is a finding about the emitter rather than about the \
                     catalog:\n{dump}"
                ));
            };
            let Some(content) = unquote_literal(rest, escaped) else {
                return Err(format!(
                    "unterminated or un-decodable nextval() literal in the dump:\n{dump}"
                ));
            };
            let Some(name) = read_name_literal(&content) else {
                return Err(format!(
                    "the dump defaults a column from nextval('{content}'), which is not \
                     a well-formed relation name — Postgres reads a regclass literal \
                     with SplitIdentifierString and answers `invalid name syntax`, so \
                     the restore loses the default:\n{dump}"
                ));
            };
            if !created.contains(&name) {
                return Err(format!(
                    "the dump defaults a column from nextval('{name}') but never creates \
                     that sequence — stock Postgres refuses the restore and pglet's own \
                     restore renumbers from 1:\n{dump}"
                ));
            }
        }
    }
    Ok(())
}

/// The full round-trip: build the source, dump it, restore into Postgres and into
/// the spawned pglet server, and compare both against the original pglet rows.
async fn run_roundtrip(pg: &Client, pglet_config: &DbConfig) -> Result<(), String> {
    let mut source = build_source();
    let dump = source
        .dump_sql()
        .map_err(|e| format!("dump_sql failed: {e}"))?;

    // The original pglet data, rendered to the same text grid the wire returns.
    let expected: Vec<Vec<Vec<Option<String>>>> = COMPARED_SELECTS
        .iter()
        .map(|select| render_embed(&embed_query(&mut source, select)))
        .collect();
    let row_counts: Vec<usize> = expected.iter().map(Vec::len).collect();
    if row_counts
        != vec![
            2, 2, 3, 1, 1, 2, 2, 2, 2, 2, 1, 1, 1, 1, 2, 2, 1, 1, 1, 2, 1, 1, 2, 1, 9, 9, 3, 2, 1,
            2, 1, 2, 13, 5, 4, 6, 1, 1, 1, 1, 1, 9, 1,
        ]
    {
        return Err(format!(
            "source did not populate the expected rows: {row_counts:?}"
        ));
    }
    // The counters must actually be the below-1 ones the `MINVALUE` guard exists for;
    // a comparison of two values that are both `1` on every build would prove nothing.
    if expected[3] != vec![vec![Some("0".to_string()), Some("-4".to_string())]] {
        return Err(format!(
            "the low-counter probe did not read the seeded below-1 values: {:?}",
            expected[3]
        ));
    }
    // The restart probe must land on the *start* (1) and not on the minimum the
    // emitted `MINVALUE` declares (−4) — otherwise the two are indistinguishable and
    // the probe proves nothing about the restored `START WITH`.
    if expected[4] != vec![vec![Some("1".to_string())]] {
        return Err(format!(
            "the restart probe did not read the seeded start: {:?}",
            expected[4]
        ));
    }
    // The identity counters and starts must be the seeded ones, and every value must
    // be clear of `1`: two probes that read `1` on the fixed build and `1` on a build
    // that lost the `setval` or the `START WITH` are indistinguishable, which is how a
    // corpus can hold the right shape and still witness nothing.
    let want_counters = ["4", "102", "504", "2", "304"];
    let want_starts = ["100", "500", "0"];
    for (index, want) in [(11_usize, &want_counters[..]), (12, &want_starts[..])] {
        let got: Vec<Option<String>> = want.iter().map(|v| Some((*v).to_string())).collect();
        if expected[index] != vec![got] {
            return Err(format!(
                "the identity probe at {index} did not read the seeded values \
                 (want {want:?}): {:?}",
                expected[index]
            ));
        }
    }
    if expected[13] != vec![vec![Some("555".to_string()), Some("explicit".to_string())]] {
        return Err(format!(
            "the BY DEFAULT accept polarity did not take the explicit id: {:?}",
            expected[13]
        ));
    }
    // The quoted-name probes must read the seeded values and not `1`. `borrowed` is
    // the `nextval` literal's observable and the counters are the `setval` literal's;
    // a build whose literal named a different relation answers `1` (pglet, from a
    // freshly minted sequence) or nothing at all (Postgres, from a column with no
    // default) rather than these.
    if expected[16] != vec![vec![Some("702".to_string()), Some("a".to_string())]] {
        return Err(format!(
            "the spaced-name nextval default did not draw from its sequence: {:?}",
            expected[16]
        ));
    }
    if expected[17] != vec![vec![Some("60".to_string()), Some("x".to_string())]] {
        return Err(format!(
            "the doubled-quote nextval default did not draw from its sequence: {:?}",
            expected[17]
        ));
    }
    if expected[18]
        != vec![vec![
            Some("3".to_string()),
            Some("703".to_string()),
            Some("61".to_string()),
        ]]
    {
        return Err(format!(
            "the quoted-name counters did not read the seeded resume points: {:?}",
            expected[18]
        ));
    }

    check_every_nextval_has_a_sequence(&dump)?;
    assert_source_refuses_identity_violations(&mut source)?;

    let dump_path = std::env::temp_dir().join(format!("pglet_dump_{}.sql", pglet_config.port));
    std::fs::write(&dump_path, &dump).map_err(|e| format!("write dump file: {e}"))?;

    // Both legs run **unconditionally**, and every failure is reported. The Postgres
    // leg used to short-circuit, which made the pglet leg unobservable whenever a
    // change reddened Postgres — so "does this break the self round-trip too?" could
    // not be answered without first editing this harness. That is the wrong place for
    // the question to live: it has to be answerable for every change, and an
    // unmeasurable leg is worse than a missing one, because it reads as covered.
    // The two legs are also not redundant — the Postgres leg is what makes the dump
    // *differential*, the pglet leg is what makes it a *round-trip*, and a defect
    // visible on only one is the common case rather than the exception.
    //
    // 1. Portability to Postgres (reset `public`, restore, compare).
    reset_schema(pg).await?;
    let pg_outcome = restore_and_compare(
        "Postgres",
        &POSTGRES_CONFIG,
        &dump_path,
        &expected,
        Some("America/New_York"),
    )
    .await;
    // Always clean the shared Postgres instance, regardless of the comparison — and
    // before the pglet leg runs, so a failure there cannot leave the shared instance
    // dirty for the next binary.
    reset_schema(pg).await.ok();

    // 2. Self round-trip into the freshly-spawned (empty) pglet server.
    let pglet_outcome =
        restore_and_compare("pglet", pglet_config, &dump_path, &expected, None).await;

    let failures: Vec<String> = [pg_outcome, pglet_outcome]
        .into_iter()
        .filter_map(Result::err)
        .collect();
    if !failures.is_empty() {
        // The dump file is deliberately **kept** when a leg failed — it is the whole
        // input to diagnosing which statement the restore choked on.
        return Err(format!(
            "{} of 2 restore legs failed:\n{}",
            failures.len(),
            failures.join("\n")
        ));
    }

    std::fs::remove_file(&dump_path).ok();
    Ok(())
}

#[tokio::main]
async fn main() {
    // Postgres is the differential ground truth; the psql client drives the
    // restore. Degrade gracefully (skip, exit 0) when either is missing.
    if let Err(e) = pglet_tests::ensure_postgres().await {
        eprintln!("[dump] {e}");
        eprintln!("[dump] skipping round-trip (Postgres unavailable)");
        return;
    }
    if !has_binary("psql") {
        eprintln!("[dump] skipping round-trip (psql client not on PATH)");
        return;
    }

    eprintln!("[dump] Building pglet_server...");
    if !build_server() {
        eprintln!("[dump] build failed");
        std::process::exit(1);
    }
    let port = find_open_port(PGLET_TEST_PORT..DYNAMIC_PORT_END);
    eprintln!("[dump] Spawning pglet_server on port {port} (in-memory)...");
    let server = spawn_server(port, None);
    if !wait_for_ready(port).await {
        eprintln!("[dump] server failed to start");
        hard_kill(server);
        std::process::exit(1);
    }

    let pglet_config = pglet_config(port);
    let pg = connect(&POSTGRES_CONFIG)
        .await
        .expect("connect to Postgres");

    println!("\n=== dump_sql Round-Trip ===\n");
    let outcome = run_roundtrip(&pg, &pglet_config).await;

    eprintln!("[dump] Shutting down server...");
    hard_kill(server);

    match outcome {
        Ok(()) => {
            println!("\x1b[32m✓\x1b[0m dump_sql portability (Postgres) + self round-trip (pglet)");
            println!("\n=== Results ===\nPassed: 1\nFailed: 0");
        }
        Err(e) => {
            println!("\x1b[31m✗\x1b[0m dump_sql round-trip: {e}");
            println!("\n=== Results ===\nPassed: 0\nFailed: 1");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::check_every_nextval_has_a_sequence;

    /// The real rendering, measured: `pglet_server dump --data-dir` over a database
    /// holding `CREATE SEQUENCE "a'b" START WITH 3` and a column defaulting from it.
    /// The one name is spelled four ways — `a'b` as the name itself, `"a'b"` as an
    /// identifier, `"a''b"` once the identifier is put inside a string literal, and
    /// `'"a''b"'` as that literal in the script.
    const QUOTED_NAME_DUMP: &str = "CREATE SEQUENCE \"a'b\" START WITH 3;\n\
         \n\
         CREATE TABLE dq (\n\
         \x20   id integer,\n\
         \x20   v integer\n\
         );\n\
         \n\
         ALTER TABLE dq ALTER COLUMN id SET DEFAULT nextval('\"a''b\"'::regclass);\n\
         \n\
         COPY dq (id, v) FROM stdin;\n\
         3\t7\n\
         \\.\n\
         \n\
         SELECT setval('\"a''b\"', 4, false);\n";

    /// A name needing the literal's `''` escape must read back whole. Scanning for the
    /// first `'` cuts it at the opening half of the pair and yields `a`, so the check
    /// failed loud over a dump that is entirely correct — a false positive
    /// indistinguishable from the finding it exists to report.
    #[test]
    fn a_doubled_quote_in_a_nextval_literal_is_read_whole() {
        assert_eq!(check_every_nextval_has_a_sequence(QUOTED_NAME_DUMP), Ok(()));
    }

    /// The other quoting form — a shape a whitespace split happened to read correctly,
    /// kept so a repair of either side cannot break it.
    #[test]
    fn a_doubled_double_quote_in_a_create_sequence_ident_is_read_whole() {
        let dump = "CREATE SEQUENCE \"a\"\"b\" START WITH 3;\n\
             CREATE TABLE dq (id integer);\n\
             ALTER TABLE dq ALTER COLUMN id SET DEFAULT nextval('\"a\"\"b\"'::regclass);\n";
        assert_eq!(check_every_nextval_has_a_sequence(dump), Ok(()));
    }

    /// A quoted sequence name containing **whitespace**, rendered by
    /// `pglet_server dump` over `CREATE SEQUENCE "a b" START WITH 3` — reachable from
    /// the engine's own accepted DDL (measured). A whitespace split reads `"a` and the
    /// check fails loud over a dump that is entirely correct.
    #[test]
    fn a_whitespace_in_a_create_sequence_ident_is_read_whole() {
        let dump = "CREATE SEQUENCE \"a b\" START WITH 3;\n\
             CREATE TABLE dqw (id integer);\n\
             ALTER TABLE dqw ALTER COLUMN id SET DEFAULT nextval('\"a b\"'::regclass);\n";
        assert_eq!(check_every_nextval_has_a_sequence(dump), Ok(()));
    }

    /// The **downcasing** half of the defect this check is best placed to see, and the
    /// silent one: `nextval('Odd_id_seq')` is well-formed SQL that names
    /// `odd_id_seq`, which no statement creates. A verbatim comparison of the two
    /// spellings matches them and reports green, and the restore then leaves stock
    /// Postgres with the table, every row, no default, and a counter at `1`.
    #[test]
    fn a_bare_mixed_case_nextval_literal_does_not_match_its_quoted_sequence() {
        let dump = "CREATE SEQUENCE \"Odd_id_seq\";\n\
             CREATE TABLE dq (id integer);\n\
             ALTER TABLE dq ALTER COLUMN id SET DEFAULT nextval('Odd_id_seq'::regclass);\n";
        let err = check_every_nextval_has_a_sequence(dump)
            .expect_err("the literal names odd_id_seq, which nothing creates");
        assert!(err.contains("nextval('odd_id_seq')"), "reported: {err}");
    }

    /// The **`invalid name syntax`** half, which is loud on Postgres rather than
    /// silent — but loud *at the restore*, which is exactly where this check exists to
    /// pre-empt it. An unquoted name with interior whitespace is no name at all.
    #[test]
    fn a_bare_spaced_nextval_literal_is_reported_as_malformed() {
        let dump = "CREATE SEQUENCE \"a b\";\n\
             CREATE TABLE dq (id integer);\n\
             ALTER TABLE dq ALTER COLUMN id SET DEFAULT nextval('a b'::regclass);\n";
        let err = check_every_nextval_has_a_sequence(dump)
            .expect_err("'a b' is not a well-formed relation name");
        assert!(
            err.contains("not a well-formed relation name"),
            "reported: {err}"
        );
        assert!(err.contains("nextval('a b')"), "reported: {err}");
    }

    /// A quoted name containing a **`;`**, on the sequence whose `START WITH 1` leaves
    /// the statement with no trailing clause — so the `;` inside the name sits right
    /// against the statement terminator. **Vacuous against the whitespace-split
    /// scanner**, measured: its `trim_end_matches(';')` stops at the closing `"`, so it
    /// read this correctly too. Kept as a regression pin, not a discriminator.
    #[test]
    fn a_semicolon_in_a_create_sequence_ident_is_read_whole() {
        let dump = "CREATE SEQUENCE \"ab;\";\n\
             CREATE TABLE dqs (id integer);\n\
             ALTER TABLE dqs ALTER COLUMN id SET DEFAULT nextval('\"ab;\"'::regclass);\n";
        assert_eq!(check_every_nextval_has_a_sequence(dump), Ok(()));
    }

    /// The reject polarity over both new shapes: a *different* name in the default must
    /// still fail loud, or the repair could have been "read nothing and match anything".
    #[test]
    fn an_orphan_default_beside_an_awkwardly_named_sequence_is_still_caught() {
        let dump = "CREATE SEQUENCE \"a b\" START WITH 3;\n\
             CREATE SEQUENCE \"ab;\";\n\
             CREATE TABLE dq (id integer);\n\
             ALTER TABLE dq ALTER COLUMN id SET DEFAULT nextval('\"a c\"'::regclass);\n";
        let err = check_every_nextval_has_a_sequence(dump).expect_err("no sequence `a c`");
        assert!(err.contains("nextval('a c')"), "reported: {err}");
    }

    /// An unterminated identifier is loud rather than silently reading to end-of-line —
    /// the identifier side's twin of `an_unterminated_nextval_literal_is_loud`.
    #[test]
    fn an_unterminated_create_sequence_ident_is_loud() {
        let dump = "CREATE SEQUENCE \"ghost\n";
        let err =
            check_every_nextval_has_a_sequence(dump).expect_err("the identifier never closes");
        assert!(err.contains("unterminated"), "reported: {err}");
    }

    /// The reject polarity over the same quoting, and the cell that separates "reads
    /// the name whole" from "errors either way": both the old and the fixed scanner
    /// return `Err` here, and only the fixed one *names the sequence the dump asked
    /// for*. A message naming `a` sends a reader looking for a sequence no statement
    /// mentions.
    #[test]
    fn a_missing_quoted_sequence_is_reported_under_its_whole_name() {
        let dump = "CREATE TABLE dq (id integer);\n\
             ALTER TABLE dq ALTER COLUMN id SET DEFAULT nextval('\"a''b\"'::regclass);\n";
        let err = check_every_nextval_has_a_sequence(dump).expect_err("no sequence is created");
        assert!(err.contains("nextval('a'b')"), "reported: {err}");
    }

    /// The **second SQL string grammar**, and the family this check could not see at
    /// all. `quote_string_literal` emits `E'…'` for any name containing a backslash, so
    /// a scan anchored on `nextval('` never enters its loop body here and the check
    /// passed *vacuously* — over correct dumps and orphaned ones alike. The rendering is
    /// measured, from `dump::tests::a_backslash_in_a_name_forces_the_escape_string_form_\
    /// in_both_literals`: the identifier position takes the backslash raw and the string
    /// position doubles it.
    #[test]
    fn a_backslash_named_sequence_is_read_in_the_escape_string_grammar() {
        let dump = "CREATE SEQUENCE \"a\\b\" START WITH 6;\n\
             CREATE TABLE bs (id integer);\n\
             ALTER TABLE bs ALTER COLUMN id SET DEFAULT nextval(E'\"a\\\\b\"'::regclass);\n";
        assert_eq!(check_every_nextval_has_a_sequence(dump), Ok(()));
    }

    /// The reject polarity of the same grammar, and the cell that separates a real
    /// reading from the vacuous pass: with the scan anchored on `nextval('` this dump —
    /// a default naming a sequence nothing creates — is reported **green**.
    #[test]
    fn an_orphan_default_in_the_escape_string_grammar_is_caught() {
        let dump = "CREATE SEQUENCE \"a\\b\" START WITH 6;\n\
             CREATE TABLE bs (id integer);\n\
             ALTER TABLE bs ALTER COLUMN id SET DEFAULT nextval(E'\"c\\\\d\"'::regclass);\n";
        let err = check_every_nextval_has_a_sequence(dump).expect_err("no sequence `c\\d`");
        assert!(err.contains("nextval('c\\d')"), "reported: {err}");
    }

    /// The downcasing half, carried into the escape grammar: the content is decoded
    /// first and then read as a name, so an *unquoted* part downcases there exactly as
    /// it does in the plain grammar. A reader that decoded but skipped
    /// `SplitIdentifierString` would match this pair and report green.
    #[test]
    fn a_bare_mixed_case_escape_string_literal_still_downcases() {
        let dump = "CREATE SEQUENCE \"Odd\\b\";\n\
             CREATE TABLE bs (id integer);\n\
             ALTER TABLE bs ALTER COLUMN id SET DEFAULT nextval(E'Odd\\\\b'::regclass);\n";
        let err = check_every_nextval_has_a_sequence(dump)
            .expect_err("the literal names odd\\b, which nothing creates");
        assert!(err.contains("nextval('odd\\b')"), "reported: {err}");
    }

    /// An escape the emitter does not produce is **declined**, not guessed. Decoding
    /// `\n` as a newline (or as a bare `n`) would silently turn a name into a different
    /// one, which is a missed orphan or a spurious one depending on the corpus — the two
    /// failures this check exists to prevent, reintroduced by its own reader.
    #[test]
    fn an_unmodelled_escape_in_a_nextval_literal_is_loud() {
        let dump = "CREATE SEQUENCE plain;\n\
             CREATE TABLE bs (id integer);\n\
             ALTER TABLE bs ALTER COLUMN id SET DEFAULT nextval(E'\"a\\nb\"'::regclass);\n";
        let err = check_every_nextval_has_a_sequence(dump).expect_err("`\\n` is not modelled");
        assert!(err.contains("un-decodable"), "reported: {err}");
    }

    /// An argument that is **not a string literal** is declined on the same policy, and
    /// this is the polarity that had gone the other way: the same class of unknown, read
    /// silently. The dump below is the literal output of the one mutation this check
    /// exists to survive — a `quote_name_literal` that stops quoting — and under a
    /// silent skip it is reported **green**, over a script whose restore leaves stock
    /// Postgres a syntax error and pglet a table with no default and a counter at `1`.
    #[test]
    fn a_nextval_argument_that_is_not_a_string_literal_is_loud() {
        let dump = "CREATE SEQUENCE \"Odd_a_seq\";\n\
             CREATE TABLE bs (id integer);\n\
             ALTER TABLE bs ALTER COLUMN id SET DEFAULT nextval(Odd_a_seq::regclass);\n";
        let err = check_every_nextval_has_a_sequence(dump)
            .expect_err("the argument is a bare identifier, not a literal this reader can read");
        assert!(err.contains("not a string literal"), "reported: {err}");
    }

    /// The same decline over the *quoted-identifier* spelling of that mutation, which
    /// reads as a well-formed name to a human and still is not a literal. Kept beside
    /// the bare form because `quote_ident` produces one or the other by name, so a
    /// corpus holding only safe bare names would exercise just the first.
    #[test]
    fn a_quoted_identifier_nextval_argument_is_loud_too() {
        let dump = "CREATE SEQUENCE \"Odd_a_seq\";\n\
             CREATE TABLE bs (id integer);\n\
             ALTER TABLE bs ALTER COLUMN id SET DEFAULT nextval(\"Odd_a_seq\"::regclass);\n";
        let err = check_every_nextval_has_a_sequence(dump)
            .expect_err("a quoted identifier is not a string literal either");
        assert!(err.contains("not a string literal"), "reported: {err}");
    }

    /// An identity column's sequence has **no `CREATE SEQUENCE`** — its `ADD GENERATED`
    /// creates it — so a column drawing from one (here a sibling of the same table) is
    /// only *not* an orphan if the check reads that statement too.
    #[test]
    fn an_add_generated_creates_the_sequence_it_names() {
        let dump = "CREATE TABLE k (\n\
             \x20   b integer,\n\
             \x20   a integer NOT NULL\n\
             );\n\
             ALTER TABLE k ALTER COLUMN a ADD GENERATED ALWAYS AS IDENTITY \
             (SEQUENCE NAME k_a_seq START WITH 100);\n\
             ALTER TABLE k ALTER COLUMN b SET DEFAULT nextval('k_a_seq'::regclass);\n";
        assert_eq!(check_every_nextval_has_a_sequence(dump), Ok(()));
    }

    /// The same, with the option list holding **only** the name — the shape an identity
    /// at the default `START WITH 1` emits, and the one where the name butts against the
    /// closing paren. A scanner that ends a bare identifier at whitespace or `;` alone
    /// reads `k_a_seq)` here, registers a name nothing references, and then reports the
    /// real reference as an orphan the dump never creates.
    #[test]
    fn an_add_generated_name_ends_at_the_closing_paren() {
        let dump = "ALTER TABLE k ALTER COLUMN a ADD GENERATED ALWAYS AS IDENTITY \
             (SEQUENCE NAME k_a_seq);\n\
             ALTER TABLE k ALTER COLUMN b SET DEFAULT nextval('k_a_seq'::regclass);\n";
        assert_eq!(check_every_nextval_has_a_sequence(dump), Ok(()));
    }

    /// The reject polarity over the same statement: reading `ADD GENERATED` must not
    /// become "register anything and match everything". A default naming a *different*
    /// sequence is still an orphan.
    #[test]
    fn an_orphan_default_beside_an_add_generated_is_still_caught() {
        let dump = "ALTER TABLE k ALTER COLUMN a ADD GENERATED ALWAYS AS IDENTITY \
             (SEQUENCE NAME k_a_seq);\n\
             ALTER TABLE k ALTER COLUMN b SET DEFAULT nextval('other_seq'::regclass);\n";
        let err = check_every_nextval_has_a_sequence(dump).expect_err("no sequence `other_seq`");
        assert!(err.contains("nextval('other_seq')"), "reported: {err}");
    }

    /// The plain case the check was written for still fails loud.
    #[test]
    fn a_missing_plain_sequence_is_still_caught() {
        let dump = "CREATE TABLE dq (\n\
             \x20   id integer DEFAULT nextval('ghost'::regclass)\n\
             );\n";
        assert!(check_every_nextval_has_a_sequence(dump).is_err());
    }

    /// COPY data is not SQL: a value containing `nextval('` must not be read as a
    /// column default (the spurious-failure direction).
    #[test]
    fn copy_data_is_not_read_as_a_column_default() {
        let dump = "CREATE TABLE dq (v text);\n\
             COPY dq (v) FROM stdin;\n\
             nextval('ghost'::regclass)\n\
             \\.\n";
        assert_eq!(check_every_nextval_has_a_sequence(dump), Ok(()));
    }

    /// ...and the *masking* direction, which a fail-loud check can never report: a COPY
    /// value beginning `CREATE SEQUENCE ` must not register a created name, or a real
    /// orphaned default two lines later passes green.
    #[test]
    fn copy_data_cannot_invent_a_created_sequence() {
        let dump = "CREATE TABLE seeds (v text);\n\
             COPY seeds (v) FROM stdin;\n\
             CREATE SEQUENCE ghost;\n\
             \\.\n\
             \n\
             CREATE TABLE dq (\n\
             \x20   id integer DEFAULT nextval('ghost'::regclass)\n\
             );\n";
        let err = check_every_nextval_has_a_sequence(dump)
            .expect_err("only a COPY data line 'creates' ghost");
        assert!(err.contains("nextval('ghost')"), "reported: {err}");
    }

    /// An unterminated literal stays loud rather than silently reading to end-of-line.
    #[test]
    fn an_unterminated_nextval_literal_is_loud() {
        let dump = "CREATE TABLE dq (id integer DEFAULT nextval('ghost\n";
        let err = check_every_nextval_has_a_sequence(dump).expect_err("the literal never closes");
        assert!(err.contains("unterminated"), "reported: {err}");
    }
}
