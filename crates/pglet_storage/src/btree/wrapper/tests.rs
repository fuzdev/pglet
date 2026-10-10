use super::*;
use pglet_sql::executor::IndexMethod;

/// SQL predicate/key-expression → the serialized planner `Expr` IR an `IndexSpec`
/// carries (native parse against an empty schema → `serialize_expr`).
fn ir(sql: &str) -> Vec<u8> {
    let expr =
        pglet_sql::parser::parse_expr(sql, &pglet_sql::catalog::Schema::new()).expect("parse");
    pglet_sql::planner::serialize_expr(&expr).expect("serialize")
}

/// A full (predicate-free, default NULLS DISTINCT) index spec — the shape the
/// pre-partial maintenance/probe tests use. `column_names` is unused without a
/// predicate, so it's left empty.
fn full_spec(index_id: u32, table: &str, positions: Vec<usize>, unique: bool) -> IndexSpec {
    IndexSpec {
        index_id,
        name: "test_index".to_string(),
        table: table.to_string(),
        key_col_positions: positions,
        key_part_ir: None,
        unique,
        predicate_ir: None,
        nulls_not_distinct: false,
        column_names: Vec::new(),
        column_types: Vec::new(),
        column_enum_orders: Vec::new(),
        method: IndexMethod::Btree,
    }
}

#[test]
fn test_create_and_drop_table() {
    let mut wrapper = BtreeStorageWrapper::new();

    // Create table
    wrapper.create_table("users").unwrap();
    assert!(wrapper.name_to_id.contains_key("users"));

    // Create duplicate fails. `create_table`/`drop_table` are `Storage` trait
    // methods, so the storage errors surface as the mapped `pglet_core` forms.
    assert!(matches!(
        wrapper.create_table("users"),
        Err(pglet_core::Error::Catalog(
            pglet_core::CatalogError::TableAlreadyExists { .. }
        ))
    ));

    // Drop table
    wrapper.drop_table("users").unwrap();
    assert!(!wrapper.name_to_id.contains_key("users"));

    // Drop non-existent fails
    assert!(matches!(
        wrapper.drop_table("users"),
        Err(pglet_core::Error::Catalog(
            pglet_core::CatalogError::TableNotFound { .. }
        ))
    ));
}

#[test]
fn test_insert_and_scan() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    // Insert rows
    let id1 = wrapper
        .insert("users", vec![Value::Int4(1), Value::Text("Alice".into())])
        .unwrap();
    let id2 = wrapper
        .insert("users", vec![Value::Int4(2), Value::Text("Bob".into())])
        .unwrap();

    assert_eq!(id1, 0);
    assert_eq!(id2, 1);

    // Scan
    let rows = wrapper.scan("users").unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn test_insert_many_rows() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("test").unwrap();

    // Insert 200 rows (should trigger multiple splits)
    for i in 0..200 {
        wrapper
            .insert("test", vec![Value::Int4(i), Value::Text(format!("row{i}"))])
            .unwrap();
    }

    // Verify all rows can be scanned
    let rows = wrapper.scan("test").unwrap();
    assert_eq!(rows.len(), 200, "Expected 200 rows, got {}", rows.len());

    // Verify each row can be retrieved via get()
    for i in 0..200 {
        let row = wrapper.get("test", i).unwrap();
        assert!(row.is_some(), "Row {i} not found");
        assert_eq!(row.unwrap()[0], Value::Int4(i as i32));
    }
}

#[test]
fn test_get() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    let id = wrapper.insert("users", vec![Value::Int4(42)]).unwrap();

    let row = wrapper.get("users", id).unwrap();
    assert_eq!(row, Some(vec![Value::Int4(42)]));

    // Non-existent row
    let row = wrapper.get("users", 999).unwrap();
    assert_eq!(row, None);
}

#[test]
fn test_delete() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    let id = wrapper.insert("users", vec![Value::Int4(42)]).unwrap();

    // Delete returns old row
    let old = wrapper.delete("users", id).unwrap();
    assert_eq!(old, Some(vec![Value::Int4(42)]));

    // Row is gone
    let row = wrapper.get("users", id).unwrap();
    assert_eq!(row, None);

    // Delete again returns None
    let old = wrapper.delete("users", id).unwrap();
    assert_eq!(old, None);
}

#[test]
fn test_update() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    let id = wrapper.insert("users", vec![Value::Int4(1)]).unwrap();

    // Update returns old row
    let old = wrapper.update("users", id, vec![Value::Int4(2)]).unwrap();
    assert_eq!(old, Some(vec![Value::Int4(1)]));

    // Read updated value
    let row = wrapper.get("users", id).unwrap();
    assert_eq!(row, Some(vec![Value::Int4(2)]));

    // Update non-existent returns None
    let old = wrapper.update("users", 999, vec![Value::Int4(3)]).unwrap();
    assert_eq!(old, None);
}

#[test]
fn test_add_column() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    wrapper.insert("users", vec![Value::Int4(1)]).unwrap();
    wrapper.insert("users", vec![Value::Int4(2)]).unwrap();

    // Add column with default
    wrapper
        .add_column("users", Value::Text("default".into()))
        .unwrap();

    // All rows should have the new column
    let rows = wrapper.scan("users").unwrap();
    for (_, row) in rows {
        assert_eq!(row.len(), 2);
        assert_eq!(row[1], Value::Text("default".into()));
    }
}

#[test]
fn test_drop_column() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    wrapper
        .insert("users", vec![Value::Int4(1), Value::Text("Alice".into())])
        .unwrap();
    wrapper
        .insert("users", vec![Value::Int4(2), Value::Text("Bob".into())])
        .unwrap();

    // Drop second column
    wrapper.drop_column("users", 1).unwrap();

    // All rows should have one column
    let rows = wrapper.scan("users").unwrap();
    for (_, row) in rows {
        assert_eq!(row.len(), 1);
    }
}

#[test]
fn test_table_isolation() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();
    wrapper.create_table("orders").unwrap();

    wrapper.insert("users", vec![Value::Int4(1)]).unwrap();
    wrapper.insert("orders", vec![Value::Int4(100)]).unwrap();

    let users = wrapper.scan("users").unwrap();
    let orders = wrapper.scan("orders").unwrap();

    assert_eq!(users.len(), 1);
    assert_eq!(orders.len(), 1);
    assert_eq!(users[0].1, vec![Value::Int4(1)]);
    assert_eq!(orders[0].1, vec![Value::Int4(100)]);
}

#[test]
fn test_table_not_found() {
    let wrapper = BtreeStorageWrapper::new();

    // `scan` is a `Storage` trait method now, so `TableNotFound` surfaces as
    // the mapped `pglet_core::Error::Catalog(CatalogError::TableNotFound)`.
    assert!(matches!(
        wrapper.scan("nonexistent"),
        Err(pglet_core::Error::Catalog(
            pglet_core::CatalogError::TableNotFound { .. }
        ))
    ));
}

// ========================================================================
// Transaction Tests
// ========================================================================

#[test]
fn test_begin_transaction() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    // Start transaction
    wrapper.begin_transaction(1).unwrap();
    assert!(wrapper.has_transaction(1));

    // Should have a snapshot version
    assert!(wrapper.transaction_snapshot_version(1).is_some());
}

#[test]
fn test_transaction_insert_visible_to_same_connection() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    // Start transaction
    wrapper.begin_transaction(1).unwrap();

    // Insert within transaction
    let id = wrapper
        .insert_in_tx(1, "users", vec![Value::Int4(42)])
        .unwrap();

    // Should be visible within same transaction (read-your-own-writes)
    let rows = wrapper.scan_in_tx(1, "users").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, vec![Value::Int4(42)]);

    // Also visible via get
    let row = wrapper.get_in_tx(1, "users", id).unwrap();
    assert_eq!(row, Some(vec![Value::Int4(42)]));
}

#[test]
fn test_transaction_commit() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    // Insert within transaction
    wrapper.begin_transaction(1).unwrap();
    wrapper
        .insert_in_tx(1, "users", vec![Value::Int4(42)])
        .unwrap();
    wrapper.commit_transaction(1).unwrap();

    // Transaction is gone
    assert!(!wrapper.has_transaction(1));

    // Data should be visible via auto-commit scan
    let rows = wrapper.scan("users").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, vec![Value::Int4(42)]);
}

#[test]
fn test_transaction_rollback() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    // Insert committed data
    wrapper.insert("users", vec![Value::Int4(1)]).unwrap();

    // Insert within transaction
    wrapper.begin_transaction(1).unwrap();
    wrapper
        .insert_in_tx(1, "users", vec![Value::Int4(2)])
        .unwrap();

    // Should see both rows in transaction
    let rows = wrapper.scan_in_tx(1, "users").unwrap();
    assert_eq!(rows.len(), 2);

    // Rollback
    wrapper.rollback_transaction(1).unwrap();

    // Transaction is gone
    assert!(!wrapper.has_transaction(1));

    // Only committed data visible
    let rows = wrapper.scan("users").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, vec![Value::Int4(1)]);
}

#[test]
fn test_transaction_update_in_tx() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    // Insert committed data
    let id = wrapper.insert("users", vec![Value::Int4(1)]).unwrap();

    // Update within transaction
    wrapper.begin_transaction(1).unwrap();
    let old = wrapper
        .update_in_tx(1, "users", id, vec![Value::Int4(2)])
        .unwrap();
    assert_eq!(old, Some(vec![Value::Int4(1)]));

    // Should see updated value in transaction
    let row = wrapper.get_in_tx(1, "users", id).unwrap();
    assert_eq!(row, Some(vec![Value::Int4(2)]));

    // Rollback
    wrapper.rollback_transaction(1).unwrap();

    // Original value visible
    let row = wrapper.get("users", id).unwrap();
    assert_eq!(row, Some(vec![Value::Int4(1)]));
}

#[test]
fn test_transaction_delete_in_tx() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    // Insert committed data
    let id = wrapper.insert("users", vec![Value::Int4(1)]).unwrap();

    // Delete within transaction
    wrapper.begin_transaction(1).unwrap();
    let old = wrapper.delete_in_tx(1, "users", id).unwrap();
    assert_eq!(old, Some(vec![Value::Int4(1)]));

    // Should NOT see row in transaction
    let row = wrapper.get_in_tx(1, "users", id).unwrap();
    assert_eq!(row, None);

    // Rollback
    wrapper.rollback_transaction(1).unwrap();

    // Row should be visible again
    let row = wrapper.get("users", id).unwrap();
    assert_eq!(row, Some(vec![Value::Int4(1)]));
}

#[test]
fn test_concurrent_transactions() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    // Insert committed data
    wrapper.insert("users", vec![Value::Int4(1)]).unwrap();

    // Start two transactions
    wrapper.begin_transaction(1).unwrap();
    wrapper.begin_transaction(2).unwrap();

    // Each inserts in their own transaction
    wrapper
        .insert_in_tx(1, "users", vec![Value::Int4(10)])
        .unwrap();
    wrapper
        .insert_in_tx(2, "users", vec![Value::Int4(20)])
        .unwrap();

    // Transaction 1 sees committed + its own insert
    let rows1 = wrapper.scan_in_tx(1, "users").unwrap();
    assert_eq!(rows1.len(), 2);

    // Transaction 2 sees committed + its own insert
    let rows2 = wrapper.scan_in_tx(2, "users").unwrap();
    assert_eq!(rows2.len(), 2);

    // Commit transaction 1
    wrapper.commit_transaction(1).unwrap();

    // Transaction 2 still sees its snapshot (doesn't see tx1's commit)
    let rows2 = wrapper.scan_in_tx(2, "users").unwrap();
    assert_eq!(rows2.len(), 2); // committed + its own, NOT tx1's insert

    // Commit transaction 2
    wrapper.commit_transaction(2).unwrap();

    // Now all data visible via auto-commit
    let rows = wrapper.scan("users").unwrap();
    assert_eq!(rows.len(), 3);
}

#[test]
fn rollback_does_not_roll_back_shared_row_id_counter() {
    // Concurrent connections share one wrapper (the server model). The row-id
    // counter is a shared monotonic allocator; a rollback must NOT reset it below a
    // concurrent connection's committed allocation, or a later insert reuses a live
    // RowId and silently overwrites the committed row.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();

    wrapper.begin_transaction(1).unwrap(); // connection A
    wrapper.begin_transaction(2).unwrap(); // connection B

    // B inserts and commits — advances the shared counter past its allocation.
    let b_id = wrapper.insert_in_tx(2, "t", vec![Value::Int4(20)]).unwrap();
    wrapper.commit_transaction(2).unwrap();

    // A inserts (advancing the counter again) then rolls back.
    let a_id = wrapper.insert_in_tx(1, "t", vec![Value::Int4(99)]).unwrap();
    assert!(
        a_id > b_id,
        "the shared counter is monotonic across connections"
    );
    wrapper.rollback_transaction(1).unwrap();

    // A fresh insert must NOT reuse B's committed RowId.
    let c_id = wrapper.insert("t", vec![Value::Int4(30)]).unwrap();
    assert!(
        c_id > b_id,
        "rollback must not let a later insert reuse a committed RowId (c_id={c_id}, b_id={b_id})"
    );

    // B's committed row survives intact — not overwritten by the reused id.
    let rows = wrapper.scan("t").unwrap();
    let b_row = rows
        .iter()
        .find(|(id, _)| *id == b_id)
        .map(|(_, v)| v.clone());
    assert_eq!(
        b_row,
        Some(vec![Value::Int4(20)]),
        "B's committed row must survive A's rollback"
    );
}

