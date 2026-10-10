use super::*;

#[test]
fn test_key_construction() {
    let key = BtreeKey::new(5, 100);
    assert_eq!(key.table_id(), Some(5));
    assert_eq!(key.row_id(), 100);
}

#[test]
fn test_row_ordering() {
    // Lower row_id should sort before higher row_id
    let r1 = BtreeKey::new(1, 1);
    let r2 = BtreeKey::new(1, 2);
    assert!(r1 < r2);
}

#[test]
fn test_table_ordering() {
    // Lower table_id should sort before higher table_id
    let t1 = BtreeKey::new(1, 100);
    let t2 = BtreeKey::new(2, 1);
    assert!(t1 < t2);
}

#[test]
fn test_encode_decode_roundtrip() {
    let cases = [
        BtreeKey::new(0, 0),
        BtreeKey::new(1, 2),
        BtreeKey::new(u32::MAX, u64::MAX),
        BtreeKey::new(12345, 98_765_432_109_876),
        BtreeKey::index(7, b"", 0),
        BtreeKey::index(7, b"hello", 42),
        BtreeKey::index(u32::MAX, &[0xff; 200], u64::MAX),
    ];

    for key in cases {
        let encoded = key.encode();
        let (decoded, consumed) = BtreeKey::decode(&encoded);
        assert_eq!(key, decoded);
        assert_eq!(consumed, encoded.len());
        assert_eq!(consumed, key.encoded_len());
    }
}

#[test]
fn test_encoded_bytes_ordering() {
    // Verify byte-wise comparison matches logical ordering, across both
    // kinds (all data keys must sort before all index keys).
    let keys = [
        BtreeKey::new(1, 1),
        BtreeKey::new(1, 2),
        BtreeKey::new(1, 100),
        BtreeKey::new(2, 1),
        BtreeKey::new(100, 1),
        BtreeKey::index(0, b"a", 0),
        BtreeKey::index(0, b"a", 1),
        BtreeKey::index(0, b"ab", 0),
        BtreeKey::index(1, b"a", 0),
    ];

    for i in 0..keys.len() - 1 {
        let e1 = keys[i].encode();
        let e2 = keys[i + 1].encode();
        assert!(
            e1 < e2,
            "Encoded ordering mismatch at index {}: {:?} should be < {:?}",
            i,
            keys[i],
            keys[i + 1]
        );
        // The hand-written Ord must agree with byte order.
        assert!(keys[i] < keys[i + 1]);
    }
}

#[test]
fn test_index_cross_length_ordering() {
    // The case the old [value_len][value] framing got WRONG: "b" (len 1)
    // sorted before "ab" (len 2) because 1 < 2. The order-preserving
    // framing must give lexicographic order: "a" < "ab" < "b".
    let a = BtreeKey::index(1, b"a", 0);
    let ab = BtreeKey::index(1, b"ab", 0);
    let b = BtreeKey::index(1, b"b", 0);

    // Encoded byte order.
    assert!(a.encode() < ab.encode(), "encoded: a < ab");
    assert!(ab.encode() < b.encode(), "encoded: ab < b");
    // Hand-written Ord agrees.
    assert!(a < ab && ab < b);
}

#[test]
fn test_index_embedded_nul() {
    // Values containing 0x00 must round-trip unchanged AND order correctly.
    let v0 = [0x00u8]; // [0x00]
    let v00 = [0x00u8, 0x00]; // [0x00, 0x00]
    let v1 = [0x01u8]; // [0x01]

    // Round-trip: encode -> decode returns the identical value bytes.
    for v in [v0.as_slice(), v00.as_slice(), v1.as_slice()] {
        let key = BtreeKey::index(1, v, 5);
        let (decoded, consumed) = BtreeKey::decode(&key.encode());
        assert_eq!(decoded, key);
        assert_eq!(consumed, key.encoded_len());
        if let BtreeKey::Index { value, .. } = decoded {
            assert_eq!(value, v, "value bytes must survive escaping");
        } else {
            panic!("expected index key");
        }
    }

    // Order: [0x00] < [0x00, 0x00] < [0x01] (lexicographic).
    let k0 = BtreeKey::index(1, &v0, 0);
    let k00 = BtreeKey::index(1, &v00, 0);
    let k1 = BtreeKey::index(1, &v1, 0);
    assert!(k0.encode() < k00.encode() && k00.encode() < k1.encode());
    assert!(k0 < k00 && k00 < k1);
}

