//! Within `grammar`, `world` names places in WPS's world graph: scope paths, the pins that fix a scope's version,
//! locations (a scope at a pin), and the `kin` pins an address holds beside a frozen location (spec 03 sections 3.1
//! and 3.2).

use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{GrammarError, string_form};

const MAX_SLUGS: usize = 8;
const MAX_SLUG_CHARS: usize = 64;
const TAG_PREFIX: &str = "tag:";
const HEAD: &str = "head";
const MAIN: &str = "main";

/// Why `slug` is not one (1 to 64 of `a-z`, `0-9` and `-`, first and last a letter or digit), or `None` when it is.
fn slug_fault(slug: &str) -> Option<String> {
    if slug.is_empty() {
        return Some("a slug is never empty".to_owned());
    }
    if let Some(odd) = slug.chars().find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')) {
        return Some(format!("the slug `{slug}` holds `{odd}`, outside a-z, 0-9 and -"));
    }
    if slug.len() > MAX_SLUG_CHARS {
        return Some(format!("the slug `{slug}` is longer than {MAX_SLUG_CHARS} characters"));
    }
    if slug.starts_with('-') || slug.ends_with('-') {
        return Some(format!("the slug `{slug}` must start and end with a letter or digit"));
    }
    None
}

fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// A scope's path in WPS: 1 to 8 slugs joined by `/`, every prefix itself a scope (`customer/demo/site/lshape-6/map`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub struct ScopePath(String);

impl ScopePath {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn slugs(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    /// The scope `below` names under this one, as a kin pin's path names a scope under its site's root scope.
    pub fn join(&self, below: &ScopePath) -> Result<ScopePath, GrammarError> {
        format!("{self}/{below}").parse()
    }

    /// Whether `other` is this scope or a scope under it.
    pub fn holds(&self, other: &ScopePath) -> bool {
        other.0 == self.0 || other.0.strip_prefix(&self.0).is_some_and(|rest| rest.starts_with('/'))
    }
}

impl FromStr for ScopePath {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let slugs = text.split('/').count();
        if slugs > MAX_SLUGS {
            let why = format!("it has {slugs} slugs, and a scope path has at most {MAX_SLUGS}");
            return Err(GrammarError::new("a scope path", text, why));
        }
        match text.split('/').find_map(slug_fault) {
            Some(why) => Err(GrammarError::new("a scope path", text, why)),
            None => Ok(Self(text.to_owned())),
        }
    }
}

/// A site's root scope path, `customer/{c}/site/{s}` (spec 03 section 3.2's `site`), the boundary scope a site's
/// conversation is keyed by (spec 01 section 3.17).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub struct SitePath(ScopePath);