#[test]
fn test_ddl_rollback_restores_table_mapping() {
    let mut wrapper = BtreeStorageWrapper::new();

    // Create table in transaction. DDL is auto-commit even in a txn, so the adapter
    // attributes it back to the connection via `note_txn_ddl` (modeled here) — this
    // is what tells rollback the catalog change is THIS txn's own.
    wrapper.begin_transaction(1).unwrap();
    wrapper.create_table("users").unwrap();
    wrapper.note_txn_ddl(1);
    assert!(wrapper.name_to_id.contains_key("users"));

    // With no concurrent DDL, rollback restores (undoes) this txn's own mapping.
    wrapper.rollback_transaction(1).unwrap();
    assert!(!wrapper.name_to_id.contains_key("users"));
}

#[test]
fn a_block_that_never_wrote_ends_without_restoring_the_directory() {
    // A block ends unrestored when it never took the writer: its `BEGIN`-time directory
    // is older than what another connection committed while it read. The host's side of
    // this is `pglet::TransactionSession`; this is the storage half it calls.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();

    wrapper.begin_transaction(1).unwrap(); // connection A, reads only

    // B commits CREATE TABLE b during A's open block (DDL is auto-commit).
    wrapper.create_table("b").unwrap();
    let b_id = *wrapper.name_to_id.get("b").expect("b created");

    wrapper.end_unwritten_transaction(1);
    assert!(!wrapper.has_transaction(1));
    assert_eq!(
        wrapper.name_to_id.get("b"),
        Some(&b_id),
        "A's end must not clobber B's CREATE committed while A read"
    );

    // The allocator is not rolled back below b's id, so a later create can't collide.
    wrapper.create_table("c").unwrap();
    let c_id = *wrapper.name_to_id.get("c").expect("c created");
    assert!(
        c_id > b_id,
        "next_table_id must not roll back over a committed id"
    );
}

#[test]
fn a_rollback_restores_the_state_the_block_took_the_writer_on() {
    // A block that takes the writer is re-based on what is committed then, so its
    // rollback — which restores the directory and the table-id counter unconditionally —
    // takes back exactly its own DDL: B's table, committed before A's first write, stays;
    // A's, created after, goes. Red without the re-base: the restore puts back the
    // `BEGIN`-time directory and `b` is gone.
    let mut wrapper = BtreeStorageWrapper::new();

    wrapper.begin_transaction(1).unwrap(); // connection A
    wrapper.create_table("b").unwrap(); // B commits before A writes
    let b_id = *wrapper.name_to_id.get("b").expect("b created");

    wrapper.rebase_transaction(1); // A takes the writer
    wrapper.create_table("a").unwrap(); // A's own DDL
    wrapper.note_txn_ddl(1);

    wrapper.rollback_transaction(1).unwrap();
    assert!(
        !wrapper.name_to_id.contains_key("a"),
        "A's own table is taken back"
    );
    assert_eq!(wrapper.name_to_id.get("b"), Some(&b_id), "B's table stays");

    // And a savepoint taken before the writer is re-based with the block.
    wrapper.begin_transaction(1).unwrap();
    wrapper.savepoint(1).unwrap();
    wrapper.create_table("b2").unwrap(); // B again, before A writes
    wrapper.rebase_transaction(1);
    wrapper.create_table("a2").unwrap();
    wrapper.note_txn_ddl(1);
    wrapper.rollback_to_savepoint(1).unwrap();
    assert!(!wrapper.name_to_id.contains_key("a2"));
    assert!(
        wrapper.name_to_id.contains_key("b2"),
        "the savepoint was re-based with the block"
    );
    wrapper.rollback_transaction(1).unwrap();
    assert!(wrapper.name_to_id.contains_key("b2"));
    assert_eq!(wrapper.cross_connection_hits(), 0);
}

#[test]
fn drop_table_in_tx_rollback_restores_rows() {
    // `BEGIN; DROP TABLE t; ROLLBACK` must leave t fully intact (the DROP-in-txn
    // row-loss residual). The in-transaction drop is buffered (deferred to commit),
    // so rollback discards the buffer and the rows are never deleted.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper.insert("t", vec![Value::Int4(1)]).unwrap();
    wrapper.insert("t", vec![Value::Int4(2)]).unwrap();

    wrapper.begin_transaction(1).unwrap();
    wrapper.drop_table_in_tx(1, "t").unwrap();
    // Within the transaction the table reads as gone (name mapping removed).
    assert!(!wrapper.name_to_id.contains_key("t"));

    wrapper.rollback_transaction(1).unwrap();

    // The name returns AND the rows were never deleted.
    assert!(wrapper.name_to_id.contains_key("t"));
    let rows = wrapper.scan("t").unwrap();
    assert_eq!(rows.len(), 2, "rolled-back DROP must restore all rows");
}

#[test]
fn drop_table_in_tx_rollback_restores_indexed_table() {
    // The cascade: `execute_drop_table` sheds the table's indexes (drop_index_in_tx)
    // before the table itself (drop_table_in_tx). A ROLLBACK must restore the rows,
    // the index def, and the index entries — all buffered, none deleted.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper
        .insert("t", vec![Value::Int4(0), Value::Int4(10)])
        .unwrap();
    wrapper
        .insert("t", vec![Value::Int4(1), Value::Int4(20)])
        .unwrap();
    wrapper
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();

    wrapper.begin_transaction(1).unwrap();
    // Mirror execute_drop_table's order: indexes first, then the table.
    wrapper.drop_index_in_tx(1, 1).unwrap();
    wrapper.drop_table_in_tx(1, "t").unwrap();

    wrapper.rollback_transaction(1).unwrap();

    // Table + rows back.
    assert!(wrapper.name_to_id.contains_key("t"));
    assert_eq!(wrapper.scan("t").unwrap().len(), 2);
    // Index def still registered and the probe works against the restored entries.
    assert!(
        wrapper.index_defs().contains_key(&1),
        "index def must survive rollback"
    );
    assert_eq!(
        wrapper.index_lookup(1, &[Value::Int4(20)]).unwrap().len(),
        1
    );
    assert_eq!(
        wrapper.index_lookup(1, &[Value::Int4(10)]).unwrap().len(),
        1
    );
}

#[test]
fn drop_in_tx_commit_applies_drop() {
    // The other half: COMMIT of a buffered drop must actually remove the table, its
    // rows, SYS_TABLE entry, the index def, and the index entries.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper
        .insert("t", vec![Value::Int4(0), Value::Int4(10)])
        .unwrap();
    wrapper
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();
    let table_id = wrapper.name_to_id["t"];

    wrapper.begin_transaction(1).unwrap();
    wrapper.drop_index_in_tx(1, 1).unwrap();
    wrapper.drop_table_in_tx(1, "t").unwrap();
    wrapper.commit_transaction(1).unwrap();

    // Catalog mapping + index def gone.
    assert!(!wrapper.name_to_id.contains_key("t"));
    assert!(!wrapper.index_defs().contains_key(&1));
    // Rows are gone from the tree, and the dropped index no longer probes.
    assert!(wrapper.inner().scan_table(table_id).unwrap().is_empty());
    assert!(
        wrapper.index_lookup(1, &[Value::Int4(10)]).is_err(),
        "a committed-dropped index must no longer be registered"
    );
}

#[test]
fn create_index_in_tx_rollback_sheds_phantom() {
    // CREATE INDEX inside a txn is built immediately (read-your-own-writes), but a
    // ROLLBACK must undo it — deregister the def AND shed its entries — or a phantom
    // storage index outlives the catalog's rollback.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper
        .insert("t", vec![Value::Int4(0), Value::Int4(10)])
        .unwrap();

    wrapper.begin_transaction(1).unwrap();
    wrapper
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();
    // Mirror the adapter: record the in-txn create so rollback can undo it.
    wrapper.note_txn_created_index(1, 1);
    // Read-your-own-writes: the index probes within the transaction.
    assert_eq!(
        wrapper.index_lookup(1, &[Value::Int4(10)]).unwrap().len(),
        1
    );

    wrapper.rollback_transaction(1).unwrap();

    // No phantom: def deregistered and the band sheds (probe errors, not stale hit).
    assert!(
        !wrapper.index_defs().contains_key(&1),
        "rolled-back CREATE INDEX must deregister the def"
    );
    assert!(wrapper.index_lookup(1, &[Value::Int4(10)]).is_err());
    // The table + its row are untouched by the index undo.
    assert_eq!(wrapper.scan("t").unwrap().len(), 1);
}

#[test]
fn create_table_pk_in_tx_rollback_no_phantom_under_id_reuse() {
    // The impactful trigger: `CREATE TABLE ... PRIMARY KEY` in a txn auto-creates a
    // unique index. On rollback `next_table_id` is restored (no row sits under the id),
    // so the next create REUSES the freed table_id — a surviving phantom pkey def would
    // then alias the new table. The created-index undo must prevent that.
    let mut wrapper = BtreeStorageWrapper::new();

    wrapper.begin_transaction(1).unwrap();
    wrapper.create_table("t").unwrap();
    wrapper.note_txn_ddl(1); // mirror the adapter's note for create_table
    let t_id = wrapper.name_to_id["t"];
    // Auto-created pkey on column 0, as execute_create_table would do.
    wrapper
        .create_index(full_spec(1, "t", vec![0], true))
        .unwrap();
    wrapper.note_txn_created_index(1, 1);
    wrapper.rollback_transaction(1).unwrap();

    // t is gone and the pkey def did NOT survive.
    assert!(!wrapper.name_to_id.contains_key("t"));
    assert!(
        !wrapper.index_defs().contains_key(&1),
        "phantom pkey def must not survive rollback"
    );

    // The next create reuses t's id, and it has NO indexes aliased onto it.
    wrapper.create_table("u").unwrap();
    let u_id = wrapper.name_to_id["u"];
    assert_eq!(
        u_id, t_id,
        "next_table_id rollback => id reuse (the footgun case)"
    );
    assert!(
        !wrapper.table_has_indexes(u_id),
        "the reused id must carry no phantom index"
    );
}

#[test]
fn create_index_in_tx_commit_keeps_index() {
    // The other half: a COMMITTED in-txn CREATE INDEX is kept (the undo only fires on
    // rollback, never on commit).
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper
        .insert("t", vec![Value::Int4(0), Value::Int4(10)])
        .unwrap();

    wrapper.begin_transaction(1).unwrap();
    wrapper
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();
    wrapper.note_txn_created_index(1, 1);
    wrapper.commit_transaction(1).unwrap();

    assert!(wrapper.index_defs().contains_key(&1));
    assert_eq!(
        wrapper.index_lookup(1, &[Value::Int4(10)]).unwrap().len(),
        1
    );
}

#[test]
fn an_advanced_transaction_reads_its_own_index_build_and_holds_one_snapshot() {
    // An in-transaction `CREATE INDEX` publishes its back-fill at once, and the
    // transaction's probes read at its own snapshot, which predates it: the probe misses
    // the row committed before the transaction until the snapshot moves onto the root the
    // build published (the repo's `EDGES.md` row 147). The move releases the snapshot it
    // replaces, or the transaction would pin the older root's pages until it ended.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper
        .insert("t", vec![Value::Int4(0), Value::Int4(10)])
        .unwrap();
    assert_eq!(wrapper.stats().active_snapshots, 0);

    wrapper.begin_transaction(1).unwrap();
    wrapper
        .create_index(full_spec(1, "t", vec![1], true))
        .unwrap();
    wrapper.note_txn_created_index(1, 1);
    assert!(
        wrapper
            .index_lookup_in_tx(1, 1, &[Value::Int4(10)])
            .unwrap()
            .is_empty(),
        "the snapshot taken before the build holds none of its entries"
    );

    wrapper.advance_transaction_snapshot(1);
    assert_eq!(
        wrapper
            .index_lookup_in_tx(1, 1, &[Value::Int4(10)])
            .unwrap()
            .len(),
        1,
        "the committed row's entry, read at the root the build published"
    );
    assert_eq!(
        wrapper.stats().active_snapshots,
        1,
        "one snapshot held: the one the move replaced is released"
    );

    wrapper.commit_transaction(1).unwrap();
    assert_eq!(wrapper.stats().active_snapshots, 0);
    assert_eq!(
        wrapper.index_lookup(1, &[Value::Int4(10)]).unwrap().len(),
        1
    );
}

#[test]
fn test_commit_empty_transaction() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    // Start transaction but don't do anything
    wrapper.begin_transaction(1).unwrap();
    wrapper.commit_transaction(1).unwrap();

    // Should be fine
    assert!(!wrapper.has_transaction(1));
}

#[test]
fn test_rollback_without_transaction() {
    let mut wrapper = BtreeStorageWrapper::new();

    // Rollback without BEGIN should be a no-op
    let result = wrapper.rollback_transaction(1);
    assert!(result.is_ok());
}

#[test]
fn test_pending_writes() {
    let mut pending = BtreePendingWrites::new();
    assert!(pending.is_empty());

    // Insert
    pending.insert(1, 0, vec![Value::Int4(42)]);
    assert!(!pending.is_empty());

    // Get inserted
    let row = pending.get(1, 0);
    assert!(matches!(row, Some(PendingRow::Data(_))));

    // Delete
    pending.delete(1, 0);
    let row = pending.get(1, 0);
    assert!(matches!(row, Some(PendingRow::Deleted)));

    // Drop table
    pending.drop_table(1);
    assert!(pending.is_table_dropped(1));
}

