//! Within `bench`, `progress` is THE BAR A HEADLESS RUN DRAWS: sim time
//! against the configured duration, with picks, picks per hour and orders
//! completed beside it.
//!
//! It lives in desvis rather than in the harness because it is a terminal's
//! business. The run itself shows its progress in plain figures, from the
//! slices made final so far; who reads them, and whether anyone draws them,
//! is the caller's choice — the Crucible server reads the same figures to
//! answer `status`.

use std::io::IsTerminal;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use dessim::sim_types::Counters;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

/// Renders a live progress bar for headless runs by polling the run's
/// `Counters` on a background thread. Dropping the reporter stops the thread and clears the
/// bar.
pub(super) struct ProgressReporter {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    bar: ProgressBar,
}

impl ProgressReporter {
    pub(super) fn start(counters: Counters, sim_duration: Duration) -> Self {
        let total_ms = (sim_duration.as_millis() as u64).max(1);
        // Hidden when stderr isn't a terminal (piped/redirected output) so we
        // don't write control codes into captured logs.
        let target =
            if std::io::stderr().is_terminal() { ProgressDrawTarget::stderr() } else { ProgressDrawTarget::hidden() };
        let bar = ProgressBar::with_draw_target(Some(total_ms), target);
        bar.set_style(
            ProgressStyle::with_template(
                "sim [{bar:32.cyan/blue}] {percent:>3}% · {msg} · wall {elapsed_precise} eta {eta_precise}",
            )
            .expect("valid progress template")
            .progress_chars("=> "),
        );
        let desired = fmt_sim_hms(total_ms);
        // Seed the message before the steady tick starts so the first frame
        // drawn already shows the zero-state instead of an empty line.
        bar.set_message(format!("0:00:00/{desired} · 0 picks · 0/h · 0 ord"));
        bar.enable_steady_tick(Duration::from_millis(120));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = std::thread::spawn({
            let bar = bar.clone();
            let stop = stop.clone();
            move || {
                while !stop.load(Ordering::Relaxed) {
                    let sim_ms = counters.shown_sim_time().as_ms();
                    let picks = counters.shown_picks();
                    let orders = counters.shown_orders_completed();
                    let pph = if sim_ms > 0 { picks as f64 * 3_600_000.0 / sim_ms as f64 } else { 0.0 };
                    bar.set_position(sim_ms.min(total_ms));
                    bar.set_message(format!(
                        "{}/{} · {} picks · {:.0}/h · {} ord",
                        fmt_sim_hms(sim_ms),
                        desired,
                        picks,
                        pph,
                        orders,
                    ));
                    std::thread::sleep(Duration::from_millis(125));
                }
            }
        });
        Self { stop, handle: Some(handle), bar }
    }
}

impl Drop for ProgressReporter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        self.bar.finish_and_clear();
    }
}

/// Format a millisecond sim-clock duration as `H:MM:SS`.
fn fmt_sim_hms(ms: u64) -> String {
    let secs = ms / 1000;
    format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}
