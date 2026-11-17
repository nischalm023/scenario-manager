//! Within the agent loop, the `backend` is the model behind one interface (spec 01 section 3.7, "the API-backed
//! driver", amended by ruling D122): one request (the frozen system prompt, the conversation's rig messages, the tool
//! definitions and the output ceiling) goes out, and its reply comes back as a stream of events in the order the
//! provider sent them: a text block's fragments, each block once it ended, and finally the whole assistant message
//! with its stop, usage and the provider's document. A failure is the stream's last item, classified so the loop can
//! tell a retry from an end. Every provider's implementation, the scripted fake's included, yields the same events,
//! so the loop journals the same entries whichever answered.

use std::fmt::{self, Display, Formatter};
use std::pin::Pin;
use std::time::Duration;

use futures::Stream;
use rig_core::completion::{Message, ToolDefinition};
use rig_core::message::ToolCall;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The model providers this build can name (ruling D122): Gemini through its API key, Anthropic's Messages API through
/// its key, Vertex (task 52, not built), and the scripted fake of the tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Gemini,
    Anthropic,
    Vertex,
    Scripted,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::Anthropic => "anthropic",
            Self::Vertex => "vertex",
            Self::Scripted => "scripted",
        }
    }
}

impl Display for Provider {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str(self.as_str())
    }
}

/// A number of tokens, as a provider counts them; a plain number only at rig's request and reply and at the database's
/// columns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tokens(pub u64);

#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub system: String,
    /// The conversation as rig holds it, ending with the user message the model answers.
    pub history: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    /// Tools stay declared but none may be called: the budget's and the compaction's summary requests.
    pub tools_off: bool,
    pub max_tokens: Tokens,
}

/// A block of the reply, as it ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Text(String),
    /// The model's reasoning as the provider shows it (Gemini's thought summary): kept in the assistant message, never
    /// journaled, since the chat never shows it (D117).
    Reasoning(String),
    Call(ToolCall),
    /// A provider item with no meaning here (an image, an opaque part): kept in the message, never journaled.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// The model finished without calling a tool.
    EndTurn,
    ToolUse,
    /// The output ceiling cut the reply.
    MaxTokens,
    /// The provider refused the content, with its category when it names one.
    Refused(Option<String>),
    /// A reason this build does not model, in the provider's words.
    Other(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUsage {
    pub input_tokens: Tokens,
    pub output_tokens: Tokens,
    pub cache_read_input_tokens: Tokens,
}

/// The reply, whole.
#[derive(Debug, Clone)]
pub struct StepEnd {
    /// The assistant message to append to the conversation, provider items (thought signatures) included; `None` when
    /// the reply held no block.
    pub message: Option<Message>,
    pub calls: Vec<ToolCall>,
    pub stop: Stop,
    pub usage: ModelUsage,
    /// The provider's own document for the reply, which `model_calls.raw` keeps.
    pub raw: Value,
    pub request_id: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub enum StepEvent {
    /// Text block `part` grew by `text`.
    Delta {
        part: u32,
        text: String,
    },
    Done {
        part: u32,
        block: Block,
    },
    Ended(Box<StepEnd>),
}

/// How a failed request ends: `Unavailable` is worth asking again (a 429, a 5xx, a lost connection); the rest end the
/// turn as section 3.7 names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    Unavailable,
    /// The provider refused the request as written (a 400): a fault of ours.
    Rejected,
    /// The provider refused the content.
    Refused,
    /// Anything section 3.7 does not name, a refused credential among them.
    Failed,
}

/// A failed model request: its class, the HTTP status when there was a reply, the provider's own words (redacted,
/// never a URL or a key), and the wait the provider asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError {
    pub class: ErrorClass,
    pub status: Option<u16>,
    pub words: String,
    pub retry_after: Option<Duration>,
}

impl Display for ModelError {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        match self.status {
            Some(status) => write!(out, "the model answered {status}: {}", self.words),
            None => write!(out, "the model could not be reached: {}", self.words),
        }
    }
}

pub type StepStream = Pin<Box<dyn Stream<Item = Result<StepEvent, ModelError>> + Send>>;

pub trait ModelBackend: Send + Sync {
    fn provider(&self) -> Provider;
    /// The model's id as the provider names it.
    fn model(&self) -> &str;
    /// Send `request`; nothing goes out until the stream is first polled, and dropping it cancels the request.
    fn step(&self, request: ModelRequest) -> StepStream;
}
