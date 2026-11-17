//! Brief I1.04 section 5: the journal fixtures that I1.05's browser tests replay through the test writer
//! (`tests/journal/*.jsonl`). Each line reads as a `JournalEntry` or a `DevLive`; each fixture numbers its entries
//! from 1 without a gap; between them they hold every kind of spec 02 section 3.2's table that a writer posts, each
//! body with spec 01 section 3.5's fields; every kind spec 02 speaks carries its sentence (spec 01 section 8,
//! statement 11); and their draft operations and keys follow spec 03 section 3.1's grammar, each `highlights` a
//! canonical number naming an entry that drew highlight groups or a Runs command carrying metrics. `stream.live`,
//! `ping` and `stream.reset` are the stream's own, so no writer posts them.

use std::fs;
use std::iter;
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use studio_protocol::{
    ApprovalDecided, ApprovalExecuted, ApprovalExpired, ApprovalNoted, ApprovalRequested, ConversationChanged, DevLive,
    DraftApplied, DraftDiscarded, DraftStale, DraftUpdated, JournalEntry, JournalKind, JournalSeq, Key, LiveKind,
    Panel, PanelCommandEntry, PanelEvent, QuestionAnswered, QuestionAsked, QuestionExpired, RunFinding, RunProgress,
    RunStatus, SubmissionEnded, SystemNote, TextDelta, TextDone, ToolFinished, ToolProgress, ToolStarted, TurnFinished,
    TurnResumed, TurnStarted, TurnWaiting, UserMessage,
};

enum Line {
    Entry(JournalEntry),
    Live(DevLive),
}

/// Every fixture, by file name, in name order.
fn fixtures() -> Vec<(String, Vec<Line>)> {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/journal");
    let mut names: Vec<PathBuf> = fs::read_dir(&folder)
        .expect("the fixture folder reads")
        .map(|entry| entry.expect("a folder entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "jsonl"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|path| {
            let name = path.file_name().expect("a file").to_string_lossy().into_owned();
            let text = fs::read_to_string(&path).expect("the fixture reads");
            let lines = text
                .lines()
                .enumerate()
                .map(|(n, line)| {
                    let value: Value =
                        serde_json::from_str(line).unwrap_or_else(|fault| panic!("{name}:{}: {fault}", n + 1));
                    if value.get("live").is_some() {
                        Line::Live(read(value, &name, n))
                    } else {
                        Line::Entry(read(value, &name, n))
                    }
                })
                .collect();
            (name, lines)
        })
        .collect()
}

fn read<T: DeserializeOwned>(value: Value, name: &str, n: usize) -> T {
    serde_json::from_value(value).unwrap_or_else(|fault| panic!("{name}:{}: {fault}", n + 1))
}

fn entries(lines: &[Line]) -> impl Iterator<Item = &JournalEntry> {
    lines.iter().filter_map(|line| match line {
        Line::Entry(entry) => Some(entry),
        Line::Live(_) => None,
    })
}

/// The fields spec 01 section 3.5 gives each kind's body (and spec 08 section 3.6 a run's events), the optional ones
/// left out.
fn fields(kind: JournalKind) -> &'static [&'static str] {
    match kind {
        JournalKind::MessageUser => &["author", "text", "context", "source", "browser_tab"],
        JournalKind::TurnStarted => &["n", "author", "message_seq", "attended", "browser_tab"],
        JournalKind::TextDone => &["block", "kind", "text"],
        JournalKind::ToolStarted => &["call", "tool", "class", "summary"],
        JournalKind::ToolProgress => &["call", "note"],
        JournalKind::ToolFinished => &["call", "outcome", "summary", "refs"],
        JournalKind::QuestionAsked => &["question", "call", "items"],
        JournalKind::QuestionAnswered => &["question", "answers", "in_words", "author"],
        JournalKind::QuestionExpired => &["question"],
        JournalKind::ApprovalRequested => &["approval", "call", "tool", "class", "summary", "input", "facts"],
        JournalKind::ApprovalDecided => &["approval", "decision", "author"],
        JournalKind::ApprovalExpired => &["approval"],
        JournalKind::ApprovalNoted => &["approval", "kind", "note"],
        JournalKind::ApprovalExecution => &["approval", "execution"],
        JournalKind::DraftUpdated => {
            &["draft", "initiated_on", "bases", "ops_total", "parts", "checked_by", "problems", "state"]
        }
        JournalKind::DraftApplied => &["draft", "seqs"],
        JournalKind::DraftDiscarded => &["draft"],
        JournalKind::DraftStale => &["draft", "reason"],
        JournalKind::PanelCommand | JournalKind::ConversationChanged => &[],
        JournalKind::PanelEvent => &["kind", "text", "body"],
        JournalKind::RunStatus => &["run", "submission", "position", "of", "attempt", "state", "from", "sim_ms"],
        JournalKind::SubmissionEnded => &["submission", "of", "counts"],
        JournalKind::RunFinding => &["run", "submission", "position", "of", "attempt", "finding"],
        JournalKind::TurnWaiting => &["on", "id"],
        JournalKind::TurnResumed => &["by"],
        JournalKind::TurnFinished => &["outcome", "usage"],
        JournalKind::SystemNote => &["text"],
    }
}

