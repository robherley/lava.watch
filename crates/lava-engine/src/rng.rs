//! Tiny deterministic RNG (xoshiro128+) — avoids pulling in `rand` so the
//! engine has zero runtime deps and compiles trivially to wasm32.

pub struct Rng {
    s: [u32; 4],
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let lo = seed as u32;
        let hi = (seed >> 32) as u32;
        let mut r = Rng {
            s: [lo | 1, hi | 1, 0xdead_beef, 0xcafe_f00d],
        };
        for _ in 0..16 {
            r.next_u32();
        }
        r
    }

    pub fn next_u32(&mut self) -> u32 {
        let result = self.s[0].wrapping_add(self.s[3]);
        let t = self.s[1] << 9;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(11);
        result
    }

    pub fn f32_unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn f32_range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32_unit()
    }
}

/// Derive a well-scattered seed from a raw entropy value (wall-clock time, a
/// counter, `Date.now()`, …). A splitmix64 finalizer, so even near-identical
/// inputs — two sessions opened in the same instant — map to unrelated seeds.
///
/// The engine can't *gather* entropy portably (no clock on
/// `wasm32-unknown-unknown`), so each host supplies its own and runs it
/// through this shared mixer.
pub fn seed_from(entropy: u64) -> u64 {
    let mut x = entropy;
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::seed_from;

    #[test]
    fn seed_from_scatters_adjacent_inputs() {
        // Deterministic …
        assert_eq!(seed_from(42), seed_from(42));
        // … but consecutive inputs land far apart.
        assert_ne!(seed_from(1), seed_from(2));
        assert_ne!(seed_from(0), seed_from(1));
    }
}
