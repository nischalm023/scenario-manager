//! The browser's half of the protocol, generated from the server's own types.
//!
//! This test IS the generator: it writes `studio/viewer/js/src/protocol.ts`
//! from `crucible::protocol` and then fails if what it wrote differs from what
//! was checked in. So a change to a message type that nobody regenerated stops
//! the gate here rather than at a browser that silently stopped understanding
//! replies.
//!
//! Run `cargo test -p crucible --test ts_bindings` after changing a message
//! type, and commit what it writes.

use std::path::PathBuf;

use crucible::protocol::{Reply, Request};
use ts_rs::{Config, TS};

/// Where the generated file goes, and how the numbers in it are spelled.
///
/// ts-rs renders `u64` as `bigint`, which is the only lossless choice in the
/// abstract and the wrong one here: every one of these fields crosses the wire
/// as JSON, and `JSON.parse` hands back a plain `number`. A type saying
/// `bigint` would describe values the browser never receives.
fn config() -> (Config, PathBuf) {
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../studio/viewer/js/src");
    (Config::new().with_large_int("number").with_out_dir(out_dir.clone()), out_dir.join("protocol.ts"))
}

#[test]
fn the_browsers_protocol_is_what_the_server_says() {
    let (config, path) = config();
    let before = std::fs::read_to_string(&path).unwrap_or_default();

    Request::export_all(&config).expect("export Request and everything it carries");
    Reply::export_all(&config).expect("export Reply and everything it carries");

    let after = std::fs::read_to_string(&path).expect("the bindings were written");
    assert!(after.contains("Submitted"), "the generated protocol names the server's replies");
    // The two are not printed: they are one long line each, and what a reader
    // needs is the file to look at, which they now have on disk.
    assert!(
        before == after,
        "{} is out of date; this test has just rewritten it — read the change and commit it",
        path.display()
    );
}
