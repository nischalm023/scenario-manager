//! Within `run-invocation`, `job` is one attempt's Job as the invoker sees it (spec 08 section 3.5): its name, the
//! request that creates it with the token that rides beside its spec, the settings a deployment hands every runner, the
//! status the invoker answers, and the `Invoker` trait itself, one Job in and one Job out (ruling D32).

use std::fmt::{self, Debug, Display, Formatter};
use std::future::Future;
use std::time::Duration;

use rustc_hash::FxHashMap;
use studio_protocol::{JobSpec, RUNNER_SETTINGS};

use crate::Fault;
use crate::settings::{CpuMillis, MemoryMib};
use crate::store::AttemptKey;
use crate::token::Token;

/// A Job's name, `run-{run ULID in lower case}-{n}`: a kube object name derived from our ULID, never an entity id, so
/// creating twice is not an error and no name repeats, attempt numbers only growing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobName(String);

impl JobName {
    pub fn of(key: &AttemptKey) -> Self {
        Self(format!("run-{}-{}", key.run.as_str().to_lowercase(), key.n.0))
    }

    /// A name as a store kept it.
    pub fn stored(name: String) -> Self {
        Self(name)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for JobName {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

/// The loops instance creating a Job, minted when the loops start. The reservation row carries it until every create
/// of the Job has returned (spec 08 section 3.4, step 5), and the reconciler's re-creation claims the row with it, so
/// one creator at a time owns a creation; a claim no running instance holds is a stopped process's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Creator(String);

impl Creator {
    pub fn mint() -> Self {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).expect("the operating system's random generator answers");
        Self(hex::encode(bytes))
    }

    /// A claim as a store kept it.
    pub fn stored(text: String) -> Self {
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A variable a deployment hands every runner by name, which is only ever one of the runner's settings
/// (`RUNNER_SETTINGS`): a runner's environment is its token, its thread count and these, so no name a configuration
/// gives can carry the API's database URL or a store credential to it (ruling D174).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerSetting {
    name: &'static str,
    value: String,
}

impl RunnerSetting {
    /// `name` with `value`; `None` when `name` is not one of the runner's settings.
    pub fn new(name: &str, value: impl Into<String>) -> Option<Self> {
        let name = RUNNER_SETTINGS.into_iter().find(|setting| *setting == name)?;
        Some(Self { name, value: value.into() })
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

/// A Job's requests, equal to its limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resources {
    pub cpu_millis: CpuMillis,
    pub memory_mib: Option<MemoryMib>,
}

/// What the dispatcher hands the invoker for one attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct JobRequest {
    pub name: JobName,
    pub image: String,
    pub spec: JobSpec,
    pub token: Token,
    /// The Job's active deadline, wall clock.
    pub deadline: Duration,
    pub resources: Resources,
    /// `agent-studio/run` and `agent-studio/attempt`, which the reconciler reads.
    pub labels: FxHashMap<String, String>,
}

/// What the invoker knows of a Job now: still pending (with the platform's reason), running, ended (with the pod's
/// termination reason or the process's exit), absent, or unknown when it cannot tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobStatus {
    Pending { reason: Option<String> },
    Running,
    Ended { reason: Option<String> },
    Absent,
    Unknown { reason: String },
}

impl JobStatus {
    /// Whether no pod of the Job runs, so its seat may free.
    pub fn stopped(&self) -> bool {
        matches!(self, Self::Ended { .. } | Self::Absent)
    }
}

/// A status as the attempt's row keeps it: `pending`, `running`, `ended`, `absent` or `unknown`, with the platform's
/// reason after a colon when it gave one.
impl Display for JobStatus {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        let (word, reason) = match self {
            Self::Pending { reason } => ("pending", reason.as_deref()),
            Self::Running => ("running", None),
            Self::Ended { reason } => ("ended", reason.as_deref()),
            Self::Absent => ("absent", None),
            Self::Unknown { reason } => ("unknown", Some(reason.as_str())),
        };
        match reason {
            Some(reason) => write!(out, "{word}: {reason}"),
            None => out.write_str(word),
        }
    }
}

/// One Job in, one Job out: `start` is idempotent by name, `stop` deletes the Job, `status` answers what runs.
pub trait Invoker: Send + Sync + 'static {
    fn start(&self, request: JobRequest) -> impl Future<Output = Result<(), Fault>> + Send;
    fn stop(&self, name: &JobName) -> impl Future<Output = Result<(), Fault>> + Send;
    fn status(&self, name: &JobName) -> impl Future<Output = Result<JobStatus, Fault>> + Send;
}

#[cfg(test)]
mod tests {
    use super::{JobStatus, RunnerSetting};

    /// Only a variable the runner reads and neither its invoker nor its image sets can be handed to it: never a store
    /// credential, a database URL under any name, its token or its build's identity (ruling D174).
    #[test]
    fn only_the_runners_own_settings_are_handed_to_it() {
        let heartbeat = RunnerSetting::new("DES_HEARTBEAT", "3600").expect("dessim's progress line");
        assert_eq!((heartbeat.name(), heartbeat.value()), ("DES_HEARTBEAT", "3600"));
        for refused in [
            "GOOGLE_SERVICE_ACCOUNT_KEY",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "STUDIO_DATABASE_URL",
            "DATABASE_URL",
            "PGPASSWORD",
            "CRUCIBLE_STORAGE_BUCKET",
            "STUDIO_RUNNER_TOKEN",
            "DESVIS_RUN_COMMIT",
        ] {
            assert_eq!(RunnerSetting::new(refused, "x"), None, "{refused} is no setting of the runner's");
        }
    }

    /// The words the attempt's row keeps, which the run page shows as the platform's view of the Job.
    #[test]
    fn a_status_says_itself_with_the_platforms_reason() {
        let pulling = JobStatus::Pending { reason: Some("ImagePullBackOff".to_owned()) };
        assert_eq!(pulling.to_string(), "pending: ImagePullBackOff");
        assert_eq!(JobStatus::Running.to_string(), "running");
        assert_eq!(JobStatus::Ended { reason: None }.to_string(), "ended");
        assert_eq!(
            JobStatus::Ended { reason: Some("DeadlineExceeded".to_owned()) }.to_string(),
            "ended: DeadlineExceeded"
        );
        assert_eq!(JobStatus::Absent.to_string(), "absent");
        assert_eq!(JobStatus::Unknown { reason: "no answer".to_owned() }.to_string(), "unknown: no answer");
    }
}
