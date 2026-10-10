use super::{JsonbValue, Value};
use crate::error::Result;

impl Value {
    /// Encode this scalar value to an **order-preserving** byte string for a
    /// secondary-index key.
    ///
    /// The returned bytes are byte-comparable in the same order as the logical
    /// values of this type, so `BtreeKey::index` can frame them directly (it adds
    /// only the escape-terminated framing, preserving whatever order these bytes
    /// already carry). The single encoder shared by storage-side index
    /// maintenance and the executor-side equality probe, so both sides agree on
    /// the byte image of a value.
    ///
    /// # Encoding by type
    ///
    /// - `Bool` → `[0]` / `[1]`.
    /// - `Int2`/`Int4`/`Int8` → big-endian with the sign bit flipped, so
    ///   negatives sort before non-negatives.
    /// - `Timestamp` / `Date` → the binary format's microseconds / days since
    ///   2000-01-01 as a sign-flipped big-endian `i64` / `i32`, identical to `Int8` /
    ///   `Int4` (UTC; pglet has no distinct `timestamptz` value), `infinity` and
    ///   `-infinity` at the extreme counts.
    /// - `Text`/`Varchar` → raw UTF-8 bytes (already order-preserving under
    ///   pglet's binary compare).
    /// - `Bytea` → raw bytes; `Uuid` → its 16 raw bytes (both order-preserving).
    /// - `Float4`/`Float8` → the value widened to `float8`, as eight big-endian bytes in
    ///   18.6's `float8_cmp_internal` order: `-0` keyed as `0`, every `NaN` as one key
    ///   above `Infinity`'s (`float_order_key`). The two widths share one key space.
    /// - `Interval` → its comparison estimate (`micros + (months*30 + days) *
    ///   USECS_PER_DAY`) as a sign-flipped big-endian `i128`, so index order
    ///   matches compare order (`30 days` and `1 mon` collide).
    /// - `Array` → each element in order — `0x01` and the element's own key (a `text` or
    ///   `bytea` element's escaped and terminated, `escape_into`), or a bare `0x02` for
    ///   a NULL element: 18.6's `array_cmp` order, elements first and then length, a NULL
    ///   element above every value and equal to another NULL. Nothing ends the array —
    ///   the framing around a key does, and a shorter array is a prefix of one that
    ///   extends it, which is where length decides. The empty array is the empty key.
    /// - `Jsonb` → 18.6's `compareJsonbContainers` order (`jsonb_order_key`).
    /// - `Null` → the empty byte string. NULLs are therefore *indexed* but,
    ///   because SQL equality never matches NULL, a probe never returns them
    ///   (the executor short-circuits `col = NULL`); NULL entries exist only so
    ///   the index stays a faithful image of the column.
    ///
    /// # Errors
    ///
    /// None: every value has a key. The `Result` stays for the callers, each of which
    /// carries an encoder error to its own (`pglet_storage`'s `unindexable`).
    // arms kept one-per-variant for the per-type encoding table; several integer
    // arms share the sign-flip shape but differ in width
    #[allow(clippy::match_same_arms)]
    pub fn encode_index_key(&self) -> Result<Vec<u8>> {
        // Flip the sign bit so two's-complement negatives (high bit set) sort
        // before non-negatives under unsigned big-endian byte order.
        match self {
            Self::Null => Ok(Vec::new()),
            Self::Bool(b) => Ok(vec![u8::from(*b)]),
            Self::Int2(v) => Ok(((*v as u16) ^ (1 << 15)).to_be_bytes().to_vec()),
            Self::Int4(v) => Ok(((*v as u32) ^ (1 << 31)).to_be_bytes().to_vec()),
            Self::Int8(v) => Ok(((*v as u64) ^ (1 << 63)).to_be_bytes().to_vec()),
            // Key on the binary format's count as a sign-flipped big-endian integer —
            // microseconds since 2000 for a timestamp (stored as UTC; same shape as
            // Int8), days since 2000 for a date (same shape as Int4). The infinity
            // sentinels are the extreme counts, so the key order is the value order.
            Self::Timestamp(ts) => Ok(((ts.to_pg_micros() as u64) ^ (1 << 63))
                .to_be_bytes()
                .to_vec()),
            Self::Date(d) => Ok(((d.to_pg_days() as u32) ^ (1 << 31)).to_be_bytes().to_vec()),
            Self::Time(t) => {
                // A `time` is always non-negative (`0 ≤ t ≤ USECS_PER_DAY`), and
                // Postgres's `time` B-tree opclass compares it as a plain signed
                // int64 — so plain big-endian bytes are order-preserving, no sign
                // flip needed (unlike Int8/Date/Timestamp).
                Ok(t.to_be_bytes().to_vec())
            }
            Self::Numeric(nn) => Ok(nn.encode_order_key()),
            Self::Text(s) => Ok(s.as_bytes().to_vec()),
            Self::Bytea(b) => Ok(b.clone()),
            Self::Uuid(bytes) => Ok(bytes.to_vec()),
            Self::Float4(v) => Ok(float_order_key(f64::from(*v))),
            Self::Float8(v) => Ok(float_order_key(*v)),
            Self::Jsonb(j) => {
                let mut out = Vec::new();
                // A top-level empty array is the one value type rank misplaces: 18.6
                // stores a top-level scalar as a one-element array, so `[]` sorts
                // below every scalar. Tag `0` is below every tag `jsonb_order_key` writes.
                if matches!(j, JsonbValue::Array(items) if items.is_empty()) {
                    out.push(0);
                } else {
                    jsonb_order_key(j, &mut out);
                }
                Ok(out)
            }
            Self::Array(arr) => {
                let mut out = Vec::new();
                for element in &arr.elements {
                    if element.is_null() {
                        out.push(2);
                    } else {
                        out.push(1);
                        let key = element.encode_index_key()?;
                        // Only a string's key can be a strict prefix of another's;
                        // every other element key is fixed-width or ends itself
                        // (a `numeric`'s, a `jsonb`'s), and is written bare.
                        if matches!(element, Self::Text(_) | Self::Bytea(_)) {
                            escape_into(&key, &mut out);
                        } else {
                            out.extend(key);
                        }
                    }
                }
                Ok(out)
            }
            // Key on the comparison estimate as a sign-flipped big-endian i128, so
            // index order == compare order and a UNIQUE interval index dedups on the
            // estimate (`30 days` collides `1 mon`), matching Postgres.
            Self::Interval(iv) => Ok(iv.index_key_bytes().to_vec()),
        }
    }
}

