//! Within `flowgraph`, the `components` are the four `flow.` components of spec 06 section 3.1 as typed values: a
//! node, a material edge, an agent segment and a return rule, each field in its newtype (simtime durations and
//! instants in whole milliseconds, the other numbers in `quantity`'s types), each reference with the pin it was
//! written with. A field the registered schema does not define is refused at the read, as is a value of the wrong
//! shape or outside its bound.

use serde::de::{Error as _, IgnoredAny};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use site_model::ids::EntityId;

use super::quantity::{
    AdmissionFactor, Backlog, Fairness, InFlightCap, LayerSize, OpenOrderCap, PerHour, Step, UnitsPerTote,
};
use crate::composition::InductionSequence;
use crate::model::ExitPath;
use crate::simtime::{SimDuration, SimOffset, SimTime};

/// The four components, by their registered names (ruling D92).
pub const NODE: &str = "flow.node";
pub const EDGE: &str = "flow.material_edge";
pub const SEGMENT: &str = "flow.agent_segment";
pub const RULE: &str = "flow.return_rule";

/// The schema version this compiler reads each of the four at (design/vocabulary, all at 0.1.0).
pub const SCHEMA: &str = "0.1.0";

/// Another entity, as a flow field names it: its identity and the pin it was written with. Within a site a
/// reference reads at the head, bare or `@head` (rulings D60 and D76).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ref {
    pub id: EntityId,
    pub pin: Option<String>,
}

impl Ref {
    pub fn at_head(&self) -> bool {
        self.pin.as_deref().is_none_or(|pin| pin == "head")
    }
}

impl<'de> Deserialize<'de> for Ref {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let (id, pin) = match text.split_once('@') {
            Some((id, pin)) => (id, Some(pin.to_owned())),
            None => (text.as_str(), None),
        };
        Ok(Ref { id: id.parse().map_err(D::Error::custom)?, pin })
    }
}

