//! Bot identity and per-bot state: which kind of machine, how it drives and its plan.

use std::sync::Arc;

use super::plan::BotPlan;
use crate::robots::runtime::FloorRobotModel;

// ---------------------------------------------------------------------------
// Bot identity
// ---------------------------------------------------------------------------

/// HTM (horizontal transport) or VTM (vertical transport).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotKind {
    Htm,
    Vtm,
}

// ---------------------------------------------------------------------------
// Internal per-bot state
// ---------------------------------------------------------------------------

/// All mutable state for a single bot. Not exposed outside `model::bot`.
pub(super) struct BotData {
    pub kind: BotKind,
    /// How the bot drives, `[empty, laden]` (index by `carrying_cargo as usize`):
    /// the drive its reservations are priced and admitted with.
    pub model: Arc<FloorRobotModel>,
    pub plans: BotPlan,
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------
