//! Within `flowgraph`, `all_in` is the all-in companion of spec 06 section 3.4 (lean 12): vendors state station paces
//! all-in, the tote swap included, and the settings leave the swap out, so beside a station figure of a node that
//! states `tote_swap_sim_ms` the panel shows the rate a station reaches with the swap added. Display only: the
//! compiler never reads the swap and the plan never carries it.

use serde::Serialize;

use super::components::{NodeSpec, Operation};
use super::consumers::seconds;
use super::quantity::PerHour;
use crate::simtime::SimDuration;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AllIn {
    /// The station figure it stands beside.
    pub field: String,
    pub value: PerHour,
    pub unit: &'static str,
    pub text: String,
}

/// A rate as the sentences write it: to a tenth, a whole number without its point ("400", "211.8").
fn rate(per_hour: f64) -> String {
    let tenths = (per_hour * 10.0).round() / 10.0;
    if tenths.fract() == 0.0 { format!("{tenths:.0}") } else { format!("{tenths:.1}") }
}

fn per_hour(cycle: SimDuration) -> f64 {
    3_600_000.0 / cycle.as_ms() as f64
}

/// The all-in rate of a pick or put node that states a tote swap; none for any other node, or where the cycle is
/// no time at all.
pub fn all_in(node: &NodeSpec) -> Option<AllIn> {
    let work = &node.work;
    let swap = work.tote_swap_sim_ms?;
    match node.operation()? {
        Operation::Pick => {
            let pick = work.pick_sim_ms?;
            let cycle = pick + swap;
            let value = PerHour::new(per_hour(cycle))?;
            let text = format!(
                "{} operator time + {} tote swap = {}, {} an hour per station, all-in",
                seconds(pick),
                seconds(swap),
                seconds(cycle),
                rate(value.get())
            );
            Some(AllIn { field: "/flow.node/work/pick_sim_ms".into(), value, unit: "per hour", text })
        }
        Operation::Put => {
            let pace = work.presentations_per_hour?;
            // The fill interval as PutawayInduction truncates it to whole milliseconds (`composition/policy.rs`).
            let interval = SimDuration::ms((3600.0 / pace.get() * 1000.0) as u64);
            let value = PerHour::new(per_hour(interval + swap))?;
            let text = format!("{} an hour fill pace = {} an hour all-in", rate(pace.get()), rate(value.get()));
            Some(AllIn { field: "/flow.node/work/presentations_per_hour".into(), value, unit: "per hour", text })
        }
        Operation::Extract | Operation::CreateContainer => None,
    }
}
