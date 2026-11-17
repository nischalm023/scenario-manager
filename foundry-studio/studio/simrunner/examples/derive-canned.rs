//! Writes `metrics.json` and `series.json` beside the canned recording in the folder named, derived from it as the
//! crate's test derives them: `cargo run -p simrunner --example derive-canned -- studio/simrunner/fixtures/canned`.

use std::env;
use std::fs;
use std::path::PathBuf;

use simrunner::recording::derive;

fn main() {
    let folder = PathBuf::from(env::args().nth(1).expect("the canned folder"));
    let recording = fs::read(folder.join("recording.worldview")).expect("the recording reads");
    let (metrics, series, stop) = derive(&recording).expect("the recording derives");
    fs::write(folder.join("metrics.json"), serde_json::to_vec(&metrics).expect("JSON")).expect("metrics.json written");
    fs::write(folder.join("series.json"), serde_json::to_vec(&series).expect("JSON")).expect("series.json written");
    println!("{} metrics, stopped at {} ms of simtime: {}", metrics.metrics.len(), stop.at.as_ms(), stop.reason);
}
