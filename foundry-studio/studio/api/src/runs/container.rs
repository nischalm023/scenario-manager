//! Within `runs`, `container` is the local invoker (spec 08 section 3.5): one container of the runner image per Job
//! name, run by the machine's container engine (docker or podman) as a cluster runs a Job of it, so a laptop runs the
//! very image a cluster does. The attempt's job spec is mounted read-only from the run store's folder, the output
//! folder from `STUDIO_RUNNER_OUT_DIR`, so what the runner writes stays on the host; the token rides in an environment
//! file the engine reads and this process deletes, never on a command line. `start` is idempotent by name; `stop` asks
//! the engine's stop, SIGTERM then SIGKILL after the grace, which the runner catches as a platform termination, and
//! answers without waiting for it, one asked at a time per name, so a runner that ignores SIGTERM holds no loop;
//! `status` is what the engine says of the container, nothing kept here, so a second API process finds the same
//! containers. A started or adopted container's life, its deadline, its exit and its removal, is `watch`'s; each
//! command line is the engine's dialect (`engine`); what a runner left without reporting is `settle`'s.

mod engine;
mod settle;
mod watch;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use run_invocation::{AttemptKey, Fault, Invoker, JobName, JobRequest, JobStatus, RunnerSetting};
use runstore::RunStorage;
use runstore::layout::{AttemptNumber as StoredAttempt, RunId, attempt_prefix};
use rustc_hash::{FxHashMap, FxHashSet};
use studio_protocol::{THREADS_VAR, TOKEN_VAR};
use tokio::process::Command;

use crate::AppState;

pub use self::engine::{BakedImage, HOST_ALIAS, find_engine};

use self::engine::{absent, baked_of, engine_words, run_args, status_of};
use self::watch::{Deadline, Watched};

/// What the invoker is told: the engine, the two host folders, and how a runner is started and stopped. The image is
/// each attempt's own, the one its request names, so an attempt runs the image its plan pinned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerSetup {
    /// The engine's executable, `docker` or `podman`.
    pub engine: PathBuf,
    /// The run store's folder (`CRUCIBLE_STORAGE_DIR`), which holds each attempt's job spec.
    pub objects: PathBuf,
    /// Each container's output folder is made under this one, named by the Job, and mounted in.
    pub out: PathBuf,
    /// The settings every runner is given besides its token and thread count, and nothing else.
    pub passed: Vec<RunnerSetting>,
    /// Run as this `uid:gid` instead of the image's user: on Linux the mounted output folder is the host user's.
    pub user: Option<String>,
    /// The stack this API belongs to, which its containers carry as a label and the only ones it adopts carry, so
    /// stacks sharing an engine never take or stop each other's runners.
    pub stack: Option<String>,
    /// Whether `host.docker.internal` must be added to the container's hosts (Linux Docker; Desktop has it).
    pub host_alias: bool,
    /// Between the engine's SIGTERM and its SIGKILL (wall clock): 90 s, as a Job's termination grace.
    pub stop_grace: Duration,
    /// How long a failed store of a runner's results waits before it is asked again (wall clock): 15 s.
    pub store_again: Duration,
}

impl ContainerSetup {
    pub fn new(engine: PathBuf, objects: PathBuf, out: PathBuf) -> Self {
        Self {
            engine,
            objects,
            out,
            passed: Vec::new(),
            user: None,
            stack: None,
            host_alias: cfg!(target_os = "linux"),
            stop_grace: Duration::from_secs(90),
            store_again: Duration::from_secs(15),
        }
    }
}

/// The runner image `reference` names as `engine` knows it, read once at startup.
pub async fn inspect_image(engine: &Path, reference: &str) -> Result<BakedImage, String> {
    let asked = Command::new(engine).args(["image", "inspect", reference]).output().await;
    let answer = asked.map_err(|fault| format!("the engine {} does not run: {fault}", engine.display()))?;
    if !answer.status.success() {
        let words = engine_words(&answer);
        return Err(format!("the engine has no runner image {reference} ({words}): build it, as ./dev.sh start does"));
    }
    baked_of(&answer.stdout).map_err(|words| format!("the runner image {reference} does not say its build: {words}"))
}

#[derive(Clone)]
pub struct ContainerInvoker {
    app: AppState,
    setup: Arc<ContainerSetup>,
    watched: Arc<Mutex<FxHashMap<JobName, Watched>>>,
    stopping: Arc<Mutex<FxHashSet<JobName>>>,
}

impl ContainerInvoker {
    pub fn new(app: AppState, setup: Arc<ContainerSetup>) -> Self {
        Self { app, setup, watched: Arc::default(), stopping: Arc::default() }
    }

