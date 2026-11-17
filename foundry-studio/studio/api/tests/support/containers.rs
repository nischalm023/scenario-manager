//! What the container tests share (spec 08 section 3.5's local invoker, a container of the runner image per attempt):
//! what they need to run at all, a live world-proto-server, an engine that answers and the runner image, each said
//! skipped when missing; a runner of a test's own, an image built of a shell script; an engine that refuses a command
//! once; the setup each container gets; a stack whose runs go to containers, its runners' listener on a port a
//! container reaches, which may hold a runner's pins or its manifest until the test lets them on; the run loops over a
//! container invoker of their own; and the reads the tests make of a run and its container.

use std::env;
use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{MatchedPath, Request, State};
use axum::http::StatusCode;
use axum::middleware::{Next, from_fn_with_state};
use axum::response::Response;
use run_invocation::{
    AttemptKey, CpuMillis, JobName, JobRequest, LoopSettings, Loops, Resources, RunnerSetting, Running, Seats, Token,
};
use runstore::RunStore as RunObjects;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use studio_api::config::{Deployment, RunConfig};
use studio_api::db::mint_ulid;
use studio_api::projects::make_local_default;
use studio_api::runs::{ContainerInvoker, ContainerSetup, HOST_ALIAS, JournalEvents, PgRunStore, Runs, find_engine};
use studio_api::{AppState, GateState, gate_routes, runner_routes};
use studio_protocol::{ApprovalRecord, AttemptNumber, Endpoint, FORMAT, JobSpec, RunnerImage, ScopePath, Ulid};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use url::Url;
use wps_client::routes::{RouteShape, Routes};
use wps_client::{BlockingCompute, Client, ClientConfig, HttpConfig, HttpWps};

use super::runs::{PERSON, RunStack, experiment, image, plan, send, short_scenario, submit, until};

/// Simtime each real run covers: enough for picks, some 20 s of wall clock in a debug build (`short_scenario`).
pub const SIM_MINUTES: u64 = 10;

/// What every test here needs: the live server, an engine that answers, and the runner image; `None` says why not.
pub struct Needed {
    pub wps: String,
    pub engine: PathBuf,
    pub image: String,
}

pub fn needed() -> Option<Needed> {
    let Ok(wps) = env::var("STUDIO_TEST_WPS_URL") else {
        eprintln!("skipped: set STUDIO_TEST_WPS_URL to a world-proto-server simrunner can read");
        return None;
    };
    let engine = engine()?;
    let Ok(image) = env::var("STUDIO_TEST_RUNNER_IMAGE") else {
        eprintln!(
            "skipped: set STUDIO_TEST_RUNNER_IMAGE to the runner image (dev.sh builds studio-simrunner:dev-<key>)"
        );
        return None;
    };
    Some(Needed { wps, engine, image })
}

/// The engine on `PATH`, when it answers; the tests that need no database still need it.
pub fn engine() -> Option<PathBuf> {
    let found = find_engine(None).filter(|engine| {
        StdCommand::new(engine)
            .arg("info")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    });
    if found.is_none() {
        eprintln!("skipped: no container engine answers (docker or podman on PATH)");
    }
    found
}

pub fn shell_image() -> String {
    env::var("STUDIO_TEST_SHELL_IMAGE").unwrap_or_else(|_| "busybox:1.36".to_owned())
}

