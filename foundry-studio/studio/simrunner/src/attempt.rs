//! Within `simrunner`, `attempt` is one attempt as it runs today, in the order spec 08 section 3.5 gives
//! a runner, every step before simtime for real and in wall clock: read the job spec and the attempt's pins on the
//! runners' listener, export the site at the plan's changesets through the WPS client, read the sealed scenario
//! version, apply the variant to the world and the workload spec, have the generator make the workload, compile
//! through the seam, lay out the run's folder, hand in the manifest, and prepare the run with dessim's own harness;
//! then the run, and the report of what it left. Attempt control is read before handing in the manifest, so a row
//! cancellation after the API calls the run `running` reaches the simulation even while its inputs are being
//! prepared. The heartbeat beats from the first line; SIGTERM, from the first line too, is the platform ending the
//! run, never a person's cancellation, and abandons whichever step before the simulation it finds running, even one a
//! person's cancel let go on. When the spec names a streaming server, the run's progress goes out to it live and its
//! ending last (`stream`).

use std::env;
use std::fs;
use std::future::Future;
use std::io::Error as IoError;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use dessim::harness::{RunOverrides as HarnessOverrides, prepare};
use dessim::sim_types::Counters;
use dessim::worldmap::{World, load_from_bytes};
use desunits::SimTime;
use reqwest::Method;
use scenario_spec::{
    Generated, REQUEST_VERSION, RUN_FILES, apply_variant, compile, request_bytes, require_catalog_motion, site_facts,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use studio_protocol::{
    AppliedPatch, AppliedTo, ByteCount, CompileRead, EndedBy, Ending, FORMAT, FindingPart, GenerateRequest,
    GeneratorSpec, HOLD_VAR, JobSpec, Manifest, ManifestAccepted, Pins, ResultsEnded, RunOverrides, RunState,
    SealedVersion, THREADS_VAR, Variant, VariantNumber, WallClock, WorkloadLine, WorkloadRead, WorldRead,
};
use thiserror::Error;
use tokio::select;
use tokio::signal::unix::{SignalKind, signal};
use tokio::task::spawn_blocking;
use tokio::time::sleep;
use url::Url;
use wps_client::routes::{RouteShape, Routes};
use wps_client::{
    BlockingCompute, Client, ClientConfig, CutExport, Exactness, HttpConfig, HttpWps, Walked, WorldStore,
};

use crate::door::control::Control;
use crate::door::heartbeat::Beating;
use crate::door::report::deliver;
use crate::door::{Door, RETRIES};
use crate::simulate::{Left, RECORDING_FILE, simulate};
use crate::stop::RunnerStop;
use crate::stream::Streaming;

/// The words an attempt SIGTERM stopped ends with.
const TERMINATED: &str = "terminated by the platform";

/// A scenario patch under this pointer changes the generator request, which a manifest names by the field keeping it.
const GENERATOR: &str = "/workload/generator/";
const REQUEST_FILE: &str = "workload.spec";

/// Why an attempt stopped before it could write its ending itself: an input it could not read or a file it could not
/// write, or the platform's stop, which the platform ends; an input the simulator's reader refused; or a person's
/// cancel, its preparation then abandoned by the platform's stop.
#[derive(Debug, Error)]
pub enum Stop {
    #[error("{0}")]
    Platform(String),
    #[error("{0}")]
    Simulator(String),
    #[error("{0}")]
    Cancelled(String),
}

impl Stop {
    pub fn ending(&self) -> Ending {
        let (state, ended_by, words) = match self {
            Self::Platform(words) => (RunState::Failed, EndedBy::Platform, words),
            Self::Simulator(words) => (RunState::Failed, EndedBy::Simulator, words),
            Self::Cancelled(words) => (RunState::Cancelled, EndedBy::Person, words),
        };
        Ending { state, ended_by, stop_reason: words.clone(), stop_detail: None, sim_ms: SimTime::ZERO }
    }
}

/// What the runner was handed: its job spec's path, its output folder, its token, and how long to hold, if at all,
/// between its manifest's acceptance and the simulation (`HOLD_VAR`, wall clock, which the cancel and restart tests set
/// to catch a runner mid-run at a known moment).
pub struct Handed {
    pub job: PathBuf,
    pub out: PathBuf,
    pub token: String,
    pub hold: Option<Duration>,
}

/// Run one attempt to its ending, written into the output folder and delivered to the API, its recording and then its
/// report, whatever the ending says. `Err` when they could not be delivered, the door never taking them or not even
/// known, the job spec not reading; what the runner left stays in its folder, for the local invoker's settle or the
/// lost-runner ending to go by.
pub async fn run(handed: &Handed) -> Result<ResultsEnded, Stop> {
    let spec = match job_spec(&handed.job) {
        Ok(spec) => spec,
        Err(stopped) => {
            write(&handed.out.join("ending.json"), &to_json(&stopped.ending()))?;
            return Err(stopped);
        }
    };
    let door = Door::new(&spec.studio.endpoint, &handed.token);
    let counters = Counters::new();
    let _beating = Beating::start(&door, &spec, counters.clone());
    let stop = RunnerStop::new();
    terminated(stop.clone());
    // The live stream to the server the spec names, if any; the run never waits on it.
    let streaming = spec.streaming.as_ref().map(|server| {
        let recording = handed.out.join(RECORDING_FILE);
        Streaming::open(server, &spec.run, spec.attempt, &handed.token, counters.clone(), recording)
    });
    let left = match attempted(handed, &spec, &door, stop, counters, streaming.as_ref()).await {
        Ok(left) => left,
        Err(stopped) => {
            eprintln!("simrunner: the attempt stopped: {stopped}");
            let ending = stopped.ending();
            write(&handed.out.join("ending.json"), &to_json(&ending))?;
            Left::ending_only(ending)
        }
    };
    if let Some(streaming) = streaming {
        streaming.finish(left.ending.clone()).await;
    }
    deliver(&door, &spec, left).await
}

fn job_spec(path: &Path) -> Result<JobSpec, Stop> {
    let bytes = fs::read(path).map_err(|fault| Stop::Platform(format!("the job spec could not be read: {fault}")))?;
    serde_json::from_slice(&bytes).map_err(|fault| Stop::Platform(format!("the job spec does not read: {fault}")))
}

/// SIGTERM is the platform ending the run, never a person's cancellation: it abandons any step of the preparation
/// still running, after a person's cancel too, and once the simulation runs it stops it, which keeps its recording.
/// It is heard from the moment this returns, so one that comes as the runner starts is not the process's death.
fn terminated(stop: Arc<RunnerStop>) {
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            tokio::spawn(async move {
                term.recv().await;
                stop.platform_failure(TERMINATED.to_owned());
            });
        }
        Err(fault) => eprintln!("simrunner: SIGTERM cannot be heard, and would end the process: {fault}"),
    }
}

