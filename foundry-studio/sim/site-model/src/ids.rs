//! Shared identities and Foundry's template-local handles.

pub use worldprotocol_core::ids::*;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A template-internal name. Inside a template an entity has no identity at all;
/// it has a handle its siblings reference with `{"local_ref": ...}`, and
/// expansion turns the handle into the key segment `local:{handle}`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct LocalHandle(pub String);

impl fmt::Display for LocalHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
