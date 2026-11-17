//! Spec 01 section 3.5's hub without a database: text deltas coalesced per block per 50 ms of wall clock, the text so
//! far handed to a subscriber joining mid-block, offsets in UTF-16 code units, live items, and a subscriber that falls
//! more than 1,000 items behind.

use std::time::Duration;

use serde_json::json;
use studio_api::journal::{Hub, HubItem, HubRecvError, LiveRefused, SUBSCRIBER_BUFFER};
use studio_protocol::{BlockId, LiveItem, LiveKind, StreamingBlock, TextDelta, TextOffset, Ulid};
use tokio::time::{sleep, timeout};

fn conversation() -> Ulid {
    "01KN0TJZANP5N0PGJD8F7A3FD1".parse().expect("a ULID")
}

fn delta(block: &str, offset: u64, text: &str) -> TextDelta {
    TextDelta { block: BlockId::new(block), offset: TextOffset::new(offset), delta: text.to_owned() }
}

/// Spec 01 section 3.5, "Text deltas": a block's first delta reaches a subscriber at once, and those that follow
/// within 50 ms of wall clock reach it as one event, placed where the subscriber's text ends.
#[tokio::test]
async fn deltas_within_50_ms_reach_a_subscriber_as_one_event() {
    let hub = Hub::new();
    let mut subscription = hub.subscribe(&conversation());
    hub.text_delta(&conversation(), delta("b1", 0, "Hel")).expect("placed");
    assert_eq!(subscription.try_recv(), Ok(Some(HubItem::Delta(delta("b1", 0, "Hel")))));
    hub.text_delta(&conversation(), delta("b1", 3, "lo, ")).expect("placed");
    hub.text_delta(&conversation(), delta("b1", 7, "world")).expect("placed");
    assert_eq!(subscription.try_recv(), Ok(None), "held back for the rest of the 50 ms");
    let coalesced = timeout(Duration::from_millis(500), subscription.recv()).await.expect("within 500 ms");
    assert_eq!(coalesced, Ok(HubItem::Delta(delta("b1", 3, "lo, world"))));
    sleep(Duration::from_millis(120)).await;
    assert_eq!(subscription.try_recv(), Ok(None), "one event for the two deltas");
}

/// Spec 01 section 3.5, "Text deltas": a subscriber joining while a block streams is handed the block's text so far,
/// deltas not yet sent included, which its stream sends as one delta from offset 0; the hub's later deltas continue
/// from there.
#[tokio::test]
async fn a_subscriber_joining_mid_block_is_handed_the_text_so_far() {
    let hub = Hub::new();
    hub.text_delta(&conversation(), delta("b1", 0, "Hello, ")).expect("placed");
    hub.text_delta(&conversation(), delta("b1", 7, "world")).expect("placed");
    let mut joining = hub.subscribe(&conversation());
    let so_far = [StreamingBlock { block: BlockId::new("b1"), text: "Hello, world".to_owned() }];
    assert_eq!(joining.streaming(), so_far);
    assert_eq!(hub.streaming(&conversation()), so_far);
    let next = timeout(Duration::from_millis(500), joining.recv()).await.expect("the held-back delta comes");
    assert_eq!(next, Ok(HubItem::Delta(delta("b1", 7, "world"))), "a duplicate of what it was handed is harmless");
}

/// Spec 01 section 3.5: offsets count UTF-16 code units, as a page's strings do; a delta placed twice changes
/// nothing, one past the text so far is refused, and so is one inside a character.
#[tokio::test]
async fn offsets_count_utf16_code_units_and_a_repeat_changes_nothing() {
    let hub = Hub::new();
    hub.text_delta(&conversation(), delta("b1", 0, "Pick 😀 rate")).expect("placed");
    hub.text_delta(&conversation(), delta("b1", 0, "Pick")).expect("a repeat of the start");
    hub.text_delta(&conversation(), delta("b1", 12, " up")).expect("placed after the emoji's two units");
    assert_eq!(hub.streaming(&conversation())[0].text, "Pick 😀 rate up");
    let past = hub.text_delta(&conversation(), delta("b1", 99, "x")).expect_err("a gap");
    assert!(matches!(past, LiveRefused::Gap { have: 15, .. }), "{past}");
    let inside = hub.text_delta(&conversation(), delta("b1", 6, "x")).expect_err("inside the emoji");
    assert!(matches!(inside, LiveRefused::Split { .. }), "{inside}");
}

/// Spec 01 section 3.5: a live-only item reaches every subscriber; with none, it goes nowhere; a `text.delta` handed
/// in as a live item is placed like any delta.
#[tokio::test]
async fn live_items_reach_every_subscriber() {
    let hub = Hub::new();
    let progress = LiveItem { kind: LiveKind::RunProgress, body: json!({ "run": "r", "sim_ms": 90000 }) };
    hub.live(&conversation(), progress.clone()).expect("nobody listens, which is no fault");
    let (mut first, mut second) = (hub.subscribe(&conversation()), hub.subscribe(&conversation()));
    hub.live(&conversation(), progress.clone()).expect("delivered");
    assert_eq!(first.try_recv(), Ok(Some(HubItem::Live(progress.clone()))));
    assert_eq!(second.try_recv(), Ok(Some(HubItem::Live(progress))));
    let as_live = LiveItem { kind: LiveKind::TextDelta, body: json!({ "block": "b1", "offset": 0, "delta": "Hi" }) };
    hub.live(&conversation(), as_live).expect("placed");
    assert_eq!(first.try_recv(), Ok(Some(HubItem::Delta(delta("b1", 0, "Hi")))));
    let malformed = LiveItem { kind: LiveKind::TextDelta, body: json!({ "block": "b1" }) };
    assert!(matches!(hub.live(&conversation(), malformed), Err(LiveRefused::Body(_))));
}

/// Spec 01 section 3.5, "Delivery": a subscriber's buffer holds 1,000 items; with 1,000 unread it still hears every
/// one, and one more makes it lag, whatever it still holds.
#[tokio::test]
async fn a_subscriber_more_than_1000_items_behind_lags() {
    let hub = Hub::new();
    let item = |n: usize| LiveItem { kind: LiveKind::TemplateEvaluation, body: json!({ "n": n }) };
    let mut subscription = hub.subscribe(&conversation());
    for n in 0..SUBSCRIBER_BUFFER {
        hub.live(&conversation(), item(n)).expect("delivered");
    }
    for n in 0..SUBSCRIBER_BUFFER {
        assert_eq!(subscription.try_recv(), Ok(Some(HubItem::Live(item(n)))));
    }
    for n in 0..=SUBSCRIBER_BUFFER {
        hub.live(&conversation(), item(n)).expect("delivered");
    }
    assert_eq!(subscription.recv().await, Err(HubRecvError::Lagged));
    let mut other = hub.subscribe(&conversation());
    hub.live(&conversation(), item(0)).expect("delivered");
    assert_eq!(other.try_recv(), Ok(Some(HubItem::Live(item(0)))), "the lag was that subscriber's alone");
}
