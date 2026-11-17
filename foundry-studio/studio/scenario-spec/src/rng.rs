//! Within `scenario-spec`, `rng` is the generator's random streams (spec 07 section 3.2; lead decided): SplitMix64,
//! each stream seeded from the workload's seed and the stream's number, so seeds 0 and 1 give two streams (dessim's
//! xorshift maps both to one) and every draw is integer arithmetic or `libm`, which gives the same bits on any
//! platform, so one generator body gives one workload from every build.

use studio_protocol::Seed;

const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
const STREAM_SPREAD: u64 = 0xD1B5_4A32_D192_ED03;
const UNIT: f64 = 1.0 / (1u64 << 53) as f64;

/// Which stream of a seed a draw comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamOf {
    /// Each order's line count and SKUs, in arrival order.
    Lines = 0,
    /// Random arrivals' exponential gaps.
    Arrivals = 1,
}

pub(crate) struct Stream {
    state: u64,
}

impl Stream {
    pub(crate) fn new(seed: Seed, of: StreamOf) -> Self {
        Self { state: seed.0 ^ (of as u64 + 1).wrapping_mul(STREAM_SPREAD) }
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GAMMA);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1), from the draw's top 53 bits.
    pub(crate) fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * UNIT
    }

    /// Uniform over 0..n, the high half of the draw times n.
    pub(crate) fn below(&mut self, n: u64) -> u64 {
        ((u128::from(self.next_u64()) * u128::from(n)) >> 64) as u64
    }

    pub(crate) fn exponential(&mut self, mean: f64) -> f64 {
        -mean * libm::log(1.0 - self.unit())
    }

    /// Knuth's product of uniforms, exact for the means a line count takes (under 50).
    pub(crate) fn poisson(&mut self, mean: f64) -> u32 {
        let floor = libm::exp(-mean);
        let (mut count, mut product) = (0, self.unit());
        while product > floor {
            count += 1;
            product *= self.unit();
        }
        count
    }
}
