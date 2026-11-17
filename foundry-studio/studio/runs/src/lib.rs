//! The Crucible server: one process that runs simulations, records each one as
//! a `.worldview`, and serves what it recorded.
//!
//! The crate is a library as well as a server so that the terminal client
//! beside it speaks the server's own protocol types rather than a second
//! spelling of them.
//!
//! The parts: [`config`] is what the server was told, [`registry`] is which
//! runs exist, [`spec`] turns a submission back into a folder, [`runs`] runs
//! and records them, [`protocol`] is what a browser says, [`ws`] is one
//! browser's socket, and [`client`] is the asking side of the same protocol,
//! which the terminal client speaks.
//!
//! Serving what was recorded is the other half: [`recording`] puts a stored
//! recording on this machine, [`floor`] compiles the world document a browser
//! cannot, [`attach`] is one browser's reading of one recording, [`series`] is
//! a chart's points, and [`outbound`] is the socket's write side, which drops
//! an attachment that has stopped keeping up.

pub mod attach;
pub mod client;
pub mod config;
pub mod floor;
pub mod outbound;
pub mod protocol;
pub mod recording;
pub mod registry;
pub mod runs;
pub mod series;
pub mod spec;
pub mod ws;

use native_modules as _;
