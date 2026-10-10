//! The storage wrapper's secondary-index registry must not retain a dropped
//! table's index defs, must never evaluate a sibling table's expression index
//! against another table's row, and must come away from a failed index build
//! holding nothing. What a failed *write* inside that build leaves is pages
//! rather than defs, which the registry cannot see, so that guarantee is over
//! the tree band instead: no readable entry there either.
//!
//! The guarantees, over the btree wrapper's registry (`index_defs` /
//! `table_indexes`) — save the last, read off the tree band:
//!
//! - **DROP TABLE sheds the storage index defs** (not just the catalog ones).
//!   `execute_drop_schema` already did this; `execute_drop_table` left them
//!   registered, a slow leak on a long-lived server and a latent stale-def
//!   footgun — a backend that ever reused a table id would then maintain the
//!   sibling's index against the new table's rows.
//! - **Autocommit `ON CONFLICT` + a sibling expression index stays correct.**
//!   fuz_app's `bootstrap_lock` seed (`BOOTSTRAP_LOCK_SEED`, `auth/auth_ddl.ts`:
//!   `INSERT … SELECT … ON CONFLICT DO NOTHING`) must not evaluate `account`'s
//!   `LOWER(email)` / `LOWER(username)` expression index against the
//!   `bootstrap_lock` row (which would fail loud
//!   "lower() requires a text argument, got Bool"). Index maintenance/probe is
//!   scoped per table, so this holds; the test locks it against regression on
//!   both the bare and the `DROP SCHEMA`-reset (the harness idiom) paths.
//! - **A failed index *build* registers nothing.** `Storage::create_index` is
//!   all-or-nothing, so a back-fill that raises — an entry its page cannot hold —
//!   leaves neither an `index_defs` entry nor a `table_indexes` one. The catalog half of that rule is `pglet`'s own
//!   `a_failed_create_index_build_leaves_no_phantom`; two of the guards here are the
//!   storage half, read straight off the registry, and its durable shape over a
//!   file-backed store.
//! - **Nor does a failed *write* inside the back-fill leave a readable entry.** The
//!   registry is one question and the tree is another: `apply_batch` can fail with some
//!   of the batch's pages already written. The third guard injects that I/O error, both
//!   at a page write and at the fsync, and reads the band back.

// A test harness: helpers `panic!` on the wrong backend / a failed statement and `.unwrap()`
// reads so a setup failure surfaces loudly here.
#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use common::{ScratchDir, open_database};
use pglet::{Database, StorageBackend};
use pglet_core::Storage;
use pglet_testing::block_io::MemBlockIo;

/// Number of registered secondary-index defs in the btree wrapper's registry.
fn index_def_count(db: &Database) -> usize {
    let StorageBackend::Btree(wrapper) = db.storage() else {
        panic!("expected a btree backend");
    };
    wrapper.index_defs().len()
}

#[test]
fn drop_table_sheds_storage_index_defs() {
    let mut db = Database::with_btree();
    // PRIMARY KEY auto-creates a unique index; the explicit ones add more — all
    // registered in the wrapper's `index_defs`.
    db.execute("CREATE TABLE t (id INT PRIMARY KEY, k INT, name TEXT)")
        .unwrap();
    db.execute("CREATE INDEX ix_k ON t(k)").unwrap();
    db.execute("CREATE UNIQUE INDEX ix_name_ci ON t (LOWER(name))")
        .unwrap();
    assert!(
        index_def_count(&db) >= 3,
        "expected pkey + ix_k + ix_name_ci registered, got {}",
        index_def_count(&db)
    );

    db.execute("DROP TABLE t").unwrap();
    assert_eq!(
        index_def_count(&db),
        0,
        "DROP TABLE must shed the storage wrapper's index defs (no leak)"
    );
}

