//! Within `config`, `runs` is spec 08 section 6's run settings, read at startup: the seats held at once across the
//! deployment, a Job's wall-clock deadline and resources, how long a runner may take to start and a cancel to be
//! confirmed, where runners reach the runners' listener and world-proto-server, the runner image every attempt runs;
//! locally, the container engine, the folder each container's output is mounted from and the stack its containers are
//! labelled with; in a cluster, the namespace and service account Jobs run in; in both, the runner settings
//! `STUDIO_RUNNER_PASSED` names, the only variables of the API's own environment a runner is given, so that no database
//! URL or store credential can reach it whatever a configuration names (ruling D174: the simulation runner has no
//! access to Postgres).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use run_invocation::{CpuMillis, MemoryMib, RunnerSetting, Seats};
use studio_protocol::RUNNER_SETTINGS;

use super::{ConfigError, Deployment, Vars};

pub const RUNS_AT_ONCE_VAR: &str = "STUDIO_RUNS_AT_ONCE";
pub const RUN_DEADLINE_SECS_VAR: &str = "STUDIO_RUN_DEADLINE_SECS";
pub const START_TIMEOUT_SECS_VAR: &str = "STUDIO_START_TIMEOUT_SECS";
pub const CANCEL_GRACE_SECS_VAR: &str = "STUDIO_CANCEL_GRACE_SECS";
pub const STREAMING_STALE_SECS_VAR: &str = "STUDIO_STREAMING_STALE_SECS";
pub const RUNNER_CPU_MILLIS_VAR: &str = "STUDIO_RUNNER_CPU_MILLIS";
pub const RUNNER_MEMORY_MIB_VAR: &str = "STUDIO_RUNNER_MEMORY_MIB";
pub const RUNNER_URL_VAR: &str = "STUDIO_RUNNER_URL";
pub const RUNNER_WPS_URL_VAR: &str = "STUDIO_RUNNER_WPS_URL";
pub const RUNNER_OUT_DIR_VAR: &str = "STUDIO_RUNNER_OUT_DIR";
pub const CONTAINER_ENGINE_VAR: &str = "STUDIO_CONTAINER_ENGINE";
pub const CONTAINER_USER_VAR: &str = "STUDIO_CONTAINER_USER";
pub const CONTAINER_STACK_VAR: &str = "STUDIO_CONTAINER_STACK";
pub const RUNNER_IMAGE_VAR: &str = "STUDIO_RUNNER_IMAGE";
pub const RUNNER_NAMESPACE_VAR: &str = "STUDIO_RUNNER_NAMESPACE";
pub const RUNNER_SERVICE_ACCOUNT_VAR: &str = "STUDIO_RUNNER_SERVICE_ACCOUNT";
pub const RUNNER_PASSED_VAR: &str = "STUDIO_RUNNER_PASSED";
pub const BUILD_BRANCH_VAR: &str = "STUDIO_BUILD_BRANCH";

/// The cluster-only variables, each refused with `local`, where runners are containers of this machine's engine.
const CLUSTER_ONLY: [&str; 2] = [RUNNER_NAMESPACE_VAR, RUNNER_SERVICE_ACCOUNT_VAR];
/// The local-only variables, each refused with `cluster`, where runners are Jobs.
const LOCAL_ONLY: [&str; 4] = [RUNNER_OUT_DIR_VAR, CONTAINER_ENGINE_VAR, CONTAINER_USER_VAR, CONTAINER_STACK_VAR];
/// Variables of the process invoker, gone with it; a configuration still naming one is refused with the words.
const GONE: [(&str, &str); 1] =
    [("STUDIO_RUNNER_BIN", "runners are containers of STUDIO_RUNNER_IMAGE now, not processes of a binary")];