fn live_fields(kind: LiveKind) -> &'static [&'static str] {
    match kind {
        LiveKind::TextDelta => &[],
        LiveKind::RunProgress => &["run", "attempt", "sim_ms", "values"],
        LiveKind::TemplateEvaluation => &["draft", "n", "revision", "state", "problems"],
    }
}

/// Brief I1.04 section 5: every line of every fixture reads as an entry or a live item, and each fixture numbers its
/// entries from 1 without a gap, in one conversation, every turn's entries naming their turn.
#[test]
fn every_line_reads_and_each_fixture_numbers_from_1_without_a_gap() {
    let fixtures = fixtures();
    assert_eq!(fixtures.len(), 3);
    for (name, lines) in &fixtures {
        let numbers: Vec<u64> = entries(lines).map(|entry| entry.seq.get()).collect();
        assert_eq!(numbers, (1..=numbers.len() as u64).collect::<Vec<_>>(), "{name}");
        let conversation = &entries(lines).next().expect("an entry").conversation;
        assert!(entries(lines).all(|entry| &entry.conversation == conversation), "{name}: one conversation");
        let turn_kinds =
            [JournalKind::TurnStarted, JournalKind::TurnWaiting, JournalKind::TurnResumed, JournalKind::TurnFinished];
        assert!(
            entries(lines).filter(|entry| turn_kinds.contains(&entry.kind)).all(|entry| entry.turn.is_some()),
            "{name}"
        );
    }
}

/// Brief I1.04 section 5: between them the fixtures hold every journaled kind and every live-only kind a writer
/// posts.
#[test]
fn the_fixtures_hold_every_kind_a_writer_posts() {
    let fixtures = fixtures();
    let lines = || fixtures.iter().flat_map(|(_, lines)| lines);
    for kind in JournalKind::ALL {
        assert!(lines().any(|line| matches!(line, Line::Entry(entry) if entry.kind == *kind)), "{kind:?}");
    }
    for kind in LiveKind::ALL {
        assert!(lines().any(|line| matches!(line, Line::Live(DevLive { live }) if live.kind == kind)), "{kind:?}");
    }
}