/// A plan names world entities by identity alone.
impl Serialize for Ref {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.id.serialize(serializer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Stations,
    Storage,
    Conveyance,
    Source,
    Sink,
}

impl NodeKind {
    pub fn word(self) -> &'static str {
        match self {
            NodeKind::Stations => "stations",
            NodeKind::Storage => "storage",
            NodeKind::Conveyance => "conveyance",
            NodeKind::Source => "source",
            NodeKind::Sink => "sink",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Pick,
    Put,
    CreateContainer,
    Extract,
}

impl Operation {
    pub fn word(self) -> &'static str {
        match self {
            Operation::Pick => "pick",
            Operation::Put => "put",
            Operation::CreateContainer => "box induction",
            Operation::Extract => "extraction",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Holds {
    InventoryTote,
    OrderContainer,
    ClosedOrder,
}

impl Holds {
    pub fn word(self) -> &'static str {
        match self {
            Holds::InventoryTote => "inventory totes",
            Holds::OrderContainer => "empty order containers",
            Holds::ClosedOrder => "closed orders",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Demand {
    Putaway,
    Orders,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Deposit {
    Ranked,
    Random,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceivesIn {
    StationBins,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapFigures {
    DisplayOnly,
}

/// What a material edge moves: units without a modelled carrier, or a carrier in one state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Moves {
    #[serde(rename = "units")]
    Units,
    #[serde(rename = "inventory_tote:stock")]
    StockTote,
    #[serde(rename = "inventory_tote:empty")]
    EmptyTote,
    #[serde(rename = "inventory_tote:after_pick")]
    ToteAfterPick,
    #[serde(rename = "order_container:empty")]
    EmptyOrderContainer,
    #[serde(rename = "order_container:new")]
    NewOrderContainer,
    #[serde(rename = "order:closed")]
    ClosedOrder,
    #[serde(rename = "order:in_bin")]
    OrderInBin,
}

impl Moves {
    pub fn is_inventory_tote(self) -> bool {
        matches!(self, Moves::StockTote | Moves::EmptyTote | Moves::ToteAfterPick)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSpec {
    pub kind: NodeKind,
    #[serde(default)]
    pub sectors: Vec<Ref>,
    pub process: Option<Process>,
    pub holds: Option<Holds>,
    pub demand: Option<Demand>,
    #[serde(default)]
    pub work: Work,
    #[serde(default)]
    pub intake: Vec<Intake>,
    pub deposit: Option<Deposit>,
    #[serde(rename = "$comment")]
    pub comment: Option<IgnoredAny>,
}

impl NodeSpec {
    pub fn operation(&self) -> Option<Operation> {
        self.process.as_ref().map(|process| process.operation)
    }

    pub fn intake(&self, moves: Moves) -> Option<&Intake> {
        self.intake.iter().find(|entry| entry.moves == moves)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Process {
    pub operation: Operation,
    #[serde(default)]
    pub gives_by: Vec<Ref>,
    #[serde(default)]
    pub receives_by: Vec<Ref>,
    pub receives_in: Option<ReceivesIn>,
}

/// A node's work figures (ruling D93): simtime durations in whole milliseconds, rates per hour.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Work {
    pub pick_sim_ms: Option<SimDuration>,
    pub pallet_swap_sim_ms: Option<SimDuration>,
    pub sequencing: Option<String>,
    pub pace_to_station: Option<bool>,
    pub fairness: Option<Fairness>,
    pub layer_size: Option<LayerSize>,
    pub max_open_orders: Option<OpenOrderCap>,
    pub presentations_per_hour: Option<PerHour>,
    pub extraction_sim_ms: Option<SimDuration>,
    pub isolated: Option<bool>,
    pub dwell_sim_ms: Option<SimDuration>,
    pub tote_swap_sim_ms: Option<SimDuration>,
    pub map_figures: Option<MapFigures>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intake {
    pub moves: Moves,
    pub admission_factor: AdmissionFactor,
    pub max_inflight: Option<InFlightCap>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeSpec {
    pub from: Ref,
    pub to: Ref,
    pub moves: Moves,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<Guard>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternative_to: Option<Ref>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bias: Option<Bias>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opens_at_sim_ms: Option<SimTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until_sim_ms: Option<SimTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_scope: Option<Vec<Ref>>,
    /// Later tiers, out of the proof of concept (spec 06 section 10): read only to refuse them by name.
    #[serde(skip_serializing)]
    pub storage_fallback: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returns_by: Option<Ref>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub induction: Option<Induction>,
    #[serde(rename = "$comment", skip_serializing)]
    pub comment: Option<IgnoredAny>,
}

/// Which closed orders take an edge, or on a leftover edge the phase it opens in. An absent field admits all.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guard {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<Vec<Container>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_path: Option<Vec<ExitPath>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    Tote,
    Tray,
    Carton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    InboundEnded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Bias {
    Named(BiasName),
    Fixed { fixed_sim_ms: SimOffset },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiasName {
    FleetPressure,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Induction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<InductionSequence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hour_gate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead_time_sim_ms: Option<SimDuration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub units_per_tote: Option<UnitsPerTote>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SegmentSpec {
    pub edge: Option<Ref>,
    pub step: Option<Step>,
    pub from: Place,
    pub to: Place,
    pub executor: Executor,
    #[serde(default)]
    pub carries: Carries,
    pub at_end: AtEnd,
    #[serde(default)]
    pub retained_for: Vec<Ref>,
    #[serde(rename = "$comment")]
    pub comment: Option<IgnoredAny>,
}

impl SegmentSpec {
    pub fn is_empty_move(&self) -> bool {
        self.carries == Carries::Nothing
    }
}

/// A node and one of its ports (spec 06 section 3.1, lean 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Place {
    pub node: Ref,
    pub port: Port,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Port {
    Presentation,
    Slot,
    Relay,
    Dropoff,
    Collect,
    OrderPresentation,
    Boundary,
}

impl Port {
    pub fn word(self) -> &'static str {
        match self {
            Port::Presentation => "presentation",
            Port::Slot => "slot",
            Port::Relay => "relay",
            Port::Dropoff => "drop-off",
            Port::Collect => "collection",
            Port::OrderPresentation => "order presentation",
            Port::Boundary => "boundary",
        }
    }

    /// Where custody may pass from one executor to the next.
    pub fn is_handoff(self) -> bool {
        matches!(self, Port::Relay | Port::Dropoff | Port::Collect)
    }

    /// The ports a node of `kind` has.
    pub fn of(kind: NodeKind) -> &'static [Port] {
        match kind {
            NodeKind::Stations => &[Port::Presentation],
            NodeKind::Storage => &[Port::Slot, Port::Relay],
            NodeKind::Conveyance => &[Port::Dropoff, Port::Collect, Port::OrderPresentation],
            NodeKind::Source | NodeKind::Sink => &[Port::Boundary],
        }
    }
}

/// Who carries: a named cohort, the cohort hard-assigned at one end (resolved per member, ruling D105), a
/// conveyance node's belt, or nothing modelled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Executor {
    Cohort(Ref),
    Assigned(End),
    Conveyance(Ref),
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum End {
    From,
    To,
}

impl<'de> Deserialize<'de> for Executor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Stated {
            cohort: Option<Ref>,
            assigned: Option<End>,
            conveyance: Option<Ref>,
            external: Option<bool>,
        }
        match Stated::deserialize(deserializer)? {
            Stated { cohort: Some(cohort), assigned: None, conveyance: None, external: None } => {
                Ok(Self::Cohort(cohort))
            }
            Stated { cohort: None, assigned: Some(end), conveyance: None, external: None } => Ok(Self::Assigned(end)),
            Stated { cohort: None, assigned: None, conveyance: Some(node), external: None } => {
                Ok(Self::Conveyance(node))
            }
            Stated { cohort: None, assigned: None, conveyance: None, external: Some(true) } => Ok(Self::External),
            _ => {
                Err(D::Error::custom("an executor names exactly one of cohort, assigned, conveyance or external: true"))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Carries {
    #[default]
    Edge,
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtEnd {
    HandsOver,
    SetsDown,
    KeepsThroughWork,
    Arrives,
    Leaves,
}

// TODO(return-rule): `min_backlog` is kept so that documents naming it still load, and no run reads it: each finished
// presentation's clearance makes its own return and batches nothing. Whether the field goes is fwd task M81tugcZ.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSpec {
    pub min_backlog: Backlog,
    #[serde(default)]
    pub destination: Destination,
    #[serde(rename = "$comment")]
    pub comment: Option<IgnoredAny>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Destination {
    #[default]
    NearestEligible,
}
