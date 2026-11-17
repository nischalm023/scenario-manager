//! Robot catalog components become immutable models, one per deployed class.
//! `materialize` validates each source, `catalog` binds cohort counts, and
//! `runtime` carries those bindings into execution and playback. Floor and
//! rack-attached VTM motion remain distinct. `successor` resolves site
//! overrides before any model is read. Legacy settings are an explicit
//! compatibility boundary; the M5F below is a fixed diagnostic reference.

pub mod aspects;
pub mod catalog;
pub mod figures;
pub mod runtime;
pub mod materialize;
pub mod successor;
pub mod vtm;

pub use catalog::RobotCatalog;
pub use figures::{HtmFigures, fleet_of_figure_key};
pub use materialize::{Refusal, RobotError, htm_figures, require_catalog_motion, vtm_figures};
pub use successor::{SUCCESSOR, resolve_successors};

use std::path::Path;

use serde_json::Value;
use site_model::document::{Document, Record};
use site_model::entity::Entity;

use crate::pathfinding::HtmKinematics;

const REFERENCE_HTM: &str = include_str!("../../tests/fixtures/robots/quicktron_m5f.world.json");

/// The Quicktron M5F's chassis kinematics: the fixed robot that the desvis
/// `route` and `fpi` diagnostics and the pathfinder tests measure with, so their
/// numbers stay comparable whatever world they load. Read from the robot's
/// model file, compiled in so a binary run from anywhere carries it. The file
/// states no ConstantSpeed-mode cruise, so the cruise is the top speed.
pub fn reference_htm() -> HtmKinematics {
    let figures = reference_htm_figures();
    figures.kinematics(figures.loaded_speed, figures.empty_speed)
}

fn reference_htm_figures() -> HtmFigures {
    let value: Value = serde_json::from_str(REFERENCE_HTM).expect("the M5F model file is JSON");
    let document = Document::from_value(Path::new("quicktron_m5f.world.json"), &value)
        .expect("the M5F model file is a world-protocol document");
    let entity = document
        .records
        .into_iter()
        .find_map(|record| match record {
            Record::Declaration(robot) => {
                Some(Entity { id: robot.id.into(), name: robot.name, components: robot.components, parent: None })
            }
            _ => None,
        })
        .expect("the M5F model file declares the robot");
    htm_figures(&entity).expect("the M5F model file states an HTM's figures")
}

#[cfg(test)]
mod tests {
    use super::{reference_htm, reference_htm_figures};
    use crate::SimDuration;

    /// The M5F's figures as its model file states them, the loaded ones
    /// field-calibrated to the empty. Its one lift stroke is the pre-jacking
    /// stroke, which enters no handoff, so its belt figures stay unstated and
    /// the relay figures stand in.
    #[test]
    fn the_m5f_model_file_gives_its_figures() {
        let m5f = reference_htm();
        for (load, params) in [("loaded", m5f.loaded), ("empty", m5f.unloaded)] {
            assert_eq!(params.max_velocity_mm_s, 4_500.0, "{load}");
            assert_eq!(params.acceleration_mm_s2, 2_000.0, "{load}");
            assert_eq!(params.deceleration_mm_s2, 2_000.0, "{load}");
        }
        assert_eq!(m5f.constant_speed_mm_s(true), 4_500.0, "the cruise is the top speed");
        assert_eq!(m5f.constant_speed_mm_s(false), 4_500.0);
        assert_eq!(m5f.turn_duration_for(true), SimDuration::ms(2_100));
        assert_eq!(m5f.turn_duration_for(false), SimDuration::ms(2_100));
        assert_eq!(m5f.reversal_duration_for(true), None, "the sheet states no 180° turn");
        assert_eq!(m5f.reversal_duration_for(false), None);
        let handoffs = reference_htm_figures();
        assert_eq!((handoffs.pickup_secs, handoffs.deposit_secs), (2.0, 2.0));
        assert_eq!(handoffs.conveyor_pickup_secs, None, "the pre-jacking stroke enters no belt pick");
        assert_eq!(handoffs.conveyor_deposit_secs, None, "nor any belt drop");
    }
}