/// A runner of the test's own: an image of the shell image whose entrypoint runs `body` with the runner's arguments, so
/// `$4` is its output folder, tagged by the script's digest so each script is built once. The shell is PID 1, which
/// ignores a SIGTERM it does not trap, so a stop of it runs out the grace.
pub fn script(engine: &Path, body: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(format!("{}\n{body}", shell_image())));
    let tag = format!("studio-test-runner:{}", &digest[..16]);
    let mut inspect = StdCommand::new(engine);
    inspect.args(["image", "inspect", &tag]).stdout(Stdio::null()).stderr(Stdio::null());
    if inspect.status().is_ok_and(|status| status.success()) {
        return tag;
    }
    let folder = env::temp_dir().join(format!("studio-test-runner-{}", super::ulid()));
    fs::create_dir_all(&folder).expect("a folder for the runner's image");
    fs::write(folder.join("runner.sh"), format!("#!/bin/sh\n{body}\n")).expect("the script written");
    let dockerfile =
        format!("FROM {}\nCOPY runner.sh /runner.sh\nENTRYPOINT [\"sh\", \"/runner.sh\"]\n", shell_image());
    fs::write(folder.join("Dockerfile"), dockerfile).expect("its Dockerfile written");
    let build = StdCommand::new(engine).args(["build", "-q", "-t", &tag]).arg(&folder).stdout(Stdio::null()).status();
    assert!(build.is_ok_and(|status| status.success()), "the engine built the runner image {tag}");
    tag
}

/// An engine standing in front of `engine` that refuses each command `refused` names the first time it is asked, as an
/// engine whose daemon is away does, leaving a file of the command's name in `folder` when it has; every other command,
/// and every later ask, goes to `engine`.
pub fn refusing_engine(engine: &Path, folder: &Path, refused: &[&str]) -> PathBuf {
    fs::create_dir_all(folder).expect("a folder for the engine's stand-in");
    let stand_in = folder.join("engine.sh");
    let body = format!(
        "#!/bin/sh\ncase \"$1\" in\n  {commands})\n    if [ ! -e '{folder}/'\"$1\" ]; then\n      touch '{folder}/'\"$1\"\n      \
         echo 'Cannot connect to the Docker daemon' >&2\n      exit 1\n    fi\n    ;;\nesac\nexec '{engine}' \"$@\"\n",
        commands = refused.join("|"),
        folder = folder.display(),
        engine = engine.display(),
    );
    fs::write(&stand_in, body).expect("the stand-in written");
    fs::set_permissions(&stand_in, Permissions::from_mode(0o755)).expect("the stand-in runs");
    stand_in
}

/// A container a stopped API process left for the Job `name`, labelled a runner's and, when `deadline` gives one, with
/// the deadline it started with (its instant in Unix seconds, and its allowance), running `body` in the shell image.
pub fn left_container(engine: &Path, name: &str, deadline: Option<(i64, u64)>, body: &str) {
    let mut run = StdCommand::new(engine);
    run.args(["run", "-d", "--name", name, "--label", "agent-studio=runner"]);
    if let Some((at, allowance)) = deadline {
        run.args(["--label", &format!("agent-studio/deadline-at={at}")]);
        run.args(["--label", &format!("agent-studio/deadline-secs={allowance}")]);
    }
    let left = run.args([shell_image().as_str(), "sh", "-c", body]).stdout(Stdio::null()).status();
    assert!(left.is_ok_and(|status| status.success()), "the engine ran the container a stopped process left");
}

/// The one runner image a stack's plans name and its attempts run: `reference`, as the engine knows it.
pub fn runner_image(reference: &str) -> RunnerImage {
    RunnerImage { digest: reference.to_owned(), tag: reference.to_owned(), ..image() }
}

/// The setup every container here gets: the runner settings `given`, and a short stop grace and store retry.
pub fn setup(engine: &Path, objects: PathBuf, out: PathBuf, given: &[(&str, &str)]) -> ContainerSetup {
    let mut setup = ContainerSetup::new(engine.to_path_buf(), objects, out);
    setup.passed = given
        .iter()
        .map(|(name, value)| RunnerSetting::new(name, *value).unwrap_or_else(|| panic!("{name} is a runner setting")))
        .collect();
    setup.stop_grace = Duration::from_secs(5);
    setup.store_again = Duration::from_millis(50);
    setup
}

/// An address on this machine as a container reaches it: the host's alias in place of loopback.
pub fn from_container(url: &str) -> String {
    url.replace("127.0.0.1", HOST_ALIAS).replace("localhost", HOST_ALIAS)
}

