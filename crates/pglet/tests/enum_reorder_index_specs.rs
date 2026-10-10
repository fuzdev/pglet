//! The storage index specs follow `Schema.enums` wherever it moves.
//!
//! `IndexSpec::column_enum_orders` is a **copy** of each column's enum member order —
//! `pglet_storage` holds no catalog to borrow one from — taken at `CREATE INDEX` and on
//! reopen. It was modelled on `IndexSpec::column_types`, whose lifetime is safe for a
//! reason this field does not share: a column's declared type cannot change here
//! (`ALTER COLUMN … SET DATA TYPE` is `0A000`), while an enum's member list can. `DROP
//! TYPE` under a live dependent column is `2BP01` on 18.6 and accepted here (a separate,
//! pre-existing gap), so `DROP TYPE` + `CREATE TYPE` with the members reordered moved
//! what the copy had frozen — and for a **partial unique** index whose predicate orders
//! that enum, the executor's per-statement `PreparedIndex` then decided membership
//! by the live order while storage maintained the entries by the frozen one. A duplicate
//! 18.6 refuses landed with no error.
//!
//! **These cannot be differential fixtures**, and the reason is the harness's, not the
//! engine's: 18.6 refuses to enter the state at all, so there is no oracle answer to
//! compare against — a fixture's `@setup` would fail on the Postgres side and be
//! recorded as the fixture's whole result.
//!
//! Each test opens its **own** database: the state under test is a catalog reorder, and
//! chaining two of them would let the second measure the first's rebuild.

// A test harness: `.unwrap()` on setup and `panic!` on an unexpected value shape, so a
// setup failure surfaces here rather than as a confusing assertion diff.
#![allow(clippy::panic, clippy::unwrap_used)]

use pglet::Database;
use pglet_sql::types::Value;

/// Rows at `k` — the count a unique partial index on `(k)` is supposed to hold to one.
fn count_at_k(db: &mut Database, k: i32) -> i64 {
    let res = db
        .execute(&format!("SELECT count(*) FROM r WHERE k = {k}"))
        .unwrap();
    let rows = &res.last().unwrap().rows;
    match &rows[0][0] {
        Value::Int8(n) => *n,
        other => panic!("expected a count, got {other:?}"),
    }
}

/// [`count_at_k`] over a named table — the two-table shapes need `ra` rather than `r`.
fn count_at_k_in(db: &mut Database, table: &str, k: i32) -> i64 {
    let res = db
        .execute(&format!("SELECT count(*) FROM {table} WHERE k = {k}"))
        .unwrap();
    let rows = &res.last().unwrap().rows;
    match &rows[0][0] {
        Value::Int8(n) => *n,
        other => panic!("expected a count, got {other:?}"),
    }
}

/// The catalog OID of the relation named `name` — `pg_class`, the same join
/// `generate_object_oid`'s counter feeds.
fn relation_oid(db: &mut Database, name: &str) -> i64 {
    let res = db
        .execute(&format!(
            "SELECT oid FROM pg_class WHERE relname = '{name}'"
        ))
        .unwrap();
    let rows = &res.last().unwrap().rows;
    match &rows[0][0] {
        Value::Int4(n) => i64::from(*n),
        Value::Int8(n) => *n,
        other => panic!("expected an oid, got {other:?}"),
    }
}

/// The member labels of every enum in the catalog, in declared order — read the way a
/// client would, so a test whose conclusion rests on the catalog half of a restore can
/// assert that half rather than assume it. `setup` registers one type, so its callers
/// read `mood`'s list and no other's.
fn member_labels(db: &mut Database) -> Vec<String> {
    let res = db
        .execute("SELECT enumlabel FROM pg_enum ORDER BY enumsortorder")
        .unwrap();
    res.last()
        .unwrap()
        .rows
        .iter()
        .map(|row| match &row[0] {
            Value::Text(s) => s.clone(),
            other => panic!("expected a label, got {other:?}"),
        })
        .collect()
}

/// `r(id, k, m)` under `CREATE UNIQUE INDEX r_u ON r (k) WHERE m > 'ok'`, over
/// `('sad','ok','happy')` — so only `happy` is in the index before a reorder.
fn setup(db: &mut Database) {
    db.execute("CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')")
        .unwrap();
    db.execute("CREATE TABLE r (id INT, k INT, m mood NOT NULL)")
        .unwrap();
    db.execute("CREATE UNIQUE INDEX r_u ON r (k) WHERE m > 'ok'")
        .unwrap();
}

/// Reorder `mood` to `('happy','ok','sad')` — the same three labels, reversed, so only
/// the *order* moves and the membership verdict flips for every row.
fn reorder(db: &mut Database) {
    db.execute("DROP TYPE mood").unwrap();
    db.execute("CREATE TYPE mood AS ENUM ('happy', 'ok', 'sad')")
        .unwrap();
}

