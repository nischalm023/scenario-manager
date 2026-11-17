//! Entity identities copied by the Foundry generators: a UUID root and an
//! optional application-defined derivation path. Only syntax is checked;
//! key names and payloads retain their exact spelling.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// One identity, held in the spelling the reader writes it in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity(String);

impl Identity {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Identity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Identity, D::Error> {
        let text = String::deserialize(deserializer)?;
        spelled(&text).map(Identity).map_err(D::Error::custom)
    }
}

impl Serialize for Identity {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// Which kind of identity a text is reads off the text itself, as it does in
/// the reader: a UUID holds no `/`, and a derivation address parts at its
/// first.
fn spelled(text: &str) -> Result<String, String> {
    match text.split_once('/') {
        None => declared(text),
        Some((root, key)) => Ok(format!("{}/{}", declared(root)?, derivation_key(key)?)),
    }
}

fn declared(text: &str) -> Result<String, String> {
    Uuid::parse_str(text)
        .map(|uuid| uuid.hyphenated().to_string())
        .map_err(|_| format!("`{text}` is not an entity identity: a declared identity is a UUID"))
}

/// A derivation key is written back exactly as it stands: the reader keeps
/// each segment as written, checking only nonemptiness and reserved delimiters.
fn derivation_key(key: &str) -> Result<&str, String> {
    let malformed =
        |segment: &str, reason: &str| format!("derivation key `{key}` is malformed at segment `{segment}`: {reason}");
    if key.is_empty() {
        return Err(malformed("", "a derivation key must name at least one segment"));
    }
    for segment in key.split('/') {
        if segment.is_empty() {
            return Err(malformed(segment, "a key segment must not be empty"));
        }
        if segment.contains('@') {
            return Err(malformed(segment, "a key segment must contain neither `/` nor `@`"));
        }
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::spelled;

    #[test]
    fn copied_references_keep_application_keys_exactly() {
        let id = "a0000000-0000-4000-8000-000000000001/DeckA/42/north-port/Δ";
        assert_eq!(spelled(id).unwrap(), id);
        for key in ["A@7", "/A", "A/", "A//B"] {
            assert!(spelled(&format!("a0000000-0000-4000-8000-000000000001/{key}")).is_err());
        }
    }
}
