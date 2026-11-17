//! Within `journal`, `projection` folds a conversation's entries into the state of everything they name (spec 01
//! section 3.6, "The projection"; brief I1.04 section 3, lead decided): turns, tool calls, questions, approvals,
//! drafts and runs, each carrying the number of the last entry that changed it. The fold is a pure function of the
//! journal from its first entry, as spec 02 section 3.2 draws the log, so a snapshot and a page of earlier entries
//! fold alike; each keeps what its own entries name, a snapshot every pending card too.

use std::hash::Hash;

use rustc_hash::{FxHashMap, FxHashSet};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use studio_protocol::{
    ApprovalState, CallId, CallState, ConversationChanged, DraftState, Execution, JournalEntry, JournalKind,
    JournalSeq, PanelCommandEntry, PanelEvent, Pending, ProjectedApproval, ProjectedCall, ProjectedDraft,
    ProjectedQuestion, ProjectedRun, ProjectedTurn, Projection, QuestionState, Ulid,
};

use super::turns::{self, TURN_KINDS, TurnFold, read};

/// Every kind that changes an object of the projection; the fold reads only these.
pub const FOLDED_KINDS: [JournalKind; 21] = [
    JournalKind::TurnStarted,
    JournalKind::TurnWaiting,
    JournalKind::TurnResumed,
    JournalKind::TurnFinished,
    JournalKind::ToolStarted,
    JournalKind::ToolProgress,
    JournalKind::ToolFinished,
    JournalKind::QuestionAsked,
    JournalKind::QuestionAnswered,
    JournalKind::QuestionExpired,
    JournalKind::ApprovalRequested,
    JournalKind::ApprovalDecided,
    JournalKind::ApprovalExpired,
    JournalKind::ApprovalNoted,
    JournalKind::ApprovalExecution,
    JournalKind::DraftUpdated,
    JournalKind::DraftApplied,
    JournalKind::DraftDiscarded,
    JournalKind::DraftStale,
    JournalKind::RunStatus,
    JournalKind::RunFinding,
];

/// Objects by id, in the order they first appeared.
#[derive(Debug)]
struct Ordered<K, V> {
    items: Vec<V>,
    index: FxHashMap<K, usize>,
}

impl<K, V> Default for Ordered<K, V> {
    fn default() -> Self {
        Self { items: Vec::new(), index: FxHashMap::default() }
    }
}

impl<K: Hash + Eq, V> Ordered<K, V> {
    fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.index.get(key).map(|n| &mut self.items[*n])
    }

    /// Put `value` under `key`, in the place of the object it replaces, if any.
    fn put(&mut self, key: K, value: V) {
        match self.index.get(&key) {
            Some(n) => self.items[*n] = value,
            None => {
                self.index.insert(key, self.items.len());
                self.items.push(value);
            }
        }
    }

    /// The object under `key`, made by `make` when there is none yet.
    fn entry(&mut self, key: K, make: impl FnOnce() -> V) -> &mut V {
        let n = *self.index.entry(key).or_insert_with(|| {
            self.items.push(make());
            self.items.len() - 1
        });
        &mut self.items[n]
    }
}

/// Everything a conversation's entries named, as the entries applied so far leave it.
#[derive(Debug, Default)]
pub struct Fold {
    turns: TurnFold,
    calls: Ordered<CallId, ProjectedCall>,
    questions: Ordered<Ulid, ProjectedQuestion>,
    approvals: Ordered<Ulid, ProjectedApproval>,
    drafts: Ordered<Ulid, ProjectedDraft>,
    runs: Ordered<Ulid, ProjectedRun>,
    unread: Vec<JournalSeq>,
}

// What the fold reads of each kind's body; the test writer refuses a body these cannot read.
#[derive(Deserialize)]
struct ToolStarted {
    call: CallId,
    tool: String,
    class: String,
    summary: String,
}

#[derive(Deserialize)]
struct ToolProgress {
    call: CallId,
    note: String,
}

