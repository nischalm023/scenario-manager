//! Statistical comparison of two `.worldview` recordings.
//!
//! Reads two profiles, aligns their time slices by `start_ms`, and computes
//! per-metric aggregate statistics, time-series shape metrics, and
//! distribution shift analysis.

mod stats;

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use dessim::metrics::profile::{PerfProfile, SliceRecord};
use dessim::metrics::worldview::read_worldview;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// One recording, with a word about it when the run behind it never ended: a
/// crashed run's slices are sound as far as they go, but its totals are not
/// there, so a comparison against it is a comparison against a shorter run.
fn opened(path: &Path, which: &str) -> PerfProfile {
    let recording = read_worldview(path)
        .unwrap_or_else(|e| panic!("failed to read the {which} recording {}: {}", path.display(), e));
    if recording.truncated {
        eprintln!(
            "note: {} has no end block — the run crashed or is still going. Its last intact slice is {} ms, and \
             its summary and result are absent",
            path.display(),
            recording.profile.time_slices.last().map(|slice| slice.start_ms).unwrap_or(0),
        );
    }
    recording.profile
}

/// Load two profiles and print a deep statistical comparison.
pub(crate) fn run_compare(old_path: &Path, new_path: &Path) {
    let old = opened(old_path, "old");
    let new = opened(new_path, "new");

    println!("=== Deep Statistical Comparison ===");
    println!();
    println!("  Old: {} ({})", old.name, old.metadata.commit_short);
    println!("  New: {} ({})", new.name, new.metadata.commit_short);
    println!("  Old slices: {}   New slices: {}", old.time_slices.len(), new.time_slices.len());

    // Align slices by start_ms.
    let aligned = align_slices(&old.time_slices, &new.time_slices);
    if aligned.is_empty() {
        println!();
        println!("  No overlapping time slices found. Cannot compare.");
        return;
    }
    println!("  Aligned slices: {}", aligned.len());

    // Collect all metric keys present in both profiles.
    let metric_keys = common_metric_keys(&aligned);
    if metric_keys.is_empty() {
        println!();
        println!("  No common metrics found across aligned slices.");
        return;
    }

    // Compute per-metric analysis.
    let mut analyses: Vec<MetricAnalysis> =
        metric_keys.iter().filter_map(|key| stats::analyze_metric(key, &aligned)).collect();

    // Sort: regressions first, then by absolute delta% descending.
    analyses.sort_by(|a, b| {
        let va = verdict_order(&a.verdict);
        let vb = verdict_order(&b.verdict);
        va.cmp(&vb).then_with(|| b.delta_pct.abs().partial_cmp(&a.delta_pct.abs()).unwrap_or(Ordering::Equal))
    });

    // Print summary table.
    print_summary_table(&analyses);

    // Print detailed per-metric breakdown for non-unchanged metrics.
    let interesting: Vec<&MetricAnalysis> = analyses.iter().filter(|a| a.verdict != Verdict::Unchanged).collect();

    if !interesting.is_empty() {
        println!();
        println!("=== Detailed Breakdown ===");
        for a in &interesting {
            print_metric_detail(a);
        }
    }

    // Report metrics only in one profile.
    report_exclusive_metrics(&old, &new);
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Aligned pair of slice records (same start_ms).
pub(super) struct AlignedSlice<'a> {
    pub old: &'a SliceRecord,
    pub new: &'a SliceRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Verdict {
    Improved,
    Regressed,
    Unchanged,
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Verdict::Improved => write!(f, "IMPROVED"),
            Verdict::Regressed => write!(f, "REGRESSED"),
            Verdict::Unchanged => write!(f, "unchanged"),
        }
    }
}

fn verdict_order(v: &Verdict) -> u8 {
    match v {
        Verdict::Regressed => 0,
        Verdict::Improved => 1,
        Verdict::Unchanged => 2,
    }
}

pub(super) struct MetricAnalysis {
    pub name: String,
    pub old_mean: f64,
    pub new_mean: f64,
    pub old_median: f64,
    pub new_median: f64,
    pub old_stddev: f64,
    pub new_stddev: f64,
    pub delta_pct: f64,
    pub smoothness_old: f64,
    pub smoothness_new: f64,
    pub ramp_old: usize,
    pub ramp_new: usize,
    pub cv_old: f64,
    pub cv_new: f64,
    pub sustained_old: f64,
    pub sustained_new: f64,
    pub frac_improved: f64,
    pub frac_regressed: f64,
    pub max_regression: f64,
    pub verdict: Verdict,
    pub flag_mixed: bool,
}