#[test]
fn reorder_then_insert_refuses_the_duplicate() {
    let mut db = Database::with_btree();
    setup(&mut db);
    db.execute("INSERT INTO r VALUES (1, 1, 'happy')").unwrap();
    reorder(&mut db);
    // Under the reordered type `sad` is the last member, so both rows satisfy
    // `m > 'ok'` and share `k = 8`.
    db.execute("INSERT INTO r VALUES (3, 8, 'sad')").unwrap();
    let dup = db.execute("INSERT INTO r VALUES (4, 8, 'sad')");
    assert!(
        dup.is_err(),
        "the second row at k=8 satisfies the live predicate and must collide"
    );
    assert_eq!(count_at_k(&mut db, 8), 1);
}

#[test]
fn a_row_stored_before_the_reorder_is_indexed_by_the_new_order() {
    let mut db = Database::with_btree();
    setup(&mut db);
    // `sad` is the *first* member here, so this row is **not** in the index.
    db.execute("INSERT INTO r VALUES (5, 7, 'sad')").unwrap();
    reorder(&mut db);
    // It is now, and its entry has to exist or the duplicate below finds nothing —
    // which is why the fix rebuilds the *entries* and not only the definition.
    let dup = db.execute("INSERT INTO r VALUES (6, 7, 'sad')");
    assert!(
        dup.is_err(),
        "the pre-reorder row is in the index under the new order, so this collides"
    );
    assert_eq!(count_at_k(&mut db, 7), 1);
}

#[test]
fn a_row_the_new_order_excludes_leaves_the_index() {
    let mut db = Database::with_btree();
    setup(&mut db);
    // `happy` is in the index before the reorder (it is the last member) and out of it
    // after (it becomes the first). The **probing** row has to be one the *new* order
    // puts in, or its key is never looked up and a stale entry is unobservable — which
    // is what made the first version of this test pass against a spec-only refresh.
    db.execute("INSERT INTO r VALUES (1, 3, 'happy')").unwrap();
    reorder(&mut db);
    db.execute("INSERT INTO r VALUES (2, 3, 'sad')")
        .expect("`happy` is out of the index under the new order, so k=3 is free");
    assert_eq!(count_at_k(&mut db, 3), 2);
}

#[test]
fn a_rolled_back_reorder_leaves_the_specs_on_the_restored_order() {
    let mut db = Database::with_btree();
    setup(&mut db);
    db.execute("INSERT INTO r VALUES (1, 1, 'happy')").unwrap();
    db.execute("BEGIN").unwrap();
    reorder(&mut db);
    db.execute("ROLLBACK").unwrap();
    // The catalog is back on ('sad','ok','happy'), and the specs have to follow it
    // *backwards* — the resync at the DDL sites cannot see a restore.
    let dup = db.execute("INSERT INTO r VALUES (2, 1, 'happy')");
    assert!(
        dup.is_err(),
        "after the rollback `happy` is the last member again, so this collides"
    );
    // ...and the other direction: `sad` is first again, so a `sad` pair is legal.
    db.execute("INSERT INTO r VALUES (3, 4, 'sad')").unwrap();
    db.execute("INSERT INTO r VALUES (4, 4, 'sad')")
        .expect("neither `sad` row is in the index under the restored order");
    assert_eq!(count_at_k(&mut db, 4), 2);
}