/// Append `bytes` escaped and terminated, so that a value sorts before every value it
/// is a strict prefix of and the bytes after it cannot reorder it: each `0x00` becomes
/// `0x00 0xFF`, and `0x00 0x00` ends the value.
fn escape_into(bytes: &[u8], out: &mut Vec<u8>) {
    for &b in bytes {
        out.push(b);
        if b == 0 {
            out.push(0xFF);
        }
    }
    out.extend([0, 0]);
}

/// Append a `jsonb` value's key in 18.6's `compareJsonbContainers` order below the top
/// level: a type tag in rank order (null, string, number, boolean, array, object), then
/// the content — a string's bytes escaped, a number's `numeric` order key (which ends
/// itself, and folds scale: `1` and `1.0` are one key), an array's element count ahead
/// of its elements (length first), an object's pair count ahead of its pairs in stored
/// order, each key's bytes escaped ahead of its value.
fn jsonb_order_key(value: &JsonbValue, out: &mut Vec<u8>) {
    let count = |n: usize, out: &mut Vec<u8>| out.extend((n as u32).to_be_bytes());
    match value {
        JsonbValue::Null => out.push(1),
        JsonbValue::String(s) => {
            out.push(2);
            escape_into(s.as_bytes(), out);
        }
        JsonbValue::Number(n) => {
            out.push(3);
            out.extend(n.encode_order_key());
        }
        JsonbValue::Bool(b) => out.extend([4, u8::from(*b)]),
        JsonbValue::Array(items) => {
            out.push(5);
            count(items.len(), out);
            for item in items {
                jsonb_order_key(item, out);
            }
        }
        JsonbValue::Object(pairs) => {
            out.push(6);
            count(pairs.len(), out);
            for (key, item) in pairs {
                escape_into(key.as_bytes(), out);
                jsonb_order_key(item, out);
            }
        }
    }
}

