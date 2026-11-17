//! Explicit models for synthetic observer event fixtures. Production observer
//! construction receives its bindings from the generated scenario.

use std::sync::Arc;

use desgraph::RobotType;
use rustc_hash::FxHashMap;

use super::time_slice::MetricsConfig;
use crate::config::SimConfig;
use crate::events::SimEvent;
use crate::model::RobotId;
use crate::robots::runtime::FloorRobotModel;

pub(super) fn legacy_models(
    config: &SimConfig,
    robots: impl IntoIterator<Item = RobotId>,
) -> Arc<FxHashMap<RobotId, Arc<FloorRobotModel>>> {
    let model = Arc::new(FloorRobotModel::legacy(config));
    Arc::new(robots.into_iter().map(|robot| (robot, model.clone())).collect())
}

/// Older event-only fixtures test attribution, not physical model selection.
/// Bind exactly their stated HTM navigation identities to an explicit legacy
/// model before handing the events to the observer.
pub(super) fn bind_legacy_nav_models(config: &mut MetricsConfig, events: &[SimEvent]) {
    let model = Arc::new(FloorRobotModel::legacy(&SimConfig::default()));
    let bindings = Arc::make_mut(&mut config.htm_models);
    for event in events {
        if let SimEvent::Nav(nav) = event {
            if nav.robot_type == RobotType::Htm {
                bindings.entry(nav.robot_id).or_insert_with(|| model.clone());
            }
        }
    }
}
