//! What refusing to load looks like, and what merely complaining looks like.
//!
//! The protocol's central bet is that a broken reference fails loudly instead of
//! contributing nothing (SPEC §2, §3 — the OpenUSD failure mode it is written
//! against). So this module carries two vocabularies that must not blur into
//! each other: [`LoadError`], which stops the load and names the record that
//! caused it precisely enough to find in a text editor; and [`Warning`], which
//! rides along on a successful result because the document is loadable but says
//! something its author probably did not mean.

use worldprotocol_core::LoadError as Core;
use crate::ids::{DerivedEntityId, DocId, EntityId};
use crate::units::Steps;
use serde_json::Error as JsonError;
use std::io::Error as IoError;
use std::path::PathBuf;
use thiserror::Error;

/// A refusal to load. Every variant names the offending record, entity, or
/// address, because "invalid document" is exactly the diagnostic this format
/// exists to replace.
#[derive(Debug, Error)]
pub enum LoadError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: IoError,
    },

    #[error("{path} is not valid JSON: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: JsonError,
    },

    #[error("{path}: {message}")]
    Shape { path: PathBuf, message: String },

    #[error("{path} declares proto_version {found}; this implementation reads 0.7.x")]
    ProtoVersion { path: PathBuf, found: String },

    #[error("{path} carries the retired field `{field}`; use {replacement}")]
    RetiredField {
        path: PathBuf,
        field: &'static str,
        replacement: &'static str,
    },

    /// A flagged export and an authored document are told apart by their
    /// keys, so a file carrying both has no one reading: either reader would
    /// drop what the other reads.
    #[error(
        "{path} carries {flagged}, the keys of a site's flagged export, beside an authored \
         document's {authored}: a file is one or the other"
    )]
    FlaggedBesideAuthored {
        path: PathBuf,
        flagged: String,
        authored: String,
    },

    /// A world is the site whole as WPS generated it, so it is built only
    /// from a flagged export that states all three of its flags.
    #[error(
        "{path} is a flagged export that does not state {} as true, and a world is built only \
         from a site's export with all three flags: ask WPS for \
         `?nested=1&derived=1&referenced=1`",
        spoken(unset)
    )]
    ExportFlagsUnset {
        path: PathBuf,
        unset: Vec<&'static str>,
    },

    #[error("the inline expansion in document {doc} is stale at {address}: {detail}")]
    StaleExpansion {
        doc: DocId,
        address: String,
        detail: String,
    },

    #[error("scenario lists the layer {path} more than once")]
    DuplicateLayer { path: PathBuf },

    #[error("layer {path} composes itself, directly or through another layer")]
    LayerCycle { path: PathBuf },

    #[error("document {doc} depends on document {dependency}, which no layer provides")]
    MissingDependency { doc: DocId, dependency: DocId },

    #[error(
        "document {doc} requires the component `{component}`, whose meaning this \
         implementation does not know"
    )]
    UnknownRequiredComponent { doc: DocId, component: String },

    #[error("{doc}: record #{index} has neither `id` nor `derivation`, so it addresses nothing")]
    RecordWithoutTarget { doc: DocId, index: usize },

    #[error("{doc}: record #{index} carries both `suppress` and `components`")]
    SuppressWithComponents { doc: DocId, index: usize },

    #[error("{doc}: records #{first} and #{second} both address {target}")]
    DuplicateTarget {
        doc: DocId,
        first: usize,
        second: usize,
        target: String,
    },

    #[error("{doc}: patch of entity {entity}, which no earlier layer declared")]
    PatchMissesTarget { doc: DocId, entity: EntityId },

    #[error("{doc}: record #{index} states id {stated} but addresses {address}")]
    DerivationIdMismatch {
        doc: DocId,
        index: usize,
        stated: EntityId,
        address: String,
    },

    #[error("{kind} addresses {address}, which no generator produces")]
    DanglingDerivation { kind: &'static str, address: String },

    #[error(
        "the `expect` pin on {address} names {expected:?} but the entity was derived at \
         {actual:?}, {distance} away with a tolerance of {tolerance}"
    )]
    ExpectFailed {
        address: String,
        expected: [i64; 2],
        actual: [i64; 2],
        distance: Steps,
        tolerance: Steps,
    },

    #[error(
        "entity {entity}: tiling extent {size} on the {axis} axis is not a positive whole \
         multiple of the pitch {pitch}"
    )]
    TilingNotDivisible {
        entity: EntityId,
        axis: &'static str,
        size: Steps,
        pitch: Steps,
    },

    /// The one thing an old document can say that a silent load would get
    /// wrong. An `exclusion` entity subtracted cells from tilings it never
    /// named; nothing reads the component now, so a document carrying one would
    /// load with every cell it meant to remove still standing.
    #[error(
        "entity {entity} carries an `exclusion` component, which this version of the format no \
         longer has: a tiling states its own carve-outs in its `exclusions` list"
    )]
    ExclusionComponentRemoved { entity: EntityId },

    /// A carve-out whose far corner cannot be computed. Every other length in
    /// the format is a step count that fits, so this is a document stating a
    /// rectangle no floor has — and the alternative to refusing it is a corner
    /// that wrapped around and a hole somewhere nobody asked for.
    #[error(
        "entity {entity}: an `exclusions` rectangle runs off the step lattice — its origin plus \
         its size does not fit in a step count"
    )]
    ExclusionOutOfRange { entity: EntityId },

    #[error(
        "entity {entity} declares geometry in frame {frame}, whose units are `{units}`: an \
         integer lattice frame is for projecting into, never for authoring in"
    )]
    GeometryInLatticeFrame {
        entity: EntityId,
        frame: EntityId,
        units: String,
    },

    #[error(
        "frame {frame} declares `unit_mm`, which only the primary frame may: a step becomes \
         physical in exactly one place"
    )]
    UnitOnNonPrimaryFrame { frame: EntityId },

    #[error(
        "the primary frame {frame} declares no `unit_mm`, so nothing says what a step is worth"
    )]
    PrimaryFrameWithoutUnit { frame: EntityId },

    /// A flagged export names no primary frame: the primary frame is the one
    /// frame that states `unit_mm` (ruling D177), so two that state it leave a
    /// step worth two things.
    #[error(
        "frames {first} and {second} both declare `unit_mm`: the primary frame is the one frame \
         that says what a step is worth, and a site has one"
    )]
    UnitOnTwoFrames { first: EntityId, second: EntityId },

    #[error(
        "cell {entity} states no `size` and no primary frame says what a step is worth, so the \
         900 mm default has no step count"
    )]
    CellSizeWithoutUnit { entity: EntityId },

    #[error(
        "rack {entity}: level {index} sits at {height}, not above level {previous} at \
         {previous_height} — a rack's levels run bottom-up and strictly increasing"
    )]
    RackLevelsNotIncreasing {
        entity: EntityId,
        index: usize,
        height: Steps,
        previous: usize,
        previous_height: Steps,
    },

    #[error(
        "rack {entity}: level {index} states `positions` and so is a relay level, but states no \
         `reach`, without which its relays cannot be bound to a cell"
    )]
    RelayLevelHasNoReach { entity: EntityId, index: usize },

    #[error("entity {entity}: `{component}.{field}` references {target}, which does not exist")]
    DanglingReference {
        entity: EntityId,
        component: String,
        field: String,
        target: EntityId,
    },

    #[error("frame {frame} is its own ancestor")]
    FrameCycle { frame: EntityId },

    #[error("entity {entity} is missing the `{component}` component its role requires")]
    MissingComponent {
        entity: EntityId,
        component: &'static str,
    },

    #[error("entity {entity}: component `{component}` does not parse: {source}")]
    ComponentShape {
        entity: EntityId,
        component: String,
        #[source]
        source: JsonError,
    },

    /// SPEC §3's uniqueness rule, reduced to what can still go wrong. A UUID and
    /// a derivation address can never be the same string, so an authored
    /// identity no longer collides with a derived one; two generators cannot
    /// collide either, since their addresses start at different roots. One
    /// generator emitting one segment twice is what remains.
    #[error("a generator derived the entity {address} twice")]
    DerivedIdCollision { address: String },

    #[error("template {template} references the parameter `{param}`, which it does not declare")]
    UnknownParam { template: EntityId, param: String },

    #[error("stamp {stamp} leaves the parameter `{param}` of template {template} unbound")]
    UnboundParam {
        stamp: EntityId,
        template: EntityId,
        param: String,
    },

    #[error("stamp {stamp} binds `{param}`, which template {template} does not declare")]
    UnexpectedParam {
        stamp: EntityId,
        template: EntityId,
        param: String,
    },

    #[error(
        "template {template} references the local handle `{handle}`, which it does not define"
    )]
    UnknownLocalRef { template: EntityId, handle: String },

    #[error(
        "template {template}: a `local_ref` stands alone or carries exactly a `key` beside it, \
         each holding a string; this one sits among {found}"
    )]
    MalformedLocalRef { template: EntityId, found: String },

    #[error(
        "entity {entity}: queue path step #{index} addresses {address}, which nothing declares \
         and no generator produces"
    )]
    QueuePathDangling {
        entity: EntityId,
        index: usize,
        address: String,
    },

    #[error(
        "entity {entity}: queue path step #{index} addresses {address}, a cell an exclusion or \
         a suppression removed — a queue cannot run through a hole"
    )]
    QueuePathThroughRemovedCell {
        entity: EntityId,
        index: usize,
        address: String,
    },

    #[error("entity {entity}: queue path step #{index} addresses {address}, which is not a cell")]
    QueuePathNotACell {
        entity: EntityId,
        index: usize,
        address: String,
    },

    /// A template's placeholder left outside any template: a parameter wherever
    /// the schema admits one, or, where it types a reference, a local handle
    /// alone or beside a key, or an address whose key is a parameter (SPEC §3,
    /// §6). Only a stamp turns one into what it stands for, so here it stands
    /// for nothing.
    #[error(
        "entity {entity}: `{component}.{field}` still holds {placeholder}, which only a stamp \
         can resolve"
    )]
    UnresolvedPlaceholder { entity: EntityId, component: String, field: String, placeholder: String },

    #[error(
        "entity {entity}: `{field}` addresses a cell by position, which this field cannot \
         resolve -- an explicit stitch pair has to name the two cells it links"
    )]
    PositionAddressUnsupported { entity: EntityId, field: &'static str },

    #[error("template {template} defines the local handle `{handle}` twice")]
    DuplicateLocalHandle { template: EntityId, handle: String },

    #[error(
        "template {template}: the local handle `{handle}` contains `/`, which SPEC §3 reserves"
    )]
    SlashInLocalHandle { template: EntityId, handle: String },

    #[error(
        "relay {address} finds no cell outward of its face within a reach of {reach}, while \
         other relays of the same run bind — a handoff shelf with no floor in front of it"
    )]
    RelayUnbound { address: String, reach: Steps },

    #[error(
        "entity {entity}: `access_cell.cell` addresses {address}, which is not a cell this world \
         holds — a binding an override wrote by hand has to name floor that exists"
    )]
    AccessCellNotALiveCell { entity: EntityId, address: String },

    #[error("stitch {stitch} is a contact seam that produces no links at all")]
    ZeroLinkStitch { stitch: EntityId },

    #[error("stitch {stitch}: endpoint {endpoint} contributes no cells")]
    StitchEndpointHasNoCells {
        stitch: EntityId,
        endpoint: EntityId,
    },

    #[error("stitch {stitch}: explicit pair #{index} does not name exactly two endpoints")]
    MalformedStitchPair { stitch: EntityId, index: usize },

    #[error("derivation key `{key}` is malformed at segment `{segment}`: {reason}")]
    BadDerivationKey {
        key: String,
        segment: String,
        reason: &'static str,
    },

    #[error("`{text}` is not an entity identity: {reason}")]
    BadEntityId { text: String, reason: &'static str },

    #[error(
        "{doc}: record #{index} declares the identity {address}, which is a derivation address — \
         a declared entity's identity is a freshly minted UUID, and a derived one is reached by \
         `derivation`"
    )]
    DeclaredIdIsAnAddress {
        doc: DocId,
        index: usize,
        address: String,
    },

    #[error("entity {entity}: {message}")]
    Unsupported { entity: EntityId, message: String },

    /// WPS reads each scope it holds at one changeset, so a flagged export
    /// lists each identity once; a second listing would make one identity name
    /// two entities.
    #[error("the flagged export lists the entity {entity} twice")]
    ListedTwice { entity: EntityId },

    /// A generated entity lives in its generator's scope, so the declared entity
    /// its identity starts at is held beside it; without it nothing says which
    /// tiling's pitch the entity's footprint is or which expansion it
    /// contributes cells to.
    #[error(
        "the flagged export holds the generated entity {entity} but not {root}, the generator its \
         identity starts at"
    )]
    GeneratedWithoutGenerator { entity: EntityId, root: EntityId },
}

