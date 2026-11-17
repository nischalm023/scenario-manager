//! Shared opaque keys and Foundry's template-local reference syntax.

pub use worldprotocol_core::derivation::*;
use crate::ids::LocalHandle;
use serde::{Deserialize, Serialize};

/// The same address written inside a template, where the generator has no UUID
/// yet: a local handle stands in its place until a stamp mints one. Expansion
/// rewrites this into a [`RawDerivation`], so nothing downstream ever sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct RawLocalDerivation {
    pub local_ref: LocalHandle,
    pub key: String,
}
