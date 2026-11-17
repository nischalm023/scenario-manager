//! Within `studio-protocol`, `grammar` is spec 03 section 3.1, the reference grammar: the compact strings that name a
//! scope at a version (a location), a Postgres object, a world entity, and the key a page element carries. Each form
//! is a type of its own with one text: `FromStr` refuses anything off the grammar, `Display` writes the canonical
//! text (rule 7: lower-case UUIDs, no leading zeros, no percent-encoding), serde reads and writes that same text,
//! and the TypeScript name of each is `string`. `tests/grammar-vectors.json` holds the grammar's golden vectors.

mod entity;
mod object;
mod world;

use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use thiserror::Error;
use ts_rs::TS;

use crate::problem::ProblemType;

pub use entity::{DeclaredId, EntityId, EntityRef, Kinship, PinRuleBreach, PinRuleError};
pub use object::{FieldKey, ObjectKey, ObjectKind, Ulid};
pub use world::{BranchName, KinPin, KinPins, Location, Pin, RefPin, ScopePath, Seq, SitePath, TagName};

/// A string off the grammar: the form it was read as, the string, and why it is not that form. As a problem it is
/// `ref-malformed` (spec 03 section 3.8), and its message names the string.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("`{input}` is not {form}: {why}")]
pub struct GrammarError {
    form: &'static str,
    input: String,
    why: String,
}

impl GrammarError {
    pub(crate) fn new(form: &'static str, input: &str, why: impl Into<String>) -> Self {
        Self { form, input: input.to_owned(), why: why.into() }
    }

    /// The same reason, told of the larger string `input` read as `form`, when a part of it is what failed.
    pub(crate) fn reframe(self, form: &'static str, input: &str) -> Self {
        Self { form, input: input.to_owned(), why: self.why }
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn problem_type(&self) -> ProblemType {
        ProblemType::RefMalformed
    }
}

/// Serde for a grammar type: its canonical text out, and in through `FromStr`, so JSON never holds a form the
/// grammar refuses.
macro_rules! string_form {
    ($($form:ty),+ $(,)?) => {$(
        impl serde::Serialize for $form {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> serde::Deserialize<'de> for $form {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = <String as serde::Deserialize>::deserialize(deserializer)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    )+};
}
pub(crate) use string_form;

/// What a page element's `data-ref` holds and a highlight matches: an entity's identity, `scope:` and a scope path,
/// an object key, or one field of an object. A key never carries a pin (rule 6: matching is by key, never by pin).
#[derive(Debug, Clone, PartialEq, Eq, Hash, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub enum Key {
    Entity(EntityId),
    Scope(ScopePath),
    Object(ObjectKey),
    Field(FieldKey),
}

/// What a command, an address or a stored input names: an entity reference, `scope:` and a location, or an object.
#[derive(Debug, Clone, PartialEq, Eq, Hash, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub enum Ref {
    Entity(EntityRef),
    Scope(Location),
    Object(ObjectKey),
}

/// The prefix that marks a scope among keys and references.
const SCOPE_PREFIX: &str = "scope:";

impl Ref {
    /// The key a page element carries for what this reference names: its pin dropped (rule 6).
    pub fn key(&self) -> Key {
        match self {
            Self::Entity(reference) => Key::Entity(reference.id().clone()),
            Self::Scope(location) => Key::Scope(location.scope().clone()),
            Self::Object(object) => Key::Object(object.clone()),
        }
    }
}

impl FromStr for Key {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let reframe = |fault: GrammarError| fault.reframe("a key", text);
        if let Some(path) = text.strip_prefix(SCOPE_PREFIX) {
            if path.contains('@') {
                return Err(GrammarError::new("a key", text, "a key never carries a pin"));
            }
            return path.parse().map(Self::Scope).map_err(reframe);
        }
        if entity::opens_with_uuid(text) {
            return text.parse().map(Self::Entity).map_err(reframe);
        }
        if text.contains('#') {
            return text.parse().map(Self::Field).map_err(reframe);
        }
        text.parse().map(Self::Object).map_err(reframe)
    }
}

impl FromStr for Ref {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let reframe = |fault: GrammarError| fault.reframe("a reference", text);
        if let Some(location) = text.strip_prefix(SCOPE_PREFIX) {
            return location.parse().map(Self::Scope).map_err(reframe);
        }
        if entity::opens_with_uuid(text) {
            return text.parse().map(Self::Entity).map_err(reframe);
        }
        if text.contains('#') {
            return Err(GrammarError::new("a reference", text, "a field key is a key only, never a reference"));
        }
        text.parse().map(Self::Object).map_err(reframe)
    }
}

impl Display for Key {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entity(id) => write!(out, "{id}"),
            Self::Scope(path) => write!(out, "{SCOPE_PREFIX}{path}"),
            Self::Object(object) => write!(out, "{object}"),
            Self::Field(field) => write!(out, "{field}"),
        }
    }
}

impl Display for Ref {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entity(reference) => write!(out, "{reference}"),
            Self::Scope(location) => write!(out, "{SCOPE_PREFIX}{location}"),
            Self::Object(object) => write!(out, "{object}"),
        }
    }
}

string_form!(Key, Ref);