#[test]
fn test_index_roundtrip_with_nuls_and_high_bytes() {
    // Encode->decode round-trip of data and index keys, including values
    // with embedded 0x00 and 0xFF bytes (the escape and escape-byte values).
    let cases = [
        BtreeKey::new(0, 0),
        BtreeKey::new(u32::MAX, u64::MAX),
        BtreeKey::index(7, b"", 0),
        BtreeKey::index(7, b"hello", 42),
        BtreeKey::index(0, &[0x00, 0xFF, 0x00, 0xFF], 1),
        BtreeKey::index(9, &[0xFF, 0xFF, 0xFF], u64::MAX),
        BtreeKey::index(u32::MAX, &[0x00; 64], 12345),
    ];
    for key in cases {
        let encoded = key.encode();
        let (decoded, consumed) = BtreeKey::decode(&encoded);
        assert_eq!(key, decoded);
        assert_eq!(consumed, encoded.len());
        assert_eq!(consumed, key.encoded_len());
    }
}

#[test]
fn test_data_sorts_before_index() {
    let data = BtreeKey::new(u32::MAX, u64::MAX);
    let index = BtreeKey::index(0, b"", 0);
    assert!(data < index);
    assert!(data.encode() < index.encode());
}

#[test]
fn test_table_range_keys() {
    let start = BtreeKey::table_start(5);
    let end = BtreeKey::table_end(5);

    // A key in table 5 should be >= start and < end
    let in_table = BtreeKey::new(5, 100);
    assert!(in_table >= start);
    assert!(in_table < end);

    // A key in table 4 should be < start
    let before = BtreeKey::new(4, 100);
    assert!(before < start);

    // A key in table 6 should be >= end
    let after = BtreeKey::new(6, 100);
    assert!(after >= end);
}

#[test]
fn test_index_range_keys() {
    let start = BtreeKey::index_start(3, b"value");
    let end = BtreeKey::index_end(3, b"value");

    // Any matching index entry sorts within [start, end].
    let hit_lo = BtreeKey::index(3, b"value", 0);
    let hit_hi = BtreeKey::index(3, b"value", u64::MAX);
    let hit_mid = BtreeKey::index(3, b"value", 12345);
    assert!(hit_lo >= start && hit_lo <= end);
    assert!(hit_hi >= start && hit_hi <= end);
    assert!(hit_mid >= start && hit_mid <= end);

    // A different value or index must fall outside.
    assert!(BtreeKey::index(3, b"other", 0) < start || BtreeKey::index(3, b"other", 0) > end);
    assert!(BtreeKey::index(4, b"value", 0) > end);
}

#[test]
fn test_index_probe_bounds_bracket_exactly() {
    // index_start/index_end for (index_id=1, value="x") must bracket every
    // row_id for that (index_id, value) and EXCLUDE neighboring values and
    // other indexes — under the order-preserving framing.
    let start = BtreeKey::index_start(1, b"x");
    let end = BtreeKey::index_end(1, b"x");

    // Every row_id for (1, "x") is inside [start, end].
    for &r in &[0u64, 1, 42, u64::MAX] {
        let hit = BtreeKey::index(1, b"x", r);
        assert!(hit >= start && hit <= end, "row_id {r} must be bracketed");
    }

    // Lexicographic neighbors "w" and "y" fall strictly outside.
    let w = BtreeKey::index(1, b"w", 0);
    let y = BtreeKey::index(1, b"y", u64::MAX);
    assert!(w < start, "\"w\" sorts before the probe range");
    assert!(y > end, "\"y\" sorts after the probe range");

    // A different index_id is outside regardless of value.
    assert!(BtreeKey::index(2, b"x", 0) > end, "index 2 is past index 1");

    // The bounds also hold byte-for-byte (on-disk order).
    assert!(w.encode() < start.encode());
    assert!(y.encode() > end.encode());
    assert!(BtreeKey::index(2, b"x", 0).encode() > end.encode());
}

#[test]
fn test_is_in_table() {
    let key = BtreeKey::new(5, 100);
    assert!(key.is_in_table(5));
    assert!(!key.is_in_table(4));
    assert!(!key.is_in_table(6));
    // Index keys are never "in a table".
    assert!(!BtreeKey::index(5, b"x", 0).is_in_table(5));
}

