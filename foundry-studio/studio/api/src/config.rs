//! Within `studio-api`, `config` is what the service is told before it serves: the variables of spec 01 section 3.1
//! that this build reads, plus spec 04 section 3.12's three WPS variables, read once at startup; the model's settings
//! are its `model` part, the agent loop's budgets and waits its `agent` part, and spec 08 section 6's run settings its
//! `runs` part. A value that does not parse, or a
//! combination spec 01 forbids, refuses the start instead of being replaced by a guess, as crucible's config does. The
//! crate's README lists every variable with its default.

use std::env::{self, VarError};
use std::fmt::{self, Debug, Display, Formatter};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::str::FromStr;

use axum::http::Uri;
use rustc_hash::FxHashSet;
use studio_protocol::Subject;
use thiserror::Error;

use crate::compute::ComputeBudget;
use crate::identity::{DevIdentity, IdentityHeaders, IdentityRules};

mod agent;
mod model;
mod runs;

pub use agent::{
    AgentSettings, COMPACT_AT_INPUT_TOKENS_VAR, CONVERSATION_MAX_INPUT_TOKENS_VAR, ModelCalls, SERVICE_SUBJECTS_VAR,
    TOOL_TIMEOUT_SECS_VAR, TURN_MAX_ACTIVE_SECS_VAR, TURN_MAX_MODEL_CALLS_VAR, TURN_MAX_OUTPUT_TOKENS_VAR,
    TURNS_AT_ONCE_VAR, Tokens, TurnsAtOnce, WAIT_TTL_HOURS_VAR,
};
pub use model::{Effort, GEMINI_API_KEY_FILE_VAR, Model, ModelTransport, OutputTokens, ThinkingDisplay};
pub use runs::{
    BUILD_BRANCH_VAR, CANCEL_GRACE_SECS_VAR, CONTAINER_ENGINE_VAR, CONTAINER_STACK_VAR, CONTAINER_USER_VAR,
    RUN_DEADLINE_SECS_VAR, RUNNER_CPU_MILLIS_VAR, RUNNER_IMAGE_VAR, RUNNER_MEMORY_MIB_VAR, RUNNER_NAMESPACE_VAR,
    RUNNER_OUT_DIR_VAR, RUNNER_PASSED_VAR, RUNNER_SERVICE_ACCOUNT_VAR, RUNNER_URL_VAR, RUNNER_WPS_URL_VAR,
    RUNS_AT_ONCE_VAR, RunConfig, START_TIMEOUT_SECS_VAR, STREAMING_STALE_SECS_VAR,
};

pub const BIND_VAR: &str = "STUDIO_BIND";
pub const RUNNER_BIND_VAR: &str = "STUDIO_RUNNER_BIND";
pub const DATABASE_URL_VAR: &str = "STUDIO_DATABASE_URL";
pub const ENV_VAR: &str = "STUDIO_ENV";
pub const IDENTITY_HEADERS_VAR: &str = "STUDIO_IDENTITY_HEADERS";
pub const DEVELOPER_ROLE_VAR: &str = "STUDIO_DEVELOPER_ROLE";
pub const DEVELOPERS_VAR: &str = "STUDIO_DEVELOPERS";
pub const DEV_IDENTITY_VAR: &str = "STUDIO_DEV_IDENTITY";
pub const WPS_URL_VAR: &str = "STUDIO_WPS_URL";
pub const WPS_ROUTES_VAR: &str = "STUDIO_WPS_ROUTES";
pub const WPS_REVISION_VAR: &str = "STUDIO_WPS_REVISION";
pub const MODEL_TRANSPORT_VAR: &str = "STUDIO_MODEL_TRANSPORT";
pub const MODEL_ID_VAR: &str = "STUDIO_MODEL_ID";
pub const MODEL_LABEL_VAR: &str = "STUDIO_MODEL_LABEL";
pub const VERTEX_PROJECT_VAR: &str = "STUDIO_VERTEX_PROJECT";
pub const VERTEX_REGION_VAR: &str = "STUDIO_VERTEX_REGION";
pub const ANTHROPIC_API_KEY_VAR: &str = "ANTHROPIC_API_KEY";
pub const MODEL_EFFORT_VAR: &str = "STUDIO_MODEL_EFFORT";
pub const MODEL_THINKING_DISPLAY_VAR: &str = "STUDIO_MODEL_THINKING_DISPLAY";
pub const MODEL_MAX_TOKENS_VAR: &str = "STUDIO_MODEL_MAX_TOKENS";
pub const COMPUTE_WORKERS_VAR: &str = "STUDIO_COMPUTE_WORKERS";
pub const COMPUTE_QUEUE_VAR: &str = "STUDIO_COMPUTE_QUEUE";

