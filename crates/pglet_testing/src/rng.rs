//! A tiny seedable PRNG for reproducible fuzz/soak loops — no `rand` dependency.

/// Seedable xorshift64. Reproducible: print the seed and re-run with it (the
/// fuzz/soak harnesses read it from an env var). Not for anything cryptographic.
#[derive(Debug, Clone)]
pub struct Xorshift(u64);

impl Xorshift {
    /// A generator seeded from `seed` (forced nonzero, as xorshift requires).
    pub const fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    /// The next 64-bit value.
    pub const fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform in `[0, n)` (`n` must be > 0).
    pub const fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}
