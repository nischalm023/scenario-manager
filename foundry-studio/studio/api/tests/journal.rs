//! Spec 01 section 3.5's writer and hub against Postgres: section 8's statement 1 for the journal (gapless, ordered
//! numbers under concurrent writers), the dedup key, `conversation-closed`, spec 01 section 3.17's site form, and a
//! hub that hears only what its writer announced after the commit.

mod support;

use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::{Value, json};
use sqlx::PgPool;
use studio_api::journal::{Appended, Hub, HubItem, JournalError, NewEntry, append, append_to_site};
use studio_protocol::{BlockId, JournalKind, JournalSeq, SitePath, TextDelta, TextOffset, Ulid};
use tokio::task::JoinSet;

const SITE: &str = "customer/demo/site/lshape-6";

fn note(text: &str) -> NewEntry {
    NewEntry::new(JournalKind::SystemNote, json!({ "text": text }))
}

/// One writer's decision for its `i`-th append in a round: roll back, and repeat a key another writer may also hold.
/// A small xorshift keeps the pattern different per round and writer, and the same on every run.
fn decide(round: u64, writer: u64, i: u64) -> (bool, bool) {
    let mut x = 0x9E37_79B9_7F4A_7C15 ^ (round << 40) ^ (writer << 20) ^ i;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    (x.is_multiple_of(5), x % 7 < 2)
}

async fn rows(pool: &PgPool, conversation: &Ulid) -> Vec<(i64, Value, Option<String>)> {
    sqlx::query_as("SELECT seq, body, dedup_key FROM journal WHERE conversation = $1 ORDER BY seq")
        .bind(conversation.as_str())
        .fetch_all(pool)
        .await
        .expect("the journal reads")
}

async fn last_seq(pool: &PgPool, conversation: &Ulid) -> i64 {
    sqlx::query_scalar("SELECT last_seq FROM conversations WHERE id = $1")
        .bind(conversation.as_str())
        .fetch_one(pool)
        .await
        .expect("the conversation reads")
}

/// Spec 01 section 8, statement 1: writers appending at once, some rolling back and some repeating keys other
/// writers hold, leave a conversation's numbers gapless and ordered, from its first number to its `last_seq`; each
/// writer's entries keep the order it wrote them in; a key lands once; a rolled-back entry leaves no trace. Three
/// rounds, the third on a conversation whose numbers start past 1, as a site's next conversation's do.
#[tokio::test]
async fn concurrent_writers_leave_gapless_ordered_numbers() {
    let scratch = scratch_or_skip!();
    for (round, first_seq) in [(0_u64, 1_i64), (1, 1), (2, 413)] {
        let site = format!("customer/demo/site/round-{round}");
        let conversation = support::conversation(&scratch.pool, Some(&site), "live", "u-1", first_seq).await;
        let (writers, appends) = (6_u64, 15_u64 + round * 5);
        let mut tasks = JoinSet::new();
        for writer in 0..writers {
            let (pool, conversation) = (scratch.pool.clone(), conversation.clone());
            tasks.spawn(async move {
                let mut kept = Vec::new();
                for i in 0..appends {
                    let (roll_back, shared) = decide(round, writer, i);
                    let mut entry = NewEntry::new(JournalKind::SystemNote, json!({ "writer": writer, "i": i }));
                    entry.dedup_key = shared.then(|| format!("shared-{i}"));
                    let mut tx = pool.begin().await.expect("a transaction");
                    let appended = append(&mut tx, &conversation, entry).await.expect("an append");
                    if roll_back {
                        tx.rollback().await.expect("a rollback");
                    } else {
                        tx.commit().await.expect("a commit");
                        kept.push((i, appended));
                    }
                }
                (writer, kept)
            });
        }
        let mut kept = FxHashMap::default();
        let rolled_back =
            (0..writers).flat_map(|w| (0..appends).map(move |i| (w, i))).filter(|(w, i)| decide(round, *w, *i).0);
        assert!(rolled_back.count() > 0, "round {round} rolls some appends back");
        while let Some(done) = tasks.join_next().await {
            let (writer, appends) = done.expect("a writer finishes");
            kept.insert(writer, appends);
        }

        let rows = rows(&scratch.pool, &conversation).await;
        let numbers: Vec<i64> = rows.iter().map(|(seq, _, _)| *seq).collect();
        let expected: Vec<i64> = (first_seq..first_seq + rows.len() as i64).collect();
        assert_eq!(numbers, expected, "round {round}: gapless from the first number");
        assert_eq!(last_seq(&scratch.pool, &conversation).await, first_seq + rows.len() as i64 - 1);

        let mut keys = FxHashSet::default();
        let mut last_i: FxHashMap<u64, u64> = FxHashMap::default();
        for (seq, body, key) in &rows {
            let (writer, i) = (body["writer"].as_u64().expect("writer"), body["i"].as_u64().expect("i"));
            assert!(last_i.insert(writer, i).is_none_or(|before| before < i), "writer {writer} in order at {seq}");
            assert!(!decide(round, writer, i).0, "a rolled-back entry ({writer}, {i}) never lands");
            if let Some(key) = key {
                assert!(keys.insert(key.clone()), "{key} lands once");
            }
        }
        let mut held = 0;
        for (writer, appends) in kept {
            for (i, appended) in appends {
                match appended {
                    Appended::Wrote(written) => {
                        let seq = written.entry().seq.get() as i64;
                        assert!(rows.iter().any(|(at, body, _)| *at == seq && body == &written.entry().body));
                    }
                    Appended::Held(seq) => {
                        let holder = rows.iter().find(|(at, _, _)| *at == seq.get() as i64).expect("the holder");
                        assert_eq!(holder.2, Some(format!("shared-{i}")), "writer {writer}'s key was held");
                        held += 1;
                    }
                }
            }
        }
        assert!(held > 0, "round {round} repeats some keys another writer holds");
    }
    scratch.close().await;
}

