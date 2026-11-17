//! Simulated time, re-exported from `desunits` at the path the simulator has
//! always used. The types moved out so wasm-safe crates (the output codec, the
//! browser viewer) can share them without depending on the simulator.

pub use desunits::simtime::*;
