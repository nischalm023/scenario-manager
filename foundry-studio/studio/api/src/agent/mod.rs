//! Within `studio-api`, `agent` is the agent loop's server half (spec 01 sections 3.4 to 3.10; brief I1.03 section 2):
//! a person's message admitted as a turn (`admit`), the turn carried out by its task (`turn`), each model request with
//! its retries streaming into the hub and the journal (`step`), the tool calls run through the registry, an asking
//! call pausing the turn on an approval and `ask_user` on a question (`calls`), the answers and decisions that resume
//! it (`cards`), Stop, the turn's calls and the endings every path shares (`end`), the transcript the model reads
//! with its context notes and compaction (`transcript`), and the test writer's stand-in for the loop (`follow`).
//! Every transition commits its row change with its journal entries in one transaction, its locks taken in one order
//! (`locks`). One API process runs a turn (LEAD-DECISIONS entry 54's pattern): its task holds the turn under a lease and
//! one of the process's `STUDIO_TURNS_AT_ONCE` places (`permit`), and a start interrupts what a stopped process left
//! running.

mod admit;
mod calls;
mod cards;
mod end;
mod follow;
mod locks;
mod permit;
mod step;
mod transcript;
mod turn;

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_loop::gemini::{self, KeyError};
use agent_loop::retry::RetrySchedule;
use agent_loop::{ModelBackend, Provider, Tokens, anthropic, shared_http};
use axum::Router;
use axum::routing::{get, post};
use rustc_hash::FxHashMap;
use serde::Serialize;
use serde_json::json;
use sqlx::{Error as SqlxError, Postgres, Transaction};
use studio_protocol::{AgentModel, JournalKind, ModelProvider, Ulid};
use tokio::time::interval;
use wps_client::wps::Wps;

pub use cards::press_decides;
pub use end::{expire_waits, sweep};
pub use follow::{FollowError, follow};
use locks::lock_turn;
use permit::{Permit, Running, busy};

use crate::config::{AgentSettings, Model, ModelTransport, ThinkingDisplay};
use crate::db::{decode, mint_ulid};
use crate::idempotency::idempotent;
use crate::journal::{Appended, NewEntry, Written, append};
use crate::{AppState, GateState};

/// How often the wait sweep looks for questions and approvals left open past `STUDIO_WAIT_TTL_HOURS` (wall clock).
const WAIT_SWEEP_EVERY: Duration = Duration::from_secs(60);

/// What the model is, as the configuration left it: a backend that answers, or why there is none.
pub enum Backend {
    Ready(Arc<dyn ModelBackend>),
    /// No request can be made: Gemini's key file is missing or empty. The sentence says why.
    Unavailable(String),
    /// Claude on Vertex, task 52's.
    NotBuilt,
}

/// The agent loop's share of `AppState`: its settings, the model, this process's instance id (a running turn's lease
/// owner) and the turns this process is running.
pub struct Agent {
    pub settings: AgentSettings,
    pub provider: ModelProvider,
    pub model_id: String,
    pub max_tokens: Tokens,
    /// Whether a configuration named the model, which `GET /api/me` then names.
    named: bool,
    backend: Backend,
    instance: Ulid,
    running: Mutex<FxHashMap<Ulid, Running>>,
    permits: AtomicU64,
    retry: RetrySchedule,
}

impl Agent {
    pub fn new(settings: AgentSettings, provider: ModelProvider, model_id: &str, backend: Backend) -> Self {
        Self {
            settings,
            provider,
            model_id: model_id.to_owned(),
            max_tokens: Tokens(32_000),
            named: true,
            backend,
            instance: mint_ulid(),
            running: Mutex::default(),
            permits: AtomicU64::new(0),
            retry: RetrySchedule::default(),
        }
    }

