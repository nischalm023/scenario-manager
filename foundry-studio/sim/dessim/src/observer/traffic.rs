//! Per-cell elapsed traffic, measured from committed motion rather than animation
//! samples. Each link belongs to its source for its first half of travel distance
//! and to its destination for its second half, including curves and map gaps.

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::pathfinding::TrapezoidalProfile;

/// Duration-weighted speed bins: [0, .5), [.5, 1), …, [4.5, 5), [5, infinity) m/s.
pub const SPEED_BINS: usize = 11;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MotionSpan {
    pub source: u32,
    pub destination: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub distance_mm: f64,
    pub profile: Option<TrapezoidalProfile>,
}

#[derive(Clone, Debug, Default)]
pub struct CellTraffic {
    pub distance_mm: f64,
    pub moving_ms: f64,
    pub wait_ms: f64,
    pub rotation_ms: f64,
    pub observed_ms: f64,
    pub speed_ms: [f64; SPEED_BINS],
}

impl CellTraffic {
    pub fn moving_speed_m_s(&self) -> Option<f64> {
        (self.moving_ms > 0.0).then(|| self.distance_mm / self.moving_ms)
    }

    /// The duration-weighted median's containing bin, not an exact quantile.
    pub fn median_bin(&self) -> Option<usize> {
        let target = self.speed_ms.iter().sum::<f64>() / 2.0;
        if target <= 0.0 {
            return None;
        }
        let mut sum = 0.0;
        self.speed_ms.iter().position(|duration| {
            sum += duration;
            sum >= target
        })
    }
}

