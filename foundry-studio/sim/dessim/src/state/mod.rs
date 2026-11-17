//! One instance of each requested manager per simulation run, created lazily.
//! Task constructors retain specific typed handles. Immutable construction
//! inputs are published before use; simulated initialization runs as tasks.

mod resolver;

pub use resolver::{StateError, StateManager, States};
