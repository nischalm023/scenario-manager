//! Within `config`, `agent` is the agent loop's settings (spec 01 section 3.1), read at startup: the per-turn budgets
//! (model calls, output tokens and wall-clock seconds spent running, waits excluded), the request sizes at which the
//! loop summarises earlier turns and past which a conversation takes no new turn, how many turns run at once in this
//! process, a tool's default wall-clock timeout, how long a question or an approval stays open, and the subjects whose
//! turns are always unattended.

use std::time::Duration;

pub use agent_loop::Tokens;
use rustc_hash::FxHashSet;
use studio_protocol::Subject;

use super::{ConfigError, Vars};

pub const TURN_MAX_MODEL_CALLS_VAR: &str = "STUDIO_TURN_MAX_MODEL_CALLS";
pub const TURN_MAX_OUTPUT_TOKENS_VAR: &str = "STUDIO_TURN_MAX_OUTPUT_TOKENS";
pub const TURN_MAX_ACTIVE_SECS_VAR: &str = "STUDIO_TURN_MAX_ACTIVE_SECS";
pub const COMPACT_AT_INPUT_TOKENS_VAR: &str = "STUDIO_COMPACT_AT_INPUT_TOKENS";
pub const CONVERSATION_MAX_INPUT_TOKENS_VAR: &str = "STUDIO_CONVERSATION_MAX_INPUT_TOKENS";
pub const TURNS_AT_ONCE_VAR: &str = "STUDIO_TURNS_AT_ONCE";
pub const TOOL_TIMEOUT_SECS_VAR: &str = "STUDIO_TOOL_TIMEOUT_SECS";
pub const WAIT_TTL_HOURS_VAR: &str = "STUDIO_WAIT_TTL_HOURS";
pub const SERVICE_SUBJECTS_VAR: &str = "STUDIO_SERVICE_SUBJECTS";

/// A number of model requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModelCalls(pub u32);

/// A number of turns running at once in one process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TurnsAtOnce(pub u32);

/// What the agent loop is told; every duration is wall clock.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentSettings {
    pub max_model_calls: ModelCalls,
    pub max_output_tokens: Tokens,
    pub max_active: Duration,
    pub compact_at: Tokens,
    pub conversation_max: Tokens,
    pub turns_at_once: TurnsAtOnce,
    pub tool_timeout: Duration,
    pub wait_ttl: Duration,
    pub service_subjects: FxHashSet<Subject>,
}

impl Default for AgentSettings {
    /// Spec 01 section 3.1's defaults.
    fn default() -> Self {
        Self {
            max_model_calls: ModelCalls(40),
            max_output_tokens: Tokens(150_000),
            max_active: Duration::from_secs(900),
            compact_at: Tokens(300_000),
            conversation_max: Tokens(600_000),
            turns_at_once: TurnsAtOnce(16),
            tool_timeout: Duration::from_secs(120),
            wait_ttl: Duration::from_secs(24 * 3_600),
            service_subjects: FxHashSet::default(),
        }
    }
}

pub(super) fn read(vars: &Vars<'_>) -> Result<AgentSettings, ConfigError> {
    let default = AgentSettings::default();
    let whole = |text: &str| text.parse::<u64>().ok().filter(|value| *value >= 1).ok_or("a whole number, 1 or more");
    let number = |var, fallback: u64| vars.read_or(var, fallback, |text| whole(text).map_err(str::to_owned));
    let calls = number(TURN_MAX_MODEL_CALLS_VAR, u64::from(default.max_model_calls.0))?;
    let turns = number(TURNS_AT_ONCE_VAR, u64::from(default.turns_at_once.0))?;
    let narrow =
        |var, value: u64| u32::try_from(value).map_err(|_| vars.unreadable(var, &value.to_string(), "smaller"));
    let compact_at = Tokens(number(COMPACT_AT_INPUT_TOKENS_VAR, default.compact_at.0)?);
    let conversation_max = Tokens(number(CONVERSATION_MAX_INPUT_TOKENS_VAR, default.conversation_max.0)?);
    if conversation_max < compact_at {
        let why = "at least STUDIO_COMPACT_AT_INPUT_TOKENS, which compaction must fit under";
        return Err(vars.unreadable(CONVERSATION_MAX_INPUT_TOKENS_VAR, &conversation_max.0.to_string(), why));
    }
    Ok(AgentSettings {
        max_model_calls: ModelCalls(narrow(TURN_MAX_MODEL_CALLS_VAR, calls)?),
        max_output_tokens: Tokens(number(TURN_MAX_OUTPUT_TOKENS_VAR, default.max_output_tokens.0)?),
        max_active: Duration::from_secs(number(TURN_MAX_ACTIVE_SECS_VAR, default.max_active.as_secs())?),
        compact_at,
        conversation_max,
        turns_at_once: TurnsAtOnce(narrow(TURNS_AT_ONCE_VAR, turns)?),
        tool_timeout: Duration::from_secs(number(TOOL_TIMEOUT_SECS_VAR, default.tool_timeout.as_secs())?),
        wait_ttl: Duration::from_secs(3_600 * number(WAIT_TTL_HOURS_VAR, 24)?),
        service_subjects: vars.read_or(SERVICE_SUBJECTS_VAR, FxHashSet::default(), super::subjects)?,
    })
}