#[test]
fn test_composite_single_segment_is_byte_identical() {
    // A one-column composite key must encode to EXACTLY the single-column
    // bytes, so existing single-column index entries on disk stay valid.
    for raw in [
        b"".as_slice(),
        b"a",
        b"hello",
        &[0x00, 0xFF, 0x00],
        &[0xFF; 50],
    ] {
        let single = BtreeKey::index(7, raw, 42);
        let composite = BtreeKey::index_composite(7, &[raw], 42);
        assert_eq!(single, composite, "in-memory key must match for {raw:?}");
        assert_eq!(
            single.encode(),
            composite.encode(),
            "encoded bytes must match for {raw:?}"
        );
    }
    // The composite-of-one value blob is the raw bytes verbatim.
    assert_eq!(encode_composite_index_key(&[b"hello"]), b"hello".to_vec());
}

#[test]
fn test_composite_tuple_ordering() {
    // Lexicographic tuple order: (a,b) < (a,c) < (b,a).
    let ab = BtreeKey::index_composite(1, &[b"a", b"b"], 0);
    let ac = BtreeKey::index_composite(1, &[b"a", b"c"], 0);
    let ba = BtreeKey::index_composite(1, &[b"b", b"a"], 0);

    assert!(ab < ac && ac < ba, "in-memory Ord must be tuple order");
    assert!(
        ab.encode() < ac.encode() && ac.encode() < ba.encode(),
        "encoded byte order must be tuple order"
    );

    // A prefix-shorter first segment sorts first: (a, z) < (aa, a).
    let a_z = BtreeKey::index_composite(1, &[b"a", b"z"], 0);
    let aa_a = BtreeKey::index_composite(1, &[b"aa", b"a"], 0);
    assert!(a_z < aa_a);
    assert!(a_z.encode() < aa_a.encode());
}

#[test]
fn test_composite_boundary_unambiguous() {
    // The classic ambiguity the framing must defeat: (a, bc) != (ab, c).
    let a_bc = BtreeKey::index_composite(1, &[b"a", b"bc"], 5);
    let ab_c = BtreeKey::index_composite(1, &[b"ab", b"c"], 5);
    assert_ne!(a_bc, ab_c, "segment boundary must be unambiguous");
    assert_ne!(a_bc.encode(), ab_c.encode());

    // Embedded NUL bytes in a segment must not be mistaken for a boundary.
    let nul_seg = BtreeKey::index_composite(1, &[&[0x00u8, 0x01], b"x"], 0);
    let other = BtreeKey::index_composite(1, &[&[0x00u8], &[0x01u8, 0x78]], 0);
    assert_ne!(nul_seg, other);
    assert_ne!(nul_seg.encode(), other.encode());
}

#[test]
fn test_composite_probe_bounds_bracket_a_full_tuple() {
    // Full-key equality probing for a composite tuple uses the same
    // index_start/index_end machinery over the composite blob.
    let blob = encode_composite_index_key(&[b"s1", b"name"]);
    let start = BtreeKey::index_start(3, &blob);
    let end = BtreeKey::index_end(3, &blob);
    for &r in &[0u64, 1, 99, u64::MAX] {
        let hit = BtreeKey::index_composite(3, &[b"s1", b"name"], r);
        assert!(hit >= start && hit <= end, "row_id {r} must be bracketed");
    }
    // A tuple differing in the second column falls outside.
    let other = BtreeKey::index_composite(3, &[b"s1", b"other"], 0);
    assert!(other < start || other > end);
}

#[test]
fn test_index_table_bounds_bracket_the_whole_band() {
    // index_table_start/end must bracket EVERY entry of one index id and exclude
    // neighboring index ids — the walk that wants the whole band.
    let start = BtreeKey::index_table_start(5);
    let end = BtreeKey::index_table_end(5);

    // Empty-value and max-value entries for index 5 are both inside [start, end).
    for v in [b"".as_slice(), b"a", b"zzz", &[0xff; 8]] {
        for &r in &[0u64, u64::MAX] {
            let hit = BtreeKey::index(5, v, r);
            assert!(hit >= start && hit < end, "entry {v:?}/{r} must be in band");
        }
    }
    // Index 4's max entry is below start; index 6's min entry is at/after end.
    assert!(BtreeKey::index(4, &[0xff; 8], u64::MAX) < start);
    assert!(BtreeKey::index(6, b"", 0) >= end);
    // The saturating top id doesn't panic and yields a usable (empty) band end.
    let _ = BtreeKey::index_table_end(u32::MAX);
}

