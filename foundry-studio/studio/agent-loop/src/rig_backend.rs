//! Within the agent loop, `rig_backend` is the model backend on rig at the completion level (ruling D122; brief I1.03
//! section 1): one `CompletionRequest` built from the loop's request, rig's `Model::stream` opened, and its events
//! relayed in order, a text part's fragments as they come and every part once it ended, then rig's response folded
//! into the reply's end, the provider's document in `raw`. A failure keeps its class, its status and the provider's
//! own words, redacted, never rig's message, which can name a URL. Every string that leaves here from the provider (a
//! failure's words, a stop's reason, each string of the document) has the keys it was given replaced, whatever the
//! provider echoes. Gemini and Anthropic share it; only their wires, made in `gemini` and `anthropic`, differ.

use std::mem;
use std::time::SystemTime;

use futures::StreamExt;
use futures::stream::{self};
use rig_core::DynModel;
use rig_core::completion::{AssistantContent, CompletionRequest, CompletionResponse, FinishReason, Message};
use rig_core::error::ProviderError;
use rig_core::message::ToolChoice;
use rig_core::operation::Completion;
use rig_core::redact;
use rig_core::streaming::{CompletionStream, Item, StreamEvent};
use serde_json::Value;

use crate::backend::{
    Block, ErrorClass, ModelBackend, ModelError, ModelRequest, ModelUsage, Provider, StepEnd, StepEvent, StepStream,
    Stop, Tokens,
};

/// How much of a provider's words a failure keeps.
const WORDS_KEPT: usize = 400;

pub struct RigBackend {
    provider: Provider,
    model_id: String,
    model: DynModel<Completion>,
    /// Provider parameters every request carries (Gemini's thought summaries).
    params: Option<Value>,
    /// Secrets no failure's words may hold, whatever a provider echoes.
    scrub: Vec<String>,
}

impl RigBackend {
    pub fn new(
        provider: Provider,
        model_id: &str,
        model: DynModel<Completion>,
        params: Option<Value>,
        scrub: Vec<String>,
    ) -> Self {
        Self { provider, model_id: model_id.to_owned(), model, params, scrub }
    }

    fn request(&self, request: ModelRequest) -> CompletionRequest {
        let mut built = CompletionRequest::from(request.history)
            .preamble(request.system)
            .tools(request.tools)
            .max_tokens(request.max_tokens.0);
        if request.tools_off {
            built = built.tool_choice(ToolChoice::None);
        }
        if let Some(params) = &self.params {
            built = built.additional_params(params.clone());
        }
        built
    }
}

enum Reading {
    Opening(Box<CompletionRequest>),
    Streaming(Box<CompletionStream>),
    Ended,
}

impl ModelBackend for RigBackend {
    fn provider(&self) -> Provider {
        self.provider
    }

    fn model(&self) -> &str {
        &self.model_id
    }

    fn step(&self, request: ModelRequest) -> StepStream {
        let (model, provider, scrub) = (self.model.clone(), self.provider, self.scrub.clone());
        let ending = scrub.clone();
        let failed = move |error: &ProviderError| model_error(error, provider, &scrub);
        let opening = Reading::Opening(Box::new(self.request(request)));
        Box::pin(stream::unfold(opening, move |reading| {
            let (model, failed, ending) = (model.clone(), failed.clone(), ending.clone());
            async move {
                let mut streamed = match reading {
                    Reading::Ended => return None,
                    Reading::Streaming(streamed) => streamed,
                    Reading::Opening(request) => match model.stream(*request) {
                        Ok(streamed) => Box::new(streamed),
                        Err(error) => return Some((Err(failed(&error)), Reading::Ended)),
                    },
                };
                loop {
                    let event = match streamed.next().await {
                        Some(Ok(Item::Event(event))) => event,
                        Some(Ok(Item::Unknown(_))) => continue,
                        Some(Err(error)) => return Some((Err(failed(&error)), Reading::Ended)),
                        None => {
                            let ended = match streamed.finish().await {
                                Ok(response) => Ok(StepEvent::Ended(Box::new(end_of(response, &ending)))),
                                Err(error) => Err(failed(&error)),
                            };
                            return Some((ended, Reading::Ended));
                        }
                    };
                    match event {
                        StreamEvent::Text { part, text } => {
                            let delta = StepEvent::Delta { part: part.index() as u32, text };
                            return Some((Ok(delta), Reading::Streaming(streamed)));
                        }
                        StreamEvent::End { part, content } => {
                            let done = StepEvent::Done { part: part.index() as u32, block: block_of(content) };
                            return Some((Ok(done), Reading::Streaming(streamed)));
                        }
                        StreamEvent::Start { .. } | StreamEvent::Reasoning { .. } | StreamEvent::Arguments { .. } => {}
                    }
                }
            }
        }))
    }
}