#[test]
fn test_row_id_persistence() {
    // Recovery reads the table directory + row-id counters back from the tree's
    // SYS_TABLE entries (written eagerly by create_table/insert), not the slot.
    use crate::Storage;

    // Phase 1: Create wrapper, insert rows (each writes/updates SYS_TABLE).
    let mut wrapper1 = BtreeStorageWrapper::new();
    wrapper1.create_table("users").unwrap();

    let id1 = wrapper1
        .insert("users", vec![Value::Text("Alice".into())])
        .unwrap();
    let id2 = wrapper1
        .insert("users", vec![Value::Text("Bob".into())])
        .unwrap();
    let id3 = wrapper1
        .insert("users", vec![Value::Text("Carol".into())])
        .unwrap();

    assert_eq!(id1, 0);
    assert_eq!(id2, 1);
    assert_eq!(id3, 2);

    let next_table_id = wrapper1.next_table_id() as u32;
    let table_id = wrapper1.table_mappings()["users"];

    // Phase 2: Recover a fresh wrapper from the SAME tree (simulating a reopen of
    // the persisted store — the SYS_TABLE entries live in the tree the Arc shares).
    let (wrapper2, _schema_bytes) =
        BtreeStorageWrapper::recover(wrapper1.btree(), next_table_id).unwrap();

    assert!(wrapper2.table_mappings().contains_key("users"));
    let restored_counters = wrapper2.row_id_counters();
    assert_eq!(restored_counters, vec![(table_id, 3)]); // next id continues at 3
}

// ========================================================================
// BtreeSnapshot Tests (Lock-Free Reads)
// ========================================================================

#[test]
fn test_read_snapshot_basic() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    wrapper.insert("users", vec![Value::Int4(1)]).unwrap();
    wrapper.insert("users", vec![Value::Int4(2)]).unwrap();

    // Get snapshot
    let snapshot = wrapper.read_snapshot();

    // Snapshot can read data
    let rows = snapshot.scan("users").unwrap();
    assert_eq!(rows.len(), 2);

    // Get specific row
    let row = snapshot.get("users", 0).unwrap();
    assert_eq!(row, Some(vec![Value::Int4(1)]));
}

#[test]
fn test_read_snapshot_isolation() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    wrapper.insert("users", vec![Value::Int4(1)]).unwrap();
    wrapper.insert("users", vec![Value::Int4(2)]).unwrap();

    // Take snapshot
    let snapshot = wrapper.read_snapshot();
    // create_table writes the SYS_TABLE directory entry (1 root swap) + 2 inserts.
    assert_eq!(snapshot.version(), 3);

    // Modify data after snapshot
    wrapper.insert("users", vec![Value::Int4(3)]).unwrap();
    wrapper.insert("users", vec![Value::Int4(4)]).unwrap();

    // Current wrapper sees all 4 rows
    let current_rows = wrapper.scan("users").unwrap();
    assert_eq!(current_rows.len(), 4);

    // Snapshot still sees only 2 rows (snapshot isolation)
    let snapshot_rows = snapshot.scan("users").unwrap();
    assert_eq!(snapshot_rows.len(), 2);
}

#[test]
fn test_read_snapshot_table_not_found() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();

    let snapshot = wrapper.read_snapshot();

    // Table exists
    assert!(snapshot.table_exists("users"));

    // Table doesn't exist
    assert!(!snapshot.table_exists("nonexistent"));

    // Scan non-existent table returns error
    let result = snapshot.scan("nonexistent");
    assert!(matches!(result, Err(StorageError::TableNotFound(_))));
}

#[test]
fn test_read_snapshot_drop_releases_snapshot() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();
    wrapper.insert("users", vec![Value::Int4(1)]).unwrap();

    // Modify to create pending GC pages
    wrapper.insert("users", vec![Value::Int4(2)]).unwrap();

    // Take snapshot
    {
        let _snapshot = wrapper.read_snapshot();
        // Snapshot active - pages pinned
        let stats = wrapper.stats();
        assert_eq!(stats.active_snapshots, 1);
    }
    // Snapshot dropped - reference released

    // After drop, no active snapshots
    let stats = wrapper.stats();
    assert_eq!(stats.active_snapshots, 0);
}

// ========================================================================
// Secondary-index maintenance + probe (the acceptance gate)
// ========================================================================

/// Row IDs from a full scan whose indexed column (position `col`) equals
/// `want` — the ground truth the index probe must match exactly.
fn scan_filter_ids(
    wrapper: &BtreeStorageWrapper,
    table: &str,
    col: usize,
    want: &Value,
) -> Vec<RowId> {
    let mut ids: Vec<RowId> = wrapper
        .scan(table)
        .unwrap()
        .into_iter()
        .filter(|(_, row)| row.get(col) == Some(want))
        .map(|(id, _)| id)
        .collect();
    ids.sort_unstable();
    ids
}

fn sorted(mut v: Vec<RowId>) -> Vec<RowId> {
    v.sort_unstable();
    v
}

#[test]
fn index_probe_matches_scan_filter() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    // Column 1 is the indexed (non-unique) value, with duplicates.
    let vals = [10, 20, 10, 30, 20, 10, 40];
    for (i, v) in vals.iter().enumerate() {
        wrapper
            .insert("t", vec![Value::Int4(i as i32), Value::Int4(*v)])
            .unwrap();
    }
    wrapper
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();

    // Value with multiple matches, a single match, and no match.
    for probe in [10, 20, 30, 40, 99] {
        let want = Value::Int4(probe);
        let by_index = sorted(
            wrapper
                .index_lookup(1, std::slice::from_ref(&want))
                .unwrap(),
        );
        let by_scan = scan_filter_ids(&wrapper, "t", 1, &want);
        assert_eq!(by_index, by_scan, "probe {probe} mismatch");
    }
    // 10 appears three times.
    assert_eq!(
        wrapper.index_lookup(1, &[Value::Int4(10)]).unwrap().len(),
        3
    );
    // 99 appears zero times.
    assert!(
        wrapper
            .index_lookup(1, &[Value::Int4(99)])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn index_backfills_existing_rows_then_maintains() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    // Insert BEFORE the index exists (back-fill must pick these up).
    wrapper
        .insert("t", vec![Value::Int4(0), Value::Text("a".into())])
        .unwrap();
    wrapper
        .insert("t", vec![Value::Int4(1), Value::Text("b".into())])
        .unwrap();
    wrapper
        .create_index(full_spec(7, "t", vec![1], false))
        .unwrap();

    // Back-filled.
    assert_eq!(
        sorted(wrapper.index_lookup(7, &[Value::Text("a".into())]).unwrap()),
        scan_filter_ids(&wrapper, "t", 1, &Value::Text("a".into()))
    );

    // Insert AFTER the index exists (maintained on the write path).
    let id = wrapper
        .insert("t", vec![Value::Int4(2), Value::Text("a".into())])
        .unwrap();
    let mut hits = wrapper.index_lookup(7, &[Value::Text("a".into())]).unwrap();
    hits.sort_unstable();
    assert!(hits.contains(&id));
    assert_eq!(
        hits,
        scan_filter_ids(&wrapper, "t", 1, &Value::Text("a".into()))
    );
}

#[test]
fn index_update_moves_entry_and_delete_removes_it() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    let id = wrapper
        .insert("t", vec![Value::Int4(0), Value::Int4(100)])
        .unwrap();
    wrapper
        .insert("t", vec![Value::Int4(1), Value::Int4(200)])
        .unwrap();
    wrapper
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();

    // Initially under value A=100.
    assert!(
        wrapper
            .index_lookup(1, &[Value::Int4(100)])
            .unwrap()
            .contains(&id)
    );
    assert!(
        !wrapper
            .index_lookup(1, &[Value::Int4(200)])
            .unwrap()
            .contains(&id)
    );

    // UPDATE moves row `id` from A=100 to B=200.
    wrapper
        .update("t", id, vec![Value::Int4(0), Value::Int4(200)])
        .unwrap();
    assert!(
        !wrapper
            .index_lookup(1, &[Value::Int4(100)])
            .unwrap()
            .contains(&id),
        "row left value A after update"
    );
    assert!(
        wrapper
            .index_lookup(1, &[Value::Int4(200)])
            .unwrap()
            .contains(&id),
        "row joined value B after update"
    );
    // Probe still equals scan-filter for both values.
    for v in [100, 200] {
        let want = Value::Int4(v);
        assert_eq!(
            sorted(
                wrapper
                    .index_lookup(1, std::slice::from_ref(&want))
                    .unwrap()
            ),
            scan_filter_ids(&wrapper, "t", 1, &want)
        );
    }

    // DELETE removes it from the index.
    wrapper.delete("t", id).unwrap();
    assert!(
        !wrapper
            .index_lookup(1, &[Value::Int4(200)])
            .unwrap()
            .contains(&id)
    );
    assert_eq!(
        sorted(wrapper.index_lookup(1, &[Value::Int4(200)]).unwrap()),
        scan_filter_ids(&wrapper, "t", 1, &Value::Int4(200))
    );
}

#[test]
fn composite_index_probes_the_full_tuple() {
    // A 2-column index keys on (col0, col1). A probe must match only rows
    // equal in BOTH columns, distinguishing (a,'x') from (a,'y') and (b,'x').
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    let r_ax = wrapper
        .insert("t", vec![Value::Int4(1), Value::Text("x".into())])
        .unwrap();
    let _r_ay = wrapper
        .insert("t", vec![Value::Int4(1), Value::Text("y".into())])
        .unwrap();
    let _r_bx = wrapper
        .insert("t", vec![Value::Int4(2), Value::Text("x".into())])
        .unwrap();
    // Composite index over both columns.
    wrapper
        .create_index(full_spec(1, "t", vec![0, 1], true))
        .unwrap();

    // (1,'x') hits exactly the one row; the other two are excluded.
    let hits = wrapper
        .index_lookup(1, &[Value::Int4(1), Value::Text("x".into())])
        .unwrap();
    assert_eq!(hits, vec![r_ax], "probe must match the full tuple only");

    // A tuple present in no row returns nothing.
    assert!(
        wrapper
            .index_lookup(1, &[Value::Int4(2), Value::Text("y".into())])
            .unwrap()
            .is_empty(),
        "(2,'y') exists in no row"
    );
}

#[test]
fn composite_index_with_null_key_column_is_not_indexed() {
    // Composite NULLS DISTINCT: a row with a NULL in any key column is not a
    // uniqueness candidate, so two such rows that agree on the non-NULL column
    // both stay out of the index (Postgres allows the "duplicate").
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper
        .insert("t", vec![Value::Int4(1), Value::Null])
        .unwrap();
    wrapper
        .insert("t", vec![Value::Int4(1), Value::Null])
        .unwrap();
    // A fully-populated row IS indexed.
    let indexed = wrapper
        .insert("t", vec![Value::Int4(1), Value::Text("a".into())])
        .unwrap();
    wrapper
        .create_index(full_spec(1, "t", vec![0, 1], true))
        .unwrap();

    // The two NULL-bearing rows produced no index entries: a probe with a NULL
    // component returns nothing.
    assert!(
        wrapper
            .index_lookup(1, &[Value::Int4(1), Value::Null])
            .unwrap()
            .is_empty(),
        "a probe with a NULL key column matches nothing"
    );
    // The fully-populated row is found by its full tuple.
    assert_eq!(
        wrapper
            .index_lookup(1, &[Value::Int4(1), Value::Text("a".into())])
            .unwrap(),
        vec![indexed]
    );
}

#[test]
fn index_probe_is_mvcc_consistent_at_snapshot_root() {
    // THE acceptance criterion: a probe at an old snapshot root observes the
    // pre-insert index state, while a current-root probe sees the new rows.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    for i in 0..4 {
        // Two rows with value 50, two with value 60.
        let v = if i < 2 { 50 } else { 60 };
        wrapper
            .insert("t", vec![Value::Int4(i), Value::Int4(v)])
            .unwrap();
    }
    wrapper
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();

    // Pin a read snapshot of the pre-insert index state.
    let snap = wrapper.read_snapshot();
    let before_50 = sorted(snap.index_lookup(1, &[Value::Int4(50)]).unwrap());
    assert_eq!(before_50.len(), 2);

    // Insert MORE rows with the same values and commit (new root).
    wrapper
        .insert("t", vec![Value::Int4(100), Value::Int4(50)])
        .unwrap();
    wrapper
        .insert("t", vec![Value::Int4(101), Value::Int4(60)])
        .unwrap();

    // The OLD snapshot still sees only the pre-insert set...
    assert_eq!(
        sorted(snap.index_lookup(1, &[Value::Int4(50)]).unwrap()),
        before_50,
        "snapshot probe must see the old index state"
    );
    // ...while the current root sees the new row too.
    let now_50 = sorted(wrapper.index_lookup(1, &[Value::Int4(50)]).unwrap());
    assert_eq!(now_50.len(), 3, "current-root probe sees the new row");
    assert_eq!(now_50, scan_filter_ids(&wrapper, "t", 1, &Value::Int4(50)));
}

