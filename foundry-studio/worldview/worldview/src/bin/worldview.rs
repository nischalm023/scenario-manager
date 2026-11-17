//! Looking at a `.worldview` from a terminal: what frames it holds, which of
//! them a seek would read, and whether its index block tells the truth.
//!
//! It exists so a question about a real file — why is this one 400 MB, why
//! does scrubbing to minute forty read so much, did this crashed run keep
//! anything — is answered by reading the file rather than by adding logging
//! somewhere. It lives behind the `cli` feature so a wasm build of the
//! library never sees clap.

use std::fs::{File, metadata};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use worldview::container::index::OrderCheck;
use worldview::container::recover::{Loaded, load, recover, verified_prefix};
use worldview::{Error, Index, Ms, Reader, Tag};

#[derive(Parser)]
#[command(name = "worldview", about = "Inspect a .worldview recording")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print every frame: where it starts, what it is, how big it is, and
    /// which sections a keyframe or a delta carries.
    Inspect { file: PathBuf },
    /// Print the frames a seek to one instant would read.
    Seek {
        file: PathBuf,
        /// The simulated instant, in milliseconds.
        ms: u64,
    },
    /// Print the index records, from the block when the file carries one.
    Index {
        file: PathBuf,
        /// Also scan the frames and say whether the block matches them.
        #[arg(long)]
        verify: bool,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("worldview: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Inspect { file } => inspect(&file),
        Command::Seek { file, ms } => seek(&file, Ms(ms)),
        Command::Index { file, verify } => index(&file, verify),
    }
}

/// How the file opened, in one line: the trailer named its frames, or a scan
/// found them and the file is live or was cut short.
fn how_it_opened(path: &Path, loaded: &Loaded) -> String {
    let len = metadata(path).map(|meta| meta.len()).unwrap_or(0);
    match loaded.through_trailer {
        true => format!("opened through the trailer: {} frames, {len} bytes", loaded.index.len()),
        false => format!(
            "opened by scanning: {} frames verified through byte {} of {len}",
            loaded.index.len(),
            loaded.verified_end
        ),
    }
}

/// List a file's frames, and — when the frames stop at damage — the frames it
/// still holds, then where the damage is. Someone looking at a crashed run
/// wants the prefix, not only the complaint.
fn inspect(path: &Path) -> Result<(), String> {
    let refused = match load(path) {
        Ok(loaded) => {
            println!("{}", how_it_opened(path, &loaded));
            return list_frames(path, &loaded.index);
        }
        Err(error) => error,
    };
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let (index, at, damage) = verified_prefix(&mut file, 0, &mut OrderCheck::default()).map_err(|e| e.to_string())?;
    if index.is_empty() {
        return Err(refused.to_string());
    }
    println!("opened by scanning: {} frames verified through byte {at}", index.len());
    list_frames(path, &index)?;
    Err(match damage {
        Some(damage) => format!("damage at offset {at}: {damage}"),
        None => refused.to_string(),
    })
}

fn list_frames(path: &Path, index: &Index) -> Result<(), String> {
    let mut reader =
        Reader::new(File::open(path).map_err(|e| e.to_string())?, index.clone()).map_err(|e| e.to_string())?;
    println!(
        "{:>12}  {:>12}  {:<9}  {:>10}  {:>10}  {:<5}  sections",
        "offset", "t_ms", "tag", "payload", "frame", "flags"
    );
    let mut totals = [(0u64, 0u64); 7];
    for record in index.records().to_vec() {
        let frame = reader.frame_at(record.offset).map_err(|e| e.to_string())?;
        let sections = match record.tag.carries_sections() {
            true => reader
                .sections_at(&record)
                .map_err(|e| e.to_string())?
                .iter()
                .map(|(id, body)| format!("{id}:{}", body.len()))
                .collect::<Vec<_>>()
                .join(" "),
            false => String::new(),
        };
        println!(
            "{:>12}  {:>12}  {:<9}  {:>10}  {:>10}  {:<5}  {sections}",
            record.offset,
            record.t.0,
            record.tag,
            frame.header.payload_len,
            record.len,
            if frame.header.compressed() { "zstd" } else { "raw" }
        );
        let slot = &mut totals[record.tag.as_byte() as usize];
        slot.0 += 1;
        slot.1 += u64::from(record.len);
    }
    println!();
    for (byte, (frames, bytes)) in totals.iter().enumerate() {
        if *frames > 0 {
            let tag = Tag::from_byte(byte as u8).expect("the slot came from a tag byte");
            println!("{tag:<9}  {frames:>7} frames  {bytes:>12} bytes");
        }
    }
    Ok(())
}

fn seek(path: &Path, t: Ms) -> Result<(), String> {
    let reader = Reader::open(path).map_err(|e| e.to_string())?;
    let plan = reader.seek(t).map_err(|e| e.to_string())?;

    println!("seeking to {t}");
    println!(
        "  keyframe at {:>12} ms, offset {:>12}, {} bytes",
        plan.keyframe.t.0, plan.keyframe.offset, plan.keyframe.len
    );
    for declare in &plan.declares {
        println!("  declare  at {:>12} ms, offset {:>12}, {} bytes", declare.t.0, declare.offset, declare.len);
    }
    for delta in &plan.deltas {
        println!("  delta    at {:>12} ms, offset {:>12}, {} bytes", delta.t.0, delta.offset, delta.len);
    }
    let bytes: u64 = plan.ranges().iter().map(|(_, len)| u64::from(*len)).sum();
    println!("  {} deltas applied, {bytes} bytes read, reaching {}", plan.deltas.len(), plan.reaches());
    if let Some(edge) = reader.live_edge()
        && t > edge
    {
        println!("  {t} is past the file's last frame at {edge}; a live caller would tail");
    }
    Ok(())
}

fn index(path: &Path, verify: bool) -> Result<(), String> {
    let mut reader = Reader::open(path).map_err(|e| e.to_string())?;
    let block = reader.index_block().map_err(|e| e.to_string())?;
    let records = match &block {
        Some(block) => {
            println!("from the index block: {} records", block.len());
            block.clone()
        }
        None => {
            let scanned = recover(path).map_err(|e| e.to_string())?;
            println!("no index block; from a scan: {} records", scanned.index.len());
            scanned.index
        }
    };
    println!("{:>12}  {:>12}  {:<9}  {:>10}", "offset", "t_ms", "tag", "len");
    for record in records.records() {
        println!("{:>12}  {:>12}  {:<9}  {:>10}", record.offset, record.t.0, record.tag, record.len);
    }
    if verify {
        println!("{}", verdict(path, block.as_ref()).map_err(|e| e.to_string())?);
    }
    Ok(())
}

/// Whether the block names exactly the frames a fresh scan finds ahead of it.
/// The block is written before the index frame and the end frame, so a scan
/// finds those two on top of it.
fn verdict(path: &Path, block: Option<&Index>) -> Result<String, Error> {
    let Some(block) = block else {
        return Ok("nothing to verify: the file carries no index block".to_owned());
    };
    let scanned = recover(path)?;
    let frames = scanned.index.records();
    let matches = frames.len() == block.len() + 2 && frames[..block.len()] == *block.records();
    Ok(match matches {
        true => format!("verified: the block names the {} frames ahead of it", block.len()),
        false => format!("the block names {} records and a scan finds {} frames", block.len(), frames.len()),
    })
}