/// Every step of the attempt from its pins to what it left, its ending in the output folder. The platform's stop
/// abandons any step before the simulation at once, the manifest's delivery and the simulator's preparation included:
/// a request in flight is dropped and a step computing on a blocking thread is left unread, so the ending and its
/// delivery have the platform's whole grace. A person's cancel, read from the manifest on, lets the preparation finish
/// and reaches the simulation, which ends at once on it and keeps its recording.
async fn attempted(
    handed: &Handed,
    spec: &JobSpec,
    door: &Door,
    stop: Arc<RunnerStop>,
    counters: Counters,
    streaming: Option<&Streaming>,
) -> Result<Left, Stop> {
    let manifest = unless_terminated(&stop, manifest_of(handed, spec, door)).await?;
    let manifest_bytes = to_json(&manifest);
    write(&handed.out.join("manifest.json"), &manifest_bytes)?;
    let _control = Control::start(door, spec, stop.clone()).await;
    let path = format!("/runner/v1/attempts/{}/{}/manifest", manifest.run, manifest.attempt.0);
    let _: ManifestAccepted = unless_terminated(&stop, door.json(Method::PUT, &path, Some(manifest_bytes))).await?;
    let harness = HarnessOverrides { num_threads: threads()?, ..HarnessOverrides::default() };
    let spec_path = handed.out.join("inputs").join("spec.json");
    let preparing = computed(move || {
        prepare(&spec_path, &harness)
            .map_err(|fault| Stop::Simulator(format!("the simulator's reader refused the run: {fault}")))
    });
    let prepared = unless_terminated(&stop, preparing).await?;
    if let Some(streaming) = streaming {
        streaming.started();
    }
    if let Some(hold) = handed.hold {
        eprintln!("simrunner: prepared, holding {} ms before the simulation as {HOLD_VAR} asks", hold.as_millis());
        select! {
            _ = sleep(hold) => {},
            _ = stop.requested() => {},
        }
    }
    let (out, run, attempt) = (handed.out.clone(), manifest.run, manifest.attempt);
    let tap = streaming.map(Streaming::tap);
    let outcome = spawn_blocking(move || simulate(prepared, &out, &run, attempt, stop, counters, tap)).await;
    outcome.map_err(|fault| Stop::Platform(format!("the simulation task could not be waited on: {fault}")))?
}