#[test]
fn test_index_range_ends_open_ended() {
    // For value "m", each end must sit on the correct side of every "m" entry, with
    // the right strictness at the boundary.
    let id = 3;
    let enc = b"m";
    let m_lo = BtreeKey::index(id, enc, 0);
    let m_hi = BtreeKey::index(id, enc, u64::MAX);
    let below = BtreeKey::index(id, b"l", u64::MAX);
    let above = BtreeKey::index(id, b"n", 0);
    // A value "m" is a strict prefix of: the nearest greater value there is.
    let next = BtreeKey::index(id, b"m\0", 0);

    // > m : starts past all of "m", at or below everything greater.
    let s = BtreeKey::index_range_start(id, enc, false);
    assert!(m_lo < s && m_hi < s, "> m excludes every 'm' row");
    assert!(above >= s && next >= s, "> m includes what is greater");

    // >= m : starts at "m" row 0, above "l".
    let s = BtreeKey::index_range_start(id, enc, true);
    assert!(m_lo >= s, ">= m includes every 'm' row");
    assert!(below < s, ">= m excludes 'l'");

    // < m : ends before all of "m", after "l".
    let e = BtreeKey::index_range_end(id, enc, false);
    assert!(below < e, "< m includes 'l'");
    assert!(m_lo >= e, "< m excludes every 'm' row");

    // <= m : ends past "m" row u64::MAX, at or below everything greater.
    let e = BtreeKey::index_range_end(id, enc, true);
    assert!(m_hi < e, "<= m includes every 'm' row");
    assert!(above >= e && next >= e, "<= m excludes what is greater");
}

#[test]
fn test_index_range_ends_compose_an_inclusive_between() {
    // BETWEEN "b" AND "d" (inclusive) brackets b, c, d and excludes a, e.
    let id = 1;
    let s = BtreeKey::index_range_start(id, b"b", true);
    let e = BtreeKey::index_range_end(id, b"d", true);
    for v in [b"b".as_slice(), b"c", b"d"] {
        for &r in &[0u64, u64::MAX] {
            let hit = BtreeKey::index(id, v, r);
            assert!(hit >= s && hit < e, "{v:?}/{r} must be in [b,d]");
        }
    }
    assert!(BtreeKey::index(id, b"a", u64::MAX) < s, "'a' excluded");
    assert!(BtreeKey::index(id, b"e", 0) >= e, "'e' excluded");
    // An inverted interval is an empty range: start >= end.
    let s2 = BtreeKey::index_range_start(id, b"d", true);
    let e2 = BtreeKey::index_range_end(id, b"b", true);
    assert!(s2 >= e2, "inverted BETWEEN is an empty range");
    // So is one value with an end excluded, whichever end.
    for (lower_inclusive, upper_inclusive) in [(true, false), (false, true), (false, false)] {
        let s = BtreeKey::index_range_start(id, b"c", lower_inclusive);
        let e = BtreeKey::index_range_end(id, b"c", upper_inclusive);
        assert!(s >= e, "{lower_inclusive}/{upper_inclusive} names no key");
    }
}

#[test]
fn test_a_range_start_is_never_the_band_start() {
    // The band's own start sorts below every key, so a walk from it holds whatever the
    // band holds; a range's start is a blob's, and holds nothing below that blob. The
    // one-byte blobs stand for the two tags a key's leading segment has.
    let id = 9;
    let null_keyed = BtreeKey::index(id, &[0x00], 7);
    let least_present = BtreeKey::index(id, &[0x01], 0);
    assert!(BtreeKey::index_table_start(id) <= null_keyed);
    let start = BtreeKey::index_range_start(id, &[0x01], true);
    assert!(
        null_keyed < start,
        "a key below the blob is below the start"
    );
    assert!(least_present >= start, "the blob's own entry is in");
}