impl SitePath {
    pub fn as_scope(&self) -> &ScopePath {
        &self.0
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl FromStr for SitePath {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let path: ScopePath = text.parse().map_err(|fault: GrammarError| fault.reframe("a site path", text))?;
        match path.slugs().collect::<Vec<_>>().as_slice() {
            ["customer", _, "site", _] => Ok(Self(path)),
            _ => Err(GrammarError::new("a site path", text, "a site's root scope is customer/{customer}/site/{site}")),
        }
    }
}

/// A scope's changeset number in its own sequence, 0 being the empty scope: a number in JSON (`{ scope, seq }`), and
/// in a pin its decimal text with no leading zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export_to = "grammar.ts")]
pub struct Seq(u64);

impl Seq {
    pub const fn new(changeset: u64) -> Self {
        Self(changeset)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl FromStr for Seq {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refuse = |why: &str| GrammarError::new("a changeset", text, why);
        if !all_digits(text) {
            return Err(refuse("a changeset is written in digits"));
        }
        if text.len() > 1 && text.starts_with('0') {
            return Err(refuse("a changeset is written without leading zeros"));
        }
        text.parse().map(Self).map_err(|_| refuse("it is larger than any changeset"))
    }
}

/// A branch's name: a slug that is never all digits, which would read as a changeset, and never `head`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub struct BranchName(String);

impl BranchName {
    /// The branch every site and template has, and the one a location without a pin follows (rule 2).
    pub fn main() -> Self {
        Self(MAIN.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for BranchName {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refuse = |why: String| GrammarError::new("a branch name", text, why);
        if let Some(why) = slug_fault(text) {
            return Err(refuse(why));
        }
        if all_digits(text) {
            return Err(refuse("an all-digit name reads as a changeset".to_owned()));
        }
        if text == HEAD {
            return Err(refuse("`head` is a pin of entity references only, never a branch".to_owned()));
        }
        Ok(Self(text.to_owned()))
    }
}

/// A tag's name: a slug, written after `tag:` in a pin.
#[derive(Debug, Clone, PartialEq, Eq, Hash, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub struct TagName(String);

impl TagName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for TagName {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match slug_fault(text) {
            Some(why) => Err(GrammarError::new("a tag name", text, why)),
            None => Ok(Self(text.to_owned())),
        }
    }
}

/// What fixes a scope's version: its own changeset (`1`), a tag (`tag:v3`), or a branch (`main`), which follows the
/// branch's head as it moves.
#[derive(Debug, Clone, PartialEq, Eq, Hash, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub enum Pin {
    Changeset(Seq),
    Tag(TagName),
    Branch(BranchName),
}

/// A pin as an entity reference may carry it: any pin, or `head`, which follows the referrer's own branch (rules 3
/// and 4) and which a location never carries.
#[derive(Debug, Clone, PartialEq, Eq, Hash, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub enum RefPin {
    Head,
    Pin(Pin),
}

impl FromStr for Pin {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let reframe = |fault: GrammarError| fault.reframe("a pin", text);
        if let Some(name) = text.strip_prefix(TAG_PREFIX) {
            return name.parse().map(Self::Tag).map_err(reframe);
        }
        if all_digits(text) {
            return text.parse().map(Self::Changeset).map_err(reframe);
        }
        text.parse().map(Self::Branch).map_err(reframe)
    }
}

impl FromStr for RefPin {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text == HEAD { Ok(Self::Head) } else { text.parse().map(Self::Pin) }
    }
}

/// What a panel shows: a scope at a pin (`customer/demo/site/lshape-6/map@1`). Rule 2: a location never says
/// `@head`, and an input without a pin is read as `@main`; the canonical text always carries the pin.
#[derive(Debug, Clone, PartialEq, Eq, Hash, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub struct Location {
    scope: ScopePath,
    pin: Pin,
}

impl Location {
    pub fn new(scope: ScopePath, pin: Pin) -> Self {
        Self { scope, pin }
    }

    pub fn scope(&self) -> &ScopePath {
        &self.scope
    }

    pub fn pin(&self) -> &Pin {
        &self.pin
    }
}

impl FromStr for Location {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let reframe = |fault: GrammarError| fault.reframe("a location", text);
        let (path, pin) = match text.split_once('@') {
            None => (text, Pin::Branch(BranchName::main())),
            Some((_, pin)) if pin.contains('@') => {
                return Err(GrammarError::new("a location", text, "a location holds one `@`"));
            }
            Some((_, HEAD)) => {
                let why = "a location never says `@head`: it has no referrer whose branch to follow";
                return Err(GrammarError::new("a location", text, why));
            }
            Some((path, pin)) => (path, pin.parse().map_err(reframe)?),
        };
        Ok(Self { scope: path.parse().map_err(reframe)?, pin })
    }
}

/// One kin scope held at a changeset in an address's `kin` parameter (`flow@2`): the scope's path below its site's
/// root scope, never at a branch or a tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub struct KinPin {
    scope: ScopePath,
    seq: Seq,
}

impl KinPin {
    pub fn scope(&self) -> &ScopePath {
        &self.scope
    }

    pub fn seq(&self) -> Seq {
        self.seq
    }
}