/// Spec 01 section 3.5, "Writers": a dedup key the conversation holds writes nothing and names the entry that holds
/// it, and the next entry takes the next number, so the sequence stays gapless.
#[tokio::test]
async fn a_held_dedup_key_writes_nothing_and_leaves_no_gap() {
    let scratch = scratch_or_skip!();
    let conversation = support::conversation(&scratch.pool, Some(SITE), "live", "u-1", 1).await;
    let keyed = || {
        let mut entry = NewEntry::new(JournalKind::RunStatus, json!({ "run": "01KN0TJZANP5N0PGJD8F7A3FDB" }));
        entry.dedup_key = Some("run.status:01KN0TJZANP5N0PGJD8F7A3FDB:1:running".to_owned());
        entry
    };
    let mut tx = scratch.pool.begin().await.expect("a transaction");
    assert!(matches!(append(&mut tx, &conversation, note("first")).await, Ok(Appended::Wrote(_))));
    let Ok(Appended::Wrote(first)) = append(&mut tx, &conversation, keyed()).await else { panic!("written") };
    assert_eq!(append(&mut tx, &conversation, keyed()).await.ok(), Some(Appended::Held(first.entry().seq)));
    let Ok(Appended::Wrote(next)) = append(&mut tx, &conversation, note("next")).await else { panic!("written") };
    tx.commit().await.expect("a commit");
    assert_eq!((first.entry().seq, next.entry().seq), (JournalSeq::new(2), JournalSeq::new(3)));
    assert_eq!(rows(&scratch.pool, &conversation).await.len(), 3);
    scratch.close().await;
}

/// Spec 01 section 3.17 and section 3.14's `conversation-closed`: an archived or discarded conversation takes no
/// entry, and a conversation that does not exist is not found.
#[tokio::test]
async fn a_closed_conversation_takes_no_entry() {
    let scratch = scratch_or_skip!();
    let archived = support::conversation(&scratch.pool, Some(SITE), "archived", "u-1", 1).await;
    let discarded = support::conversation(&scratch.pool, None, "discarded", "u-1", 1).await;
    let mut tx = scratch.pool.begin().await.expect("a transaction");
    for closed in [&archived, &discarded] {
        let refused = append(&mut tx, closed, note("too late")).await.expect_err("closed");
        assert!(matches!(refused, JournalError::Closed { .. }), "{refused}");
        let (status, problem) = (refused.answer("/x").0.status, refused.answer("/x").0.type_);
        assert_eq!((status, serde_json::to_value(problem).expect("a type")), (409, json!("conversation-closed")));
    }
    let missing = append(&mut tx, &support::ulid(), note("nobody")).await.expect_err("missing");
    assert!(matches!(missing, JournalError::NotFound(_)), "{missing}");
    tx.rollback().await.expect("a rollback");
    scratch.close().await;
}

