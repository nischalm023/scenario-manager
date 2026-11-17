//! Spec 01 section 3.11's person's draft changesets through the API's router (brief I1.11), on the L-Shape made at 6
//! aisles: the fold and a stale revision (spec 01 statement 5's draft half), the diff a read under a draft draws
//! (spec 06 statement 12's API half), Apply's compile gate, a warnings-only apply and the read precondition (spec 06
//! statement 8), and the apply path on the phase-3 client: the claim, a lost answer settled by the replay, a refusal
//! that reopens the draft, and a start resuming a draft left being applied.

mod support;

use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use studio_api::GateState;
use studio_api::drafts::recover;
use studio_protocol::{DeclaredId, ScopePath};
use support::Scratch;
use support::l_shape::{SITE, call, ready, send_as};
use tokio::time::sleep;
use wps_client::Op;
use wps_client::wps::Wps;

const FACTOR: &str = "/flow.node/intake/0/admission_factor";

/// The reference of the node named `name` in the flow at the site's head, or with `sector` of the map sector.
async fn named(router: &Router, name: &str, sector: bool) -> String {
    let (status, read) = call(router, "GET", &format!("/api/flow/{SITE}/flow"), None).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    let nodes = read["nodes"].as_array().unwrap();
    let sectors: Vec<&Value> = nodes.iter().flat_map(|node| node["sectors"].as_array().unwrap()).collect();
    let mut entities = if sector { sectors } else { nodes.iter().collect() }.into_iter();
    let found = entities.find(|entity| entity["name"] == name);
    found.unwrap_or_else(|| panic!("nothing is named {name}"))["ref"].as_str().unwrap().to_owned()
}

fn patch(target: &str, pointer: &str, value: Value) -> Value {
    json!({ "op": "patch", "target": target, "pointer": pointer, "value": value })
}

/// The changesets the flow read at the site's heads resolved, as the Flow panel sends them with its first edit.
async fn read_seqs(router: &Router) -> Value {
    let (status, read) = call(router, "GET", &format!("/api/flow/{SITE}/flow"), None).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    read["seqs"].clone()
}

/// The first edit's body: the site as the panel read it and the changesets that read resolved.
fn first_edit(seqs: Value, ops: Vec<Value>) -> Value {
    json!({ "site": format!("{SITE}@main"), "seqs": seqs, "summary": "A change from the Flow panel", "ops": ops })
}