// ---------------------------------------------------------------------------
// Slice alignment
// ---------------------------------------------------------------------------

fn align_slices<'a>(old: &'a [SliceRecord], new: &'a [SliceRecord]) -> Vec<AlignedSlice<'a>> {
    let old_by_start: HashMap<u64, &SliceRecord> = old.iter().map(|s| (s.start_ms, s)).collect();

    let mut aligned = Vec::new();
    for ns in new {
        if let Some(os) = old_by_start.get(&ns.start_ms) {
            aligned.push(AlignedSlice { old: os, new: ns });
        }
    }
    aligned
}

fn common_metric_keys(aligned: &[AlignedSlice]) -> Vec<String> {
    let mut keys = BTreeSet::new();
    for pair in aligned {
        for key in pair.old.metrics.keys() {
            if pair.new.metrics.contains_key(key) {
                keys.insert(key.clone());
            }
        }
    }
    keys.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

fn print_summary_table(analyses: &[MetricAnalysis]) {
    println!();
    println!(
        "  {:<30} {:>10} {:>10} {:>9} {:>8} {:>8} {:>10}",
        "Metric", "Old Mean", "New Mean", "Delta%", "Regr%", "Impr%", "Verdict"
    );
    println!("  {:-<30} {:-<10} {:-<10} {:-<9} {:-<8} {:-<8} {:-<10}", "", "", "", "", "", "", "");

    for a in analyses {
        let flag = if a.flag_mixed { " !" } else { "" };
        println!(
            "  {:<30} {:>10.1} {:>10.1} {:>+8.1}% {:>7.0}% {:>7.0}% {:>10}{}",
            truncate(&a.name, 30),
            a.old_mean,
            a.new_mean,
            a.delta_pct,
            a.frac_regressed * 100.0,
            a.frac_improved * 100.0,
            a.verdict,
            flag,
        );
    }

    // Legend.
    let flagged = analyses.iter().any(|a| a.flag_mixed);
    if flagged {
        println!();
        println!("  ! = aggregate improved but >10% of slices regressed");
    }
}

fn print_metric_detail(a: &MetricAnalysis) {
    println!();
    println!("  --- {} ({}) ---", a.name, a.verdict);
    println!("    Aggregate:  old_mean={:.2}  new_mean={:.2}  delta={:+.1}%", a.old_mean, a.new_mean, a.delta_pct);
    println!("    Median:     old={:.2}  new={:.2}", a.old_median, a.new_median);
    println!("    Stddev:     old={:.2}  new={:.2}", a.old_stddev, a.new_stddev);
    println!(
        "    Smoothness: old={:.2}  new={:.2}  (first-diff variance, lower=smoother)",
        a.smoothness_old, a.smoothness_new
    );
    println!("    Ramp-up:    old={} slices  new={} slices  (to 80% of mean)", a.ramp_old, a.ramp_new);
    println!("    CV (steady): old={:.3}  new={:.3}  (lower=more consistent)", a.cv_old, a.cv_new);
    println!(
        "    Sustained:  old={:.3}  new={:.3}  (last-quartile / overall mean, 1.0=stable)",
        a.sustained_old, a.sustained_new
    );
    println!(
        "    Shift:      {:.0}% improved, {:.0}% regressed, max_regression={:.2}",
        a.frac_improved * 100.0,
        a.frac_regressed * 100.0,
        a.max_regression
    );
    if a.flag_mixed {
        println!("    WARNING: aggregate improved but >10% of slices regressed");
    }
}

fn report_exclusive_metrics(old: &PerfProfile, new: &PerfProfile) {
    let mut old_keys = BTreeSet::new();
    let mut new_keys = BTreeSet::new();

    for slice in &old.time_slices {
        for key in slice.metrics.keys() {
            old_keys.insert(key.clone());
        }
    }
    for slice in &new.time_slices {
        for key in slice.metrics.keys() {
            new_keys.insert(key.clone());
        }
    }

    let only_old: Vec<_> = old_keys.difference(&new_keys).collect();
    let only_new: Vec<_> = new_keys.difference(&old_keys).collect();

    if !only_old.is_empty() || !only_new.is_empty() {
        println!();
        println!("=== Metrics Present In Only One Profile ===");
        if !only_old.is_empty() {
            println!("  Only in old: {}", only_old.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
        }
        if !only_new.is_empty() {
            println!("  Only in new: {}", only_new.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}...", &s[..max - 3]) }
}
