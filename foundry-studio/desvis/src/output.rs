//! Within desvis, `output` is WHAT A FINISHED RUN PRINTS AND LEAVES BEHIND:
//! where its recording is written, the results table, and the side-by-side
//! comparison of two runs.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use dessim::StopReason;
use dessim::events::{ToteSource, VtmCycleDirection, VtmIdleReasonKind};
use dessim::harness::RunIdentity;
use dessim::metrics::profile::{PerfProfile, ScenarioSource, SummaryStats};
use dessim::metrics::worldview::EXTENSION;
use dessim::model::RobotId;
use dessim::observer::duty::DutyBreakdown;

/// Where a run on the code `identity` names records itself in `output_dir`:
/// one file per branch and commit.
pub(crate) fn recording_path(identity: &RunIdentity, output_dir: &Path) -> PathBuf {
    let branch_slug = identity.branch.replace('/', "_");
    output_dir.join(format!("{}_{}.{EXTENSION}", branch_slug, identity.commit_short))
}

/// Print benchmark results summary to stdout (with profile metadata).
pub(crate) fn print_summary(prof: &PerfProfile, summary: &SummaryStats, profile_path: &Path) {
    let file_size = std::fs::metadata(profile_path).map(|m| m.len()).unwrap_or(0);

    println!();
    println!("=== Benchmark Profile ===");
    println!("  Name:        {}", prof.name);
    println!("  Branch:      {}", prof.metadata.branch);
    println!("  Commit:      {}", prof.metadata.commit_short);
    println!("  Recording:   {} ({:.1} KB)", profile_path.display(), file_size as f64 / 1024.0);
    print_results(summary);
}

/// Print results summary to stdout (no profile metadata).
pub(crate) fn print_results(summary: &SummaryStats) {
    println!();
    println!("=== Results ===");
    println!("  Wall clock:  {:.1}s", summary.wall_clock_secs);
    println!("  Sim time:    {:.0}s ({:.1}h)", summary.sim_time_secs, summary.sim_time_secs / 3600.0);
    println!("  Picks:       {} ({:.0}/hr)", summary.total_picks, summary.throughput_picks_per_hour);
    println!("  Putaways:    {} ({:.0}/hr)", summary.total_putaways, summary.throughput_putaway_items_per_hour);
    println!(
        "  HTM eff m/s: {:.2} loaded / {:.2} empty / {:.2} all  (measured, including rotation)",
        summary.htm_effective_speed_loaded_mps,
        summary.htm_effective_speed_empty_mps,
        summary.htm_effective_speed_combined_mps,
    );
    println!("  Fully picked orders: {}", summary.total_orders_completed);
    println!("  Events:      {}", summary.event_count);
    println!("  DA50 idle:   {:.1}%", summary.vtm_idle_pct * 100.0);
    println!("  D30 idle:    {:.1}%", summary.htm_idle_pct * 100.0);
    println!("  Picked units/stn/hr: {:.1}", summary.avg_presentations_per_station_per_hour);
    println!("  PF fail:     {:.2}%", summary.pathfinder_failure_rate * 100.0);
    println!("  Flow score:  {:.3}  (fraction of gaps <= pick_time)", summary.flow_score);

    let sim_elapsed_ms = (summary.sim_time_secs * 1000.0) as u64;
    print_per_bot_duty("DA50", "A", &summary.vtm_duty_totals, summary.vtm_fleet_size, sim_elapsed_ms);
    print_duty_histogram("D30", "D", &summary.htm_duty_totals, summary.htm_fleet_size, sim_elapsed_ms);

    if !summary.station_gap_stats.is_empty() {
        println!();
        println!("=== Station Gap Analysis (inter-presentation idle time) ===");
        println!("  {:<10} {:>6} {:>8} {:>8} {:>8} {:>8}", "Station", "Count", "Min ms", "Mean ms", "P95 ms", "Max ms");
        println!("  {:-<10} {:-<6} {:-<8} {:-<8} {:-<8} {:-<8}", "", "", "", "", "", "");
        for gs in &summary.station_gap_stats {
            println!(
                "  {:<10} {:>6} {:>8} {:>8.0} {:>8} {:>8}",
                format!("Stn {:>2}", gs.station_id.0),
                gs.count,
                gs.min_ms,
                gs.mean_ms,
                gs.p95_ms,
                gs.max_ms,
            );
        }
    }

    print_vtm_idle_gap_by_source(summary);
}

