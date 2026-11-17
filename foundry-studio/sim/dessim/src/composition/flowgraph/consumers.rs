//! Within `flowgraph`, `consumers` gathers for each value a run holds once (a root's configuration field, or a
//! setting the plan's block states) every flow entity and field that feeds it (spec 06 section 3.2, where each
//! value lands, and the defaults section 3.1 writes down). Root inference takes the value; two entities stating
//! different values for one holder are `consumer-conflict`, since the run would keep only one of them.

use site_model::ids::EntityId;

use super::components::{Bias, Deposit, Holds, Moves, Operation};
use super::graph::{Graph, Node};
use super::label::Labels;
use super::problem::{Code, Problem};
use super::quantity::{AdmissionFactor, InFlightCap, OpenOrderCap, PerHour, UnitsPerTote};
use super::roles::{self, Root, Served};
use crate::composition::InductionSequence;
use crate::simtime::{SimDuration, SimOffset, SimTime};

/// One flow entity's statement of a value: where, what, and how a sentence shows it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Stated<T> {
    pub entity: EntityId,
    pub field: String,
    pub value: T,
    pub shown: String,
}

/// Every statement of one value, in identity order, with the words a conflict needs: what the value is, which
/// field holds it today, and which root will read it per flow.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Feed<T> {
    what: &'static str,
    holder: &'static str,
    reader: &'static str,
    pub stated: Vec<Stated<T>>,
}

impl<T> Feed<T> {
    fn new(what: &'static str, holder: &'static str, reader: &'static str) -> Self {
        Self { what, holder, reader, stated: Vec::new() }
    }

    fn add(&mut self, entity: &EntityId, field: String, value: T, shown: String) {
        self.stated.push(Stated { entity: entity.clone(), field, value, shown });
    }

    pub fn first(&self) -> Option<&Stated<T>> {
        self.stated.first()
    }
}

impl<T: PartialEq> Feed<T> {
    fn conflict(&self, labels: &Labels) -> Option<Problem> {
        let first = self.stated.first()?;
        let other = self.stated.iter().find(|stated| stated.value != first.value)?;
        let text = format!(
            "{} and {} state different {} ({}, {}); {} holds one until {} reads its flows.",
            labels.of(&first.entity),
            labels.of(&other.entity),
            self.what,
            first.shown,
            other.shown,
            self.holder,
            self.reader,
        );
        Some(Problem::error(Code::ConsumerConflict, &other.entity, Some(other.field.clone()), text))
    }
}

/// Seconds as section 3.4's sentences write them: "7.0 s", "6.77 s".
pub fn seconds(duration: SimDuration) -> String {
    let ms = duration.as_ms();
    let decimals = if ms.is_multiple_of(100) {
        1
    } else if ms.is_multiple_of(10) {
        2
    } else {
        3
    };
    format!("{:.*} s", decimals, ms as f64 / 1000.0)
}

fn instant(at: SimTime) -> String {
    format!("simtime {}", seconds(SimDuration::ms(at.as_ms())))
}