impl FromStr for KinPin {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refuse = |why: &str| GrammarError::new("a kin pin", text, why);
        let Some((path, changeset)) = text.split_once('@') else {
            return Err(refuse("a kin pin names its changeset, as `flow@2` does"));
        };
        if changeset.contains('@') {
            return Err(refuse("a kin pin holds one `@`"));
        }
        if !all_digits(changeset) {
            return Err(refuse("a kin pin holds its scope at a changeset, never at a branch or a tag"));
        }
        let reframe = |fault: GrammarError| fault.reframe("a kin pin", text);
        Ok(Self { scope: path.parse().map_err(reframe)?, seq: changeset.parse().map_err(reframe)? })
    }
}

/// The `kin` parameter: kin pins joined by `,` (`flow@2,fleet@1`), kept in the order written.
#[derive(Debug, Clone, PartialEq, Eq, Hash, TS)]
#[ts(type = "string", export_to = "grammar.ts")]
pub struct KinPins(Vec<KinPin>);

impl KinPins {
    pub fn pins(&self) -> &[KinPin] {
        &self.0
    }

    /// Refuse these pins beside `shown`, the location they ride with, whose site's root scope is `site`: kin pins
    /// stand only beside a changeset location, each names a scope under the site within a scope path's eight slugs
    /// (section 3.1), and none names the shown scope (spec 03 section 3.2, `kin`).
    pub fn check_beside(&self, shown: &Location, site: &ScopePath) -> Result<(), GrammarError> {
        let refuse = |why: String| GrammarError::new("kin pins", &self.to_string(), why);
        if !matches!(shown.pin, Pin::Changeset(_)) {
            return Err(refuse(format!("kin pins stand only beside a changeset location, never beside `{shown}`")));
        }
        if !site.holds(&shown.scope) {
            return Err(refuse(format!("the shown scope `{}` is not under the site `{site}`", shown.scope)));
        }
        for pin in &self.0 {
            let Ok(scope) = site.join(&pin.scope) else {
                let slugs = site.slugs().count() + pin.scope.slugs().count();
                let path = format!("{site}/{}", pin.scope);
                let why = format!(
                    "the kin pin `{pin}` names `{path}`, {slugs} slugs, and a scope path has at most {MAX_SLUGS}"
                );
                return Err(refuse(why));
            };
            if scope == shown.scope {
                return Err(refuse(format!("the kin pin `{pin}` names the shown scope `{}`", shown.scope)));
            }
        }
        Ok(())
    }
}

impl FromStr for KinPins {
    type Err = GrammarError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let reframe = |fault: GrammarError| fault.reframe("kin pins", text);
        let pins = text.split(',').map(KinPin::from_str).collect::<Result<Vec<_>, _>>().map_err(reframe)?;
        for (n, pin) in pins.iter().enumerate() {
            if pins[..n].iter().any(|earlier| earlier.scope == pin.scope) {
                return Err(GrammarError::new("kin pins", text, format!("they name `{}` twice", pin.scope)));
            }
        }
        Ok(Self(pins))
    }
}

impl Display for ScopePath {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

impl Display for SitePath {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        write!(out, "{}", self.0)
    }
}

impl Display for Seq {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        write!(out, "{}", self.0)
    }
}

impl Display for BranchName {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

impl Display for TagName {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

impl Display for Pin {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Changeset(seq) => write!(out, "{seq}"),
            Self::Tag(name) => write!(out, "{TAG_PREFIX}{name}"),
            Self::Branch(name) => write!(out, "{name}"),
        }
    }
}

impl Display for RefPin {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Head => out.write_str(HEAD),
            Self::Pin(pin) => write!(out, "{pin}"),
        }
    }
}

impl Display for Location {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        write!(out, "{}@{}", self.scope, self.pin)
    }
}

impl Display for KinPin {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        write!(out, "{}@{}", self.scope, self.seq)
    }
}

impl Display for KinPins {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        for (n, pin) in self.0.iter().enumerate() {
            if n > 0 {
                out.write_str(",")?;
            }
            write!(out, "{pin}")?;
        }
        Ok(())
    }
}

string_form!(ScopePath, SitePath, BranchName, TagName, Pin, RefPin, Location, KinPin, KinPins);