/// A stack whose runs go to containers, with its runners' listener on a port of its own that a container reaches.
pub struct Local {
    pub stack: RunStack<HttpWps>,
    _runners: JoinHandle<()>,
}

pub async fn local(pool: &PgPool, needed: &Needed, image: &str, given: &[(&str, &str)], seats: u32) -> Local {
    let (stack, listener) = local_unserved(pool, needed, image, given, seats).await;
    let runners = serve_runners(&stack.gate.app, listener);
    Local { stack, _runners: runners }
}

/// `local`'s stack with its runners' listener bound and not yet served, for a test whose API processes take turns
/// serving it.
pub async fn local_unserved(
    pool: &PgPool,
    needed: &Needed,
    image: &str,
    given: &[(&str, &str)],
    seats: u32,
) -> (RunStack<HttpWps>, TcpListener) {
    make_local_default(pool).await.expect("the default project");
    super::person(pool, PERSON).await;
    let routes = Routes::new(Url::parse(&needed.wps).expect("a URL"), RouteShape::Path).expect("routes");
    let http = HttpWps::new(HttpConfig::new(routes)).expect("an HTTP client");
    let world = Client::start(http, ClientConfig::default(), Arc::new(BlockingCompute)).await.expect("the live server");
    let slug = mint_ulid().to_string().to_lowercase();
    // On Linux the alias is the bridge, which a loopback bind does not answer; Docker Desktop forwards loopback.
    let bind = if cfg!(target_os = "linux") { "0.0.0.0:0" } else { "127.0.0.1:0" };
    let listener = TcpListener::bind(bind).await.expect("a port for the runners");
    let port = listener.local_addr().expect("its address").port();
    let mut config = RunConfig::local(&format!("http://{HOST_ALIAS}:{port}"), image);
    config.runner_wps_url = Some(from_container(&needed.wps));
    config.at_once = Seats(seats);
    // Two cores, so each runner gets two executor threads from its invoker (the suite runs several at once).
    config.cpu_millis = CpuMillis(2_000);
    // Long enough for a runner sharing the machine with the suite's others to stop, keep its recording and report.
    config.cancel_grace = Duration::from_secs(5);
    let objects = super::runstore_folder();
    let out = env::temp_dir().join(format!("studio-runner-out-{slug}"));
    let setup = setup(&needed.engine, objects.clone(), out, given);
    let runs = Runs::new(config, &needed.wps).container(runner_image(image), setup);
    let objects = RunObjects::local_folder(&objects).expect("the run store");
    let app = AppState::new(
        pool.clone(),
        super::gate_rules(),
        Deployment::Local,
        super::MODEL_LABEL,
        Arc::new(BlockingCompute),
        objects,
    )
    .with_runs(runs);
    let gate = GateState::new(app, world);
    let router = gate_routes(gate.clone());
    let site = made_site(&router, &format!("t-{}", &slug[slug.len() - 10..])).await;
    (RunStack { router, gate, site }, listener)
}

/// The runners' routes over `app`, served on `listener` for the rest of the test.
pub fn serve_runners(app: &AppState, listener: TcpListener) -> JoinHandle<()> {
    let served = runner_routes(app.clone());
    tokio::spawn(async move { axum::serve(listener, served).await.expect("the runners' listener") })
}

/// Which of a runner's requests a `DoorGate` holds: the pins it asks for first, as it makes its manifest, or the
/// manifest it then hands in.
pub enum DoorRequest {
    Pins,
    Manifest,
}

impl DoorRequest {
    /// The route of the request, as the runners' router names it.
    fn route(&self) -> &'static str {
        match self {
            Self::Pins => "/runner/v1/attempts/{run}/{n}",
            Self::Manifest => "/runner/v1/attempts/{run}/{n}/manifest",
        }
    }
}

