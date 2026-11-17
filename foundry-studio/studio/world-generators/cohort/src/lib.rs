//! The cohort generator: a cohort becomes the robots it counts — one per index,
//! each an instance of the cohort's class under the cohort's assignment.
//!
//! What it is FOR is giving each robot an identity of its own. A fleet is
//! written as a class, a count and a policy, because forty identical robots
//! written out by hand are forty chances to get one wrong; but a robot that is
//! down, one placed by hand for a scenario, or one a report singles out has to
//! be something a record can name. So each index becomes an entity, keyed
//! `robot:{i}` under the cohort, exactly as the reference reader derives it.
//!
//! It copies, and reads nothing the cohort names: a robot carries the
//! reference to its class that the cohort holds, and nothing of the class
//! itself. A copied reference is read as the reader reads an identity
//! (`seed_identity`), so a cohort the reader refuses is refused here in the
//! reader's sentence, and one the reader writes back in another spelling is
//! written back in that spelling here.

use seed_identity::Identity;
use serde::{Deserialize, Serialize};
use serde_json::json;
use worldgen_sdk::{export_generator, Derived, Generator, Key, Object};

/// The view this generator wants its input through. It travels in the manifest,
/// so a host that has never heard of fleets can still find its objects.
const LENS: &str = r#"
// Census 21 and 22's cohort without its class: every cohort, whatever class it
// names. A robot copies the reference to its class and reads nothing of the
// class, so binding the class as well would be asking for what no robot
// depends on.
pattern cohorted { c: cohort; }
lens cohorted { anchor c: cohorted; }
"#;

/// N robots of one class plus an assignment policy, as the reference reader
/// reads one: every field of the component, in the shape the reader demands.
///
/// `count` is `u64` where the reader's is `usize`, which is the one place this
/// view must not copy the reader's spelling: this is compiled for
/// `wasm32-wasip2`, where `usize` is 32 bits, and the reader's is 64. Read as
/// `usize` here, a count past four billion would be refused while
/// deserialising where the reader takes it.
#[derive(Debug, Deserialize)]
struct Cohort {
    robot_class: Identity,
    count: u64,
    #[serde(default)]
    assignment: Option<Assignment>,
}

/// Where a cohort's robots may work, written back onto every robot as the
/// reader writes it: an empty `to` and an absent one both read as no entities,
/// and both are written as no `to` at all.
#[derive(Debug, Deserialize, Serialize)]
struct Assignment {
    kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    to: Vec<Identity>,
}

struct CohortGenerator;

impl Generator for CohortGenerator {
    const NAME: &'static str = "cohort";
    const VERSION: &'static str = "0.1.0";
    const LENS: &'static str = LENS;
    // The whole of what a robot carries: nothing a document writes adds to it,
    // so the lower bound the contract asks for is the whole list.
    const WRITES: &'static [&'static str] = &["robot"];

    fn generate(object: &Object) -> Result<Vec<Derived>, String> {
        let anchor = object.anchor();
        let cohort: Cohort = anchor.require("cohort")?;
        let mut out = Vec::new();
        for index in 0..cohort.count {
            let mut robot = json!({ "robot_class": cohort.robot_class, "cohort": anchor.id(), "index": index });
            if let Some(assignment) = &cohort.assignment {
                robot["assignment"] = json!(assignment);
            }
            out.push(Derived::at(Key::new(format!("robot:{index}"))?).with("robot", robot));
        }
        Ok(out)
    }
}

export_generator!(CohortGenerator);
