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