impl MotionSpan {
    /// Integrate the observed part of each acceleration/cruise/deceleration phase.
    /// Split at speed-bin boundaries and the link midpoint to conserve distance
    /// and duration, independent of slicing or playback microstep cadence.
    pub fn accumulate(&self, from_ms: u64, until_ms: u64, cells: &mut FxHashMap<u32, CellTraffic>) {
        let lo = from_ms.max(self.start_ms) as f64;
        let hi = until_ms.min(self.end_ms) as f64;
        let duration_s = self.end_ms.saturating_sub(self.start_ms) as f64 / 1000.0;
        if hi <= lo || duration_s <= 0.0 || self.distance_mm <= 0.0 {
            return;
        }
        let phases = if let Some(p) = self.profile.filter(|p| p.total_time_s > 0.0 && p.total_distance_mm > 0.0) {
            let time_scale = duration_s / p.total_time_s;
            let velocity_scale = self.distance_mm / p.total_distance_mm / time_scale;
            [
                (
                    p.accel_time_s * time_scale,
                    p.entry_velocity_mm_s * velocity_scale,
                    p.peak_velocity_mm_s * velocity_scale,
                ),
                (
                    p.cruise_time_s * time_scale,
                    p.peak_velocity_mm_s * velocity_scale,
                    p.peak_velocity_mm_s * velocity_scale,
                ),
                (
                    p.decel_time_s * time_scale,
                    p.peak_velocity_mm_s * velocity_scale,
                    p.exit_velocity_mm_s * velocity_scale,
                ),
            ]
        } else {
            let v = self.distance_mm / duration_s;
            [(duration_s, v, v), (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)]
        };
        let mut phase_start = self.start_ms as f64;
        let mut distance_before = 0.0;
        for (seconds, v0, v1) in phases {
            if seconds <= 0.0 {
                continue;
            }
            let acceleration = (v1 - v0) / seconds;
            let position = |t: f64| v0 * t + acceleration * t * t / 2.0;
            let left = ((lo - phase_start) / 1000.0).clamp(0.0, seconds);
            let right = ((hi - phase_start) / 1000.0).clamp(0.0, seconds);
            if right > left {
                let mut cuts = vec![left, right];
                if acceleration.abs() > 1e-9 {
                    for bin in 1..SPEED_BINS {
                        let t = (bin as f64 * 500.0 - v0) / acceleration;
                        if t > left && t < right {
                            cuts.push(t);
                        }
                    }
                }
                let half = self.distance_mm / 2.0 - distance_before;
                if half > position(left) && half < position(right) {
                    let speed = (v0 * v0 + 2.0 * acceleration * half).max(0.0).sqrt();
                    let t = 2.0 * half / (v0 + speed);
                    if t > left && t < right {
                        cuts.push(t);
                    }
                }
                cuts.sort_by(f64::total_cmp);
                for pair in cuts.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    let midpoint = (a + b) / 2.0;
                    let cell = if distance_before + position(midpoint) < self.distance_mm / 2.0 {
                        self.source
                    } else {
                        self.destination
                    };
                    let row = cells.entry(cell).or_default();
                    let ms = (b - a) * 1000.0;
                    row.distance_mm += position(b) - position(a);
                    row.moving_ms += ms;
                    row.observed_ms += ms;
                    let bin = (((v0 + acceleration * midpoint) / 500.0).max(0.0) as usize).min(SPEED_BINS - 1);
                    row.speed_ms[bin] += ms;
                }
            }
            phase_start += seconds * 1000.0;
            distance_before += position(seconds);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cruise(speed: f64) -> MotionSpan {
        MotionSpan { source: 1, destination: 2, start_ms: 0, end_ms: 4000, distance_mm: speed * 4000.0, profile: None }
    }

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-7, "{a} != {b}");
    }

    #[test]
    fn link_halves_conserve_distance_and_clip_to_observed_time() {
        let span = cruise(2.0);
        let mut cells = FxHashMap::default();
        span.accumulate(1000, 3000, &mut cells);
        for id in [1, 2] {
            close(cells[&id].distance_mm, 2000.0);
            close(cells[&id].moving_ms, 1000.0);
            close(cells[&id].moving_speed_m_s().unwrap(), 2.0);
        }
        cells.clear();
        span.accumulate(0, 1500, &mut cells);
        assert!(!cells.contains_key(&2));
        close(cells[&1].moving_ms, 1500.0);
        close(cells[&1].distance_mm, 3000.0);
    }

    #[test]
    fn accelerated_motion_is_invariant_to_observation_boundaries() {
        let mut span = cruise(1.5);
        span.profile = Some(TrapezoidalProfile {
            entry_velocity_mm_s: 0.0,
            exit_velocity_mm_s: 0.0,
            peak_velocity_mm_s: 2000.0,
            accel_time_s: 1.0,
            cruise_time_s: 2.0,
            decel_time_s: 1.0,
            accel_distance_mm: 1000.0,
            cruise_distance_mm: 4000.0,
            decel_distance_mm: 1000.0,
            total_time_s: 4.0,
            total_distance_mm: 6000.0,
            acceleration_mm_s2: 2000.0,
            deceleration_mm_s2: 2000.0,
        });
        let mut whole = FxHashMap::default();
        let mut split = FxHashMap::default();
        span.accumulate(0, 4000, &mut whole);
        for bounds in [0, 237, 1000, 1753, 2000, 3999, 4000].windows(2) {
            span.accumulate(bounds[0], bounds[1], &mut split);
        }
        for id in [1, 2] {
            close(whole[&id].distance_mm, 3000.0);
            close(whole[&id].moving_ms, 2000.0);
            close(split[&id].distance_mm, whole[&id].distance_mm);
            close(split[&id].moving_ms, whole[&id].moving_ms);
            for (a, b) in whole[&id].speed_ms.iter().zip(split[&id].speed_ms) {
                close(*a, b);
            }
        }
    }

    #[test]
    fn two_speed_populations_remain_distinct_in_duration_histogram() {
        let mut cells = FxHashMap::default();
        cruise(0.75).accumulate(0, 4000, &mut cells);
        cruise(4.25).accumulate(0, 4000, &mut cells);
        let row = &cells[&1];
        close(row.moving_speed_m_s().unwrap(), 2.5);
        close(row.speed_ms[1], 2000.0);
        close(row.speed_ms[8], 2000.0);
        assert_eq!(row.speed_ms.iter().filter(|v| **v > 0.0).count(), 2);
        assert_eq!(row.median_bin(), Some(1));
    }
}