impl LoadError {
    /// The refusal a dangling override or suppression raises (SPEC §3).
    pub fn dangling(kind: &'static str, address: &DerivedEntityId) -> LoadError {
        LoadError::DanglingDerivation {
            kind,
            address: address.to_string(),
        }
    }
}

/// Something loadable that the author probably did not mean. Warnings ride on a
/// successful load rather than stopping it, because every one of them
/// is a judgement about intent, and a validator that refuses on judgement calls
/// is a validator people route around.
#[derive(Debug, Clone, PartialEq)]
pub enum Warning {
    /// SPEC §3: tiling keys are ordinal, so a pitch change re-aims an address at
    /// a different patch of floor. A record that pins no `expect` will follow
    /// the pitch instead of failing.
    UnpinnedTiledCellRecord { kind: &'static str, address: String },

    /// SPEC §8: overrides run after expansion, so replacing a component
    /// the generator reads as *input* changes nothing about what it already
    /// produced. A layer that meant to change the expansion wanted a patch.
    OverrideReplacesGeneratorInput { address: String, component: String },

    /// SPEC §9: the document uses a component whose meaning changes what the
    /// load produces, but does not list it in `required_components` — so a
    /// consumer that has never heard of it will read the document happily and
    /// simulate something else.
    LoadBearingComponentNotRequired { doc: DocId, components: Vec<String> },

    /// SPEC §7: an access names a floor-locomotion class onto an `along` region
    /// that contributes no surviving floor cells, so the class is
    /// declared to work a face it has no ground to stand on.
    FloorServedAlongIsNoCellGrid {
        access: EntityId,
        along: EntityId,
        robot_class: EntityId,
    },

    /// SPEC §7: every relay of one run failed to find an access cell, which is
    /// the ordinary shape of a converted legacy site where relay zones stand
    /// shoulder to shoulder. Steel nobody reaches is a statement about the site,
    /// not an authoring mistake — one relay failing while its neighbours bind
    /// still refuses the load.
    SteelNobodyReaches { run: EntityId, relays: usize },

    /// SPEC §5: two relays standing at different places on their bays bound the
    /// same cell. A cell under each relay stack is the shape the format expects,
    /// so one cell serving two stacks says the floor was laid without the steel
    /// in mind — a misalignment to go and look at, not a document to refuse.
    /// Shelves stacked at one place on one bay share their cell by design and
    /// say nothing here.
    RelaysApartShareOneCell {
        cell: EntityId,
        a: EntityId,
        b: EntityId,
    },

    /// SPEC §4, §12.12: cells that a declared integer frame cannot tell apart,
    /// summarized once per frame rather than once per square.
    ///
    /// One warning per square would be the wrong shape for what this measures.
    /// A converted legacy site folds hundreds of squares for one reason — its
    /// relay pitch and its lattice pitch are incommensurate — and a reader needs
    /// the scale of that loss and a few examples of it, not a list to scroll
    /// past. The count is the finding; the samples are how to go look.
    GridProjectionCollisions {
        frame: EntityId,
        /// Squares claimed by cells no stitch joins.
        squares: usize,
        /// How many cells take part across all of them.
        cells: usize,
        /// The lowest few squares by coordinate, so the sample is the same on
        /// every run and in every implementation.
        samples: Vec<GridCollision>,
    },

    /// SPEC §7: a cohort is hard-assigned somewhere none of whose cells admit
    /// the class it is made of.
    CohortAssignedWhereClassCannotGo {
        cohort: EntityId,
        robot_class: EntityId,
        assigned_to: EntityId,
    },

    /// SPEC §7: a flow rule compiles arcs over cells that admit no agent class
    /// at all, so the routing decision governs floor nothing can occupy.
    FlowOverUninhabitableCells { region: EntityId },

    /// SPEC §5: two cell_grids share a boundary with no stitch between them.
    /// The spec calls this an error; it ships as a warning here because "touch"
    /// has no numeric definition yet (see SPEC-GAPS).
    TouchingRegionsWithoutStitch { a: EntityId, b: EntityId },

    /// SPEC §7: a robot class is taller than the lowest level of a rack whose
    /// footprint a cell_grid admitting that class crosses — it would drive under
    /// steel it does not fit under.
    ClassTallerThanLowestLevel {
        rack: EntityId,
        robot_class: EntityId,
        lowest_level: Steps,
        height: Steps,
    },
}

/// One square an integer frame cannot resolve, and who is standing in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridCollision {
    pub coordinate: [i64; 2],
    pub cells: Vec<String>,
}

