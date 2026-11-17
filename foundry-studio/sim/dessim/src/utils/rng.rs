//! The crate's deterministic pseudo-randomness: a seeded xorshift64.
//!
//! Every random choice a run makes comes from one of these, seeded from a
//! configured value, so the same seed replays the same run. There is no
//! unseeded source anywhere — a draw off the clock or off the OS would make
//! two runs of the same scenario incomparable, which is the one thing a
//! simulation's randomness must never do.
//!
//! Lifted verbatim from `scenario_gen::orders`, which had the only copy.

/// Simple deterministic PRNG (xorshift64).
pub struct Rng(u64);

impl Rng {
    /// Seed zero would make xorshift emit zeroes forever, so it maps to one;
    /// every other seed is used as given.
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn index(&mut self, len: usize) -> usize {
        (self.next_u64() % len as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_replays_the_same_draws() {
        let draws = |seed| {
            let mut rng = Rng::new(seed);
            (0..8).map(|_| rng.next_u64()).collect::<Vec<_>>()
        };
        assert_eq!(draws(0), draws(0), "seed 0 is reproducible");
        assert_ne!(draws(0), draws(7), "different seeds diverge");
    }

    /// Seed 0 is the configured default, and xorshift is degenerate at zero.
    #[test]
    fn seed_zero_does_not_collapse_to_a_constant() {
        let mut rng = Rng::new(0);
        let first = rng.next_u64();
        assert_ne!(first, 0);
        assert_ne!(rng.next_u64(), first);
    }

    #[test]
    fn index_stays_inside_the_length() {
        let mut rng = Rng::new(42);
        for _ in 0..64 {
            assert!(rng.index(5) < 5);
        }
    }
}
