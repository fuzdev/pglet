use super::*;
use pglet_sql::executor::IndexMethod;

/// SQL predicate/key-expression → the serialized planner `Expr` IR an `IndexSpec`
/// carries (native parse against an empty schema → `serialize_expr`).
fn ir(sql: &str) -> Vec<u8> {
    let expr =
        pglet_sql::parser::parse_expr(sql, &pglet_sql::catalog::Schema::new()).expect("parse");
    pglet_sql::planner::serialize_expr(&expr).expect("serialize")
}

/// A full (predicate-free, default NULLS DISTINCT) index spec.
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
fn test_create_and_scan() {
    let mut storage = MemoryStorage::new();
    storage.create_table("users").unwrap();
    let id1 = storage
        .insert("users", vec![Value::Int4(1), Value::Text("Alice".into())])
        .unwrap();
    let id2 = storage
        .insert("users", vec![Value::Int4(2), Value::Text("Bob".into())])
        .unwrap();

    assert_eq!(id1, 0);
    assert_eq!(id2, 1);

    let rows = storage.scan("users").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, 0);
    assert_eq!(rows[0].1[0], Value::Int4(1));
    assert_eq!(rows[1].0, 1);
    assert_eq!(rows[1].1[0], Value::Int4(2));
}

#[test]
fn test_get() {
    let mut storage = MemoryStorage::new();
    storage.create_table("users").unwrap();
    let id = storage
        .insert("users", vec![Value::Int4(1), Value::Text("Alice".into())])
        .unwrap();

    let row = storage.get("users", id).unwrap();
    assert!(row.is_some());
    assert_eq!(row.unwrap()[0], Value::Int4(1));

    let missing = storage.get("users", 999).unwrap();
    assert!(missing.is_none());
}

#[test]
fn test_delete() {
    let mut storage = MemoryStorage::new();
    storage.create_table("users").unwrap();
    let id1 = storage.insert("users", vec![Value::Int4(1)]).unwrap();
    let id2 = storage.insert("users", vec![Value::Int4(2)]).unwrap();

    let deleted = storage.delete("users", id1).unwrap();
    assert!(deleted.is_some());
    assert_eq!(deleted.unwrap()[0], Value::Int4(1));

    let rows = storage.scan("users").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, id2);
    assert_eq!(rows[0].1[0], Value::Int4(2));

    // Delete non-existent row
    let deleted = storage.delete("users", id1).unwrap();
    assert!(deleted.is_none());
}

#[test]
fn test_update() {
    let mut storage = MemoryStorage::new();
    storage.create_table("users").unwrap();
    let id = storage
        .insert("users", vec![Value::Int4(1), Value::Text("Alice".into())])
        .unwrap();

    let old = storage
        .update(
            "users",
            id,
            vec![Value::Int4(1), Value::Text("Alicia".into())],
        )
        .unwrap();
    assert!(old.is_some());
    assert_eq!(old.unwrap()[1], Value::Text("Alice".into()));

    let row = storage.get("users", id).unwrap().unwrap();
    assert_eq!(row[1], Value::Text("Alicia".into()));

    // Update non-existent row
    let old = storage
        .update("users", 999, vec![Value::Int4(999)])
        .unwrap();
    assert!(old.is_none());
}

#[test]
fn test_add_column() {
    let mut storage = MemoryStorage::new();
    storage.create_table("users").unwrap();
    storage
        .insert("users", vec![Value::Int4(1), Value::Text("Alice".into())])
        .unwrap();
    storage
        .insert("users", vec![Value::Int4(2), Value::Text("Bob".into())])
        .unwrap();

    // Add a new column with NULL default
    storage.add_column("users", Value::Null).unwrap();

    let rows = storage.scan("users").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].1.len(), 3);
    assert_eq!(rows[0].1[2], Value::Null);
    assert_eq!(rows[1].1[2], Value::Null);
}

fn sorted(mut v: Vec<RowId>) -> Vec<RowId> {
    v.sort_unstable();
    v
}

