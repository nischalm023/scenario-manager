//! Within `studio-api`, `approvals` are spec 01 section 3.10's approvals as far as a button act needs them: a
//! person's press recorded as their approval (rule 6) with the input it approves, its execution's states each
//! journaled as `approval.execution` on the conversation the press came from, what the execution sends kept with the
//! press, and `GET /api/approvals/{a}`, which a page with no conversation attached reads once a second (wall clock)
//! after a press. An execution has one worker (LEAD-DECISIONS entry 54, one API process): it holds the approval in
//! this process and claims it in Postgres, and every move of its state goes forward, never back; a worker holding a
//! claimed write whose outcome is unknown asks again under one policy, Create site's and a draft apply's alike
//! (`Asks`, entry 58). Cards, decisions and attribution arrive with the agent loop (task 3); Create site and a run's
//! submission record presses here.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::Uri;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use rustc_hash::FxHashSet;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgRow;
use sqlx::{Error as SqlxError, PgConnection, Postgres, Row, Transaction};
use studio_protocol::{
    ApprovalAnswer, ApprovalExecuted, ApprovalRecord, ApprovalState, ApprovalVia, Execution, JournalKind, Person,
    Problem, ProblemType, Subject, Ulid, WallClock,
};
use tokio::time::{Instant, sleep};
use wps_client::wps::Wps;

use crate::db::{decode, mint_ulid, read_word, word};
use crate::idempotency::write_canonical;
use crate::journal::{Appended, JournalError, NewEntry, Written, append};
use crate::problem::{ProblemAnswer, internal};
use crate::{AppState, GateState};

pub fn routes<S: Wps>() -> Router<GateState<S>> {
    Router::new().route("/api/approvals/{approval}", get(read))
}

/// A press as its route records it: the conversation it came from, the act's tool and class, the exact input it
/// approves with the digest its act names it by (a plan's `plan_sha256`; else the SHA-256 of its canonical JSON), the
/// summary and card facts a person reads, and who pressed.
pub struct Press<'a> {
    pub conversation: Option<&'a Ulid>,
    pub tool: &'a str,
    pub class: &'a str,
    pub input: Value,
    pub digest: Option<String>,
    pub summary: String,
    pub facts: Value,
    pub by: &'a Subject,
}

