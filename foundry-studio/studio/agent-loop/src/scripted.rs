//! Within the agent loop, `scripted` is the fake model the API's tests drive (brief I1.03 section 1): each request
//! takes the next scripted step, a reply of text, reasoning and tool calls, a failure, or a hang that answers nothing
//! until the stream is dropped, and yields the events a provider's reply would. Every request is kept, so a test can
//! read what the loop sent (the summary request's `tools_off`, the history after compaction).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::stream;
use rig_core::completion::{AssistantContent, Message};
use rig_core::message::{AssistantMessage, CallId, Reasoning, Text, ToolCall, ToolFunction, ToolName};
use serde_json::{Value, json};

use crate::backend::{
    Block, ErrorClass, ModelBackend, ModelError, ModelRequest, ModelUsage, Provider, StepEnd, StepEvent, StepStream,
    Stop, Tokens,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ScriptedBlock {
    Text(String),
    /// The model's reasoning, as Gemini's thought summary arrives.
    Reasoning(String),
    /// A call of the tool with this wire name.
    Call {
        tool: String,
        input: Value,
    },
}

#[derive(Debug, Clone)]
pub enum ScriptedStep {
    /// A reply of `blocks`; `stop` defaults to a tool use when it calls a tool, else the end of the turn.
    Reply {
        blocks: Vec<ScriptedBlock>,
        stop: Option<Stop>,
        usage: ModelUsage,
    },
    Fail(ModelError),
    Hang,
}

impl ScriptedStep {
    pub fn reply(blocks: Vec<ScriptedBlock>) -> Self {
        Self::Reply {
            blocks,
            stop: None,
            usage: ModelUsage { input_tokens: Tokens(100), output_tokens: Tokens(10), ..Default::default() },
        }
    }

    pub fn text(words: &str) -> Self {
        Self::reply(vec![ScriptedBlock::Text(words.to_owned())])
    }

    pub fn call(tool: &str, input: Value) -> Self {
        Self::reply(vec![ScriptedBlock::Call { tool: tool.to_owned(), input }])
    }

    /// The same reply, reporting `usage`.
    pub fn costing(self, input_tokens: Tokens, output_tokens: Tokens) -> Self {
        match self {
            Self::Reply { blocks, stop, .. } => Self::Reply {
                blocks,
                stop,
                usage: ModelUsage { input_tokens, output_tokens, cache_read_input_tokens: Tokens(0) },
            },
            other => other,
        }
    }
}

#[derive(Default)]
struct Script {
    steps: VecDeque<ScriptedStep>,
    forever: Option<ScriptedStep>,
    seen: Vec<ModelRequest>,
}

#[derive(Clone, Default)]
pub struct Scripted(Arc<Mutex<Script>>);

impl Scripted {
    pub fn new(steps: Vec<ScriptedStep>) -> Self {
        Self(Arc::new(Mutex::new(Script { steps: steps.into(), ..Script::default() })))
    }

    /// The step every request takes once the script ran out; without one, a request past the end fails.
    pub fn then_forever(self, step: ScriptedStep) -> Self {
        self.lock().forever = Some(step);
        self
    }

    /// Steps to take after those left.
    pub fn push(&self, steps: Vec<ScriptedStep>) {
        self.lock().steps.extend(steps);
    }

    /// Every request sent so far, in order.
    pub fn requests(&self) -> Vec<ModelRequest> {
        self.lock().seen.clone()
    }

    fn lock(&self) -> MutexGuard<'_, Script> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ModelBackend for Scripted {
    fn provider(&self) -> Provider {
        Provider::Scripted
    }

    fn model(&self) -> &str {
        "scripted"
    }

    fn step(&self, request: ModelRequest) -> StepStream {
        let step = {
            let mut script = self.lock();
            script.seen.push(request);
            script.steps.pop_front().or_else(|| script.forever.clone())
        };
        match step {
            Some(ScriptedStep::Reply { blocks, stop, usage }) => Box::pin(stream::iter(reply(blocks, stop, usage))),
            Some(ScriptedStep::Fail(error)) => Box::pin(stream::iter([Err(error)])),
            Some(ScriptedStep::Hang) => Box::pin(stream::pending()),
            None => {
                let words = "the script has no step left".to_owned();
                let error = ModelError { class: ErrorClass::Failed, status: None, words, retry_after: None };
                Box::pin(stream::iter([Err(error)]))
            }
        }
    }
}

fn reply(blocks: Vec<ScriptedBlock>, stop: Option<Stop>, usage: ModelUsage) -> Vec<Result<StepEvent, ModelError>> {
    let (mut events, mut content, mut calls) = (Vec::new(), Vec::new(), Vec::new());
    for (part, block) in blocks.into_iter().enumerate() {
        let part = part as u32;
        let (block, item) = match block {
            ScriptedBlock::Text(text) => {
                events.push(Ok(StepEvent::Delta { part, text: text.clone() }));
                (Block::Text(text.clone()), AssistantContent::Text(Text::new(text)))
            }
            ScriptedBlock::Reasoning(thought) => {
                (Block::Reasoning(thought.clone()), AssistantContent::Reasoning(Reasoning::new(thought)))
            }
            ScriptedBlock::Call { tool, input } => {
                let name = ToolName::new(tool).expect("a scripted call names its tool");
                let call = ToolCall::new(CallId::from_wire(""), ToolFunction::new(name, input));
                calls.push(call.clone());
                (Block::Call(call.clone()), AssistantContent::ToolCall(call))
            }
        };
        events.push(Ok(StepEvent::Done { part, block }));
        content.push(item);
    }
    let stop = stop.unwrap_or(if calls.is_empty() { Stop::EndTurn } else { Stop::ToolUse });
    let message = (!content.is_empty()).then(|| Message::Assistant(AssistantMessage::new(content)));
    let raw = json!({ "scripted": true });
    let end = StepEnd { message, calls, stop, usage, raw, request_id: None, model: Some("scripted".to_owned()) };
    events.push(Ok(StepEvent::Ended(Box::new(end))));
    events
}