#[test]
fn test_try_decode() {
    let key = BtreeKey::new(1, 2);
    let encoded = key.encode();

    // Full bytes work
    assert_eq!(BtreeKey::try_decode(&encoded), Some((key, encoded.len())));

    // Too short fails gracefully
    assert_eq!(BtreeKey::try_decode(&encoded[0..8]), None);
    assert_eq!(BtreeKey::try_decode(&[]), None);

    // Index keys: truncated header and truncated value both fail.
    let ikey = BtreeKey::index(9, b"abcdef", 7);
    let ienc = ikey.encode();
    assert_eq!(BtreeKey::try_decode(&ienc), Some((ikey, ienc.len())));
    assert_eq!(BtreeKey::try_decode(&ienc[0..7]), None);
    assert_eq!(BtreeKey::try_decode(&ienc[0..ienc.len() - 1]), None);
}

#[test]
fn test_system_encode_decode_roundtrip() {
    let cases = [
        BtreeKey::sys_catalog(0),
        BtreeKey::sys_catalog(u32::MAX),
        BtreeKey::sys_table(1),
        BtreeKey::sys_table(u32::MAX),
    ];
    for key in cases {
        let encoded = key.encode();
        assert_eq!(encoded.len(), BtreeKey::SYSTEM_SIZE);
        let (decoded, consumed) = BtreeKey::decode(&encoded);
        assert_eq!(decoded, key);
        assert_eq!(consumed, BtreeKey::SYSTEM_SIZE);
        assert_eq!(
            BtreeKey::try_decode(&encoded),
            Some((key.clone(), encoded.len()))
        );
        assert_eq!(key.table_id(), None);
        assert_eq!(key.row_id(), 0);
    }
}

#[test]
fn test_system_sorts_after_data_and_index() {
    let data = BtreeKey::new(u32::MAX, u64::MAX);
    let index = BtreeKey::index(u32::MAX, b"\xff\xff", u64::MAX);
    let sys = BtreeKey::sys_catalog(0);
    assert!(data < sys);
    assert!(index < sys);
    // Byte order agrees with Ord across all three kinds.
    assert!(data.encode() < sys.encode());
    assert!(index.encode() < sys.encode());
}

#[test]
fn test_system_band_ordering() {
    // Catalog chunks sort before table meta, each by ascending id; byte order
    // matches the hand-written Ord.
    let keys = [
        BtreeKey::sys_catalog(0),
        BtreeKey::sys_catalog(1),
        BtreeKey::sys_catalog(u32::MAX),
        BtreeKey::sys_table(0),
        BtreeKey::sys_table(5),
        BtreeKey::sys_table(u32::MAX),
    ];
    for pair in keys.windows(2) {
        assert!(pair[0] < pair[1], "{:?} < {:?}", pair[0], pair[1]);
        assert!(
            pair[0].encode() < pair[1].encode(),
            "encoded {:?} < {:?}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn test_system_band_bounds() {
    // The catalog band [sys_catalog_start .. sys_table_start) brackets exactly
    // the catalog chunks; the table band [sys_table_start .. sys_table_end(n))
    // brackets exactly the table-meta entries with table_id < n.
    let cat_lo = BtreeKey::sys_catalog_start();
    let tab_lo = BtreeKey::sys_table_start();
    let tab_hi = BtreeKey::sys_table_end(10);

    for chunk in [0u32, 1, 9999, u32::MAX] {
        let k = BtreeKey::sys_catalog(chunk);
        assert!(k >= cat_lo && k < tab_lo, "catalog {chunk} in band");
    }
    for table_id in [0u32, 1, 9] {
        let k = BtreeKey::sys_table(table_id);
        assert!(k >= tab_lo && k < tab_hi, "table {table_id} in band");
    }
    // table_id == next_table_id is the exclusive upper, not included.
    assert!(BtreeKey::sys_table(10) >= tab_hi);
}

#[test]
fn test_system_as_system_accessor() {
    assert_eq!(
        BtreeKey::sys_catalog(7).as_system(),
        Some((SysKind::Catalog, 7))
    );
    assert_eq!(
        BtreeKey::sys_table(3).as_system(),
        Some((SysKind::Table, 3))
    );
    assert_eq!(BtreeKey::new(1, 2).as_system(), None);
    assert_eq!(BtreeKey::index(1, b"x", 2).as_system(), None);
}

#[test]
fn test_system_try_decode_truncation() {
    let key = BtreeKey::sys_table(42);
    let enc = key.encode();
    assert_eq!(BtreeKey::try_decode(&enc), Some((key, enc.len())));
    for cut in 0..enc.len() {
        assert_eq!(
            BtreeKey::try_decode(&enc[..cut]),
            None,
            "truncated to {cut}"
        );
    }
}