/// Diagnostic: attribute VTM pre-cycle idle gap to the pick vs. putaway flow,
/// split by cycle direction and wait-reason. Folds over the already-populated
/// `vtm_cycle_log` at print time (no serialized aggregate — a HashMap with a
/// tuple/enum key does not round-trip through the profile JSON). The
/// `Return × AwaitingHtmDeposit` rows are the payload: how much of the
/// return-cycle deposit wait each flow manufactures.
fn print_vtm_idle_gap_by_source(summary: &SummaryStats) {
    if summary.vtm_cycle_log.is_empty() {
        return;
    }

    // Key: (source, direction, reason) → (cycle_count, summed pre_cycle_wait_ms).
    // The shared enums don't all derive `Hash`/`Ord`, and the key space is
    // tiny (2 sources × 2 directions × 5 reasons), so accumulate into a `Vec`
    // with linear lookup rather than a HashMap.
    let mut rows: Vec<((ToteSource, VtmCycleDirection, VtmIdleReasonKind), (u64, u64))> = Vec::new();
    for rec in &summary.vtm_cycle_log {
        let key = (rec.tote_source, rec.direction, rec.wait_reason);
        match rows.iter_mut().find(|(k, _)| *k == key) {
            Some((_, acc)) => {
                acc.0 += 1;
                acc.1 += rec.pre_cycle_wait_ms;
            }
            None => rows.push((key, (1, rec.pre_cycle_wait_ms))),
        }
    }

    // Sort by descending summed gap so the biggest reclaimable waits lead;
    // stable tiebreak on a numeric (source, direction, reason) key so the
    // table order is deterministic (the shared enums don't derive `Ord`).
    rows.sort_by(|a, b| b.1.1.cmp(&a.1.1).then_with(|| sort_key(a.0).cmp(&sort_key(b.0))));

    println!();
    println!("=== VTM pre-cycle idle gap by source × direction × reason ===");
    println!("  {:<8} {:<7} {:<18} {:>7} {:>9} {:>9}", "Source", "Dir", "Reason", "Cycles", "Gap(s)", "Mean(ms)");
    println!("  {:-<8} {:-<7} {:-<18} {:-<7} {:-<9} {:-<9}", "", "", "", "", "", "");
    for ((source, direction, reason), (count, gap_ms)) in rows {
        let mean_ms = if count > 0 { gap_ms / count } else { 0 };
        println!(
            "  {:<8} {:<7} {:<18} {:>7} {:>9.1} {:>9}",
            source.to_string(),
            direction_label(direction),
            reason_label(reason),
            count,
            gap_ms as f64 / 1000.0,
            mean_ms,
        );
    }
}

/// Deterministic tiebreak key for the idle-gap table rows. Orders by
/// source, then direction, then reason — the shared enums don't derive
/// `Ord`, so map each to a small integer tuple here.
fn sort_key(key: (ToteSource, VtmCycleDirection, VtmIdleReasonKind)) -> (u8, u8, u8) {
    let (source, direction, reason) = key;
    let s = match source {
        ToteSource::Pick => 0,
        ToteSource::Putaway => 1,
    };
    let d = match direction {
        VtmCycleDirection::Forward => 0,
        VtmCycleDirection::Return => 1,
    };
    let r = match reason {
        VtmIdleReasonKind::Continuous => 0,
        VtmIdleReasonKind::AwaitingDemand => 1,
        VtmIdleReasonKind::AwaitingTote => 2,
        VtmIdleReasonKind::AwaitingSlot => 3,
        VtmIdleReasonKind::AwaitingHtmDeposit => 4,
    };
    (s, d, r)
}

/// Short column label for a VTM cycle direction.
fn direction_label(direction: VtmCycleDirection) -> &'static str {
    match direction {
        VtmCycleDirection::Forward => "Forward",
        VtmCycleDirection::Return => "Return",
    }
}

/// Short column label for a VTM idle-reason kind.
fn reason_label(reason: VtmIdleReasonKind) -> &'static str {
    match reason {
        VtmIdleReasonKind::Continuous => "Continuous",
        VtmIdleReasonKind::AwaitingDemand => "AwaitingDemand",
        VtmIdleReasonKind::AwaitingTote => "AwaitingTote",
        VtmIdleReasonKind::AwaitingSlot => "AwaitingSlot",
        VtmIdleReasonKind::AwaitingHtmDeposit => "AwaitingHtmDeposit",
    }
}

