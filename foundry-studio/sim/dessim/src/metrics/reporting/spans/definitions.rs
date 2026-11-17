//! Within `spans`, a definition is WHAT A METRIC OF TIME IN A STATE COUNTS: the robots it measures and the category
//! each state the channel states counts under. The D30 state, D30 wait, DA50 state and DA50 wait definitions count each
//! state under the category today's fleet reports (`d30_state`, `state`) give the same time, a robot with no task
//! under the reports' own no-task categories, which they file under waiting. Where the channel states as no task what
//! the reports explain after the fact, a resident D30's stand before its pickup and a DA50's wait before its cycle, the
//! reports' categories for those explanations get no value. A D30 awaiting a committed move's departure off a station,
//! which the D30 report files as no task, counts under a category of the D30 wait definition's own.

use std::fmt::Debug;

use worldview::frames::playback::RobotClass;
use worldview::frames::robot_state::{CellRole, RobotState, WaitCause};

use crate::metrics::reporting::{
    D30_STATES, D30State, D30WaitCategory, DA50_STATES, DA50_WAIT_REASONS, Da50State, Da50WaitReason,
};

/// What a metric of time in a state measures: every window states each category's time, and a reader computes each
/// definition once for every metric on it.
pub trait SpanDefinition {
    type Category: Copy + Eq + Debug + 'static;
    /// Every robot of this class the recording names.
    const ROBOTS: RobotClass;
    /// Every category: the report's, in the order it lists them, and any of the definition's own beside its kin.
    const CATEGORIES: &'static [Self::Category];
    /// The categories the channel never states for these robots: a metric gives them no value, never a measured zero.
    const UNSUPPORTED: &'static [Self::Category];
    /// The category `state` counts under; `None` for a state the definition does not measure, as a wait definition
    /// measures no travel.
    fn category(state: RobotState) -> Option<Self::Category>;
}

/// A D30's time by the D30 report's states.
#[derive(Debug)]
pub struct D30StateDefinition;

impl SpanDefinition for D30StateDefinition {
    type Category = D30State;
    const ROBOTS: RobotClass = RobotClass::Htm;
    const CATEGORIES: &'static [D30State] = &D30_STATES;
    /// No fact gives a D30's alignment a length, and every instant has a state, so the channel states neither.
    const UNSUPPORTED: &'static [D30State] = &[D30State::Alignment, D30State::Unknown];

    fn category(state: RobotState) -> Option<D30State> {
        Some(match state {
            RobotState::Pickup => D30State::Pickup,
            RobotState::Drop => D30State::Drop,
            RobotState::Alignment => D30State::Alignment,
            RobotState::EmptyTravel => D30State::EmptyTravel,
            RobotState::LadenTravel => D30State::LadenTravel,
            RobotState::TravelToPark => D30State::TravelToPark,
            RobotState::StationProcessing => D30State::StationProcessing,
            RobotState::Waiting { .. } | RobotState::NoTask { .. } => D30State::Waiting,
        })
    }
}

/// A D30's waiting time by the D30 report's ratified categories: what holds the D30 read jointly with the role of
/// the cell it stands on, as the report reads a hold. A D30 with no task on a cell the map gives no role stands
/// elsewhere, as on open floor. A D30 awaiting a committed move's departure anywhere but a station's presentation cell
/// counts under the definition's own category, beside the report's for the same stand on that cell.
#[derive(Debug)]
pub struct D30WaitDefinition;

