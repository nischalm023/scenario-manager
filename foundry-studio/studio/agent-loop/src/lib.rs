//! Agent Studio's agent loop (spec 01 sections 3.7 and 3.8; ruling D122), the parts that know no database: the model
//! behind one interface (`backend`), on rig at the completion level for Gemini and Anthropic (`rig_backend`,
//! `gemini`, `anthropic`), a scripted fake the API's tests drive (`scripted`), the retry schedule (`retry`), the tool
//! registry every call runs through (`registry`) and the frozen system prompt (`prompt`). The turn itself, its
//! journal, questions and approvals are the API's (`studio-api`'s `agent` module), which holds them in Postgres.

pub mod anthropic;
pub mod backend;
pub mod gemini;
pub mod prompt;
pub mod registry;
pub mod retry;
pub mod rig_backend;
pub mod scripted;

pub use backend::{
    Block, ErrorClass, ModelBackend, ModelError, ModelRequest, ModelUsage, Provider, StepEnd, StepEvent, StepStream,
    Stop, Tokens,
};
pub use registry::{Card, Class, Registry, RegistryError, Tool, ToolFuture, wire_name};
pub use rig_backend::RigBackend;
/// The process-wide reqwest client every provider sends through.
pub use rig_reqwest::shared as shared_http;
pub use scripted::{Scripted, ScriptedBlock, ScriptedStep};

/// rig's history and tool types, re-exported so the API speaks the same ones without naming rig itself.
pub mod rig {
    pub use rig_core::completion::{AssistantContent, Message, ToolDefinition};
    pub use rig_core::message::{
        AssistantMessage, CallId, Text, ToolCall, ToolFunction, ToolName, ToolResult, ToolResultContent, UserContent,
    };
    pub use rig_core::redact;
}