/// Print side-by-side comparison of old vs new summary stats.
pub(crate) fn print_comparison(old: &SummaryStats, new: &SummaryStats, old_path: &Path, new_path: &Path) {
    println!();
    println!("=== Comparison ===");
    println!(
        "  Old profile: {}",
        old_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| old_path.to_string_lossy().to_string())
    );
    println!(
        "  New profile: {}",
        new_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| new_path.to_string_lossy().to_string())
    );
    println!();
    println!("  {:<20} {:>12} {:>12} {:>10}", "Metric", "Old", "New", "Delta");
    println!("  {:-<20} {:-<12} {:-<12} {:-<10}", "", "", "", "");

    let metrics: &[(&str, f64, f64, bool)] = &[
        ("Throughput (p/hr)", old.throughput_picks_per_hour, new.throughput_picks_per_hour, false),
        (
            "Picked units/stn/hr",
            old.avg_presentations_per_station_per_hour,
            new.avg_presentations_per_station_per_hour,
            false,
        ),
        ("Picks", old.total_picks as f64, new.total_picks as f64, false),
        ("DA50 idle %", old.vtm_idle_pct * 100.0, new.vtm_idle_pct * 100.0, true),
        ("D30 idle %", old.htm_idle_pct * 100.0, new.htm_idle_pct * 100.0, true),
        ("Wall clock (s)", old.wall_clock_secs, new.wall_clock_secs, true),
        ("Fully picked orders", old.total_orders_completed as f64, new.total_orders_completed as f64, false),
        ("PF fail %", old.pathfinder_failure_rate * 100.0, new.pathfinder_failure_rate * 100.0, true),
        ("Flow score", old.flow_score, new.flow_score, false),
    ];
    for &(label, o, n, lower_better) in metrics {
        print_cmp_row(label, o, n, lower_better);
    }

    // Profiles captured before the configured-fleet idle denominator
    // (greymatter-foundry#254 rule) carry fleet_size 0 and computed idle
    // over dispatched robots only. Across that boundary an idle delta is
    // definitional, not a performance change — say so instead of letting
    // the regression marker stand unqualified.
    let legacy = |s: &SummaryStats| s.vtm_fleet_size == 0 && s.htm_fleet_size == 0;
    if legacy(old) != legacy(new) {
        println!();
        println!("  note: idle % semantics differ between these profiles (legacy = dispatched");
        println!("        robots only; current = configured fleet). Idle deltas above are");
        println!("        definitional, not behavioral.");
    }
}

/// Per-bot duty breakdown for small fleets (VTMs): one row per bot in a
/// sorted-ascending grid, then a summary line. Sorting puts cold bots at
/// the top so starvation tails are visible at a glance.
fn print_per_bot_duty(
    label: &str,
    prefix: &str,
    duty: &HashMap<RobotId, DutyBreakdown>,
    fleet_size: usize,
    sim_elapsed_ms: u64,
) {
    if duty.is_empty() || sim_elapsed_ms == 0 {
        return;
    }
    let total_ms = sim_elapsed_ms as f64;
    let mut bots: Vec<(RobotId, f64)> =
        duty.iter().map(|(&id, d)| (id, d.busy().as_ms() as f64 / total_ms * 100.0)).collect();
    bots.sort_by(|a, b| a.1.total_cmp(&b.1));
    let n = bots.len();
    let mean = bots.iter().map(|(_, p)| *p).sum::<f64>() / n as f64;
    let var = bots.iter().map(|(_, p)| (*p - mean).powi(2)).sum::<f64>() / n as f64;
    let stddev = var.sqrt();
    let (min_id, min_pct) = bots[0];
    let (max_id, max_pct) = bots[n - 1];

    let cols = 5;
    let rows = n.div_ceil(cols);
    println!();
    let population = duty_population_label(n, fleet_size);
    println!("=== {label} per-bot duty ({population}, sorted asc) ===");
    for row in 0..rows {
        let mut line = String::from("  ");
        for col in 0..cols {
            let idx = row * cols + col;
            if idx < n {
                let (id, pct) = bots[idx];
                line.push_str(&format!("{prefix}{:>3}:{pct:>5.1}%   ", id.0));
            }
        }
        println!("{}", line.trim_end());
    }
    println!(
        "  busy: mean={:.1}%  stddev={:.1}%  min={:.1}% ({prefix}{}) max={:.1}% ({prefix}{})  range={:.1}pp",
        mean,
        stddev,
        min_pct,
        min_id.0,
        max_pct,
        max_id.0,
        max_pct - min_pct
    );
}