/// Brief I1.04 section 5: each body carries spec 01 section 3.5's fields for its kind, and reads as the protocol's
/// type for its kind (`panel.command` as spec 03's entry), which is the type the studio pages read it as.
#[test]
fn each_body_carries_its_kinds_fields() {
    fn reads<T: DeserializeOwned>(body: &Value) -> Result<(), String> {
        serde_json::from_value::<T>(body.clone()).map(drop).map_err(|fault| fault.to_string())
    }
    for (name, lines) in fixtures() {
        for line in &lines {
            let (kind, body, wanted) = match line {
                Line::Entry(entry) => (entry.kind.as_str(), &entry.body, fields(entry.kind)),
                Line::Live(DevLive { live }) => (live.kind.as_str(), &live.body, live_fields(live.kind)),
            };
            for field in wanted {
                assert!(body.get(field).is_some(), "{name}: a {kind} body lacks `{field}`: {body}");
            }
            let typed = match kind {
                "message.user" => reads::<UserMessage>(body),
                "turn.started" => reads::<TurnStarted>(body),
                "turn.waiting" => reads::<TurnWaiting>(body),
                "turn.resumed" => reads::<TurnResumed>(body),
                "turn.finished" => reads::<TurnFinished>(body),
                "text.done" => reads::<TextDone>(body),
                "system.note" => reads::<SystemNote>(body),
                "tool.started" => reads::<ToolStarted>(body),
                "tool.progress" => reads::<ToolProgress>(body),
                "tool.finished" => reads::<ToolFinished>(body),
                "question.asked" => reads::<QuestionAsked>(body),
                "question.answered" => reads::<QuestionAnswered>(body),
                "question.expired" => reads::<QuestionExpired>(body),
                "approval.requested" => reads::<ApprovalRequested>(body),
                "approval.decided" => reads::<ApprovalDecided>(body),
                "approval.expired" => reads::<ApprovalExpired>(body),
                "approval.noted" => reads::<ApprovalNoted>(body),
                "approval.execution" => reads::<ApprovalExecuted>(body),
                "draft.updated" => reads::<DraftUpdated>(body),
                "draft.applied" => reads::<DraftApplied>(body),
                "draft.discarded" => reads::<DraftDiscarded>(body),
                "draft.stale" => reads::<DraftStale>(body),
                "panel.command" => reads::<PanelCommandEntry>(body),
                "panel.event" => reads::<PanelEvent>(body),
                "run.status" => reads::<RunStatus>(body),
                "run.finding" => reads::<RunFinding>(body),
                "submission.ended" => reads::<SubmissionEnded>(body),
                "conversation.changed" => reads::<ConversationChanged>(body),
                "text.delta" => reads::<TextDelta>(body),
                "run.progress" => reads::<RunProgress>(body),
                _ => Ok(()),
            };
            typed.unwrap_or_else(|fault| panic!("{name}: a {kind} body: {fault}"));
        }
    }
}

/// Spec 01 section 8, statement 11: every journaled kind spec 02 speaks carries a non-empty sentence in the fixtures:
/// a text block's text, a summary of a call that did not end `ok`, each question's text, an approval's summary and
/// moved-since note, a failed or unknown execution's sentence, a panel command's announcement, a `plan-differs`
/// finding's sentence and a system note's text.
#[test]
fn every_spoken_kind_carries_a_sentence() {
    let said = |value: &Value| value.as_str().is_some_and(|text| !text.trim().is_empty());
    for (name, lines) in fixtures() {
        for entry in entries(&lines) {
            let body = &entry.body;
            let spoken = match entry.kind {
                JournalKind::TextDone | JournalKind::SystemNote => said(&body["text"]),
                JournalKind::ToolFinished => body["outcome"] == "ok" || said(&body["summary"]),
                JournalKind::QuestionAsked => body["items"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty() && items.iter().all(|item| said(&item["text"]))),
                JournalKind::ApprovalRequested => said(&body["summary"]),
                JournalKind::ApprovalNoted => said(&body["note"]),
                JournalKind::ApprovalExecution => {
                    !matches!(body["execution"].as_str(), Some("failed" | "unknown")) || said(&body["sentence"])
                }
                JournalKind::PanelCommand => said(&body["announce"]),
                JournalKind::RunFinding => {
                    body["finding"]["kind"] != "plan-differs" || said(&body["finding"]["sentence"])
                }
                _ => true,
            };
            assert!(spoken, "{name}: entry {} ({}) has nothing to speak: {body}", entry.seq.get(), entry.kind.as_str());
        }
    }
}

/// Spec 01 section 3.11's operations, the words a draft's operation is written in.
const OPERATIONS: [&str; 3] = ["declare", "patch", "delete"];