/// The defaults spec 01 section 3.1 states, both listeners on loopback; WPS's is the address world-proto-server
/// answers on by default (`127.0.0.1:7413`), for local work only.
const DEFAULT_BIND: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 3100));
const DEFAULT_RUNNER_BIND: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 3101));
const DEFAULT_DEVELOPER_ROLE: &str = "studio-developer";
const DEFAULT_LOCAL_WPS_URL: &str = "http://127.0.0.1:7413";

/// What the service was told at startup.
#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub runner_bind: SocketAddr,
    pub database_url: Secret,
    pub deployment: Deployment,
    pub identity: IdentityRules,
    pub wps: Wps,
    pub model: Model,
    pub compute: ComputeBudget,
    pub runs: RunConfig,
    pub agent: AgentSettings,
}

/// A variable the service refuses to start with, or one it lacks.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("{0} is not set, and the service cannot start without it")]
    Missing(&'static str),
    #[error("{0} is set but blank; unset it or give it a value")]
    Blank(&'static str),
    #[error("{0} does not hold text")]
    NotText(String),
    #[error("{var} is {value:?}, which is not {expected}")]
    Unreadable { var: &'static str, value: String, expected: String },
    #[error("{var} must be set when {when}")]
    RequiredWhen { var: &'static str, when: &'static str },
    #[error("{var} cannot be used with STUDIO_ENV=cluster: {why}")]
    RefusedInCluster { var: &'static str, why: &'static str },
    #[error("{var} cannot be used with STUDIO_ENV=local: {why}")]
    RefusedLocally { var: &'static str, why: &'static str },
    #[error("{var} is no longer read: {why}")]
    Gone { var: &'static str, why: &'static str },
}

/// Where the service runs: `local`, a developer's machine, or `cluster`, a deployment behind the login gate. It
/// decides which identity source and which model transport may be used (spec 01 section 3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deployment {
    Local,
    Cluster,
}

/// Where world-proto-server answers, how its scope routes are spelled, and the world-proto revision it was built
/// from when dev.sh says so (`STUDIO_WPS_REVISION`), which the client's start-up probe holds the server to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wps {
    pub url: Uri,
    pub routes: WpsRoutes,
    pub revision: Option<String>,
}

/// How WPS spells a scope's routes (spec 04 section 3.12): `path`, `/scopes/<path>/-/<operation>`, the one shape
/// world-proto-server serves since c384e59, which answers the older spelling with 404. `STUDIO_WPS_ROUTES` keeps
/// this one value, so a configuration naming another shape refuses the start rather than meet those 404s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WpsRoutes {
    Path,
}

/// A value that must never reach a log or an error message, such as a key or a connection string with its password.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl Debug for Secret {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str("Secret(..)")
    }
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(|var| match env::var(var) {
            Ok(value) => Ok(Some(value)),
            Err(VarError::NotPresent) => Ok(None),
            Err(VarError::NotUnicode(_)) => Err(ConfigError::NotText(var.to_owned())),
        })
    }

    /// Read the variables `lookup` answers: the environment's, or a test's.
    pub fn from_vars(lookup: impl Fn(&str) -> Result<Option<String>, ConfigError>) -> Result<Self, ConfigError> {
        let vars = Vars(&lookup);
        let deployment = vars.read(ENV_VAR, &vars.required(ENV_VAR)?, |text| text.parse())?;
        if deployment == Deployment::Cluster {
            let never_in_cluster = [
                (DEVELOPERS_VAR, "developers come only from the gate's roles there"),
                (DEV_IDENTITY_VAR, "identity comes only from the gate's headers there"),
            ];
            for (var, why) in never_in_cluster {
                if vars.stated(var)?.is_some() {
                    return Err(ConfigError::RefusedInCluster { var, why });
                }
            }
        }
        let wps_url = match (vars.stated(WPS_URL_VAR)?, deployment) {
            (Some(url), _) => url,
            (None, Deployment::Local) => DEFAULT_LOCAL_WPS_URL.to_owned(),
            (None, Deployment::Cluster) => {
                return Err(ConfigError::RequiredWhen { var: WPS_URL_VAR, when: "STUDIO_ENV is cluster" });
            }
        };
        let runner_bind = vars.read_or(RUNNER_BIND_VAR, DEFAULT_RUNNER_BIND, socket_address)?;
        Ok(Self {
            bind: vars.read_or(BIND_VAR, DEFAULT_BIND, socket_address)?,
            runner_bind,
            database_url: Secret(vars.required(DATABASE_URL_VAR)?),
            deployment,
            identity: IdentityRules {
                headers: vars.read_or(IDENTITY_HEADERS_VAR, IdentityHeaders::default(), |text| text.parse())?,
                developer_role: vars.stated(DEVELOPER_ROLE_VAR)?.unwrap_or_else(|| DEFAULT_DEVELOPER_ROLE.to_owned()),
                developers: vars.read_or(DEVELOPERS_VAR, FxHashSet::default(), subjects)?,
                dev_identity: vars.read_or(DEV_IDENTITY_VAR, None, |text| text.parse::<DevIdentity>().map(Some))?,
            },
            wps: Wps {
                url: vars.read(WPS_URL_VAR, &wps_url, wps_url_of)?,
                routes: vars.read_or(WPS_ROUTES_VAR, WpsRoutes::Path, |text| text.parse())?,
                revision: vars.stated(WPS_REVISION_VAR)?,
            },
            model: model::read(&vars, deployment)?,
            compute: ComputeBudget {
                workers: vars.read_or(COMPUTE_WORKERS_VAR, ComputeBudget::default().workers, |text| text.parse())?,
                queue: vars.read_or(COMPUTE_QUEUE_VAR, ComputeBudget::default().queue, |text| text.parse())?,
            },
            runs: runs::read(&vars, deployment, runner_bind)?,
            agent: agent::read(&vars)?,
        })
    }
}