/// Per-bot duty histogram for large fleets (HTMs): 10 fixed bins, bar
/// widths normalized to the largest bin, then a summary line. Used when a
/// flat list would be too long to scan.
fn print_duty_histogram(
    label: &str,
    prefix: &str,
    duty: &HashMap<RobotId, DutyBreakdown>,
    fleet_size: usize,
    sim_elapsed_ms: u64,
) {
    if duty.is_empty() || sim_elapsed_ms == 0 {
        return;
    }
    let total_ms = sim_elapsed_ms as f64;
    let pcts: Vec<(RobotId, f64)> =
        duty.iter().map(|(&id, d)| (id, d.busy().as_ms() as f64 / total_ms * 100.0)).collect();
    let n = pcts.len();
    let mean = pcts.iter().map(|(_, p)| *p).sum::<f64>() / n as f64;
    let var = pcts.iter().map(|(_, p)| (*p - mean).powi(2)).sum::<f64>() / n as f64;
    let stddev = var.sqrt();
    let (min_id, min_pct) = *pcts.iter().min_by(|a, b| a.1.total_cmp(&b.1)).expect("non-empty checked above");
    let (max_id, max_pct) = *pcts.iter().max_by(|a, b| a.1.total_cmp(&b.1)).expect("non-empty checked above");

    let mut bins = [0usize; 10];
    for (_, pct) in &pcts {
        let bin = ((*pct / 10.0).floor() as usize).min(9);
        bins[bin] += 1;
    }
    let max_bin = bins.iter().copied().max().expect("ten-element array");
    const BAR_W: usize = 20;

    println!();
    let population = duty_population_label(n, fleet_size);
    println!("=== {label} duty histogram ({population}) ===");
    for (i, &count) in bins.iter().enumerate() {
        let lo = i * 10;
        let hi = lo + 10;
        let bar_w = if max_bin > 0 { count * BAR_W / max_bin } else { 0 };
        let bar: String = "#".repeat(bar_w);
        println!("  {:>3}-{:<3}% {:<width$} {}", lo, hi, bar, count, width = BAR_W);
    }
    println!(
        "  busy: mean={:.1}%  stddev={:.1}%  min={:.1}% ({prefix}{}) max={:.1}% ({prefix}{})  range={:.1}pp",
        mean,
        stddev,
        min_pct,
        min_id.0,
        max_pct,
        max_id.0,
        max_pct - min_pct
    );
}

/// Describe the population represented by a serialized duty map.
///
/// Current profiles carry one row per configured robot, including zero-duty
/// robots. A zero fleet size marks a legacy profile whose map may contain only
/// robots that worked. A nonzero mismatch is surfaced instead of being
/// mislabeled as either population.
fn duty_population_label(row_count: usize, fleet_size: usize) -> String {
    match fleet_size {
        0 => format!("{row_count} bots; legacy map may omit zero-duty robots"),
        configured if row_count == configured => {
            format!("{configured} configured; full roster")
        }
        configured => format!("{row_count} rows; expected {configured} configured"),
    }
}

fn print_cmp_row(label: &str, old: f64, new: f64, lower_is_better: bool) {
    let delta = new - old;
    let pct = if old.abs() > 1e-9 { (delta / old) * 100.0 } else { 0.0 };
    let sign = if delta > 0.0 { "+" } else { "" };

    let indicator = if delta.abs() < 1e-9 {
        "="
    } else if (lower_is_better && delta < 0.0) || (!lower_is_better && delta > 0.0) {
        "^" // improved
    } else {
        "v" // regressed
    };

    println!("  {:<20} {:>12.1} {:>12.1} {}{:.1}% {}", label, old, new, sign, pct, indicator);
}

/// Compute total expected picks from a scenario source.
pub(crate) fn expected_picks_from_source(source: &ScenarioSource) -> u64 {
    match source {
        ScenarioSource::Loaded { orders, .. } => orders.iter().flat_map(|o| &o.lines).map(|l| l.quantity as u64).sum(),
        ScenarioSource::Parametric(_) => 0,
    }
}

/// Report completion status based on why the simulation stopped.
///
/// Only checks pick counts when the sim claims to have completed naturally.
/// User-stop, watchdog, wall-timeout, and errors just print a status line.
pub(crate) fn check_completion(completed: u64, expected: u64, stop_reason: &StopReason) {
    use dessim::StopReason;
    match stop_reason {
        StopReason::Complete => {
            if expected == 0 {
                return;
            }
            if completed >= expected {
                println!("\nCompletion: {}/{} picks -- OK", completed, expected);
            } else {
                eprintln!(
                    "\nCompletion: {}/{} picks -- short (sim reported complete but picks missing)",
                    completed, expected
                );
            }
        }
        StopReason::UserStop => {
            println!("\nStopped by user: {}/{} picks completed.", completed, expected);
        }
        StopReason::Watchdog(detail) => {
            eprintln!("\nWatchdog halt: {}/{} picks completed. {detail}", completed, expected);
        }
        StopReason::WallTimeout => {
            println!("\nWall-clock timeout: {}/{} picks completed.", completed, expected);
        }
        StopReason::SimTimeLimit => {
            println!("\nSim time limit: {}/{} picks completed.", completed, expected);
        }
        StopReason::Error(detail) => {
            eprintln!("\nSimulation error: {}/{} picks completed. {detail}", completed, expected);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::duty_population_label;

    #[test]
    fn duty_population_label_distinguishes_current_legacy_and_mismatch() {
        assert_eq!(duty_population_label(340, 340), "340 configured; full roster");
        assert_eq!(duty_population_label(271, 0), "271 bots; legacy map may omit zero-duty robots");
        assert_eq!(duty_population_label(271, 340), "271 rows; expected 340 configured");
    }
}
