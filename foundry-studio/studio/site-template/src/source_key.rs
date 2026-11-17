//! Within `site-template`, a `source_key` is what in a template made an entity of a made site (spec 05 section
//! 3.5): the template entity's UUID, then for a placement's product one `/instance:{i},{j}/local:{handle}` step per
//! placement it was laid out by. The engine writes it into the made entity's `external_ids`, which is how an update
//! finds the entity again; it is a join key, never a reference.

use std::cmp::Ordering;
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use studio_protocol::DeclaredId;

use crate::count::InstanceIndex;

const INSTANCE: &str = "instance:";
const LOCAL: &str = "local:";

/// `entity-id { "/instance:" i "," j "/local:" handle }`, the UUID in lower case.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceKey(String);

/// One placement step of a key: the instance's indexes and the piece internal's handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step<'a> {
    pub instance: [InstanceIndex; 2],
    pub handle: &'a str,
}

impl SourceKey {
    /// The key of a top-level template entity: its id.
    pub fn entity(id: DeclaredId) -> Self {
        Self(id.to_string())
    }

    /// The key of the product this key's placement lays out at instance `(i, j)` under `handle`.
    pub fn product(&self, instance: [InstanceIndex; 2], handle: &str) -> Self {
        Self(format!("{}/{INSTANCE}{},{}/{LOCAL}{handle}", self.0, instance[0], instance[1]))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The top-level template entity the key starts at.
    pub fn root(&self) -> DeclaredId {
        self.0[..36].parse().expect("a source key opens with the UUID its grammar checked")
    }

    pub fn steps(&self) -> impl Iterator<Item = Step<'_>> {
        let segments: Vec<&str> = self.0[36..].split('/').skip(1).collect();
        segments
            .chunks(2)
            .map(|pair| Step {
                instance: instance_indexes(&pair[0][INSTANCE.len()..]).expect("checked by the grammar"),
                handle: &pair[1][LOCAL.len()..],
            })
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// The handle of the last step, for a product; `None` for a top-level entity.
    pub fn handle(&self) -> Option<&str> {
        self.steps().last().map(|step| step.handle)
    }

    /// The key of the placement that laid this product out, one step shorter; `None` for a top-level entity.
    pub fn placement(&self) -> Option<SourceKey> {
        let cut = self.0.rfind("/instance:")?;
        Some(Self(self.0[..cut].to_owned()))
    }
}

fn instance_indexes(payload: &str) -> Option<[InstanceIndex; 2]> {
    let (i, j) = payload.split_once(',')?;
    let index = |text: &str| -> Option<InstanceIndex> {
        let digits = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
        if !digits || (text.len() > 1 && text.starts_with('0')) {
            return None;
        }
        text.parse().ok().map(InstanceIndex::new)
    };
    Some([index(i)?, index(j)?])
}

impl FromStr for SourceKey {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refuse = |why: &str| format!("`{text}` is not a source key: {why}");
        let Some((uuid, rest)) = text.split_at_checked(36) else {
            return Err(refuse("a source key opens with a UUID"));
        };
        uuid.parse::<DeclaredId>().map_err(|fault| refuse(&fault.to_string()))?;
        let segments: Vec<&str> = rest.split('/').collect();
        if !segments[0].is_empty() {
            return Err(refuse("its steps follow the UUID after a `/`"));
        }
        let steps = &segments[1..];
        if !steps.len().is_multiple_of(2) {
            return Err(refuse("each step is an instance and a local handle"));
        }
        for pair in steps.chunks(2) {
            let well_formed = pair[0].strip_prefix(INSTANCE).and_then(instance_indexes).is_some()
                && pair[1].strip_prefix(LOCAL).is_some_and(|handle| !handle.is_empty());
            if !well_formed {
                return Err(refuse("a step is `instance:{i},{j}/local:{handle}`"));
            }
        }
        Ok(Self(text.to_owned()))
    }
}

/// Keys order by their UUID, then step by step: instance indexes as numbers, handles as text, so `instance:10,0`
/// follows `instance:2,0` (the "source-key order" a placement's products take in a list).
impl Ord for SourceKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0[..36].cmp(&other.0[..36]).then_with(|| {
            let ours = self.steps().map(|step| (step.instance, step.handle));
            let theirs = other.steps().map(|step| (step.instance, step.handle));
            ours.cmp(theirs)
        })
    }
}

impl PartialOrd for SourceKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Display for SourceKey {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

impl Serialize for SourceKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SourceKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?.parse().map_err(Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "6b0d1f2e-9a41-4c55-8d3e-71a2c4e9b0f7";

    #[test]
    fn keys_read_and_order_by_instance_number() {
        let nested: SourceKey = format!("{ROOT}/instance:0,0/local:row/instance:4,0/local:station").parse().unwrap();
        assert_eq!(nested.handle(), Some("station"));
        assert_eq!(nested.placement().unwrap().as_str(), format!("{ROOT}/instance:0,0/local:row"));
        assert_eq!(nested.root().to_string(), ROOT);
        let at = |i: u64| {
            SourceKey::entity(ROOT.parse().unwrap()).product([InstanceIndex::new(i), InstanceIndex::new(0)], "sector")
        };
        assert!(at(2) < at(10));
        for bad in [
            ROOT.to_uppercase(),
            format!("{ROOT}/instance:3/local:x"),
            format!("{ROOT}/local:x"),
            format!("{ROOT}/instance:01,0/local:x"),
        ] {
            assert!(bad.parse::<SourceKey>().is_err(), "{bad}");
        }
    }
}
