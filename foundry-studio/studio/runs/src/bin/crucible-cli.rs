//! The terminal client: submit a spec to a Crucible server and watch it to its
//! end, without a browser.
//!
//! It is here rather than in a crate of its own so that it speaks the server's
//! own `protocol` types; a second spelling of the messages is a second thing to
//! keep in step. It is what `scripts/worldview/cli_gate.sh` drives, and what a
//! developer uses to put a fixture through the server. Asking a question and
//! reading the answer to it is `crucible::client`, where a test can drive it
//! against a socket saying whatever it likes; what is left here is the
//! commands, the files a submission carries, and the frames an attachment
//! decodes.
//!
//! `attach` is the other half: it reads a run's frames off the wire and decodes
//! each one with `worldview/worldview` — the crate a browser will decode them with —
//! so what the server sends can be shown to be readable without a browser in
//! the room.
//!
//! Submitting reads the spec file and every file it names off this machine's
//! disk and sends them INLINE, keyed by the reference the spec writes, because
//! the server shares no filesystem with whoever submits. `--simtime` and
//! `--threads` are written into the spec's own settings before it is sent: the
//! server takes a spec and nothing else, so anything a run needs said is said
//! in the spec.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use crucible::client::{ask, connect, max_message_bytes};
use crucible::protocol::{
    AttachFrom, AttemptState, AttemptSummary, Progress, Reply, Request, SubmittedFile, SubmittedSpec,
};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message;
use worldview::frames::instant;
use worldview::{
    Declare, Delta, Descriptors, End, Frame, Index, Keyframe, Ms, Payload, RunState, Scenario, Sections, Tag,
};
use worldview_floor::FloorDoc;

/// How often the client asks a running run how it is getting on.
const POLL: Duration = Duration::from_secs(2);

/// What a server accepts in one message by default (`CRUCIBLE_MAX_MESSAGE_BYTES`),
/// for one that greeted this client with no number of its own.
///
/// The files are checked against whichever number stands before they are
/// encoded, so a submission too large to send is refused here rather than after
/// both sides have buffered hundreds of megabytes. Base64 is four bytes out for
/// every three in.
const SERVER_CEILING_BYTES: usize = 32 * 1024 * 1024;

