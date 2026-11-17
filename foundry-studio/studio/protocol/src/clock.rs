//! Within `studio-protocol`, `clock` is time as the protocol writes it: which clock a value counts in (`Clock`, the
//! one discriminator a page reads to tag a value with its clock, foundry CLAUDE.md's two clocks and the wall clock;
//! `DurationClock` for a duration, which scheduler time never measures), and wall-clock time, RFC 3339 with an offset,
//! the clock of `at` and every `*_at` field (spec 01 section 3.6). Simtime never travels in `WallClock`: it is integer
//! milliseconds in `sim_ms` fields, desunits' `SimTime`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The clock a value counts in: simtime, the simulation's own; scheduler time, the order in which things are booked;
/// or the wall clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "clock.ts")]
pub enum Clock {
    Sim,
    Scheduler,
    Wall,
}

/// The clock a duration counts in, or the hour of a rate per hour: simtime or the wall clock. Scheduler time orders
/// what is booked and measures no stretch of time, so no duration counts in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "clock.ts")]
pub enum DurationClock {
    Sim,
    Wall,
}

/// An instant on the wall clock, written in UTC (`2026-10-18T19:23:24.512Z`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(type = "string", export_to = "clock.ts")]
pub struct WallClock(DateTime<Utc>);

impl WallClock {
    pub fn now() -> Self {
        Self(Utc::now())
    }

    pub fn instant(self) -> DateTime<Utc> {
        self.0
    }
}

impl From<DateTime<Utc>> for WallClock {
    fn from(instant: DateTime<Utc>) -> Self {
        Self(instant)
    }
}
