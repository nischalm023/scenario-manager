//! Within `listeners`, `metrics` is a viewer's socket for one metric of an attempt on the external port
//! (`/metrics/{run}/{attempt}`), in the metric stream protocol (`studio/protocol/src/metric_stream.rs`): an `attach`
//! first, within the bound, naming the metric and carrying the same watch ticket the API minted for the attempt,
//! which `admission` checks as it checks a `watch`; a metric the server does not compute is refused
//! `unknown-metric`; then the metric's rows, as `metric_streams` serves them.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use studio_metrics::{Ask, find};
use studio_protocol::{AttemptNumber, MetricRequest, StreamRefusal};
use tokio::time::timeout;

use super::{Listening, admission, refuse};
use crate::metric_streams::serve_metric;
use crate::streams::StreamKey;

/// The largest message a viewer may send: its attach, a ticket and a few words.
const MESSAGE_LIMIT: usize = 4 << 10;

/// `GET /metrics/{run}/{attempt}`: the upgrade, or 404 for a path that names no attempt.
pub async fn upgrade(
    upgrade: WebSocketUpgrade,
    State(listening): State<Listening>,
    Path((run, attempt)): Path<(String, u32)>,
) -> Response {
    let Ok(run) = run.parse() else { return (StatusCode::NOT_FOUND, "no such run").into_response() };
    let key = StreamKey { run, attempt: AttemptNumber(attempt) };
    upgrade.max_message_size(MESSAGE_LIMIT).on_upgrade(move |socket| serve(socket, listening, key)).into_response()
}

async fn serve(socket: WebSocket, listening: Listening, key: StreamKey) {
    let within = listening.bounds.write_within;
    let (mut sink, mut source) = socket.split();
    let request = match timeout(listening.bounds.first_message, source.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match MetricRequest::read(&text) {
            Ok(request) => request,
            Err(refusal) => return refuse(&mut sink, refusal, within).await,
        },
        Ok(Some(Ok(Message::Binary(_)))) => return refuse(&mut sink, StreamRefusal::Binary, within).await,
        _ => return refuse(&mut sink, StreamRefusal::NoGreeting, within).await,
    };
    let MetricRequest::Attach { metric, token, trailing, .. } = request;
    if let Err(refusal) = admission::viewer(&listening.pool, &key, &token).await {
        return refuse(&mut sink, refusal, within).await;
    }
    let Some(metric) = find(&metric) else { return refuse(&mut sink, StreamRefusal::UnknownMetric, within).await };
    serve_metric(sink, source, key, Ask { metric, trailing }, listening.attach()).await;
}
