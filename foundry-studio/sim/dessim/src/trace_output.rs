//! Filtered trace-record output for the `trace` feature.
//!
//! This layer consumes only records emitted by `crate::trace_event!`. The
//! pathfinder's search-decision trace is a separate hook-based system under
//! `pathfinding::pathfinder_trace` and is intentionally not routed through
//! this output.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;

use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::DefaultGuard;
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use crate::config::{TraceConfig, TraceFormat, TraceWindow};

pub(crate) fn install_subscriber(config: &TraceConfig) -> Result<Option<DefaultGuard>, String> {
    #[cfg(feature = "perf")]
    {
        use tracing_subscriber::prelude::*;

        use crate::tracing_agg::AggregationLayer;

        if config.enabled {
            let subscriber =
                tracing_subscriber::registry().with(AggregationLayer::new()).with(TraceLayer::new(config)?);
            Ok(Some(tracing::subscriber::set_default(subscriber)))
        } else {
            let subscriber = tracing_subscriber::registry().with(AggregationLayer::new());
            Ok(Some(tracing::subscriber::set_default(subscriber)))
        }
    }

    #[cfg(not(feature = "perf"))]
    {
        use tracing_subscriber::prelude::*;

        if config.enabled {
            let subscriber = tracing_subscriber::registry().with(TraceLayer::new(config)?);
            Ok(Some(tracing::subscriber::set_default(subscriber)))
        } else {
            Ok(None)
        }
    }
}

struct TraceLayer {
    /// Filter clauses; each clause is the AND of its tokens, and an event
    /// passes if any clause matches (OR across clauses). Empty = accept all.
    clauses: Vec<Vec<FilterToken>>,
    window: Option<TraceWindow>,
    format: TraceFormat,
    writer: Mutex<Box<dyn Write + Send>>,
}

impl TraceLayer {
    fn new(config: &TraceConfig) -> Result<Self, String> {
        let clauses: Vec<Vec<FilterToken>> = config.filters.iter().map(|s| parse_filter(s)).collect();
        Ok(Self {
            clauses,
            window: config.window.clone(),
            format: config.format,
            writer: Mutex::new(open_writer(config.out.as_deref())?),
        })
    }

    fn accepts(&self, target: &str, fields: &BTreeMap<String, String>) -> bool {
        self.matches_filters(target, fields) && self.matches_window(fields)
    }

    fn matches_filters(&self, target: &str, fields: &BTreeMap<String, String>) -> bool {
        if self.clauses.is_empty() {
            return true;
        }
        self.clauses.iter().any(|clause| Self::clause_matches(clause, target, fields))
    }

    fn clause_matches(clause: &[FilterToken], target: &str, fields: &BTreeMap<String, String>) -> bool {
        clause.iter().all(|token| match token {
            FilterToken::Any(needle) => {
                contains_case_insensitive(target, needle)
                    || fields.keys().any(|key| contains_case_insensitive(key, needle))
                    || fields.values().any(|value| contains_case_insensitive(value, needle))
            }
            FilterToken::Field { key, value } => {
                fields.get(key).map(|field_value| field_value_matches(field_value, value)).unwrap_or(false)
            }
        })
    }

    fn matches_window(&self, fields: &BTreeMap<String, String>) -> bool {
        let Some(window) = &self.window else {
            return true;
        };
        // Only accept events whose fields carry at least one parseable
        // timestamp that falls inside the window. Events without any time
        // field cannot be placed on the timeline; drop them when a window
        // is specified so unrelated noise from outside the window doesn't
        // leak through.
        fields.values().filter_map(|value| parse_trace_time_value(value)).any(|ms| window.contains(ms))
    }

    fn write_record(&self, target: &str, fields: &BTreeMap<String, String>) {
        let result = match self.format {
            TraceFormat::Text => self.write_text(fields),
            TraceFormat::Jsonl => self.write_jsonl(target, fields),
        };
        if let Err(err) = result {
            eprintln!("[trace] failed to write trace record: {err}");
        }
    }

    fn write_text(&self, fields: &BTreeMap<String, String>) -> io::Result<()> {
        let kind = fields.get("trace_kind").map(String::as_str).unwrap_or("record");
        let name = fields.get("trace_name").map(String::as_str).unwrap_or("unknown");
        let mut writer = self.writer.lock().unwrap();
        write!(writer, "[trace {kind}] {name}")?;
        for (key, value) in fields {
            if key == "trace_kind" || key == "trace_name" {
                continue;
            }
            // `span` is auto-attached to events from their containing #[trace] span;
            // suppress it when it just repeats trace_name (true for #[trace] return
            // events and any in-function trace_event! whose name matches the span).
            if key == "span" && value == name {
                continue;
            }
            write!(writer, " {key}={value}")?;
        }
        writeln!(writer)?;
        writer.flush()
    }