/// The person's draft made by its first edit, as the Flow panel makes one, at the cut it has just read.
async fn made(router: &Router, ops: Vec<Value>) -> Value {
    let body = first_edit(read_seqs(router).await, ops);
    let (status, made) = call(router, "POST", "/api/drafts", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    made
}

/// Press Apply on `draft`: the status and, for a `202`, the approval it answers.
async fn press(router: &Router, draft: &Value) -> (StatusCode, Value) {
    call(router, "POST", &format!("/api/drafts/{}/apply", draft["draft"].as_str().unwrap()), None).await
}

/// Read approval `id` until its execution is one of `ended` (up to thirty seconds of wall clock).
async fn execution(router: &Router, id: &str, ended: &[&str]) -> Value {
    for _ in 0..1200 {
        let (_, read) = call(router, "GET", &format!("/api/approvals/{id}"), None).await;
        if ended.iter().any(|state| read["approval"]["execution"] == *state) {
            return read["approval"].clone();
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!("approval {id} never reached {ended:?}");
}

/// Press Apply, then read the draft once the press's execution has ended.
async fn applied(router: &Router, draft: &Value) -> (StatusCode, Value) {
    let (status, pressed) = press(router, draft).await;
    if status != StatusCode::ACCEPTED {
        return (status, pressed);
    }
    execution(router, pressed["approval"]["id"].as_str().unwrap(), &["ok", "failed"]).await;
    call(router, "GET", &format!("/api/drafts/{}", draft["draft"].as_str().unwrap()), None).await
}

/// The kinds of the entries in the site's conversations, in order, and the bodies of those of `kind`.
async fn journaled(pool: &PgPool, kind: &str) -> (Vec<String>, Vec<Value>) {
    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT j.kind, j.body FROM journal j JOIN conversations c ON c.id = j.conversation WHERE c.site = $1 \
         ORDER BY j.seq",
    )
    .bind(SITE)
    .fetch_all(pool)
    .await
    .unwrap();
    let bodies = rows.iter().filter(|(found, _)| found == kind).map(|(_, body)| body.clone()).collect();
    (rows.into_iter().map(|(kind, _)| kind).collect(), bodies)
}

/// The flow's head as the read at the site's heads finds it.
async fn flow_head(router: &Router) -> u64 {
    let (_, read) = call(router, "GET", &format!("/api/flow/{SITE}/flow"), None).await;
    read["seqs"][format!("{SITE}/flow")].as_u64().unwrap()
}

/// Spec 01 statement 5's draft half: a field edited twice is one operation and a field set back none, through the
/// routes; a `PATCH` at a stale revision answers `draft-changed` and changes nothing; only the draft's person
/// changes it.
#[tokio::test]
async fn a_draft_folds_its_edits_and_refuses_a_stale_revision() {
    let Some(scratch) = Scratch::open().await else { return };
    let (_, router) = ready(&scratch.pool).await;
    let pick = named(&router, "Pick stations", false).await;
    let draft = made(&router, vec![patch(&pick, FACTOR, json!(5.0))]).await;
    assert_eq!((&draft["revision"], &draft["ops_total"], &draft["state"]), (&json!(1), &json!(1), &json!("ready")));
    assert_eq!(draft["parts"], json!([{ "scope": format!("{SITE}/flow"), "ops": 1 }]));
    let at = format!("/api/drafts/{}", draft["draft"].as_str().unwrap());
    let again = json!({ "revision": 1, "ops": [patch(&pick, FACTOR, json!(6.0))] });
    let (status, twice) = call(&router, "PATCH", &at, Some(again.clone())).await;
    assert_eq!((status, &twice["revision"], &twice["ops_total"]), (StatusCode::OK, &json!(2), &json!(1)), "{twice}");
    let (status, stale) = call(&router, "PATCH", &at, Some(again)).await;
    assert_eq!((status, &stale["type"]), (StatusCode::CONFLICT, &json!("draft-changed")), "{stale}");
    let (status, other) =
        send_as(&router, "someone-else", "PATCH", &at, Some(json!({ "revision": 2, "ops": [] }))).await;
    assert_eq!((status, &other["type"]), (StatusCode::FORBIDDEN, &json!("not-draft-owner")), "{other}");
    let (_, read) = call(&router, "GET", &at, None).await;
    assert_eq!((&read["revision"], &read["ops"]), (&json!(2), &json!([patch(&pick, FACTOR, json!(6.0))])));
    let back = json!({ "revision": 2, "ops": [patch(&pick, FACTOR, json!(4.0))] });
    let (status, none) = call(&router, "PATCH", &at, Some(back)).await;
    assert_eq!((status, &none["ops_total"], &none["parts"]), (StatusCode::OK, &json!(0), &json!([])), "{none}");
    scratch.close().await;
}

/// Spec 06 statement 12's API half: a read under a draft marks exactly the patched entity changed, with was and
/// becomes for exactly the patched field; once the draft is discarded the read names its state and draws nothing.
#[tokio::test]
async fn a_read_under_a_draft_marks_exactly_the_patched_entity() {
    let Some(scratch) = Scratch::open().await else { return };
    let (_, router) = ready(&scratch.pool).await;
    let pick = named(&router, "Pick stations", false).await;
    let draft = made(&router, vec![patch(&pick, FACTOR, json!(5.0))]).await;
    let id = draft["draft"].as_str().unwrap();
    let (status, read) = call(&router, "GET", &format!("/api/flow/{SITE}/flow?draft={id}"), None).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(
        (&read["draft"]["ref"], &read["draft"]["state"], &read["draft"]["revision"]),
        (&json!(id), &json!("ready"), &json!(1))
    );
    assert_eq!(read["draft"]["base"]["plan"]["roots"].as_array().map(Vec::len), Some(7), "the cut's own plan");
    let lists = ["nodes", "edges", "segments", "return_rules"];
    let marked: Vec<&Value> =
        lists.iter().flat_map(|list| read[list].as_array().unwrap()).filter(|e| e.get("diff").is_some()).collect();
    assert_eq!(marked.len(), 1, "exactly one entity is marked");
    assert_eq!(marked[0]["ref"], json!(pick));
    let fields = json!([{ "pointer": FACTOR, "was": 4.0, "becomes": 5.0 }]);
    assert_eq!(marked[0]["diff"], json!({ "op": "changed", "fields": fields }));
    assert_eq!(read["problems"], json!([]));
    let (status, _) = call(&router, "POST", &format!("/api/drafts/{id}/discard"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, after) = call(&router, "GET", &format!("/api/flow/{SITE}/flow?draft={id}"), None).await;
    assert_eq!(after["draft"]["state"], "discarded");
    assert!(lists.iter().flat_map(|list| after[list].as_array().unwrap()).all(|e| e.get("diff").is_none()));
    scratch.close().await;
}

/// Spec 06 statement 8, Apply's compile gate: a draft that empties a node is checked `drafting` with `node-empty`,
/// and its Apply answers 422 `validation-failed` with that problem and writes nothing; a sector edit aimed at a flow
/// entity makes no draft.
#[tokio::test]
async fn applying_a_draft_that_empties_a_node_is_refused() {
    let Some(scratch) = Scratch::open().await else { return };
    let (_, router) = ready(&scratch.pool).await;
    let pick = named(&router, "Pick stations", false).await;
    let astray = first_edit(read_seqs(&router).await, vec![patch(&pick, "/sector/members", json!([]))]);
    let (status, wrong) = call(&router, "POST", "/api/drafts", Some(astray)).await;
    assert_eq!((status, &wrong["type"]), (StatusCode::UNPROCESSABLE_ENTITY, &json!("validation-failed")), "{wrong}");
    let sector = named(&router, "Put stations", true).await;
    let draft = made(&router, vec![patch(&sector, "/sector/members", json!([]))]).await;
    let codes = |answer: &Value| -> Vec<String> {
        answer["problems"].as_array().unwrap().iter().map(|p| p["code"].as_str().unwrap().to_owned()).collect()
    };
    assert_eq!((&draft["state"], &draft["parts"][0]["scope"]), (&json!("drafting"), &json!(format!("{SITE}/map"))));
    assert!(codes(&draft).contains(&"node-empty".to_owned()), "{draft}");
    let (status, refused) = applied(&router, &draft).await;
    assert_eq!(
        (status, &refused["type"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("validation-failed")),
        "{refused}"
    );
    assert!(codes(&refused).contains(&"node-empty".to_owned()), "{refused}");
    let (_, read) = call(&router, "GET", &format!("/api/drafts/{}", draft["draft"].as_str().unwrap()), None).await;
    assert_eq!((&read["state"], &read["approval"]), (&json!("drafting"), &Value::Null));
    scratch.close().await;
}

/// Spec 06 statement 8: a draft whose would-be site holds only warnings applies, and the flow's head moves to the
/// changeset the apply wrote; here the warning is a node no edge touches, staged beside the made flow.
#[tokio::test]
async fn a_draft_with_only_warnings_applies_and_moves_the_head() {
    let Some(scratch) = Scratch::open().await else { return };
    let (gate, router) = ready(&scratch.pool).await;
    let flow: ScopePath = format!("{SITE}/flow").parse().unwrap();
    let lone = json!({ "flow.node": { "kind": "sink" } }).as_object().cloned().unwrap_or_else(Map::new);
    let declared = Op::Declare { id: DeclaredId::mint(), name: Some("Unused sink".into()), components: lone };
    let base = support::stage(&gate.world, &flow, vec![declared], None).await;
    let pick = named(&router, "Pick stations", false).await;
    let draft = made(&router, vec![patch(&pick, FACTOR, json!(5.0))]).await;
    let problems = draft["problems"].as_array().unwrap();
    assert!(!problems.is_empty() && problems.iter().all(|p| p["severity"] == "warning"), "{draft}");
    assert_eq!(draft["state"], "ready");
    let (status, read) = applied(&router, &draft).await;
    assert_eq!((status, &read["state"]), (StatusCode::OK, &json!("applied")), "{read}");
    let written = read["applied_seqs"][flow.as_str()].as_u64().expect("the flow was written");
    assert!(written > base.get(), "the flow's head moved past {base:?} to {written}");
    let (_, head) = call(&router, "GET", &format!("/api/flow/{SITE}/flow"), None).await;
    assert_eq!(head["seqs"][flow.as_str()], json!(written));
    let after = head["nodes"].as_array().unwrap().iter().find(|node| node["ref"] == json!(pick)).unwrap().clone();
    assert_eq!(after["intake"][0]["admission_factor"], json!(5.0));
    scratch.close().await;
}

/// Spec 06 statement 8, the read precondition in the sub-scope layout: a flow-only draft whose map moved after it was
/// built is refused `head-moved` at Apply, naming the map, and goes stale.
#[tokio::test]
async fn a_flow_only_draft_whose_map_moved_is_refused_head_moved() {
    let Some(scratch) = Scratch::open().await else { return };
    let (gate, router) = ready(&scratch.pool).await;
    let pick = named(&router, "Pick stations", false).await;
    let draft = made(&router, vec![patch(&pick, FACTOR, json!(5.0))]).await;
    let map: ScopePath = format!("{SITE}/map").parse().unwrap();
    let frame = json!({ "frame": {} }).as_object().cloned().unwrap_or_else(Map::new);
    support::stage(
        &gate.world,
        &map,
        vec![Op::Declare { id: DeclaredId::mint(), name: None, components: frame }],
        None,
    )
    .await;
    let (status, refused) = applied(&router, &draft).await;
    assert_eq!((status, &refused["type"]), (StatusCode::CONFLICT, &json!("head-moved")), "{refused}");
    let moved: Vec<&str> = refused["moved"].as_array().unwrap().iter().filter_map(|m| m["scope"].as_str()).collect();
    assert_eq!(moved, [map.as_str()]);
    let (_, read) = call(&router, "GET", &format!("/api/drafts/{}", draft["draft"].as_str().unwrap()), None).await;
    assert_eq!((&read["state"], &read["stale_reason"]["reason"]), (&json!("stale"), &json!("head-moved")));
    scratch.close().await;
}

/// Spec 01 section 3.11: a person's draft records the cut their panel read. When the map moves after the read and
/// before the first edit, the edit is refused `head-moved` naming the map, and no draft is made at the newer head
/// (Astra's task-11 read: the first edit took a newer cut than the person read).
#[tokio::test]
async fn a_first_edit_after_the_site_moved_is_refused_head_moved() {
    let Some(scratch) = Scratch::open().await else { return };
    let (gate, router) = ready(&scratch.pool).await;
    let pick = named(&router, "Pick stations", false).await;
    let seqs = read_seqs(&router).await;
    let map: ScopePath = format!("{SITE}/map").parse().unwrap();
    let frame = json!({ "frame": {} }).as_object().cloned().unwrap_or_else(Map::new);
    let declared = Op::Declare { id: DeclaredId::mint(), name: None, components: frame };
    let head = support::stage(&gate.world, &map, vec![declared], None).await;
    let body = first_edit(seqs.clone(), vec![patch(&pick, FACTOR, json!(5.0))]);
    let (status, refused) = call(&router, "POST", "/api/drafts", Some(body)).await;
    assert_eq!((status, &refused["type"]), (StatusCode::CONFLICT, &json!("head-moved")), "{refused}");
    let moved = json!([{ "scope": map.as_str(), "base": seqs[map.as_str()], "head": head.get() }]);
    assert_eq!(refused["moved"], moved);
    let (_, mine) = call(&router, "GET", &format!("/api/drafts?site={SITE}&mine=1"), None).await;
    assert_eq!(mine["items"], json!([]), "nothing was made");
    scratch.close().await;
}

/// Spec 06 sections 3.4 and 5, the stored-cut overlay: once the flow's head moves past a draft's cut, the read under
/// the draft still draws over the cut, its base plan is the cut's own plan, not the head's, and the parameter list
/// under the draft gives the cut's values (Astra's task-11 read: the comparison mixed the head into the base). The
/// head moves on a node the draft leaves alone, through a field the plan reads: the put stations' admission factor.
#[tokio::test]
async fn a_read_under_a_draft_keeps_its_base_at_the_drafts_cut() {
    let Some(scratch) = Scratch::open().await else { return };
    let (gate, router) = ready(&scratch.pool).await;
    let pick = named(&router, "Pick stations", false).await;
    let put = named(&router, "Put stations", false).await;
    let (_, before) = call(&router, "GET", &format!("/api/flow/{SITE}/flow"), None).await;
    let draft = made(&router, vec![patch(&pick, FACTOR, json!(5.0))]).await;
    let id = draft["draft"].as_str().unwrap();
    let flow: ScopePath = format!("{SITE}/flow").parse().unwrap();
    let export = gate.world.wps().export_scope(&flow, None).await.expect("the flow's export");
    let document: Value = serde_json::from_slice(&export.bytes).expect("the export reads as JSON");
    let entities = document["entities"].as_array().expect("the flow's entities");
    let stations = entities.iter().find(|entity| entity["id"] == put.as_str()).expect("the put node");
    let mut node = stations["components"]["flow.node"].clone();
    node["intake"][0]["admission_factor"] = json!(2.0);
    let later = json!({ "flow.node": node }).as_object().cloned().unwrap();
    let patched =
        Op::Patch { id: put.parse::<DeclaredId>().unwrap(), name: Some("Put stations".into()), components: later };
    support::stage(&gate.world, &flow, vec![patched], None).await;
    let (_, head) = call(&router, "GET", &format!("/api/flow/{SITE}/flow"), None).await;
    assert_ne!(head["plan"]["digest"], before["plan"]["digest"], "the head's plan moved");
    let (status, under) = call(&router, "GET", &format!("/api/flow/{SITE}/flow?draft={id}"), None).await;
    assert_eq!(status, StatusCode::OK, "{under}");
    assert_eq!(under["draft"]["base"]["plan"]["digest"], before["plan"]["digest"], "the base is the draft's cut");
    assert_eq!(under["seqs"], before["seqs"], "the overlay reads the draft's cut");
    let (status, listed) = call(&router, "GET", &format!("/api/flow-parameters/{SITE}/flow?draft={id}"), None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let factor = (listed["parameters"].as_array().unwrap().iter())
        .find(|p| p["ref"] == put.as_str() && p["pointer"] == FACTOR)
        .expect("the put stations' admission factor is listed");
    assert_eq!(factor["value"], json!(1.5), "the draft's cut holds 1.5 where the head holds 2");
    scratch.close().await;
}

/// Spec 01 section 3.11, the claim: a press answers 202 with its approval and claims the draft `applying`; while the
/// write is in doubt a second press answers that same approval, and PATCH and Discard answer `draft-applying`; the
/// draft then applies once, its edit and its ending told to the site's live conversation (spec 06 section 3.4), the
/// approval's execution beside them naming the draft (Astra's task-11 read, the claim and the draft-changed P2s).
#[tokio::test]
async fn a_claimed_draft_answers_one_approval_and_takes_no_edit_until_it_applies() {
    let Some(scratch) = Scratch::open().await else { return };
    let (gate, router) = ready(&scratch.pool).await;
    let pick = named(&router, "Pick stations", false).await;
    let draft = made(&router, vec![patch(&pick, FACTOR, json!(5.0))]).await;
    let (id, before) = (draft["draft"].as_str().unwrap().to_owned(), flow_head(&router).await);
    gate.world.wps().drop_next_unapplied();
    gate.world.wps().drop_next_unapplied();
    let (status, first) = press(&router, &draft).await;
    assert_eq!((status, &first["approval"]["tool"]), (StatusCode::ACCEPTED, &json!("changeset.apply")), "{first}");
    let approval = first["approval"]["id"].as_str().unwrap().to_owned();
    let (status, again) = press(&router, &draft).await;
    assert_eq!((status, &again["approval"]["id"]), (StatusCode::ACCEPTED, &json!(approval)), "the first press's");
    let (_, read) = call(&router, "GET", &format!("/api/drafts/{id}"), None).await;
    assert_eq!((&read["state"], &read["approval"]), (&json!("applying"), &json!(approval)));
    let edit = json!({ "revision": 1, "ops": [patch(&pick, FACTOR, json!(6.0))] });
    let (status, refused) = call(&router, "PATCH", &format!("/api/drafts/{id}"), Some(edit)).await;
    assert_eq!((status, &refused["type"]), (StatusCode::CONFLICT, &json!("draft-applying")), "{refused}");
    let (status, refused) = call(&router, "POST", &format!("/api/drafts/{id}/discard"), None).await;
    assert_eq!((status, &refused["type"]), (StatusCode::CONFLICT, &json!("draft-applying")), "{refused}");
    execution(&router, &approval, &["ok"]).await;
    let (_, read) = call(&router, "GET", &format!("/api/drafts/{id}"), None).await;
    assert_eq!(read["state"], "applied");
    assert_eq!(flow_head(&router).await, before + 1, "one changeset landed");
    let (kinds, executions) = journaled(&scratch.pool, "approval.execution").await;
    let changed = journaled(&scratch.pool, "panel.event").await.1;
    assert_eq!(changed.iter().map(|event| &event["kind"]).collect::<Vec<_>>(), [&json!("flow.draft_changed")]);
    assert_eq!(kinds.iter().filter(|kind| *kind == "draft.applied").count(), 1);
    let states: Vec<&Value> = executions.iter().map(|body| &body["execution"]).collect();
    assert_eq!((states.first(), states.last()), (Some(&&json!("running")), Some(&&json!("ok"))), "{executions:?}");
    assert!(executions.iter().all(|body| body["draft"] == json!(id)), "each names the draft it applies");
    scratch.close().await;
}

/// Spec 01 section 3.13 and spec 04 section 3.9: an answer lost after the write landed leaves the outcome unknown, never
/// failed; the same body is sent again under the same identifier, world-proto-server replays the first answer, and
/// the draft applies with one changeset written (Astra's task-11 read: a lost answer is never "wrote nothing").
#[tokio::test]
async fn a_lost_answer_after_the_write_landed_is_settled_by_the_replay() {
    let Some(scratch) = Scratch::open().await else { return };
    let (gate, router) = ready(&scratch.pool).await;
    let pick = named(&router, "Pick stations", false).await;
    let draft = made(&router, vec![patch(&pick, FACTOR, json!(5.0))]).await;
    let before = flow_head(&router).await;
    gate.world.wps().lose_next_answer();
    gate.world.wps().lose_next_answer();
    let (status, pressed) = press(&router, &draft).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{pressed}");
    let approval = pressed["approval"]["id"].as_str().unwrap();
    let settled = execution(&router, approval, &["ok", "failed"]).await;
    assert_eq!((&settled["execution"], &settled["outcome"]["replayed"]), (&json!("ok"), &json!(true)), "{settled}");
    assert_eq!(flow_head(&router).await, before + 1, "the replay wrote nothing more");
    let states: Vec<Value> = journaled(&scratch.pool, "approval.execution")
        .await
        .1
        .into_iter()
        .map(|body| body["execution"].clone())
        .collect();
    assert_eq!(states, [json!("running"), json!("unknown"), json!("ok")]);
    scratch.close().await;
}

/// Spec 01 section 3.11's exit for a definitive refusal (round 11 item 27(g)): world-proto-server refuses the body,
/// writing nothing, so the draft is open again, `drafting` with the refusal among its problems, its approval released
/// and failed with the refusal; a later press is a new approval and applies.
#[tokio::test]
async fn a_refusal_opens_the_draft_again_and_releases_its_approval() {
    let Some(scratch) = Scratch::open().await else { return };
    let (gate, router) = ready(&scratch.pool).await;
    let pick = named(&router, "Pick stations", false).await;
    let draft = made(&router, vec![patch(&pick, FACTOR, json!(5.0))]).await;
    let id = draft["draft"].as_str().unwrap();
    gate.world.wps().refuse_next("generation_refused", "a rule this client does not check refused it");
    let (status, pressed) = press(&router, &draft).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{pressed}");
    let first = pressed["approval"]["id"].as_str().unwrap().to_owned();
    let failed = execution(&router, &first, &["failed", "ok"]).await;
    assert_eq!(failed["execution"], "failed", "{failed}");
    let (_, read) = call(&router, "GET", &format!("/api/drafts/{id}"), None).await;
    assert_eq!((&read["state"], &read["approval"]), (&json!("drafting"), &Value::Null), "{read}");
    assert!(read["problems"].to_string().contains("a rule this client does not check"), "{read}");
    let (status, again) = press(&router, &draft).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{again}");
    let second = again["approval"]["id"].as_str().unwrap();
    assert_ne!(second, first, "a new approval");
    assert_eq!(execution(&router, second, &["ok", "failed"]).await["execution"], "ok");
    scratch.close().await;
}

/// Spec 01 section 3.11 and LEAD-DECISIONS entry 54: a start resumes a draft a stopped process left being applied,
/// under the approval that claimed it, from the body kept at the claim, while in one process an approval has one
/// worker: this process's recovery, run beside the press's worker, leaves it alone, and a restarted process's (its
/// own state, the same database and world) resumes it; the write lands once, and the worker that finds it settled
/// does nothing.
#[tokio::test]
async fn a_start_resumes_a_draft_left_being_applied() {
    let Some(scratch) = Scratch::open().await else { return };
    let (gate, router) = ready(&scratch.pool).await;
    let pick = named(&router, "Pick stations", false).await;
    let draft = made(&router, vec![patch(&pick, FACTOR, json!(5.0))]).await;
    let before = flow_head(&router).await;
    gate.world.wps().drop_next_unapplied();
    gate.world.wps().drop_next_unapplied();
    let (status, pressed) = press(&router, &draft).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{pressed}");
    let approval = pressed["approval"]["id"].as_str().unwrap();
    execution(&router, approval, &["unknown"]).await;
    assert_eq!(recover(&gate).await.unwrap(), 1, "the one draft left applying, held by this process's worker");
    let restarted =
        GateState { app: support::state(scratch.pool.clone(), support::gate_rules()), world: gate.world.clone() };
    assert_eq!(recover(&restarted).await.unwrap(), 1, "a restarted process resumes it");
    assert_eq!(execution(&router, approval, &["ok", "failed"]).await["execution"], "ok");
    sleep(Duration::from_secs(3)).await;
    assert_eq!(flow_head(&router).await, before + 1, "one changeset, though two processes sent the body");
    let (kinds, executions) = journaled(&scratch.pool, "approval.execution").await;
    assert_eq!(kinds.iter().filter(|kind| *kind == "draft.applied").count(), 1);
    let states: Vec<&Value> = executions.iter().map(|body| &body["execution"]).collect();
    assert_eq!(states, [&json!("running"), &json!("unknown"), &json!("ok")], "each move once, forward only");
    let (_, read) = call(&router, "GET", &format!("/api/drafts/{}", draft["draft"].as_str().unwrap()), None).await;
    assert_eq!(read["state"], "applied");
    scratch.close().await;
}
