//! Within `agent`, the `turn` is one turn carried out by its task (spec 01 section 3.4): the turn loaded with the
//! identity and page context of the message that started it, its first user message composed (earlier turns
//! summarised first when the conversation has grown past its gate), then model request after model request, each
//! reply's calls run, until the model ends without calling a tool, a budget is reached, a call pauses the turn on a
//! person, or something ends it. The task holds the turn's lease, renewed while it runs, and its permit, and lets go of
//! both at every wait and ending; a resumption after an answer, a decision or a message is a new task that first
//! settles the waiting call, a message's words waiting on the turn's row until they follow their reply's results. The
//! budgets are checked before each request and each call, the wall-clock time spent running counted across waits (spec
//! 01 section 3.4).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_loop::prompt::SYSTEM_PROMPT;
use agent_loop::rig::{Message, ToolResult};
use agent_loop::{ModelBackend, ModelRequest, Registry, Stop, Tokens};
use serde_json::Value;
use sqlx::{Error as SqlxError, PgConnection, Row};
use studio_protocol::{CallId, PageContext, SitePath, TurnNumber, TurnState, Ulid};
use tokio::time::{Instant, interval};
use tokio_util::sync::CancellationToken;
use wps_client::wps::Wps;

use super::calls::{self, Ran};
use super::end::{self, Budget, Ending, Next};
use super::step::{self, Failure, Purpose};
use super::transcript::{self, closing, page_line, user_message};
use super::{Agent, Backend, Permit, lock_turn};
use crate::GateState;
use crate::db::decode;
use crate::identity::Identity;
use crate::tools::{Shown, ToolContext, registry};

/// How often a running turn renews its lease, and how far ahead (wall clock).
const RENEW_EVERY: Duration = Duration::from_secs(10);
const LEASE_AHEAD_SECS: i32 = 30;
/// The output ceiling a reply cut inside a tool call is asked again with, at most (spec 01 section 3.7).
const LARGEST_CEILING: Tokens = Tokens(128_000);

pub(super) enum Start {
    Fresh,
    /// After an answer, a decision or a message that ended the wait.
    Resumed,
}

/// The turn as its task holds it.
pub(super) struct Turn<S: Wps> {
    pub gate: GateState<S>,
    pub id: Ulid,
    pub n: TurnNumber,
    pub conversation: Ulid,
    pub site: Option<SitePath>,
    pub identity: Identity,
    pub attended: bool,
    pub page: Option<PageContext>,
    pub cancel: CancellationToken,
    pub registry: Arc<Registry<ToolContext<S>>>,
    /// What the turn's last `panel.show` opened, as its row keeps it, which a later command without a location acts on.
    pub shown: Arc<Mutex<Option<Shown>>>,
    /// When this stretch of running began (wall clock), for the active-time budget.
    pub stretch: Instant,
    /// Wall-clock time the turn spent running before this stretch, waits excluded (`turns.active_ms` at its load).
    pub active_before: Duration,
    pub ceiling: Tokens,
}

impl<S: Wps> Turn<S> {
    pub fn agent(&self) -> &Agent {
        &self.gate.app.agent
    }

    /// Wall-clock time the turn has spent running, this stretch included and waits excluded.
    pub fn active_spent(&self) -> Duration {
        self.active_before + self.stretch.elapsed()
    }

    /// What is left of the turn's active-time budget (wall clock).
    pub fn active_left(&self) -> Duration {
        self.agent().settings.max_active.saturating_sub(self.active_spent())
    }

    pub fn backend(&self) -> Option<Arc<dyn ModelBackend>> {
        match &self.agent().backend {
            Backend::Ready(backend) => Some(backend.clone()),
            _ => None,
        }
    }

    /// A tool's context for `call`, its writes keyed by the call's id, or by the approval's when `approval` executes,
    /// acting as `identity`.
    pub fn context(&self, call: &CallId, identity: Identity, approval: Option<Ulid>) -> ToolContext<S> {
        let key = approval.as_ref().map_or_else(|| call.to_string(), Ulid::to_string);
        ToolContext {
            gate: self.gate.clone(),
            identity,
            conversation: self.conversation.clone(),
            site: self.site.clone(),
            turn: self.id.clone(),
            turn_n: self.n.get(),
            call: call.clone(),
            key,
            cancel: self.cancel.child_token(),
            page: self.page.clone(),
            shown: self.shown.clone(),
            approval,
            progress: Arc::default(),
        }
    }