fn optional<T>(value: Option<T>, show: impl Fn(T) -> String) -> String {
    value.map_or_else(|| "none".to_owned(), show)
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Consumers {
    pub time_per_pick: Feed<SimDuration>,
    pub pallet_swap: Feed<SimDuration>,
    pub sequencing: Feed<String>,
    pub pace_to_station: Feed<bool>,
    pub pick_admission: Feed<AdmissionFactor>,
    pub max_inflight: Feed<Option<InFlightCap>>,
    pub pick_opens: Feed<SimTime>,
    pub fill_pace: Feed<PerHour>,
    pub put_admission: Feed<AdmissionFactor>,
    pub induction_sequence: Feed<InductionSequence>,
    pub hour_gate: Feed<bool>,
    pub lead_time: Feed<Option<SimDuration>>,
    pub units_per_tote: Feed<UnitsPerTote>,
    pub rack_cutoff: Feed<Option<SimTime>>,
    pub direct_bias: Feed<Option<SimOffset>>,
    pub random_slots: Feed<bool>,
    pub extraction: Feed<SimDuration>,
    pub bypass_extraction: Feed<bool>,
    pub bypass_box_induction: Feed<bool>,
    pub buffer_dwell: Feed<SimDuration>,
    pub loop_admission: Feed<AdmissionFactor>,
    pub max_open_orders: Feed<Option<OpenOrderCap>>,
    pub loop_opens: Feed<SimTime>,
}

/// The intake entry for `moves` and its pointer.
fn intake(node: &Node, moves: Moves) -> Option<(String, AdmissionFactor, Option<InFlightCap>)> {
    let at = node.spec.intake.iter().position(|entry| entry.moves == moves)?;
    let entry = &node.spec.intake[at];
    Some((format!("/flow.node/intake/{at}"), entry.admission_factor, entry.max_inflight))
}

impl Consumers {
    pub fn gather(graph: &Graph) -> Self {
        let mut c = Consumers {
            time_per_pick: Feed::new("times per pick", "operator.time_per_pick_secs", "PickDemand"),
            pallet_swap: Feed::new("pallet swaps", "operator.pallet_swap_secs", "PickDemand"),
            sequencing: Feed::new("sequencing", "PickDemand.sequencing", "PickDemand"),
            pace_to_station: Feed::new("pacing to the station", "PickDemand.pace_to_station", "PickDemand"),
            pick_admission: Feed::new("stock-tote admission factors", "PickDemand.queue_factor", "PickDemand"),
            max_inflight: Feed::new("in-flight caps", "PickDemand.max_inflight_picks", "PickDemand"),
            pick_opens: Feed::new("pick openings", "PickDemand.embargo_secs", "PickDemand"),
            fill_pace: Feed::new("fill paces", "PutawayInduction.max_presentations_per_hour", "PutawayInduction"),
            put_admission: Feed::new("empty-tote admission factors", "putaway.htm_wip_factor", "PutawaySupplyDemand"),
            induction_sequence: Feed::new("induction sequences", "PutawayInduction.sequence", "PutawayInduction"),
            hour_gate: Feed::new("hour gates", "putaway.ignore_hour_gate", "PutawayInduction"),
            lead_time: Feed::new("lead times", "PutawayInduction.lead_time_secs", "PutawayInduction"),
            units_per_tote: Feed::new("units per tote", "PutawayInduction.units_per_tote", "PutawayInduction"),
            rack_cutoff: Feed::new("rack cut-offs", "putaway.forbid_rack_after_secs", "PutawaySupplyDemand"),
            direct_bias: Feed::new(
                "direct-empty biases",
                "PutawaySupplyDemand.direct_bias_secs",
                "PutawaySupplyDemand",
            ),
            random_slots: Feed::new("deposit rules", "putaway.random_slots", "the relay service"),
            extraction: Feed::new("extraction times", "FulfillOrders.extraction_secs", "FulfillOrders"),
            bypass_extraction: Feed::new("isolation switches", "FulfillOrders.bypass_extraction", "FulfillOrders"),
            bypass_box_induction: Feed::new(
                "isolation switches",
                "FulfillOrders.bypass_box_induction",
                "FulfillOrders",
            ),
            buffer_dwell: Feed::new("buffer dwells", "FulfillOrders.buffer_dwell_secs", "FulfillOrders"),
            loop_admission: Feed::new("stock-tote admission factors", "FulfillOrders.queue_factor", "FulfillOrders"),
            max_open_orders: Feed::new(
                "open-order caps",
                "FulfillOrders.max_active_orders_per_machine",
                "FulfillOrders",
            ),
            loop_opens: Feed::new("pick openings", "pick.embargo_secs", "FulfillOrders"),
        };
        for node in &graph.nodes {
            c.node(node);
        }
        for edge in &graph.edges {
            let spec = &edge.spec;
            let field = |name: &str| format!("/flow.material_edge/{name}");
            let opens = spec.opens_at_sim_ms.unwrap_or(SimTime::ZERO);
            match roles::served_by(graph, edge) {
                Served::Root(Root::PickDemand) => {
                    c.pick_opens.add(&edge.id, field("opens_at_sim_ms"), opens, instant(opens))
                }
                Served::Root(Root::FulfillOrders) if spec.moves == Moves::StockTote => {
                    c.loop_opens.add(&edge.id, field("opens_at_sim_ms"), opens, instant(opens))
                }
                Served::Root(Root::PutawaySupplyDemand) if spec.alternative_to.is_some() => {
                    let fixed = match spec.bias {
                        Some(Bias::Fixed { fixed_sim_ms }) => Some(fixed_sim_ms),
                        _ => None,
                    };
                    let shown = optional(fixed, |bias| format!("{} ms", bias.as_ms()));
                    c.direct_bias.add(&edge.id, field("bias"), fixed, shown);
                }
                Served::Root(Root::PutawaySupplyDemand) => {
                    let until = spec.until_sim_ms;
                    c.rack_cutoff.add(&edge.id, field("until_sim_ms"), until, optional(until, instant));
                }
                Served::Root(Root::PutawayInduction) => {
                    let Some(induction) = spec.induction else { continue };
                    let field = |name: &str| format!("/flow.material_edge/induction/{name}");
                    let sequence = induction.sequence.unwrap_or_default();
                    c.induction_sequence.add(&edge.id, field("sequence"), sequence, format!("{sequence:?}"));
                    let gate = induction.hour_gate.unwrap_or(true);
                    c.hour_gate.add(&edge.id, field("hour_gate"), gate, gate.to_string());
                    let lead = induction.lead_time_sim_ms;
                    c.lead_time.add(&edge.id, field("lead_time_sim_ms"), lead, optional(lead, seconds));
                    let units = induction.units_per_tote.unwrap_or(UnitsPerTote::ONE);
                    c.units_per_tote.add(&edge.id, field("units_per_tote"), units, units.get().to_string());
                }
                _ => {}
            }
        }
        c
    }

    fn node(&mut self, node: &Node) {
        let (id, work) = (&node.id, &node.spec.work);
        let field = |name: &str| format!("/flow.node/work/{name}");
        if roles::is_pick(node)
            && let Some(time) = work.pick_sim_ms
        {
            self.time_per_pick.add(id, field("pick_sim_ms"), time, seconds(time));
        }
        if roles::is_stations(node, Operation::Pick) {
            let swap = work.pallet_swap_sim_ms.unwrap_or(SimDuration::ZERO);
            self.pallet_swap.add(id, field("pallet_swap_sim_ms"), swap, seconds(swap));
            let sequencing = work.sequencing.clone().unwrap_or_else(|| "all_totes".to_owned());
            self.sequencing.add(id, field("sequencing"), sequencing.clone(), sequencing);
            let pace = work.pace_to_station.unwrap_or(false);
            self.pace_to_station.add(id, field("pace_to_station"), pace, pace.to_string());
            if let Some((at, factor, cap)) = intake(node, Moves::StockTote) {
                self.pick_admission.add(id, format!("{at}/admission_factor"), factor, factor.get().to_string());
                let shown = optional(cap, |cap| cap.get().to_string());
                self.max_inflight.add(id, format!("{at}/max_inflight"), cap, shown);
            }
        }
        if roles::is_stations(node, Operation::Put) {
            if let Some(pace) = work.presentations_per_hour {
                self.fill_pace.add(id, field("presentations_per_hour"), pace, format!("{} an hour", pace.get()));
            }
            if let Some((at, factor, _)) = intake(node, Moves::EmptyTote) {
                self.put_admission.add(id, format!("{at}/admission_factor"), factor, factor.get().to_string());
            }
        }
        if roles::is_storage(node, Holds::InventoryTote) {
            let random = node.spec.deposit == Some(Deposit::Random);
            let shown = if random { "random" } else { "ranked" };
            self.random_slots.add(id, "/flow.node/deposit".into(), random, shown.into());
        }
        if roles::is_stations(node, Operation::Extract) {
            if let Some(time) = work.extraction_sim_ms {
                self.extraction.add(id, field("extraction_sim_ms"), time, seconds(time));
            }
            let isolated = work.isolated.unwrap_or(false);
            self.bypass_extraction.add(id, field("isolated"), isolated, isolated.to_string());
        }
        if roles::is_stations(node, Operation::CreateContainer) {
            let isolated = work.isolated.unwrap_or(false);
            self.bypass_box_induction.add(id, field("isolated"), isolated, isolated.to_string());
        }
        if roles::is_storage(node, Holds::ClosedOrder)
            && let Some(dwell) = work.dwell_sim_ms
        {
            self.buffer_dwell.add(id, field("dwell_sim_ms"), dwell, seconds(dwell));
        }
        if roles::is_pick_loop(node) {
            if let Some((at, factor, _)) = intake(node, Moves::StockTote) {
                self.loop_admission.add(id, format!("{at}/admission_factor"), factor, factor.get().to_string());
            }
            let cap = work.max_open_orders;
            let shown = optional(cap, |cap| cap.get().to_string());
            self.max_open_orders.add(id, field("max_open_orders"), cap, shown);
        }
    }

    /// `consumer-conflict` for every holder fed two values.
    pub fn conflicts(&self, labels: &Labels) -> Vec<Problem> {
        [
            self.time_per_pick.conflict(labels),
            self.pallet_swap.conflict(labels),
            self.sequencing.conflict(labels),
            self.pace_to_station.conflict(labels),
            self.pick_admission.conflict(labels),
            self.max_inflight.conflict(labels),
            self.pick_opens.conflict(labels),
            self.fill_pace.conflict(labels),
            self.put_admission.conflict(labels),
            self.induction_sequence.conflict(labels),
            self.hour_gate.conflict(labels),
            self.lead_time.conflict(labels),
            self.units_per_tote.conflict(labels),
            self.rack_cutoff.conflict(labels),
            self.direct_bias.conflict(labels),
            self.random_slots.conflict(labels),
            self.extraction.conflict(labels),
            self.bypass_extraction.conflict(labels),
            self.bypass_box_induction.conflict(labels),
            self.buffer_dwell.conflict(labels),
            self.loop_admission.conflict(labels),
            self.max_open_orders.conflict(labels),
            self.loop_opens.conflict(labels),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}