/// The **`DROP TYPE`-only** rollback: the block removes the type and never re-creates
/// it, so the name the restore brings back is one the live catalog no longer holds.
///
/// This is the arm of `enums_moved_by_restore` its **second** loop answers. The other
/// backward scripts here that move an enum at all move it by *reordering* — a
/// `DROP TYPE` followed by a `CREATE TYPE` of the same name — which leaves `mood` present
/// in both maps with different labels, so the *first* loop names it and the second finds
/// nothing to add. Do not read that as the reach argument: the measurement is.
/// With the second loop deleted and both new guards present,
/// `cargo test --workspace --no-fail-fast` is 1690 passed / 2 failed, and the two are
/// this test and `pglet_sql`'s `enums_moved_tests` — so the pre-existing tests stay green
/// to a one while this script writes a duplicate under a unique partial index.
///
/// The mechanism: the forward `DROP TYPE` rebuilds `r_u`'s spec with no enum, so
/// `column_enum_orders` falls back to the label bytes, where `'happy' < 'ok'` and neither
/// row is in the index. The `ROLLBACK` puts `('sad','ok','happy')` back, under which
/// `happy` is the last member and both rows are in it — so the specs have to follow, or
/// the second row lands.
///
/// **Two widths, and the second needs a row stored before the block.** The pair at
/// `k = 9` is decided by the restored *definition* alone — both its rows are inserted
/// after the `ROLLBACK`, so the only entry either probes is the one the other just wrote,
/// and a resync that swapped the def and left the entries (`reattach_index`'s failure
/// mode, which `Storage::rebuild_index`'s own doc names) passes it. The pre-block row at
/// `k = 2` is the arm that reads the **entries**: the forward `DROP TYPE` took its entry
/// out of `r_u` (label bytes put `happy` below `ok`), and only a re-derivation puts it
/// back. Measured
/// with `rebuild_indexes_over_enums` reattaching instead of rebuilding — a definition-only
/// backward resync — this file is 17 passed / 4 failed with the pre-block arm and 18 / 3
/// without it, this test being the one that moves between them; the other three failures
/// are the siblings that already store a row before their block.
#[test]
fn a_rolled_back_drop_type_with_no_re_create_leaves_the_specs_on_the_restored_order() {
    let mut db = Database::with_btree();
    setup(&mut db);
    // Stored **before** the block, so the restore has an entry to move and not only a
    // definition: `happy` is the last member here, so this row is in `r_u` at `k = 2`.
    db.execute("INSERT INTO r VALUES (7, 2, 'happy')").unwrap();
    db.execute("BEGIN").unwrap();
    db.execute("DROP TYPE mood").unwrap();
    db.execute("ROLLBACK").unwrap();
    // The **premise**, not the claim: the catalog half of the restore is other tests' to
    // hold, and this one's conclusion — "so the specs have to follow" — is worthless if
    // the catalog did not move. Read it rather than assumed.
    assert_eq!(
        member_labels(&mut db),
        vec!["sad", "ok", "happy"],
        "the rollback restores the member list the block dropped"
    );
    // The discriminating pair, asserted first: a panicking pin pre-empts every later
    // one, and it is this pair — not the accept-side one below — that the second loop's
    // deletion reddens.
    db.execute("INSERT INTO r VALUES (3, 9, 'happy')")
        .expect("the first row at k=9 is the index's only claimant");
    let dup = db
        .execute("INSERT INTO r VALUES (4, 9, 'happy')")
        .expect_err("the restored order puts `happy` last, so both rows are in `r_u`");
    // Which error, not merely that there was one: a fix that made this statement fail
    // for any other reason would satisfy an `is_err()` and leave the index wrong.
    let message = dup.to_string();
    assert!(
        message.contains("r_u"),
        "expected the unique index's own refusal, got {message:?}"
    );
    assert_eq!(count_at_k(&mut db, 9), 1);
    // The **entries** width, which the pair above cannot reach: the pre-block row's entry
    // has to be back in `r_u`, and a def-only refresh leaves it where the forward
    // `DROP TYPE` put it — out.
    let stale = db
        .execute("INSERT INTO r VALUES (8, 2, 'happy')")
        .expect_err("the pre-block row is back in `r_u` under the restored order");
    let stale = stale.to_string();
    assert!(
        stale.contains("r_u"),
        "expected the unique index's own refusal, got {stale:?}"
    );
    assert_eq!(count_at_k(&mut db, 2), 1);
    // The accept side, which the label-byte fallback gets right by luck and so separates
    // nothing on its own: `sad` is the first member again, so a `sad` pair is legal.
    db.execute("INSERT INTO r VALUES (5, 4, 'sad')").unwrap();
    db.execute("INSERT INTO r VALUES (6, 4, 'sad')")
        .expect("neither `sad` row is in the index under the restored order");
    assert_eq!(count_at_k(&mut db, 4), 2);
}

/// A caught `DO … EXCEPTION` restores the catalog through the same
/// `Schema::restore_from` the two rollbacks take, so it has to move the specs back too.
///
/// The third backward caller, and the one that was written without the pair: the body
/// reorders `mood`, raises, and the handler swallows it — leaving the catalog on
/// `('sad','ok','happy')` and, before the fix, the index specs on the block's
/// `('happy','ok','sad')`. Every statement here reported success, so nothing was loud;
/// what it produced was a duplicate under a unique partial index. 18.6 cannot be the
/// oracle for it (its own `DROP TYPE` under a live dependent column is `2BP01`, so the
/// body's first statement is what *it* catches) — the invariant this holds is pglet's
/// own, that its catalog and its index specs agree.
///
/// Both directions. The `k = 1` pair is asserted first because a panicking pin pre-empts
/// every later one, and it is what each of the three mutations measured red on — `happy`
/// is the last member again under the restored order, so those two rows collide. The
/// `sad` pair below it separates the same way and is simply never reached under those
/// reverts.
#[test]
fn a_caught_do_block_leaves_the_specs_on_the_restored_order() {
    let mut db = Database::with_btree();
    setup(&mut db);
    db.execute("INSERT INTO r VALUES (1, 1, 'happy')").unwrap();
    db.execute(
        "DO $$ BEGIN \
           DROP TYPE mood; \
           CREATE TYPE mood AS ENUM ('happy', 'ok', 'sad'); \
           INSERT INTO nosuch_table VALUES (1); \
         EXCEPTION WHEN others THEN NULL; END $$",
    )
    .expect("the handler swallows the body's error");
    let dup = db.execute("INSERT INTO r VALUES (2, 1, 'happy')");
    assert!(
        dup.is_err(),
        "the handler restored the catalog, so `happy` is the last member again and \
         this collides"
    );
    // ...and the other direction: `sad` is first again, so a `sad` pair is legal.
    db.execute("INSERT INTO r VALUES (3, 4, 'sad')").unwrap();
    db.execute("INSERT INTO r VALUES (4, 4, 'sad')")
        .expect("neither `sad` row is in the index under the restored order");
    assert_eq!(count_at_k(&mut db, 4), 2);
}