#[test]
fn index_probe_on_larger_row_count() {
    // Point-read sanity over enough rows to span multiple leaf pages.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    let n = 500;
    for i in 0..n {
        // 50 distinct values, ~10 rows each.
        wrapper
            .insert("t", vec![Value::Int4(i), Value::Int4(i % 50)])
            .unwrap();
    }
    wrapper
        .create_index(full_spec(3, "t", vec![1], false))
        .unwrap();

    for v in [0, 7, 25, 49] {
        let want = Value::Int4(v);
        let by_index = sorted(
            wrapper
                .index_lookup(3, std::slice::from_ref(&want))
                .unwrap(),
        );
        let by_scan = scan_filter_ids(&wrapper, "t", 1, &want);
        assert_eq!(by_index, by_scan, "value {v}");
        assert_eq!(by_index.len(), (n as usize).div_ceil(50));
    }
    // A value present in no row.
    assert!(
        wrapper
            .index_lookup(3, &[Value::Int4(999)])
            .unwrap()
            .is_empty()
    );
}

/// Ground truth for an in-transaction probe: row IDs whose indexed column
/// (position `col`) equals `want`, computed from the read-your-own-writes
/// `scan_in_tx` — exactly what `index_lookup_in_tx` must match.
fn tx_scan_filter_ids(
    wrapper: &BtreeStorageWrapper,
    conn_id: i32,
    table: &str,
    col: usize,
    want: &Value,
) -> Vec<RowId> {
    let mut ids: Vec<RowId> = wrapper
        .scan_in_tx(conn_id, table)
        .unwrap()
        .into_iter()
        .filter(|(_, row)| row.get(col) == Some(want))
        .map(|(id, _)| id)
        .collect();
    ids.sort_unstable();
    ids
}

#[test]
fn index_lookup_in_tx_reflects_uncommitted_writes() {
    // The read-your-own-writes acceptance gate at the storage layer: the
    // in-tx probe must equal the in-tx scan-filter through insert/update/
    // delete, all uncommitted. A naive committed-index probe disagrees.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    // Committed seed: rows 0,2 have k=10; row 1 has k=20.
    wrapper
        .insert("t", vec![Value::Int4(0), Value::Int4(10)])
        .unwrap();
    wrapper
        .insert("t", vec![Value::Int4(1), Value::Int4(20)])
        .unwrap();
    wrapper
        .insert("t", vec![Value::Int4(2), Value::Int4(10)])
        .unwrap();
    wrapper
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();

    wrapper.begin_transaction(9).unwrap();

    // (a) Uncommitted INSERT of a new k=10 row.
    let inserted = wrapper
        .insert_in_tx(9, "t", vec![Value::Int4(3), Value::Int4(10)])
        .unwrap();
    let probe = sorted(
        wrapper
            .index_lookup_in_tx(9, 1, &[Value::Int4(10)])
            .unwrap(),
    );
    assert_eq!(
        probe,
        tx_scan_filter_ids(&wrapper, 9, "t", 1, &Value::Int4(10))
    );
    assert!(
        probe.contains(&inserted),
        "uncommitted insert must be probed"
    );

    // The committed (non-tx) probe still does NOT see it — proves the
    // distinction is real, not an accident of shared state.
    assert!(
        !wrapper
            .index_lookup(1, &[Value::Int4(10)])
            .unwrap()
            .contains(&inserted),
        "committed-index probe must not see the uncommitted row"
    );

    // (b) Uncommitted UPDATE moving row 1 (k=20) onto k=10.
    wrapper
        .update_in_tx(9, "t", 1, vec![Value::Int4(1), Value::Int4(10)])
        .unwrap();
    assert_eq!(
        sorted(
            wrapper
                .index_lookup_in_tx(9, 1, &[Value::Int4(10)])
                .unwrap()
        ),
        tx_scan_filter_ids(&wrapper, 9, "t", 1, &Value::Int4(10))
    );
    // And row 1 has left k=20.
    assert!(
        wrapper
            .index_lookup_in_tx(9, 1, &[Value::Int4(20)])
            .unwrap()
            .is_empty()
    );

    // (c) Uncommitted DELETE of row 0 (k=10).
    wrapper.delete_in_tx(9, "t", 0).unwrap();
    let probe = sorted(
        wrapper
            .index_lookup_in_tx(9, 1, &[Value::Int4(10)])
            .unwrap(),
    );
    assert_eq!(
        probe,
        tx_scan_filter_ids(&wrapper, 9, "t", 1, &Value::Int4(10))
    );
    assert!(
        !probe.contains(&0),
        "uncommitted delete must drop from the probe"
    );

    // NULL probe matches nothing even mid-transaction.
    assert!(
        wrapper
            .index_lookup_in_tx(9, 1, &[Value::Null])
            .unwrap()
            .is_empty()
    );

    wrapper.rollback_transaction(9).unwrap();
}

/// The pending index's core uniqueness guarantee: two rows inserted in the
/// same transaction under the same **fresh** key (no committed row involved) both
/// show up in the fast-path probe, so the second insert's uniqueness check sees the
/// first — intra-batch duplicate detection during a bulk load. Also pins the fast
/// path equal to the scan-filter ground truth as inserts accumulate.
#[test]
fn pending_index_catches_intra_txn_duplicate() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();
    // No committed rows carry k=99, so this exercises the pure fast path (the
    // transaction stays insert-only, so no scan fallback is triggered).
    wrapper.begin_transaction(7).unwrap();

    let a = wrapper
        .insert_in_tx(7, "t", vec![Value::Int4(0), Value::Int4(99)])
        .unwrap();
    assert_eq!(
        sorted(
            wrapper
                .index_lookup_in_tx(7, 1, &[Value::Int4(99)])
                .unwrap()
        ),
        vec![a],
        "the first uncommitted insert is visible under its key"
    );

    let b = wrapper
        .insert_in_tx(7, "t", vec![Value::Int4(1), Value::Int4(99)])
        .unwrap();
    let probe = sorted(
        wrapper
            .index_lookup_in_tx(7, 1, &[Value::Int4(99)])
            .unwrap(),
    );
    assert_eq!(
        probe,
        sorted(vec![a, b]),
        "the second insert's probe must see BOTH pending rows (the duplicate)"
    );
    assert_eq!(
        probe,
        tx_scan_filter_ids(&wrapper, 7, "t", 1, &Value::Int4(99)),
        "fast path equals the scan-filter ground truth"
    );
    // A distinct key isn't polluted by the fast path.
    assert!(
        wrapper
            .index_lookup_in_tx(7, 1, &[Value::Int4(1)])
            .unwrap()
            .is_empty()
    );

    wrapper.rollback_transaction(7).unwrap();
}

/// A staged row that **moves** keys retracts the key it left, in the probe and in the
/// cross-connection claim alike.
///
/// The pending index is maintained rather than rebuilt, so the retraction is the half
/// that has nothing to fall back on: drop it and the vacated key keeps answering — a
/// spurious `23505` for the owning connection and a claim against every other one, for
/// a key nothing is writing. Asserted from both readers — the connection's own probe
/// and the cross-connection claim walk — since they take the same branch and a
/// regression in it should redden both.
#[test]
fn pending_index_retracts_a_key_a_staged_row_left() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper
        .create_index(full_spec(1, "t", vec![1], true))
        .unwrap();

    wrapper.begin_transaction(3).unwrap();
    // Two staged rows under *different* keys, so a retraction that removes the whole
    // key entry rather than one row id is caught by the survivor below.
    let moved = wrapper
        .insert_in_tx(3, "t", vec![Value::Int4(0), Value::Int4(10)])
        .unwrap();
    let stays = wrapper
        .insert_in_tx(3, "t", vec![Value::Int4(1), Value::Int4(20)])
        .unwrap();
    // …and a third that shares the key being vacated, so the entry must survive with
    // exactly the other row in it.
    let shares = wrapper
        .insert_in_tx(3, "t", vec![Value::Int4(2), Value::Int4(10)])
        .unwrap();

    wrapper
        .update_in_tx(3, "t", moved, vec![Value::Int4(0), Value::Int4(30)])
        .unwrap();

    for key in [10, 20, 30] {
        assert_eq!(
            sorted(
                wrapper
                    .index_lookup_in_tx(3, 1, &[Value::Int4(key)])
                    .unwrap()
            ),
            tx_scan_filter_ids(&wrapper, 3, "t", 1, &Value::Int4(key)),
            "the probe must equal the scan-filter ground truth at key {key}"
        );
    }
    assert_eq!(
        sorted(
            wrapper
                .index_lookup_in_tx(3, 1, &[Value::Int4(10)])
                .unwrap()
        ),
        vec![shares],
        "the vacated key keeps the row that still holds it and loses the one that moved"
    );
    assert_eq!(
        sorted(
            wrapper
                .index_lookup_in_tx(3, 1, &[Value::Int4(20)])
                .unwrap()
        ),
        vec![stays],
        "an untouched staged row is not disturbed by another row's move"
    );

    // The same fact as another connection sees it: key 10 is still claimed, by the row
    // that never moved off it, and key 30 by the one that moved on.
    assert!(
        wrapper
            .unique_key_claimed_elsewhere(4, 1, &[Value::Int4(10)])
            .unwrap(),
        "the row still holding the vacated key goes on claiming it"
    );
    assert!(
        wrapper
            .unique_key_claimed_elsewhere(4, 1, &[Value::Int4(30)])
            .unwrap(),
        "the key it moved onto is claimed"
    );

    // A staged row that is deleted retracts too — and with the last holder gone, the
    // key is claimed by nobody.
    wrapper.delete_in_tx(3, "t", shares).unwrap();
    assert!(
        wrapper
            .index_lookup_in_tx(3, 1, &[Value::Int4(10)])
            .unwrap()
            .is_empty(),
        "a deleted staged row holds no key"
    );
    assert!(
        !wrapper
            .unique_key_claimed_elsewhere(4, 1, &[Value::Int4(10)])
            .unwrap(),
        "no staged row holds the key, so nothing claims it"
    );

    wrapper.rollback_transaction(3).unwrap();
}

/// Column DDL is the write the pending index cannot follow, so it routes the table to
/// the **scan** branch — the branch that re-encodes each buffered row. This is the
/// dropped-column arm; the added-column one is the test below.
///
/// A dropped column is what makes that routing load-bearing rather than cautious: it
/// shifts the positions a key encoding reads, so a staged row's entry in the pending
/// index describes a key the row no longer has. Only re-encoding at probe time gets
/// that right, and it also keeps the scan branch exercised now that the accelerator
/// answers the insert/update/delete shapes.
#[test]
fn drop_column_routes_probes_to_the_pending_scan() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    // Key on the **last** of three columns, so dropping the first shifts it away.
    wrapper
        .create_index(full_spec(1, "t", vec![2], true))
        .unwrap();

    wrapper.begin_transaction(5).unwrap();
    wrapper
        .insert_in_tx(5, "t", vec![Value::Int4(1), Value::Int4(2), Value::Int4(3)])
        .unwrap();
    assert_eq!(
        sorted(wrapper.index_lookup_in_tx(5, 1, &[Value::Int4(3)]).unwrap()).len(),
        1,
        "the staged row holds key 3 before the DDL"
    );

    wrapper.drop_column_in_tx(5, "t", 0).unwrap();

    assert_eq!(
        sorted(wrapper.index_lookup_in_tx(5, 1, &[Value::Int4(3)]).unwrap()),
        tx_scan_filter_ids(&wrapper, 5, "t", 2, &Value::Int4(3)),
        "after a dropped column the probe must equal the scan-filter ground truth"
    );
    assert!(
        wrapper
            .index_lookup_in_tx(5, 1, &[Value::Int4(3)])
            .unwrap()
            .is_empty(),
        "the key the row held under the old layout is gone with the column"
    );
    assert!(
        !wrapper
            .unique_key_claimed_elsewhere(6, 1, &[Value::Int4(3)])
            .unwrap(),
        "the claim walk reads the same re-encoding, so it claims nothing either"
    );

    wrapper.rollback_transaction(5).unwrap();
}

/// The `add_column_in_tx` half of that routing. An added column shifts no key position,
/// so a staged row's pending-index entry survives the DDL intact and the **committed**
/// rows it re-stages are what show the marking is load-bearing here: the pending index
/// never held their committed images, so unmarked, the fast path reads an empty map and
/// both readers lose them — a `UNIQUE` key silently free for a second insert.
#[test]
fn add_column_routes_committed_rows_to_the_pending_scan() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    // Key on the first column, so the seed rows below are committed rows the DDL
    // re-stages rather than rows already in the pending index.
    wrapper
        .create_index(full_spec(1, "t", vec![0], true))
        .unwrap();
    let committed = wrapper
        .insert("t", vec![Value::Int4(0), Value::Int4(0)])
        .unwrap();
    wrapper
        .insert("t", vec![Value::Int4(2), Value::Int4(2)])
        .unwrap();

    wrapper.begin_transaction(7).unwrap();
    wrapper.add_column_in_tx(7, "t", Value::Null).unwrap();

    assert_eq!(
        sorted(wrapper.index_lookup_in_tx(7, 1, &[Value::Int4(0)]).unwrap()),
        tx_scan_filter_ids(&wrapper, 7, "t", 0, &Value::Int4(0)),
        "after an added column the probe must equal the scan-filter ground truth"
    );
    assert_eq!(
        sorted(wrapper.index_lookup_in_tx(7, 1, &[Value::Int4(0)]).unwrap()),
        vec![committed],
        "the committed row the DDL re-staged still holds its key, so a second insert of \
         it is refused rather than silently doubling the row"
    );
    // The same fact through the other reader, and the branch answering **yes** rather
    // than only no.
    assert!(
        wrapper
            .unique_key_claimed_elsewhere(8, 1, &[Value::Int4(0)])
            .unwrap(),
        "the re-staged row claims its key against another connection too"
    );
    assert!(
        !wrapper
            .unique_key_claimed_elsewhere(8, 1, &[Value::Int4(9)])
            .unwrap(),
        "and claims nothing for a key no row holds"
    );

    wrapper.rollback_transaction(7).unwrap();
}

