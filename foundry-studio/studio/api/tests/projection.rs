//! Spec 01 section 3.6's projection as a pure function of the journal (brief I1.04 section 3): every object a
//! conversation's entries name, its state after each entry, the number of the last entry that changed it, and the
//! objects a window of entries keeps.

use serde_json::{Value, json};
use studio_api::journal::{Fold, Named};
use studio_protocol::{
    ApprovalState, CallId, CallState, DraftState, EnvelopeVersion, Execution, JournalEntry, JournalKind, JournalSeq,
    QuestionState, TurnState, Ulid, WaitingOn, WallClock,
};

const TURN: &str = "01KN0TJZANP5N0PGJD8F7A3T01";
const APPROVAL: &str = "01KN0TJZANP5N0PGJD8F7A3A01";
const QUESTION: &str = "01KN0TJZANP5N0PGJD8F7A3Q01";
const DRAFT: &str = "01KN0TJZANP5N0PGJD8F7A3D01";
const RUN: &str = "01KN0TJZANP5N0PGJD8F7A3R01";

fn ulid(id: &str) -> Ulid {
    id.parse().expect("a ULID")
}

/// The entries of one turn that asks an approval and a question, drafts, and watches a run, numbered from 1.
fn journal() -> Vec<JournalEntry> {
    let author = json!({ "subject": "u-1", "name": "Person u-1" });
    let turn = Some(TURN);
    let steps: Vec<(JournalKind, Option<&str>, Value)> = vec![
        (
            JournalKind::MessageUser,
            turn,
            json!({ "author": author, "text": "Make the picks faster", "context": null, "source": "composer", "browser_tab": "tab-1" }),
        ),
        (
            JournalKind::TurnStarted,
            turn,
            json!({ "n": 1, "author": author, "message_seq": 1, "attended": true, "browser_tab": "tab-1" }),
        ),
        (
            JournalKind::ToolStarted,
            turn,
            json!({ "call": "toolu_1", "tool": "flow.apply", "class": "asking", "summary": "Apply the flow" }),
        ),
        (JournalKind::ToolProgress, turn, json!({ "call": "toolu_1", "note": "Checking the draft" })),
        (
            JournalKind::ApprovalRequested,
            turn,
            json!({ "approval": APPROVAL, "call": "toolu_1", "tool": "flow.apply", "class": "asking", "summary": "Apply the flow", "input": {}, "facts": {} }),
        ),
        (JournalKind::TurnWaiting, turn, json!({ "on": "approval", "id": APPROVAL, "reply_block": "b1" })),
        (JournalKind::ApprovalDecided, turn, json!({ "approval": APPROVAL, "decision": "approve", "author": author })),
        (JournalKind::TurnResumed, turn, json!({ "by": "decision" })),
        (JournalKind::ApprovalExecution, turn, json!({ "approval": APPROVAL, "execution": "running" })),
        (JournalKind::ApprovalExecution, turn, json!({ "approval": APPROVAL, "execution": "ok" })),
        (
            JournalKind::ToolFinished,
            turn,
            json!({ "call": "toolu_1", "outcome": "ok", "summary": "Applied as flow changeset 2", "refs": [] }),
        ),
        (
            JournalKind::ToolStarted,
            turn,
            json!({ "call": "toolu_2", "tool": "ask_user", "class": "read", "summary": "Ask which aisle" }),
        ),
        (JournalKind::QuestionAsked, turn, json!({ "question": QUESTION, "call": "toolu_2", "items": [] })),
        (
            JournalKind::QuestionAnswered,
            turn,
            json!({ "question": QUESTION, "answers": [], "in_words": false, "author": author }),
        ),
        (JournalKind::DraftUpdated, turn, json!({ "draft": DRAFT, "state": "drafting", "ops_total": 1 })),
        (JournalKind::DraftUpdated, turn, json!({ "draft": DRAFT, "state": "ready", "ops_total": 2 })),
        (JournalKind::RunStatus, None, json!({ "run": RUN, "state": "queued" })),
        (JournalKind::RunStatus, None, json!({ "run": RUN, "state": "running" })),
        (JournalKind::RunFinding, None, json!({ "run": RUN, "finding": { "kind": "plan-differs" } })),
        (JournalKind::TurnFinished, turn, json!({ "outcome": "done", "reply_block": "b3", "usage": {} })),
        (JournalKind::DraftApplied, None, json!({ "draft": DRAFT, "seqs": { "flow": 3 } })),
        (
            JournalKind::ToolFinished,
            None,
            json!({ "call": "toolu_unknown", "outcome": "ok", "summary": "No such call" }),
        ),
    ];
    steps
        .into_iter()
        .zip(1..)
        .map(|((kind, turn, body), seq)| JournalEntry {
            v: EnvelopeVersion,
            seq: JournalSeq::new(seq),
            at: WallClock::now(),
            conversation: ulid("01KN0TJZANP5N0PGJD8F7A3C01"),
            turn: turn.map(ulid),
            kind,
            caused_by: None,
            body,
        })
        .collect()
}