#[derive(Deserialize)]
struct ToolFinished {
    call: CallId,
    outcome: CallState,
    summary: String,
    #[serde(default)]
    problem: Option<Value>,
}

#[derive(Deserialize)]
struct OfQuestion {
    question: Ulid,
    #[serde(default)]
    call: Option<CallId>,
}

#[derive(Deserialize)]
struct OfApproval {
    approval: Ulid,
    #[serde(default)]
    call: Option<CallId>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Decision {
    Approve,
    Deny,
}

#[derive(Deserialize)]
struct Decided {
    approval: Ulid,
    decision: Decision,
}

#[derive(Deserialize)]
struct Executed {
    approval: Ulid,
    execution: Execution,
}

#[derive(Deserialize)]
struct OfDraft {
    draft: Ulid,
}

#[derive(Deserialize)]
struct Updated {
    draft: Ulid,
    state: DraftState,
}

#[derive(Deserialize)]
struct OfRun {
    run: Ulid,
}

impl Fold {
    pub fn of<'a>(entries: impl IntoIterator<Item = &'a JournalEntry>) -> Self {
        let mut fold = Self::default();
        entries.into_iter().for_each(|entry| fold.apply(entry));
        fold
    }

    /// Apply one entry; a kind that changes no object changes nothing, and a body the fold cannot read is noted in
    /// `unread`.
    pub fn apply(&mut self, entry: &JournalEntry) {
        if TURN_KINDS.contains(&entry.kind) {
            return self.turns.apply(entry);
        }
        let (seq, body, turn) = (entry.seq, &entry.body, entry.turn.clone());
        let applied = match entry.kind {
            JournalKind::ToolStarted => read(body).map(|started| self.started(started, turn, seq)).is_some(),
            JournalKind::ToolProgress => read(body).is_some_and(|progress| self.progressed(progress, seq)),
            JournalKind::ToolFinished => read(body).is_some_and(|finished| self.finished(finished, seq)),
            JournalKind::QuestionAsked => read(body).map(|question| self.asked(question, body, turn, seq)).is_some(),
            JournalKind::QuestionAnswered | JournalKind::QuestionExpired => {
                read(body).is_some_and(|question| self.answered(entry.kind, question, body, seq))
            }
            JournalKind::ApprovalRequested => {
                read(body).map(|approval| self.requested(approval, body, turn, seq)).is_some()
            }
            JournalKind::ApprovalDecided => read(body).is_some_and(|decided| self.decided(decided, body, seq)),
            JournalKind::ApprovalExpired | JournalKind::ApprovalNoted => {
                read(body).is_some_and(|approval| self.noted(entry.kind, approval, body, seq))
            }
            JournalKind::ApprovalExecution => {
                read(body).map(|executed| self.executed(executed, body, turn, seq)).is_some()
            }
            JournalKind::DraftUpdated => {
                read(body).map(|Updated { draft, state }| self.drafted(draft, state, body, turn, seq)).is_some()
            }
            JournalKind::DraftApplied | JournalKind::DraftDiscarded | JournalKind::DraftStale => {
                let state = match entry.kind {
                    JournalKind::DraftApplied => DraftState::Applied,
                    JournalKind::DraftDiscarded => DraftState::Discarded,
                    _ => DraftState::Stale,
                };
                read(body).map(|OfDraft { draft }| self.drafted(draft, state, body, turn, seq)).is_some()
            }
            JournalKind::RunStatus | JournalKind::RunFinding => {
                read(body).map(|OfRun { run }| self.ran(entry.kind, run, body, seq)).is_some()
            }
            _ => true,
        };
        if !applied {
            self.unread.push(seq);
        }
    }

    fn started(&mut self, body: ToolStarted, turn: Option<Ulid>, seq: JournalSeq) {
        let ToolStarted { call, tool, class, summary } = body;
        let started = ProjectedCall {
            call: call.clone(),
            turn,
            tool,
            class,
            state: CallState::Running,
            summary,
            note: None,
            problem: None,
            as_of_seq: seq,
        };
        self.calls.put(call, started);
    }