/// The insert and update `Err(_)` arms. `pending_btree_index_keys` derives a row's keys
/// as a **set** and fails as a set, so one index whose key fails to derive (a division by
/// the row's zero) would cost the fast path that row's enforced unique key too.
///
/// The delete arm is not covered here: every write that stages a row whose keys fail to
/// derive marks the table itself, so a probe reaching that arm's fast path first needs
/// an index registered after the staging.
#[test]
fn a_failed_key_derivation_routes_probes_to_the_pending_scan() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    // The enforced key, which encodes, beside a plain index whose key expression raises
    // for a zero — a failure of the row's value, not of its type, so no key type the
    // encoder learns to take can close it.
    wrapper
        .create_index(full_spec(1, "t", vec![0], true))
        .unwrap();
    let mut failing = full_spec(2, "t", Vec::new(), false);
    failing.key_part_ir = Some(vec![ir("10 / v")]);
    failing.column_names = vec!["k".into(), "v".into()];
    wrapper.create_index(failing).unwrap();

    // (a) The insert arm: the row's whole key set fails, so it stages no entry.
    wrapper.begin_transaction(11).unwrap();
    let staged = wrapper
        .insert_in_tx(11, "t", vec![Value::Int4(1), Value::Int4(0)])
        .unwrap();
    assert_eq!(
        sorted(
            wrapper
                .index_lookup_in_tx(11, 1, &[Value::Int4(1)])
                .unwrap()
        ),
        tx_scan_filter_ids(&wrapper, 11, "t", 0, &Value::Int4(1)),
        "a row whose key set fails to derive must still probe as the scan filter sees it"
    );
    assert_eq!(
        sorted(
            wrapper
                .index_lookup_in_tx(11, 1, &[Value::Int4(1)])
                .unwrap()
        ),
        vec![staged],
        "the staged row holds its enforced unique key"
    );
    assert!(
        wrapper
            .unique_key_claimed_elsewhere(12, 1, &[Value::Int4(1)])
            .unwrap(),
        "and claims it against another connection"
    );
    wrapper.rollback_transaction(11).unwrap();

    // (b) The update arm, over a row staged **without** a failure — a NULL key encodes
    // to no entry rather than to an error, so the table reaches the update unmarked.
    wrapper.begin_transaction(13).unwrap();
    let moved = wrapper
        .insert_in_tx(13, "t", vec![Value::Int4(1), Value::Null])
        .unwrap();
    assert_eq!(
        sorted(
            wrapper
                .index_lookup_in_tx(13, 1, &[Value::Int4(1)])
                .unwrap()
        ),
        vec![moved],
        "the fast path holds the row before the failing update"
    );
    wrapper
        .update_in_tx(13, "t", moved, vec![Value::Int4(2), Value::Int4(0)])
        .unwrap();
    assert!(
        wrapper
            .index_lookup_in_tx(13, 1, &[Value::Int4(1)])
            .unwrap()
            .is_empty(),
        "the key the row moved off must be released, not left frozen in the fast path"
    );
    assert_eq!(
        sorted(
            wrapper
                .index_lookup_in_tx(13, 1, &[Value::Int4(2)])
                .unwrap()
        ),
        tx_scan_filter_ids(&wrapper, 13, "t", 0, &Value::Int4(2)),
        "and the key it moved onto must be held"
    );
    wrapper.rollback_transaction(13).unwrap();
}

/// Ground truth for a range probe: row ids of `table` whose column 1 (`Int4`)
/// falls in the value range, via a committed scan + filter (NULL keys excluded).
fn scan_range_ids(
    wrapper: &BtreeStorageWrapper,
    lower: Option<(i32, bool)>,
    upper: Option<(i32, bool)>,
) -> Vec<RowId> {
    let mut ids: Vec<RowId> = wrapper
        .scan("t")
        .unwrap()
        .into_iter()
        .filter(|(_, row)| match row.get(1) {
            Some(Value::Int4(v)) => {
                let lo_ok = lower.is_none_or(|(b, incl)| if incl { *v >= b } else { *v > b });
                let hi_ok = upper.is_none_or(|(b, incl)| if incl { *v <= b } else { *v < b });
                lo_ok && hi_ok
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .collect();
    ids.sort_unstable();
    ids
}

#[test]
fn index_range_matches_scan_filter_for_each_op() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    // Duplicates and a NULL key (id 4) that every value range must exclude.
    let vals = [
        Some(30),
        Some(10),
        Some(20),
        Some(10),
        None,
        Some(40),
        Some(20),
    ];
    for (i, v) in vals.iter().enumerate() {
        let key = v.map_or(Value::Null, Value::Int4);
        wrapper
            .insert("t", vec![Value::Int4(i as i32), key])
            .unwrap();
    }
    wrapper
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();

    let probe = |lo: Option<(i32, bool)>, hi: Option<(i32, bool)>| -> Vec<RowId> {
        let l = lo.map(|(v, incl)| (Value::Int4(v), incl));
        let h = hi.map(|(v, incl)| (Value::Int4(v), incl));
        sorted(wrapper.index_range_lookup(1, l, h, true, None).unwrap())
    };

    // > 20, >= 20, < 20, <= 20, BETWEEN 10..30, fully unbounded — each a SET match.
    for (lo, hi) in [
        (Some((20, false)), None),
        (Some((20, true)), None),
        (None, Some((20, false))),
        (None, Some((20, true))),
        (Some((10, true)), Some((30, true))),
        (None, None),
    ] {
        assert_eq!(
            probe(lo, hi),
            scan_range_ids(&wrapper, lo, hi),
            "{lo:?}..{hi:?}"
        );
    }
    // NULL-keyed row never appears.
    assert!(
        !probe(None, None).contains(&4),
        "NULL key excluded from ranges"
    );
}

#[test]
fn index_range_order_and_limit() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    // Keys 50,10,30,20,40 at ids 0..5; ascending key order = ids [1,3,2,4,0].
    for (i, v) in [50, 10, 30, 20, 40].iter().enumerate() {
        wrapper
            .insert("t", vec![Value::Int4(i as i32), Value::Int4(*v)])
            .unwrap();
    }
    wrapper
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();

    let asc = wrapper
        .index_range_lookup(1, None, None, true, None)
        .unwrap();
    assert_eq!(asc, vec![1, 3, 2, 4, 0], "ascending = index-key order");
    let desc = wrapper
        .index_range_lookup(1, None, None, false, None)
        .unwrap();
    assert_eq!(desc, vec![0, 4, 2, 3, 1], "descending = reverse key order");

    // LIMIT: ascending = first-n in key order (early-stopped); descending = last-n.
    assert_eq!(
        wrapper
            .index_range_lookup(1, None, None, true, Some(2))
            .unwrap(),
        vec![1, 3]
    );
    assert_eq!(
        wrapper
            .index_range_lookup(1, None, None, false, Some(2))
            .unwrap(),
        vec![0, 4]
    );
}

#[test]
fn index_range_partial_index_only_satisfying_rows() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    // Column 2 is the predicate flag; only `flag = 1` rows are indexed.
    for (i, (k, flag)) in [(10, 1), (20, 0), (30, 1), (40, 0)].iter().enumerate() {
        wrapper
            .insert(
                "t",
                vec![Value::Int4(i as i32), Value::Int4(*k), Value::Int4(*flag)],
            )
            .unwrap();
    }
    let spec = IndexSpec {
        index_id: 1,
        name: "test_index".to_string(),
        table: "t".to_string(),
        key_col_positions: vec![1],
        key_part_ir: None,
        unique: false,
        predicate_ir: Some(ir("flag = 1")),
        nulls_not_distinct: false,
        column_names: vec!["id".into(), "k".into(), "flag".into()],
        column_types: Vec::new(),
        column_enum_orders: Vec::new(),
        method: IndexMethod::Btree,
    };
    wrapper.create_index(spec).unwrap();

    let got = sorted(
        wrapper
            .index_range_lookup(
                1,
                Some((Value::Int4(0), true)),
                Some((Value::Int4(100), true)),
                true,
                None,
            )
            .unwrap(),
    );
    assert_eq!(
        got,
        vec![0, 2],
        "partial index range skips non-satisfying rows"
    );
}

#[test]
fn index_range_is_mvcc_consistent_at_snapshot_root() {
    // A range probe at an old snapshot root sees the pre-insert index state;
    // the current root sees the new rows.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    for i in 0..4 {
        wrapper
            .insert("t", vec![Value::Int4(i), Value::Int4(10 * (i + 1))])
            .unwrap();
    }
    wrapper
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();

    // Snapshot the pre-insert state: keys 10,20,30,40 → ids 0,1,2,3.
    let snap = wrapper.read_snapshot();
    // The snapshot's inherent range method borrows its bound (`>= 15`).
    let bound = Value::Int4(15);
    let before = sorted(
        snap.index_range_lookup(1, Some((&bound, true)), None, true, None)
            .unwrap(),
    );
    assert_eq!(before, vec![1, 2, 3], ">= 15 over the snapshot");

    // Add a new in-range row and commit (new root).
    wrapper
        .insert("t", vec![Value::Int4(4), Value::Int4(25)])
        .unwrap();

    // Old snapshot is unchanged...
    assert_eq!(
        sorted(
            snap.index_range_lookup(1, Some((&bound, true)), None, true, None)
                .unwrap()
        ),
        before,
        "snapshot range probe must see the old index state"
    );
    // ...current root sees the new row too.
    let now = sorted(
        wrapper
            .index_range_lookup(1, Some((Value::Int4(15), true)), None, true, None)
            .unwrap(),
    );
    assert_eq!(
        now,
        vec![1, 2, 3, 4],
        "current-root range probe sees the new row"
    );
}