#[test]
fn index_probe_matches_scan_filter_and_maintains() {
    let mut storage = MemoryStorage::new();
    storage.create_table("t").unwrap();
    let vals = [10, 20, 10, 30, 20, 10];
    let mut ids = Vec::new();
    for (i, v) in vals.iter().enumerate() {
        ids.push(
            storage
                .insert("t", vec![Value::Int4(i as i32), Value::Int4(*v)])
                .unwrap(),
        );
    }
    // Back-fill existing rows.
    storage
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();

    let scan_ids = |storage: &MemoryStorage, want: &Value| -> Vec<RowId> {
        let mut r: Vec<RowId> = storage
            .scan("t")
            .unwrap()
            .into_iter()
            .filter(|(_, row)| row.get(1) == Some(want))
            .map(|(id, _)| id)
            .collect();
        r.sort_unstable();
        r
    };

    for probe in [10, 20, 30, 99] {
        let want = Value::Int4(probe);
        assert_eq!(
            sorted(
                storage
                    .index_lookup(1, std::slice::from_ref(&want))
                    .unwrap()
            ),
            scan_ids(&storage, &want),
            "probe {probe}"
        );
    }
    assert_eq!(
        storage.index_lookup(1, &[Value::Int4(10)]).unwrap().len(),
        3
    );

    // UPDATE moves an entry; DELETE removes it.
    let moved = ids[0]; // value 10
    storage
        .update("t", moved, vec![Value::Int4(0), Value::Int4(20)])
        .unwrap();
    assert!(
        !storage
            .index_lookup(1, &[Value::Int4(10)])
            .unwrap()
            .contains(&moved)
    );
    assert!(
        storage
            .index_lookup(1, &[Value::Int4(20)])
            .unwrap()
            .contains(&moved)
    );

    storage.delete("t", moved).unwrap();
    assert!(
        !storage
            .index_lookup(1, &[Value::Int4(20)])
            .unwrap()
            .contains(&moved)
    );
    assert_eq!(
        sorted(storage.index_lookup(1, &[Value::Int4(20)]).unwrap()),
        scan_ids(&storage, &Value::Int4(20))
    );

    // NULL never matches; unknown index errors. The backend's
    // `IndexNotFound` maps to `pglet_core::Error::Execution` at the trait
    // boundary, so match the mapped form.
    assert!(storage.index_lookup(1, &[Value::Null]).unwrap().is_empty());
    let err = storage.index_lookup(42, &[Value::Int4(10)]).unwrap_err();
    assert!(matches!(&err, pglet_core::Error::Execution(m) if m == "index not found: 42"));
}

#[test]
fn composite_index_probes_the_full_tuple_and_skips_null_keys() {
    let mut storage = MemoryStorage::new();
    storage.create_table("t").unwrap();
    let ax = storage
        .insert("t", vec![Value::Int4(1), Value::Text("x".into())])
        .unwrap();
    storage
        .insert("t", vec![Value::Int4(1), Value::Text("y".into())])
        .unwrap();
    // Two NULL-bearing rows: composite NULLS DISTINCT keeps them out of the index.
    storage
        .insert("t", vec![Value::Int4(1), Value::Null])
        .unwrap();
    storage
        .insert("t", vec![Value::Int4(1), Value::Null])
        .unwrap();
    storage
        .create_index(full_spec(1, "t", vec![0, 1], true))
        .unwrap();

    // Full-tuple probe matches exactly one row.
    assert_eq!(
        storage
            .index_lookup(1, &[Value::Int4(1), Value::Text("x".into())])
            .unwrap(),
        vec![ax]
    );
    // A NULL component matches nothing (the NULL rows were never indexed).
    assert!(
        storage
            .index_lookup(1, &[Value::Int4(1), Value::Null])
            .unwrap()
            .is_empty()
    );
    // A tuple present in no row returns nothing.
    assert!(
        storage
            .index_lookup(1, &[Value::Int4(2), Value::Text("x".into())])
            .unwrap()
            .is_empty()
    );
}