/// Every step before the manifest, in the order spec 08 section 3.5 gives them: the pins, the site exported at the
/// plan's changesets, the sealed scenario version and the variant applied to both, the workload generated and the run
/// compiled, the run's folder laid out, and the manifest that says what each step read and made. Loading the world
/// and compiling the run compute on blocking threads, so they can be abandoned with the rest.
async fn manifest_of(handed: &Handed, spec: &JobSpec, door: &Door) -> Result<Manifest, Stop> {
    let pins: Pins =
        door.json(Method::GET, &format!("/runner/v1/attempts/{}/{}", spec.run, spec.attempt.0), None).await?;
    let world = wps(&pins.wps.endpoint).await?;
    let cut = world.resolve_fixed_cut(&pins.cut.unit, &pins.cut.seqs).await.map_err(|fault| {
        Stop::Platform(format!("world-proto-server could not resolve the plan's changesets: {fault}"))
    })?;
    let export = world.export(&cut).await.map_err(|fault| {
        Stop::Platform(format!("world-proto-server could not export the site at its pins: {fault}"))
    })?;
    let scenario = &pins.scenario;
    let sealed: SealedVersion = door
        .json(Method::GET, &format!("/runner/v1/scenarios/{}/versions/{}", scenario.id, scenario.version.0), None)
        .await?;
    let mut content = sealed.version.content;
    content.run_controls.seed = pins.seed;
    let unpatched = content.clone();
    let variant = Variant { n: VariantNumber(1), label: String::new(), patches: pins.variant.clone() };
    let exported = export.bytes.clone();
    let (applied, world_bytes, map, content) = computed(move || {
        let fetched: Value = serde_json::from_slice(&exported)
            .map_err(|fault| Stop::Platform(format!("the export does not read as JSON: {fault}")))?;
        let mut world_doc = fetched.clone();
        let applied = apply_variant(&variant, &mut world_doc, &mut content)
            .map_err(|fault| Stop::Simulator(format!("the variant does not apply: {fault}")))?;
        let world_bytes = if world_doc == fetched { exported.to_vec() } else { to_json(&world_doc) };
        let map = load_from_bytes(&world_bytes).map_err(|fault| Stop::Simulator(fault.to_string()))?;
        require_catalog_motion(&map).map_err(Stop::Simulator)?;
        Ok((applied, world_bytes, map, content))
    })
    .await?;
    let world_read = world_read(&export, &map.world);
    let overrides =
        RunOverrides { duration_sim_ms: Some(pins.duration_sim_ms), report_window: Some(pins.report_window.clone()) };
    let facts = site_facts(&map);
    let request_of = |generator: &GeneratorSpec| GenerateRequest {
        v: REQUEST_VERSION,
        generator: generator.clone(),
        duration_sim_ms: pins.duration_sim_ms,
        slots: facts.slots,
        brings_stock_in: facts.brings_stock_in,
    };
    let request = request_of(&content.workload.generator);
    let sent = request_bytes(&request);
    let stream = door.bytes(Method::POST, "/runner/v1/workloads/generate", Some(sent.clone())).await?;
    let (generated, compiled, content, stream) = computed(move || {
        let generated = generated(&stream)?;
        let compilation = compile(&map, &content, &generated);
        let compiled = compilation.compiled.ok_or_else(|| {
            let problems: Vec<&str> = compilation.problems.iter().map(|problem| problem.text.as_str()).collect();
            Stop::Simulator(problems.join(" "))
        })?;
        Ok((generated, compiled, content, stream))
    })
    .await?;
    let inputs = handed.out.join("inputs");
    let spec_json = compiled.spec(&content, &overrides, RUN_FILES);
    let spec_bytes = to_json(&spec_json);
    for (file, bytes) in [
        ("spec.json", spec_bytes.as_slice()),
        (RUN_FILES.world, world_bytes.as_slice()),
        (RUN_FILES.items, &generated.items_csv()),
        (RUN_FILES.orders, &generated.orders_csv()),
    ] {
        write(&inputs.join(file), bytes)?;
    }
    // Each patch's artifact hashed either side of the variant: the world as exported and as loaded, the generator
    // request built from the version and as sent, or the run's spec.
    let world_after = sha256(&world_bytes);
    let request_before = sha256(&request_bytes(&request_of(&unpatched.workload.generator)));
    let request_after = sha256(&sent);
    // Floor traffic is the only sweep that patches spec.json: it keeps the flow and workload, so both sides use
    // this compiled plan. Generator sweeps instead hash their request, and world sweeps hash their export.
    let spec_before = sha256(&to_json(&compiled.spec(&unpatched, &overrides, RUN_FILES)));
    let spec_after = sha256(&spec_bytes);
    let variant = applied
        .into_iter()
        .map(|applied| {
            let (file, before, after) = match applied.patch.applied_to {
                AppliedTo::World => (RUN_FILES.world, &export.sha256, &world_after),
                AppliedTo::Spec if applied.patch.pointer.starts_with(GENERATOR) => {
                    (REQUEST_FILE, &request_before, &request_after)
                }
                AppliedTo::Spec => ("spec.json", &spec_before, &spec_after),
            };
            AppliedPatch {
                file: file.to_owned(),
                sha256_before: before.clone(),
                sha256_after: after.clone(),
                patch: applied.patch,
                was: applied.was,
            }
        })
        .collect();
    let workload_sha256 = sha256(&stream);
    let mut differs = Vec::new();
    for (part, same) in [
        (FindingPart::Compile, Some(&compiled.digest) == pins.expected.compile_sha256.as_ref()),
        (FindingPart::World, export.sha256 == pins.expected.world_sha256),
        (FindingPart::Workload, workload_sha256 == pins.expected.workload_sha256),
        (FindingPart::Reads, export.scopes == pins.reads),
        (FindingPart::Exact, world_read.exact == pins.cut.exact),
        (FindingPart::Compiler, compiled.compiler == pins.expected.compiler),
    ] {
        if !same {
            differs.push(part);
        }
    }
    Ok(Manifest {
        v: FORMAT,
        run: pins.run.clone(),
        attempt: pins.attempt,
        plan: pins.plan.clone(),
        experiment: pins.experiment.clone(),
        scenario: pins.scenario.clone(),
        cut: pins.cut.clone(),
        reads: export.scopes.clone(),
        // WPS writes the site whole in one answer, so no carried scope's own export is read.
        exports: Vec::new(),
        world: world_read,
        variant,
        workload: WorkloadRead {
            spec: request,
            seed: content.workload.generator.seed,
            generator: generated.header.generator.clone(),
            sha256: workload_sha256,
            bytes: ByteCount(stream.len() as u64),
        },
        compile: CompileRead {
            v: FORMAT,
            compiler: compiled.compiler,
            digest: Some(compiled.digest),
            expected: pins.expected.clone(),
            differs,
        },
        seed: pins.seed,
        duration_sim_ms: pins.duration_sim_ms,
        report_window: pins.report_window.clone(),
        runner_image: pins.runner.image.clone(),
        fetched_at: WallClock::now(),
        stub: false,
    })
}

