//! Statistical helpers and per-metric analysis.

use std::cmp::Ordering;

use super::{AlignedSlice, MetricAnalysis, Verdict};

// ---------------------------------------------------------------------------
// Per-metric analysis
// ---------------------------------------------------------------------------

fn is_lower_better(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.contains("idle") || k.contains("fail") || k.contains("error") || k.contains("latency") || k.contains("ramp")
}

/// Compute aggregate statistics, time-series shape, and verdict for one metric.
pub(super) fn analyze_metric(key: &str, aligned: &[AlignedSlice]) -> Option<MetricAnalysis> {
    // A run states a metric only in the slices where the thing it measures
    // existed — `htm_state_ForkOp` is absent from a slice in which no D30
    // was in a fork operation — so the comparison runs over the slices that state
    // it on both sides. Reading an absent metric as zero would move every
    // average it appears in.
    let stated: Vec<(f64, f64)> =
        aligned.iter().filter_map(|pair| Some((*pair.old.metrics.get(key)?, *pair.new.metrics.get(key)?))).collect();
    let old_vals: Vec<f64> = stated.iter().map(|(old, _)| *old).collect();
    let new_vals: Vec<f64> = stated.iter().map(|(_, new)| *new).collect();

    if old_vals.is_empty() {
        return None;
    }

    // Aggregate statistics.
    let old_mean = mean(&old_vals);
    let new_mean = mean(&new_vals);
    let old_median = median(&old_vals);
    let new_median = median(&new_vals);
    let old_stddev = stddev(&old_vals, old_mean);
    let new_stddev = stddev(&new_vals, new_mean);

    let delta_pct = if old_mean.abs() > 1e-12 {
        ((new_mean - old_mean) / old_mean) * 100.0
    } else if new_mean.abs() > 1e-12 {
        100.0 // went from zero to something
    } else {
        0.0
    };

    // Time-series shape: smoothness (first-difference variance).
    let smoothness_old = first_diff_variance(&old_vals);
    let smoothness_new = first_diff_variance(&new_vals);

    // Ramp-up: slices until metric exceeds 80% of its eventual mean.
    let ramp_old = ramp_up_slices(&old_vals, old_mean);
    let ramp_new = ramp_up_slices(&new_vals, new_mean);

    // Consistency: coefficient of variation of steady-state portion (after ramp).
    let cv_old = coeff_of_variation(&old_vals[ramp_old..]);
    let cv_new = coeff_of_variation(&new_vals[ramp_new..]);

    // Sustained performance: last-quartile mean vs overall mean.
    let sustained_old = last_quartile_ratio(&old_vals, old_mean);
    let sustained_new = last_quartile_ratio(&new_vals, new_mean);

    // Distribution shift: per-slice deltas.
    let deltas: Vec<f64> = old_vals.iter().zip(new_vals.iter()).map(|(o, n)| n - o).collect();

    let (improved_count, regressed_count) = if is_lower_better(key) {
        (deltas.iter().filter(|&&d| d < 0.0).count(), deltas.iter().filter(|&&d| d > 0.0).count())
    } else {
        (deltas.iter().filter(|&&d| d > 0.0).count(), deltas.iter().filter(|&&d| d < 0.0).count())
    };
    let total = deltas.len() as f64;
    let frac_improved = improved_count as f64 / total;
    let frac_regressed = regressed_count as f64 / total;

    let max_regression = if is_lower_better(key) {
        deltas.iter().copied().filter(|&d| d > 0.0).fold(0.0_f64, f64::max)
    } else {
        deltas.iter().copied().filter(|&d| d < 0.0).map(|d| d.abs()).fold(0.0_f64, f64::max)
    };

    // Verdict logic (polarity-aware).
    let effective_delta = if is_lower_better(key) { -delta_pct } else { delta_pct };
    let verdict = if effective_delta > 1.0 && frac_regressed < 0.10 {
        Verdict::Improved
    } else if effective_delta < -1.0 || frac_regressed > 0.30 {
        Verdict::Regressed
    } else {
        Verdict::Unchanged
    };

    // Flag: aggregate improved but > 10% of slices regressed.
    let flag_mixed = delta_pct > 1.0 && frac_regressed > 0.10;

    Some(MetricAnalysis {
        name: key.to_string(),
        old_mean,
        new_mean,
        old_median,
        new_median,
        old_stddev,
        new_stddev,
        delta_pct,
        smoothness_old,
        smoothness_new,
        ramp_old,
        ramp_new,
        cv_old,
        cv_new,
        sustained_old,
        sustained_new,
        frac_improved,
        frac_regressed,
        max_regression,
        verdict,
        flag_mixed,
    })
}

// ---------------------------------------------------------------------------
// Statistical helpers (no external crate)
// ---------------------------------------------------------------------------

fn mean(vals: &[f64]) -> f64 {
    if vals.is_empty() {
        return 0.0;
    }
    vals.iter().sum::<f64>() / vals.len() as f64
}

fn median(vals: &[f64]) -> f64 {
    if vals.is_empty() {
        return 0.0;
    }
    let mut sorted = vals.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let n = sorted.len();
    if n % 2 == 0 { (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0 } else { sorted[n / 2] }
}

fn stddev(vals: &[f64], m: f64) -> f64 {
    if vals.len() < 2 {
        return 0.0;
    }
    let var = vals.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (vals.len() - 1) as f64;
    var.sqrt()
}

fn first_diff_variance(vals: &[f64]) -> f64 {
    if vals.len() < 2 {
        return 0.0;
    }
    let diffs: Vec<f64> = vals.windows(2).map(|w| w[1] - w[0]).collect();
    let m = mean(&diffs);
    let var = diffs.iter().map(|d| (d - m).powi(2)).sum::<f64>() / diffs.len() as f64;
    var
}

fn ramp_up_slices(vals: &[f64], eventual_mean: f64) -> usize {
    if eventual_mean.abs() < 1e-12 {
        return 0;
    }
    let threshold = 0.8 * eventual_mean;
    for (i, &v) in vals.iter().enumerate() {
        if v >= threshold {
            return i;
        }
    }
    vals.len()
}

fn coeff_of_variation(vals: &[f64]) -> f64 {
    if vals.is_empty() {
        return 0.0;
    }
    let m = mean(vals);
    if m.abs() < 1e-12 {
        return 0.0;
    }
    let sd = stddev(vals, m);
    sd / m
}

fn last_quartile_ratio(vals: &[f64], overall_mean: f64) -> f64 {
    if vals.is_empty() || overall_mean.abs() < 1e-12 {
        return 1.0;
    }
    let start = vals.len() * 3 / 4;
    let tail = &vals[start..];
    if tail.is_empty() {
        return 1.0;
    }
    mean(tail) / overall_mean
}