/// Spec 01 section 3.6, "The projection": folding a turn's entries leaves each object in the state spec 02 section
/// 3.2 draws (a call waiting while its approval waits, then running, then done; an approval approved and executed;
/// a question answered; a draft applied; a run at its latest status with its finding), each with the number of the
/// last entry that changed it; an entry naming an object nobody started is noted, never guessed at.
#[test]
fn the_fold_leaves_each_object_as_its_last_entry_did() {
    let entries = journal();
    let fold = Fold::of(&entries);
    let everything = fold.projection(&Named::of(&entries), false);

    let turn = &everything.turns[0];
    assert_eq!((turn.n.get(), turn.state, turn.waiting_on), (1, TurnState::Done, None));
    assert_eq!((turn.reply_block.as_ref().map(|b| b.as_str()), turn.as_of_seq), (Some("b3"), JournalSeq::new(20)));
    let calls: Vec<(&str, CallState, u64)> =
        everything.calls.iter().map(|call| (call.call.as_str(), call.state, call.as_of_seq.get())).collect();
    assert_eq!(calls, [("toolu_1", CallState::Ok, 11), ("toolu_2", CallState::Running, 12)]);
    assert_eq!(everything.calls[0].note, None, "a finished call shows no progress note");
    let approval = &everything.approvals[0];
    assert_eq!(
        (approval.state, approval.execution, approval.as_of_seq),
        (ApprovalState::Approved, Some(Execution::Ok), JournalSeq::new(10))
    );
    assert_eq!(approval.requested["summary"], "Apply the flow", "the first entry's body");
    let question = &everything.questions[0];
    assert_eq!(
        (question.state, question.as_of_seq, question.call.as_ref()),
        (QuestionState::Answered, JournalSeq::new(14), Some(&CallId::new("toolu_2")))
    );
    let draft = &everything.drafts[0];
    assert_eq!((draft.state, draft.as_of_seq), (DraftState::Applied, JournalSeq::new(21)));
    assert_eq!(draft.updated.as_ref().map(|body| &body["ops_total"]), Some(&json!(2)), "the latest draft.updated");
    let run = &everything.runs[0];
    assert_eq!((&run.status["state"], run.findings.len(), run.as_of_seq), (&json!("running"), 1, JournalSeq::new(19)));
    assert_eq!(fold.unread(), [JournalSeq::new(22)]);
    assert_eq!(fold.open_turn(), None);
}

/// Spec 01 section 3.6: part way through, the fold's state is the state as of that entry: the call waits with its
/// approval, the turn waits on it, and the approval is pending, which `pending` lists.
#[test]
fn part_way_the_fold_holds_the_state_as_of_that_entry() {
    let entries = journal();
    let fold = Fold::of(&entries[..6]);
    let projection = fold.projection(&Named::of(&entries[..6]), true);
    assert_eq!(projection.calls[0].state, CallState::Waiting);
    assert_eq!(projection.calls[0].note.as_deref(), Some("Checking the draft"));
    let open = fold.open_turn().expect("the turn waits");
    assert_eq!((open.state, open.waiting_on), (TurnState::Waiting, Some(WaitingOn::Approval)));
    assert_eq!(fold.pending().approvals, [ulid(APPROVAL)]);
}

/// Spec 01 section 3.6: a window of entries keeps what it names, every call and card of a turn it names, and, for a
/// snapshot, every pending card with its turn; a run no entry of the window names stays out.
#[test]
fn a_window_keeps_what_it_names_and_the_turns_it_touches() {
    let entries = journal();
    let fold = Fold::of(&entries);
    let last_two = fold.projection(&Named::of(&entries[19..21]), false);
    assert_eq!(last_two.turns.len(), 1, "turn.finished names the turn");
    assert_eq!((last_two.calls.len(), last_two.questions.len(), last_two.approvals.len()), (2, 1, 1), "the turn's own");
    assert_eq!(last_two.drafts.len(), 1);
    assert!(last_two.runs.is_empty(), "no entry of the window names the run");

    let asking = Fold::of(&entries[..13]);
    let nothing_named = asking.projection(&Named::of(&[]), true);
    assert_eq!(nothing_named.questions.len(), 1, "the open question, from before the window");
    assert_eq!(nothing_named.turns.len(), 1, "with its turn's header");
    assert!(nothing_named.calls.is_empty() && nothing_named.approvals.is_empty(), "and nothing else of the turn");

    let watching = Fold::of(&entries[..19]);
    let runs = watching.projection(&Named::of(&entries[16..18]), true);
    assert_eq!((runs.runs.len(), runs.drafts.len(), runs.turns.len()), (1, 1, 1), "the run, the open draft, its turn");
    assert!(runs.questions.is_empty(), "the answered question waits on nobody");
}