/// `step`, abandoned the moment the platform stops the attempt, whether or not a person's cancel came first. The
/// attempt then ends as its first stop says: cancelled, by the person who asked first, or failed in the platform's
/// words.
pub(crate) async fn unless_terminated<T>(
    stop: &RunnerStop,
    step: impl Future<Output = Result<T, Stop>>,
) -> Result<T, Stop> {
    select! {
        biased;
        words = stop.terminated() => Err(if stop.cancelled_first() {
            Stop::Cancelled(format!("cancelled before the simulation, its preparation abandoned: {words}"))
        } else {
            Stop::Platform(words)
        }),
        done = step => done,
    }
}

/// `work` computed on a blocking thread, so the preparation it belongs to can be abandoned while it runs.
pub(crate) async fn computed<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Stop> + Send + 'static,
) -> Result<T, Stop> {
    let joined = spawn_blocking(work).await;
    joined.map_err(|fault| Stop::Platform(format!("a step of the preparation could not be waited on: {fault}")))?
}

/// The invoker's thread count (`THREADS_VAR`) as dessim's executor thread count, none when unset.
fn threads() -> Result<Option<usize>, Stop> {
    match env::var(THREADS_VAR) {
        Ok(text) => text
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|threads| *threads > 0)
            .map(Some)
            .ok_or_else(|| Stop::Platform(format!("{THREADS_VAR} is {text:?}, not a count of threads above 0"))),
        Err(_) => Ok(None),
    }
}

