//! Packaged products and the pieces they are made of: templates and the stamps
//! that place them, the station-shaped components a template assembles, and the
//! provenance a cataloged product carries.
//!
//! SPEC §6's bet is that an EasyLoop is not a primitive — it is a template in a
//! catalog document built out of cells, queues, ports and presentation points,
//! so a new station kind is a new document rather than a schema change. This
//! module holds the machinery that bet needs: a template's parameters and
//! internals, a stamp's bindings and repetition, and the component names a
//! station's parts carry. `lineage` sits here for the same reason — it is what a
//! *product* says about where it came from, whether that product is a template,
//! a carrier type, or a robot class in the fleet catalog next door.

use crate::derivation::{RawDerivation, RawLocalDerivation};
use crate::ids::{EntityId, LocalHandle};
use crate::units::{Coord, Steps};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A reusable sub-world: internals named by local handles instead of UUIDs,
/// positioned in the template's own local frame, parameterized where the product
/// varies.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Template {
    #[serde(default)]
    pub params: BTreeMap<String, ParamDecl>,
    #[serde(default)]
    pub entities: Vec<TemplateEntity>,
}

/// A parameter's declaration. A parameter with no default must be bound by every
/// stamp; leaving one unbound is a load error rather than a silent zero.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct ParamDecl {
    #[serde(default)]
    pub default: Option<Value>,
}

/// One entity inside a template, addressed by its handle.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct TemplateEntity {
    pub local: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub components: BTreeMap<String, Value>,
}

/// One placement of a template, optionally repeated.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Stamp {
    pub template: EntityId,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    #[serde(default)]
    pub repeat: Option<Repeat>,
}

/// Instance `i` sits at `pose.position + i·step`. The step is expressed in the
/// frame of the stamp's pose and is deliberately **not** rotated by the stamp's
/// heading, so a rotated row marches along the same world axis as an unrotated
/// one and the two rows span the same extent (SPEC §6).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Repeat {
    pub count: usize,
    pub step: [Coord; 2],
}

/// A workstation: what it does and which product it is.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Station {
    pub kind: String,
    #[serde(default)]
    pub role: Option<String>,
}

/// Where things enter or leave the world.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Port {
    pub direction: String,
    #[serde(default)]
    pub of: Vec<Value>,
    /// The station a sink port feeds.
    #[serde(default)]
    pub into: Option<EntityId>,
    /// The station a source port draws from.
    #[serde(default)]
    pub from: Option<EntityId>,
}

/// How anything in the format names one cell (SPEC §3). Never a bare key string:
/// a key alone would only mean something relative to whichever entity happened
/// to carry it, and the ordinary case — a station queueing over floor it does
/// not own — could not be written at all.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum CellAddress {
    /// A generator and the key beneath it: how a derived cell is named.
    Derived(RawDerivation),
    /// The template-internal form. Inside a template the generator has no UUID
    /// to name, so a local handle stands in and expansion rewrites it — the same
    /// rule a template's own stitch endpoints follow.
    Local(RawLocalDerivation),
    /// The template-internal form of `Id`: a local handle alone, naming a cell
    /// the template declares itself, which expansion rewrites to that cell's
    /// stamped identity. Carried because this type spells every form SPEC §3
    /// gives a cell address (SPEC-GAPS 37); outside a template the placeholder
    /// walk refuses one before any stage reads it.
    Handle(LocalRef),
    /// A cell by its own identity, which is the only way to name one that was
    /// authored directly: a declared cell has no generator and so no derivation
    /// address. Written as a bare UUID string.
    Id(EntityId),
    /// A cell by WHERE IT IS: a position in the addressing entity's frame,
    /// bound to the nearest cell the way a relay's stand point already is.
    ///
    /// This is the form to prefer. A `Derived` address names a column and a row,
    /// which is a coordinate into a decomposition the document had to choose in
    /// advance -- so it silently means somewhere else if the pitch or extent
    /// ever changes, and a queue path has no `expect` pin to catch that. A
    /// position survives any decomposition, because it never mentions one.
    At([i64; 2]),
}

/// A local handle alone, `{"local_ref": "stand"}`, closed to that one key as
/// the schema's `local_ref` is, with only the `$comment` SPEC §2 lets stand
/// anywhere beside it. Without the closure the untagged `CellAddress` would
/// read a handle beside anything else as this, a local address whose key is a
/// parameter among them, and drop what stood beside it.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct LocalRef {
    pub local_ref: LocalHandle,
    #[serde(rename = "$comment", default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// An ordered path of cell addresses with a discipline.
///
/// There is no admission count beside the path, because the path IS the
/// admission: every cell it names admits an occupant, and a scenario admitting
/// fewer states a shorter path. A separate count stated the same fact a second
/// time and let the two disagree -- and the consumer that mattered never read
/// it, taking a queue's depth to be its path length all along.
///
/// There is no `into` beside the path either, and for the same reason. A queue
/// used to be able to name the thing it fed; what it feeds is the LAST CELL OF
/// ITS OWN PATH (Daniel, 2026-09-02 and 2026-09-04), so the field said a second
/// time what the path already said and could disagree with it. No document in
/// the tree ever wrote one -- `lens/patterns/world.lens`'s `queue_into_station`
/// is the pattern that says so, and it answers zero everywhere.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Queue {
    #[serde(default)]
    pub path: Vec<CellAddress>,
    #[serde(default)]
    pub discipline: Option<String>,
}