#[tokio::main]
async fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("crucible-cli: {failure}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "usage:
  crucible-cli submit <spec.json> [--simtime <minutes>] [--threads <n>] [--name <name>] [--wait]
  crucible-cli status <run>
  crucible-cli list
  crucible-cli cancel <run>
  crucible-cli attach <run> [<ms>|live] [--sections playback,scalars] [--watch <metric>]
                            [--slow <secs>] [--detach-after <frames>]
  crucible-cli series <run> [<metric>[,<metric>...]] [--until <ms>]   (none = every metric)
  --server <ws url>   the server to talk to (default: the CRUCIBLE_SERVER variable, else ws://127.0.0.1:3001/ws)";

async fn run(args: Vec<String>) -> Result<(), String> {
    let mut args = args.into_iter().peekable();
    let command = args.next().ok_or_else(|| USAGE.to_string())?;
    let mut positional = Vec::new();
    let mut server = std::env::var("CRUCIBLE_SERVER").unwrap_or_else(|_| "ws://127.0.0.1:3001/ws".to_string());
    let (mut simtime_minutes, mut threads, mut name, mut wait) = (None, None, None, false);
    let (mut sections, mut until, mut slow, mut watched) = (Vec::new(), None, None, None);
    let mut detach_after = None;

    while let Some(argument) = args.next() {
        let mut value = |flag: &str| args.next().ok_or_else(|| format!("{flag} needs a value"));
        match argument.as_str() {
            "--server" => server = value("--server")?,
            "--simtime" => simtime_minutes = Some(value("--simtime")?.parse::<u64>().map_err(|e| e.to_string())?),
            "--threads" => threads = Some(value("--threads")?.parse::<u64>().map_err(|e| e.to_string())?),
            "--name" => name = Some(value("--name")?),
            "--wait" => wait = true,
            "--slow" => slow = Some(value("--slow")?.parse::<u64>().map_err(|e| e.to_string())?),
            "--sections" => sections = value("--sections")?.split(',').map(str::to_string).collect(),
            "--watch" => watched = Some(value("--watch")?),
            "--detach-after" => {
                detach_after = Some(value("--detach-after")?.parse::<u64>().map_err(|e| e.to_string())?)
            }
            "--until" => until = Some(value("--until")?.parse::<u64>().map_err(|e| e.to_string())?),
            flag if flag.starts_with("--") => return Err(format!("no such flag: {flag}\n{USAGE}")),
            other => positional.push(other.to_string()),
        }
    }

    match command.as_str() {
        "submit" => {
            let spec = positional.first().ok_or_else(|| USAGE.to_string())?;
            // Asked before a single file is read: the server greets a socket
            // with what it takes in one message, and a submission over that is
            // refused here rather than encoded and sent to be dropped.
            let ceiling = max_message_bytes(&server).await?.unwrap_or(SERVER_CEILING_BYTES);
            let submitted = read_submission(Path::new(spec), simtime_minutes, threads, name, ceiling)?;
            let run = submit(&server, submitted).await?;
            println!("{run}");
            match wait {
                true => watch(&server, &run).await,
                false => Ok(()),
            }
        }
        "status" => {
            let run = positional.first().ok_or_else(|| USAGE.to_string())?;
            print_status(&ask(&server, Request::Status { id: "1".into(), run: run.clone() }).await?);
            Ok(())
        }
        "list" => {
            let Reply::List { runs, .. } = ask(&server, Request::List { id: "1".into() }).await? else {
                return Err("the server did not answer with a list".to_string());
            };
            for run in runs {
                let state = run.attempts.last().map(|attempt| attempt.state.as_str()).unwrap_or("-");
                println!("{}  {:<28} {state}", run.run, run.name);
            }
            Ok(())
        }
        "cancel" => {
            let run = positional.first().ok_or_else(|| USAGE.to_string())?;
            ask(&server, Request::Cancel { id: "1".into(), run: run.clone() }).await?;
            println!("{run} asked to stop");
            Ok(())
        }
        "attach" => {
            let run = positional.first().ok_or_else(|| USAGE.to_string())?;
            let from = match positional.get(1).map(String::as_str) {
                None | Some("live") => AttachFrom::Live,
                Some(ms) => AttachFrom::At(ms.parse::<u64>().map_err(|e| format!("{ms}: {e}"))?),
            };
            attach(&server, run, from, sections, slow, watched, detach_after).await
        }
        "series" => {
            let run = positional.first().ok_or_else(|| USAGE.to_string())?;
            // Naming none asks for every metric the run declared.
            let metrics: Vec<String> =
                positional.get(1).map(|named| named.split(',').map(str::to_string).collect()).unwrap_or_default();
            let asked = Request::Series { id: "1".into(), run: run.clone(), metrics, until };
            let Reply::Series { metrics, .. } = ask(&server, asked).await? else {
                return Err("the server did not answer with a series".to_string());
            };
            for metric in metrics {
                let last = metric.points.last().map(|(at, value)| format!("{value} at {at} ms"));
                println!("{:<40} {:>6} points  {}", metric.metric, metric.points.len(), last.unwrap_or_default());
            }
            Ok(())
        }
        other => Err(format!("no such command: {other}\n{USAGE}")),
    }
}

/// Read a spec and every file it names, with the run's own settings written in,
/// refusing one the server has said it will not take in a message.
fn read_submission(
    spec_path: &Path,
    simtime_minutes: Option<u64>,
    threads: Option<u64>,
    name: Option<String>,
    ceiling: usize,
) -> Result<SubmittedSpec, String> {
    let text = std::fs::read_to_string(spec_path).map_err(|e| format!("cannot read {}: {e}", spec_path.display()))?;
    let mut spec: Value =
        serde_json::from_str(&text).map_err(|e| format!("{} is not JSON: {e}", spec_path.display()))?;
    let rewriting = simtime_minutes.is_some() || threads.is_some();

    // Written into the spec's `settings`, which is a `SimConfig`: `sim_duration`
    // is a Duration, spelled as serde spells one. A spec with no settings block
    // gets one — every field of a `SimConfig` has a default, so an empty block
    // says exactly what no block says.
    if simtime_minutes.is_some() || threads.is_some() {
        let root = spec.as_object_mut().ok_or("the spec is not a JSON object")?;
        let settings = root.entry("settings").or_insert_with(|| serde_json::json!({}));
        if !settings.is_object() {
            return Err(format!("the spec's settings is {settings}, not a block of settings"));
        }
        if let Some(minutes) = simtime_minutes {
            settings["sim_duration"] = serde_json::json!({ "secs": minutes * 60, "nanos": 0 });
        }
        if let Some(threads) = threads {
            settings["num_threads"] = serde_json::json!(threads);
        }
    }

    let directory = spec_path.parent().unwrap_or(Path::new("."));
    let mut read = Vec::new();
    let mut total = 0usize;
    for reference in references(&spec)? {
        let path: PathBuf = directory.join(&reference);
        let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        eprintln!("  {reference} ({} bytes)", bytes.len());
        total += bytes.len();
        read.push((reference, bytes));
    }
    // Checked before anything is encoded: a site extract's workload runs to
    // hundreds of megabytes, and finding that out after base64ing the lot into
    // one string costs both ends the memory for nothing.
    let encoded = total.div_ceil(3) * 4;
    if encoded > ceiling {
        return Err(format!(
            "these files are {total} bytes, {encoded} once encoded, and the server accepts \
             {ceiling} in one message. Raise CRUCIBLE_MAX_MESSAGE_BYTES on the server, \
             or run this spec through desvis"
        ));
    }
    let files: Vec<SubmittedFile> =
        read.into_iter().map(|(reference, bytes)| SubmittedFile { reference, bytes: BASE64.encode(bytes) }).collect();

    // The recording embeds the text as submitted, so the file's own bytes go up
    // untouched unless something was actually asked for. When something was,
    // the output says so: the recording will not match the file on disk.
    let submitted_text = match rewriting {
        false => text,
        true => {
            eprintln!("  (the spec was rewritten with the settings asked for; the recording embeds the rewrite)");
            serde_json::to_string_pretty(&spec).map_err(|e| e.to_string())?
        }
    };
    Ok(SubmittedSpec { spec: submitted_text, files, name })
}

/// Every file a spec names: its world document and its scenario's workload.
fn references(spec: &Value) -> Result<Vec<String>, String> {
    const SLOTS: [&str; 6] = ["items_csv", "orders_csv", "bulk_csv", "outbound_csv", "onhand_csv", "stock_top_up_csv"];
    let world = spec.get("world").and_then(Value::as_str).ok_or("the spec names no world document")?;
    let mut references = vec![world.to_string()];
    if let Some(scenario) = spec.get("scenario") {
        references
            .extend(SLOTS.iter().filter_map(|slot| scenario.get(slot).and_then(Value::as_str).map(str::to_string)));
    }
    Ok(references)
}

/// Submit, and report the run's id.
async fn submit(server: &str, spec: SubmittedSpec) -> Result<String, String> {
    match ask(server, Request::Submit { id: "1".into(), spec }).await? {
        Reply::Submitted { run, .. } => Ok(run),
        other => Err(format!("the server answered a submit with {other:?}")),
    }
}

/// Ask for status until the run is over, printing each change.
async fn watch(server: &str, run: &str) -> Result<(), String> {
    let mut last = String::new();
    loop {
        let reply = ask(server, Request::Status { id: "1".into(), run: run.to_string() }).await?;
        let Reply::Status { run, attempts, progress, .. } = &reply else {
            return Err(format!("the server answered a status with {reply:?}"));
        };
        let line = summary_line(run, attempts, progress);
        if line != last {
            println!("{line}");
            last = line;
        }
        match attempts.last() {
            Some(attempt) if attempt.state.is_over() => {
                return match attempt.state {
                    AttemptState::Failed => Err(attempt
                        .error
                        .clone()
                        .or_else(|| attempt.stop_reason.clone())
                        .unwrap_or_else(|| "the run failed".to_string())),
                    _ => Ok(()),
                };
            }
            _ => tokio::time::sleep(POLL).await,
        }
    }
}

/// Print a status reply as a person reads it.
fn print_status(reply: &Reply) {
    let Reply::Status { run, name, created_at, attempts, .. } = reply else {
        println!("{reply:?}");
        return;
    };
    println!("{run}  {name}  submitted {created_at}");
    for attempt in attempts {
        println!(
            "  attempt {}: {} · sim {} ms · {} picks{}{}",
            attempt.n,
            attempt.state.as_str(),
            attempt.progress.sim_ms,
            attempt.progress.picks,
            attempt.stop_reason.as_deref().map(|why| format!(" · {why}")).unwrap_or_default(),
            attempt.object_key.as_deref().map(|key| format!(" · {key}")).unwrap_or_default(),
        );
        if let Some(held) = attempt.recording {
            println!(
                "    recording: reaches {} ms · {} frames · {} bytes",
                held.live_edge_ms, held.frames, held.size_bytes
            );
        }
    }
}

/// One line saying where a run has got to.
fn summary_line(run: &str, attempts: &[AttemptSummary], progress: &Progress) -> String {
    match attempts.last() {
        Some(attempt) => format!(
            "{run}: {} · sim {} ms · {} picks{}",
            attempt.state.as_str(),
            progress.sim_ms,
            progress.picks,
            attempt.object_key.as_deref().map(|key| format!(" · {key}")).unwrap_or_default(),
        ),
        None => format!("{run}: no attempt yet"),
    }
}

/// Watch a run: ask to attach, then decode every frame the server sends with
/// the same crate a browser decodes them with, and say what each one held.
///
/// `--slow <seconds>` is the back-pressure gate in the small: the client asks to
/// attach and then stops reading for that long, which is what a browser busy
/// elsewhere looks like from the server. The server fills the socket, waits its
/// configured wait, gives up on the attachment and says where to resume; a
/// client that starts reading again while the server is still saying so is told.
async fn attach(
    server: &str,
    run: &str,
    from: AttachFrom,
    sections: Vec<String>,
    slow: Option<u64>,
    watching: Option<String>,
    detach_after: Option<u64>,
) -> Result<(), String> {
    let mut socket = connect(server).await?;
    let asked = Request::Attach { id: "1".into(), run: run.to_string(), from, sections };
    let text = serde_json::to_string(&asked).map_err(|e| e.to_string())?;
    socket.send(Message::Text(text.into())).await.map_err(|e| e.to_string())?;
    if let Some(seconds) = slow {
        tokio::time::sleep(Duration::from_secs(seconds)).await;
    }

    let mut heard = Heard { watching, ..Heard::default() };
    while let Some(message) = socket.next().await {
        match message.map_err(|e| e.to_string())? {
            Message::Binary(bytes) => {
                heard.frame(&bytes)?;
                if detach_after == Some(heard.frames) {
                    let stop = serde_json::to_string(&Request::Detach { id: "2".into() }).expect("encodes");
                    socket.send(Message::Text(stop.into())).await.map_err(|e| e.to_string())?;
                }
            }
            Message::Text(text) => {
                let reply: Reply = serde_json::from_str(&text).map_err(|e| format!("{e}: {text}"))?;
                match reply {
                    Reply::Attached { attempt, from_ms, live_edge_ms, .. } => {
                        println!("attached: attempt {attempt} · from {from_ms} ms · the run reaches {live_edge_ms} ms");
                    }
                    Reply::Ended { outcome, .. } => {
                        println!("{heard}");
                        println!("ended: {outcome}");
                        return Ok(());
                    }
                    Reply::Lagged { resume_from_ms, .. } => {
                        println!("{heard}");
                        println!("lagged: re-attach at {resume_from_ms} ms");
                        return Ok(());
                    }
                    Reply::Detached { .. } => {
                        // Whatever was in flight when the request went may
                        // still land; what matters is that no frame arrives
                        // after this reply.
                        let at = heard.frames;
                        println!("{heard}");
                        println!("detached: after {at} frames");
                        return Ok(());
                    }
                    Reply::Error { message, .. } => return Err(message),
                    // The greeting opens every socket and answers no request;
                    // this one asked to watch a run, so it is nothing to say.
                    Reply::Hello { .. } => {}
                    other => println!("{other:?}"),
                }
            }
            Message::Close(_) => break,
            _ => continue,
        }
    }
    println!("{heard}");
    Err("the socket closed before the run's end frame arrived".to_string())
}

/// What the frames said, decoded as they arrived.
#[derive(Default)]
struct Heard {
    frames: u64,
    bytes: usize,
    deltas: u64,
    declares: u64,
    reached_ms: u64,
    descriptors: Option<Descriptors>,
    /// The run rebuilt from the frames as they arrive, which is what a browser
    /// does with them: the keyframe replaces it and every delta advances it.
    state: Option<RunState>,
    /// The delta received but not yet applied. A delta states where its slice
    /// BEGINS; only the next frame says where it ends, and the last slice of a
    /// run that stopped part way through one ends at the end block's stamp, not
    /// a whole slice later. So each delta waits for the frame behind it.
    pending: Option<(Ms, Delta)>,
    /// A metric to report the running total of, so the value the wire adds up
    /// to can be held against the one `series` assembled from the file.
    watching: Option<String>,
}

impl Heard {
    /// Decode one frame the way a browser will: the scenario block becomes the
    /// floor and the descriptors, the keyframe becomes a state, and every delta
    /// after it advances that state.
    fn frame(&mut self, bytes: &[u8]) -> Result<(), String> {
        let (frame, consumed) = Frame::decode_at(bytes, 0).map_err(|e| format!("frame {}: {e}", self.frames))?;
        // One binary message is one frame and nothing behind it. Bytes past the
        // frame are a producer this client does not understand, and reading the
        // frame and dropping the rest would hide that.
        if consumed != bytes.len() {
            return Err(format!(
                "frame {}: the message is {} bytes and the frame is {consumed}; one message is one frame",
                self.frames,
                bytes.len()
            ));
        }
        let body = frame.contents(0).map_err(|e| e.to_string())?;
        self.frames += 1;
        self.bytes += bytes.len();
        self.reached_ms = self.reached_ms.max(frame.header.t.0);
        // This frame's stamp is where the slice before it ended.
        self.settle(frame.header.t)?;
        match frame.header.tag {
            Tag::Scenario => println!("{}", self.scenario(&body)?),
            Tag::Index => {
                let index = Index::from_block(&body, 0).map_err(|e| e.to_string())?;
                println!("index:    {} records up to the instant asked for", index.len());
            }
            Tag::Declare => {
                let declared = Declare::from_bytes(&body, 0).map_err(|e| e.to_string())?;
                let descriptors = self.descriptors.get_or_insert_default();
                descriptors.push(&declared);
                // The state holds a scalars array laid out by the metrics
                // declared when it was built; a metric declared after the
                // keyframe widens that layout, or the next delta naming it is
                // refused as a metric the run never declared.
                if let Some(state) = self.state.as_mut() {
                    state.scalars.describe(descriptors);
                }
                self.declares += 1;
            }
            Tag::Keyframe => println!("{}", self.keyframe(&body, frame.header.t)?),
            Tag::Delta => {
                self.deltas += 1;
                let sections = Sections::decode(body.to_vec(), 0).map_err(|e| e.to_string())?;
                self.pending = Some((frame.header.t, Delta::decode(&sections, 0).map_err(|e| e.to_string())?));
            }
            Tag::End => {
                let end = End::from_bytes(&body, 0).map_err(|e| e.to_string())?;
                println!("end:      {} · {} frames were written to the file", end.stop_reason, end.frames_written);
            }
        }
        Ok(())
    }

    fn scenario(&mut self, body: &[u8]) -> Result<String, String> {
        let scenario = Scenario::from_bytes(body, 0).map_err(|e| e.to_string())?;
        let floor: FloorDoc = scenario
            .floor
            .as_ref()
            .ok_or("the scenario block carries no compiled floor")?
            .read("the compiled floor")
            .map_err(|e| e.to_string())?;
        let census: Vec<(String, Vec<u32>)> = scenario
            .census
            .as_ref()
            .ok_or("the scenario block carries no census")?
            .read("the entity census")
            .map_err(|e| e.to_string())?;
        let first: Vec<&str> = scenario.metrics.iter().take(4).map(|metric| metric.name.as_str()).collect();
        self.descriptors = Some(Descriptors::from_scenario(&scenario));
        Ok(format!(
            "scenario: {} · {} metrics ({}, …) · {} cells named · world {} bytes · {} data files · producer {} bytes\n\
             floor:    {} cells at {} mm · {} edges · {} bays of steel · {} census rows",
            scenario.run_name,
            scenario.metrics.len(),
            first.join(", "),
            scenario.cells.len(),
            scenario.world.0.len(),
            scenario.data.as_ref().map_or(0, Vec::len),
            scenario.producer.as_ref().map_or(0, |producer| producer.0.len()),
            floor.cells.iter().filter(|cell| cell.is_some()).count(),
            floor.cell_size_mm,
            floor.edges.len(),
            floor.rack.steel_mm.len(),
            census.len(),
        ))
    }

    /// Apply the delta that was waiting, now that a frame has said where its
    /// slice ends.
    ///
    /// The end block's stamp is the run's stop, so the final partial slice of
    /// a run that stopped part way through one ends THERE and not a whole
    /// slice later — a state claiming to reach past the end frame is a viewer
    /// with its cursor off the end of the run. A slice the next frame says
    /// ends before it began is refused, as the state refuses it.
    fn settle(&mut self, ends_at: Ms) -> Result<(), String> {
        let Some((begins_at, delta)) = self.pending.take() else { return Ok(()) };
        let Some(state) = self.state.as_mut() else { return Ok(()) };
        state.apply(instant(begins_at), instant(ends_at), &delta).map_err(|e| format!("at {} ms: {e}", begins_at.0))
    }

    fn keyframe(&mut self, body: &[u8], t: Ms) -> Result<String, String> {
        let sections = Sections::decode(body.to_vec(), 0).map_err(|e| e.to_string())?;
        let keyframe = Keyframe::decode(&sections, 0).map_err(|e| e.to_string())?;
        let descriptors = self.descriptors.clone().unwrap_or_default();
        let state = RunState::from_keyframe(instant(t), &keyframe, &descriptors).map_err(|e| e.to_string())?;
        self.state = Some(state.clone());
        Ok(format!(
            "keyframe: {} ms · sections {:?} · {} robots · {} totes · {} stations · {} metric values",
            t.0,
            sections.ids(),
            state.playback.robots.len(),
            state.playback.totes.len(),
            state.playback.stations.len(),
            state.scalars.values.len(),
        ))
    }
}

impl std::fmt::Display for Heard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let (Some(name), Some(state)) = (&self.watching, &self.state) {
            let slot = self.descriptors.as_ref().and_then(|known| known.metric_slot(name));
            let value = slot.and_then(|slot| state.scalars.values.get(slot).copied().flatten());
            writeln!(f, "applied:  {name} = {value:?} at {} ms, from the deltas alone", state.reached.as_ms())?;
        }
        write!(
            f,
            "read:     {} frames, {} bytes · {} deltas · {} later declarations · reaching {} ms",
            self.frames, self.bytes, self.deltas, self.declares, self.reached_ms
        )
    }
}