/// Ground truth for an in-tx range probe: ids whose column 1 (`Int4`) is in the
/// value range, via the read-your-own-writes `scan_in_tx`.
fn tx_scan_range_ids(
    wrapper: &BtreeStorageWrapper,
    conn_id: i32,
    lower: Option<(i32, bool)>,
    upper: Option<(i32, bool)>,
) -> Vec<RowId> {
    let mut ids: Vec<RowId> = wrapper
        .scan_in_tx(conn_id, "t")
        .unwrap()
        .into_iter()
        .filter(|(_, row)| match row.get(1) {
            Some(Value::Int4(v)) => {
                let lo_ok = lower.is_none_or(|(b, incl)| if incl { *v >= b } else { *v > b });
                let hi_ok = upper.is_none_or(|(b, incl)| if incl { *v <= b } else { *v < b });
                lo_ok && hi_ok
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .collect();
    ids.sort_unstable();
    ids
}

#[test]
fn index_range_lookup_in_tx_reflects_uncommitted_writes() {
    // RYOW for ranges: the in-tx range probe must equal the in-tx scan-filter
    // through insert/update/delete, all uncommitted.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    // Committed seed: keys 10,20,30 at ids 0,1,2.
    for (i, v) in [10, 20, 30].iter().enumerate() {
        wrapper
            .insert("t", vec![Value::Int4(i as i32), Value::Int4(*v)])
            .unwrap();
    }
    wrapper
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();

    wrapper.begin_transaction(9).unwrap();
    // `>= 15` lower bound; borrow it fresh at each in-tx call (the inherent
    // method borrows, unlike the by-value committed-trait method).
    let lo_val = Value::Int4(15);
    let lo_ref = || Some((&lo_val, true));

    // (a) Uncommitted INSERT of an in-range key (25 at id 3).
    let inserted = wrapper
        .insert_in_tx(9, "t", vec![Value::Int4(3), Value::Int4(25)])
        .unwrap();
    let probe = sorted(
        wrapper
            .index_range_lookup_in_tx(9, 1, lo_ref(), None, true, None)
            .unwrap(),
    );
    assert_eq!(
        probe,
        tx_scan_range_ids(&wrapper, 9, Some((15, true)), None)
    );
    assert!(
        probe.contains(&inserted),
        "uncommitted insert is in the range"
    );
    // The committed range probe still does NOT see it.
    assert!(
        !sorted(
            wrapper
                .index_range_lookup(1, Some((Value::Int4(15), true)), None, true, None)
                .unwrap()
        )
        .contains(&inserted),
        "committed range probe must not see the uncommitted row"
    );

    // (b) Uncommitted UPDATE moving id 0 (key 10) into range (key 40).
    wrapper
        .update_in_tx(9, "t", 0, vec![Value::Int4(0), Value::Int4(40)])
        .unwrap();
    assert_eq!(
        sorted(
            wrapper
                .index_range_lookup_in_tx(9, 1, lo_ref(), None, true, None)
                .unwrap()
        ),
        tx_scan_range_ids(&wrapper, 9, Some((15, true)), None)
    );

    // (c) Uncommitted DELETE of id 2 (key 30, in range) is respected.
    wrapper.delete_in_tx(9, "t", 2).unwrap();
    let probe = sorted(
        wrapper
            .index_range_lookup_in_tx(9, 1, lo_ref(), None, true, None)
            .unwrap(),
    );
    assert_eq!(
        probe,
        tx_scan_range_ids(&wrapper, 9, Some((15, true)), None)
    );
    assert!(
        !probe.contains(&2),
        "uncommitted delete drops from the range"
    );

    // In-tx descending order is the reverse of ascending.
    let asc = wrapper
        .index_range_lookup_in_tx(9, 1, None, None, true, None)
        .unwrap();
    let mut desc = wrapper
        .index_range_lookup_in_tx(9, 1, None, None, false, None)
        .unwrap();
    desc.reverse();
    assert_eq!(asc, desc, "in-tx desc == reverse of asc");

    wrapper.rollback_transaction(9).unwrap();
}

/// [`full_spec`] under `NULLS NOT DISTINCT`: the one B-tree shape that holds an entry
/// for a NULL key.
fn nulls_not_distinct_spec(
    index_id: u32,
    table: &str,
    positions: Vec<usize>,
    unique: bool,
) -> IndexSpec {
    IndexSpec {
        nulls_not_distinct: true,
        ..full_spec(index_id, table, positions, unique)
    }
}

/// One end of a range read over `Int4` keys: the bound and whether the range includes
/// it, or `None` for no bound that side.
type RangeEnd = Option<(i32, bool)>;

/// Every bound shape a range read takes over `Int4` keys around `pivots`: no bound,
/// each one-sided comparison, and each two-sided pair of them that names an interval.
fn range_bound_shapes(pivots: &[i32]) -> Vec<(RangeEnd, RangeEnd)> {
    let mut ends: Vec<RangeEnd> = vec![None];
    for &pivot in pivots {
        ends.push(Some((pivot, true)));
        ends.push(Some((pivot, false)));
    }
    let mut shapes = Vec::new();
    for lower in &ends {
        for upper in &ends {
            if let (Some((lo, _)), Some((hi, _))) = (lower, upper)
                && lo >= hi
            {
                continue;
            }
            shapes.push((*lower, *upper));
        }
    }
    shapes
}

/// What a range read over column 1 of `rows` answers: the rows whose key is a non-NULL
/// value within the bounds, in `(key, row id)` order, reversed when descending, cut to
/// `limit`.
fn expected_range_ids(
    rows: &[(RowId, Vec<Value>)],
    lower: RangeEnd,
    upper: RangeEnd,
    ascending: bool,
    limit: Option<usize>,
) -> Vec<RowId> {
    let mut hits: Vec<(i32, RowId)> = rows
        .iter()
        .filter_map(|(id, row)| {
            let Some(Value::Int4(v)) = row.get(1) else {
                return None;
            };
            let lo_ok = lower.is_none_or(|(b, incl)| if incl { *v >= b } else { *v > b });
            let hi_ok = upper.is_none_or(|(b, incl)| if incl { *v <= b } else { *v < b });
            (lo_ok && hi_ok).then_some((*v, *id))
        })
        .collect();
    hits.sort_unstable();
    let mut ids: Vec<RowId> = hits.into_iter().map(|(_, id)| id).collect();
    if !ascending {
        ids.reverse();
    }
    if let Some(n) = limit {
        ids.truncate(n);
    }
    ids
}

#[test]
fn index_range_leaves_out_the_null_keys_a_nulls_not_distinct_index_holds() {
    // A comparison is not true of a NULL, so no range read answers a NULL-keyed row —
    // at the current root and at a snapshot's, unique index and not.
    for unique in [false, true] {
        let keys: &[Option<i32>] = if unique {
            &[Some(30), None, Some(10), Some(-5), Some(40), Some(20)]
        } else {
            &[Some(30), None, Some(10), Some(-5), None, Some(40), Some(10)]
        };
        let mut wrapper = BtreeStorageWrapper::new();
        wrapper.create_table("t").unwrap();
        for (i, key) in keys.iter().enumerate() {
            wrapper
                .insert(
                    "t",
                    vec![Value::Int4(i as i32), key.map_or(Value::Null, Value::Int4)],
                )
                .unwrap();
        }
        wrapper
            .create_index(nulls_not_distinct_spec(1, "t", vec![1], unique))
            .unwrap();
        let rows = wrapper.scan("t").unwrap();
        // The premise: this index does hold its NULL-keyed rows, under a NULL key.
        let null_keyed: Vec<RowId> = rows
            .iter()
            .filter(|(_, row)| row[1] == Value::Null)
            .map(|(id, _)| *id)
            .collect();
        assert!(!null_keyed.is_empty());
        assert_eq!(
            sorted(wrapper.index_lookup(1, &[Value::Null]).unwrap()),
            null_keyed,
            "unique={unique}: the index holds the NULL-keyed rows"
        );

        let snapshot = wrapper.read_snapshot();
        for (lower, upper) in range_bound_shapes(&[-5, 10, 25, 99]) {
            for ascending in [true, false] {
                for limit in [None, Some(1), Some(3)] {
                    let label = format!(
                        "unique={unique} {lower:?}..{upper:?} ascending={ascending} limit={limit:?}"
                    );
                    let want = expected_range_ids(&rows, lower, upper, ascending, limit);
                    let lo = lower.map(|(v, incl)| (Value::Int4(v), incl));
                    let hi = upper.map(|(v, incl)| (Value::Int4(v), incl));
                    assert_eq!(
                        snapshot
                            .index_range_lookup(
                                1,
                                lo.as_ref().map(|(v, incl)| (v, *incl)),
                                hi.as_ref().map(|(v, incl)| (v, *incl)),
                                ascending,
                                limit,
                            )
                            .unwrap(),
                        want,
                        "snapshot: {label}"
                    );
                    assert_eq!(
                        wrapper
                            .index_range_lookup(1, lo, hi, ascending, limit)
                            .unwrap(),
                        want,
                        "current root: {label}"
                    );
                }
            }
        }
        // A NULL bound is no bound that side, and brings no NULL key with it.
        for ascending in [true, false] {
            assert_eq!(
                wrapper
                    .index_range_lookup(
                        1,
                        Some((Value::Null, true)),
                        Some((Value::Int4(25), false)),
                        ascending,
                        None,
                    )
                    .unwrap(),
                expected_range_ids(&rows, None, Some((25, false)), ascending, None),
                "unique={unique}: a NULL lower bound"
            );
        }
    }
}

#[test]
fn index_range_in_tx_leaves_out_null_keys_committed_and_pending() {
    // The in-transaction range read answers from the pending-merged rows rather than the
    // committed index; it takes the same bounds, so a NULL-keyed row is in no range
    // whether it is committed, this transaction's insert, or a row this transaction
    // moved onto a NULL key — and a row moved off one is in.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    for (i, key) in [Some(30), None, Some(10), None, Some(-5)]
        .iter()
        .enumerate()
    {
        wrapper
            .insert(
                "t",
                vec![Value::Int4(i as i32), key.map_or(Value::Null, Value::Int4)],
            )
            .unwrap();
    }
    wrapper
        .create_index(nulls_not_distinct_spec(1, "t", vec![1], false))
        .unwrap();

    let conn = 9;
    wrapper.begin_transaction(conn).unwrap();
    let check = |wrapper: &BtreeStorageWrapper, stage: &str| {
        let rows = wrapper.scan_in_tx(conn, "t").unwrap();
        assert!(
            rows.iter().any(|(_, row)| row[1] == Value::Null),
            "{stage}: a NULL-keyed row is visible"
        );
        for (lower, upper) in range_bound_shapes(&[-5, 10, 25, 99]) {
            for ascending in [true, false] {
                for limit in [None, Some(1), Some(3)] {
                    let lo = lower.map(|(v, incl)| (Value::Int4(v), incl));
                    let hi = upper.map(|(v, incl)| (Value::Int4(v), incl));
                    assert_eq!(
                        wrapper
                            .index_range_lookup_in_tx(
                                conn,
                                1,
                                lo.as_ref().map(|(v, incl)| (v, *incl)),
                                hi.as_ref().map(|(v, incl)| (v, *incl)),
                                ascending,
                                limit,
                            )
                            .unwrap(),
                        expected_range_ids(&rows, lower, upper, ascending, limit),
                        "{stage}: {lower:?}..{upper:?} ascending={ascending} limit={limit:?}"
                    );
                }
            }
        }
    };
    check(&wrapper, "committed rows only");
    wrapper
        .insert_in_tx(conn, "t", vec![Value::Int4(5), Value::Null])
        .unwrap();
    wrapper
        .insert_in_tx(conn, "t", vec![Value::Int4(6), Value::Int4(20)])
        .unwrap();
    check(
        &wrapper,
        "a pending NULL-keyed insert and a pending present one",
    );
    // Row 0 (key 30) onto a NULL key; row 1 (a NULL key) onto 15.
    wrapper
        .update_in_tx(conn, "t", 0, vec![Value::Int4(0), Value::Null])
        .unwrap();
    wrapper
        .update_in_tx(conn, "t", 1, vec![Value::Int4(1), Value::Int4(15)])
        .unwrap();
    check(&wrapper, "a row moved onto a NULL key and one moved off");
    wrapper.delete_in_tx(conn, "t", 3).unwrap();
    check(&wrapper, "a committed NULL-keyed row deleted");
    wrapper.commit_transaction(conn).unwrap();

    // Committed, the index walk answers what the in-transaction read did.
    let rows = wrapper.scan("t").unwrap();
    for (lower, upper) in range_bound_shapes(&[-5, 10, 25, 99]) {
        for ascending in [true, false] {
            assert_eq!(
                wrapper
                    .index_range_lookup(
                        1,
                        lower.map(|(v, incl)| (Value::Int4(v), incl)),
                        upper.map(|(v, incl)| (Value::Int4(v), incl)),
                        ascending,
                        None,
                    )
                    .unwrap(),
                expected_range_ids(&rows, lower, upper, ascending, None),
                "after commit: {lower:?}..{upper:?} ascending={ascending}"
            );
        }
    }
}

#[test]
fn index_range_with_no_lower_bound_holds_the_least_present_key() {
    // An empty `text` is the least key a present value has — one byte, the presence tag.
    // A range with no lower bound starts there: above the NULL key, and not past `''`.
    for nulls_not_distinct in [false, true] {
        let mut wrapper = BtreeStorageWrapper::new();
        wrapper.create_table("t").unwrap();
        for (i, key) in [Some("a"), None, Some(""), Some("b")].iter().enumerate() {
            wrapper
                .insert(
                    "t",
                    vec![
                        Value::Int4(i as i32),
                        key.map_or(Value::Null, |k| Value::Text(k.to_string())),
                    ],
                )
                .unwrap();
        }
        wrapper
            .create_index(IndexSpec {
                nulls_not_distinct,
                ..full_spec(1, "t", vec![1], false)
            })
            .unwrap();
        let below = |upper: &str, inclusive: bool, ascending: bool| {
            wrapper
                .index_range_lookup(
                    1,
                    None,
                    Some((Value::Text(upper.to_string()), inclusive)),
                    ascending,
                    None,
                )
                .unwrap()
        };
        let label = format!("nulls_not_distinct={nulls_not_distinct}");
        assert_eq!(below("b", false, true), vec![2, 0], "{label}: < 'b'");
        assert_eq!(below("b", false, false), vec![0, 2], "{label}: < 'b' desc");
        assert_eq!(below("", true, true), vec![2], "{label}: <= ''");
        assert_eq!(below("", false, true), Vec::<RowId>::new(), "{label}: < ''");
        assert_eq!(
            wrapper
                .index_range_lookup(1, None, None, true, None)
                .unwrap(),
            vec![2, 0, 3],
            "{label}: every non-NULL key, in key order"
        );
    }
}

#[test]
fn index_range_over_a_composite_key_leaves_out_a_null_leading_part() {
    // The lower end is the least key whose leading part is present, so the rule holds
    // by the key's leading tag and not by the index having one column.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    let int = |v: Option<i32>| v.map_or(Value::Null, Value::Int4);
    for (a, b) in [
        (None, Some(1)),
        (Some(1), None),
        (None, None),
        (Some(i32::MIN), Some(2)),
        (Some(1), Some(2)),
    ] {
        wrapper.insert("t", vec![int(a), int(b)]).unwrap();
    }
    wrapper
        .create_index(nulls_not_distinct_spec(1, "t", vec![0, 1], true))
        .unwrap();
    assert_eq!(
        sorted(
            wrapper
                .index_lookup(1, &[Value::Null, Value::Null])
                .unwrap()
        ),
        vec![2],
        "the index holds the all-NULL key"
    );
    assert_eq!(
        wrapper
            .index_range_lookup(1, None, None, true, None)
            .unwrap(),
        vec![3, 1, 4],
        "the keys whose leading part is present, in key order"
    );
    wrapper.begin_transaction(4).unwrap();
    wrapper
        .insert_in_tx(4, "t", vec![Value::Null, Value::Int4(9)])
        .unwrap();
    assert_eq!(
        wrapper
            .index_range_lookup_in_tx(4, 1, None, None, true, None)
            .unwrap(),
        vec![3, 1, 4],
        "in a transaction, with a pending NULL-leading key"
    );
    wrapper.rollback_transaction(4).unwrap();
}

#[test]
fn index_range_unknown_index_errors() {
    let wrapper = BtreeStorageWrapper::new();
    let err = wrapper
        .index_range_lookup(999, None, None, true, None)
        .unwrap_err();
    assert!(matches!(&err, pglet_core::Error::Execution(m) if m == "index not found: 999"));
}

#[test]
fn index_lookup_unknown_index_errors_and_null_is_empty() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper
        .insert("t", vec![Value::Int4(0), Value::Int4(1)])
        .unwrap();
    wrapper
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();

    // Unknown index id is a loud error, not an empty result (mapped to
    // `pglet_core::Error::Execution` at the trait boundary).
    let err = wrapper.index_lookup(999, &[Value::Int4(1)]).unwrap_err();
    assert!(matches!(&err, pglet_core::Error::Execution(m) if m == "index not found: 999"));
    // NULL never matches equality.
    assert!(wrapper.index_lookup(1, &[Value::Null]).unwrap().is_empty());

    // drop_index removes the entries and the def.
    wrapper.drop_index(1).unwrap();
    let err = wrapper.index_lookup(1, &[Value::Int4(1)]).unwrap_err();
    assert!(matches!(&err, pglet_core::Error::Execution(m) if m == "index not found: 1"));
}

#[test]
fn test_multiple_concurrent_snapshots() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("users").unwrap();
    wrapper.insert("users", vec![Value::Int4(1)]).unwrap();

    // Take first snapshot
    let snapshot1 = wrapper.read_snapshot();

    // Add more data
    wrapper.insert("users", vec![Value::Int4(2)]).unwrap();

    // Take second snapshot
    let snapshot2 = wrapper.read_snapshot();

    // Add even more data
    wrapper.insert("users", vec![Value::Int4(3)]).unwrap();

    // Each snapshot sees different data
    assert_eq!(snapshot1.scan("users").unwrap().len(), 1);
    assert_eq!(snapshot2.scan("users").unwrap().len(), 2);
    assert_eq!(wrapper.scan("users").unwrap().len(), 3);

    // Both snapshots active
    let stats = wrapper.stats();
    assert_eq!(stats.active_snapshots, 2);
}

// ========================================================================
// Partial / NULLS NOT DISTINCT index maintenance
// ========================================================================

/// A partial-index spec over `t(id, path, deleted)` keyed on `path` (pos 1)
/// with predicate `deleted IS NULL` — the soft-delete shape.
fn partial_spec(index_id: u32) -> IndexSpec {
    IndexSpec {
        index_id,
        name: "test_index".to_string(),
        table: "t".to_string(),
        key_col_positions: vec![1],
        key_part_ir: None,
        unique: true,
        predicate_ir: Some(ir("deleted IS NULL")),
        nulls_not_distinct: false,
        column_names: vec!["id".into(), "path".into(), "deleted".into()],
        column_types: Vec::new(),
        column_enum_orders: Vec::new(),
        method: IndexMethod::Btree,
    }
}

#[test]
fn partial_index_only_holds_predicate_satisfying_rows() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    // Active row (deleted = NULL) IS indexed.
    wrapper
        .insert(
            "t",
            vec![Value::Int4(0), Value::Text("/a".into()), Value::Null],
        )
        .unwrap();
    // Soft-deleted row (deleted = 'x') is NOT indexed even with same path.
    wrapper
        .insert(
            "t",
            vec![
                Value::Int4(1),
                Value::Text("/a".into()),
                Value::Text("x".into()),
            ],
        )
        .unwrap();
    wrapper.create_index(partial_spec(1)).unwrap();

    // Only the active row appears under '/a'.
    assert_eq!(
        wrapper
            .index_lookup(1, &[Value::Text("/a".into())])
            .unwrap(),
        vec![0]
    );
}