impl SpanDefinition for D30WaitDefinition {
    type Category = D30WaitCategory;
    const ROBOTS: RobotClass = RobotClass::Htm;
    const CATEGORIES: &'static [D30WaitCategory] = &[
        D30WaitCategory::NoTaskParked,
        D30WaitCategory::NoTaskElsewhere,
        D30WaitCategory::Presenting,
        D30WaitCategory::AwaitingStationDeparture,
        D30WaitCategory::AwaitingDepartureElsewhere,
        D30WaitCategory::AwaitingRelayDepositResident,
        D30WaitCategory::AwaitingRelayDepositDrove,
        D30WaitCategory::AwaitingRelaySlot,
        D30WaitCategory::InQueueStation,
        D30WaitCategory::Congestion,
        D30WaitCategory::Unknown,
        D30WaitCategory::NoFacts,
        D30WaitCategory::ConflictingFacts,
    ];
    /// A resident D30's stand before its pickup is no task, and every instant has one state from committed facts.
    const UNSUPPORTED: &'static [D30WaitCategory] =
        &[D30WaitCategory::AwaitingRelayDepositResident, D30WaitCategory::NoFacts, D30WaitCategory::ConflictingFacts];

    fn category(state: RobotState) -> Option<D30WaitCategory> {
        use D30WaitCategory as Wait;
        let (cause, cell) = match state {
            RobotState::NoTask { cell: Some(CellRole::Relay | CellRole::Charging) } => return Some(Wait::NoTaskParked),
            RobotState::NoTask { .. } => return Some(Wait::NoTaskElsewhere),
            RobotState::Waiting { cause, cell } => (cause, cell),
            _ => return None,
        };
        Some(match (cause, cell) {
            (WaitCause::AwaitingStationService | WaitCause::RouteHold, Some(CellRole::Presentation)) => {
                Wait::Presenting
            }
            (WaitCause::AwaitingStationService | WaitCause::RouteHold, Some(CellRole::StationQueue)) => {
                Wait::InQueueStation
            }
            (WaitCause::AwaitingDeparture, Some(CellRole::Presentation)) => Wait::AwaitingStationDeparture,
            (WaitCause::AwaitingDeparture, _) => Wait::AwaitingDepartureElsewhere,
            // Awaiting an operator on no station cell: the report has no category for that hold.
            (WaitCause::AwaitingStationService, _) => Wait::Unknown,
            (WaitCause::AwaitingDeposit, _) => Wait::AwaitingRelayDepositDrove,
            (WaitCause::AwaitingSlot, _) => Wait::AwaitingRelaySlot,
            (WaitCause::RouteHold, _) => Wait::Congestion,
        })
    }
}

/// A DA50's time by the DA50 report's states. A DA50 never stands at a station, so station processing stated for one
/// is time the report cannot name.
#[derive(Debug)]
pub struct Da50StateDefinition;

impl SpanDefinition for Da50StateDefinition {
    type Category = Da50State;
    const ROBOTS: RobotClass = RobotClass::Vtm;
    const CATEGORIES: &'static [Da50State] = &DA50_STATES;
    /// The simulator does not model a DA50's travel to park or to unblock another, and states no DA50 at a station.
    const UNSUPPORTED: &'static [Da50State] =
        &[Da50State::TravelToPark, Da50State::TravelToUnblockOther, Da50State::Unknown];

    fn category(state: RobotState) -> Option<Da50State> {
        Some(match state {
            RobotState::Pickup => Da50State::Pickup,
            RobotState::Drop => Da50State::Drop,
            RobotState::Alignment => Da50State::Alignment,
            RobotState::EmptyTravel => Da50State::EmptyTravel,
            RobotState::LadenTravel => Da50State::LadenTravel,
            RobotState::TravelToPark => Da50State::TravelToPark,
            RobotState::StationProcessing => Da50State::Unknown,
            RobotState::Waiting { .. } | RobotState::NoTask { .. } => Da50State::Waiting,
        })
    }
}

/// A DA50's waiting time by the DA50 report's reasons. A DA50's cycle is committed only once B has passed the wait
/// before it, so the channel states every DA50 stand as no task, and the reasons the report reads off the cycle's
/// commit get no value.
#[derive(Debug)]
pub struct Da50WaitDefinition;