/// What the run loops and the local invoker are told; every duration is wall clock.
#[derive(Debug, Clone, PartialEq)]
pub struct RunConfig {
    pub at_once: Seats,
    pub deadline: Duration,
    pub start_timeout: Duration,
    pub cancel_grace: Duration,
    /// A streaming server whose heartbeat is older than this is dead, and no attempt is pointed at it.
    pub streaming_stale_after: Duration,
    pub cpu_millis: CpuMillis,
    pub memory_mib: Option<MemoryMib>,
    /// The runners' listener as a runner reaches it, written into every job spec.
    pub runner_url: String,
    /// world-proto-server as a runner reaches it, written into every attempt's pins; unset, the API's own.
    pub runner_wps_url: Option<String>,
    /// Locally, the folder under which each container gets an empty output folder of its own, mounted in.
    pub runner_out: Option<PathBuf>,
    /// Locally, the container engine's executable; unset, `docker` then `podman` on `PATH`.
    pub container_engine: Option<PathBuf>,
    /// Locally, the `uid:gid` a container runs as, where the image's user could not write the mounted folder.
    pub container_user: Option<String>,
    /// Locally, the stack this API belongs to, which labels its containers so that stack's dev.sh stops only its own.
    pub container_stack: Option<String>,
    /// The runner image every attempt runs, locally a container of it, in a cluster a Job; the one image plans name.
    pub runner_image: String,
    /// In a cluster, the branch this API's image and the runner image were built from, which the image sets.
    pub build_branch: Option<String>,
    /// In a cluster, the namespace Jobs are created in; unset, the API pod's own.
    pub runner_namespace: Option<String>,
    /// In a cluster, the service account a runner pod runs as; unset, the namespace's default.
    pub runner_service_account: Option<String>,
    /// The runner settings copied from the API's own environment into every runner, container or pod: each name
    /// `STUDIO_RUNNER_PASSED` lists, with its value here.
    pub runner_passed: Vec<RunnerSetting>,
}

impl RunConfig {
    /// Spec 08 section 6's defaults for a local service whose runners reach it at `runner_url` and run `runner_image`.
    pub fn local(runner_url: &str, runner_image: &str) -> Self {
        Self {
            at_once: Seats(5),
            deadline: Duration::from_secs(21_600),
            start_timeout: Duration::from_secs(900),
            cancel_grace: Duration::from_secs(60),
            streaming_stale_after: Duration::from_secs(10),
            cpu_millis: CpuMillis(1_000),
            memory_mib: None,
            runner_url: runner_url.to_owned(),
            runner_wps_url: None,
            runner_out: None,
            container_engine: None,
            container_user: None,
            container_stack: None,
            runner_image: runner_image.to_owned(),
            build_branch: None,
            runner_namespace: None,
            runner_service_account: None,
            runner_passed: Vec::new(),
        }
    }
}