/// The journal entries the routes in `text` name in `highlights`, each value read whole: to the next `&` or `#`, or to
/// the route's end, which is its string's for a route written alone and the space, `)` or `>` after one written in
/// text (Markdown's `[…](…)` and `<…>`). A value the browser refuses (`studio/ui/src/shell/routes.ts`), anything but a
/// positive integer without leading zeros (`026`, `26.5`, `26oops`), is the fault.
fn highlights_in(text: &str) -> Result<Vec<JournalSeq>, String> {
    text.match_indices("highlights=")
        .filter(|(at, _)| text[..*at].ends_with(['?', '&']))
        .map(|(at, name)| {
            let alone = !text[..at].contains(|c: char| c.is_whitespace() || c == '(' || c == '<');
            let ends = |c: char| c == '&' || c == '#' || (!alone && (c.is_whitespace() || c == ')' || c == '>'));
            let value = text[at + name.len()..].split(ends).next().unwrap_or_default();
            let canonical = value.bytes().all(|byte| byte.is_ascii_digit()) && !value.starts_with('0');
            let seq = value.parse().ok().filter(|_| canonical).map(JournalSeq::new);
            seq.ok_or_else(|| format!("`{text}` names highlights by `{value}`, not a canonical positive integer"))
        })
        .collect()
}

/// Every string in `value`, at any depth.
fn strings(value: &Value) -> Vec<&str> {
    match value {
        Value::String(text) => vec![text.as_str()],
        Value::Array(items) => items.iter().flat_map(strings).collect(),
        Value::Object(fields) => fields.values().flat_map(strings).collect(),
        _ => Vec::new(),
    }
}

/// Spec 03 section 3.1's grammar in a fixture the pages' tests replay, so that the next drift fails here instead of
/// teaching a page a wrong shape: every draft operation is one of spec 01 section 3.11's, on a key the grammar reads;
/// and every `highlights` names an entry of its fixture that a reader takes something from (spec 03 section 3.3): the
/// highlight groups the browser draws, or a Runs command's `metrics`, which the Runs panel headlines on reload
/// (`studio/api/src/results/headline.rs`). A `panel.command`'s own routes name itself or nothing: itself when it drew
/// groups, as the API's panel tools write it; nothing when no reader would take anything; and either for a Runs command
/// carrying only metrics, whose routes those tools leave without `highlights` today (`studio/api/src/tools/panels.rs`).
fn follows_the_grammar(lines: &[Line]) -> Result<(), String> {
    let commands: Vec<(JournalSeq, PanelCommandEntry)> = entries(lines)
        .filter(|entry| entry.kind == JournalKind::PanelCommand)
        .map(|entry| (entry.seq, serde_json::from_value(entry.body.clone()).expect("a panel.command body")))
        .collect();
    let drew = |command: &PanelCommandEntry| command.highlight.as_ref().is_some_and(|groups| !groups.is_empty());
    let headlines = |command: &PanelCommandEntry| {
        command.panel == Panel::Runs && command.metrics.as_ref().is_some_and(|metrics| !metrics.is_empty())
    };
    for (seq, command) in &commands {
        for route in iter::once(&command.route).chain(&command.as_it_was) {
            let fits = match highlights_in(route)?.as_slice() {
                [] => !drew(command),
                [named] => named == seq && (drew(command) || headlines(command)),
                _ => false,
            };
            if !fits {
                let seq = seq.get();
                return Err(format!(
                    "entry {seq}'s route `{route}`: a command's routes name highlights={seq} when it drew groups, may \
                     when it is a Runs command carrying metrics, and else name none"
                ));
            }
        }
    }
    for entry in entries(lines) {
        if entry.kind == JournalKind::DraftUpdated {
            let updated: DraftUpdated = serde_json::from_value(entry.body.clone()).expect("a draft.updated body");
            for op in updated.parts.iter().flat_map(|part| &part.ops) {
                if !OPERATIONS.contains(&op.op.as_str()) {
                    return Err(format!("entry {}'s operation `{}`", entry.seq.get(), op.op));
                }
                if let Err(fault) = op.key.parse::<Key>() {
                    let seq = entry.seq.get();
                    return Err(format!("entry {seq}'s operation names what it touches off the grammar: {fault}"));
                }
            }
        }
        for text in strings(&entry.body) {
            for named in highlights_in(text)? {
                if !commands.iter().any(|(at, command)| *at == named && (drew(command) || headlines(command))) {
                    let (seq, named) = (entry.seq.get(), named.get());
                    return Err(format!(
                        "entry {seq} names highlights={named}, which no entry of this fixture drew or headlines"
                    ));
                }
            }
        }
    }
    Ok(())
}