/// The variables as one lookup answers them, each trimmed. A variable set but blank is a mistake, never an absence:
/// whoever wrote its name meant to give it a value.
struct Vars<'a>(&'a dyn Fn(&str) -> Result<Option<String>, ConfigError>);

impl Vars<'_> {
    /// A variable another one names, such as one handed on to every runner, as it is set: untrimmed, blank or not.
    fn named(&self, var: &str) -> Result<Option<String>, ConfigError> {
        (self.0)(var)
    }

    fn stated(&self, var: &'static str) -> Result<Option<String>, ConfigError> {
        match (self.0)(var)? {
            Some(value) if value.trim().is_empty() => Err(ConfigError::Blank(var)),
            Some(value) => Ok(Some(value.trim().to_owned())),
            None => Ok(None),
        }
    }

    fn required(&self, var: &'static str) -> Result<String, ConfigError> {
        self.stated(var)?.ok_or(ConfigError::Missing(var))
    }

    fn required_when(&self, var: &'static str, when: &'static str) -> Result<String, ConfigError> {
        self.stated(var)?.ok_or(ConfigError::RequiredWhen { var, when })
    }

    /// `text`, the value of `var`, read by `parse`, whose error says what the value should have been.
    fn read<T>(
        &self,
        var: &'static str,
        text: &str,
        parse: impl Fn(&str) -> Result<T, String>,
    ) -> Result<T, ConfigError> {
        parse(text).map_err(|expected| self.unreadable(var, text, expected))
    }

    fn read_or<T>(
        &self,
        var: &'static str,
        default: T,
        parse: impl Fn(&str) -> Result<T, String>,
    ) -> Result<T, ConfigError> {
        self.stated(var)?.map_or(Ok(default), |text| self.read(var, &text, parse))
    }

    fn unreadable(&self, var: &'static str, value: &str, expected: impl Display) -> ConfigError {
        ConfigError::Unreadable { var, value: value.to_owned(), expected: expected.to_string() }
    }
}

/// `STUDIO_DEVELOPERS`: subjects joined by `,`, each a developer while `STUDIO_ENV` is local.
fn subjects(text: &str) -> Result<FxHashSet<Subject>, String> {
    let subjects = text.split(',').map(|subject| subject.trim().parse::<Subject>());
    subjects.collect::<Result<_, _>>().map_err(|_| "subjects joined by `,`, none of them blank".to_owned())
}

fn socket_address(text: &str) -> Result<SocketAddr, String> {
    text.parse().map_err(|_| "an address and port, such as 127.0.0.1:3100".to_owned())
}

/// WPS's base address: `http` or `https`, a host and maybe a port, as `http://127.0.0.1:7413`.
fn wps_url_of(text: &str) -> Result<Uri, String> {
    let expected = || "an http or https address of world-proto-server, such as http://127.0.0.1:7413".to_owned();
    let url: Uri = text.parse().map_err(|_| expected())?;
    let web = matches!(url.scheme_str(), Some("http" | "https")) && url.authority().is_some();
    if web { Ok(url) } else { Err(expected()) }
}

impl FromStr for Deployment {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "local" => Ok(Self::Local),
            "cluster" => Ok(Self::Cluster),
            _ => Err("local or cluster".to_owned()),
        }
    }
}

impl FromStr for WpsRoutes {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "path" => Ok(Self::Path),
            _ => Err("path, the one route shape world-proto-server serves since c384e59".to_owned()),
        }
    }
}
