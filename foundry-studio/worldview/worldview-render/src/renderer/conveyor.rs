//! Within `renderer`, `conveyor` is A CONTAINER PART-WAY ALONG A BELT: it
//! projects the travels a run recorded onto the conveyor drawing, so a
//! container between two landmarks moves along the belt instead of standing on
//! the landmark it left.
//!
//! The interpolation shows travel and presentation dwell; motor controls and
//! container spacing remain outside the simulation's current conveyor model.
//! It runs over the container rows after they are built, because a travel
//! overrides where the container's own location says it stands.

use rustc_hash::FxHashMap;
use worldview::frames::playback::ToteTransit;

use super::instances::ToteInstance;
use crate::rack::{BeltRoute, RackVisual};

pub(super) fn apply_transits(
    instances: &mut [ToteInstance],
    transits: &[ToteTransit],
    cursor_ms: u64,
    rack: &RackVisual,
) {
    let active: FxHashMap<_, _> = transits
        .iter()
        .filter(|travel| travel.start.as_ms() <= cursor_ms && cursor_ms < travel.end.as_ms())
        .map(|travel| (travel.tote.0, travel))
        .collect();
    for instance in instances.iter_mut().filter(|instance| instance.tote_type == 4) {
        let Some(travel) = active.get(&instance.tote_id) else { continue };
        let route = rack
            .easyloop_belts
            .iter()
            .flat_map(|belt| &belt.routes)
            .find(|route| route.from_cell == travel.from_cell.0 && route.to_cell == travel.to_cell.0);
        let Some(route) = route else { continue };
        let fraction = (cursor_ms - travel.start.as_ms()) as f32 / (travel.end.as_ms() - travel.start.as_ms()) as f32;
        if let Some((position, heading, pitch)) = sample(route, fraction) {
            [instance.world_x, instance.world_y, instance.world_z] = position;
            instance.heading = heading;
            instance.pitch = pitch;
        }
    }
}

fn sample(route: &BeltRoute, fraction: f32) -> Option<([f32; 3], f32, f32)> {
    let distance =
        |a: &[f32; 3], b: &[f32; 3]| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
    let total: f32 = route.points.windows(2).map(|pair| distance(&pair[0], &pair[1])).sum();
    if total <= 0.0 {
        return route.points.first().copied().map(|point| (point, 0.0, 0.0));
    }
    let mut remaining = total * fraction.clamp(0.0, 1.0);
    for pair in route.points.windows(2) {
        let length = distance(&pair[0], &pair[1]);
        if length == 0.0 {
            continue;
        }
        if remaining <= length {
            let t = remaining / length;
            let position = std::array::from_fn(|axis| pair[0][axis] + (pair[1][axis] - pair[0][axis]) * t);
            let dx = pair[1][0] - pair[0][0];
            let dy = pair[1][1] - pair[0][1];
            let heading = dy.atan2(dx) + std::f32::consts::FRAC_PI_2;
            let pitch = (pair[0][2] - pair[1][2]).atan2(dx.hypot(dy));
            return Some((position, heading, pitch));
        }
        remaining -= length;
    }
    let end = route.points.last().copied()?;
    let prior = route.points.get(route.points.len().checked_sub(2)?)?;
    Some((
        end,
        (end[1] - prior[1]).atan2(end[0] - prior[0]) + std::f32::consts::FRAC_PI_2,
        (prior[2] - end[2]).atan2((end[0] - prior[0]).hypot(end[1] - prior[1])),
    ))
}

#[cfg(test)]
mod tests {
    use desgraph::CellId;
    use desunits::{SimTime, ToteId};

    use super::*;
    use crate::rack::EasyLoopBelt;

    #[test]
    fn a_recorded_tote_moves_continuously_without_moving_other_custody() {
        let mut rack = RackVisual::uniform(1);
        rack.easyloop_belts.push(EasyLoopBelt {
            landmarks: Default::default(),
            routes: vec![BeltRoute {
                from_cell: 1,
                to_cell: 2,
                points: vec![[0.0, 0.0, 0.65], [0.0, 1.0, 0.65], [2.0, 1.0, 0.65]],
            }],
        });
        let travel = ToteTransit {
            tote: ToteId(7),
            from_cell: CellId(1),
            to_cell: CellId(2),
            start: SimTime::ms(1000),
            end: SimTime::ms(4000),
        };
        let original = ToteInstance {
            tote_id: 7,
            contents_units: 3,
            world_x: 0.0,
            world_y: 0.0,
            world_z: 0.65,
            tote_type: 4,
            designation: 0,
            heading: 0.0,
            pitch: 0.0,
            physical_type: 0,
        };
        // Cursor order deliberately reverses. Each pose depends only on the record.
        for (cursor, expected) in [(1500, [0.0, 0.5]), (3000, [1.0, 1.0]), (1500, [0.0, 0.5])] {
            let mut instances =
                [original, ToteInstance { tote_type: 3, ..original }, ToteInstance { tote_id: 9, ..original }];
            apply_transits(&mut instances, std::slice::from_ref(&travel), cursor, &rack);
            assert_eq!([instances[0].world_x, instances[0].world_y], expected);
            assert_eq!(instances[0].contents_units, 3);
            assert_eq!([instances[1].world_x, instances[1].world_y], [0.0, 0.0]);
            assert_eq!([instances[2].world_x, instances[2].world_y], [0.0, 0.0]);
        }
        // Outside travel, the existing landmark pose is preserved for dwell.
        for cursor in [999, 4000, 6000] {
            let mut instances = [original];
            apply_transits(&mut instances, std::slice::from_ref(&travel), cursor, &rack);
            assert_eq!([instances[0].world_x, instances[0].world_y], [0.0, 0.0]);
        }
    }
}
