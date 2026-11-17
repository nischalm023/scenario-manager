//! Within `studio-api`, `results` are what a run's attempt left, as spec 08 sections 3.7 to 3.12 read them (brief
//! I1.10): its declared metrics over a window of simtime and one metric by simulated period (`metrics`, over the
//! arithmetic of `values` and the rows `stored` reads), the headline rule and the run list's headline column
//! (`headline`), a project's pinned metrics (`pins`), the run's inputs as values (`inputs`), a comparison of runs
//! (`compare`), the stored recording as a download (`recording`), the metric streams' catalogue (`streams`), and how a
//! quantity is said in a sentence (`words`).
//! The Runs panel and the agent's run tools read the same routes, so a figure the agent quotes is one the panel shows;
//! no metric's name is written here (statement 9).

mod compare;
mod headline;
mod inputs;
pub(crate) mod metrics;
mod pins;
mod recording;
mod stored;
mod streams;
mod values;
mod words;

use axum::Router;
use axum::routing::{get, put};
use wps_client::wps::Wps;

use crate::{AppState, GateState};

pub use compare::{Design, compare_inputs, compare_metrics};
pub use headline::{choose as headlines, fill as fill_headlines};
pub use recording::download_name;
pub use values::{Declared, Instants, Stored, Unrecorded, periods, row, running_at, shaped, window_difference};

/// The result routes (spec 08 section 3.7) this build serves.
pub fn routes<S: Wps>(_app: &AppState) -> Router<GateState<S>> {
    Router::new()
        .route("/api/runs/{run}/metrics", get(metrics::metrics))
        .route("/api/runs/{run}/series", get(metrics::series))
        .route("/api/runs/{run}/inputs", get(inputs::inputs))
        .route("/api/runs/{run}/recording", get(recording::recording::<S>))
        .route("/api/runs/compare", get(compare::compare))
        .route("/api/metric-streams", get(streams::catalogue))
        .route("/api/projects/{project}/metric-pins", put(pins::put))
}
