//! Seeded randomness for the deterministic core.
//!
//! Hand-rolled xoshiro256** seeded via splitmix64 rather than pulling the
//! `rand` ecosystem: the RNG sits *inside* the deterministic fold's blast
//! radius, so its byte-for-byte behavior must be owned by this crate, not by
//! a dependency's semver policy. ~60 lines buys immunity to upstream drift.
//!
//! Streams are *derived*, not shared: `derive_stream(seed, tag)` gives every
//! consumer (each fixture, worldgen, ...) its own independent generator, so
//! simulation order can never perturb another consumer's randomness.

#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[derive(Debug, Clone)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    pub fn seed_from(seed: u64) -> Self {
        let mut sm = seed;
        Rng {
            s: [
                splitmix64(&mut sm),
                splitmix64(&mut sm),
                splitmix64(&mut sm),
                splitmix64(&mut sm),
            ],
        }
    }

    /// xoshiro256** core step.
    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Uniform in [0, 1) with 53 bits of precision.
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in 0..n. Modulo bias is negligible for the small n used here.
    pub fn below(&mut self, n: u32) -> u32 {
        debug_assert!(n > 0);
        (self.next_u64() % n as u64) as u32
    }

    /// Uniform integer in lo..=hi.
    pub fn range_i32(&mut self, lo: i32, hi: i32) -> i32 {
        debug_assert!(lo <= hi);
        lo + self.below((hi - lo + 1) as u32) as i32
    }

    /// Gaussian via Box–Muller. Same-build reproducible (the determinism bar
    /// the architecture commits to); not bit-portable across compilers.
    pub fn normal(&mut self, mu: f64, sigma: f64) -> f64 {
        let u1 = self.f64().max(f64::MIN_POSITIVE);
        let u2 = self.f64();
        let z = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
        mu + sigma * z
    }

    /// Poisson via Knuth's method — fine for the small λ of goal counts.
    pub fn poisson(&mut self, lambda: f64) -> u32 {
        let l = (-lambda).exp();
        let mut k = 0u32;
        let mut p = 1.0f64;
        loop {
            p *= self.f64();
            if p <= l {
                return k;
            }
            k += 1;
            if k > 30 {
                return 30; // pathological λ guard; unreachable at game values
            }
        }
    }

    /// Fisher–Yates.
    pub fn shuffle<T>(&mut self, slice: &mut [T]) {
        for i in (1..slice.len()).rev() {
            let j = self.below((i + 1) as u32) as usize;
            slice.swap(i, j);
        }
    }
}

/// An independent stream for `(world seed, tag)`. Tags are stable constants
/// per consumer (e.g. a fixture id) so replays and forks agree.
pub fn derive_stream(seed: u64, tag: u64) -> Rng {
    let mut sm = seed ^ tag.wrapping_mul(0xA24B_AED4_963E_E407);
    let mixed = splitmix64(&mut sm);
    Rng::seed_from(mixed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Rng::seed_from(42);
        let mut b = Rng::seed_from(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn derived_streams_are_independent_of_consumption_order() {
        let mut s1 = derive_stream(7, 100);
        let _burn: u64 = (0..500)
            .map(|_| derive_stream(7, 99).next_u64())
            .fold(0u64, |acc, x| acc.wrapping_add(x));
        let mut s2 = derive_stream(7, 100);
        for _ in 0..100 {
            assert_eq!(s1.next_u64(), s2.next_u64());
        }
    }
}