    fn progressed(&mut self, body: ToolProgress, seq: JournalSeq) -> bool {
        let Some(call) = self.calls.get_mut(&body.call) else { return false };
        (call.note, call.as_of_seq) = (Some(body.note), seq);
        true
    }

    fn finished(&mut self, body: ToolFinished, seq: JournalSeq) -> bool {
        let Some(call) = self.calls.get_mut(&body.call).filter(|_| !is_open(body.outcome)) else { return false };
        (call.state, call.summary, call.problem, call.note) = (body.outcome, body.summary, body.problem, None);
        call.as_of_seq = seq;
        true
    }

    fn asked(&mut self, body: OfQuestion, asked: &Value, turn: Option<Ulid>, seq: JournalSeq) {
        let question = ProjectedQuestion {
            id: body.question.clone(),
            call: body.call,
            turn,
            state: QuestionState::Open,
            asked: asked.clone(),
            answered: None,
            as_of_seq: seq,
        };
        self.questions.put(body.question, question);
    }

    fn answered(&mut self, kind: JournalKind, body: OfQuestion, answer: &Value, seq: JournalSeq) -> bool {
        let Some(question) = self.questions.get_mut(&body.question) else { return false };
        if kind == JournalKind::QuestionAnswered {
            (question.state, question.answered) = (QuestionState::Answered, Some(answer.clone()));
        } else {
            question.state = QuestionState::Expired;
        }
        question.as_of_seq = seq;
        true
    }

    fn requested(&mut self, body: OfApproval, requested: &Value, turn: Option<Ulid>, seq: JournalSeq) {
        self.call_waits(body.call.as_ref(), CallState::Waiting, seq);
        let approval = ProjectedApproval {
            id: body.approval.clone(),
            call: body.call,
            turn,
            state: ApprovalState::Pending,
            execution: None,
            requested: requested.clone(),
            decided: None,
            executed: None,
            noted: None,
            as_of_seq: seq,
        };
        self.approvals.put(body.approval, approval);
    }

    fn decided(&mut self, body: Decided, decided: &Value, seq: JournalSeq) -> bool {
        let Some(approval) = self.approvals.get_mut(&body.approval) else { return false };
        let (state, call) = match body.decision {
            Decision::Approve => (ApprovalState::Approved, CallState::Running),
            Decision::Deny => (ApprovalState::Denied, CallState::Denied),
        };
        (approval.state, approval.decided, approval.as_of_seq) = (state, Some(decided.clone()), seq);
        let asked_by = approval.call.clone();
        self.call_waits(asked_by.as_ref(), call, seq);
        true
    }

    fn noted(&mut self, kind: JournalKind, body: OfApproval, noted: &Value, seq: JournalSeq) -> bool {
        let Some(approval) = self.approvals.get_mut(&body.approval) else { return false };
        if kind == JournalKind::ApprovalExpired {
            approval.state = ApprovalState::Expired;
        } else {
            approval.noted = Some(noted.clone());
        }
        approval.as_of_seq = seq;
        true
    }

    /// An execution's change; a button's approval, which no `approval.requested` precedes since the press is the
    /// approval (spec 01 section 3.10, rule 6), first appears here, approved.
    fn executed(&mut self, body: Executed, executed: &Value, turn: Option<Ulid>, seq: JournalSeq) {
        let pressed = || ProjectedApproval {
            id: body.approval.clone(),
            call: None,
            turn,
            state: ApprovalState::Approved,
            execution: None,
            requested: Value::Null,
            decided: None,
            executed: None,
            noted: None,
            as_of_seq: seq,
        };
        let approval = self.approvals.entry(body.approval.clone(), pressed);
        (approval.execution, approval.executed, approval.as_of_seq) =
            (Some(body.execution), Some(executed.clone()), seq);
    }

