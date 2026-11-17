//! Within `grammar`, `entity` names world entities: a declared entity's UUID, a derived entity's key chain under its
//! generator's UUID, and the reference that may carry a pin, with rules 4 and 5 of spec 03 section 3.1 on which pin
//! a reference may carry. `DeclaredId::mint` is the one place Agent Studio makes an entity id (spec 03 section 6):
//! a random v4, never derived from anything.

use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use thiserror::Error;
use ts_rs::TS;
use uuid::Uuid;

use super::{GrammarError, Pin, RefPin, string_form};
use crate::problem::ProblemType;

/// `8-4-4-4-12` hex digits and four hyphens.
const UUID_CHARS: usize = 36;
const UUID_HYPHENS: [usize; 4] = [8, 13, 18, 23];

/// Whether `text` opens with UUID-shaped characters in either case, so a parser tells an entity from an object key
/// or a `scope:` key before it insists on lower case.
pub(super) fn opens_with_uuid(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() > 8 && bytes[..8].iter().all(u8::is_ascii_hexdigit) && bytes[8] == b'-'
}

/// A declared entity's identity: a UUID in lower case (`b6a9a763-687c-4067-909b-460e986e2229`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub struct DeclaredId(Uuid);

impl DeclaredId {
    /// A new identity for an entity Agent Studio declares: random, so that no two declarations can collide by
    /// construction and nothing can recompute one (the standing ruling: ids are never minted deterministically).
    pub fn mint() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl FromStr for DeclaredId {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refuse = |why: &str| GrammarError::new("a UUID", text, why);
        if text.len() != UUID_CHARS || UUID_HYPHENS.iter().any(|&at| text.as_bytes()[at] != b'-') {
            return Err(refuse("a UUID is 36 characters, hex digits grouped 8-4-4-4-12 by hyphens"));
        }
        let digits = text.char_indices().filter(|(at, _)| !UUID_HYPHENS.contains(at)).map(|(_, digit)| digit);
        if let Some(odd) = digits.clone().find(|digit| !digit.is_ascii_hexdigit()) {
            return Err(refuse(&format!("`{odd}` is not a hex digit")));
        }
        if digits.clone().any(|digit| digit.is_ascii_uppercase()) {
            return Err(refuse("a UUID is written in lower case"));
        }
        Uuid::parse_str(text).map(Self).map_err(|_| refuse("it is not a UUID"))
    }
}

/// An entity's identity: a declared entity's UUID, or a derived entity's generator UUID and its key chain
/// (`1d931662-e00e-4cb8-b431-38d5c08f119d/cell:3,17`), each segment kept byte for byte as its generator wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub struct EntityId {
    declared: DeclaredId,
    /// Empty for a declared entity, else every segment, each after its `/`.
    chain: String,
}

impl EntityId {
    /// The UUID the identity opens with: the entity's own, or the generator's that derived it.
    pub fn declared(&self) -> DeclaredId {
        self.declared
    }

    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.chain.split('/').skip(1)
    }
}

impl From<DeclaredId> for EntityId {
    fn from(declared: DeclaredId) -> Self {
        Self { declared, chain: String::new() }
    }
}

impl FromStr for EntityId {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refuse = |why: &str| GrammarError::new("an entity id", text, why);
        let Some((uuid, chain)) = text.split_at_checked(UUID_CHARS) else {
            return Err(refuse("an entity id opens with a UUID"));
        };
        let declared = uuid.parse().map_err(|fault: GrammarError| fault.reframe("an entity id", text))?;
        if !chain.is_empty() && !chain.starts_with('/') {
            return Err(refuse("a derived entity's key chain follows its UUID after a `/`"));
        }
        for segment in chain.split('/').skip(1) {
            if segment.is_empty() {
                return Err(refuse("a key chain's segment is never empty"));
            }
            if segment.contains('@') {
                return Err(refuse("an entity id holds no `@`: a pin belongs to a reference, never to a key"));
            }
            if segment.chars().any(char::is_control) {
                return Err(refuse("a key chain's segment holds no control character"));
            }
        }
        Ok(Self { declared, chain: chain.to_owned() })
    }
}

