//! Within the agent loop, the `registry` is the catalogue of tools the agent may call (spec 01 section 3.8): each a
//! typed value with its catalogue name, approval class, description, full input schema and its own words for a call,
//! registered once. The model sees each by its wire name (the catalogue name with `.` turned into `_`) through rig's
//! `ToolDefinition`; every call it makes is looked up by that name and its input checked against the full schema
//! before anything executes. The registry knows no database: a tool's context is its host's (`Cx`, the API's).

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use jsonschema::Validator;
use rig_core::completion::ToolDefinition;
use rig_core::message::ToolName;
use rustc_hash::FxHashMap;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use studio_protocol::{ApprovalFacts, Problem, ProblemType};
use thiserror::Error;

/// The most a result may hold, as compact JSON (spec 01 section 3.8, "Results").
pub const RESULT_LIMIT: usize = 64 * 1024;

/// A tool's approval class (ruling D61): `read`, `draft`, `page` and `cancel` never ask; the other four ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    Read,
    Draft,
    Page,
    Apply,
    Submit,
    Destroy,
    Publish,
    Cancel,
}

impl Class {
    pub fn asks(self) -> bool {
        matches!(self, Self::Apply | Self::Submit | Self::Destroy | Self::Publish)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Draft => "draft",
            Self::Page => "page",
            Self::Apply => "apply",
            Self::Submit => "submit",
            Self::Destroy => "destroy",
            Self::Publish => "publish",
            Self::Cancel => "cancel",
        }
    }
}

/// What an asking tool's card step leaves for a person to decide: the card's summary, the input that alone executes
/// once approved (with whatever it pinned), and the facts the card shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub summary: String,
    pub stored_input: Value,
    pub facts: ApprovalFacts,
}

pub type ToolFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, Problem>> + Send + 'a>>;

pub trait Tool<Cx>: Send + Sync {
    fn name(&self) -> &'static str;
    fn class(&self) -> Class;
    /// What the model reads: what the tool does, when to use it and what it returns.
    fn description(&self) -> &'static str;
    /// The full JSON Schema (2020-12) of the input, the authority every call is checked against.
    fn input_schema(&self) -> Value;
    /// `tool.started`'s summary, in the tool's own words.
    fn describe(&self, input: &Value) -> String;
    /// `tool.finished`'s summary of a call that ended `ok`.
    fn summarize(&self, input: &Value, result: &Value) -> String;
    fn card<'a>(&'a self, _cx: &'a Cx, _input: &'a Value) -> ToolFuture<'a, Card> {
        let problem = Problem::new(ProblemType::Internal, format!("{} asks for no approval.", self.name()));
        Box::pin(async move { Err(problem) })
    }
    fn execute<'a>(&'a self, cx: &'a Cx, input: Value) -> ToolFuture<'a, Value>;
    /// Wall-clock time a call may take, when the tool's own differs from the registry's default.
    fn timeout(&self) -> Option<Duration> {
        None
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("the tool name `{0}` gives the wire name `{1}`, which a model does not take")]
    WireName(&'static str, String),
    #[error("the tools `{0}` and `{1}` share the wire name `{2}`")]
    Collision(&'static str, &'static str, String),
    #[error("the input schema of `{0}` is no JSON Schema: {1}")]
    Schema(&'static str, String),
}

struct Entry<Cx> {
    tool: Box<dyn Tool<Cx>>,
    wire: String,
    schema: Value,
    validator: Validator,
}

pub struct Registry<Cx> {
    entries: Vec<Entry<Cx>>,
    by_name: FxHashMap<&'static str, usize>,
    by_wire: FxHashMap<String, usize>,
    digest: String,
}

/// A catalogue name as the model sees it.
pub fn wire_name(name: &str) -> String {
    name.replace('.', "_")
}

fn legal_wire(wire: &str) -> bool {
    (1..=128).contains(&wire.len()) && wire.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}

impl<Cx> Registry<Cx> {
    /// The catalogue of `tools`, in their order; refused when a wire name is illegal or shared, or a schema compiles to
    /// no validator.
    pub fn new(tools: Vec<Box<dyn Tool<Cx>>>) -> Result<Self, RegistryError> {
        let (mut entries, mut by_name, mut by_wire) = (Vec::new(), FxHashMap::default(), FxHashMap::default());
        for tool in tools {
            let (name, wire, schema) = (tool.name(), wire_name(tool.name()), tool.input_schema());
            if !legal_wire(&wire) {
                return Err(RegistryError::WireName(name, wire));
            }
            if let Some(&held) = by_wire.get(&wire) {
                let other: &Entry<Cx> = &entries[held];
                return Err(RegistryError::Collision(other.tool.name(), name, wire));
            }
            let validator = jsonschema::draft202012::new(&schema)
                .map_err(|fault| RegistryError::Schema(name, fault.to_string()))?;
            by_name.insert(name, entries.len());
            by_wire.insert(wire.clone(), entries.len());
            entries.push(Entry { tool, wire, schema, validator });
        }
        let listed: Vec<Value> =
            entries.iter().map(|entry| json!([entry.wire, entry.tool.description(), entry.schema])).collect();
        let digest = format!("{:x}", Sha256::digest(Value::Array(listed).to_string().as_bytes()));
        Ok(Self { entries, by_name, by_wire, digest })
    }

    /// Every tool as rig sends it to the model, in catalogue order.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        (self.entries.iter())
            .map(|entry| {
                let name = ToolName::new(entry.wire.clone()).expect("a legal wire name is never empty");
                ToolDefinition::new(name, entry.tool.description(), entry.schema.clone())
            })
            .collect()
    }

    pub fn tool(&self, name: &str) -> Option<&dyn Tool<Cx>> {
        self.by_name.get(name).map(|n| self.entries[*n].tool.as_ref())
    }

    pub fn by_wire(&self, wire: &str) -> Option<&dyn Tool<Cx>> {
        self.by_wire.get(wire).map(|n| self.entries[*n].tool.as_ref())
    }

    /// Every (catalogue name, wire name) pair, in catalogue order.
    pub fn names(&self) -> impl Iterator<Item = (&'static str, &str)> {
        self.entries.iter().map(|entry| (entry.tool.name(), entry.wire.as_str()))
    }

    /// Check `input` against the full schema of the tool `name`: each failure names its field, as `validation-failed`
    /// lists them.
    pub fn check(&self, name: &str, input: &Value) -> Result<(), Vec<String>> {
        let Some(entry) = self.by_name.get(name).map(|n| &self.entries[*n]) else {
            return Err(vec![format!("there is no tool `{name}`")]);
        };
        let failures: Vec<String> = (entry.validator.iter_errors(input))
            .map(|error| match error.instance_path.as_str() {
                "" => format!("the input: {error}"),
                path => format!("`{path}`: {error}"),
            })
            .collect();
        if failures.is_empty() { Ok(()) } else { Err(failures) }
    }

    /// A digest of every definition the model is sent, so a conversation can tell its catalogue changed.
    pub fn digest(&self) -> &str {
        &self.digest
    }
}