/// A holding pool whose capacity is simply its cell count.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Waiting {}

/// A PICK PLACE: where an operator takes goods out of an inventory carrier
/// presented beside them and puts those goods into the carrier an HTM is
/// holding out.
///
/// The happening, on the cell it happens at. It is what a `station`'s
/// `kind: "pick"` said from one entity away, and saying it here is what lets a
/// machine be a frame with cells posed in it and no `station` component
/// anywhere. Fieldless on purpose: which carrier is held out is said by the
/// `presentation_point` on the same cell, and what is picked is said by the
/// order the carrier belongs to.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Pick {}

/// An INDUCT PLACE: where an operator puts goods into a carrier presented
/// beside them.
///
/// The opposite direction from [`Pick`], and fieldless for the same reason.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Induct {}

/// Where a carrier is presented to an operator.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct PresentationPoint {
    #[serde(rename = "for", default)]
    pub for_: Option<String>,
}

/// A shapeless named grouping used as a selector for behaviours. A generator
/// listed as a member stands for its whole expansion.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Sector {
    #[serde(default)]
    pub members: Vec<EntityId>,
}

/// A demand source: what the world is asked to do, bound to external data.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct Demand {
    pub kind: String,
    #[serde(default)]
    pub source: Option<Value>,
}

/// A carrier's physical type. Inventory-versus-order is a per-instance
/// designation, not a different type.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct CarrierType {
    pub kind: String,
    #[serde(default)]
    pub size: Option<Vec<Steps>>,
    #[serde(default)]
    pub self_carrying: bool,
}

/// How a cataloged product says where it came from: an ordered list, origin
/// first, one entry per party that handled it (SPEC §10.1).
///
/// The motivating case is one product with three parties behind it — Quicktron
/// manufactures the A5, Dematic distributes it as the DA50, GreyOrange
/// integrates it into a site — which is one class with one set of capabilities
/// and three names. That is a live commercial fact rather than a join key into a
/// legacy system, which is why it is not `external_ids`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct Lineage(pub Vec<LineageEntry>);

/// One party's part in a product's history.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct LineageEntry {
    /// The organization's name in words; v0 has no entity for an organization.
    pub party: String,
    pub role: LineageRole,
    /// The product's code in *that party's own* catalog.
    #[serde(default)]
    pub designation: Option<String>,
    /// The options this party actually fitted — as-built, never a catalog of
    /// what the party offers.
    #[serde(default)]
    pub options: Vec<LineageOption>,
}

/// What a party did to the product, from the closed set SPEC §10.1 fixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum LineageRole {
    Manufacturer,
    Distributor,
    Integrator,
    Operator,
}

/// One fitting, on the entry of the party that applied it.
///
/// An option *describes*; it never restates or overrides what the class states
/// flat, because one capability readable in two places is one capability two
/// readers can disagree about (SPEC §10.1).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(test, derive(schemars::JsonSchema))]
pub struct LineageOption {
    /// In human words, and required: an option nobody can name is not one.
    pub name: String,
    /// The option's code in the naming party's catalog — the same word as the
    /// entry-level field on purpose, because it is one concept at two scales.
    #[serde(default)]
    pub designation: Option<String>,
    /// Whose catalog the option came out of, when that is somebody other than
    /// the party that applied it.
    #[serde(default)]
    pub offered_by: Option<String>,
    /// Free text naming the stock item this displaced.
    #[serde(default)]
    pub replaces: Option<String>,
    /// An open description of the option. It describes; it does not rule.
    #[serde(default)]
    pub detail: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A bare handle is exactly the handle, with a comment or without one.
    /// Anything else beside it, a key a parameter supplies included, makes it no
    /// cell address at all rather than a handle with the rest dropped.
    #[test]
    fn a_bare_handle_is_closed_to_its_one_key() {
        for value in
            [json!({ "local_ref": "stand" }), json!({ "local_ref": "stand", "$comment": "the template's own" })]
        {
            let address: CellAddress = serde_json::from_value(value.clone()).expect("a bare handle reads");
            assert!(
                matches!(address, CellAddress::Handle(LocalRef { ref local_ref, .. }) if local_ref.0 == "stand"),
                "{value}"
            );
        }
        for value in [
            json!({ "local_ref": "patch", "key": { "$param": "queue" } }),
            json!({ "local_ref": "patch", "key": ["cell", 1, 0] }),
            json!({ "local_ref": "stand", "depth": 2 }),
        ] {
            assert!(serde_json::from_value::<CellAddress>(value.clone()).is_err(), "{value}");
        }
    }
}