    /// The model the configuration names: Gemini with the key its file holds, Anthropic with its key, or Vertex, which
    /// answers `not-built`. A key file others can read refuses the start; a missing or empty one leaves the agent
    /// answering `model-unavailable`, and the start says so.
    pub fn from_config(settings: AgentSettings, model: &Model) -> Result<Self, String> {
        let thoughts = model.thinking_display == ThinkingDisplay::Updates;
        let (provider, backend) = match &model.transport {
            ModelTransport::Gemini { key_file } => match gemini::read_key(key_file) {
                Ok(key) => {
                    let backend = gemini::backend(&key, &model.id, shared_http(), None, thoughts);
                    (ModelProvider::Gemini, Backend::Ready(Arc::new(backend)))
                }
                Err(open @ KeyError::Open { .. }) => return Err(open.to_string()),
                Err(missing) => {
                    let why = format!("the agent has no Gemini key: {missing}");
                    eprintln!("studio-api: {why}; a message answers model-unavailable until one is in place");
                    (ModelProvider::Gemini, Backend::Unavailable(why))
                }
            },
            ModelTransport::Anthropic { api_key } => {
                let backend = anthropic::backend(api_key.expose(), &model.id, shared_http(), None);
                (ModelProvider::Anthropic, Backend::Ready(Arc::new(backend)))
            }
            ModelTransport::Vertex { .. } => (ModelProvider::Vertex, Backend::NotBuilt),
        };
        let mut agent = Self::new(settings, provider, &model.id, backend);
        agent.max_tokens = Tokens(u64::from(model.max_tokens.get()));
        Ok(agent)
    }

    /// A model that answers no message, as a service built without one: what `AppState::new` holds until
    /// `with_agent` gives it the configured one.
    pub fn unconfigured() -> Self {
        let why = "the agent's model is not configured in this process".to_owned();
        let unnamed = Self::new(AgentSettings::default(), ModelProvider::Gemini, "none", Backend::Unavailable(why));
        Self { named: false, ..unnamed }
    }

    /// The scripted fake of the tests, retrying at once.
    pub fn scripted(settings: AgentSettings, backend: Arc<dyn ModelBackend>) -> Self {
        let model = backend.model().to_owned();
        let mut agent = Self::new(settings, ModelProvider::Gemini, &model, Backend::Ready(backend));
        agent.retry = RetrySchedule { first_wait: Duration::from_millis(5), retries: 3 };
        agent
    }

    /// The provider and model `GET /api/me` names, when a configuration named them.
    pub fn model(&self) -> Option<AgentModel> {
        self.named.then(|| AgentModel { provider: self.provider, model: self.model_id.clone() })
    }

    /// The provider a conversation records at its first turn.
    fn provider_word(&self) -> &'static str {
        match &self.backend {
            Backend::Ready(backend) if backend.provider() == Provider::Scripted => "scripted",
            _ => match self.provider {
                ModelProvider::Gemini => "gemini",
                ModelProvider::Anthropic => "anthropic",
                ModelProvider::Vertex => "vertex",
            },
        }
    }
}

pub fn routes<S: Wps>(app: &AppState) -> Router<GateState<S>> {
    Router::new()
        .route("/api/conversations/{conversation}/messages", idempotent(app, post(admit::message)))
        .route("/api/questions/{question}/answers", post(cards::answers))
        .route("/api/approvals/{approval}/decision", post(cards::decision))
        .route("/api/turns/{turn}/stop", post(end::stop))
        .route("/api/turns/{turn}/calls", get(end::calls))
}

/// The wait sweep, once a minute for as long as the service runs.
pub async fn sweep_waits_every_minute(app: AppState) {
    let mut every = interval(WAIT_SWEEP_EVERY);
    loop {
        every.tick().await;
        if let Err(fault) = expire_waits(&app).await {
            eprintln!("studio-api: the waits past STUDIO_WAIT_TTL_HOURS were not expired: {fault}");
        }
    }
}

/// Journal one entry of `turn` in `tx`; every entry the loop writes has no dedup key, so one is always written.
pub(crate) async fn journal(
    tx: &mut Transaction<'_, Postgres>,
    conversation: &Ulid,
    turn: Option<&Ulid>,
    kind: JournalKind,
    body: impl Serialize,
    caused_by: Option<String>,
) -> Result<Written, SqlxError> {
    let entry = NewEntry { turn: turn.cloned(), caused_by, ..NewEntry::new(kind, json!(body)) };
    match append(tx, conversation, entry).await {
        Ok(Appended::Wrote(written)) => Ok(written),
        Ok(Appended::Held(_)) => Err(decode("an entry without a dedup key was held")),
        Err(fault) => Err(decode(fault.to_string())),
    }
}
