//! `DES_HEARTBEAT=<sim-secs>`: one stderr line per planned-frontier
//! stride. Compare runs by JOINING lines on the `stride=` key — never
//! by line-diffing the streams (wall fields differ run to run).

use std::time::Instant;

use crate::model::SimTime;

pub(super) struct Heartbeat {
    beat_ms: u64,
    next_boundary_ms: u64,
    wall_start: Instant,
    prev_wall_secs: f64,
    prev_sim_ms: u64,
    prev_cycles: u64,
}

impl Heartbeat {
    pub(super) fn from_env() -> Option<Self> {
        let secs: f64 = std::env::var("DES_HEARTBEAT").ok()?.parse().ok()?;
        (secs > 0.0).then(|| Self {
            beat_ms: (secs * 1000.0) as u64,
            next_boundary_ms: (secs * 1000.0) as u64,
            wall_start: Instant::now(),
            prev_wall_secs: 0.0,
            prev_sim_ms: 0,
            prev_cycles: 0,
        })
    }

    pub(super) fn tick(&mut self, sim: SimTime, cycles: u64, pending: usize) {
        let sim_ms = sim.as_ms();
        if sim_ms < self.next_boundary_ms {
            return;
        }
        let stride_key_secs = self.next_boundary_ms / 1000;
        let wall = self.wall_start.elapsed().as_secs_f64();
        let dwall = wall - self.prev_wall_secs;
        let dsim_secs = (sim_ms - self.prev_sim_ms) as f64 / 1000.0;
        let rate = if dwall > 0.0 { dsim_secs / dwall } else { f64::INFINITY };
        eprintln!(
            "[hb] stride={stride_key_secs}s sim={:.1}s wall={wall:.1}s Δwall={dwall:.2}s \
             rate={rate:.2}:1 Δcycles={} pending={}",
            sim_ms as f64 / 1000.0,
            cycles - self.prev_cycles,
            pending,
        );
        self.prev_wall_secs = wall;
        self.prev_sim_ms = sim_ms;
        self.prev_cycles = cycles;

        self.next_boundary_ms = (sim_ms / self.beat_ms + 1) * self.beat_ms;
    }
}