impl SpanDefinition for Da50WaitDefinition {
    type Category = Da50WaitReason;
    const ROBOTS: RobotClass = RobotClass::Vtm;
    const CATEGORIES: &'static [Da50WaitReason] = &DA50_WAIT_REASONS;
    const UNSUPPORTED: &'static [Da50WaitReason] = &[
        Da50WaitReason::AwaitingDemand,
        Da50WaitReason::AwaitingTote,
        Da50WaitReason::AwaitingSlot,
        Da50WaitReason::AwaitingD30Deposit,
        Da50WaitReason::Unknown,
        Da50WaitReason::ConflictingFacts,
    ];

    fn category(state: RobotState) -> Option<Da50WaitReason> {
        match state {
            RobotState::NoTask { .. } => Some(Da50WaitReason::NoTask),
            // A DA50 holds for no cause the channel names: the report has no reason for one.
            RobotState::Waiting { .. } => Some(Da50WaitReason::Unknown),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::reporting::D30_WAIT_CATEGORIES;

    const CAUSES: [WaitCause; 5] = [
        WaitCause::AwaitingDeposit,
        WaitCause::AwaitingSlot,
        WaitCause::AwaitingStationService,
        WaitCause::AwaitingDeparture,
        WaitCause::RouteHold,
    ];
    const CELLS: [Option<CellRole>; 6] = [
        None,
        Some(CellRole::Charging),
        Some(CellRole::Presentation),
        Some(CellRole::StationQueue),
        Some(CellRole::Relay),
        Some(CellRole::Route),
    ];

    /// Every state the channel can state: the seven kinds that stand for no cause, a wait for each cause on each role,
    /// and no task on each role.
    fn every_state() -> Vec<RobotState> {
        let mut states = vec![
            RobotState::Pickup,
            RobotState::Drop,
            RobotState::Alignment,
            RobotState::EmptyTravel,
            RobotState::LadenTravel,
            RobotState::TravelToPark,
            RobotState::StationProcessing,
        ];
        for cause in CAUSES {
            states.extend(CELLS.map(|cell| RobotState::Waiting { cause, cell }));
        }
        states.extend(CELLS.map(|cell| RobotState::NoTask { cell }));
        states
    }

    /// A state definition counts every state, a wait definition every stand and nothing else, each under a category
    /// it lists, so no time a definition counts falls outside its windows' rows; and it lists every category it calls
    /// unsupported.
    fn counts_its_states<D: SpanDefinition>(waits_only: bool) {
        for state in every_state() {
            let standing = matches!(state, RobotState::Waiting { .. } | RobotState::NoTask { .. });
            match D::category(state) {
                Some(category) => assert!(D::CATEGORIES.contains(&category), "{state:?} as {category:?}"),
                None => assert!(waits_only && !standing, "{state:?} counts under no category"),
            }
        }
        assert!(D::UNSUPPORTED.iter().all(|category| D::CATEGORIES.contains(category)), "{:?}", D::UNSUPPORTED);
    }

    #[test]
    fn every_state_counts_under_a_category_the_definition_lists() {
        counts_its_states::<D30StateDefinition>(false);
        counts_its_states::<D30WaitDefinition>(true);
        counts_its_states::<Da50StateDefinition>(false);
        counts_its_states::<Da50WaitDefinition>(true);
        assert_eq!(D30WaitDefinition::category(RobotState::LadenTravel), None, "a wait definition measures no travel");
    }

    /// The D30 wait definition lists the D30 report's categories in the report's order, and awaiting a departure off
    /// a station, which only the channel states, beside awaiting one on a station's presentation cell.
    #[test]
    fn the_d30_wait_definition_lists_the_reports_categories_and_its_own_beside_its_kin() {
        let listed = D30WaitDefinition::CATEGORIES;
        let own = D30WaitCategory::AwaitingDepartureElsewhere;
        let reports: Vec<D30WaitCategory> = listed.iter().copied().filter(|category| *category != own).collect();
        assert_eq!(reports, D30_WAIT_CATEGORIES, "the report's categories, in its order");
        let kin = listed.iter().position(|category| *category == D30WaitCategory::AwaitingStationDeparture);
        assert_eq!(listed.iter().position(|category| *category == own), kin.map(|kin| kin + 1));
    }

    /// A category no state the channel states counts under is time no producer can state, so a metric gives it no
    /// value, never a measured zero: the reports' categories for what is decided after the fact, and for facts
    /// missing or in conflict.
    #[test]
    fn a_category_no_state_counts_under_is_unsupported() {
        fn unreached_are_unsupported<D: SpanDefinition>() {
            let reached: Vec<D::Category> = every_state().into_iter().filter_map(D::category).collect();
            for category in D::CATEGORIES.iter().filter(|category| !reached.contains(category)) {
                assert!(D::UNSUPPORTED.contains(category), "no state counts under {category:?}, and it is charted");
            }
        }
        unreached_are_unsupported::<D30StateDefinition>();
        unreached_are_unsupported::<D30WaitDefinition>();
        unreached_are_unsupported::<Da50StateDefinition>();
        unreached_are_unsupported::<Da50WaitDefinition>();
    }

    /// The table that sets the D30 report's wait categories against the channel's states, a row for each category the
    /// channel can reach: no task by where the D30 stands, a hold on a station's cells by the cell, and every other
    /// wait by its cause alone.
    #[test]
    fn a_d30s_stand_counts_under_the_category_the_d30_report_gives_the_same_hold() {
        use D30WaitCategory as Wait;
        let no_task = [
            (Some(CellRole::Relay), Wait::NoTaskParked),
            (Some(CellRole::Charging), Wait::NoTaskParked),
            (Some(CellRole::Route), Wait::NoTaskElsewhere),
            (Some(CellRole::Presentation), Wait::NoTaskElsewhere),
            (None, Wait::NoTaskElsewhere),
        ];
        for (cell, category) in no_task {
            let state = RobotState::NoTask { cell };
            assert_eq!(D30WaitDefinition::category(state), Some(category), "no task on {cell:?}");
            assert_eq!(D30StateDefinition::category(state), Some(D30State::Waiting), "the report files it as waiting");
        }
        let waits = [
            (WaitCause::AwaitingStationService, Some(CellRole::Presentation), Wait::Presenting),
            (WaitCause::RouteHold, Some(CellRole::Presentation), Wait::Presenting),
            (WaitCause::AwaitingStationService, Some(CellRole::StationQueue), Wait::InQueueStation),
            (WaitCause::RouteHold, Some(CellRole::StationQueue), Wait::InQueueStation),
            (WaitCause::AwaitingStationService, Some(CellRole::Route), Wait::Unknown),
            (WaitCause::AwaitingDeparture, Some(CellRole::Presentation), Wait::AwaitingStationDeparture),
            (WaitCause::AwaitingDeparture, Some(CellRole::Route), Wait::AwaitingDepartureElsewhere),
            (WaitCause::AwaitingDeparture, Some(CellRole::Relay), Wait::AwaitingDepartureElsewhere),
            (WaitCause::AwaitingDeparture, None, Wait::AwaitingDepartureElsewhere),
            (WaitCause::AwaitingDeposit, Some(CellRole::Relay), Wait::AwaitingRelayDepositDrove),
            (WaitCause::AwaitingSlot, Some(CellRole::Relay), Wait::AwaitingRelaySlot),
            (WaitCause::RouteHold, Some(CellRole::Route), Wait::Congestion),
            (WaitCause::RouteHold, Some(CellRole::Relay), Wait::Congestion),
        ];
        for (cause, cell, category) in waits {
            let state = RobotState::Waiting { cause, cell };
            assert_eq!(D30WaitDefinition::category(state), Some(category), "{cause:?} on {cell:?}");
            assert_eq!(D30StateDefinition::category(state), Some(D30State::Waiting));
        }
    }

    /// A DA50 stands on its rail with no task; it never stands at a station, so station processing stated for one is
    /// time its report cannot name.
    #[test]
    fn a_da50s_states_count_under_the_da50_reports_states_and_reasons() {
        let no_task = RobotState::NoTask { cell: None };
        assert_eq!(Da50StateDefinition::category(no_task), Some(Da50State::Waiting));
        assert_eq!(Da50WaitDefinition::category(no_task), Some(Da50WaitReason::NoTask));
        let held = RobotState::Waiting { cause: WaitCause::AwaitingSlot, cell: None };
        assert_eq!(Da50StateDefinition::category(held), Some(Da50State::Waiting));
        assert_eq!(Da50WaitDefinition::category(held), Some(Da50WaitReason::Unknown));
        assert_eq!(Da50StateDefinition::category(RobotState::StationProcessing), Some(Da50State::Unknown));
        assert_eq!(Da50StateDefinition::category(RobotState::Pickup), Some(Da50State::Pickup));
        assert_eq!(Da50WaitDefinition::category(RobotState::Pickup), None);
    }
}
