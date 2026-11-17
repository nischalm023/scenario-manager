//! Within `studio-metrics`, `ending` is HOW A RECORDING'S END BLOCK SAYS THE RUN ENDED, AS THE WIRE SAYS IT: the end
//! block's stop reason, in the simulator's own words (dessim spells its `StopReason` as Rust prints it, a detail in
//! brackets after the reason's name), read as one of the stream's outcomes.

use studio_protocol::RunOutcome;

/// The outcome the end block's `stop_reason` says, or `None` for a reason this build does not know.
pub fn run_outcome(stop_reason: &str) -> Option<RunOutcome> {
    let name = stop_reason.split_once('(').map_or(stop_reason, |(name, _)| name);
    Some(match name {
        "Complete" => RunOutcome::Complete,
        "UserStop" => RunOutcome::UserStop,
        "Watchdog" => RunOutcome::Watchdog,
        "WallTimeout" => RunOutcome::WallTimeout,
        "SimTimeLimit" => RunOutcome::SimTimeLimit,
        "Error" => RunOutcome::Error,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use dessim::StopReason;

    use super::*;

    /// Every stop reason dessim has, in a list a new one cannot be left out of: the match below stops compiling until
    /// it is added, and so until the wire names it.
    fn every_stop_reason() -> [StopReason; 6] {
        let every = [
            StopReason::Complete,
            StopReason::UserStop,
            StopReason::Watchdog("no progress for 60 s".to_owned()),
            StopReason::WallTimeout,
            StopReason::SimTimeLimit,
            StopReason::Error("the planner failed".to_owned()),
        ];
        for reason in &every {
            match reason {
                StopReason::Complete
                | StopReason::UserStop
                | StopReason::Watchdog(_)
                | StopReason::WallTimeout
                | StopReason::SimTimeLimit
                | StopReason::Error(_) => {}
            }
        }
        every
    }

    /// The end block spells a stop reason as dessim's writer does, `format!("{:?}", reason)`, its detail and all; each
    /// reads as its own outcome, and words no reason spells read as none.
    #[test]
    fn every_stop_reason_the_simulator_writes_reads_as_its_own_outcome() {
        let outcomes: Vec<Option<RunOutcome>> =
            every_stop_reason().iter().map(|reason| run_outcome(&format!("{reason:?}"))).collect();
        let expected = [
            RunOutcome::Complete,
            RunOutcome::UserStop,
            RunOutcome::Watchdog,
            RunOutcome::WallTimeout,
            RunOutcome::SimTimeLimit,
            RunOutcome::Error,
        ];
        assert_eq!(outcomes, expected.map(Some));
        assert_eq!(run_outcome("sim duration limit reached"), None, "the reason's words, not its name");
        assert_eq!(run_outcome("Paused"), None);
    }
}