/// Where each of a runner's requests of one kind is held, unanswered, until the test lets it on to the API: `arrived`
/// says one has come, and `release` lets one on. A request never let on never reaches the API, and its runner waits on
/// it.
pub struct DoorGate {
    held: DoorRequest,
    pub arrived: Notify,
    pub release: Notify,
}

impl DoorGate {
    pub fn holding(held: DoorRequest) -> Arc<Self> {
        Arc::new(Self { held, arrived: Notify::new(), release: Notify::new() })
    }
}

/// `local`'s stack, its runners' routes behind `gate`.
pub async fn local_holding(pool: &PgPool, needed: &Needed, image: &str, gate: Arc<DoorGate>) -> Local {
    let (stack, listener) = local_unserved(pool, needed, image, &[], 1).await;
    let held = runner_routes(stack.gate.app.clone()).layer(from_fn_with_state(gate, hold_at_gate));
    let runners = tokio::spawn(async move { axum::serve(listener, held).await.expect("the runners' listener") });
    Local { stack, _runners: runners }
}

async fn hold_at_gate(State(gate): State<Arc<DoorGate>>, request: Request, next: Next) -> Response {
    if request.extensions().get::<MatchedPath>().is_some_and(|route| route.as_str() == gate.held.route()) {
        gate.arrived.notify_one();
        gate.release.notified().await;
    }
    next.run(request).await
}