#[test]
fn draft_operations_keys_and_highlights_follow_the_grammar() {
    for (name, lines) in fixtures() {
        if let Err(fault) = follows_the_grammar(&lines) {
            panic!("{name}: {fault}");
        }
    }
}

/// A fixture of `written` entries, each a kind and its body, numbered from 1 in one conversation.
fn fixture_of(written: Vec<(JournalKind, Value)>) -> Vec<Line> {
    let conversation = "01M587F8H0DBFW4ATBHPV077MK";
    let line = |(n, (kind, body)): (usize, (JournalKind, Value))| {
        let entry = json!({ "v": 1, "seq": n + 1, "at": "2026-10-20T12:00:00.000Z", "conversation": conversation,
                            "turn": null, "kind": kind, "caused_by": null, "body": body });
        Line::Entry(read(entry, "a written fixture", n))
    };
    written.into_iter().enumerate().map(line).collect()
}

fn show(panel: Panel, route: &str, metrics: Option<&[&str]>, groups: bool) -> (JournalKind, Value) {
    let highlight = groups.then(|| {
        json!([{ "refs": ["ffe868cd-a382-4dc7-8c27-cc41bf3433a9"], "note": "where stock totes arrive for picking",
                 "marks": 4, "on_floor": 4 }])
    });
    let body = json!({ "action": "show", "panel": panel, "resolved": [], "metrics": metrics, "highlight": highlight,
                       "route": route, "announce": "The agent opened a panel." });
    (JournalKind::PanelCommand, body)
}

/// Spec 03 section 3.3: the Runs panel headlines the `metrics` of the command it opened from, through
/// `highlights={seq}` on reload (`studio/api/src/results/headline.rs`), so a route may name a Runs command that carries
/// metrics and drew no groups; one that carries neither, or metrics off the Runs panel, it may not.
#[test]
fn highlights_may_name_a_runs_command_by_its_metrics() {
    let run = "/runs/01M5884M40PSVWCBM620T3QGHG";
    let named = format!("{run}?highlights=1");
    let reloaded = (JournalKind::MessageUser, json!({ "context": { "route": named } }));
    let headlined = fixture_of(vec![show(Panel::Runs, &named, Some(&["htm_cycle_ms"]), false), reloaded]);
    assert_eq!(follows_the_grammar(&headlined), Ok(()));
    let unnamed = fixture_of(vec![show(Panel::Runs, run, Some(&["htm_cycle_ms"]), false)]);
    assert_eq!(follows_the_grammar(&unnamed), Ok(()));
    let unread = fixture_of(vec![show(Panel::Runs, &named, None, false)]);
    assert!(follows_the_grammar(&unread).is_err());
    let map = "/w/customer/demo/site/lshape-6/map@1?highlights=1";
    let off_runs = fixture_of(vec![show(Panel::Map, map, Some(&["htm_cycle_ms"]), false)]);
    assert!(follows_the_grammar(&off_runs).is_err());
}

/// The browser reads `highlights` whole and takes only a positive integer without leading zeros
/// (`studio/ui/src/shell/routes.ts`), so the check reads it whole too: a route written alone to its end, one written
/// in Markdown to the `)` that closes it.
#[test]
fn highlights_are_read_whole_as_a_canonical_positive_integer() {
    let map = "/w/customer/demo/site/lshape-6/map@1";
    let routed = |value: &str| fixture_of(vec![show(Panel::Map, &format!("{map}?highlights={value}"), None, true)]);
    assert_eq!(follows_the_grammar(&routed("1")), Ok(()));
    for value in ["1oops", "1.5", "01", "1)", "+1", "0", ""] {
        assert!(follows_the_grammar(&routed(value)).is_err(), "highlights={value}");
    }
    let linked = json!({ "text": format!("The [stock inlets]({map}?highlights=1) are drawn.") });
    let drawn = show(Panel::Map, &format!("{map}?highlights=1"), None, true);
    assert_eq!(follows_the_grammar(&fixture_of(vec![drawn, (JournalKind::TextDone, linked)])), Ok(()));
}
