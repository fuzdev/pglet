//! MD5 (RFC 1321), the digest behind the SQL `md5(text | bytea)` function.
//!
//! In-house rather than a dependency: the algorithm is a page of code, and
//! every dependency is weighed against the wasm size budget (`TRADEOFFS.md`).
//! It is a checksum here, never a security primitive — 18.6's `md5()` is the same.

/// The per-round left-rotation amounts (RFC 1321 § 3.4).
const SHIFTS: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, //
    5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, //
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, //
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// `floor(abs(sin(i + 1)) * 2^32)` for each step `i` (RFC 1321 § 3.4's table `T`).
const SINES: [u32; 64] = [
    0xd76a_a478,
    0xe8c7_b756,
    0x2420_70db,
    0xc1bd_ceee,
    0xf57c_0faf,
    0x4787_c62a,
    0xa830_4613,
    0xfd46_9501,
    0x6980_98d8,
    0x8b44_f7af,
    0xffff_5bb1,
    0x895c_d7be,
    0x6b90_1122,
    0xfd98_7193,
    0xa679_438e,
    0x49b4_0821,
    0xf61e_2562,
    0xc040_b340,
    0x265e_5a51,
    0xe9b6_c7aa,
    0xd62f_105d,
    0x0244_1453,
    0xd8a1_e681,
    0xe7d3_fbc8,
    0x21e1_cde6,
    0xc337_07d6,
    0xf4d5_0d87,
    0x455a_14ed,
    0xa9e3_e905,
    0xfcef_a3f8,
    0x676f_02d9,
    0x8d2a_4c8a,
    0xfffa_3942,
    0x8771_f681,
    0x6d9d_6122,
    0xfde5_380c,
    0xa4be_ea44,
    0x4bde_cfa9,
    0xf6bb_4b60,
    0xbebf_bc70,
    0x289b_7ec6,
    0xeaa1_27fa,
    0xd4ef_3085,
    0x0488_1d05,
    0xd9d4_d039,
    0xe6db_99e5,
    0x1fa2_7cf8,
    0xc4ac_5665,
    0xf429_2244,
    0x432a_ff97,
    0xab94_23a7,
    0xfc93_a039,
    0x655b_59c3,
    0x8f0c_cc92,
    0xffef_f47d,
    0x8584_5dd1,
    0x6fa8_7e4f,
    0xfe2c_e6e0,
    0xa301_4314,
    0x4e08_11a1,
    0xf753_7e82,
    0xbd3a_f235,
    0x2ad7_d2bb,
    0xeb86_d391,
];

/// The MD5 digest of `data`, as the 32 lower-case hex digits 18.6's `md5()` answers.
#[must_use]
pub fn md5_hex(data: &[u8]) -> String {
    let mut out = String::with_capacity(32);
    crate::types::push_lower_hex(&mut out, &md5_digest(data));
    out
}

/// The 16-byte MD5 digest of `data`.
fn md5_digest(data: &[u8]) -> [u8; 16] {
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let (blocks, tail) = data.as_chunks::<64>();
    for block in blocks {
        compress(&mut state, block);
    }
    // The padding: a `1` bit, zeros up to 56 bytes mod 64, then the message length in
    // bits as a little-endian `u64` — one final block, or two when the tail leaves
    // fewer than nine bytes of room.
    let mut last = [0u8; 128];
    last[..tail.len()].copy_from_slice(tail);
    last[tail.len()] = 0x80;
    let len = if tail.len() < 56 { 64 } else { 128 };
    // `usize` is at most 64 bits on every target pglet builds for
    let bits = (data.len() as u64).wrapping_mul(8);
    last[len - 8..len].copy_from_slice(&bits.to_le_bytes());
    for block in last[..len].as_chunks::<64>().0 {
        compress(&mut state, block);
    }
    let mut out = [0u8; 16];
    for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        *chunk = word.to_le_bytes();
    }
    out
}

/// One 64-byte block through the four rounds (RFC 1321 § 3.4).
// `a`–`d` and `f`/`g` are RFC 1321's own names for the round registers
#[allow(clippy::many_single_char_names)]
fn compress(state: &mut [u32; 4], block: &[u8; 64]) {
    let mut words = [0u32; 16];
    for (word, bytes) in words.iter_mut().zip(block.as_chunks::<4>().0) {
        *word = u32::from_le_bytes(*bytes);
    }
    let [mut a, mut b, mut c, mut d] = *state;
    for i in 0..64 {
        let (f, g) = match i / 16 {
            0 => ((b & c) | (!b & d), i),
            1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
            2 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let rotated = a
            .wrapping_add(f)
            .wrapping_add(SINES[i])
            .wrapping_add(words[g])
            .rotate_left(SHIFTS[i]);
        a = d;
        d = c;
        c = b;
        b = b.wrapping_add(rotated);
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
}

#[cfg(test)]
mod tests {
    use super::md5_hex;

    /// RFC 1321 § A.5's test suite, verbatim.
    #[test]
    fn rfc_1321_test_suite() {
        let cases: [(&str, &str); 7] = [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (
                "abcdefghijklmnopqrstuvwxyz",
                "c3fcd3d76192e4007dfb496cca67e13b",
            ),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
                "d174ab98d277d9f5a5611c2c9f419d9f",
            ),
            (
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ];
        for (input, digest) in cases {
            assert_eq!(md5_hex(input.as_bytes()), digest, "md5({input:?})");
        }
    }

    /// The padding's boundary lengths: 55 bytes leaves room for the length in the last
    /// block, 56 through 63 do not and spill into a second, 64 is a whole block with the
    /// padding wholly in the next. Digests from 18.6's `md5(repeat('a', n))`.
    #[test]
    fn padding_boundaries_agree_with_18_6() {
        let cases: [(usize, &str); 5] = [
            (55, "ef1772b6dff9a122358552954ad0df65"),
            (56, "3b0c8ac703f828b04c6c197006d17218"),
            (63, "b06521f39153d618550606be297466d5"),
            (64, "014842d480b571495a4a0363793f7367"),
            (65, "c743a45e0d2e6a95cb859adae0248435"),
        ];
        for (n, digest) in cases {
            assert_eq!(md5_hex(&vec![b'a'; n]), digest, "md5(repeat('a', {n}))");
        }
    }
}