/// The WPS client over the address the pins name, probed as every client is at start, asked again while it does not
/// answer.
async fn wps(endpoint: &str) -> Result<Client<HttpWps>, Stop> {
    let unreadable = |fault: String| Stop::Platform(format!("world-proto-server at {endpoint}: {fault}"));
    let base = Url::parse(endpoint).map_err(|fault| unreadable(fault.to_string()))?;
    let routes = Routes::new(base, RouteShape::Path).map_err(unreadable)?;
    let mut last = String::new();
    for wait in [Duration::ZERO].iter().chain(RETRIES.iter()) {
        sleep(*wait).await;
        let http = HttpWps::new(HttpConfig::new(routes.clone())).map_err(|fault| unreadable(fault.to_string()))?;
        match Client::start(http, ClientConfig::default(), Arc::new(BlockingCompute)).await {
            Ok(client) => return Ok(client),
            Err(fault) => last = fault.to_string(),
        }
    }
    Err(unreadable(format!("it did not answer after three tries: {last}")))
}

/// The generator's stream read back into what it made.
fn generated(stream: &[u8]) -> Result<Generated, Stop> {
    let mut header = None;
    let (mut items, mut orders) = (Vec::new(), Vec::new());
    for line in stream.split(|byte| *byte == b'\n').filter(|line| !line.is_empty()) {
        match serde_json::from_slice::<WorkloadLine>(line) {
            Ok(WorkloadLine::Header(read)) => header = Some(read),
            Ok(WorkloadLine::Item(item)) => items.push(item),
            Ok(WorkloadLine::Order(order)) => orders.push(order),
            Err(fault) => return Err(Stop::Platform(format!("the generator's stream does not read: {fault}"))),
        }
    }
    let header = header.ok_or_else(|| Stop::Platform("the generator's stream has no header".to_owned()))?;
    Ok(Generated { header, items, orders })
}

/// The manifest's world: the export as read, before the variant, which WPS both assembled and generated, writing the
/// site as its flagged export; and the primary frame of the world the runner loaded from it, the frame that states
/// `unit_mm`, since the file names none.
fn world_read(export: &CutExport, loaded: &World) -> WorldRead {
    let (exact, exact_reason) = match &export.exact {
        Exactness::Exact => (true, None),
        Exactness::ComponentsOnly { reason } => (false, Some(reason.clone())),
    };
    WorldRead {
        sha256: export.sha256.clone(),
        scopes: export.scopes.clone(),
        walked: match &export.walked {
            Walked::Annotations => "annotations".to_owned(),
            Walked::AnnotationsAndList { list } => format!("annotations-and-list:{list}"),
        },
        exact,
        exact_reason,
        assembled_by: "server".to_owned(),
        derived_by: "server".to_owned(),
        primary_frame: loaded.frames.primary().map(ToString::to_string),
    }
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), Stop> {
    let fault = |fault: IoError| Stop::Platform(format!("{} could not be written: {fault}", path.display()));
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(fault)?;
    }
    fs::write(path, bytes).map_err(fault)
}

fn to_json<T: Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).expect("a runner's file is JSON")
}

pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::mpsc;

    use tokio::net::TcpListener;
    use tokio::time::timeout;

    use super::*;

    /// The manifest says WPS assembled the world and generated its derived entities, never the runner; and it names
    /// the primary frame of the world the runner loaded, the frame that states `unit_mm`, from a file that names none,
    /// as WPS's flagged export names none.
    #[test]
    fn the_world_read_names_wps_and_the_loaded_worlds_primary_frame() {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sim/dessim/tests/fixtures/minimal_easyloop.json");
        let authored = fs::read(&path).unwrap_or_else(|fault| panic!("{} reads: {fault}", path.display()));
        let map = load_from_bytes(&authored).expect("the site loads");
        let primary = map.world.frames.primary().map(ToString::to_string);
        assert!(primary.is_some(), "the loaded world has a primary frame");
        let mut file: Value = serde_json::from_slice(&authored).expect("the site reads as JSON");
        let fields = file.as_object_mut().expect("the site is an object");
        for prologue in ["proto_version", "doc", "primary_frame"] {
            fields.remove(prologue);
        }
        let bytes = serde_json::to_vec(&file).expect("the file encodes");
        let export = CutExport {
            sha256: sha256(&bytes),
            bytes: bytes.into(),
            scopes: BTreeMap::new(),
            tag: None,
            exact: Exactness::Exact,
            walked: Walked::Annotations,
            holders: Default::default(),
        };
        let read = world_read(&export, &map.world);
        assert_eq!((read.assembled_by.as_str(), read.derived_by.as_str()), ("server", "server"));
        assert_eq!(read.primary_frame, primary);
    }

    /// A listener that takes every connection and never answers; its address.
    async fn silent() -> String {
        let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", socket.local_addr().unwrap());
        tokio::spawn(async move {
            let mut held = Vec::new();
            loop {
                held.push(socket.accept().await.unwrap());
            }
        });
        endpoint
    }

    /// SIGTERM abandons the manifest's delivery to a listener that never answers it, at once and in the platform's
    /// words.
    #[tokio::test]
    async fn sigterm_abandons_a_manifest_the_door_never_answers() {
        let door = Door::new(&silent().await, "attempt-test-token");
        let stop = RunnerStop::new();
        let path = "/runner/v1/attempts/01KZAVPQNES65HV4EZADPCS7E8/1/manifest";
        let delivering =
            unless_terminated(&stop, door.json::<ManifestAccepted>(Method::PUT, path, Some(b"{}".to_vec())));
        let terminating = async {
            sleep(Duration::from_millis(200)).await;
            stop.platform_failure(TERMINATED.to_owned());
        };
        let (abandoned, ()) = timeout(Duration::from_secs(5), async { tokio::join!(delivering, terminating) })
            .await
            .expect("abandoned at once");
        assert_eq!(abandoned.expect_err("terminated").to_string(), TERMINATED);
    }

    /// SIGTERM abandons a step still computing on its blocking thread, at once, leaving it to finish unread, alone or
    /// after a person's cancel that let the step go on; the attempt then ends as its first stop says.
    #[tokio::test]
    async fn sigterm_abandons_a_step_still_computing_alone_or_after_a_cancel() {
        for cancelled_first in [false, true] {
            let (release, released) = mpsc::channel::<()>();
            let stop = RunnerStop::new();
            if cancelled_first {
                stop.cancel();
            }
            let computing = unless_terminated(&stop, computed(move || Ok(released.recv().is_ok())));
            let terminating = async {
                sleep(Duration::from_millis(200)).await;
                stop.platform_failure(TERMINATED.to_owned());
            };
            let (abandoned, ()) = timeout(Duration::from_secs(5), async { tokio::join!(computing, terminating) })
                .await
                .expect("abandoned at once");
            let ending = abandoned.expect_err("terminated").ending();
            let (state, ended_by, words) = if cancelled_first {
                let words = format!("cancelled before the simulation, its preparation abandoned: {TERMINATED}");
                (RunState::Cancelled, EndedBy::Person, words)
            } else {
                (RunState::Failed, EndedBy::Platform, TERMINATED.to_owned())
            };
            assert_eq!((ending.state, ending.ended_by, ending.stop_reason), (state, ended_by, words));
            release.send(()).expect("the abandoned step let finish");
        }
    }

    /// A person's cancel abandons no step of the preparation: it reaches the simulation, which ends on it.
    #[tokio::test]
    async fn a_persons_cancel_lets_the_preparation_finish() {
        let stop = RunnerStop::new();
        stop.cancel();
        let finished = unless_terminated(&stop, computed(|| Ok(7))).await;
        assert_eq!(finished.expect("the step finished"), 7);
    }
}