fn block_of(content: AssistantContent) -> Block {
    match content {
        AssistantContent::Text(text) => Block::Text(text.text),
        AssistantContent::Reasoning(reasoning) => Block::Reasoning(reasoning.text),
        AssistantContent::ToolCall(call) => Block::Call(call),
        _ => Block::Other,
    }
}

/// The reply, whole: its message to append, its calls, how it stopped, what it cost and the provider's document, every
/// string of the stop and the document with `scrub`'s keys replaced.
fn end_of(response: CompletionResponse, scrub: &[String]) -> StepEnd {
    let calls: Vec<_> = response.tool_calls().cloned().collect();
    let mut raw = response.raw.clone();
    scrub_document(&mut raw, scrub);
    let refused = |raw: &Value| {
        let reason = raw.pointer("/stop_details/category").or_else(|| raw.pointer("/candidates/0/finishReason"));
        reason.and_then(Value::as_str).map(str::to_owned)
    };
    let stop = match response.finish_reason() {
        Some(FinishReason::Length) => Stop::MaxTokens,
        Some(FinishReason::ContentFilter) => Stop::Refused(refused(&raw)),
        Some(FinishReason::Other(reason)) => Stop::Other(scrubbed(&reason, scrub)),
        _ if !calls.is_empty() => Stop::ToolUse,
        _ => match &response.error {
            Some(error) => Stop::Other(scrubbed(&redact::text(error), scrub)),
            None => Stop::EndTurn,
        },
    };
    let usage = ModelUsage {
        input_tokens: Tokens(response.usage.input_tokens.unwrap_or(0)),
        output_tokens: Tokens(response.usage.output_tokens.unwrap_or(0)),
        cache_read_input_tokens: Tokens(response.usage.cached_input_tokens.unwrap_or(0)),
    };
    let message: Option<Message> = response.message();
    StepEnd {
        message,
        calls,
        stop,
        usage,
        model: response.model().map(str::to_owned),
        request_id: response.provider_request_id.clone(),
        raw,
    }
}

/// `text` with each of `scrub`'s keys replaced, wherever the provider put it.
fn scrubbed(text: &str, scrub: &[String]) -> String {
    let mut text = text.to_owned();
    for secret in scrub.iter().filter(|secret| !secret.is_empty()) {
        text = text.replace(secret.as_str(), redact::REDACTED);
    }
    text
}

/// The provider's document with every string in it, keys of objects included, `scrubbed`.
fn scrub_document(document: &mut Value, scrub: &[String]) {
    match document {
        Value::String(text) => *text = scrubbed(text, scrub),
        Value::Array(items) => items.iter_mut().for_each(|item| scrub_document(item, scrub)),
        Value::Object(fields) => {
            let named: Vec<(String, Value)> = mem::take(fields).into_iter().collect();
            for (name, mut value) in named {
                scrub_document(&mut value, scrub);
                fields.insert(scrubbed(&name, scrub), value);
            }
        }
        _ => {}
    }
}

/// `error` as the loop reads it. A reply's words are the provider's own message (its JSON body's `error.message`,
/// else the body), redacted and cut; a failure with no reply says only that the connection failed, since rig's
/// transport error names the URL.
fn model_error(error: &ProviderError, provider: Provider, scrub: &[String]) -> ModelError {
    let report = error.report();
    let status = error.provider_response_status().map(|status| status.as_u16()).or(report.http_status);
    let class = match () {
        _ if error.is_retryable() => ErrorClass::Unavailable,
        _ if report.refusal => ErrorClass::Refused,
        _ if status == Some(400) => ErrorClass::Rejected,
        _ => ErrorClass::Failed,
    };
    let words = match error.provider_response_body() {
        Some(body) => serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|json| json.pointer("/error/message").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| body.to_owned()),
        None if matches!(error, ProviderError::Truncated) => "the reply ended before the provider ended it".into(),
        None => format!("the connection to {provider} failed before a reply"),
    };
    let mut words = scrubbed(&redact::text(&words), scrub);
    if words.chars().count() > WORDS_KEPT {
        words = words.chars().take(WORDS_KEPT).chain(['…']).collect();
    }
    ModelError { class, status, words, retry_after: report.retry_after(SystemTime::now()) }
}
