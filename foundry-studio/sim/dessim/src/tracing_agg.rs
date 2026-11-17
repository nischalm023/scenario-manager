//! Aggregation layer for the `perf` feature.
//!
//! Collects per-span timing and event counts during simulation, then prints
//! a summary to stderr when the layer is dropped (i.e., when the tracing
//! subscriber guard goes out of scope at sim end).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use tracing::field::{Field, Visit};
use tracing::span;
use tracing::span::{Attributes, Id};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

/// Stored in span extensions to track wall-clock entry time.
struct SpanTiming {
    start: Instant,
    key: String,
}

/// Per-span aggregated statistics.
struct SpanStats {
    durations_us: Vec<f64>,
}

/// Aggregates span timings and event counts. Prints summary on drop.
pub(crate) struct AggregationLayer {
    stats: Mutex<AggState>,
}

struct AggState {
    spans: HashMap<String, SpanStats>,
    astar_cache_hits: u64,
    astar_cache_misses: u64,
}

impl AggregationLayer {
    pub(crate) fn new() -> Self {
        Self { stats: Mutex::new(AggState { spans: HashMap::new(), astar_cache_hits: 0, astar_cache_misses: 0 }) }
    }
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for AggregationLayer {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut visitor = SpanKeyVisitor::default();
            attrs.record(&mut visitor);
            let key = visitor.key(span.name());
            span.extensions_mut().insert(SpanTiming { start: Instant::now(), key });
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(&id) {
            if let Some(timing) = span.extensions().get::<SpanTiming>() {
                let duration_us = timing.start.elapsed().as_secs_f64() * 1_000_000.0;
                let mut state = self.stats.lock().unwrap();
                state
                    .spans
                    .entry(timing.key.clone())
                    .or_insert_with(|| SpanStats { durations_us: Vec::new() })
                    .durations_us
                    .push(duration_us);
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        struct CacheVisitor {
            hit: Option<bool>,
        }
        impl Visit for CacheVisitor {
            fn record_bool(&mut self, field: &Field, value: bool) {
                if field.name() == "cache_hit" {
                    self.hit = Some(value);
                }
            }
            fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
        }
        let mut v = CacheVisitor { hit: None };
        event.record(&mut v);
        if let Some(hit) = v.hit {
            let mut state = self.stats.lock().unwrap();
            if hit {
                state.astar_cache_hits += 1;
            } else {
                state.astar_cache_misses += 1;
            }
        }
    }
}

#[derive(Default)]
struct SpanKeyVisitor {
    task: Option<String>,
    demand: Option<String>,
}

impl SpanKeyVisitor {
    fn key(self, span_name: &str) -> String {
        if let Some(task) = self.task {
            format!("{span_name}[task={task}]")
        } else if let Some(demand) = self.demand {
            format!("{span_name}[demand={demand}]")
        } else {
            span_name.to_string()
        }
    }

    fn record_labeled(&mut self, field_name: &str, value: String) {
        match field_name {
            "task" => self.task = Some(value),
            "demand" => self.demand = Some(value),
            _ => {}
        }
    }
}

impl Visit for SpanKeyVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_labeled(field.name(), value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let raw = format!("{value:?}");
        self.record_labeled(field.name(), raw.trim_matches('"').to_string());
    }
}

impl Drop for AggregationLayer {
    fn drop(&mut self) {
        let state = self.stats.lock().unwrap();
        if state.spans.is_empty() && state.astar_cache_hits == 0 && state.astar_cache_misses == 0 {
            return;
        }

        eprintln!("\n=== dessim perf summary ===");

        // Sort span names for deterministic output.
        let mut names: Vec<&str> = state.spans.keys().map(String::as_str).collect();
        names.sort();

        for name in &names {
            let s = &state.spans[*name];
            let n = s.durations_us.len();
            if n == 0 {
                continue;
            }
            let total: f64 = s.durations_us.iter().sum();
            let avg = total / n as f64;
            let mut sorted = s.durations_us.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let p50 = sorted[n / 2];
            let p99_idx = ((n as f64 * 0.99) as usize).min(n - 1);
            let p99 = sorted[p99_idx];
            let max = sorted[n - 1];

            eprintln!(
                "  {:<45} {:>6} calls  avg {:>8.1}us  p50 {:>8.1}us  p99 {:>8.1}us  max {:>8.1}us  total {:>8.1}ms",
                name,
                n,
                avg,
                p50,
                p99,
                max,
                total / 1000.0,
            );
        }

        let total_cache = state.astar_cache_hits + state.astar_cache_misses;
        if total_cache > 0 {
            let hit_rate = state.astar_cache_hits as f64 / total_cache as f64 * 100.0;
            eprintln!(
                "\n  A* cache: {} hits, {} misses, {:.1}% hit rate",
                state.astar_cache_hits, state.astar_cache_misses, hit_rate,
            );
        }

        eprintln!("===========================\n");
    }
}
