//! Within `results`, `streams` is the metric streams' catalogue: `GET /api/metric-streams`, every metric the
//! streaming server computes from a recording, each as it is served unless an attach asks for another bucketing, so
//! the run page offers the charts before any socket is open. The list is `studio_metrics`' own, named once there.

use axum::Json;
use studio_metrics::catalogue as computed;
use studio_protocol::MetricCatalogue;

/// `GET /api/metric-streams`.
pub async fn catalogue() -> Json<MetricCatalogue> {
    Json(MetricCatalogue { metrics: computed() })
}
