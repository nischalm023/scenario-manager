//! The encoding every payload the vocabulary defines keeps, and the carrier
//! for the payloads the container only ferries.
//!
//! A payload is one MessagePack value. Records inside it are positional
//! arrays, never maps with string keys, so a record is a plain
//! `#[derive(Serialize, Deserialize)]` struct written through
//! `rmp_serde::to_vec`, which encodes struct fields as an array in
//! declaration order. Three things follow, and breaking any of them silently
//! changes the format:
//!
//! - A record's array is fixed in arity. No `skip_serializing_if`, no
//!   `#[serde(default)]`, no field added to the middle or the end: a reader
//!   counts positions. A payload gains a field with a container version.
//! - An enum inside a record is one byte, through an explicit conversion.
//!   Serde's own enum form is a one-entry map keyed by the variant name,
//!   which the convention forbids and which costs bytes per record.
//! - A quantity or an identity is its newtype — [`crate::container::Ms`], and
//!   the run's own ids. Each serializes as its inner integer, so the wire is
//!   the integer and the Rust type is the newtype, at no cost in bytes.
//!
//! The section table, the index records and the trailer are the exceptions:
//! those the container packs itself, because a reader searches them without
//! decoding anything.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::container::error::{Error, Result};

/// The MessagePack a frame of one kind carries.
pub trait Payload: Serialize + DeserializeOwned + Sized {
    /// What this payload is called when an error has to name it.
    const NAME: &'static str;

    /// Encode as MessagePack, positional per the convention above. The caller
    /// wraps the bytes in a frame.
    fn to_bytes(&self) -> Result<Vec<u8>> {
        rmp_serde::to_vec(self).map_err(|e| Error::Encode { what: Self::NAME, detail: e.to_string() })
    }

    /// Decode a frame's decompressed payload. `offset` is where the frame
    /// header sits, so a shape complaint says which frame is wrong.
    fn from_bytes(bytes: &[u8], offset: u64) -> Result<Self> {
        rmp_serde::from_slice(bytes).map_err(|e| Error::PayloadShape {
            offset,
            expected: Self::NAME,
            detail: e.to_string(),
        })
    }
}

/// A MessagePack document this crate carries but does not read: the world
/// document, the scenario and its data files, the simulation configuration,
/// a stop reason's own structure, the summary statistics.
///
/// It exists so the container stays free of the simulator, which does not
/// build for wasm32, while a consumer that does link it still finds every
/// byte it needs in the file. The producer writes its own type's MessagePack
/// here and the consumer decodes it back into that type; nothing in between
/// has an opinion.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Opaque(#[serde(with = "serde_bytes")] pub Vec<u8>);

impl Opaque {
    /// Carry a value's MessagePack, positional like everything else.
    pub fn of<T: Serialize>(value: &T, what: &'static str) -> Result<Self> {
        Ok(Self(rmp_serde::to_vec(value).map_err(|e| Error::Encode { what, detail: e.to_string() })?))
    }

    /// Read the carried bytes back into the producer's own type.
    pub fn read<T: DeserializeOwned>(&self, what: &'static str) -> Result<T> {
        rmp_serde::from_slice(&self.0).map_err(|e| Error::OpaqueShape { what, detail: e.to_string() })
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Payload for Opaque {
    const NAME: &'static str = "opaque";
}
