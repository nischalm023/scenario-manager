//! Within `run-invocation`, `store` is what the loops read and write and what they tell (spec 08 sections 3.4 and
//! 3.6): `RunStore`, the attempts' rows behind a trait (seats, the guarded ending, the transitions log and its cursor,
//! a re-minted token, and the platform's view of a held Job), and `RunEvents`, which turns a transition, a submission's
//! end or a run's progress into its conversation's words. The API implements both.

use std::fmt::{self, Display, Formatter};
use std::future::Future;
use std::time::Duration;

use desunits::SimTime;
use studio_protocol::{AttemptNumber, Ending, EndingCounts, ProgressValue, RunState, StreamingServer, Ulid, WallClock};

use crate::Fault;
use crate::job::{Creator, JobName};
use crate::settings::Seats;
use crate::token::Token;

/// One attempt of one run.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttemptKey {
    pub run: Ulid,
    pub n: AttemptNumber,
}

impl Display for AttemptKey {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        write!(out, "{}/{}", self.run, self.n.0)
    }
}

/// A seat just reserved: the attempt, the Job's recorded name, the image its plan names and a fresh token.
#[derive(Debug, Clone)]
pub struct Reserved {
    pub key: AttemptKey,
    pub job: JobName,
    pub image: String,
    pub token: Token,
}

/// An attempt as the dispatcher reads it again around a create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptRow {
    pub state: RunState,
    pub job: Option<JobName>,
    pub cancel_requested: bool,
}

/// An attempt holding a seat: its Job is recorded and not known stopped, whether or not the attempt has ended; who is
/// creating its Job, if anyone, and when its latest creation began; and when its runner's own report ended it, if it
/// did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldAttempt {
    pub key: AttemptKey,
    pub job: JobName,
    pub image: String,
    pub state: RunState,
    pub dispatched_at: WallClock,
    pub cancel_requested_at: Option<WallClock>,
    pub creator: Option<Creator>,
    pub claimed_at: WallClock,
    pub reported_at: Option<WallClock>,
}

/// The transaction that wrote a transition, as Postgres numbers transactions: the log's first order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct TxId(pub i64);

/// A transition's row in the log: its order within its transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct TransitionId(pub i64);

/// How many attempts a submission's runs have had, all told: the number of a round of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttemptCount(pub u32);

/// Where the watcher stands in the transitions log: the transaction and the row it last handed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Cursor {
    pub tx: TxId,
    pub id: TransitionId,
}

/// One change of an attempt's state, in the order the log hands it on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub at: Cursor,
    pub key: AttemptKey,
    pub from: Option<RunState>,
    pub to: RunState,
}

/// A submission whose runs had all ended as the transitions log stood at one transition, with its attempts counted
/// and its runs counted by how they ended then: one end per round of attempts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    pub submission: Ulid,
    pub attempts: AttemptCount,
    pub counts: EndingCounts,
}

/// A running attempt's progress as last recorded: how far in simtime, and its discovered running values.
#[derive(Debug, Clone, PartialEq)]
pub struct RunningProgress {
    pub key: AttemptKey,
    pub sim_ms: SimTime,
    pub values: Vec<ProgressValue>,
}

/// Whether an ending took: the first ending wins, and a later one changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    Wrote,
    AlreadyEnded,
}

/// The attempts' rows, as the loops need them (spec 08 section 3.4's list).
pub trait RunStore: Send + Sync + 'static {
    /// In one transaction, while fewer than `limit` seats are held, claim the first waiting attempt, record its Job's
    /// name with `creator`'s claim on its creation and mint its token; `None` when the seats are full or nothing waits.
    fn reserve(&self, limit: Seats, creator: &Creator) -> impl Future<Output = Result<Option<Reserved>, Fault>> + Send;
    /// Every create `creator` made of the attempt's Job has returned: its claim is let go.
    fn release(&self, key: &AttemptKey, creator: &Creator) -> impl Future<Output = Result<(), Fault>> + Send;
    fn attempt(&self, key: &AttemptKey) -> impl Future<Output = Result<Option<AttemptRow>, Fault>> + Send;
    /// The guarded ending: written only while the attempt is queued or running.
    fn end(&self, key: &AttemptKey, ending: &Ending) -> impl Future<Output = Result<Ended, Fault>> + Send;
    /// `end`, for an attempt whose Job could not be created, the ending typed as that beside its words.
    fn end_uncreated(&self, key: &AttemptKey, ending: &Ending) -> impl Future<Output = Result<Ended, Fault>> + Send;
    /// The invoker confirmed that no pod of the Job runs: the seat frees and the token dies.
    fn job_stopped(&self, key: &AttemptKey) -> impl Future<Output = Result<(), Fault>> + Send;
    fn held(&self) -> impl Future<Output = Result<Vec<HeldAttempt>, Fault>> + Send;
    /// Under the attempt's row lock, claim for `creator` the re-creation of a recorded Job the invoker does not know,
    /// with a fresh token whose hash replaces the old one's: only while the attempt is queued, not cancelled, its Job
    /// not known stopped, not `creator`'s already and `grace` past its last creation's start; `None` when it may not.
    fn reclaim(
        &self,
        key: &AttemptKey,
        creator: &Creator,
        grace: Duration,
    ) -> impl Future<Output = Result<Option<Token>, Fault>> + Send;
    fn cursor(&self) -> impl Future<Output = Result<Cursor, Fault>> + Send;
    /// The transitions after `cursor` from transactions that can no longer commit behind them, in order.
    fn transitions_after(&self, cursor: Cursor) -> impl Future<Output = Result<Vec<Transition>, Fault>> + Send;
    fn advance(&self, cursor: Cursor) -> impl Future<Output = Result<(), Fault>> + Send;
    fn progress_of_running(&self) -> impl Future<Output = Result<Vec<RunningProgress>, Fault>> + Send;
    /// The round of `at`'s submission that `at` ended: every run's latest transition up to and including `at` an
    /// ending, so the round ends at its last run's own ending in the log's order, whatever has happened since.
    fn round_ended(&self, at: &Transition) -> impl Future<Output = Result<Option<Round>, Fault>> + Send;
    /// What the invoker last said of a held attempt's Job, in words, kept on its row for whoever reads the attempt.
    fn platform_status(&self, key: &AttemptKey, words: &str) -> impl Future<Output = Result<(), Fault>> + Send;
    /// Point the attempt at one streaming server whose heartbeat is younger than `stale_after` by the store's own
    /// clock, picked at random (ruling D174), and record which on its row, so a viewer can be told where its stream
    /// is; `None`, recorded too, when no server is alive.
    fn stream_to(
        &self,
        key: &AttemptKey,
        stale_after: Duration,
    ) -> impl Future<Output = Result<Option<StreamingServer>, Fault>> + Send;
}

/// What the conversation hears: one entry per transition and per round's end, each under spec 08's dedup key, so a
/// repeat after a restart writes nothing; progress is sent live, never journaled.
pub trait RunEvents: Send + Sync + 'static {
    fn status_changed(&self, transition: &Transition) -> impl Future<Output = Result<(), Fault>> + Send;
    fn submission_ended(&self, round: &Round) -> impl Future<Output = Result<(), Fault>> + Send;
    fn progress(&self, progress: &RunningProgress) -> impl Future<Output = Result<(), Fault>> + Send;
}