#[test]
fn partial_index_update_moves_row_in_and_out() {
    // The asymmetric-UPDATE trap: a row enters the partial index when an update
    // makes it satisfy the predicate, and leaves when an update makes it fail.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    // Start soft-deleted (not in the index).
    let id = wrapper
        .insert(
            "t",
            vec![
                Value::Int4(0),
                Value::Text("/p".into()),
                Value::Text("d".into()),
            ],
        )
        .unwrap();
    wrapper.create_index(partial_spec(1)).unwrap();
    assert!(
        wrapper
            .index_lookup(1, &[Value::Text("/p".into())])
            .unwrap()
            .is_empty(),
        "soft-deleted row is not indexed"
    );

    // UPDATE un-deletes it (deleted → NULL): it ENTERS the partial index.
    wrapper
        .update(
            "t",
            id,
            vec![Value::Int4(0), Value::Text("/p".into()), Value::Null],
        )
        .unwrap();
    assert_eq!(
        wrapper
            .index_lookup(1, &[Value::Text("/p".into())])
            .unwrap(),
        vec![id],
        "row entered the index on update into the predicate"
    );

    // UPDATE re-deletes it (deleted → 'd'): it LEAVES the partial index. The
    // asymmetric delete must fire only because the OLD row was in the index.
    wrapper
        .update(
            "t",
            id,
            vec![
                Value::Int4(0),
                Value::Text("/p".into()),
                Value::Text("d".into()),
            ],
        )
        .unwrap();
    assert!(
        wrapper
            .index_lookup(1, &[Value::Text("/p".into())])
            .unwrap()
            .is_empty(),
        "row left the index on update out of the predicate"
    );
}

#[test]
fn partial_index_ryow_sees_uncommitted_active_insert() {
    // RYOW for a partial index: an uncommitted insert that satisfies the
    // predicate is visible to the same-txn probe; one that fails it is not.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper.create_index(partial_spec(1)).unwrap();

    wrapper.begin_transaction(9).unwrap();
    // Active insert (deleted = NULL) — satisfies the predicate.
    let active = wrapper
        .insert_in_tx(
            9,
            "t",
            vec![Value::Int4(0), Value::Text("/q".into()), Value::Null],
        )
        .unwrap();
    // Soft-deleted insert with the SAME path — fails the predicate.
    let _deleted = wrapper
        .insert_in_tx(
            9,
            "t",
            vec![
                Value::Int4(1),
                Value::Text("/q".into()),
                Value::Text("d".into()),
            ],
        )
        .unwrap();

    let probe = wrapper
        .index_lookup_in_tx(9, 1, &[Value::Text("/q".into())])
        .unwrap();
    assert_eq!(
        probe,
        vec![active],
        "only the predicate-satisfying uncommitted row is probed"
    );
    wrapper.rollback_transaction(9).unwrap();
}

// ========================================================================
// Expression-key index maintenance + probe
// ========================================================================

/// A unique expression-key spec over `t(id, email)` keyed on `LOWER(email)`.
fn lower_email_spec(index_id: u32) -> IndexSpec {
    IndexSpec {
        index_id,
        name: "test_index".to_string(),
        table: "t".to_string(),
        // Positions unused for an expression index.
        key_col_positions: Vec::new(),
        key_part_ir: Some(vec![ir("LOWER(email)")]),
        unique: true,
        predicate_ir: None,
        nulls_not_distinct: false,
        column_names: vec!["id".into(), "email".into()],
        column_types: Vec::new(),
        column_enum_orders: Vec::new(),
        method: IndexMethod::Btree,
    }
}

#[test]
fn expression_index_keys_on_the_computed_value() {
    // The index keys on LOWER(email): 'A@x' and 'a@x' fold to the same key, so
    // a probe with either casing finds both rows; a distinct lowercased key
    // finds only its row.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    let r0 = wrapper
        .insert("t", vec![Value::Int4(0), Value::Text("A@x".into())])
        .unwrap();
    let r1 = wrapper
        .insert("t", vec![Value::Int4(1), Value::Text("a@x".into())])
        .unwrap();
    let r2 = wrapper
        .insert("t", vec![Value::Int4(2), Value::Text("b@x".into())])
        .unwrap();
    wrapper.create_index(lower_email_spec(1)).unwrap();

    // Probe is on the COMPUTED key (a lowercased text value), so 'a@x' matches
    // both the 'A@x' and 'a@x' rows.
    assert_eq!(
        sorted(
            wrapper
                .index_lookup(1, &[Value::Text("a@x".into())])
                .unwrap()
        ),
        sorted(vec![r0, r1])
    );
    // The distinct key finds only its row.
    assert_eq!(
        wrapper
            .index_lookup(1, &[Value::Text("b@x".into())])
            .unwrap(),
        vec![r2]
    );
    // A key present in no row returns nothing.
    assert!(
        wrapper
            .index_lookup(1, &[Value::Text("c@x".into())])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn expression_index_update_moves_computed_key() {
    // An UPDATE that changes the SOURCE column moves the entry to the new
    // COMPUTED key: the maintenance delete/insert must use the same eval as the
    // probe, or the old entry would dangle.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    let id = wrapper
        .insert("t", vec![Value::Int4(0), Value::Text("Old@x".into())])
        .unwrap();
    wrapper.create_index(lower_email_spec(1)).unwrap();

    // Initially under the lowercased 'old@x'.
    assert_eq!(
        wrapper
            .index_lookup(1, &[Value::Text("old@x".into())])
            .unwrap(),
        vec![id]
    );

    // UPDATE the source email; the computed key moves to 'new@x'.
    wrapper
        .update("t", id, vec![Value::Int4(0), Value::Text("New@x".into())])
        .unwrap();
    assert!(
        wrapper
            .index_lookup(1, &[Value::Text("old@x".into())])
            .unwrap()
            .is_empty(),
        "old computed key must be vacated"
    );
    assert_eq!(
        wrapper
            .index_lookup(1, &[Value::Text("new@x".into())])
            .unwrap(),
        vec![id],
        "row now found under the new computed key"
    );
}

#[test]
fn nulls_not_distinct_indexes_null_keys() {
    // Under NULLS NOT DISTINCT, two NULL keys fold to the same entry and a NULL
    // probe finds them; under default NULLS DISTINCT they'd be unindexed.
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    let r0 = wrapper
        .insert("t", vec![Value::Int4(0), Value::Null])
        .unwrap();
    let r1 = wrapper
        .insert("t", vec![Value::Int4(1), Value::Null])
        .unwrap();
    let spec = IndexSpec {
        index_id: 1,
        name: "test_index".to_string(),
        table: "t".to_string(),
        key_col_positions: vec![1],
        key_part_ir: None,
        unique: true,
        predicate_ir: None,
        nulls_not_distinct: true,
        column_names: vec!["id".into(), "k".into()],
        column_types: Vec::new(),
        column_enum_orders: Vec::new(),
        method: IndexMethod::Btree,
    };
    wrapper.create_index(spec).unwrap();

    // A NULL probe finds BOTH NULL-keyed rows (they share the empty key blob).
    let mut hits = wrapper.index_lookup(1, &[Value::Null]).unwrap();
    hits.sort_unstable();
    assert_eq!(hits, vec![r0, r1]);
}

// --- GIN (`@>` containment) ---------------------------------------------

/// A non-partial GIN spec over a single array/JSONB column at `col`.
fn gin_spec(index_id: u32, table: &str, col: usize, col_names: &[&str]) -> IndexSpec {
    IndexSpec {
        index_id,
        name: "test_index".to_string(),
        table: table.to_string(),
        key_col_positions: vec![col],
        key_part_ir: None,
        unique: false,
        predicate_ir: None,
        nulls_not_distinct: false,
        column_names: col_names.iter().map(|s| (*s).to_string()).collect(),
        column_types: Vec::new(),
        column_enum_orders: Vec::new(),
        method: IndexMethod::Gin,
    }
}

/// A `text[]` value.
fn tarr(elems: &[&str]) -> Value {
    Value::Array(pglet_sql::types::ArrayValue::new(
        pglet_sql::types::TypeOid::Text,
        elems
            .iter()
            .map(|s| Value::Text((*s).to_string()))
            .collect(),
    ))
}

fn txt(s: &str) -> Value {
    Value::Text(s.to_string())
}

#[test]
fn gin_lookup_intersects_posting_lists_and_maintains_through_writes() {
    let mut w = BtreeStorageWrapper::new();
    w.create_table("t").unwrap();
    // Insert BEFORE create_index so back-fill is exercised too.
    let r0 = w
        .insert("t", vec![Value::Int4(1), tarr(&["a", "b"])])
        .unwrap();
    let r1 = w.insert("t", vec![Value::Int4(2), tarr(&["c"])]).unwrap();
    let r2 = w
        .insert("t", vec![Value::Int4(3), tarr(&["a", "c"])])
        .unwrap();
    w.create_index(gin_spec(1, "t", 1, &["id", "refs"]))
        .unwrap();

    // Single-key probes == element membership.
    assert_eq!(w.gin_lookup(1, &[txt("a")]).unwrap(), vec![r0, r2]);
    assert_eq!(w.gin_lookup(1, &[txt("c")]).unwrap(), vec![r1, r2]);
    // Multi-key probe == intersection (contains ALL keys).
    assert_eq!(w.gin_lookup(1, &[txt("a"), txt("c")]).unwrap(), vec![r2]);
    // A miss and the empty-keys contract (caller full-scans `@> {}`).
    assert!(w.gin_lookup(1, &[txt("z")]).unwrap().is_empty());
    assert!(w.gin_lookup(1, &[]).unwrap().is_empty());

    // INSERT after create is maintained.
    let r3 = w.insert("t", vec![Value::Int4(4), tarr(&["a"])]).unwrap();
    assert_eq!(w.gin_lookup(1, &[txt("a")]).unwrap(), vec![r0, r2, r3]);

    // UPDATE rewrites the row's element entries: r1 {c} -> {a,d}.
    w.update("t", r1, vec![Value::Int4(2), tarr(&["a", "d"])])
        .unwrap();
    assert_eq!(
        w.gin_lookup(1, &[txt("c")]).unwrap(),
        vec![r2],
        "r1 left 'c'"
    );
    assert_eq!(
        w.gin_lookup(1, &[txt("a")]).unwrap(),
        vec![r0, r1, r2, r3],
        "r1 joined 'a'"
    );

    // DELETE removes every one of the row's entries.
    w.delete("t", r0).unwrap();
    assert_eq!(w.gin_lookup(1, &[txt("a")]).unwrap(), vec![r1, r2, r3]);

    // Unknown index errors loud.
    assert!(w.gin_lookup(99, &[txt("a")]).is_err());
}

#[test]
fn gin_lookup_on_jsonb_is_a_sound_superset() {
    use pglet_sql::types::JsonbValue;
    let mut w = BtreeStorageWrapper::new();
    w.create_table("t").unwrap();
    let obj = |pairs: &[(&str, &str)]| {
        Value::Jsonb(JsonbValue::Object(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), JsonbValue::String((*v).to_string())))
                .collect(),
        ))
    };
    let r0 = w
        .insert("t", vec![Value::Int4(1), obj(&[("kind", "note")])])
        .unwrap();
    let r1 = w
        .insert(
            "t",
            vec![Value::Int4(2), obj(&[("kind", "task"), ("k2", "note")])],
        )
        .unwrap();
    w.create_index(gin_spec(1, "t", 1, &["id", "data"]))
        .unwrap();

    // `data @> {"kind":"note"}` decomposes to {k:kind, vs:note}. r0 matches
    // exactly; r1 has both the key "kind" and the value "note" (under k2) so it
    // is a lossy candidate — proving the recheck is load-bearing.
    let kind_note = pglet_sql::gin::gin_decompose(&obj(&[("kind", "note")]));
    let got = w.gin_lookup(1, &kind_note).unwrap();
    assert!(got.contains(&r0), "true match present");
    assert!(
        got.contains(&r1),
        "lossy candidate present (recheck rejects later)"
    );
}