/// Row ids of `t` whose column 1 (`Int4`) falls in the value range, via a full
/// scan + filter — the oracle a range probe must match (as a SET).
fn scan_range_ids(
    storage: &MemoryStorage,
    lower: Option<(i32, bool)>,
    upper: Option<(i32, bool)>,
) -> Vec<RowId> {
    let mut r: Vec<RowId> = storage
        .scan("t")
        .unwrap()
        .into_iter()
        .filter(|(_, row)| match row.get(1) {
            Some(Value::Int4(v)) => {
                let lo_ok = lower.is_none_or(|(b, incl)| if incl { *v >= b } else { *v > b });
                let hi_ok = upper.is_none_or(|(b, incl)| if incl { *v <= b } else { *v < b });
                lo_ok && hi_ok
            }
            // NULL keys are never in a value range.
            _ => false,
        })
        .map(|(id, _)| id)
        .collect();
    r.sort_unstable();
    r
}

#[test]
fn memory_index_range_matches_scan_filter_for_each_op() {
    let mut storage = MemoryStorage::new();
    storage.create_table("t").unwrap();
    // Values with duplicates and a NULL — NULL must be excluded from ranges.
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
        storage
            .insert("t", vec![Value::Int4(i as i32), key])
            .unwrap();
    }
    storage
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();

    let probe = |lo: Option<(i32, bool)>, hi: Option<(i32, bool)>| -> Vec<RowId> {
        let lo = lo.map(|(v, incl)| (Value::Int4(v), incl));
        let hi = hi.map(|(v, incl)| (Value::Int4(v), incl));
        sorted(storage.index_range_lookup(1, lo, hi, true, None).unwrap())
    };

    // > 20, >= 20, < 20, <= 20, BETWEEN 10 AND 30, fully unbounded.
    assert_eq!(
        probe(Some((20, false)), None),
        scan_range_ids(&storage, Some((20, false)), None)
    );
    assert_eq!(
        probe(Some((20, true)), None),
        scan_range_ids(&storage, Some((20, true)), None)
    );
    assert_eq!(
        probe(None, Some((20, false))),
        scan_range_ids(&storage, None, Some((20, false)))
    );
    assert_eq!(
        probe(None, Some((20, true))),
        scan_range_ids(&storage, None, Some((20, true)))
    );
    assert_eq!(
        probe(Some((10, true)), Some((30, true))),
        scan_range_ids(&storage, Some((10, true)), Some((30, true)))
    );
    // Fully unbounded value range == every non-NULL key.
    assert_eq!(probe(None, None), scan_range_ids(&storage, None, None));
    // The NULL-keyed row (id 4) never appears in any range result.
    assert!(
        !probe(None, None).contains(&4),
        "NULL key excluded from range"
    );
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
/// each one-sided comparison, and each two-sided pair of them.
fn range_bound_shapes(pivots: &[i32]) -> Vec<(RangeEnd, RangeEnd)> {
    let mut ends: Vec<RangeEnd> = vec![None];
    for &pivot in pivots {
        ends.push(Some((pivot, true)));
        ends.push(Some((pivot, false)));
    }
    let mut shapes = Vec::new();
    for lower in &ends {
        for upper in &ends {
            // An interval that names no key is `memory_index_range_with_no_interval_answers_no_id`'s.
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
    rows: &[(RowId, Option<i32>)],
    lower: RangeEnd,
    upper: RangeEnd,
    ascending: bool,
    limit: Option<usize>,
) -> Vec<RowId> {
    let mut hits: Vec<(i32, RowId)> = rows
        .iter()
        .filter_map(|&(id, key)| {
            let v = key?;
            let lo_ok = lower.is_none_or(|(b, incl)| if incl { v >= b } else { v > b });
            let hi_ok = upper.is_none_or(|(b, incl)| if incl { v <= b } else { v < b });
            (lo_ok && hi_ok).then_some((v, id))
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
fn memory_index_range_leaves_out_the_null_keys_a_nulls_not_distinct_index_holds() {
    // A comparison is not true of a NULL, so no range read answers a NULL-keyed row —
    // over the index that holds one as over the index that does not. Unique and not.
    for unique in [false, true] {
        let keys: &[Option<i32>] = if unique {
            &[Some(30), None, Some(10), Some(-5), Some(40), Some(20)]
        } else {
            &[Some(30), None, Some(10), Some(-5), None, Some(40), Some(10)]
        };
        let mut storage = MemoryStorage::new();
        storage.create_table("t").unwrap();
        let mut rows: Vec<(RowId, Option<i32>)> = Vec::new();
        for (i, key) in keys.iter().enumerate() {
            let id = storage
                .insert(
                    "t",
                    vec![Value::Int4(i as i32), key.map_or(Value::Null, Value::Int4)],
                )
                .unwrap();
            rows.push((id, *key));
        }
        storage
            .create_index(nulls_not_distinct_spec(1, "t", vec![1], unique))
            .unwrap();
        // The premise: this index does hold its NULL-keyed rows, under a NULL key.
        let null_keyed: Vec<RowId> = rows
            .iter()
            .filter(|(_, key)| key.is_none())
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(
            sorted(storage.index_lookup(1, &[Value::Null]).unwrap()),
            null_keyed,
            "unique={unique}: the index holds the NULL-keyed rows"
        );

        for (lower, upper) in range_bound_shapes(&[-5, 10, 25, 99]) {
            for ascending in [true, false] {
                for limit in [None, Some(1), Some(3)] {
                    let got = storage
                        .index_range_lookup(
                            1,
                            lower.map(|(v, incl)| (Value::Int4(v), incl)),
                            upper.map(|(v, incl)| (Value::Int4(v), incl)),
                            ascending,
                            limit,
                        )
                        .unwrap();
                    assert_eq!(
                        got,
                        expected_range_ids(&rows, lower, upper, ascending, limit),
                        "unique={unique} {lower:?}..{upper:?} ascending={ascending} limit={limit:?}"
                    );
                }
            }
        }
        // A NULL bound is no bound that side, and brings no NULL key with it.
        for ascending in [true, false] {
            assert_eq!(
                storage
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
fn memory_index_range_with_no_lower_bound_holds_the_least_present_key() {
    // An empty `text` is the least key a present value has — one byte, the presence tag.
    // A range with no lower bound starts there: above the NULL key, and not past `''`.
    for nulls_not_distinct in [false, true] {
        let mut storage = MemoryStorage::new();
        storage.create_table("t").unwrap();
        for (i, key) in [Some("a"), None, Some(""), Some("b")].iter().enumerate() {
            storage
                .insert(
                    "t",
                    vec![
                        Value::Int4(i as i32),
                        key.map_or(Value::Null, |k| Value::Text(k.to_string())),
                    ],
                )
                .unwrap();
        }
        storage
            .create_index(IndexSpec {
                nulls_not_distinct,
                ..full_spec(1, "t", vec![1], false)
            })
            .unwrap();
        let below = |upper: &str, inclusive: bool, ascending: bool| {
            storage
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
        // Nothing sorts below `''`: equal ends with the upper excluded name no key, which
        // `BTreeMap::range` must not be handed.
        assert_eq!(below("", false, true), Vec::<RowId>::new(), "{label}: < ''");
        assert_eq!(
            storage
                .index_range_lookup(1, None, None, true, None)
                .unwrap(),
            vec![2, 0, 3],
            "{label}: every non-NULL key, in key order"
        );
    }
}

#[test]
fn memory_index_range_over_a_composite_key_leaves_out_a_null_leading_part() {
    // The lower end is the least key whose leading part is present, so the rule holds
    // by the key's leading tag and not by the index having one column.
    let mut storage = MemoryStorage::new();
    storage.create_table("t").unwrap();
    let int = |v: Option<i32>| v.map_or(Value::Null, Value::Int4);
    for (a, b) in [
        (None, Some(1)),
        (Some(1), None),
        (None, None),
        (Some(i32::MIN), Some(2)),
        (Some(1), Some(2)),
    ] {
        storage.insert("t", vec![int(a), int(b)]).unwrap();
    }
    storage
        .create_index(nulls_not_distinct_spec(1, "t", vec![0, 1], true))
        .unwrap();
    assert_eq!(
        sorted(
            storage
                .index_lookup(1, &[Value::Null, Value::Null])
                .unwrap()
        ),
        vec![2],
        "the index holds the all-NULL key"
    );
    assert_eq!(
        storage
            .index_range_lookup(1, None, None, true, None)
            .unwrap(),
        vec![3, 1, 4],
        "the keys whose leading part is present, in key order"
    );
}

#[test]
fn memory_index_range_with_no_interval_answers_no_id() {
    // `BTreeMap::range` panics on an inverted interval and on an equal one with both
    // ends excluded; the lookup must answer no id for both, as the B+tree path does.
    let mut storage = MemoryStorage::new();
    storage.create_table("t").unwrap();
    for v in [1, 2, 3, 4, 5] {
        storage
            .insert("t", vec![Value::Int4(v), Value::Int4(v)])
            .unwrap();
    }
    storage
        .create_index(full_spec(1, "t", vec![1], true))
        .unwrap();
    let b = |v: i32, incl: bool| Some((Value::Int4(v), incl));
    for (lo, hi) in [
        (b(4, true), b(2, true)),
        (b(4, false), b(2, false)),
        (b(3, false), b(3, false)),
        (b(3, true), b(3, false)),
        (b(3, false), b(3, true)),
    ] {
        for ascending in [true, false] {
            assert_eq!(
                storage
                    .index_range_lookup(1, lo.clone(), hi.clone(), ascending, None)
                    .unwrap(),
                Vec::<RowId>::new(),
                "{lo:?}..{hi:?}"
            );
        }
    }
    // An equal interval with both ends included is the one key.
    assert_eq!(
        storage
            .index_range_lookup(1, b(3, true), b(3, true), true, None)
            .unwrap(),
        vec![2]
    );
}

#[test]
fn memory_index_range_order_and_limit() {
    let mut storage = MemoryStorage::new();
    storage.create_table("t").unwrap();
    // Keys 50,10,30,20,40 at ids 0..5; ascending key order is 10,20,30,40,50.
    for (i, v) in [50, 10, 30, 20, 40].iter().enumerate() {
        storage
            .insert("t", vec![Value::Int4(i as i32), Value::Int4(*v)])
            .unwrap();
    }
    storage
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();

    // Ascending order: ids by increasing key value → [1,3,2,4,0].
    let asc = storage
        .index_range_lookup(1, None, None, true, None)
        .unwrap();
    assert_eq!(asc, vec![1, 3, 2, 4, 0], "ascending = index-key order");
    // Descending is the reverse.
    let desc = storage
        .index_range_lookup(1, None, None, false, None)
        .unwrap();
    assert_eq!(desc, vec![0, 4, 2, 3, 1], "descending = reverse key order");

    // LIMIT 2 ascending = first two in key order; descending = last two.
    assert_eq!(
        storage
            .index_range_lookup(1, None, None, true, Some(2))
            .unwrap(),
        vec![1, 3]
    );
    assert_eq!(
        storage
            .index_range_lookup(1, None, None, false, Some(2))
            .unwrap(),
        vec![0, 4]
    );
    // Limit larger than the band returns the whole band.
    assert_eq!(
        storage
            .index_range_lookup(1, None, None, true, Some(99))
            .unwrap(),
        asc
    );
}

#[test]
fn memory_index_range_partial_index_only_satisfying_rows() {
    let mut storage = MemoryStorage::new();
    storage.create_table("t").unwrap();
    // Column 2 is the predicate flag; only `flag = 1` rows are indexed.
    for (i, (k, flag)) in [(10, 1), (20, 0), (30, 1), (40, 0)].iter().enumerate() {
        storage
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
    storage.create_index(spec).unwrap();

    // Range 0..100 only returns the predicate-satisfying rows (ids 0 and 2).
    let got = sorted(
        storage
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
fn memory_index_range_null_bound_is_unbounded_and_unknown_index() {
    let mut storage = MemoryStorage::new();
    storage.create_table("t").unwrap();
    for (i, v) in [10, 20, 30].iter().enumerate() {
        storage
            .insert("t", vec![Value::Int4(i as i32), Value::Int4(*v)])
            .unwrap();
    }
    storage
        .create_index(full_spec(1, "t", vec![1], false))
        .unwrap();

    // A NULL upper bound is treated as unbounded that side: `>= 20` with NULL hi.
    let got = sorted(
        storage
            .index_range_lookup(
                1,
                Some((Value::Int4(20), true)),
                Some((Value::Null, true)),
                true,
                None,
            )
            .unwrap(),
    );
    assert_eq!(got, vec![1, 2], "NULL bound == unbounded that side");

    // Unknown index errors (mapped to `pglet_core::Error::Execution`).
    let err = storage
        .index_range_lookup(99, None, None, true, None)
        .unwrap_err();
    assert!(matches!(&err, pglet_core::Error::Execution(m) if m == "index not found: 99"));
}

#[test]
fn test_drop_column() {
    let mut storage = MemoryStorage::new();
    storage.create_table("users").unwrap();
    storage
        .insert(
            "users",
            vec![
                Value::Int4(1),
                Value::Text("Alice".into()),
                Value::Text("alice@test.com".into()),
            ],
        )
        .unwrap();

    // Drop the middle column (name)
    storage.drop_column("users", 1).unwrap();

    let rows = storage.scan("users").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1.len(), 2);
    assert_eq!(rows[0].1[0], Value::Int4(1));
    assert_eq!(rows[0].1[1], Value::Text("alice@test.com".into()));
}

/// `Storage::rebuild_index` is all-or-nothing: a rebuild that raises leaves the index
/// standing, definition and entries alike.
///
/// This backend's `rebuild_index` used to drop the old definition before asking
/// `create_index` to build the new one, so a key that fails to derive left the index
/// gone from storage while the catalog still carried it. The contract matters to
/// `executor::ddl::rebuild_indexes_over_enum_staged`, whose unwind puts back only the
/// specs that *succeeded* — it would have to put the failing one back too if a failed
/// rebuild could half-apply.
#[test]
fn a_failed_rebuild_leaves_the_index_standing() {
    let mut storage = MemoryStorage::new();
    storage.create_table("t").unwrap();
    let row_id = storage
        .insert("t", vec![Value::Int4(1), Value::Int4(0)])
        .unwrap();
    storage
        .create_index(full_spec(7, "t", vec![0], true))
        .unwrap();

    // The rebuilt key divides by the row's zero — a failure of the row's value, which
    // no key type the encoder learns to take can close.
    let mut failing = full_spec(7, "t", Vec::new(), true);
    failing.key_part_ir = Some(vec![ir("10 / v")]);
    failing.column_names = vec!["k".into(), "v".into()];
    assert!(storage.rebuild_index(failing).is_err());

    assert_eq!(
        storage.index_lookup(7, &[Value::Int4(1)]).unwrap(),
        vec![row_id],
        "the failed rebuild must have left the index on the key it was built with"
    );
}

/// An oversized row is refused by the first of its table's indexes in id (creation)
/// order — 18.6's order — whatever order the indexes were registered in and whatever
/// order the index map iterates. Eight fresh stores, each with its own randomly seeded
/// map, so a map-order walk passes by accident with probability `(1/6)^8`.
#[test]
fn an_oversized_row_is_refused_by_its_first_index_in_id_order() {
    for _ in 0..8 {
        let mut storage = MemoryStorage::new();
        storage.create_table("t").unwrap();
        for id in [5, 2, 6, 1, 4, 3] {
            let mut spec = full_spec(id, "t", vec![0], false);
            spec.name = format!("i{id}");
            storage.create_index(spec).unwrap();
        }
        let err = storage
            .insert("t", vec![Value::Text("x".repeat(3000))])
            .unwrap_err();
        assert!(
            matches!(&err, pglet_core::Error::BtreeIndexRowTooLarge(size) if size.index == "i1"),
            "{err:?}"
        );
    }
}
