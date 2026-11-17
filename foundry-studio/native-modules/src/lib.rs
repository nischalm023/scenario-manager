//! The modules this repository compiles into the simulator, beside the ones a user will script. It holds the record
//! types of the typed streams a run reads, and, in `migration`, the tasks. A task registers its name with `#[task]`, so
//! a program that runs a simulation, or reads a spec or a recording that names tasks, links this crate
//! (`use native_modules as _;`), or it finds no task by name.

pub mod migration;
pub mod stream;

// The files in `migration` name the core as `crate::…` and each other as `crate::flows::…`; these two imports resolve
// those paths. They go when `migration` is empty.
use dessim::*;
use migration::flows;