/// A reference to an entity: its identity and, where it needs one, a pin (`0c2f6d5e-8a41-4f7e-9b13-5d2e7a9c4f10@tag:v2`).
/// It names no scope (rule 3): the id's owner is WPS's to look up, and the pin comes from the location it is read at.
#[derive(Debug, Clone, PartialEq, Eq, Hash, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub struct EntityRef {
    id: EntityId,
    pin: Option<RefPin>,
}

impl EntityRef {
    pub fn new(id: EntityId, pin: Option<RefPin>) -> Self {
        Self { id, pin }
    }

    pub fn id(&self) -> &EntityId {
        &self.id
    }

    pub fn pin(&self) -> Option<&RefPin> {
        self.pin.as_ref()
    }

    /// Rules 4 and 5 for an entity whose scope stands `kinship` to the location the reference is read at: the
    /// reference as a command keeps it (a kin reference's `@head` dropped as redundant), or the rule it breaks.
    pub fn under_pin_rule(self, kinship: Kinship) -> Result<EntityRef, PinRuleError> {
        let breach = match (kinship, &self.pin) {
            (Kinship::Kin, None) => return Ok(self),
            (Kinship::Kin, Some(RefPin::Head)) => return Ok(Self { pin: None, ..self }),
            (Kinship::Kin, Some(RefPin::Pin(_))) => PinRuleBreach::PinnedInsideBoundary,
            (Kinship::Lateral, Some(RefPin::Pin(Pin::Changeset(_) | Pin::Tag(_)))) => return Ok(self),
            (Kinship::Lateral, _) => PinRuleBreach::LateralNeedsVersion,
        };
        Err(PinRuleError { breach, reference: self })
    }
}

impl FromStr for EntityRef {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let reframe = |fault: GrammarError| fault.reframe("an entity reference", text);
        let Some((id, pin)) = text.split_once('@') else {
            return Ok(Self { id: text.parse().map_err(reframe)?, pin: None });
        };
        if pin.contains('@') {
            return Err(GrammarError::new("an entity reference", text, "a reference holds one `@`"));
        }
        Ok(Self { id: id.parse().map_err(reframe)?, pin: Some(pin.parse().map_err(reframe)?) })
    }
}

/// Where a referenced entity's scope stands to the location a reference is read at (spec 03 section 3.1, "Kin and
/// lateral"). WPS knows the scope that owns an id; the grammar knows only what each answer allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kinship {
    /// The shown scope, or another scope under the same boundary flag: the same site.
    Kin,
    /// Another site, the customer scope, a catalog item.
    Lateral,
}

/// Which of rules 4 and 5 a reference breaks; spec 04 words the same two as `PinnedInsideBoundary` and
/// `LateralNeedsVersion`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinRuleBreach {
    PinnedInsideBoundary,
    LateralNeedsVersion,
}

impl PinRuleBreach {
    pub fn problem_type(self) -> ProblemType {
        match self {
            Self::PinnedInsideBoundary => ProblemType::RefPinConflicts,
            Self::LateralNeedsVersion => ProblemType::RefNeedsVersion,
        }
    }
}

/// A reference that breaks a pin rule, told with the rule in words, as a tool's failed row shows it.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub struct PinRuleError {
    pub breach: PinRuleBreach,
    pub reference: EntityRef,
}

impl Display for PinRuleError {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        let reference = &self.reference;
        match self.breach {
            PinRuleBreach::PinnedInsideBoundary => write!(
                out,
                "`{reference}` names an entity of the shown site, which follows the location's own pin; drop its pin"
            ),
            PinRuleBreach::LateralNeedsVersion => write!(
                out,
                "`{reference}` names an entity outside the shown site, so it needs a changeset or a tag \
                 (`@3`, `@tag:v2`), never a branch, `@head` or no pin"
            ),
        }
    }
}

impl Display for DeclaredId {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        write!(out, "{}", self.0.hyphenated())
    }
}

impl Display for EntityId {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        write!(out, "{}{}", self.declared, self.chain)
    }
}

impl Display for EntityRef {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        match &self.pin {
            Some(pin) => write!(out, "{}@{pin}", self.id),
            None => write!(out, "{}", self.id),
        }
    }
}

string_form!(DeclaredId, EntityId, EntityRef);
