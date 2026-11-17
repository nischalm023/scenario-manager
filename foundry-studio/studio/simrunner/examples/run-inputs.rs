//! Runs one attempt's `inputs/` folder the way simrunner would, into a folder of your choosing, so a run can be
//! repeated by hand and compared with desvis headless on the same inputs (the parity check):
//! `cargo run -p simrunner --example run-inputs -- <attempt folder>/inputs/spec.json <out folder> [threads]`.

use std::env;
use std::path::PathBuf;

use dessim::harness::{RunOverrides, prepare};
use dessim::sim_types::Counters;
use simrunner::simulate::simulate;
use simrunner::stop::RunnerStop;
use studio_protocol::{AttemptNumber, Ulid};

fn main() {
    let mut args = env::args().skip(1);
    let spec = PathBuf::from(args.next().expect("the spec.json"));
    let out = PathBuf::from(args.next().expect("the output folder"));
    let threads = args.next().map(|text| text.parse().expect("a thread count"));
    std::fs::create_dir_all(&out).expect("the output folder made");
    let run: Ulid = "01KZAVPQNES65HV4EZADPCS7E8".parse().expect("a ULID");
    let prepared = prepare(&spec, &RunOverrides { num_threads: threads, ..RunOverrides::default() })
        .unwrap_or_else(|fault| panic!("the spec does not prepare: {fault}"));
    let ending = simulate(prepared, &out, &run, AttemptNumber(1), RunnerStop::new(), Counters::new(), None)
        .expect("the run simulates")
        .ending;
    println!("{}", serde_json::to_string_pretty(&ending).expect("JSON"));
}