    fn drafted(&mut self, id: Ulid, state: DraftState, body: &Value, turn: Option<Ulid>, seq: JournalSeq) {
        let made = || ProjectedDraft { id: id.clone(), turn, state, updated: None, ended: None, as_of_seq: seq };
        let draft = self.drafts.entry(id.clone(), made);
        if state.is_open() {
            draft.updated = Some(body.clone());
        } else {
            draft.ended = Some(body.clone());
        }
        (draft.state, draft.as_of_seq) = (state, seq);
    }

    fn ran(&mut self, kind: JournalKind, id: Ulid, body: &Value, seq: JournalSeq) {
        let made = || ProjectedRun { run: id.clone(), status: Value::Null, findings: Vec::new(), as_of_seq: seq };
        let run = self.runs.entry(id.clone(), made);
        if kind == JournalKind::RunStatus {
            run.status = body.clone();
        } else {
            run.findings.push(body.clone());
        }
        run.as_of_seq = seq;
    }

    /// The row of the call an approval was asked by turns `state`, as spec 02 section 3.2 draws it.
    fn call_waits(&mut self, call: Option<&CallId>, state: CallState, seq: JournalSeq) {
        if let Some(call) = call.and_then(|call| self.calls.get_mut(call)) {
            (call.state, call.as_of_seq) = (state, seq);
        }
    }

    /// The turn still running or waiting.
    pub fn open_turn(&self) -> Option<&ProjectedTurn> {
        self.turns.open()
    }

    /// The numbers of the entries the fold could not read, which leave nothing in the projection.
    pub fn unread(&self) -> Vec<JournalSeq> {
        let mut unread: Vec<JournalSeq> = self.unread.iter().chain(&self.turns.unread).copied().collect();
        unread.sort();
        unread
    }

    /// The questions still open, the approvals still pending and the drafts still open.
    pub fn pending(&self) -> Pending {
        Pending {
            questions: ids(&self.questions.items, |question| {
                (question.state == QuestionState::Open).then_some(&question.id)
            }),
            approvals: ids(&self.approvals.items, |approval| {
                (approval.state == ApprovalState::Pending).then_some(&approval.id)
            }),
            drafts: ids(&self.drafts.items, |draft| draft.state.is_open().then_some(&draft.id)),
        }
    }

    /// What `named` names, every call, card and draft of each turn it names, and, `with_pending`, every pending card
    /// and open draft with its turn's header.
    pub fn projection(&self, named: &Named, with_pending: bool) -> Projection {
        let pending = if with_pending { self.pending() } else { Pending::default() };
        let of_turn = |turn: &Option<Ulid>| turn.as_ref().is_some_and(|turn| named.turns.contains(turn));
        let questions: Vec<ProjectedQuestion> = kept(&self.questions.items, |question| {
            named.questions.contains(&question.id)
                || of_turn(&question.turn)
                || pending.questions.contains(&question.id)
        });
        let approvals: Vec<ProjectedApproval> = kept(&self.approvals.items, |approval| {
            named.approvals.contains(&approval.id)
                || of_turn(&approval.turn)
                || pending.approvals.contains(&approval.id)
        });
        let drafts: Vec<ProjectedDraft> = kept(&self.drafts.items, |draft| {
            named.drafts.contains(&draft.id) || of_turn(&draft.turn) || pending.drafts.contains(&draft.id)
        });
        let card_turns: FxHashSet<&Ulid> = (questions.iter().map(|card| &card.turn))
            .chain(approvals.iter().map(|card| &card.turn))
            .chain(drafts.iter().map(|draft| &draft.turn))
            .flatten()
            .collect();
        Projection {
            turns: kept(self.turns.turns(), |turn| named.turns.contains(&turn.id) || card_turns.contains(&turn.id)),
            calls: kept(&self.calls.items, |call| named.calls.contains(&call.call) || of_turn(&call.turn)),
            questions,
            approvals,
            drafts,
            runs: kept(&self.runs.items, |run| named.runs.contains(&run.run)),
        }
    }
}

fn kept<T: Clone>(items: &[T], keep: impl Fn(&T) -> bool) -> Vec<T> {
    items.iter().filter(|item| keep(item)).cloned().collect()
}

