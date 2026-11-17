//! The container: the bytes a `.worldview` is made of, below the meaning of
//! any payload.
//!
//! **What it is for.** A run is watched while it executes, left, rejoined,
//! scrubbed backward and forward, and analysed afterwards. The container
//! makes every one of those one cheap operation over one artifact: seek to
//! any simulated instant without decoding from the start, tail the live edge,
//! read a crashed run's prefix, and rebuild the workbook from the file alone.
//!
//! **The shape.** A file is frames and nothing else, then a trailer. A frame
//! is a twenty-byte header and one stored payload, compressed on its own, and
//! it is independently decodable: its header says where it ends and its
//! checksum says whether it arrived whole, so a reader can start at any frame
//! offset and a file cut mid-write is a valid prefix. The first frame is the
//! scenario frame, which carries the magic ahead of its compressed body, so
//! one rule opens a file and a stream alike.
//!
//! **What it does not know.** Nothing here knows about robots, totes, metrics
//! or the simulator. A keyframe's and a delta's payloads are sections keyed by
//! a `u8` the scenario block gives a meaning to; every other payload is bytes
//! the container carries. The vocabulary that reads them is a separate layer.

use std::fmt;

use serde::{Deserialize, Serialize};

pub mod compress;
pub mod error;
pub mod frame;
pub mod index;
pub mod payload;
pub mod reader;
pub mod recover;
pub mod sections;
pub mod writer;

/// A simulated instant, in milliseconds from the start of the run — the one
/// clock the container stamps frames with and searches them by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Ms(pub u64);

impl Ms {
    pub const ZERO: Self = Self(0);
}

impl fmt::Display for Ms {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ms", self.0)
    }
}