/// A failed `DO` with no handler restores the catalog too — the statement's own undo,
/// which takes the same pair as the caught form, since a body's forward rebuild is
/// published at once and no storage rollback takes it back.
///
/// The body reorders `mood` and then fails, so the statement answers the failing
/// statement's `42P01` and leaves the catalog on `('sad','ok','happy')`; the specs must
/// follow it back, or the `k = 1` pair below lands a duplicate. 18.6 cannot be the oracle
/// here for the reason the caught form's doc gives. The catalog half is read first, so
/// an undo that restores nothing reddens on the member list and one that restores the
/// catalog without the resync reddens on the `k = 1` collision.
#[test]
fn a_failed_do_block_leaves_the_specs_on_the_restored_order() {
    let mut db = Database::with_btree();
    setup(&mut db);
    db.execute("INSERT INTO r VALUES (1, 1, 'happy')").unwrap();
    let err = db
        .execute(
            "DO $$ BEGIN \
               DROP TYPE mood; \
               CREATE TYPE mood AS ENUM ('happy', 'ok', 'sad'); \
               INSERT INTO nosuch_table VALUES (1); \
             END $$",
        )
        .unwrap_err();
    assert_eq!(err.sqlstate(), "42P01");
    assert_eq!(member_labels(&mut db), ["sad", "ok", "happy"]);
    let dup = db.execute("INSERT INTO r VALUES (2, 1, 'happy')");
    assert!(
        dup.is_err(),
        "the undo restored the catalog, so `happy` is the last member again and this \
         collides"
    );
    db.execute("INSERT INTO r VALUES (3, 4, 'sad')").unwrap();
    db.execute("INSERT INTO r VALUES (4, 4, 'sad')")
        .expect("neither `sad` row is in the index under the restored order");
    assert_eq!(count_at_k(&mut db, 4), 2);
}

/// The control for the one above: a caught body that raises **without** touching the
/// enum leaves everything where it was.
///
/// It is an **accept-side** pin and not a separating one: it does not separate the
/// fix from a resync firing on every caught block. Replacing
/// `enums_moved_by_restore` with exactly that — a blanket
/// `snapshot.enums.keys().cloned().collect()` — leaves the whole file green, measured.
/// Over a catalog the caught block never moved, a blanket resync re-derives byte-identical
/// specs, so it is a slower correct implementation rather than a wrong one and there is no
/// **behavioural** answer for a test to disagree about. Its *output* is another matter, and
/// that separation is a different file's: `enums_moved_by_restore` is `pub` and pure, so
/// `pglet_sql`'s `enums_moved_tests` asserts the names it returns and is the one test in
/// the workspace that reddens under the blanket (measured: 1691 passed, 1 failed, and the
/// failure is that assertion). What the `moved` comparison buys *here* is that an ordinary
/// caught block re-derives nothing — a cost, which no assertion in this file reads.
#[test]
fn a_caught_do_block_that_moves_no_enum_changes_nothing() {
    let mut db = Database::with_btree();
    setup(&mut db);
    db.execute("INSERT INTO r VALUES (1, 1, 'happy')").unwrap();
    db.execute(
        "DO $$ BEGIN INSERT INTO nosuch_table VALUES (1); EXCEPTION WHEN others THEN NULL; END $$",
    )
    .expect("the handler swallows the body's error");
    let dup = db.execute("INSERT INTO r VALUES (2, 1, 'happy')");
    assert!(dup.is_err(), "`happy` is still the last member");
    db.execute("INSERT INTO r VALUES (3, 4, 'sad')").unwrap();
    db.execute("INSERT INTO r VALUES (4, 4, 'sad')")
        .expect("neither `sad` row is in the index");
    assert_eq!(count_at_k(&mut db, 4), 2);
}

#[test]
fn dropping_an_unrelated_type_leaves_the_index_alone() {
    let mut db = Database::with_btree();
    setup(&mut db);
    db.execute("CREATE TYPE other AS ENUM ('x', 'y')").unwrap();
    db.execute("INSERT INTO r VALUES (1, 1, 'happy')").unwrap();
    db.execute("DROP TYPE other").unwrap();
    let dup = db.execute("INSERT INTO r VALUES (2, 1, 'happy')");
    assert!(
        dup.is_err(),
        "a type no column of `r` declares must not disturb `r_u`"
    );
}

#[test]
fn a_reorder_to_the_same_member_list_leaves_the_index_alone() {
    let mut db = Database::with_btree();
    setup(&mut db);
    db.execute("INSERT INTO r VALUES (1, 1, 'happy')").unwrap();
    db.execute("DROP TYPE mood").unwrap();
    db.execute("CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')")
        .unwrap();
    let dup = db.execute("INSERT INTO r VALUES (2, 1, 'happy')");
    assert!(
        dup.is_err(),
        "the members are unchanged, so every membership verdict is unchanged"
    );
}

