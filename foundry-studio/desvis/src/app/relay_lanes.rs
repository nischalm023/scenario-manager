//! Within `app`, `relay_lanes` is EACH RELAY ZONE'S BUSY LANE AS THE SLICES
//! STATE IT: what the Gantt's relay-access section draws, built from the
//! slices received while a run goes and from a recording's slices on
//! playback, so the two are one reading.

use desgraph::ZoneId;
use desmap::WarehouseMap;
use dessim::metrics::profile::{SliceRecord, relay_zone_busy_lanes};
use dessim::model::relay::RelayUtilizationTimeline;

/// Each relay zone with its relay count, as the run's map states them.
pub(crate) fn relay_zones(map: &WarehouseMap) -> Vec<(ZoneId, u32)> {
    map.zones.iter().filter(|zone| !zone.relays.is_empty()).map(|zone| (zone.id, zone.relays.len() as u32)).collect()
}

/// The lanes over `slices`, the first slices of a run, or `None` when there
/// are none or one of them does not state every zone's busy time: a recording
/// written before slices stated it draws no relay lanes.
pub(crate) fn relay_lanes(
    slices: &[SliceRecord],
    slice_width_ms: u64,
    zones: &[(ZoneId, u32)],
) -> Option<RelayUtilizationTimeline> {
    slices.first()?;
    relay_zone_busy_lanes(slices, slice_width_ms, zones.iter().copied())
}

/// `lanes` with `slices`, the next ones after those it covers, added to each
/// zone's lane, or `None` when one of them does not state every zone's busy
/// time, and no lane over the run would be true.
pub(crate) fn extend_relay_lanes(
    mut lanes: RelayUtilizationTimeline,
    slices: &[SliceRecord],
    zones: &[(ZoneId, u32)],
) -> Option<RelayUtilizationTimeline> {
    let more = relay_zone_busy_lanes(slices, lanes.slice_width_ms, zones.iter().copied())?;
    // Both come out in zone order, over the same zones.
    for (lane, more) in lanes.zones.iter_mut().zip(more.zones) {
        lane.busy_ms.extend(more.busy_ms);
    }
    Some(lanes)
}

#[cfg(test)]
mod tests {
    use dessim::metrics::profile::scalars_to_map;
    use dessim::model::SimDuration;
    use dessim::observer::time_slice::ScalarCounters;

    use super::*;

    /// A slice stating `per_zone`'s relay busy time.
    fn slice(start_ms: u64, per_zone: &[(u32, u64)]) -> SliceRecord {
        let per_zone = per_zone.iter().map(|&(zone, ms)| (ZoneId(zone), SimDuration::ms(ms))).collect();
        let scalars = ScalarCounters { relay_reserved_busy_per_zone: per_zone, ..Default::default() };
        SliceRecord {
            start_ms,
            metrics: scalars_to_map(&scalars),
            grid: Default::default(),
            bot_locations: Default::default(),
            relay_state: Default::default(),
            tote_state: Default::default(),
            station_diag: Default::default(),
        }
    }

    fn busy(lanes: &RelayUtilizationTimeline) -> Vec<(u32, u32, Vec<u64>)> {
        lanes.zones.iter().map(|lane| (lane.zone_id, lane.relays, lane.busy_ms.clone())).collect()
    }

    /// Lanes extended by each slice as it arrives are the lanes of all the
    /// slices read at once, which is how a run watched live and its file draw
    /// the same lanes.
    #[test]
    fn lanes_extended_as_slices_arrive_are_the_lanes_of_all_of_them() {
        let zones = [(ZoneId(4), 3), (ZoneId(2), 1)];
        let slices =
            [slice(0, &[(2, 100), (4, 0)]), slice(10_000, &[(2, 0), (4, 900)]), slice(20_000, &[(2, 50), (4, 50)])];
        let all = relay_lanes(&slices, 10_000, &zones).expect("the slices state both zones");
        let grown = relay_lanes(&slices[..1], 10_000, &zones).expect("the first slice states both zones");
        let grown = extend_relay_lanes(grown, &slices[1..2], &zones).expect("the second states both");
        let grown = extend_relay_lanes(grown, &slices[2..], &zones).expect("the third states both");
        assert_eq!(busy(&grown), busy(&all));
        assert_eq!(busy(&all), [(2, 1, vec![100, 0, 50]), (4, 3, vec![0, 900, 50])]);
    }

    /// Slices from before the zones' busy time was stated give no lanes, and
    /// neither do no slices at all; a slice arriving later that states no
    /// figure for a zone ends the lanes rather than drawing a gap as idle.
    #[test]
    fn slices_that_state_no_zone_give_no_lanes() {
        let zones = [(ZoneId(4), 3)];
        assert!(relay_lanes(&[slice(0, &[])], 10_000, &zones).is_none());
        assert!(relay_lanes(&[], 10_000, &zones).is_none());
        let lanes = relay_lanes(&[slice(0, &[(4, 100)])], 10_000, &zones).expect("the first slice states the zone");
        assert!(extend_relay_lanes(lanes, &[slice(10_000, &[])], &zones).is_none());
    }
}
