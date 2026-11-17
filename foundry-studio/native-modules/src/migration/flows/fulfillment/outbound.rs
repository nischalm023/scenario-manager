//! Select outbound handoffs from declared process roles and order traits.
//! Unclassified legacy maps retain their common extraction pool; once roles
//! are declared, a missing destination is an error rather than a fallback.

use std::sync::Arc;

use desgraph::{CellId, StationId, StationType};
use desmap::{OutboundRole, Station};

use crate::error::PlanError;
use crate::model::order::{ExitPath, OrderContainer, OrderSpec};

#[derive(Clone)]
pub(super) struct OutboundStations(Arc<[(StationId, CellId, Option<OutboundRole>)]>);

impl OutboundStations {
    pub fn new(stations: &[Station]) -> Self {
        let mut endpoints: Vec<_> = stations
            .iter()
            .filter(|station| station.station_type == StationType::Extraction)
            .map(|station| (station.id, station.delivery_cell(), station.outbound_role))
            .collect();
        endpoints.sort_by_key(|&(station, _, _)| station);
        Self(endpoints.into())
    }

    pub fn all(&self) -> impl Iterator<Item = (StationId, CellId)> + '_ {
        self.0.iter().map(|&(station, cell, _)| (station, cell))
    }

    pub fn for_order(&self, order: &OrderSpec) -> Result<Vec<(StationId, CellId)>, PlanError> {
        let specialized = self.0.iter().any(|(_, _, role)| role.is_some());
        let wanted = role_for(order.container, order.exit_path);
        let endpoints: Vec<_> = self
            .0
            .iter()
            .filter(|(_, _, role)| !specialized || *role == Some(wanted))
            .map(|&(station, cell, _)| (station, cell))
            .collect();
        if endpoints.is_empty() {
            return Err(super::invalid(format!("order {:?} needs a declared {} handoff", order.id, wanted.label())));
        }
        Ok(endpoints)
    }
}

fn role_for(container: OrderContainer, exit_path: ExitPath) -> OutboundRole {
    match container {
        OrderContainer::Carton => OutboundRole::SealerDropoff,
        OrderContainer::Tray => OutboundRole::Flex,
        OrderContainer::Tote if exit_path == ExitPath::Buffer => OutboundRole::RouteDispense,
        OrderContainer::Tote => OutboundRole::Bagging,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::OrderId;

    #[test]
    fn package_and_channel_select_the_process_even_when_buffering_is_bypassed() {
        let roles = [
            OutboundRole::RouteDispense,
            OutboundRole::Bagging,
            OutboundRole::Flex,
            OutboundRole::SealerDropoff,
            OutboundRole::SealedBoxPickup,
        ];
        let endpoints = OutboundStations(
            roles
                .iter()
                .enumerate()
                .map(|(i, &role)| (StationId(i as u32), CellId(i as u32), Some(role)))
                .collect::<Vec<_>>()
                .into(),
        );
        for exit_path in [ExitPath::Buffer, ExitPath::Dock, ExitPath::Sorter] {
            for (container, expected) in [
                (OrderContainer::Carton, 3),
                (OrderContainer::Tray, 2),
                (OrderContainer::Tote, if exit_path == ExitPath::Buffer { 0 } else { 1 }),
            ] {
                let order = OrderSpec { container, exit_path, ..OrderSpec::unstated(OrderId(1), vec![]) };
                assert_eq!(endpoints.for_order(&order).unwrap(), [(StationId(expected), CellId(expected))]);
            }
        }
    }

    #[test]
    fn missing_role_does_not_route_to_a_generic_port() {
        let endpoints = OutboundStations(
            vec![(StationId(1), CellId(1), Some(OutboundRole::Flex)), (StationId(2), CellId(2), None)].into(),
        );
        let order = OrderSpec::unstated(OrderId(1), vec![]);
        assert!(endpoints.for_order(&order).unwrap_err().to_string().contains("Bagging"));
        let legacy = OutboundStations(vec![(StationId(2), CellId(2), None)].into());
        assert_eq!(legacy.for_order(&order).unwrap(), [(StationId(2), CellId(2))]);
    }
}