/// The index whose key is an **expression** over the enum, rather than a partial
/// predicate over it. Its entries move under a reorder for the same reason, and it is
/// what makes the scan's condition the **table's** columns rather than "does this index
/// have a partial predicate": that narrower spelling passes every other test here.
///
/// The reorder is deliberately *not* a reversal — `('sad','ok','happy')` to
/// `('sad','happy','ok')` — because a reversal moves both the stored row's key and the
/// probing row's key together and the two rules agree by accident. Here `m > 'ok'` is
/// true for `happy` before and false for everything after.
#[test]
fn an_expression_key_over_the_enum_is_rebuilt_too() {
    let mut db = Database::with_btree();
    db.execute("CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')")
        .unwrap();
    db.execute("CREATE TABLE r (id INT, k INT, m mood NOT NULL)")
        .unwrap();
    db.execute("CREATE UNIQUE INDEX r_x ON r ((m > 'ok'))")
        .unwrap();
    db.execute("INSERT INTO r VALUES (1, 1, 'happy')").unwrap();
    db.execute("DROP TYPE mood").unwrap();
    db.execute("CREATE TYPE mood AS ENUM ('sad', 'happy', 'ok')")
        .unwrap();
    // Under the new order nothing is `> 'ok'`, so the stored row's key is `false` and so
    // is this one's — a collision. Against a frozen spec the stored row still keys
    // `true` and this one keys `false`, and the duplicate lands.
    let dup = db.execute("INSERT INTO r VALUES (2, 2, 'sad')");
    assert!(
        dup.is_err(),
        "both rows key `false` under the live order, so the expression key collides"
    );
}

/// `DROP TYPE` **without** a re-create is a move of the member order too: a column left
/// declaring a gone type has no order at all (`enum_order_of` answers `None`), so every
/// comparison beside it falls back to the label bytes — which for `mood` is the reverse
/// verdict. Without this the whole `DROP TYPE` half of the resync is unguarded, because
/// every other test here re-creates the type immediately and the `CREATE TYPE` half then
/// covers for it.
///
/// The observing statement is an `UPDATE` of a **non-enum** column: an `INSERT` cannot
/// reach the state at all (`validate_column` refuses a column whose enum type the catalog
/// lacks, "enum type … not found in catalog"), so the shape that reads it has to be one
/// that writes no enum value.
#[test]
fn dropping_the_type_without_re_creating_it_rebuilds_too() {
    let mut db = Database::with_btree();
    setup(&mut db);
    // Neither row is in the index while `mood` exists — `sad` is its first member.
    db.execute("INSERT INTO r VALUES (1, 1, 'sad')").unwrap();
    db.execute("INSERT INTO r VALUES (2, 2, 'sad')").unwrap();
    db.execute("DROP TYPE mood").unwrap();
    // By label bytes `'sad' > 'ok'`, so both rows are in the index now and moving the
    // second onto the first's key collides.
    let clash = db.execute("UPDATE r SET k = 1 WHERE id = 2");
    assert!(
        clash.is_err(),
        "with no member order left, both `sad` rows are in the index by label bytes"
    );
    assert_eq!(count_at_k(&mut db, 1), 1);
}

/// The in-memory backend reaches the same rebuild through a different implementation —
/// its index defs **own** their entries, so `reattach_index` there would silently empty
/// the index rather than leave stale entries. Both sides of the split need the pin.
#[test]
fn the_memory_backend_rebuilds_too() {
    let mut db = Database::in_memory();
    setup(&mut db);
    db.execute("INSERT INTO r VALUES (5, 7, 'sad')").unwrap();
    reorder(&mut db);
    let dup = db.execute("INSERT INTO r VALUES (6, 7, 'sad')");
    assert!(dup.is_err(), "the pre-reorder row must be in the index");
    assert_eq!(count_at_k(&mut db, 7), 1);
    // And the entries it *did* hold are still there: an emptied index would make this
    // second `sad` pair legal.
    db.execute("INSERT INTO r VALUES (8, 9, 'sad')").unwrap();
    let dup2 = db.execute("INSERT INTO r VALUES (9, 9, 'sad')");
    assert!(dup2.is_err(), "the rebuilt index must hold its new entries");
}

/// The in-memory backend's **backward** direction, and it is a behaviour pin rather than
/// a guard on the resync — say which, because the two are not the same claim.
///
/// Measured: deleting `resync_index_specs` from this backend's rollback arm leaves this
/// test green. The reason is structural — the memory rollback restores the whole store
/// (`*mem = snapshot`), and an index definition's spec is *in* that store, so it comes
/// back with everything else. What this pins is the **outcome** a caller sees, which is
/// the half that survives someone narrowing that wholesale restore; the resync's own
/// necessity is `a_rolled_back_reorder_leaves_the_specs_on_the_restored_order`'s to hold,
/// on the B+tree backend, where the store is not restored wholesale.
#[test]
fn the_memory_backend_rolls_a_reorder_back_too() {
    let mut db = Database::in_memory();
    setup(&mut db);
    db.execute("INSERT INTO r VALUES (1, 1, 'happy')").unwrap();
    db.execute("BEGIN").unwrap();
    reorder(&mut db);
    db.execute("ROLLBACK").unwrap();
    let dup = db.execute("INSERT INTO r VALUES (2, 1, 'happy')");
    assert!(
        dup.is_err(),
        "after the rollback `happy` is the last member again, so this collides"
    );
    // ...and the other direction: `sad` is first again, so a `sad` pair is legal.
    db.execute("INSERT INTO r VALUES (3, 4, 'sad')").unwrap();
    db.execute("INSERT INTO r VALUES (4, 4, 'sad')")
        .expect("neither `sad` row is in the index under the restored order");
    assert_eq!(count_at_k(&mut db, 4), 2);
}