#[test]
fn gin_lookup_partial_index_only_indexes_satisfying_rows() {
    let mut w = BtreeStorageWrapper::new();
    w.create_table("t").unwrap();
    // (id, refs, deleted)
    let r0 = w
        .insert("t", vec![Value::Int4(1), tarr(&["a"]), Value::Bool(false)])
        .unwrap();
    let _r1 = w
        .insert("t", vec![Value::Int4(2), tarr(&["a"]), Value::Bool(true)])
        .unwrap();
    let mut spec = gin_spec(1, "t", 1, &["id", "refs", "deleted"]);
    spec.predicate_ir = Some(ir("deleted = false"));
    w.create_index(spec).unwrap();

    // Only the non-deleted row is indexed.
    assert_eq!(w.gin_lookup(1, &[txt("a")]).unwrap(), vec![r0]);
}

#[test]
fn gin_lookup_snapshot_is_mvcc_consistent() {
    let mut w = BtreeStorageWrapper::new();
    w.create_table("t").unwrap();
    let r0 = w.insert("t", vec![Value::Int4(1), tarr(&["a"])]).unwrap();
    w.create_index(gin_spec(1, "t", 1, &["id", "refs"]))
        .unwrap();

    let snap = w.read_snapshot();
    // A write after the snapshot must not be visible through it.
    w.insert("t", vec![Value::Int4(2), tarr(&["a"])]).unwrap();
    assert_eq!(snap.gin_lookup(1, &[txt("a")]).unwrap(), vec![r0]);
    // ...but the live committed probe sees both.
    assert_eq!(w.gin_lookup(1, &[txt("a")]).unwrap().len(), 2);
}

// ============================================================================
// Cross-connection unique-key claims
//
// The storage half of `EDGES.md` row 98, at the level the server guards in
// `pglet_tests`' `test_concurrent` cannot reach cheaply: the walk itself, its
// self-exclusion, and the aborted-block arms. The SQL layer decides *which*
// indexes ask; these decide what the answer is.
// ============================================================================

/// A key another connection has staged is claimed; a connection's own staged key
/// is not claimed against itself; and committing or rolling back releases it.
#[test]
fn unique_key_claim_sees_other_connections_and_not_itself() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper
        .create_index(full_spec(1, "t", vec![0], true))
        .unwrap();

    // Nothing staged anywhere: no claim.
    assert!(
        !wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(7)])
            .unwrap()
    );

    wrapper.begin_transaction(1).unwrap();
    wrapper
        .insert_in_tx(1, "t", vec![Value::Int4(7), Value::Int4(70)])
        .unwrap();

    // Connection 2 sees connection 1's staged key…
    assert!(
        wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(7)])
            .unwrap(),
        "another connection's staged key must be claimed"
    );
    // …and connection 1 does not see its own. The row-id exclusion on the read side
    // hides a self-claim behind a self-row, so this is the only place it shows.
    assert!(
        !wrapper
            .unique_key_claimed_elsewhere(1, 1, &[Value::Int4(7)])
            .unwrap(),
        "a connection must not claim a key against itself"
    );
    // A key nobody staged is free either way.
    assert!(
        !wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(8)])
            .unwrap()
    );

    // ROLLBACK releases: the staged write stops existing, so the claim does too.
    wrapper.rollback_transaction(1).unwrap();
    assert!(
        !wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(7)])
            .unwrap(),
        "ROLLBACK must release the claim"
    );

    // COMMIT releases the *claim* — the key is then held by a committed row instead,
    // which is the read probe's business and not this one's.
    wrapper.begin_transaction(1).unwrap();
    wrapper
        .insert_in_tx(1, "t", vec![Value::Int4(9), Value::Int4(90)])
        .unwrap();
    wrapper.commit_transaction(1).unwrap();
    assert!(
        !wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(9)])
            .unwrap(),
        "a committed write holds its key as a row, not as a claim"
    );
    assert_eq!(
        wrapper.index_lookup(1, &[Value::Int4(9)]).unwrap().len(),
        1,
        "…and the committed row is what the read probe finds"
    );
}

/// A key staged by an `UPDATE` is claimed too, and the key the row moved **off** is
/// not — the update's staged image is what the claim reads, never the committed one it
/// replaces.
#[test]
fn unique_key_claim_covers_an_updated_row() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper
        .insert("t", vec![Value::Int4(1), Value::Int4(10)])
        .unwrap();
    wrapper
        .create_index(full_spec(1, "t", vec![0], true))
        .unwrap();
    let row_id = wrapper.scan("t").unwrap()[0].0;

    wrapper.begin_transaction(1).unwrap();
    wrapper
        .update_in_tx(1, "t", row_id, vec![Value::Int4(5), Value::Int4(10)])
        .unwrap();

    assert!(
        wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(5)])
            .unwrap(),
        "a key an UPDATE stages must be claimed"
    );
    // The key it moved *off* is no longer staged by anyone. It is still a committed
    // row until the block commits, which is the read probe's answer, not a claim.
    assert!(
        !wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(1)])
            .unwrap(),
        "the vacated key is not claimed"
    );

    wrapper.rollback_transaction(1).unwrap();
}

/// The claim key is the index's own: a NULL key is not claimed under the default
/// `NULLS DISTINCT` and is claimed under `NULLS NOT DISTINCT`, exactly as the
/// self-probe encodes it.
#[test]
fn unique_key_claim_follows_null_distinctness() {
    let mut nulls_distinct = full_spec(1, "t", vec![0], true);
    let mut nulls_not_distinct = full_spec(2, "u", vec![0], true);
    nulls_not_distinct.nulls_not_distinct = true;

    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper.create_table("u").unwrap();
    nulls_distinct.table = "t".to_string();
    wrapper.create_index(nulls_distinct).unwrap();
    wrapper.create_index(nulls_not_distinct).unwrap();

    wrapper.begin_transaction(1).unwrap();
    wrapper.insert_in_tx(1, "t", vec![Value::Null]).unwrap();
    wrapper.insert_in_tx(1, "u", vec![Value::Null]).unwrap();

    assert!(
        !wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Null])
            .unwrap(),
        "a NULL key under NULLS DISTINCT is not a uniqueness candidate, so not claimed"
    );
    assert!(
        wrapper
            .unique_key_claimed_elsewhere(2, 2, &[Value::Null])
            .unwrap(),
        "under NULLS NOT DISTINCT two NULLs collide, claim included"
    );

    wrapper.rollback_transaction(1).unwrap();
}

/// An aborted block claims what its innermost open savepoint holds, and nothing at
/// all with no savepoint open — 18.6's two arms. A successful `ROLLBACK TO
/// SAVEPOINT` revives the block, and writes made after the recovery claim again.
#[test]
fn unique_key_claim_suspends_on_abort_except_under_a_savepoint() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper
        .create_index(full_spec(1, "t", vec![0], true))
        .unwrap();

    // No savepoint open: the abort aborted the whole block, so nothing is claimed.
    wrapper.begin_transaction(1).unwrap();
    wrapper.insert_in_tx(1, "t", vec![Value::Int4(1)]).unwrap();
    assert!(
        wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(1)])
            .unwrap()
    );
    wrapper.suspend_claims(1);
    assert!(
        !wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(1)])
            .unwrap(),
        "an aborted block with no savepoint holds no claim"
    );
    wrapper.rollback_transaction(1).unwrap();

    // Savepoint open: only that subtransaction aborted, so the pre-savepoint write is
    // still live and still claims — while the post-savepoint one is not in the frame.
    wrapper.begin_transaction(1).unwrap();
    wrapper.insert_in_tx(1, "t", vec![Value::Int4(2)]).unwrap();
    wrapper.savepoint(1).unwrap();
    wrapper.insert_in_tx(1, "t", vec![Value::Int4(3)]).unwrap();
    wrapper.suspend_claims(1);
    assert!(
        wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(2)])
            .unwrap(),
        "a pre-savepoint write keeps its claim through an abort"
    );
    assert!(
        !wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(3)])
            .unwrap(),
        "a post-savepoint write does not survive the abort, so it claims nothing"
    );

    // The block revives; a write made after the recovery claims like any other.
    wrapper.rollback_to_savepoint(1).unwrap();
    wrapper.insert_in_tx(1, "t", vec![Value::Int4(4)]).unwrap();
    assert!(
        wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(4)])
            .unwrap(),
        "a write made after the block revived must claim its key"
    );
    assert!(
        wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(2)])
            .unwrap(),
        "and the pre-savepoint write claims again"
    );
    wrapper.rollback_transaction(1).unwrap();
}

/// `ROLLBACK TO SAVEPOINT` releases exactly the claims whose writes it undid — the
/// release is structural, since the claim *is* the buffered write.
#[test]
fn unique_key_claim_released_by_savepoint_rollback() {
    let mut wrapper = BtreeStorageWrapper::new();
    wrapper.create_table("t").unwrap();
    wrapper
        .create_index(full_spec(1, "t", vec![0], true))
        .unwrap();

    wrapper.begin_transaction(1).unwrap();
    wrapper.insert_in_tx(1, "t", vec![Value::Int4(1)]).unwrap();
    wrapper.savepoint(1).unwrap();
    wrapper.insert_in_tx(1, "t", vec![Value::Int4(2)]).unwrap();
    assert!(
        wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(2)])
            .unwrap()
    );

    wrapper.rollback_to_savepoint(1).unwrap();
    assert!(
        !wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(2)])
            .unwrap(),
        "ROLLBACK TO SAVEPOINT must release the claim it undid"
    );
    assert!(
        wrapper
            .unique_key_claimed_elsewhere(2, 1, &[Value::Int4(1)])
            .unwrap(),
        "a write made before the savepoint keeps its claim"
    );

    wrapper.rollback_transaction(1).unwrap();
}

/// An unregistered index fails loud rather than answering "not claimed" — the same
/// refusal the read probe makes, so a bad `index_id` cannot read as "no conflict".
#[test]
fn unique_key_claim_unknown_index_errors() {
    let wrapper = BtreeStorageWrapper::new();
    assert!(matches!(
        wrapper.unique_key_claimed_elsewhere(1, 999, &[Value::Int4(1)]),
        Err(StorageError::IndexNotFound(999))
    ));
}

/// An oversized row is refused by the first of its table's indexes in id (creation)
/// order — 18.6's order — on the autocommit path and inside a transaction, whether the
/// indexes were created or reattached out of order. Eight fresh stores each, so a walk
/// in map order passes by accident with probability `(1/6)^8` per path.
#[test]
fn an_oversized_row_is_refused_by_its_first_index_in_id_order() {
    let too_long = || vec![Value::Text("x".repeat(3000))];
    let names_i1 = |err: &pglet_core::Error| matches!(err, pglet_core::Error::BtreeIndexRowTooLarge(size) if size.index == "i1");
    for reattach in [false, true] {
        for _ in 0..8 {
            let mut wrapper = BtreeStorageWrapper::new();
            wrapper.create_table("t").unwrap();
            for id in [5, 2, 6, 1, 4, 3] {
                let mut spec = full_spec(id, "t", vec![0], false);
                spec.name = format!("i{id}");
                if reattach {
                    wrapper.reattach_index(spec).unwrap();
                } else {
                    wrapper.create_index(spec).unwrap();
                }
            }
            let err = wrapper.insert("t", too_long()).unwrap_err();
            assert!(names_i1(&err), "autocommit: {err:?}");
            wrapper.begin_transaction(1).unwrap();
            let err = wrapper.insert_in_tx(1, "t", too_long()).unwrap_err();
            assert!(
                matches!(&err, StorageError::IndexEntry(inner) if names_i1(inner)),
                "in a transaction: {err:?}"
            );
        }
    }
}