    fn request(&self, history: Vec<Message>, tools_off: bool) -> ModelRequest {
        let tools = self.registry.definitions();
        ModelRequest { system: SYSTEM_PROMPT.to_owned(), history, tools, tools_off, max_tokens: self.ceiling }
    }

    /// The history as the next request sends it.
    async fn history(&self) -> Result<Vec<Message>, SqlxError> {
        let mut conn = self.gate.app.pool.acquire().await?;
        Ok(transcript::history(&transcript::rows(&mut conn, &self.conversation).await?))
    }
}

/// Carry out turn `id` from `start` under `permit`, taken before the turn's row committed as running, holding its
/// lease while it runs.
pub(super) async fn drive<S: Wps>(gate: GateState<S>, id: Ulid, start: Start, permit: Permit) {
    let agent = &gate.app.agent;
    let renewing = tokio::spawn(renew(gate.clone(), id.clone(), agent.instance.clone()));
    let outcome = match load(&gate, &id, permit.cancel.clone()).await {
        Ok(Some(mut turn)) => run(&mut turn, start).await,
        Ok(None) => Ok(()),
        Err(fault) => Err(fault),
    };
    renewing.abort();
    if let Err(fault) = outcome {
        eprintln!("studio-api: turn {id} failed on the service's side: {fault}");
        let words = "The agent stopped: the service failed on its side, and the fault is in its log.";
        if let Err(again) = end::ending(&gate, &id, Ending::failed("internal", words.to_owned())).await {
            eprintln!("studio-api: turn {id} could not be ended either: {again}");
        }
    }
    drop(permit);
}

/// Renew the running turn's lease every 10 s (wall clock) until the task stops it.
async fn renew<S: Wps>(gate: GateState<S>, id: Ulid, owner: Ulid) {
    let mut every = interval(RENEW_EVERY);
    loop {
        every.tick().await;
        let renewed = sqlx::query(
            "UPDATE turns SET lease_owner = $2, lease_expires_at = now() + make_interval(secs => $3) \
             WHERE id = $1 AND state = 'running'",
        )
        .bind(id.as_str())
        .bind(owner.as_str())
        .bind(LEASE_AHEAD_SECS)
        .execute(&gate.app.pool)
        .await;
        if let Err(fault) = renewed {
            eprintln!("studio-api: turn {id}'s lease was not renewed: {fault}");
        }
    }
}