/// The order-preserving key of a float, in 18.6's `float8_cmp_internal` order:
/// `-Infinity`, the negatives, zero, the positives, `Infinity`, then `NaN`.
///
/// Two values that compare equal there share one key, which is what a unique index
/// enforces and an equality probe matches: `-0` takes `0`'s key, and every `NaN` —
/// either sign, any payload — takes the one key above `Infinity`'s. A `float4` is
/// keyed as the `float8` it widens to exactly, so the two widths share one order.
/// The rest is the IEEE-754 bit pattern made unsigned-comparable: a non-negative
/// value's sign bit is set, and a negative value's bits are all inverted so a larger
/// magnitude sorts lower.
fn float_order_key(v: f64) -> Vec<u8> {
    let bits = v.to_bits();
    let key = if v.is_nan() {
        u64::MAX
    } else if v == 0.0 {
        1 << 63
    } else if bits >> 63 == 0 {
        bits | (1 << 63)
    } else {
        !bits
    };
    key.to_be_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::super::{ArrayValue, TypeOid, Value};

    fn enc(v: &Value) -> Vec<u8> {
        v.encode_index_key().expect("indexable value")
    }

    #[test]
    fn encode_index_key_ints_are_sign_ordered() {
        // Negatives sort before zero before positives, across widths.
        for vals in [
            vec![
                Value::Int2(-2),
                Value::Int2(-1),
                Value::Int2(0),
                Value::Int2(1),
                Value::Int2(2),
            ],
            vec![
                Value::Int4(i32::MIN),
                Value::Int4(-1),
                Value::Int4(0),
                Value::Int4(1),
                Value::Int4(i32::MAX),
            ],
            vec![
                Value::Int8(i64::MIN),
                Value::Int8(-1),
                Value::Int8(0),
                Value::Int8(1),
                Value::Int8(i64::MAX),
            ],
        ] {
            for w in vals.windows(2) {
                assert!(
                    enc(&w[0]) < enc(&w[1]),
                    "{:?} should encode < {:?}",
                    w[0],
                    w[1]
                );
            }
        }
    }

    #[test]
    fn encode_index_key_bool_orders_false_before_true() {
        assert_eq!(enc(&Value::Bool(false)), vec![0]);
        assert_eq!(enc(&Value::Bool(true)), vec![1]);
        assert!(enc(&Value::Bool(false)) < enc(&Value::Bool(true)));
    }

    #[test]
    fn encode_index_key_text_is_lexicographic() {
        let a = Value::Text("a".into());
        let ab = Value::Text("ab".into());
        let b = Value::Text("b".into());
        assert!(enc(&a) < enc(&ab));
        assert!(enc(&ab) < enc(&b));
        // Raw UTF-8 bytes, unframed (framing is BtreeKey's job).
        assert_eq!(enc(&Value::Text("hello".into())), b"hello".to_vec());
    }

    #[test]
    fn encode_index_key_uuid_and_bytea_are_raw_ordered() {
        let lo = Value::Uuid([0u8; 16]);
        let mut hi_bytes = [0u8; 16];
        hi_bytes[0] = 1;
        let hi = Value::Uuid(hi_bytes);
        assert_eq!(enc(&lo).len(), 16);
        assert!(enc(&lo) < enc(&hi));

        assert!(enc(&Value::Bytea(vec![0x00])) < enc(&Value::Bytea(vec![0x01])));
        assert_eq!(enc(&Value::Bytea(vec![0xde, 0xad])), vec![0xde, 0xad]);
    }

    #[test]
    fn encode_index_key_timestamp_is_chronological() {
        use time::macros::datetime;
        let early = Value::timestamp(datetime!(1999-01-01 00:00:00 UTC));
        let mid = Value::timestamp(datetime!(2024-06-15 12:30:45 UTC));
        let late = Value::timestamp(datetime!(2099-12-31 23:59:59 UTC));
        assert!(enc(&early) < enc(&mid));
        assert!(enc(&mid) < enc(&late));
    }

    #[test]
    fn encode_index_key_date_is_chronological() {
        use time::macros::date;
        // Pre-epoch < epoch < post-epoch, so negatives sort before non-negatives.
        let early = Value::date(date!(1999 - 06 - 15));
        let epoch = Value::date(date!(2000 - 01 - 01));
        let late = Value::date(date!(2024 - 12 - 31));
        assert!(enc(&early) < enc(&epoch));
        assert!(enc(&epoch) < enc(&late));
        // Same shape/width as an Int4 key.
        assert_eq!(enc(&epoch).len(), 4);
    }

    /// The infinities key outside the domain's edges, for a date and a timestamp — the
    /// order an index-ordered scan reads.
    #[test]
    fn encode_index_key_infinities_bound_the_domain() {
        use super::super::Infinite;
        use time::macros::{date, datetime};
        let dates = [
            Value::Date(Infinite::NegInfinity),
            Value::date(date!(0001 - 01 - 01)),
            Value::date(date!(9999 - 12 - 31)),
            Value::Date(Infinite::Infinity),
        ];
        let stamps = [
            Value::Timestamp(Infinite::NegInfinity),
            Value::timestamp(datetime!(0001-01-01 00:00:00 UTC)),
            Value::timestamp(datetime!(9999-12-31 23:59:59.999999 UTC)),
            Value::Timestamp(Infinite::Infinity),
        ];
        for vals in [&dates, &stamps] {
            for w in vals.windows(2) {
                assert!(
                    enc(&w[0]) < enc(&w[1]),
                    "{:?} should encode < {:?}",
                    w[0],
                    w[1]
                );
            }
        }
    }

    #[test]
    fn encode_index_key_time_is_chronological() {
        // Midnight < mid-morning < the 24:00:00 endpoint, order-preserving under
        // byte compare (plain big-endian, always non-negative → no sign flip).
        let midnight = Value::Time(0);
        let morning = Value::Time(9 * 3_600_000_000);
        let endpoint = Value::Time(86_400_000_000);
        assert!(enc(&midnight) < enc(&morning));
        assert!(enc(&morning) < enc(&endpoint));
        // Same shape/width as an Int8 key (8 bytes).
        assert_eq!(enc(&midnight).len(), 8);
    }

    #[test]
    fn encode_index_key_null_is_empty() {
        // NULL indexes to the empty key (indexed but never equality-matched).
        assert_eq!(enc(&Value::Null), Vec::<u8>::new());
    }

    /// A float keys in 18.6's `float8_cmp_internal` order, across both widths and the
    /// values an unsigned compare of the raw bits would misplace.
    #[test]
    fn encode_index_key_floats_follow_float8_cmp_internal() {
        let ascending = [
            Value::Float8(f64::NEG_INFINITY),
            Value::Float8(-f64::MAX),
            Value::Float4(-2.5),
            Value::Float8(-1.5),
            Value::Float8(-f64::MIN_POSITIVE),
            Value::Float8(-5e-324),
            Value::Float8(0.0),
            Value::Float8(5e-324),
            Value::Float4(f32::MIN_POSITIVE),
            Value::Float8(1.5),
            Value::Float4(2.5),
            Value::Float8(f64::MAX),
            Value::Float4(f32::INFINITY),
            Value::Float8(f64::NAN),
        ];
        for w in ascending.windows(2) {
            assert!(
                enc(&w[0]) < enc(&w[1]),
                "{:?} should encode < {:?}",
                w[0],
                w[1]
            );
        }
        assert_eq!(enc(&Value::Float8(1.5)).len(), 8);
    }

    /// Values `float8_cmp_internal` calls equal share one key — what a unique index
    /// enforces and an equality probe matches.
    #[test]
    fn encode_index_key_floats_equal_under_the_opclass_share_a_key() {
        let zero = enc(&Value::Float8(0.0));
        assert_eq!(enc(&Value::Float8(-0.0)), zero);
        assert_eq!(enc(&Value::Float4(-0.0)), zero);
        // Every NaN, either sign and any payload: `Infinity - Infinity` carries the
        // sign bit on x86, and an arithmetic NaN's sign is not fixed under wasm.
        let nan = enc(&Value::Float8(f64::NAN));
        for bits in [
            0xfff8_0000_0000_0000_u64,
            0x7ff8_0000_0000_0001,
            0x7ff0_0000_0000_0001,
            0xffff_ffff_ffff_ffff,
        ] {
            assert_eq!(enc(&Value::Float8(f64::from_bits(bits))), nan, "{bits:#x}");
        }
        assert_eq!(enc(&Value::Float4(f32::NAN)), nan);
        assert_eq!(enc(&Value::Float4(-f32::NAN)), nan);
        assert!(nan > enc(&Value::Float8(f64::INFINITY)));
        // A `float4` keys as the `float8` it widens to: equal where the widened value
        // is, and not where the narrow value only rounds to the same digits.
        assert_eq!(enc(&Value::Float4(1.5)), enc(&Value::Float8(1.5)));
        assert_ne!(enc(&Value::Float4(1.1)), enc(&Value::Float8(1.1)));
        assert_eq!(
            enc(&Value::Float4(1.1)),
            enc(&Value::Float8(f64::from(1.1_f32)))
        );
    }

    fn jsonb(text: &str) -> Value {
        Value::Jsonb(super::super::parse_jsonb(text).expect("valid json"))
    }

    fn int_array(elements: &[Option<i32>]) -> Value {
        Value::Array(ArrayValue::new(
            TypeOid::Int4,
            elements
                .iter()
                .map(|e| e.map_or(Value::Null, Value::Int4))
                .collect(),
        ))
    }

    /// 18.6's `ORDER BY` over these `jsonb` values, measured: `[]` below every
    /// top-level scalar, type rank, an array by length before its elements, an object
    /// by pair count before its pairs. The strings are chosen so that no collation
    /// reorders them.
    #[test]
    fn encode_index_key_jsonb_follows_the_measured_order() {
        let ascending = [
            "[]",
            "null",
            r#""""#,
            r#""a""#,
            r#""z""#,
            "-1",
            "0",
            "100",
            "false",
            "true",
            "[null]",
            r#"["a"]"#,
            "[1]",
            "[true]",
            "[[]]",
            "[{}]",
            "[0,0]",
            "[[1],3]",
            "[[1,2],3]",
            "{}",
            r#"{"a":null}"#,
            r#"{"a":1}"#,
            r#"{"aa":1}"#,
            r#"{"b":1}"#,
            r#"{"a":1,"b":1}"#,
            r#"{"b":1,"aa":2}"#,
            r#"{"b":2,"aa":1}"#,
        ];
        for w in ascending.windows(2) {
            assert!(
                enc(&jsonb(w[0])) < enc(&jsonb(w[1])),
                "{} should encode < {}",
                w[0],
                w[1]
            );
        }
    }

    /// Values 18.6's `jsonb` equality folds share one key: a number's scale and
    /// exponent form, at any depth.
    #[test]
    fn encode_index_key_jsonb_folds_numeric_scale() {
        for (a, b) in [
            ("1", "1.0"),
            ("0", "-0"),
            ("100", "1e2"),
            ("[1.0]", "[1]"),
            (r#"{"a":1.00}"#, r#"{"a":1}"#),
        ] {
            assert_eq!(enc(&jsonb(a)), enc(&jsonb(b)), "{a} and {b}");
        }
        assert_ne!(enc(&jsonb(r#""a""#)), enc(&jsonb(r#""A""#)));
    }

    /// 18.6's `array_cmp` order, measured: elements before length, a NULL element
    /// above every value, two NULL elements equal.
    #[test]
    fn encode_index_key_arrays_follow_the_measured_order() {
        let ascending = [
            int_array(&[]),
            int_array(&[Some(-1), Some(5), Some(5)]),
            int_array(&[Some(1)]),
            int_array(&[Some(1), Some(2)]),
            int_array(&[Some(1), Some(2), Some(3)]),
            int_array(&[Some(1), None]),
            int_array(&[Some(2)]),
            int_array(&[None]),
            int_array(&[None, Some(1)]),
            int_array(&[None, None]),
        ];
        for w in ascending.windows(2) {
            assert!(
                enc(&w[0]) < enc(&w[1]),
                "{:?} should encode < {:?}",
                w[0],
                w[1]
            );
        }
        assert_eq!(
            enc(&int_array(&[Some(1), None])),
            enc(&int_array(&[Some(1), None]))
        );
        // An element's key is the element type's own: a float's `-0` is `0` and every
        // `NaN` one key, a `numeric`'s scale folds, and an empty string is not a NULL
        // and not the end of the array.
        let of = |ty, elements: Vec<Value>| enc(&Value::Array(ArrayValue::new(ty, elements)));
        assert_eq!(
            of(TypeOid::Float8, vec![Value::Float8(-0.0)]),
            of(TypeOid::Float8, vec![Value::Float8(0.0)])
        );
        let text = |s: &str| Value::Text(s.to_string());
        let empty = of(TypeOid::Text, vec![]);
        let one_empty = of(TypeOid::Text, vec![text("")]);
        let two_empty = of(TypeOid::Text, vec![text(""), text("")]);
        let nul = of(TypeOid::Text, vec![text("\0")]);
        assert!(empty < one_empty && one_empty < two_empty && two_empty < nul);
        assert!(one_empty < of(TypeOid::Text, vec![Value::Null]));
        // `{ab}` against `{a,b}`: an element boundary is not a byte of the element.
        assert!(
            of(TypeOid::Text, vec![text("a"), text("b")]) < of(TypeOid::Text, vec![text("ab")])
        );
    }
}
