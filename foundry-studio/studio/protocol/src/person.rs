//! Within `studio-protocol`, `person` is who acts: the gate's stable subject for a person, and `GET /api/me`'s answer
//! (spec 01 sections 3.2 and 3.6).

use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::grammar::string_form;

/// A person's stable identity from the login gate: Keycloak's `sub`, the key of the `people` table. Never blank.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, TS)]
#[ts(type = "string", export_to = "person.ts")]
pub struct Subject(String);

impl Subject {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("a subject is never blank")]
pub struct BlankSubject;

impl FromStr for Subject {
    type Err = BlankSubject;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.trim().is_empty() { Err(BlankSubject) } else { Ok(Self(text.to_owned())) }
    }
}

impl Display for Subject {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

string_form!(Subject);

/// A person as an entry names them: the gate's subject, and their name as it showed when the entry was written (the
/// `author` of a message or an answer, the `by` of a dismissal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "person.ts")]
pub struct Person {
    pub subject: Subject,
    pub name: String,
}

/// `GET /api/me`'s answer: who the gate says is asking, whether they are a developer (spec 01 section 3.2), the
/// runner image they chose, `null` for the default (spec 01 section 3.16), and the agent's model as the server's
/// configuration names it (task 3; ruling D122), which spec 02's Settings shows a developer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "person.ts")]
pub struct Me {
    pub subject: Subject,
    pub email: String,
    pub name: String,
    pub developer: bool,
    /// A registered image's digest; null until a developer chooses one.
    pub runner_image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub agent: Option<AgentModel>,
}

/// Who answers the agent's requests (ruling D122): Gemini through its API key, Anthropic through its key, or Claude on
/// Vertex, which task 52 builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "person.ts")]
pub enum ModelProvider {
    Gemini,
    Anthropic,
    Vertex,
}

/// The agent's provider and its model's id, set by the server's configuration, never by a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "person.ts")]
pub struct AgentModel {
    pub provider: ModelProvider,
    pub model: String,
}