fn ids<T>(items: &[T], id: impl Fn(&T) -> Option<&Ulid>) -> Vec<Ulid> {
    items.iter().filter_map(id).cloned().collect()
}

/// The objects a range of entries names: each entry's turn, and the call, question, approval, draft or run its
/// body or its `caused_by` names.
#[derive(Debug, Default)]
pub struct Named {
    turns: FxHashSet<Ulid>,
    calls: FxHashSet<CallId>,
    questions: FxHashSet<Ulid>,
    approvals: FxHashSet<Ulid>,
    drafts: FxHashSet<Ulid>,
    runs: FxHashSet<Ulid>,
}

impl Named {
    pub fn of<'a>(entries: impl IntoIterator<Item = &'a JournalEntry>) -> Self {
        let mut named = Self::default();
        for entry in entries {
            named.turns.extend(entry.turn.clone());
            let field = |name: &str| entry.body.get(name).and_then(Value::as_str);
            let ulid = |name: &str| field(name).and_then(|id| id.parse::<Ulid>().ok());
            named.calls.extend(field("call").map(CallId::new));
            named.questions.extend(ulid("question"));
            named.approvals.extend(ulid("approval"));
            named.drafts.extend(ulid("draft"));
            named.runs.extend(ulid("run"));
            // A wait names its card by `id`; a cause is a call, a question or an approval.
            let causes = [ulid("id"), entry.caused_by.as_deref().and_then(|cause| cause.parse().ok())];
            for cause in causes.into_iter().flatten() {
                named.questions.insert(cause.clone());
                named.approvals.insert(cause);
            }
            named.calls.extend(entry.caused_by.as_deref().map(CallId::new));
        }
        named
    }
}

fn is_open(state: CallState) -> bool {
    matches!(state, CallState::Running | CallState::Waiting)
}

/// Whether the fold, or the API's own typed bodies, read `body` as an entry of `kind` written in `turn`: the test
/// writer refuses what they cannot. A kind nothing reads takes any body.
pub fn readable(kind: JournalKind, body: &Value, turn: Option<&Ulid>) -> Result<(), String> {
    fn check<T: DeserializeOwned>(body: &Value) -> Result<T, String> {
        T::deserialize(body).map_err(|fault| fault.to_string())
    }
    match kind {
        _ if TURN_KINDS.contains(&kind) => turns::readable(kind, body, turn),
        JournalKind::ToolStarted => check::<ToolStarted>(body).map(drop),
        JournalKind::ToolProgress => check::<ToolProgress>(body).map(drop),
        JournalKind::ToolFinished => match check::<ToolFinished>(body)?.outcome {
            outcome if is_open(outcome) => {
                Err("a finished call's outcome is ok, failed, denied, stopped or interrupted".into())
            }
            _ => Ok(()),
        },
        JournalKind::QuestionAsked | JournalKind::QuestionAnswered | JournalKind::QuestionExpired => {
            check::<OfQuestion>(body).map(drop)
        }
        JournalKind::ApprovalRequested | JournalKind::ApprovalExpired | JournalKind::ApprovalNoted => {
            check::<OfApproval>(body).map(drop)
        }
        JournalKind::ApprovalDecided => check::<Decided>(body).map(drop),
        JournalKind::ApprovalExecution => check::<Executed>(body).map(drop),
        JournalKind::DraftUpdated => check::<Updated>(body).map(drop),
        JournalKind::DraftApplied | JournalKind::DraftDiscarded | JournalKind::DraftStale => {
            check::<OfDraft>(body).map(drop)
        }
        JournalKind::RunStatus | JournalKind::RunFinding => check::<OfRun>(body).map(drop),
        JournalKind::ConversationChanged => check::<ConversationChanged>(body).map(drop),
        JournalKind::PanelCommand => check::<PanelCommandEntry>(body).map(drop),
        JournalKind::PanelEvent => check::<PanelEvent>(body).map(drop),
        _ => Ok(()),
    }
}