/// fuz_app's `bootstrap_lock` seed shape (`auth/auth_ddl.ts`): `account` carries
/// `LOWER(...)` expression indexes, then a sibling `bootstrap_lock` table's `ON CONFLICT DO
/// NOTHING` insert runs in autocommit. The sibling's write must not touch
/// `account`'s indexes.
fn run_auth_bootstrap_shape(db: &mut Database) {
    let stmts = [
        "CREATE TABLE account (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), \
         username TEXT UNIQUE NOT NULL, email TEXT, password_hash TEXT NOT NULL)",
        "CREATE UNIQUE INDEX idx_account_email ON account (LOWER(email)) WHERE email IS NOT NULL",
        "CREATE UNIQUE INDEX idx_account_username_ci ON account (LOWER(username))",
        // Populate so the partial LOWER(email) index actually holds an entry.
        "INSERT INTO account (username, email, password_hash) VALUES ('Alice', 'Alice@X.com', 'h')",
        "CREATE TABLE bootstrap_lock (id INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1), \
         bootstrapped BOOLEAN NOT NULL DEFAULT false)",
        // The shape under test: INSERT … SELECT … ON CONFLICT DO NOTHING, the
        // `bootstrapped` value a Bool from EXISTS — never lowercased.
        "INSERT INTO bootstrap_lock (id, bootstrapped) \
         SELECT 1, EXISTS(SELECT 1 FROM account) ON CONFLICT DO NOTHING",
    ];
    for s in stmts {
        db.execute(s)
            .unwrap_or_else(|e| panic!("autocommit `{}` failed: {e}", &s[..s.len().min(48)]));
    }
    let rows = db
        .execute("SELECT id, bootstrapped FROM bootstrap_lock")
        .unwrap()
        .remove(0)
        .rows;
    assert_eq!(rows.len(), 1, "exactly one bootstrap_lock row");
}

#[test]
fn autocommit_onconflict_sibling_expression_index_clean() {
    run_auth_bootstrap_shape(&mut Database::with_btree());
}

#[test]
fn autocommit_onconflict_sibling_expression_index_clean_after_reset() {
    // The spine harness resets per-test with `DROP SCHEMA public CASCADE; CREATE
    // SCHEMA public`. Re-running the shape after a reset must stay clean.
    let mut db = Database::with_btree();
    run_auth_bootstrap_shape(&mut db);
    db.execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .unwrap();
    assert_eq!(
        index_def_count(&db),
        0,
        "DROP SCHEMA must leave the index registry empty"
    );
    run_auth_bootstrap_shape(&mut db);
}

/// The container-key refusal names its type in **SQL**, not in Rust.
///
/// `executor::ddl`'s `validate_btree_key_type` stands in for no 18.6 error at all —
/// 18.6 has a default B-tree opclass for `jsonb` and for arrays and creates both
/// indexes — so no differential fixture can hold this: an `@expect-error` fixture
/// compares SQLSTATEs against an oracle that succeeds, and the *message* is never
/// compared at all. That is exactly why the `format!("{col_type:?}")` this slot used to
/// hold survived: it shipped `data type Jsonb` and `data type TextArray …` to clients —
/// two spellings from one **producer**.
///
/// The assertion is on the **rendered type name**, not on the sentence, so it does not
/// re-pin the wording `Error::UnindexableType` owns.
#[test]
fn the_container_key_refusal_names_its_type_in_sql() {
    for (column_type, sql_name) in [
        ("JSONB", "jsonb"),
        ("TEXT[]", "text[]"),
        ("INT[]", "integer[]"),
    ] {
        let mut db = Database::with_btree();
        db.execute(&format!("CREATE TABLE ck (id INT, c {column_type})"))
            .unwrap();
        let err = db
            .execute("CREATE INDEX ck_ix ON ck (c)")
            .expect_err("a container key column has no B-tree opclass here");
        let message = err.to_string();
        assert!(
            message.contains(&format!("data type {sql_name} ")),
            "expected the SQL type name in {message:?}"
        );
        // The `TypeOid` variant spellings this slot used to print. `[` cannot be the
        // test — `text[]` is the *correct* array rendering.
        for debug_spelling in ["Jsonb", "TextArray", "Int4Array", "Text[]"] {
            assert!(
                !message.contains(debug_spelling),
                "the Rust Debug spelling {debug_spelling:?} leaked into {message:?}"
            );
        }
    }
}

