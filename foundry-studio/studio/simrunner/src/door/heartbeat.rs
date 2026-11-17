//! Within `door`, `heartbeat` is the runner saying it is alive and how far it is, every 5 s of wall clock from the
//! moment it starts until it exits: `PUT /runner/v1/attempts/{run}/{n}/heartbeat` with dessim's progress figures, the
//! simtime the run's final slices reach and what they state, zeros while the inputs are still being prepared. A beat that does not land is dropped, not asked
//! again; the next comes in 5 s. The API keeps the latest with its instant, and reads a running attempt not heard for
//! 30 s as stalled.

use std::time::Duration;

use dessim::sim_types::Counters;
use reqwest::Method;
use studio_protocol::{Heartbeat, JobSpec, ProgressValue};
use tokio::task::JoinHandle;
use tokio::time::sleep;

use super::{Door, chain};

pub const EVERY: Duration = Duration::from_secs(5);
const REQUEST_LIMIT: Duration = Duration::from_secs(2);

/// The beating task; dropping it stops the beats, as the attempt's end does.
pub struct Beating(JoinHandle<()>);

impl Beating {
    pub fn start(door: &Door, spec: &JobSpec, counters: Counters) -> Self {
        let (door, path) = (door.clone(), format!("/runner/v1/attempts/{}/{}/heartbeat", spec.run, spec.attempt.0));
        Self(tokio::spawn(async move {
            let mut missed = false;
            loop {
                let body = serde_json::to_vec(&beat_of(&counters)).expect("a heartbeat is JSON");
                let sent = door
                    .request(Method::PUT, &path)
                    .timeout(REQUEST_LIMIT)
                    .header("content-type", "application/json")
                    .body(body)
                    .send()
                    .await;
                match sent {
                    Ok(answer) if answer.status().is_success() => missed = false,
                    Ok(answer) if !missed => {
                        eprintln!("simrunner: the heartbeat was answered {}; beating on", answer.status());
                        missed = true;
                    }
                    Err(fault) if !missed => {
                        eprintln!("simrunner: the heartbeat did not land: {}; beating on", chain(&fault));
                        missed = true;
                    }
                    _ => {}
                }
                sleep(EVERY).await;
            }
        }))
    }
}

impl Drop for Beating {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// What one beat says: the simtime the run's final slices reach, and the picks and completed orders they state.
pub fn beat_of(counters: &Counters) -> Heartbeat {
    let value = |metric: &str, label: &str, count: u64| ProgressValue {
        metric: metric.to_owned(),
        label: label.to_owned(),
        value: Some(count as f64),
    };
    Heartbeat {
        sim_ms: counters.shown_sim_time(),
        values: vec![
            value("picks", "Picks", counters.shown_picks()),
            value("orders_completed", "Orders completed", counters.shown_orders_completed()),
        ],
    }
}
