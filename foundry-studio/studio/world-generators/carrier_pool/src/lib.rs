//! The carrier-pool generator: a carrier pool becomes the carriers it counts —
//! one per index, each of the pool's carrier type, under the pool's permanent
//! designation and its placement rule.
//!
//! What it is FOR is giving each carrier an identity of its own. A pool is
//! written as a type, a designation, a count and a rule for where they start,
//! because a thousand totes written out by hand say one thing a thousand times;
//! but a curated pre-fill, a damaged tote, or one a run follows has to be
//! something a record can name. So each index becomes an entity, keyed
//! `carrier:{i}` under the pool, exactly as the reference reader derives it.
//!
//! It copies, and reads nothing the pool names: a carrier carries the
//! references its pool holds — the carrier type, and the sector a placement
//! names — and nothing of what they name. A copied reference is read as the
//! reader reads an identity (`seed_identity`), so a pool the reader refuses is
//! refused here in the reader's sentence, and one the reader writes back in
//! another spelling is written back in that spelling here.

use seed_identity::Identity;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use worldgen_sdk::{export_generator, Derived, Generator, Key, Object};

/// The view this generator wants its input through. It travels in the manifest,
/// so a host that has never heard of carriers can still find its objects.
const LENS: &str = r#"
// Every carrier pool, whatever type it names. A carrier copies the reference to
// its type and reads nothing of the type, so binding the type as well would be
// asking for what no carrier depends on.
pattern pooled { p: carrier_pool; }
lens pooled { anchor p: pooled; }
"#;

/// A population of carriers of one type, as the reference reader reads one:
/// every field of the component, in the shape the reader demands.
///
/// `count` is `u64` where the reader's is `usize`, which is the one place this
/// view must not copy the reader's spelling: this is compiled for
/// `wasm32-wasip2`, where `usize` is 32 bits, and the reader's is 64. Read as
/// `usize` here, a count past four billion would be refused while
/// deserialising where the reader takes it.
#[derive(Debug, Deserialize)]
struct CarrierPool {
    carrier_type: Identity,
    designation: String,
    count: u64,
    #[serde(default)]
    placement: Option<Placement>,
}

/// Where a pool's carriers start, written back onto every carrier as the
/// reader writes it: the rule's kind, the sector it names if it names one —
/// absent rather than `null` when it names none — and every other field as
/// stated.
#[derive(Debug, Deserialize, Serialize)]
struct Placement {
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sector: Option<Identity>,
    #[serde(flatten)]
    rest: Map<String, Value>,
}

struct CarrierPoolGenerator;

impl Generator for CarrierPoolGenerator {
    const NAME: &'static str = "carrier_pool";
    const VERSION: &'static str = "0.1.0";
    const LENS: &'static str = LENS;
    // The whole of what a carrier carries: nothing a document writes adds to
    // it, so the lower bound the contract asks for is the whole list.
    const WRITES: &'static [&'static str] = &["carrier"];

    fn generate(object: &Object) -> Result<Vec<Derived>, String> {
        let anchor = object.anchor();
        let pool: CarrierPool = anchor.require("carrier_pool")?;
        let mut out = Vec::new();
        for index in 0..pool.count {
            let mut carrier = json!({
                "carrier_type": pool.carrier_type,
                "pool": anchor.id(),
                "index": index,
                "designation": pool.designation,
            });
            if let Some(placement) = &pool.placement {
                carrier["placement"] = json!(placement);
            }
            out.push(Derived::at(Key::new(format!("carrier:{index}"))?).with("carrier", carrier));
        }
        Ok(out)
    }
}

export_generator!(CarrierPoolGenerator);