/// The one container-key shape still refused, and what the refusal costs: nothing.
///
/// A non-unique B-tree over a single bare `jsonb` or array column has nowhere to record
/// its method (`EDGES.md` row 25), so each spelling of it is `0A000` at the DDL — the
/// default method, a declared `btree`, a partial predicate, `IF NOT EXISTS` over a free
/// name — and leaves no index, no storage def, and a table that takes its next write,
/// on both storage backends. The shapes beside it build: a unique one, a composite, an
/// expression key, and the `gin` the refusal's own derivation would have made of it.
#[test]
fn the_refused_bare_container_btree_leaves_the_table_as_it_was() {
    for (column_type, value) in [("JSONB", "'[1,2]'"), ("INT[]", "'{1,2}'")] {
        for (backend, mut db) in [
            ("btree", Database::with_btree()),
            ("memory", Database::in_memory()),
        ] {
            let label = format!("{backend}, {column_type}");
            // The registry is the B+tree wrapper's; the memory backend is read through
            // the catalog and the writes alone.
            let defs = |db: &Database| (backend == "btree").then(|| index_def_count(db));
            db.execute(&format!("CREATE TABLE ck (id INT, c {column_type})"))
                .unwrap();
            db.execute(&format!("INSERT INTO ck VALUES (1, {value})"))
                .unwrap();
            let before = defs(&db);
            for sql in [
                "CREATE INDEX ck_ix ON ck (c)",
                "CREATE INDEX ck_ix ON public.ck USING btree (c)",
                "CREATE INDEX ck_ix ON ck (c) WHERE id > 0",
                "CREATE INDEX IF NOT EXISTS ck_ix ON ck (c)",
                "CREATE INDEX ON ck (c)",
            ] {
                let err = db.execute(sql).expect_err("the bare container B-tree");
                assert_eq!(err.sqlstate(), "0A000", "{label}: {sql}: {err}");
                assert!(db.schema().indexes.is_empty(), "{label}: {sql}");
                assert_eq!(defs(&db), before, "{label}: {sql}");
            }
            db.execute(&format!("INSERT INTO ck VALUES (2, {value})"))
                .unwrap();
            db.execute("UPDATE ck SET id = 3 WHERE id = 2").unwrap();

            // The accept side: every other shape over the same column builds over the
            // rows already there, and the unique one then holds.
            for sql in [
                "CREATE INDEX ck_c ON ck (c, id)",
                "CREATE INDEX ck_e ON ck ((c IS NULL), c)",
                "CREATE INDEX ck_g ON ck USING gin (c)",
                "DELETE FROM ck WHERE id = 3",
                "CREATE UNIQUE INDEX ck_u ON ck (c)",
            ] {
                db.execute(sql)
                    .unwrap_or_else(|e| panic!("{label}: {sql}: {e}"));
            }
            let err = db
                .execute(&format!("INSERT INTO ck VALUES (4, {value})"))
                .expect_err("the unique container key holds");
            assert_eq!(err.sqlstate(), "23505", "{label}: {err}");
        }
    }
}

/// A `jsonb` string whose GIN token is longer than a B+tree page holds as a key: the
/// statement-shaped failure left inside the storage build (`pglet`'s
/// `the_gin_token_refusal_is_the_storage_builds_own` holds that it is the build's).
fn long_jsonb_string() -> String {
    format!("\"{}\"", "x".repeat(5000))
}

/// A failed back-fill leaves the storage registry exactly as it found it.
///
/// The registry is the half no catalog assertion can see, and it is the half that
/// used to survive a `ROLLBACK`: the executor's catalog write was shed by the
/// snapshot restore while `BtreeStorageWrapper::create_index`'s def stood, so the
/// table's writes went on failing against an index `DROP INDEX` answered `42704`
/// for. Read directly off `index_defs()` rather than through SQL, because the
/// symptom SQL shows depends on which write happens to touch an indexable row.
#[test]
fn a_failed_index_build_registers_no_storage_def() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE fb (a INT, f FLOAT8, j JSONB)")
        .unwrap();
    db.execute(&format!(
        "INSERT INTO fb VALUES (1, 1.5, '{}'), (0, 2.5, '{{\"k\":2}}')",
        long_jsonb_string()
    ))
    .unwrap();
    let before = index_def_count(&db);

    for sql in [
        "CREATE INDEX fb_ix ON fb USING gin (j)",
        "CREATE INDEX fb_ix ON fb USING gin (j) WHERE f > 0",
        "CREATE INDEX fb_ix ON fb ((10 / a))",
    ] {
        db.execute(sql).expect_err("the build refuses");
        assert_eq!(
            index_def_count(&db),
            before,
            "{sql} left a def in the storage registry"
        );
    }

    // The same inside a block, where the rollback used to shed the catalog half only.
    // The trigger is the GIN token because that is what reaches the *storage* build: a
    // key expression that raises (`(10 / a)` over the zero) is refused by the
    // executor's entry sizing, ahead of it.
    db.execute("BEGIN").unwrap();
    let err = db
        .execute("CREATE INDEX fb_ix ON fb USING gin (j)")
        .expect_err("the back-fill refuses");
    assert_eq!(err.sqlstate(), "54000", "{err}");
    db.execute("ROLLBACK").unwrap();
    assert_eq!(index_def_count(&db), before);

    // And a build that succeeds still registers, so the assertion above is not
    // vacuously true of every call.
    db.execute("CREATE INDEX fb_ix ON fb (a)").unwrap();
    assert_eq!(index_def_count(&db), before + 1);
}

