//! Within `flowgraph`, `parameters` is spec 06 section 3.4's parameter list: every scalar field the four components
//! state, labelled from one static field catalogue, with its value, unit and clock, whether a sweep may vary it, a
//! closed field's options in the customer's words with the simulator's value in brackets, and the all-in companion
//! beside a station figure. The Sweep group chooses from it (spec 07), a run lists it among its inputs (spec 08
//! section 3.12), and `patch` lets only what it marks sweepable through.

use serde::Serialize;
use serde_json::Value;
use site_model::ids::EntityId;

use super::all_in::{AllIn, all_in};
use super::{Accepts, Compiled};
use crate::worldmap::flows::FlowScope;

/// Which clock a field's value counts in; durations and instants are simtime in whole milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Clock {
    Sim,
}

/// One field of the catalogue: where it is (`*` for a list index), its words, and what a sweep may do with it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    pub pointer: &'static str,
    pub label: &'static str,
    pub short_label: &'static str,
    pub unit: Option<&'static str>,
    pub clock: Option<Clock>,
    /// Numbers, booleans and closed values may vary; never a reference, a kind, what an edge moves, a structural
    /// closed value or a display-only figure.
    pub sweepable: bool,
}

const fn field(pointer: &'static str, label: &'static str, short_label: &'static str) -> Field {
    Field { pointer, label, short_label, unit: None, clock: None, sweepable: false }
}

const fn swept(
    pointer: &'static str,
    label: &'static str,
    short_label: &'static str,
    unit: Option<&'static str>,
) -> Field {
    Field { pointer, label, short_label, unit, clock: None, sweepable: true }
}

const fn timed(pointer: &'static str, label: &'static str, short_label: &'static str, sweepable: bool) -> Field {
    Field { pointer, label, short_label, unit: Some("ms"), clock: Some(Clock::Sim), sweepable }
}

const PER_HOUR: Option<&str> = Some("per hour");

pub const CATALOGUE: [Field; 58] = [
    field("/flow.node/kind", "kind of place", "kind"),
    field("/flow.node/sectors/*", "sector", "sector"),
    field("/flow.node/process/operation", "operation", "operation"),
    field("/flow.node/process/gives_by/*", "edge bringing what units come out of", "gives by"),
    field("/flow.node/process/receives_by/*", "edge bringing what units go into", "receives by"),
    field("/flow.node/process/receives_in", "where orders sit", "receives in"),
    field("/flow.node/holds", "what rests here", "holds"),
    field("/flow.node/demand", "demand stream", "demand"),
    timed("/flow.node/work/pick_sim_ms", "time per pick", "pick", true),
    timed("/flow.node/work/pallet_swap_sim_ms", "pallet swap", "pallet swap", true),
    swept("/flow.node/work/sequencing", "sequencing", "sequencing", None),
    swept("/flow.node/work/pace_to_station", "pacing to the station", "pacing", None),
    swept("/flow.node/work/fairness", "consecutive picks from one order at most", "fairness", None),
    swept("/flow.node/work/layer_size", "totes per layer", "layer", None),
    swept("/flow.node/work/max_open_orders", "open orders at most", "open orders", None),
    swept("/flow.node/work/presentations_per_hour", "fill pace per station", "pace", PER_HOUR),
    timed("/flow.node/work/extraction_sim_ms", "time per extraction", "extraction", true),
    swept("/flow.node/work/isolated", "isolated (an experiment switch)", "isolated", None),
    timed("/flow.node/work/dwell_sim_ms", "dwell", "dwell", true),
    timed("/flow.node/work/tote_swap_sim_ms", "tote swap (display only)", "tote swap", false),
    field("/flow.node/work/map_figures", "the map's own figures", "map figures"),
    field("/flow.node/intake/*/moves", "intake carrier", "intake"),
    swept("/flow.node/intake/*/admission_factor", "admission factor", "admission", None),
    swept("/flow.node/intake/*/max_inflight", "in flight at most", "in flight", None),
    swept("/flow.node/deposit", "slot choice", "deposit", None),
    field("/flow.material_edge/from", "from", "from"),
    field("/flow.material_edge/to", "to", "to"),
    field("/flow.material_edge/moves", "what moves", "moves"),
    field("/flow.material_edge/when/container/*", "container admitted", "container"),
    field("/flow.material_edge/when/exit_path/*", "exit path admitted", "exit path"),
    field("/flow.material_edge/when/routing/*", "routing admitted", "routing"),
    field("/flow.material_edge/when/phase", "phase", "phase"),
    field("/flow.material_edge/alternative_to", "alternative to", "alternative to"),
    field("/flow.material_edge/bias", "bias", "bias"),
    timed("/flow.material_edge/bias/fixed_sim_ms", "fixed bias", "bias", true),
    timed("/flow.material_edge/opens_at_sim_ms", "opens at", "opens", true),
    timed("/flow.material_edge/until_sim_ms", "until", "until", true),
    field("/flow.material_edge/storage_scope/*", "storage sector", "scope"),
    field("/flow.material_edge/returns_by", "return rule", "return rule"),
    swept("/flow.material_edge/induction/sequence", "induction sequence", "sequence", None),
    swept("/flow.material_edge/induction/hour_gate", "hour gate", "hour gate", None),
    timed("/flow.material_edge/induction/lead_time_sim_ms", "lead time", "lead time", true),
    swept("/flow.material_edge/induction/units_per_tote", "units per inbound tote", "units per tote", None),
    field("/flow.agent_segment/edge", "edge", "edge"),
    field("/flow.agent_segment/step", "step", "step"),
    field("/flow.agent_segment/from/node", "from node", "from"),
    field("/flow.agent_segment/from/port", "from port", "from port"),
    field("/flow.agent_segment/to/node", "to node", "to"),
    field("/flow.agent_segment/to/port", "to port", "to port"),
    field("/flow.agent_segment/executor/cohort", "cohort", "cohort"),
    field("/flow.agent_segment/executor/assigned", "cohort assigned at", "assigned"),
    field("/flow.agent_segment/executor/conveyance", "conveyance", "conveyance"),
    field("/flow.agent_segment/executor/external", "nothing modelled", "external"),
    field("/flow.agent_segment/carries", "carries", "carries"),
    field("/flow.agent_segment/at_end", "at the end", "at end"),
    field("/flow.agent_segment/retained_for/*", "kept for", "kept for"),
    field("/flow.return_rule/min_backlog", "backlog, which no run reads", "backlog"),
    field("/flow.return_rule/destination", "where a returning tote goes", "destination"),
];

