//! Within `worldview-render`, `view` is WHAT THE VIEWER IS LOOKING AT: the run
//! itself, one class of robot's reachable hops, or what the run measured.
//!
//! It is one enum rather than a number because the choice decides what is
//! drawn — whether robots and containers stand on the floor, which arrows are
//! drawn, and how the cell shader shades a cell — and a viewer that passed the
//! wrong number would get a silently different picture. The shader still reads
//! a number, and [`ViewMode::shader_code`] is the only place one is minted.

/// What the cells are shaded by while the simulation is being looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measured {
    /// Plain cells: the run is drawn, nothing is tinted.
    Nothing,
    /// Counted visits, per cell.
    Visits,
    /// How much of a cell's exposure robots spent waiting on it.
    Wait,
    /// The mean speed robots crossed a cell at.
    Speed,
}

/// What a viewer is looking at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    /// The run as it is: the robots and the containers stand on the floor.
    State,
    /// Where an unladen D30 may go.
    UnladenHtm,
    /// Where a laden D30 may go.
    LadenHtm,
    /// Where a DA50 may go.
    Vtm,
    /// What the run measured, painted on the cells.
    Simulation(Measured),
}

impl ViewMode {
    /// Whether the robots and the containers are drawn. They stand on the
    /// floor only while the run itself is being looked at: a reachability or a
    /// measurement view is about the cells, and movers over it would be read
    /// as part of the answer.
    pub fn draws_movers(self) -> bool {
        self == Self::State
    }

    /// The arrows this view asks for, if any.
    pub fn arrows(self) -> Option<ArrowSet> {
        match self {
            Self::UnladenHtm => Some(ArrowSet::UnladenHtm),
            Self::LadenHtm => Some(ArrowSet::LadenHtm),
            Self::Vtm => Some(ArrowSet::Vtm),
            Self::State | Self::Simulation(_) => None,
        }
    }

    /// The number the cell and bot shaders switch on. The values are the
    /// shaders' own and are not free to renumber.
    pub fn shader_code(self) -> u32 {
        match self {
            Self::State => 0,
            Self::UnladenHtm => 1,
            Self::LadenHtm => 2,
            Self::Vtm => 3,
            Self::Simulation(Measured::Nothing) => 4,
            Self::Simulation(Measured::Visits) => 5,
            Self::Simulation(Measured::Wait) => 6,
            Self::Simulation(Measured::Speed) => 7,
        }
    }
}

/// Which hops the arrows show. Each is a contiguous run of the one arrow
/// instance buffer, which is why a view asks for one set and never two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrowSet {
    UnladenHtm,
    LadenHtm,
    Vtm,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shader numbers are a contract with `grid.wgsl`, and only the run
    /// itself draws movers.
    #[test]
    fn every_view_states_its_shader_code_and_whether_movers_stand_in_it() {
        let all = [
            (ViewMode::State, 0, true),
            (ViewMode::UnladenHtm, 1, false),
            (ViewMode::LadenHtm, 2, false),
            (ViewMode::Vtm, 3, false),
            (ViewMode::Simulation(Measured::Nothing), 4, false),
            (ViewMode::Simulation(Measured::Visits), 5, false),
            (ViewMode::Simulation(Measured::Wait), 6, false),
            (ViewMode::Simulation(Measured::Speed), 7, false),
        ];
        for (view, code, movers) in all {
            assert_eq!(view.shader_code(), code);
            assert_eq!(view.draws_movers(), movers);
        }
        assert_eq!(ViewMode::LadenHtm.arrows(), Some(ArrowSet::LadenHtm));
        assert_eq!(ViewMode::State.arrows(), None);
    }
}
