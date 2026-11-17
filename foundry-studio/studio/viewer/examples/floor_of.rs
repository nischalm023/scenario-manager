//! Compile a world document into the floor document a browser is sent with no
//! run around it (spec 03 section 3.6): the one compile, written as
//! `FloorAndRoll`, exactly as a server writes it.
//!
//!   cargo run -p crucible-viewer --example floor_of -- <world.json> <out>
//!
//! The viewer package's browser test draws the floor this writes from the
//! made L-Shape under `tests/fixtures/l_shape`.

use std::env::args;
use std::fs::{read, write};
use std::process::ExitCode;

use dessim::worldmap::load_from_bytes;
use worldview_render::FloorAndRoll;

fn main() -> ExitCode {
    let asked: Vec<String> = args().skip(1).collect();
    let [world, out] = asked.as_slice() else {
        eprintln!("floor_of: give the world document to compile and the file to write the floor document to");
        return ExitCode::from(2);
    };
    let written = read(world)
        .map_err(|failure| format!("{world}: {failure}"))
        .and_then(|bytes| load_from_bytes(&bytes).map_err(|failure| format!("{world} will not load: {failure}")))
        .and_then(|map| worldview_floor::compile(&map).map_err(|failure| format!("{world} states no floor: {failure}")))
        .and_then(|compiled| {
            FloorAndRoll::write(&compiled.floor, &compiled.roll()).map_err(|failure| failure.to_string())
        })
        .and_then(|bytes| write(out, &bytes).map(|()| bytes.len()).map_err(|failure| format!("{out}: {failure}")));
    match written {
        Ok(bytes) => {
            eprintln!("floor_of: wrote {bytes} bytes to {out}");
            ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("floor_of: {why}");
            ExitCode::FAILURE
        }
    }
}