/// The running turn `id`, or `None` when it no longer runs.
async fn load<S: Wps>(gate: &GateState<S>, id: &Ulid, cancel: CancellationToken) -> Result<Option<Turn<S>>, SqlxError> {
    let row = sqlx::query(
        "SELECT t.n, t.conversation, t.started_by, t.attended, t.message_seq, t.state, t.active_ms, t.panel_context, \
             c.site, p.email, p.name \
         FROM turns t JOIN conversations c ON c.id = t.conversation LEFT JOIN people p ON p.subject = t.started_by \
         WHERE t.id = $1",
    )
    .bind(id.as_str())
    .fetch_optional(&gate.app.pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    if row.try_get::<String, _>("state")? != "running" {
        return Ok(None);
    }
    let conversation: Ulid = row.try_get::<String, _>("conversation")?.parse().map_err(decode)?;
    let subject: String = row.try_get("started_by")?;
    let identity = Identity {
        email: row.try_get::<Option<String>, _>("email")?.unwrap_or_else(|| format!("{subject}@localhost")),
        name: row.try_get::<Option<String>, _>("name")?.unwrap_or_else(|| subject.clone()),
        developer: gate.app.identity.developers.iter().any(|developer| developer.as_str() == subject),
        subject: subject.parse().map_err(decode)?,
    };
    let message: Option<Value> = sqlx::query_scalar("SELECT body FROM journal WHERE conversation = $1 AND seq = $2")
        .bind(conversation.as_str())
        .bind(row.try_get::<i64, _>("message_seq")?)
        .fetch_optional(&gate.app.pool)
        .await?;
    let page = message.and_then(|body| serde_json::from_value(body["context"].clone()).ok());
    let shown: Option<Shown> =
        row.try_get::<Option<Value>, _>("panel_context")?.map(serde_json::from_value).transpose().map_err(decode)?;
    Ok(Some(Turn {
        gate: gate.clone(),
        id: id.clone(),
        n: TurnNumber::new(row.try_get::<i32, _>("n")? as u32),
        conversation,
        site: row.try_get::<Option<String>, _>("site")?.map(|site| site.parse()).transpose().map_err(decode)?,
        identity,
        attended: row.try_get("attended")?,
        page,
        cancel,
        registry: Arc::new(registry::<S>()),
        shown: Arc::new(Mutex::new(shown)),
        stretch: Instant::now(),
        active_before: Duration::from_millis(row.try_get::<i64, _>("active_ms")? as u64),
        ceiling: gate.app.agent.max_tokens,
    }))
}

async fn run<S: Wps>(turn: &mut Turn<S>, start: Start) -> Result<(), SqlxError> {
    if turn.backend().is_none() {
        let words = "The agent's model cannot be reached in this process.".to_owned();
        return end::finish(turn, Ending::failed("model-unavailable", words)).await;
    }
    match start {
        Start::Fresh => {
            if let Some(ending) = super::step::compact_if_needed(turn).await? {
                return end::finish(turn, ending).await;
            }
            open(turn).await?;
        }
        Start::Resumed => match calls::resume(turn).await? {
            Ran::Paused => return Ok(()),
            Ran::Ended(ending) => return end::finish(turn, ending).await,
            Ran::Done(results) => {
                if let Some(ending) = after_calls(turn, results).await? {
                    return end::finish(turn, ending).await;
                }
            }
        },
    }
    let ending = carry_on(turn).await?;
    end::finish(turn, ending).await
}

/// The turn's first user message (spec 01 section 3.7, "A user message, in order"): results closing the calls a
/// stopped or interrupted turn left, the context notes, the page context and project, then the person's words.
async fn open<S: Wps>(turn: &mut Turn<S>) -> Result<(), SqlxError> {
    let pool = &turn.gate.app.pool;
    let mut conn = pool.acquire().await?;
    let rows = transcript::rows(&mut conn, &turn.conversation).await?;
    let closings: Vec<ToolResult> = transcript::unanswered(&rows).iter().map(closing).collect();
    let (message_seq, after): (i64, Option<i64>) = sqlx::query_as(
        "SELECT t.message_seq, (SELECT MAX(p.notes_through) FROM turns p WHERE p.conversation = t.conversation \
             AND p.n < t.n) FROM turns t WHERE t.id = $1",
    )
    .bind(turn.id.as_str())
    .fetch_one(&mut *conn)
    .await?;
    let notes = transcript::notes(&mut conn, &turn.conversation, after.unwrap_or(0), message_seq).await?;
    let words: Option<String> =
        sqlx::query_scalar("SELECT body->>'text' FROM journal WHERE conversation = $1 AND seq = $2")
            .bind(turn.conversation.as_str())
            .bind(message_seq)
            .fetch_one(&mut *conn)
            .await?;
    let place = project_line(&mut conn, turn).await?;
    drop(conn);
    let mut texts: Vec<String> = notes.into_iter().collect();
    texts.extend(turn.page.iter().map(page_line));
    texts.push(place);
    texts.extend(words);
    let mut tx = pool.begin().await?;
    transcript::append(&mut tx, &turn.conversation, &turn.id, &user_message(closings, texts), None).await?;
    sqlx::query("UPDATE turns SET notes_through = $2 WHERE id = $1")
        .bind(turn.id.as_str())
        .bind(message_seq)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// Where the conversation stands, for the model: its site, or a new site's project and customer.
async fn project_line<S: Wps>(conn: &mut PgConnection, turn: &Turn<S>) -> Result<String, SqlxError> {
    let (name, customer): (String, Option<String>) = sqlx::query_as(
        "SELECT p.name, p.customer FROM projects p JOIN conversations c ON c.project = p.id WHERE c.id = $1",
    )
    .bind(turn.conversation.as_str())
    .fetch_one(conn)
    .await?;
    let customer = customer.map_or_else(|| "none".to_owned(), |customer| format!("`{customer}`"));
    Ok(match &turn.site {
        Some(site) => {
            format!("This conversation belongs to the site {site}, of the project {name} (customer {customer}).")
        }
        None => format!("This is a New site conversation of the project {name}, whose customer is {customer}."),
    })
}

/// Request after request until the turn ends or pauses; `None` once it paused on a person.
async fn carry_on<S: Wps>(turn: &mut Turn<S>) -> Result<Ending, SqlxError> {
    loop {
        if turn.cancel.is_cancelled() {
            return Ok(Ending::stopped());
        }
        let request = turn.request(turn.history().await?, false);
        let reply = match step::request(turn, request, Purpose::Step).await? {
            Ok(reply) => reply,
            Err(Failure::Stopped) => return Ok(Ending::stopped()),
            Err(Failure::Model(error)) => return Ok(Ending::of_model(&error, turn.backend())),
            Err(Failure::Budget(budget)) => {
                carry(turn, Vec::new(), Some(&budget)).await?;
                return summarise(turn, budget).await;
            }
        };
        match &reply.stop {
            Stop::MaxTokens if !reply.calls.is_empty() => {
                if turn.ceiling >= LARGEST_CEILING.min(Tokens(turn.agent().max_tokens.0 * 2)) {
                    let words = "The model's reply was cut inside a tool call twice, so nothing of it ran.";
                    return Ok(Ending::failed("model-output-truncated", words.to_owned()));
                }
                turn.ceiling = Tokens(turn.ceiling.0 * 2).min(LARGEST_CEILING);
                continue;
            }
            Stop::Refused(category) => {
                let words = format!(
                    "The model refused to continue{}.",
                    category.as_ref().map(|category| format!(" ({category})")).unwrap_or_default()
                );
                return Ok(Ending::failed("model-refused", words));
            }
            Stop::Other(reason) => {
                let words =
                    format!("The model's provider ended the reply for a reason this build does not know: {reason}.");
                return Ok(Ending::failed("agent-provider-failed", words));
            }
            Stop::EndTurn | Stop::MaxTokens | Stop::ToolUse => {}
        }
        let model_call = step::appended(turn, &reply).await?;
        if reply.stop != Stop::ToolUse || reply.calls.is_empty() {
            return Ok(Ending::done(reply.stop == Stop::MaxTokens));
        }
        match calls::run(turn, model_call, reply.calls.clone()).await? {
            Ran::Paused => return Ok(Ending::Paused),
            Ran::Ended(ending) => return Ok(ending),
            Ran::Done(results) => {
                if let Some(ending) = after_calls(turn, results).await? {
                    return Ok(ending);
                }
            }
        }
    }
}

/// Every call of a reply has its result: they go to the model in one user message, past a budget with its notice
/// before the one summary request the turn ends on (spec 01 section 3.4, "The loop's budgets").
async fn after_calls<S: Wps>(turn: &mut Turn<S>, results: Vec<ToolResult>) -> Result<Option<Ending>, SqlxError> {
    let budget = end::over_budget(turn, Next::Request).await?;
    carry(turn, results, budget.as_ref()).await?;
    match budget {
        Some(budget) => summarise(turn, budget).await.map(Some),
        None => Ok(None),
    }
}

/// One user message (spec 01 section 3.7, "A user message, in order"): `results`, then the texts of the messages that
/// ended the turn's waits, kept on its row until now and taken off it in the same commit, then `budget`'s notice.
async fn carry<S: Wps>(turn: &Turn<S>, results: Vec<ToolResult>, budget: Option<&Budget>) -> Result<(), SqlxError> {
    let mut tx = turn.gate.app.pool.begin().await?;
    lock_turn(&mut tx, turn.id.as_str()).await?;
    let pending: Value = sqlx::query_scalar("SELECT pending_words FROM turns WHERE id = $1")
        .bind(turn.id.as_str())
        .fetch_one(&mut *tx)
        .await?;
    let mut texts: Vec<String> = serde_json::from_value(pending).map_err(decode)?;
    texts.extend(budget.map(Budget::notice));
    transcript::append(&mut tx, &turn.conversation, &turn.id, &user_message(results, texts), None).await?;
    sqlx::query("UPDATE turns SET pending_words = '[]' WHERE id = $1").bind(turn.id.as_str()).execute(&mut *tx).await?;
    tx.commit().await
}

/// The one last request, with tools off, once `budget` is reached: the turn ends `done` naming it, the summary kept in
/// the transcript only when the request stops usably with text (spec 01 section 3.4, "The loop's budgets").
async fn summarise<S: Wps>(turn: &mut Turn<S>, budget: Budget) -> Result<Ending, SqlxError> {
    let request = turn.request(turn.history().await?, true);
    match step::request(turn, request, Purpose::Summary).await? {
        Ok(reply) if step::usable(&reply).is_some() => {
            step::appended(turn, &reply).await?;
        }
        Ok(_) | Err(Failure::Budget(_)) => {}
        Err(Failure::Stopped) => return Ok(Ending::stopped()),
        Err(Failure::Model(error)) => return Ok(Ending::of_model(&error, turn.backend())),
    }
    Ok(Ending::Done { cut: false, budget: Some(budget.word()) })
}

pub(super) fn state_word(state: TurnState) -> &'static str {
    match state {
        TurnState::Running => "running",
        TurnState::Waiting => "waiting",
        TurnState::Done => "done",
        TurnState::Failed => "failed",
        TurnState::Stopped => "stopped",
        TurnState::Interrupted => "interrupted",
    }
}