/// A row written **inside** the transaction that moves the enum is indexed by the moved
/// order, and the mechanism is not this rebuild.
///
/// `rebuild_index` re-derives the entries from the table's **committed** rows, so a write
/// still pending in the same transaction is outside what it can see — which reads like a
/// hole and is not one: a pending write is materialized at commit through
/// `emit_index_ops` against the `index_defs` the rebuild has *already* replaced, so it
/// lands under the new rule. Nothing else in the repo states that, and a reader of
/// `rebuild_index` would have to derive it from two files.
#[test]
fn a_write_pending_across_the_reorder_is_indexed_by_the_new_order() {
    let mut db = Database::with_btree();
    // A bespoke pair of orders, not `setup`/`reorder`: `happy` has to be **out** of the
    // index under the starting order *and* out of it under the label bytes the interim
    // `DROP TYPE` falls back to, so that only the re-`CREATE TYPE`'s order puts it in.
    // With the shared pair the label bytes already agree with the destination and the
    // test passes with the `CREATE TYPE` rebuild deleted — measured.
    db.execute("CREATE TYPE mood AS ENUM ('happy', 'ok', 'sad')")
        .unwrap();
    db.execute("CREATE TABLE r (id INT, k INT, m mood NOT NULL)")
        .unwrap();
    db.execute("CREATE UNIQUE INDEX r_u ON r (k) WHERE m > 'ok'")
        .unwrap();
    db.execute("BEGIN").unwrap();
    // `happy` is the first member here, so this row is out of the index as it is written
    // — and stays out under the label bytes (`'happy' < 'ok'`).
    db.execute("INSERT INTO r VALUES (1, 1, 'happy')").unwrap();
    db.execute("DROP TYPE mood").unwrap();
    db.execute("CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')")
        .unwrap();
    db.execute("COMMIT").unwrap();
    // Under ('sad','ok','happy') it is `> 'ok'` and so in the index — which only a
    // colliding second row can observe.
    let dup = db.execute("INSERT INTO r VALUES (2, 1, 'happy')");
    assert!(
        dup.is_err(),
        "the row written inside the reordering transaction must be in the index"
    );
    assert_eq!(count_at_k(&mut db, 1), 1);
}

/// `rf(id, d INT, m mood NOT NULL)` under a **partial** index keyed on `(10 / d)` — the
/// shape that makes the resync's own rebuild fail.
///
/// A key expression is evaluated only for a row the predicate admits, so an index whose
/// predicate admits no row builds fine over a `d` of zero, and a write of one lands. A
/// member-order move that admits such a row then makes `rebuild_index` raise the
/// division's `22012` — inside a `CREATE TYPE` / `DROP TYPE` that has otherwise
/// succeeded. That is the reaching failure the staging in
/// `executor::ddl::rebuild_indexes_over_enum` exists for. The trigger is the row's
/// **value**, not its type: no key type the encoder learns to take can close it.
fn setup_unencodable(db: &mut Database, members: &str, predicate: &str) {
    db.execute(&format!("CREATE TYPE mood AS ENUM ({members})"))
        .unwrap();
    db.execute("CREATE TABLE rf (id INT, d INT, m mood NOT NULL)")
        .unwrap();
    db.execute(&format!(
        "CREATE INDEX rf_f ON rf ((10 / d)) WHERE {predicate}"
    ))
    .unwrap();
}

/// A `DROP TYPE` whose index rebuild fails must leave the type registered.
///
/// Without the staging both halves moved: the statement reported the encoder's error and
/// the catalog kept the drop, so a type no statement claims to have removed was gone —
/// the `docs/ddl.md` rule (*a failed DDL statement must leave the catalog untouched*)
/// broken by the index-rebuild code itself, and unrecoverable on the autocommit path.
#[test]
fn a_drop_type_whose_rebuild_fails_leaves_the_type_registered() {
    let mut db = Database::with_btree();
    // Under ('sad','ok','happy') the `sad` row is not `> 'ok'`, so the index is empty and
    // evaluates no key. Dropping the type falls back to the label bytes, where
    // `'sad' > 'ok'`, and the row's key — `10 / 0` — then has to be evaluated.
    setup_unencodable(&mut db, "'sad', 'ok', 'happy'", "m > 'ok'");
    db.execute("INSERT INTO rf VALUES (1, 0, 'sad')").unwrap();
    let dropped = db.execute("DROP TYPE mood");
    assert_eq!(
        dropped.unwrap_err().sqlstate(),
        "22012",
        "the rebuild has to evaluate the admitted row's key and cannot"
    );
    // The type is still there — which only a *second* `CREATE TYPE` of the same name can
    // observe, since a surviving column declaring a gone type reads the same either way.
    let recreate = db.execute("CREATE TYPE mood AS ENUM ('a', 'b')");
    assert!(
        recreate.is_err(),
        "a failed DROP TYPE must leave the name taken"
    );
    // ...and on its original member list, not the label bytes: `sad` is still the first
    // member, so nothing is in the index and a second zero `d` lands.
    db.execute("INSERT INTO rf VALUES (2, 0, 'sad')").unwrap();
}