impl std::fmt::Display for GridCollision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}, {}] {{{}}}",
            self.coordinate[0],
            self.coordinate[1],
            self.cells.join(", ")
        )
    }
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Warning::UnpinnedTiledCellRecord { kind, address } => write!(
                f,
                "{kind} of the tiled cell {address} carries no `expect` pin, so a pitch change \
                 would re-aim it silently"
            ),
            Warning::OverrideReplacesGeneratorInput { address, component } => write!(
                f,
                "the override of {address} replaces `{component}`, which its generator reads \
                 as input — overrides run after expansion, so the expansion is unchanged"
            ),
            Warning::LoadBearingComponentNotRequired { doc, components } => write!(
                f,
                "document {doc} uses {} but does not list {} in `required_components`, so a \
                 consumer that cannot expand them would read it without saying so",
                components.join(", "),
                if components.len() == 1 { "it" } else { "them" }
            ),
            Warning::FloorServedAlongIsNoCellGrid {
                access,
                along,
                robot_class,
            } => write!(
                f,
                "the access {access} serves the floor class {robot_class} from a face worked \
                 along {along}, which contributes no surviving floor cells: the class has \
                 no cells to stand on"
            ),
            Warning::SteelNobodyReaches { run, relays } => write!(
                f,
                "not one of run {run}'s {relays} relays finds a cell within reach — \
                 steel nobody reaches"
            ),
            Warning::RelaysApartShareOneCell { cell, a, b } => write!(
                f,
                "relays {a} and {b} stand at different places on their bays and both bind cell \
                 {cell}: a cell is expected under each relay stack, so the floor and the steel \
                 are out of step here"
            ),
            Warning::GridProjectionCollisions {
                frame,
                squares,
                cells,
                samples,
            } => write!(
                f,
                "projecting into the integer frame {frame} folds {cells} cells into {squares} \
                 squares that no stitch joins — that much of the layout a consumer of this \
                 frame cannot tell apart; e.g. {}",
                samples
                    .iter()
                    .map(GridCollision::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            Warning::CohortAssignedWhereClassCannotGo {
                cohort,
                robot_class,
                assigned_to,
            } => write!(
                f,
                "cohort {cohort} is hard-assigned to {assigned_to}, none of whose cells admit \
                 the class {robot_class}"
            ),
            Warning::FlowOverUninhabitableCells { region } => write!(
                f,
                "region {region} carries a flow rule but its cells admit no agent class"
            ),
            Warning::TouchingRegionsWithoutStitch { a, b } => write!(
                f,
                "cell_grids {a} and {b} share a boundary with no stitch between them"
            ),
            Warning::ClassTallerThanLowestLevel {
                rack,
                robot_class,
                lowest_level,
                height,
            } => write!(
                f,
                "rack {rack}'s lowest level sits at {lowest_level}, below the {height} of class \
                 {robot_class}, which some cell under the rack's footprint admits"
            ),
        }
    }
}

impl From<Core> for LoadError {
    fn from(error: Core) -> Self {
        match error {
            Core::BadDerivationKey { key, segment, reason } => Self::BadDerivationKey { key, segment, reason },
            Core::BadEntityId { text, reason } => Self::BadEntityId { text, reason },
            Core::MissingComponent { entity, component } => Self::MissingComponent { entity, component },
            Core::ComponentShape { entity, component, source } => Self::ComponentShape { entity, component, source },
        }
    }
}

/// `names` as a sentence names them, each in backticks: "`a`", "`a` and `b`", "`a`, `b` and `c`".
pub(crate) fn spoken(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.as_slice() {
        [rest @ .., last] if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => quoted.concat(),
    }
}
