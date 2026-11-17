//! The component vocabulary: the named meanings an entity can carry.
//!
//! SPEC §10 fixes what each component name means; this module is where those
//! meanings become Rust types the pipeline can compute with. Two rules shape
//! everything here. First, a typed view is a *reading* of a raw JSON value, not
//! a replacement for it — [`crate::entity::Entity`] keeps the original so an
//! unknown field survives a bake. Second, none of these structs deny unknown
//! fields, because SPEC §9 makes ignoring them a requirement rather than a
//! kindness; what a struct does refuse is a field that is present and the wrong
//! shape, since that is a producer bug, not a version skew.
//!
//! The `#[cfg_attr(test, derive(schemars::JsonSchema))]` on the structs in these
//! four modules — and on the scalars, identities and addresses they are built
//! from — has one consumer, [`schema_sync`]. It derives each struct's field
//! census so that the JSON Schemas can be compared against the structs
//! themselves instead of against a hand-kept list of field names. Nothing
//! outside a test build sees it.

pub mod fleet;
pub mod product;
pub mod spatial;
pub mod structure;

/// The comparison that keeps `schema/` and these structs saying one thing.
#[cfg(test)]
mod schema_sync;

/// Every component name whose meaning this implementation knows.
///
/// A document's `required_components` is checked against this list: a consumer
/// that does not understand a required semantic must refuse the document rather
/// than mis-simulate it (SPEC §9, after glTF's `extensionsRequired`). Names
/// derived entities carry (`relay`, `slot`, `robot`, `carrier`) are listed too,
/// so a baked document round-trips through the same check.
///
/// `external_ids` is listed with no struct behind it, and that is not an
/// omission. SPEC §2 fixes its meaning — the identifiers other systems know an
/// entity by, joinable and never a reference target — and fixes no keys, since
/// every key is another system's spelling. There is nothing for a typed view to
/// read, and knowing the name is the whole of knowing the meaning.
pub const KNOWN_COMPONENTS: &[&str] = &[
    "access",
    "access_cell",
    "carrier",
    "carrier_pool",
    "carrier_type",
    "cell",
    "cohort",
    "demand",
    "external_ids",
    "flow",
    "frame",
    "induct",
    "lineage",
    "charging",
    "pick",
    "pose",
    "port",
    "presentation_point",
    "queue",
    "rack",
    "region",
    "relay",
    "robot",
    "robot_class",
    "sector",
    "slot",
    "stamp",
    "station",
    "stitch",
    "storage",
    "template",
    "tiling",
    "waiting",
];

/// Whether this implementation knows what a component name means. Dot-namespaced
/// product components (`go.easyloop`) are deliberately not known: they are
/// carried verbatim and ignored, which is the whole point of namespacing them.
pub fn is_known(component: &str) -> bool {
    KNOWN_COMPONENTS.contains(&component)
}

/// The components a document cannot be read without understanding: the
/// generator inputs — `tiling` among them, and it decides which cells exist,
/// carve-outs and all — plus the one that decides which cells are joined, plus
/// the junction that says which class works which face. A
/// consumer blind to any of these does not fail — it quietly produces a smaller
/// world, or a world whose steel nobody is declared to reach — which is exactly
/// the mis-simulation `required_components` exists to prevent (SPEC §9).
pub const LOAD_BEARING: &[&str] = &[
    "access",
    "carrier_pool",
    "cohort",
    "flow",
    "frame",
    "rack",
    "region",
    "stamp",
    "stitch",
    "template",
    "tiling",
];

/// The components SPEC §8 step 2 calls *generator inputs*: replacing one in a
/// later layer changes what expansion produces, so the merge must finish before any
/// expansion starts.
///
/// `access` left this list in v0.6 and the omission is the point: an access
/// derives nothing now, so replacing one changes what the checks say and never
/// what exists.
pub const GENERATOR_INPUTS: &[&str] = &[
    "carrier_pool",
    "cohort",
    "flow",
    "frame",
    "rack",
    "region",
    "stamp",
    "template",
    "tiling",
];