#[cfg(test)]
mod tests {
    use worldview::frames::scalars::{AggregationClass, MetricDescriptor, MetricSemantics, MetricUnit, ScalarsDelta};
    use worldview::{Declare, Delta, Frame, Keyframe, Ms, Payload, Sections, Tag};

    use super::Heard;

    /// One frame of sections as the server sends it: one message.
    fn message(tag: Tag, t: u64, sections: Vec<(u8, Vec<u8>)>) -> Vec<u8> {
        let laid_out: Vec<(u8, &[u8])> = sections.iter().map(|(id, body)| (*id, body.as_slice())).collect();
        Frame::encode(tag, Ms(t), &Sections::encode(&laid_out), true).expect("the frame encodes")
    }

    /// A metric the run declares after the keyframe the client opened on is
    /// read from the next delta, as a browser reads it, not refused for
    /// naming a metric the run never declared.
    #[test]
    fn a_metric_declared_after_the_keyframe_is_read_from_the_next_delta() {
        let mut heard = Heard::default();
        let keyframe = Keyframe {
            playback: Some(Default::default()),
            scalars: Some(Default::default()),
            heat: Some(Default::default()),
            robot_state: None,
        };
        heard.frame(&message(Tag::Keyframe, 0, keyframe.encode().unwrap())).expect("the keyframe opens the state");
        let declared = Declare::Metric(MetricDescriptor {
            name: "picks".to_owned(),
            label: "Picks".to_owned(),
            description: String::new(),
            semantics: MetricSemantics {
                unit: MetricUnit::Count,
                denominator_description: None,
                aggregation: AggregationClass::Additive,
            },
        });
        let declaration = Frame::encode(Tag::Declare, Ms(0), &declared.to_bytes().unwrap(), true).unwrap();
        heard.frame(&declaration).expect("the declaration");
        let delta = Delta { scalars: Some(ScalarsDelta { values: vec![Some(3.0)] }), ..Delta::default() };
        heard.frame(&message(Tag::Delta, 0, delta.encode().unwrap())).expect("the delta waits for its end");
        // The next frame says where the slice ends, which applies the delta.
        heard.frame(&message(Tag::Delta, 10_000, Delta::default().encode().unwrap())).expect("the delta applies");
        assert_eq!(heard.state.expect("the state").scalars.values, [Some(3.0)]);
    }
}