/// A Rectangle made at its defaults by Create site under `customer`, as a person makes one: the draft, its first
/// evaluation, the press and its execution.
pub async fn made_site(router: &Router, customer: &str) -> ScopePath {
    let (status, started) =
        send(router, "POST", "/api/template-drafts", Some(json!({ "template": "template/site/rectangle" }))).await;
    assert_eq!(status, StatusCode::CREATED, "{started}");
    let draft = started["draft"].as_str().expect("the draft's id").to_owned();
    until("the draft's first evaluation", || async {
        let (_, read) = send(router, "GET", &format!("/api/template-drafts/{draft}"), None).await;
        read["latest"]["state"] == "ready"
    })
    .await;
    let (_, read) = send(router, "GET", &format!("/api/template-drafts/{draft}"), None).await;
    let evaluation = read["latest"]["n"].as_u64().expect("the ready evaluation's number");
    let body =
        json!({ "customer": customer, "slug": "s", "name": format!("Runs {customer}"), "evaluation": evaluation });
    let (status, accepted) = send(router, "POST", &format!("/api/template-drafts/{draft}/site"), Some(body)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    // Create site answers its approval's record, as Apply and Run do (bd0f97b9e).
    let record: ApprovalRecord = serde_json::from_value(accepted["approval"].clone()).expect("the approval's record");
    let approval = record.id;
    until("Create site to land", || async {
        let (_, read) = send(router, "GET", &format!("/api/approvals/{approval}"), None).await;
        read["approval"]["execution"] == "ok"
    })
    .await;
    format!("customer/{customer}/site/s").parse().expect("the made site")
}

/// The run loops with a container invoker of their own, as one API process runs them, the invoker adopting first
/// what a stopped process left running; every wait short.
pub async fn loops(stack: &RunStack<HttpWps>) -> Running {
    loops_of(&stack.gate.app).await
}

pub async fn loops_of(app: &AppState) -> Running {
    let mut settings: LoopSettings = app.runs.loop_settings();
    settings.dispatch_every = Duration::from_millis(100);
    settings.watch_every = Duration::from_millis(100);
    settings.reconcile_every = Duration::from_millis(200);
    settings.lost_grace = Duration::from_millis(500);
    settings.create_grace = Duration::from_millis(200);
    let invoker = ContainerInvoker::new(app.clone(), app.runs.container_setup().cloned().expect("a container setup"));
    invoker.adopt().await.expect("the containers a stopped process left, adopted");
    Loops::new(
        Arc::new(PgRunStore::new(app.clone())),
        Arc::new(JournalEvents::new(app.clone())),
        Arc::new(invoker),
        settings,
        app.runs.wakes.clone(),
    )
    .spawn()
}

/// `count` runs of Peak hour submitted: one, or a sweep of orders per hour.
pub async fn submitted(stack: &RunStack<HttpWps>, name: &str, count: u32) -> Vec<Ulid> {
    let scenario = short_scenario(stack, name, SIM_MINUTES).await;
    let values: Vec<u32> = (0..count).map(|n| 100 + 50 * n).collect();
    let made = experiment(stack, &scenario, 1, if count > 1 { &values } else { &[] }).await;
    let (plan, hash) = plan(stack, &made.id).await;
    submit(stack, &plan, &hash).await;
    plan.runs.iter().map(|run| run.run.clone()).collect()
}

/// The latest attempt's state and the run's summary.
pub async fn run_of(stack: &RunStack<HttpWps>, run: &Ulid) -> Value {
    let (status, read) = send(&stack.router, "GET", &format!("/api/runs/{run}"), None).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    read
}

pub async fn state_is(stack: &RunStack<HttpWps>, run: &Ulid, state: &str) -> bool {
    run_of(stack, run).await["state"] == state
}

pub async fn cancel(stack: &RunStack<HttpWps>, run: &Ulid) -> Value {
    let (status, answer) = send(&stack.router, "POST", &format!("/api/runs/{run}/cancel"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{answer}");
    answer
}

/// The Job of `run`'s first attempt, which the attempt's key names before the dispatcher records it.
pub fn first_job(run: &Ulid) -> JobName {
    JobName::of(&AttemptKey { run: run.clone(), n: AttemptNumber(1) })
}

/// The host folder the container of `run`'s first attempt writes into, named by its Job.
pub fn out_of(stack: &RunStack<HttpWps>, run: &Ulid) -> PathBuf {
    stack.gate.app.runs.container_setup().expect("a setup").out.join(first_job(run).as_str())
}

/// The containers of some runs' first attempts, removed when the test that submitted the runs ends, passing or
/// failing: a watch removes a container once its runner exits, which a test that has read what it came for does not
/// wait for, so without this each run of the tests would leave its containers on the engine.
pub struct RemovedAtEnd {
    engine: PathBuf,
    jobs: Vec<JobName>,
}

impl RemovedAtEnd {
    pub fn of<'a>(engine: &Path, runs: impl IntoIterator<Item = &'a Ulid>) -> Self {
        Self { engine: engine.to_path_buf(), jobs: runs.into_iter().map(first_job).collect() }
    }
}

impl Drop for RemovedAtEnd {
    fn drop(&mut self) {
        for job in &self.jobs {
            // A teardown, which cannot fail its test: a container the watch removed already is gone either way.
            let _ = StdCommand::new(&self.engine).args(["rm", "-f", job.as_str()]).output();
        }
    }
}

/// Whether the engine says the container of `name` runs.
pub fn container_runs(engine: &Path, name: &str) -> bool {
    let answer = StdCommand::new(engine).args(["inspect", "-f", "{{.State.Running}}", name]).output();
    answer.is_ok_and(|answer| String::from_utf8_lossy(&answer.stdout).trim() == "true")
}

pub fn container_exists(engine: &Path, name: &str) -> bool {
    StdCommand::new(engine)
        .args(["inspect", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// A job request for an attempt of a run no row holds, running `image`, whose runner reaches no listener: what an
/// invoker is handed, without the loops.
pub fn request_alone(image: &str) -> JobRequest {
    let key = AttemptKey { run: mint_ulid(), n: AttemptNumber(1) };
    JobRequest {
        name: JobName::of(&key),
        image: image.to_owned(),
        spec: JobSpec {
            v: FORMAT,
            run: key.run.clone(),
            attempt: key.n,
            studio: Endpoint { endpoint: "http://127.0.0.1:9".to_owned() },
            streaming: None,
        },
        token: Token::mint(),
        deadline: Duration::from_secs(60),
        resources: Resources { cpu_millis: CpuMillis(1_000), memory_mib: None },
        labels: Default::default(),
    }
}