/// The durable shape: a failed build, a later committed statement, a reopen.
///
/// The phantom was not live-only. The autocommit path rolls back **storage** and
/// leaves `schema` mutated, and the next successful write serializes the whole
/// catalog — so one ordinary statement after the failure persisted the phantom index,
/// and it came back on reopen with the table still unwritable. The controlled half is
/// the `CREATE INDEX` that *succeeds* after the reopen under the same name: a
/// surviving phantom holds that name `42P07` forever.
#[test]
fn a_failed_index_build_leaves_no_phantom_across_a_reopen() {
    let dir = ScratchDir::new("failed_index_build_reopen");

    {
        let mut db = Database::open(&dir.path).expect("open");
        db.execute("CREATE TABLE dz (a INT, j JSONB)").unwrap();
        db.execute(&format!(
            "INSERT INTO dz VALUES (1, '{}')",
            long_jsonb_string()
        ))
        .unwrap();
        // The failure has to come from the **storage** build, the step the catalog
        // write used to precede, and the GIN token is what reaches it: a key
        // expression that raises is refused by the executor's entry sizing first.
        let err = db
            .execute("CREATE INDEX dz_ix ON dz USING gin (j)")
            .expect_err("the back-fill refuses the token");
        assert_eq!(err.sqlstate(), "54000", "{err}");
        // The ordinary statement that persists the catalog, phantom included — and a
        // row a surviving def would refuse.
        db.execute(&format!(
            "INSERT INTO dz VALUES (2, '{}')",
            long_jsonb_string()
        ))
        .unwrap();
    }

    let mut db = Database::open(&dir.path).expect("reopen");
    assert!(
        db.schema().get_index("dz_ix").is_none(),
        "a failed build's index survived the reopen"
    );
    assert_eq!(index_def_count(&db), 0);
    // The table is writable and holds both rows.
    db.execute("INSERT INTO dz VALUES (3, '3.5')").unwrap();
    let rows = db.execute("SELECT a FROM dz").unwrap().remove(0).rows;
    assert_eq!(rows.len(), 3);
    // The name is free.
    db.execute("CREATE INDEX dz_ix ON dz (a)").unwrap();
}

