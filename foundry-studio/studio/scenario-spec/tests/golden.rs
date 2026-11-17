//! The generator's bytes, frozen (Astra's review of I1.08, point (a)): each case's NDJSON stream is committed under
//! `tests/golden/` beside its sha256 here, and this crate version must make exactly those bytes, so a build on another
//! platform that makes other bytes fails here rather than in a run's `plan-differs`. The cases cover every release,
//! both stock modes, and the draws that go through `libm` (random arrivals' logarithm, each order's line count).
//! A change to the generator changes its crate version and these files together: with `SCENARIO_SPEC_WRITE_GOLDEN`
//! set, the test writes each case's bytes instead of comparing them, and the digests below are copied from its output.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use desunits::SimDuration;
use scenario_spec::{SiteFacts, generate, stated_defaults};
use sha2::{Digest, Sha256};
use studio_protocol::{GeneratorSpec, OrdersPerHour, Release, Seed, SkuCount, SlotCount};

const WRITE: &str = "SCENARIO_SPEC_WRITE_GOLDEN";

struct Case {
    name: &'static str,
    release: Release,
    seed: u64,
    brings_stock_in: bool,
    sha256: &'static str,
}

const CASES: [Case; 4] = [
    Case {
        name: "all-at-start",
        release: Release::AllAtStart,
        seed: 0,
        brings_stock_in: true,
        sha256: "bfdfddc35be1dbff5341fa97cb1ef8cdedba8686f7f7a70e515eba04130fa59a",
    },
    Case {
        name: "even",
        release: Release::Even,
        seed: 1,
        brings_stock_in: false,
        sha256: "9b497c99baf0e57d32d2479f862fac879f9c73826ce7743187c8fb5aa1dd6b89",
    },
    Case {
        name: "random",
        release: Release::Random,
        seed: 7,
        brings_stock_in: true,
        sha256: "58426411b017d03f91c2cf4fa24612c23675dce34725a415408b86222e8f11c4",
    },
    Case {
        name: "waves",
        release: Release::Waves,
        seed: 3,
        brings_stock_in: true,
        sha256: "72e4f53afe32323da100709501a77d63d461ed9857a87e8f5cd60a1bb5816c5b",
    },
];

/// 60 orders an hour of 40 SKUs for one hour of simtime on 120 slots: small files that still draw every stream.
fn spec(case: &Case) -> GeneratorSpec {
    let mut spec = stated_defaults().workload.generator;
    spec.orders_per_hour = OrdersPerHour(60.0);
    spec.sku_count = SkuCount(40);
    spec.release = case.release;
    spec.seed = Seed(case.seed);
    if case.release == Release::Waves {
        spec.wave_every_sim_ms = Some(SimDuration::ms(900_000));
    }
    spec
}

fn file(case: &Case) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{}.ndjson", case.name))
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn every_case_makes_its_committed_bytes_and_digest() {
    for case in &CASES {
        let facts = SiteFacts { slots: SlotCount(120), brings_stock_in: case.brings_stock_in };
        let made = generate(&spec(case), SimDuration::ms(3_600_000), facts);
        let bytes = made.ndjson();
        if env::var_os(WRITE).is_some() {
            fs::write(file(case), &bytes).expect("the golden file is written");
            println!("{}: {}", case.name, made.sha256());
            continue;
        }
        let committed = fs::read(file(case)).expect("the golden file reads");
        assert!(bytes == committed, "{} makes other bytes than {}", case.name, file(case).display());
        assert_eq!((made.sha256().as_str(), hex(&committed).as_str()), (case.sha256, case.sha256), "{}", case.name);
    }
}
