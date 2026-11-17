//! `scenario-spec` is spec 07 section 3.2's one crate for the check and the runner (lead decided: the frozen-plan
//! audit's open point 6): what a scenario version may hold, the seeded generator that makes a run's orders and stock
//! on demand from the version's spec (ruling D96), the map from a version to dessim's run spec, the setup and order
//! kinds the flow compiler takes, the two facts it reads of the site, the closed values this build accepts, a
//! variant's application, and the one seam where the flow compiler joins a run. It does no I/O: its callers hand it a
//! version's content, the run's overrides, one variant, the world document and the site facts, and two builds of one
//! crate version give one answer.

mod compile;
mod content;
mod fragment;
mod generator;
mod rng;
mod site;
mod variant;

pub use compile::{COMPILER, Compilation, Compiled, compile};
pub use content::{
    HOUR_MS, SEED_LIMIT, SLICE_MS, Violation, effective, stated_defaults, validate, validate_generator,
    validate_overrides,
};
pub use fragment::{OrderKind, RUN_FILES, Setup, SpecFiles, order_kinds, setup, spec_fragment};
pub use generator::{
    GENERATOR_VERSION, Generated, REQUEST_VERSION, WorkloadLines, check_request, facts_of, generate, generate_request,
    ndjson_line, request_bytes,
};
pub use site::{PICK_SEQUENCING, RELEASE, SiteFacts, pick_sequencing, site_facts, this_build_accepts, word_of};
pub use variant::{Applied, VariantError, apply_variant, set};

// Both Studio entry points reject the simulator's legacy settings fallback.
pub use dessim::robots::require_catalog_motion;