    fn engine(&self) -> Command {
        Command::new(&self.setup.engine)
    }

    /// `docker <args>`, its output; a failure to run the engine at all is a fault.
    async fn run(&self, args: &[&str]) -> Result<Output, Fault> {
        Ok(self.engine().args(args).output().await?)
    }
}

impl Invoker for ContainerInvoker {
    async fn start(&self, request: JobRequest) -> Result<(), Fault> {
        // The name goes in before the first await, so a second start of it, racing or late, finds it and starts
        // nothing; the engine refuses a second container of the name anyway.
        let deadline = Deadline::from_now(request.deadline);
        {
            let mut watched = self.watched.lock().expect("the watched containers' lock");
            if watched.contains_key(&request.name) {
                return Ok(());
            }
            watched.insert(request.name.clone(), Watched::new(deadline));
        }
        let started = self.create(&request, deadline).await;
        if started.is_err() {
            self.watched.lock().expect("the watched containers' lock").remove(&request.name);
        }
        started
    }

    async fn stop(&self, name: &JobName) -> Result<(), Fault> {
        if !self.stopping.lock().expect("the stopping containers' lock").insert(name.clone()) {
            return Ok(());
        }
        let (invoker, name) = (self.clone(), name.clone());
        tokio::spawn(async move {
            let grace = invoker.setup.stop_grace.as_secs().to_string();
            match invoker.run(&["stop", "-t", &grace, name.as_str()]).await {
                Ok(answer) if answer.status.success() || absent(&answer) => {}
                Ok(answer) => eprintln!("studio-api: {name}'s container was not stopped: {}", engine_words(&answer)),
                Err(fault) => eprintln!("studio-api: {name}'s container was not stopped: {fault}"),
            }
            invoker.stopping.lock().expect("the stopping containers' lock").remove(&name);
        });
        Ok(())
    }

    async fn status(&self, name: &JobName) -> Result<JobStatus, Fault> {
        let answer = self
            .run(&["inspect", "-f", "{{.State.Status}} {{.State.ExitCode}} {{.State.OOMKilled}}", name.as_str()])
            .await?;
        if absent(&answer) {
            return Ok(JobStatus::Absent);
        }
        if !answer.status.success() {
            return Err(engine_words(&answer).into());
        }
        let status = status_of(&String::from_utf8_lossy(&answer.stdout));
        // An exit this process is still settling reads as pending, so the reconciler ends no attempt on the bare
        // exit status before what the runner left is read.
        let settling =
            status.stopped() && self.watched.lock().expect("the watched containers' lock").contains_key(name);
        Ok(if settling {
            JobStatus::Pending { reason: Some("exited; its exit is being settled".to_owned()) }
        } else {
            status
        })
    }
}

impl ContainerInvoker {
    /// The container for `request`, its name already claimed: its job spec written, its folder emptied, the
    /// container started detached with its token in an environment file and `deadline` in its labels, and watched.
    async fn create(&self, request: &JobRequest, deadline: Deadline) -> Result<(), Fault> {
        let key = AttemptKey { run: request.spec.run.clone(), n: request.spec.attempt };
        let prefix = attempt_prefix(&RunId::new(key.run.as_str())?, StoredAttempt::new(key.n.0)?);
        self.app
            .runstore
            .put(&format!("{prefix}job.json"), Bytes::from(serde_json::to_vec_pretty(&request.spec)?))
            .await?;
        let out = self.setup.out.join(request.name.as_str());
        if out.exists() {
            fs::remove_dir_all(&out)?;
        }
        fs::create_dir_all(&out)?;
        let environment = self.setup.out.join(format!("{}.env", request.name));
        let mut file = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&environment)?;
        let cores = request.resources.cpu_millis.0.div_ceil(1_000);
        writeln!(file, "{TOKEN_VAR}={}", request.token.expose())?;
        writeln!(file, "{THREADS_VAR}={cores}")?;
        for setting in &self.setup.passed {
            writeln!(file, "{}={}", setting.name(), setting.value())?;
        }
        drop(file);
        let args = run_args(&self.setup, request, &deadline, &self.setup.objects.join(prefix), &out, &environment);
        let answer = self.engine().args(&args).output().await;
        let _ = fs::remove_file(&environment);
        let answer = answer?;
        if !answer.status.success() && !String::from_utf8_lossy(&answer.stderr).contains("is already in use") {
            return Err(format!("the engine would not run the runner image: {}", engine_words(&answer)).into());
        }
        self.watch(request.name.clone(), key, deadline);
        Ok(())
    }
}