pub(super) fn read(vars: &Vars<'_>, deployment: Deployment, runner_bind: SocketAddr) -> Result<RunConfig, ConfigError> {
    let seconds = |var, default| vars.read_or(var, Duration::from_secs(default), whole_seconds);
    let runner_url = match (vars.stated(RUNNER_URL_VAR)?, deployment) {
        (Some(url), _) => vars.read(RUNNER_URL_VAR, &url, web_address)?,
        (None, Deployment::Local) => format!("http://{runner_bind}"),
        (None, Deployment::Cluster) => {
            return Err(ConfigError::RequiredWhen { var: RUNNER_URL_VAR, when: "STUDIO_ENV is cluster" });
        }
    };
    for (var, why) in GONE {
        if vars.stated(var)?.is_some() {
            return Err(ConfigError::Gone { var, why });
        }
    }
    if deployment == Deployment::Cluster {
        for var in LOCAL_ONLY {
            if vars.stated(var)?.is_some() {
                return Err(ConfigError::RefusedInCluster { var, why: "a cluster runs its runners as Jobs" });
            }
        }
    } else {
        for var in CLUSTER_ONLY {
            if vars.stated(var)?.is_some() {
                return Err(ConfigError::RefusedLocally { var, why: "a local service runs its runners as containers" });
            }
        }
    }
    Ok(RunConfig {
        at_once: vars.read_or(RUNS_AT_ONCE_VAR, Seats(5), |text| at_least(text, 1, "seats").map(Seats))?,
        deadline: seconds(RUN_DEADLINE_SECS_VAR, 21_600)?,
        start_timeout: seconds(START_TIMEOUT_SECS_VAR, 900)?,
        cancel_grace: seconds(CANCEL_GRACE_SECS_VAR, 60)?,
        streaming_stale_after: seconds(STREAMING_STALE_SECS_VAR, 10)?,
        cpu_millis: vars.read_or(RUNNER_CPU_MILLIS_VAR, CpuMillis(1_000), |text| {
            at_least(text, 1_000, "millicores").map(CpuMillis)
        })?,
        memory_mib: vars.read_or(RUNNER_MEMORY_MIB_VAR, None, |text| {
            at_least(text, 1, "mebibytes").map(|mib| Some(MemoryMib(mib)))
        })?,
        runner_url,
        runner_wps_url: vars.read_or(RUNNER_WPS_URL_VAR, None, |text| web_address(text).map(Some))?,
        runner_out: vars.stated(RUNNER_OUT_DIR_VAR)?.map(PathBuf::from),
        container_engine: vars.stated(CONTAINER_ENGINE_VAR)?.map(PathBuf::from),
        container_user: vars.stated(CONTAINER_USER_VAR)?,
        container_stack: vars.stated(CONTAINER_STACK_VAR)?,
        runner_image: vars.required(RUNNER_IMAGE_VAR)?,
        build_branch: vars.stated(BUILD_BRANCH_VAR)?,
        runner_namespace: vars.stated(RUNNER_NAMESPACE_VAR)?,
        runner_service_account: vars.stated(RUNNER_SERVICE_ACCOUNT_VAR)?,
        runner_passed: passed(vars)?,
    })
}

/// The runner settings `STUDIO_RUNNER_PASSED` names, each with its value here; a name that is no runner setting, or
/// one this environment does not set, refuses the start.
fn passed(vars: &Vars<'_>) -> Result<Vec<RunnerSetting>, ConfigError> {
    let Some(text) = vars.stated(RUNNER_PASSED_VAR)? else { return Ok(Vec::new()) };
    let mut passed = Vec::new();
    for name in vars.read(RUNNER_PASSED_VAR, &text, settings)? {
        let Some(value) = vars.named(name)? else {
            let expected = format!("names of variables this environment sets, which {name} is not");
            return Err(vars.unreadable(RUNNER_PASSED_VAR, &text, expected));
        };
        passed.push(RunnerSetting::new(name, value).expect("one of the runner's settings"));
    }
    Ok(passed)
}

/// `STUDIO_RUNNER_PASSED`: names joined by `,`, each one of the runner's settings, none of which carries the API's
/// database or a store credential (ruling D174).
fn settings(text: &str) -> Result<Vec<&'static str>, String> {
    let named = text.split(',').map(|name| RUNNER_SETTINGS.into_iter().find(|setting| *setting == name.trim()));
    let named: Option<Vec<&'static str>> = named.collect();
    named.ok_or_else(|| format!("runner settings joined by `,`, each one of {}", RUNNER_SETTINGS.join(", ")))
}

fn whole_seconds(text: &str) -> Result<Duration, String> {
    at_least(text, 1, "seconds").map(|seconds| Duration::from_secs(u64::from(seconds)))
}

fn at_least(text: &str, least: u32, what: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .ok()
        .filter(|value| *value >= least)
        .ok_or_else(|| format!("a whole number of {what}, {least} or more"))
}

fn web_address(text: &str) -> Result<String, String> {
    let web = (text.starts_with("http://") || text.starts_with("https://")) && text.len() > "https://".len();
    if web { Ok(text.trim_end_matches('/').to_owned()) } else { Err("an http or https address".to_owned()) }
}