/// A back-fill whose page writes fail part-way leaves no *readable* entry in the band.
///
/// The all-or-nothing contract is a statement about what `create_index` **registers**,
/// and every other guard here reads the registry. This one asks the question the
/// registry cannot answer: the back-fill's last step is a write, and a write can fail
/// with some of the batch's pages already on the device. What that leaves is pages, not
/// entries — `CowBtree::apply_batch` validates every insert, builds the copied pages in
/// a working set, `put`s them, `flush`es, and publishes the new root **once**, after
/// every fallible step — so a reader still descends the pre-batch root, from which the
/// pages the failed batch wrote are unreachable. Measured here for the one band that
/// matters to `create_index`'s contract, not for the tree at large.
///
/// Both halves of that tail are driven, through `MemBlockIo`'s data-region fault modes:
/// a `put` that fails mid-loop (the device's `data-page write failure`) and a `flush`
/// that fails with every page written (its `sync failure`). The error text is asserted,
/// because it is what says the statement failed *there* rather than somewhere harmless
/// earlier; the mid-loop arm additionally proves the batch spanned more than one page,
/// since a one-page batch would have taken its single allowed write and returned `Ok`.
///
/// A refused write fails the store: the statement answers `58030`, every later one
/// `57P02`, and the band is read back after a reopen of the image, which is the state
/// that write left behind. The band is read by giving it a def with nothing else in it.
/// The failed statement wrote no catalog, so the counter and the catalog max are where
/// they were and the next `CREATE INDEX` on the table takes the **same** `index_id` —
/// asserted, since the probe is worthless against a different band. Emptying the table first
/// makes that build's own back-fill contribute nothing, so a row id the range scan
/// returns can only have come from the failed batch. With the delete removed the same
/// scan returns all 2000, which is what says an empty answer means something.
#[test]
fn a_failed_back_fill_write_leaves_no_readable_entry_in_the_band() {
    for (label, fail_at_put_number, wanted) in [
        (
            "put fails mid-loop",
            Some(1_usize),
            "data-page write failure",
        ),
        ("flush fails, every page written", None, "sync failure"),
    ] {
        let io = MemBlockIo::new();
        let policy = io.policy();
        let mut db = open_database(Box::new(io.clone()));

        db.execute("CREATE TABLE ib (a INT)").unwrap();
        // Enough rows that the index band spans more than one page, so "part-way"
        // through the put loop is a real position and not the whole batch.
        for chunk in 0..40 {
            let values: Vec<String> = (0..50).map(|i| format!("({})", chunk * 50 + i)).collect();
            db.execute(&format!("INSERT INTO ib VALUES {}", values.join(",")))
                .unwrap();
        }
        let table_oid = db.schema().get_table("ib").unwrap().oid;
        let expected_index_oid = db.schema().next_oid.max(table_oid + 1);
        let defs_before = index_def_count(&db);

        match fail_at_put_number {
            Some(successes) => policy.arm_fail_data_writes_after(successes),
            None => policy.arm_fail_syncs(),
        }
        let err = db
            .execute("CREATE INDEX ib_ix ON ib (a)")
            .expect_err("the injected I/O error must surface");
        policy.disarm();
        // Loud, and in this arm's own step rather than somewhere else on the path.
        assert_eq!(err.sqlstate(), "58030", "{label}: {err}");
        assert!(
            err.to_string().contains(wanted),
            "{label}: expected a failure at `{wanted}`, got: {err}"
        );

        assert_eq!(index_def_count(&db), defs_before, "{label}");
        assert!(db.schema().get_index("ib_ix").is_none(), "{label}");
        let refused = db
            .execute("SELECT count(*) FROM ib")
            .expect_err("a store that failed a write serves nothing");
        assert_eq!(refused.sqlstate(), "57P02", "{label}: {refused}");
        drop(db);
        let mut db = open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));

        // Empty the table, then build the index for real: same id, empty back-fill.
        db.execute("DELETE FROM ib").unwrap();
        db.execute("CREATE INDEX ib_ix ON ib (a)").unwrap();
        let index_oid = db.schema().get_index("ib_ix").unwrap().oid;
        assert_eq!(
            index_oid, expected_index_oid,
            "{label}: the probe index took a different band than the failed build"
        );

        let StorageBackend::Btree(wrapper) = db.storage() else {
            panic!("expected a btree backend");
        };
        let in_band = wrapper
            .index_range_lookup(u32::try_from(index_oid).unwrap(), None, None, true, None)
            .unwrap();
        assert!(
            in_band.is_empty(),
            "{label}: the failed back-fill left {} readable entries in the band",
            in_band.len()
        );
    }
}

/// A declared constraint's index whose storage build fails leaves no catalog entry.
///
/// No SQL statement fails inside `create_unique_index`'s storage build — every refusal
/// of a constraint's key is the executor's, ahead of it — so the trigger is the device:
/// an injected data-page write failure under the back-fill. The catalog is read on the
/// live handle, before the reopen a failed store needs: that is the state a catalog
/// write ahead of the build would have left behind.
#[test]
fn a_failed_constraint_index_storage_build_writes_no_catalog() {
    for (statement, name) in [
        (
            "ALTER TABLE cb ADD CONSTRAINT cb_a_key UNIQUE (a)",
            "cb_a_key",
        ),
        ("ALTER TABLE cb ADD PRIMARY KEY (a)", "cb_pkey"),
    ] {
        let io = MemBlockIo::new();
        let policy = io.policy();
        let mut db = open_database(Box::new(io.clone()));
        db.execute("CREATE TABLE cb (a INT NOT NULL)").unwrap();
        for chunk in 0..40 {
            let values: Vec<String> = (0..50).map(|i| format!("({})", chunk * 50 + i)).collect();
            db.execute(&format!("INSERT INTO cb VALUES {}", values.join(",")))
                .unwrap();
        }
        let next_oid = db.schema().next_oid;
        let defs_before = index_def_count(&db);

        policy.arm_fail_data_writes_after(1);
        let err = db
            .execute(statement)
            .expect_err("the injected I/O error must surface");
        policy.disarm();
        assert_eq!(err.sqlstate(), "58030", "{statement}: {err}");
        assert!(
            err.to_string().contains("data-page write failure"),
            "{statement}: {err}"
        );

        assert!(db.schema().get_index(name).is_none(), "{statement}");
        assert_eq!(db.schema().next_oid, next_oid, "{statement}");
        assert_eq!(index_def_count(&db), defs_before, "{statement}");
    }
}
