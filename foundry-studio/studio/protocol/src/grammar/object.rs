//! Within `grammar`, `object` names Agent Studio's own Postgres objects: a ULID, the `kind:ULID` key a field holds
//! when it may name several kinds, and the field key that points into one object (spec 03 section 3.1).

use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{GrammarError, string_form};

const ULID_CHARS: usize = 26;
/// Crockford's base32 in upper case: the digits and the letters but I, L, O and U.
const CROCKFORD: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A Postgres object's id: a ULID, 26 characters of Crockford base32 in upper case. A field that names one kind of
/// object holds this bare form (`run`, `scenario`, `draft`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub struct Ulid(String);

impl Ulid {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Ulid {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refuse = |why: String| GrammarError::new("a ULID", text, why);
        if let Some(odd) = text.chars().find(|c| !CROCKFORD.contains(*c)) {
            let case = if CROCKFORD.contains(odd.to_ascii_uppercase()) { ", and a ULID is upper case" } else { "" };
            return Err(refuse(format!("`{odd}` is not in Crockford's base32{case}")));
        }
        if text.len() != ULID_CHARS {
            return Err(refuse(format!("a ULID is {ULID_CHARS} characters, and this is {}", text.len())));
        }
        Ok(Self(text.to_owned()))
    }
}

/// The kinds of Postgres object a key names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export_to = "grammar.ts")]
pub enum ObjectKind {
    Conversation,
    Scenario,
    Experiment,
    Run,
    Submission,
    Draft,
    TemplateDraft,
}

impl ObjectKind {
    pub const ALL: [ObjectKind; 7] = [
        Self::Conversation,
        Self::Scenario,
        Self::Experiment,
        Self::Run,
        Self::Submission,
        Self::Draft,
        Self::TemplateDraft,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Scenario => "scenario",
            Self::Experiment => "experiment",
            Self::Run => "run",
            Self::Submission => "submission",
            Self::Draft => "draft",
            Self::TemplateDraft => "template-draft",
        }
    }
}

/// One Postgres object, as a field that can hold several kinds names it: `run:01KN0TJZANP5N0PGJD8F7A3FDB`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub struct ObjectKey {
    kind: ObjectKind,
    id: Ulid,
}

impl ObjectKey {
    pub fn new(kind: ObjectKind, id: Ulid) -> Self {
        Self { kind, id }
    }

    pub fn kind(&self) -> ObjectKind {
        self.kind
    }

    pub fn id(&self) -> &Ulid {
        &self.id
    }
}

impl FromStr for ObjectKey {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refuse = |why: String| GrammarError::new("an object key", text, why);
        let Some((word, id)) = text.split_once(':') else {
            return Err(refuse(
                "an object key is a kind and a ULID, as `run:01KN0TJZANP5N0PGJD8F7A3FDB` is".to_owned(),
            ));
        };
        let Some(kind) = ObjectKind::ALL.into_iter().find(|kind| kind.as_str() == word) else {
            return Err(refuse(format!("`{word}` is not a kind of object")));
        };
        Ok(Self { kind, id: id.parse().map_err(|fault: GrammarError| fault.reframe("an object key", text))? })
    }
}

/// One field of a Postgres object, named for `data-ref` and highlights and never stored: an object key, `#`, and an
/// RFC 6901 JSON pointer into the object (`scenario:01KN0TJZANP5N0PGJD8F7A3FDB#/fleet/htm_count`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub struct FieldKey {
    object: ObjectKey,
    pointer: String,
}

impl FieldKey {
    pub fn object(&self) -> &ObjectKey {
        &self.object
    }

    pub fn pointer(&self) -> &str {
        &self.pointer
    }
}

impl FromStr for FieldKey {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refuse = |why: &str| GrammarError::new("a field key", text, why);
        let Some((object, pointer)) = text.split_once('#') else {
            return Err(refuse("a field key is an object key, `#` and a JSON pointer"));
        };
        if !pointer.is_empty() && !pointer.starts_with('/') {
            return Err(refuse("a JSON pointer is empty or starts with `/`"));
        }
        let mut escapes = pointer.split('~').skip(1);
        if escapes.any(|after| !(after.starts_with('0') || after.starts_with('1'))) {
            return Err(refuse("in a JSON pointer, `~` is followed by `0` or `1`"));
        }
        let object = object.parse().map_err(|fault: GrammarError| fault.reframe("a field key", text))?;
        Ok(Self { object, pointer: pointer.to_owned() })
    }
}

impl Display for Ulid {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

impl Display for ObjectKind {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str(self.as_str())
    }
}

impl Display for ObjectKey {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        write!(out, "{}:{}", self.kind, self.id)
    }
}

impl Display for FieldKey {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        write!(out, "{}#{}", self.object, self.pointer)
    }
}

string_form!(Ulid, ObjectKey, FieldKey);