/// The same for `CREATE TYPE`: a re-create whose rebuild fails must not register the
/// name.
///
/// The two orders are chosen so the drop **succeeds** — `happy` is out of the index under
/// `('ok','happy','sad')` by position *and* out of it by the label bytes the drop falls
/// back to — and only the re-create's order admits the row.
#[test]
fn a_create_type_whose_rebuild_fails_leaves_the_name_free() {
    let mut db = Database::with_btree();
    setup_unencodable(&mut db, "'ok', 'happy', 'sad'", "m > 'sad'");
    db.execute("INSERT INTO rf VALUES (1, 0, 'happy')").unwrap();
    db.execute("DROP TYPE mood")
        .expect("`happy` is out of the index by position and by label bytes alike");
    let created = db.execute("CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')");
    assert!(
        created.is_err(),
        "under the new order `happy > sad`, so the rebuild has to evaluate the row's key"
    );
    // The name is free again — a failed CREATE TYPE that had registered would make this
    // one a duplicate rather than a fresh registration.
    db.execute("CREATE TYPE mood AS ENUM ('ok', 'happy', 'sad')")
        .expect("the failed CREATE TYPE must not have taken the name");
}

/// Two tables carrying the same enum, one index each — the split a single-index setup
/// cannot reach: the loop moves the **first** index's spec onto the staged order, raises
/// on the second, and staging then holds only the *catalog* back.
///
/// The two names are chosen deliberately, not incidentally: `Schema.indexes` is a
/// `BTreeMap`, so the loop runs in lexical index-name order and `a_u` — the unique
/// partial index whose membership the move flips — is rebuilt **before** `z_f`, the
/// one keyed on `(10 / d)`, whose rebuild raises. Reversing the names (the sibling test below)
/// aborts the loop before `a_u` moves and answers correctly with or without the unwind,
/// which is exactly why the order is pinned here.
///
/// Measured before the unwind, on this shape: the failed `DROP TYPE` left the executor
/// reading the pre-statement order and storage holding the staged one, and two rows
/// sharing `k` both landed — `(2 rows)` against the `23505` the same script without the
/// failing statement raises.
#[test]
fn a_failed_rebuild_puts_back_the_specs_it_had_already_moved() {
    let mut db = Database::with_btree();
    db.execute("CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')")
        .unwrap();
    db.execute("CREATE TABLE ra (id INT, k INT, m mood NOT NULL)")
        .unwrap();
    db.execute("CREATE UNIQUE INDEX a_u ON ra (k) WHERE m > 'ok'")
        .unwrap();
    db.execute("CREATE TABLE rb (id INT, d INT, m mood NOT NULL)")
        .unwrap();
    db.execute("CREATE INDEX z_f ON rb ((10 / d)) WHERE m > 'ok'")
        .unwrap();
    db.execute("INSERT INTO rb VALUES (1, 0, 'sad')").unwrap();

    let dropped = db.execute("DROP TYPE mood");
    assert!(
        dropped.is_err(),
        "the label-byte fallback admits the `sad` row, whose key cannot be evaluated"
    );

    // `happy` is `> 'ok'` under the type the statement failed to move, so both rows are
    // in `a_u` and share `k = 7`.
    db.execute("INSERT INTO ra VALUES (1, 7, 'happy')").unwrap();
    let dup = db.execute("INSERT INTO ra VALUES (2, 7, 'happy')");
    assert!(
        dup.is_err(),
        "the unique index whose spec the failed statement moved must be back on the \
         order the catalog is still on"
    );
    assert_eq!(count_at_k_in(&mut db, "ra", 7), 1);
}

/// The same two indexes with the names swapped, so the loop raises on the `(10 / d)` one
/// **first** and never reaches the unique index.
///
/// A control rather than a guard: it answers correctly with the unwind and without it,
/// and it is here to pin that the sibling above measures the *partial* rebuild rather
/// than something about the shape both share.
#[test]
fn the_failing_index_first_never_moves_the_other_one() {
    let mut db = Database::with_btree();
    db.execute("CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')")
        .unwrap();
    db.execute("CREATE TABLE ra (id INT, k INT, m mood NOT NULL)")
        .unwrap();
    db.execute("CREATE UNIQUE INDEX z_u ON ra (k) WHERE m > 'ok'")
        .unwrap();
    db.execute("CREATE TABLE rb (id INT, d INT, m mood NOT NULL)")
        .unwrap();
    db.execute("CREATE INDEX a_f ON rb ((10 / d)) WHERE m > 'ok'")
        .unwrap();
    db.execute("INSERT INTO rb VALUES (1, 0, 'sad')").unwrap();

    assert!(db.execute("DROP TYPE mood").is_err());

    db.execute("INSERT INTO ra VALUES (1, 7, 'happy')").unwrap();
    assert!(db.execute("INSERT INTO ra VALUES (2, 7, 'happy')").is_err());
    assert_eq!(count_at_k_in(&mut db, "ra", 7), 1);
}