    fn write_jsonl(&self, target: &str, fields: &BTreeMap<String, String>) -> io::Result<()> {
        let mut record = Map::new();
        record.insert("target".to_string(), Value::String(target.to_string()));
        for (key, value) in fields {
            record.insert(key.clone(), Value::String(value.clone()));
        }
        let mut writer = self.writer.lock().unwrap();
        serde_json::to_writer(&mut *writer, &Value::Object(record))?;
        writeln!(writer)?;
        writer.flush()
    }
}

impl<S> Layer<S> for TraceLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut visitor = TraceVisitor::default();
        attrs.record(&mut visitor);
        span.extensions_mut().insert(TraceSpanFields(visitor.fields));
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut visitor = TraceVisitor::default();
        values.record(&mut visitor);
        let mut extensions = span.extensions_mut();
        let fields = extensions.get_mut::<TraceSpanFields>();
        match fields {
            Some(fields) => fields.0.extend(visitor.fields),
            None => {
                extensions.insert(TraceSpanFields(visitor.fields));
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let target = event.metadata().target();
        if target != crate::TRACE_RECORD_TARGET {
            return;
        }

        let mut visitor = TraceVisitor::default();
        event.record(&mut visitor);
        if let Some(span) = ctx.lookup_current() {
            visitor.fields.entry("span".to_string()).or_insert_with(|| span.name().to_string());
            if let Some(span_fields) = span.extensions().get::<TraceSpanFields>() {
                for (key, value) in &span_fields.0 {
                    visitor.fields.entry(key.clone()).or_insert_with(|| value.clone());
                }
            }
        }

        if self.accepts(target, &visitor.fields) {
            self.write_record(target, &visitor.fields);
        }
    }
}

struct TraceSpanFields(BTreeMap<String, String>);

#[derive(Default)]
struct TraceVisitor {
    fields: BTreeMap<String, String>,
}

impl TraceVisitor {
    fn record(&mut self, field: &Field, value: String) {
        self.fields.insert(field.name().to_string(), clean_debug_string(value));
    }
}

impl Visit for TraceVisitor {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record(field, value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record(field, value.to_string());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.record(field, format!("{value:?}"));
    }
}

enum FilterToken {
    Any(String),
    Field { key: String, value: String },
}

fn parse_filter(clause: &str) -> Vec<FilterToken> {
    clause
        .split(',')
        .filter_map(|token| {
            let token = token.trim();
            if token.is_empty() {
                return None;
            }
            if let Some((key, value)) = token.split_once('=') {
                return Some(FilterToken::Field { key: key.trim().to_string(), value: value.trim().to_string() });
            }
            Some(FilterToken::Any(token.to_string()))
        })
        .collect()
}

fn open_writer(out: Option<&Path>) -> Result<Box<dyn Write + Send>, String> {
    let Some(path) = out else {
        return Ok(Box::new(io::stderr()));
    };
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create trace output directory {}: {err}", parent.display()))?;
        }
    }
    let file = File::create(path).map_err(|err| format!("failed to create trace output {}: {err}", path.display()))?;
    Ok(Box::new(file))
}

fn clean_debug_string(value: String) -> String {
    value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(&value).to_string()
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())
}

fn field_value_matches(field_value: &str, expected: &str) -> bool {
    field_value == expected || trailing_digits(field_value).map(|digits| digits == expected).unwrap_or(false)
}

fn trailing_digits(value: &str) -> Option<&str> {
    let start = value
        .char_indices()
        .rev()
        .find_map(|(idx, ch)| (!ch.is_ascii_digit()).then_some(idx + ch.len_utf8()))
        .unwrap_or(0);
    (start < value.len()).then_some(&value[start..])
}

fn parse_trace_time_value(value: &str) -> Option<u64> {
    let value = value.trim().trim_matches('"');
    if let Some(seconds) = value.strip_suffix('s') {
        return seconds.trim().parse::<f64>().ok().map(|seconds| (seconds * 1000.0).round() as u64);
    }
    if let Some(ms) = value.strip_prefix("ms(").and_then(|v| v.strip_suffix(')')) {
        return ms.replace('_', "").parse::<u64>().ok();
    }
    if let Some(time) = value.strip_prefix("t(").and_then(|v| v.strip_suffix(')')) {
        return time.replace('_', "").parse::<u64>().ok();
    }
    None
}
