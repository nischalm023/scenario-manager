//! Orders as their sources state them. What happens to an order during a run
//! is dispatch state and lives in the demand flows.

mod registry;

#[cfg(test)]
mod tests;

use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

use crate::model::destination::Destination;
use crate::model::ids::{OrderId, SkuId};
use crate::model::location::ToteType;
use crate::simtime::SimTime;

pub use registry::OrderService;

pub type OrderContainer = ToteType;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderLine {
    pub sku: SkuId,
    pub quantity: u32,
}

/// How the order leaves the station, not where it is headed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitPath {
    Dock,
    Sorter,
    Buffer,
}

impl ExitPath {
    pub const UNSTATED: Self = Self::Dock;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderSpec {
    pub id: OrderId,
    pub lines: Vec<OrderLine>,
    pub container: OrderContainer,
    pub is_container_shared: bool,
    pub exit_path: ExitPath,
    pub destination: Option<Destination>,
    /// `None`: from the start of the run.
    pub arrival: Option<SimTime>,
    pub deadline: Option<SimTime>,
    /// Inert: producers set it, the solver never reads it (enforced by a test).
    pub source_label: Option<String>,
}

impl OrderSpec {
    pub fn unstated(id: OrderId, lines: Vec<OrderLine>) -> Self {
        Self {
            id,
            lines,
            container: OrderContainer::Tote,
            is_container_shared: false,
            exit_path: ExitPath::UNSTATED,
            destination: None,
            arrival: None,
            deadline: None,
            source_label: None,
        }
    }

    /// One `(line_index, sku)` per unit: a pick retrieves one unit at a time.
    pub fn pick_units(&self) -> impl Iterator<Item = (usize, SkuId)> + '_ {
        self.lines.iter().enumerate().flat_map(|(i, line)| (0..line.quantity).map(move |_| (i, line.sku)))
    }

    /// Distinct SKUs; a producer may repeat a SKU across lines.
    pub fn unique_sku_count(&self) -> usize {
        self.lines.iter().map(|line| line.sku).collect::<FxHashSet<SkuId>>().len()
    }

    pub fn unit_count(&self) -> u32 {
        self.lines.iter().map(|line| line.quantity).sum()
    }

    pub fn is_multi_sku(&self) -> bool {
        let Some(first) = self.lines.first() else { return false };
        self.lines.iter().any(|line| line.sku != first.sku)
    }

    /// Drives trip count: at `units_per_tote = 1` every unit is its own tote.
    pub fn is_multi_unit(&self) -> bool {
        self.unit_count() > 1
    }
}