/// The catalogue entry for a concrete pointer, list indices matching `*`.
pub fn lookup(pointer: &str) -> Option<&'static Field> {
    CATALOGUE.iter().find(|field| {
        let (mut want, mut have) = (field.pointer.split('/'), pointer.split('/'));
        loop {
            match (want.next(), have.next()) {
                (None, None) => return true,
                (Some("*"), Some(index)) if !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()) => {}
                (Some(a), Some(b)) if a == b => {}
                _ => return false,
            }
        }
    })
}

/// One closed value a field may take, in the customer's words with the simulator's value in brackets.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Choice {
    pub value: &'static str,
    pub label: &'static str,
}

/// A closed field's choices: for sequencing only what both the chosen runner image and this build run.
fn options(pointer: &str, accepts: &Accepts) -> Vec<Choice> {
    let choices: &[Choice] = match pointer.rsplit('/').next() {
        Some("sequencing") => &[
            Choice { value: "all_totes", label: "Any case order (all totes)" },
            Choice { value: "sequenced", label: "Line order (sequenced)" },
        ],
        Some("deposit") => &[
            Choice { value: "ranked", label: "Best-scoring slot (ranked)" },
            Choice { value: "random", label: "Any eligible slot, drawn at random (random)" },
        ],
        Some("sequence") => &[
            Choice { value: "sku_demand", label: "By demand for each SKU (sku_demand)" },
            Choice { value: "order_release", label: "By order release (order_release)" },
        ],
        _ => &[],
    };
    let sequencing = pointer.ends_with("/sequencing");
    choices.iter().filter(|choice| !sequencing || accepts.allows_sequencing(choice.value)).cloned().collect()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Parameter {
    #[serde(rename = "ref")]
    pub reference: EntityId,
    pub pointer: String,
    /// The entity's label and the field's: "Put stations, fill pace per station".
    pub label: String,
    pub short_label: &'static str,
    pub value: Value,
    pub unit: Option<&'static str>,
    pub clock: Option<Clock>,
    pub sweepable: bool,
    pub options: Vec<Choice>,
    pub all_in: Option<AllIn>,
}

/// Every scalar field `flows` states, in identity order and, within an entity, in the order of its JSON.
pub fn parameters(flows: &FlowScope, compiled: &Compiled, accepts: &Accepts) -> Vec<Parameter> {
    let mut out = Vec::new();
    for entity in &flows.entities {
        for (component, value) in entity.flow_components() {
            let mut leaves = Vec::new();
            scalars(value, &format!("/{component}"), &mut leaves);
            for (pointer, value) in leaves {
                let Some(field) = lookup(&pointer) else { continue };
                let node = compiled.graph.node(&entity.id);
                let companion = node.and_then(|node| all_in(&node.spec)).filter(|all_in| all_in.field == pointer);
                out.push(Parameter {
                    reference: entity.id.clone(),
                    label: format!("{}, {}", compiled.labels.of(&entity.id), field.label),
                    short_label: field.short_label,
                    value,
                    unit: field.unit,
                    clock: field.clock,
                    sweepable: field.sweepable,
                    options: options(&pointer, accepts),
                    all_in: companion,
                    pointer,
                });
            }
        }
    }
    out
}

/// Every scalar leaf below `value`, by JSON pointer.
fn scalars(value: &Value, at: &str, out: &mut Vec<(String, Value)>) {
    match value {
        Value::Object(fields) => {
            for (name, field) in fields.iter().filter(|(name, _)| !name.starts_with('$')) {
                scalars(field, &format!("{at}/{}", name.replace('~', "~0").replace('/', "~1")), out);
            }
        }
        Value::Array(items) => {
            items.iter().enumerate().for_each(|(index, item)| scalars(item, &format!("{at}/{index}"), out))
        }
        scalar => out.push((at.to_owned(), scalar.clone())),
    }
}
