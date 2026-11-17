//! Within `run-invocation`, a `token` is a bearer secret the API mints and keeps only as its SHA-256 (spec 01 section
//! 3.16): an attempt's token, which opens that attempt's routes on the runners' listener and its stream on a streaming
//! server, and a watch ticket's, which opens that stream to a viewer. One type, so every minting and every check of a
//! presented token hashes the same way.

use std::fmt::{self, Debug, Formatter};

use sha2::{Digest, Sha256};

/// A bearer token: 32 random bytes in hex, which never reaches a log, a command line or a job spec.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    pub fn mint() -> Self {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("the operating system's random generator answers");
        Self(hex::encode(bytes))
    }

    /// A token as a runner or a viewer presented it.
    pub fn presented(text: &str) -> Self {
        Self(text.to_owned())
    }

    pub fn sha256(&self) -> String {
        hex::encode(Sha256::digest(self.0.as_bytes()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl Debug for Token {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str("Token(..)")
    }
}