/// Record `press` as its person's approval, approved at this wall-clock instant, its execution not started.
pub async fn record_button(tx: &mut Transaction<'_, Postgres>, press: Press<'_>) -> Result<Ulid, SqlxError> {
    let id = mint_ulid();
    let digest = press.digest.unwrap_or_else(|| {
        let mut canonical = Vec::new();
        write_canonical(&press.input, &mut canonical);
        format!("{:x}", Sha256::digest(&canonical))
    });
    sqlx::query(
        "INSERT INTO approvals (id, conversation, tool, class, input, input_digest, summary, facts, via, state, \
             decided_by, decided_at, execution) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'button', 'approved', $9, now(), 'not_started')",
    )
    .bind(id.as_str())
    .bind(press.conversation.map(Ulid::as_str))
    .bind(press.tool)
    .bind(press.class)
    .bind(&press.input)
    .bind(digest)
    .bind(&press.summary)
    .bind(&press.facts)
    .bind(press.by.as_str())
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

/// Keep what an approval's execution sends, Create site's plan for one, in the transaction that records the approval,
/// so that every attempt sends the same; it never changes afterwards.
pub async fn record_sent(tx: &mut Transaction<'_, Postgres>, approval: &Ulid, sent: &Value) -> Result<(), SqlxError> {
    sqlx::query("UPDATE approvals SET sent = $2 WHERE id = $1")
        .bind(approval.as_str())
        .bind(sent)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The approvals a worker of this process is executing. There is one API process, whose start settles what a stopped
/// one left before it accepts a write (LEAD-DECISIONS entry 54), so an approval held here has its one worker, and a
/// second (the start's recovery beside a press's, or a press asked again) finds it held and does nothing.
#[derive(Default)]
pub struct Executing(Mutex<FxHashSet<Ulid>>);

/// A worker's hold on one approval's execution, let go when the worker ends.
pub struct Holding {
    executing: Arc<Executing>,
    approval: Ulid,
}

impl Executing {
    /// Hold `approval` for the calling worker; `None` when another worker of this process holds it.
    pub fn hold(self: &Arc<Self>, approval: &Ulid) -> Option<Holding> {
        let mut held = self.0.lock().expect("the executing set's lock");
        held.insert(approval.clone()).then(|| Holding { executing: self.clone(), approval: approval.clone() })
    }
}

impl Drop for Holding {
    fn drop(&mut self) {
        if let Ok(mut held) = self.executing.0.lock() {
            held.remove(&self.approval);
        }
    }
}

/// How long a worker keeps asking about a claimed write whose outcome is unknown, and its first and longest wait
/// between asks, each wait doubling the last (wall clock): spec 04 section 3.9's five minutes, after which the next
/// start asks again.
const ASK_WINDOW: Duration = Duration::from_secs(300);
const FIRST_WAIT: Duration = Duration::from_secs(2);
const LONGEST_WAIT: Duration = Duration::from_secs(30);

/// The asks one worker has left about a claimed write whose outcome is unknown, Create site's and a draft apply's
/// alike (LEAD-DECISIONS-IN-ABSENCE.md entry 58): waits doubling from the first to the longest, `wait` answering
/// `false` once the next would end past the window, which leaves the write to the next start's recovery.
pub(crate) struct Asks {
    until: Instant,
    wait: Duration,
}

impl Asks {
    pub(crate) fn new() -> Self {
        Self::since(Instant::now())
    }

    fn since(start: Instant) -> Self {
        Self { until: start + ASK_WINDOW, wait: FIRST_WAIT }
    }

    /// The wait before the next ask, at `now`, or `None` once it would end past the window.
    fn next_wait(&mut self, now: Instant) -> Option<Duration> {
        if now + self.wait > self.until {
            return None;
        }
        let wait = self.wait;
        self.wait = (self.wait * 2).min(LONGEST_WAIT);
        Some(wait)
    }

    /// Wait for the next ask; `false`, at once, when the window has closed.
    pub(crate) async fn wait(&mut self) -> bool {
        let Some(wait) = self.next_wait(Instant::now()) else { return false };
        sleep(wait).await;
        true
    }
}

/// The states an execution may move to `execution` from (spec 01 section 3.10): `running` only from `not_started`, an
/// outcome only from `running` or `unknown`, `unknown` only from `running`. Nothing moves back, so a late worker can
/// never set a settled execution `running` again.
fn moves_from(execution: Execution) -> Vec<String> {
    let from: &[Execution] = match execution {
        Execution::NotStarted => &[],
        Execution::Running => &[Execution::NotStarted],
        Execution::Unknown => &[Execution::Running],
        Execution::Ok | Execution::Failed | Execution::Interrupted => &[Execution::Running, Execution::Unknown],
    };
    from.iter().map(|state| word(*state)).collect()
}

/// An execution that moved: its `approval.execution` entry, for the hub once the caller commits, when a conversation
/// is attached and takes entries.
pub struct Moved(pub Option<Written>);

/// Move `approval`'s execution to `execution` with what the act answered and, for a failed or unknown one, the
/// sentence a person reads, and journal `approval.execution` on its conversation, naming whose yes it carries out
/// (spec 01 section 3.10); `None`, nothing written, when the execution may not move there from where it stands.
pub async fn executed(
    tx: &mut Transaction<'_, Postgres>,
    approval: &Ulid,
    execution: Execution,
    sentence: Option<&str>,
    outcome: Option<&Value>,
) -> Result<Option<Moved>, SqlxError> {
    moved(tx, approval, None, execution, sentence, outcome).await
}

/// `executed` for an apply, whose `approval.execution` entries name the changeset draft it executes (spec 01 section
/// 3.11).
pub async fn applying(
    tx: &mut Transaction<'_, Postgres>,
    approval: &Ulid,
    draft: &Ulid,
    execution: Execution,
    sentence: Option<&str>,
    outcome: Option<&Value>,
) -> Result<Option<Moved>, SqlxError> {
    moved(tx, approval, Some(draft), execution, sentence, outcome).await
}

async fn moved(
    tx: &mut Transaction<'_, Postgres>,
    approval: &Ulid,
    draft: Option<&Ulid>,
    execution: Execution,
    sentence: Option<&str>,
    outcome: Option<&Value>,
) -> Result<Option<Moved>, SqlxError> {
    let mut stored = outcome.cloned().unwrap_or(Value::Null);
    if let (Some(sentence), Value::Object(fields)) = (sentence, &mut stored) {
        fields.insert("sentence".into(), json!(sentence));
    } else if let Some(sentence) = sentence {
        stored = json!({ "sentence": sentence });
    }
    let row: Option<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "UPDATE approvals a SET execution = $2, outcome = COALESCE($3, a.outcome) \
         WHERE a.id = $1 AND a.execution = ANY($4) \
         RETURNING a.conversation, a.decided_by, (SELECT p.name FROM people p WHERE p.subject = a.decided_by)",
    )
    .bind(approval.as_str())
    .bind(word(execution))
    .bind((!stored.is_null()).then_some(&stored))
    .bind(moves_from(execution))
    .fetch_optional(&mut **tx)
    .await?;
    let Some((conversation, subject, name)) = row else { return Ok(None) };
    let Some(conversation) = conversation else { return Ok(Some(Moved(None))) };
    let conversation: Ulid = conversation.parse().map_err(decode)?;
    let subject = subject.ok_or_else(|| decode(format!("approval {approval} executes with no one's yes")))?;
    let approver = Person { name: name.unwrap_or_else(|| subject.clone()), subject: subject.parse().map_err(decode)? };
    let body = ApprovalExecuted {
        approval: approval.clone(),
        execution,
        sentence: sentence.map(str::to_owned),
        unattributable: None,
        approver,
        draft: draft.cloned(),
    };
    let entry = NewEntry {
        caused_by: Some(approval.to_string()),
        ..NewEntry::new(JournalKind::ApprovalExecution, json!(body))
    };
    match append(tx, &conversation, entry).await {
        Ok(Appended::Wrote(written)) => Ok(Some(Moved(Some(written)))),
        Ok(Appended::Held(_)) | Err(JournalError::Closed { .. }) => Ok(Some(Moved(None))),
        Err(JournalError::Database(fault)) => Err(fault),
        Err(other) => Err(decode(other.to_string())),
    }
}

/// Claim `approval`'s execution for a worker that holds it in this process, `true` when the worker runs it:
/// `not_started` becomes `running` with its entry, and one already `running` or `unknown` is held by no worker here,
/// so with one API process a stopped process left it, and the worker resumes it as it stands. A settled one is
/// `false`, and the worker does nothing.
pub async fn claim(state: &AppState, approval: &Ulid) -> Result<bool, SqlxError> {
    claim_applying(state, approval, None).await
}

/// `claim` for an apply, whose `running` entry names the changeset draft it executes.
pub async fn claim_applying(state: &AppState, approval: &Ulid, draft: Option<&Ulid>) -> Result<bool, SqlxError> {
    let mut tx = state.pool.begin().await?;
    if let Some(Moved(entry)) = moved(&mut tx, approval, draft, Execution::Running, None, None).await? {
        tx.commit().await?;
        state.hub.announce(entry);
        return Ok(true);
    }
    let execution: Option<String> = sqlx::query_scalar("SELECT execution FROM approvals WHERE id = $1")
        .bind(approval.as_str())
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    let resumable = [Execution::Running, Execution::Unknown].map(word);
    Ok(execution.is_some_and(|execution| resumable.contains(&execution)))
}

/// `GET /api/approvals/{a}`.
async fn read(
    State(state): State<AppState>,
    uri: Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<ApprovalAnswer>, ProblemAnswer> {
    let instance = uri.path();
    let nothing = || ProblemAnswer::from(Problem::new(ProblemType::NotFound, "No approval has that id.").at(instance));
    let Ok(Path(id)) = path else { return Err(nothing()) };
    let id: Ulid = id.parse().map_err(|_| nothing())?;
    let mut conn = state.pool.acquire().await.map_err(|fault| internal("read the approval", fault, instance))?;
    approval_record(&mut conn, &id)
        .await
        .map_err(|fault| internal("read the approval", fault, instance))?
        .map(|approval| Json(ApprovalAnswer { approval }))
        .ok_or_else(nothing)
}

/// `approval` as the approval routes and a button's `202` answer it, or `None` when no approval has that id.
pub async fn approval_record(conn: &mut PgConnection, approval: &Ulid) -> Result<Option<ApprovalRecord>, SqlxError> {
    let row = sqlx::query(
        "SELECT a.conversation, a.tool, a.class, a.via, a.state, a.execution, a.outcome, a.requested_at, \
             a.decided_by, p.name AS decided_by_name, a.decided_at \
         FROM approvals a LEFT JOIN people p ON p.subject = a.decided_by WHERE a.id = $1",
    )
    .bind(approval.as_str())
    .fetch_optional(conn)
    .await?;
    row.map(|row| record(approval.clone(), &row)).transpose()
}

fn record(id: Ulid, row: &PgRow) -> Result<ApprovalRecord, SqlxError> {
    let outcome: Option<Value> = row.try_get("outcome")?;
    let sentence =
        outcome.as_ref().and_then(|outcome| outcome.get("sentence")).and_then(Value::as_str).map(str::to_owned);
    let decided_by =
        match (row.try_get::<Option<String>, _>("decided_by")?, row.try_get::<Option<String>, _>("decided_by_name")?) {
            (Some(subject), name) => Some(Person {
                name: name.unwrap_or_else(|| subject.clone()),
                subject: subject.parse().map_err(decode)?,
            }),
            (None, _) => None,
        };
    let at = |column: &str| row.try_get::<Option<DateTime<Utc>>, _>(column).map(|at| at.map(WallClock::from));
    Ok(ApprovalRecord {
        id,
        conversation: row
            .try_get::<Option<String>, _>("conversation")?
            .map(|id| id.parse())
            .transpose()
            .map_err(decode)?,
        tool: row.try_get("tool")?,
        class: row.try_get("class")?,
        via: read_word::<ApprovalVia>(&row.try_get::<String, _>("via")?)?,
        state: read_word::<ApprovalState>(&row.try_get::<String, _>("state")?)?,
        execution: row
            .try_get::<Option<String>, _>("execution")?
            .map(|word| read_word::<Execution>(&word))
            .transpose()?,
        sentence,
        outcome,
        requested_at: WallClock::from(row.try_get::<DateTime<Utc>, _>("requested_at")?),
        decided_by,
        decided_at: at("decided_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec 04 section 3.9's one policy (LEAD-DECISIONS-IN-ABSENCE.md entry 58): the asks about an unknown outcome
    /// wait 2, 4, 8 and 16 s and then 30 s each, of wall clock, and stop once another would end past five minutes,
    /// leaving the write to the next start.
    #[test]
    fn asks_double_to_thirty_seconds_and_stop_within_five_minutes() {
        let start = Instant::now();
        let mut asks = Asks::since(start);
        let (mut now, mut waits) = (start, Vec::new());
        while let Some(wait) = asks.next_wait(now) {
            waits.push(wait.as_secs());
            now += wait;
        }
        assert_eq!(waits[..5], [2, 4, 8, 16, 30]);
        assert!(waits[4..].iter().all(|wait| *wait == 30), "{waits:?}");
        assert_eq!(waits.iter().sum::<u64>(), 300, "five minutes of asking");
    }
}