/// Spec 01 section 3.17's last paragraph: the site form appends to the site's live conversation; a key an archived
/// conversation of the site holds writes nothing anywhere; a site nobody opened has no live conversation.
#[tokio::test]
async fn the_site_form_appends_to_the_live_conversation_once_per_key() {
    let scratch = scratch_or_skip!();
    let site: SitePath = SITE.parse().expect("a site path");
    let archived = support::conversation(&scratch.pool, Some(SITE), "archived", "u-1", 1).await;
    sqlx::query(
        "INSERT INTO journal (conversation, seq, kind, body, dedup_key) \
         VALUES ($1, 1, 'run.status', '{}', 'run.status:r:1:running')",
    )
    .bind(archived.as_str())
    .execute(&scratch.pool)
    .await
    .expect("the archived conversation holds a key");
    sqlx::query("UPDATE conversations SET last_seq = 1 WHERE id = $1")
        .bind(archived.as_str())
        .execute(&scratch.pool)
        .await
        .expect("its counter");
    let live = support::conversation(&scratch.pool, Some(SITE), "live", "u-1", 2).await;

    let keyed = |key: &str| {
        let mut entry = NewEntry::new(JournalKind::RunStatus, json!({}));
        entry.dedup_key = Some(key.to_owned());
        entry
    };
    let mut tx = scratch.pool.begin().await.expect("a transaction");
    let held = append_to_site(&mut tx, &site, keyed("run.status:r:1:running")).await.expect("an append");
    assert_eq!(held, Appended::Held(JournalSeq::new(1)));
    let Appended::Wrote(written) =
        append_to_site(&mut tx, &site, keyed("run.status:r:1:finished")).await.expect("an append")
    else {
        panic!("a new key is written");
    };
    assert_eq!((&written.entry().conversation, written.entry().seq), (&live, JournalSeq::new(2)));
    let unopened: SitePath = "customer/demo/site/nobody-yet".parse().expect("a site path");
    let refused = append_to_site(&mut tx, &unopened, keyed("k")).await.expect_err("no live conversation");
    assert!(matches!(refused, JournalError::NoLiveConversation(_)), "{refused}");
    tx.commit().await.expect("a commit");
    assert_eq!(rows(&scratch.pool, &archived).await.len(), 1, "nothing lands in the archived conversation");
    scratch.close().await;
}

/// Spec 01 section 3.5, "Writers": a subscriber hears an entry only once its writer announces it, which it does
/// after the commit; a `text.done` ends its block's streaming and a `turn.finished` every block's.
#[tokio::test]
async fn the_hub_hears_what_was_announced_after_the_commit() {
    let scratch = scratch_or_skip!();
    let conversation = support::conversation(&scratch.pool, Some(SITE), "live", "u-1", 1).await;
    let hub = Hub::new();
    let mut subscription = hub.subscribe(&conversation);
    for block in ["b1", "b2"] {
        let delta = TextDelta { block: BlockId::new(block), offset: TextOffset::new(0), delta: "Reading".to_owned() };
        hub.text_delta(&conversation, delta).expect("a delta");
        assert!(matches!(subscription.recv().await, Ok(HubItem::Delta(_))));
    }

    let mut tx = scratch.pool.begin().await.expect("a transaction");
    let done = NewEntry::new(JournalKind::TextDone, json!({ "block": "b1", "kind": "text", "text": "Reading." }));
    let Appended::Wrote(written) = append(&mut tx, &conversation, done).await.expect("an append") else { panic!() };
    assert_eq!(subscription.try_recv(), Ok(None), "nothing is heard before the announcement");
    tx.commit().await.expect("a commit");
    hub.announce([written]);
    assert_eq!(subscription.recv().await.ok(), Some(HubItem::Wake(JournalSeq::new(1))));
    let streaming: Vec<String> =
        hub.streaming(&conversation).into_iter().map(|block| block.block.to_string()).collect();
    assert_eq!(streaming, ["b2"]);

    let mut tx = scratch.pool.begin().await.expect("a transaction");
    let finished = NewEntry::new(JournalKind::TurnFinished, json!({ "outcome": "stopped" }));
    let Appended::Wrote(written) = append(&mut tx, &conversation, finished).await.expect("an append") else { panic!() };
    tx.commit().await.expect("a commit");
    hub.announce([written]);
    assert_eq!(subscription.recv().await.ok(), Some(HubItem::Wake(JournalSeq::new(2))));
    assert!(hub.streaming(&conversation).is_empty());
    scratch.close().await;
}
