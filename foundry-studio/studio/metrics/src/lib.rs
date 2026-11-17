//! `studio-metrics` is what the streaming server computes from a run's WorldView recording and serves as metric
//! streams (`studio/protocol/src/metric_stream.rs`), kept apart from the server so the API can name the same catalogue
//! without linking the server. One `Follower` reads an attempt's recording once and computes every metric from it, by
//! family: `series`, the seventy-four time series desvis draws from the slices' scalars; `states`, the shares of time
//! in a state the robot state channel gives through the span engine; `reports`, the report tabs the journals give
//! through the builders desvis calls; and `summary`, what only the end block states. `metric` names every metric and
//! lists the catalogue; `ending` reads the end block's stop reason as the wire's outcome.

mod ending;
mod follower;
mod metric;
mod reports;
mod series;
mod states;
mod summary;

pub use ending::run_outcome;
pub use follower::{Ask, Follower, Made, Opened, Unreadable, Unrecorded, Unservable};
pub use metric::{Metric, catalogue, find};
pub use reports::ReportSeries;
pub use series::{TimeSeries, recorded_clock};
pub use states::StateSeries;
pub use summary::{Figure, SummarySeries, figures};