/// The partial-rebuild split on the in-memory backend, whose defs own their entries.
#[test]
fn a_failed_rebuild_puts_back_the_specs_it_had_already_moved_in_memory() {
    let mut db = Database::in_memory();
    db.execute("CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')")
        .unwrap();
    db.execute("CREATE TABLE ra (id INT, k INT, m mood NOT NULL)")
        .unwrap();
    db.execute("CREATE UNIQUE INDEX a_u ON ra (k) WHERE m > 'ok'")
        .unwrap();
    db.execute("CREATE TABLE rb (id INT, d INT, m mood NOT NULL)")
        .unwrap();
    db.execute("CREATE INDEX z_f ON rb ((10 / d)) WHERE m > 'ok'")
        .unwrap();
    db.execute("INSERT INTO rb VALUES (1, 0, 'sad')").unwrap();

    assert!(db.execute("DROP TYPE mood").is_err());

    db.execute("INSERT INTO ra VALUES (1, 7, 'happy')").unwrap();
    assert!(db.execute("INSERT INTO ra VALUES (2, 7, 'happy')").is_err());
    assert_eq!(count_at_k_in(&mut db, "ra", 7), 1);
}

/// A `ROLLBACK` is not the repair either, and was measured not to be: staging leaves the
/// catalog unmoved, so `enums_moved_by_restore` finds nothing to resync and the storage
/// half stays where the failed loop left it. The unwind is what closes this spelling too.
#[test]
fn a_failed_rebuild_inside_a_block_is_unwound_by_the_statement_not_the_rollback() {
    let mut db = Database::with_btree();
    db.execute("CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')")
        .unwrap();
    db.execute("CREATE TABLE ra (id INT, k INT, m mood NOT NULL)")
        .unwrap();
    db.execute("CREATE UNIQUE INDEX a_u ON ra (k) WHERE m > 'ok'")
        .unwrap();
    db.execute("CREATE TABLE rb (id INT, d INT, m mood NOT NULL)")
        .unwrap();
    db.execute("CREATE INDEX z_f ON rb ((10 / d)) WHERE m > 'ok'")
        .unwrap();
    db.execute("INSERT INTO rb VALUES (1, 0, 'sad')").unwrap();

    db.execute("BEGIN").unwrap();
    assert!(db.execute("DROP TYPE mood").is_err());
    db.execute("ROLLBACK").unwrap();

    db.execute("INSERT INTO ra VALUES (1, 7, 'happy')").unwrap();
    assert!(db.execute("INSERT INTO ra VALUES (2, 7, 'happy')").is_err());
    assert_eq!(count_at_k_in(&mut db, "ra", 7), 1);
}

/// `next_oid` is a catalog fact like the rest, so a `CREATE TYPE` whose rebuild fails
/// must not have advanced it.
///
/// The route is narrow and worth naming: a `CREATE TYPE` of a **fresh** name cannot fail
/// this way at all (no table carries a column of it, so the rebuild loop is empty), and a
/// duplicate name is refused before the mint. Only the re-create of a name a live column
/// still declares mints and then fails. Measured before the mint moved: two tables either
/// side of the failed statement took `16387` and `16389`, the number in between consumed
/// by a statement that reported an error.
#[test]
fn a_create_type_whose_rebuild_fails_leaves_the_oid_counter_where_it_was() {
    let mut db = Database::with_btree();
    setup_unencodable(&mut db, "'ok', 'happy', 'sad'", "m > 'sad'");
    db.execute("INSERT INTO rf VALUES (1, 0, 'happy')").unwrap();
    db.execute("DROP TYPE mood").unwrap();
    db.execute("CREATE TABLE pa (id INT)").unwrap();
    let before = relation_oid(&mut db, "pa");

    let created = db.execute("CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')");
    assert!(
        created.is_err(),
        "under the new order `happy > sad`, so the rebuild has to evaluate the row's key"
    );

    db.execute("CREATE TABLE pb (id INT)").unwrap();
    assert_eq!(
        relation_oid(&mut db, "pb"),
        before + 1,
        "the refused CREATE TYPE must not have taken an OID"
    );
    // What this cannot see: dropping the counter write altogether also passes, because
    // `next_object_oid` re-seeds from the live catalog max. That is a different bug (an
    // OID freed by a `DROP` would be handed out again, which is what the monotonic rule
    // on `generate_object_oid` exists to prevent) and it wants its own guard.
}
