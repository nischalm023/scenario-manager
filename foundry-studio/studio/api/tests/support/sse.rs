//! A server-sent events reader for the stream tests: a response body read frame by frame and cut into events as an
//! `EventSource` cuts them (WHATWG HTML 9.2.6): fields up to a blank line, a line starting `:` ignored.

use std::str;
use std::time::Duration;

use axum::body::Body;
use http_body_util::BodyExt;
use serde_json::Value;
use tokio::time::timeout;

/// One event as the stream sent it: `id` only when it carried an `id:` line.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SseEvent {
    pub id: Option<String>,
    pub event: Option<String>,
    pub data: String,
    pub retry: Option<String>,
}

impl SseEvent {
    pub fn kind(&self) -> &str {
        self.event.as_deref().unwrap_or("message")
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.data).unwrap_or_else(|fault| panic!("{:?}'s data is JSON: {fault}", self.kind()))
    }
}

pub struct EventReader {
    body: Body,
    text: String,
}

impl EventReader {
    pub fn new(body: Body) -> Self {
        Self { body, text: String::new() }
    }

    /// The next event within `within`; `None` once the stream has ended. A wait past `within` fails the test.
    pub async fn next(&mut self, within: Duration) -> Option<SseEvent> {
        loop {
            if let Some(end) = self.text.find("\n\n") {
                let block: String = self.text.drain(..end + 2).collect();
                return Some(parse(&block));
            }
            let frame = timeout(within, self.body.frame()).await.expect("an event arrives in time");
            match frame {
                None => return None,
                Some(frame) => {
                    let frame = frame.expect("the body reads");
                    if let Ok(data) = frame.into_data() {
                        self.text.push_str(str::from_utf8(&data).expect("the stream is UTF-8"));
                    }
                }
            }
        }
    }

    /// Events up to and including the first of `kind`.
    pub async fn until(&mut self, kind: &str, within: Duration) -> Vec<SseEvent> {
        let mut seen = Vec::new();
        loop {
            let event = self.next(within).await.unwrap_or_else(|| panic!("the stream ended before {kind}: {seen:?}"));
            let found = event.kind() == kind;
            seen.push(event);
            if found {
                return seen;
            }
        }
    }
}

fn parse(block: &str) -> SseEvent {
    let mut event = SseEvent::default();
    for line in block.lines().filter(|line| !line.is_empty() && !line.starts_with(':')) {
        let (field, value) = line.split_once(':').map_or((line, ""), |(field, value)| (field, value));
        let value = value.strip_prefix(' ').unwrap_or(value).to_owned();
        match field {
            "id" => event.id = Some(value),
            "event" => event.event = Some(value),
            "retry" => event.retry = Some(value),
            "data" if event.data.is_empty() => event.data = value,
            "data" => event.data = format!("{}\n{value}", event.data),
            _ => {}
        }
    }
    event
}
