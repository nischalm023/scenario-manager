//! The Map panel's routes (spec 03 sections 3.5, 3.6 and 8) over an L-Shape made from the development seed's shell
//! template in the client's mock (`support::made`): a site's floor under its key, the marks a reference covers, find,
//! and the summary. The routes are called without the identity layer, which needs the database; the summary reads
//! made-from records from Postgres, so its tests skip without one.

mod support;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use studio_api::floors::{self, floor_document, key_text};
use studio_api::map::{SiteAt, site_at};
use studio_api::projects::make_local_default;
use studio_api::{AppState, GateState, gate_routes};
use studio_protocol::{BranchName, DeclaredId, EntityId, Pin, ProblemType, ScopePath, Seq};
use support::made::{world, world_holding};
use support::plan_site::{evaluated, made_site};
use worldview_render::FloorAndRoll;
use wps_client::draft::Op;
use wps_client::{Client, MockWps, WorldStore};

const SITE: &str = "customer/demo/site/lshape-6";
const MAP: &str = "customer/demo/site/lshape-6/map@main";

async fn made_with(world: Client<MockWps>, app: AppState) -> (GateState<MockWps>, Router) {
    made_site(&world, "l-shape", &[], SITE, "Small L-shape").await;
    let gate = GateState::new(app, world);
    let router = studio_api::map::routes().merge(floors::routes()).with_state(gate.clone());
    (gate, router)
}

async fn made() -> (GateState<MockWps>, Router) {
    made_with(world().await, support::state(support::unconnected_pool(), support::gate_rules())).await
}

async fn call(router: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Vec<u8>) {
    let request = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let (status, _, answer) = support::call(router, request.body(body).expect("a request")).await;
    (status, answer)
}

async fn answered(router: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let (status, answer) = call(router, method, uri, body).await;
    (status, serde_json::from_slice(&answer).unwrap_or(Value::Null))
}

async fn at(gate: &GateState<MockWps>, location: &str) -> SiteAt {
    site_at(gate, location, None, "/test").await.expect("the made site reads")
}

/// The entity of the made L-Shape carrying `component` whose label is `label`.
fn named(read: &SiteAt, component: &str, label: &str) -> EntityId {
    let loaded = read.loaded.as_ref().as_ref().expect("loaded");
    let mut found: Vec<&EntityId> = loaded
        .entities
        .iter()
        .filter(|(_, entry)| entry.label == label && entry.components.iter().any(|name| name == component))
        .map(|(id, _)| id)
        .collect();
    found.sort_by_key(|id| id.to_string());
    found.first().map(|id| (*id).clone()).unwrap_or_else(|| panic!("no {component} named {label}"))
}

/// Statement 6, floor parity: the floor document under a site's key is the floor compiled from the very export a run
/// of that cut loads, whose sha256 the key names; a prologue change at a later head gives a new key, and the old key
/// keeps its old bytes.
#[tokio::test]
async fn a_sites_floor_under_its_key_is_the_floor_of_the_export_a_run_loads() {
    let (gate, router) = made().await;
    let read = at(&gate, MAP).await;
    let export = gate.world.export(&read.cut).await.expect("the cut's export");
    assert_eq!(read.key.document, hex(&Sha256::digest(&export.bytes)), "the key names the bytes' sha256");
    let (status, served) = call(&router, "GET", &format!("/api/floors/{}", key_text(&read.key)), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(served, floor_document(&export.bytes).expect("compiles").to_vec(), "byte for byte");

    let root: ScopePath = SITE.parse().unwrap();
    let frame = read.loaded.as_ref().as_ref().unwrap().primary.declared();
    support::stage(&gate.world, &root, Vec::new(), Some(support::prologue("Small L-shape, renamed", frame))).await;
    let renamed = at(&gate, MAP).await;
    assert_ne!(renamed.key, read.key, "a later prologue is a later key");
    let (_, again) = call(&router, "GET", &format!("/api/floors/{}", key_text(&read.key)), None).await;
    assert_eq!(again, served, "and the old key's bytes stay what they were");
}

/// A floor the cache let go is compiled again from the cut its key was minted for.
#[tokio::test]
async fn a_floor_the_cache_let_go_is_compiled_again_from_its_cut() {
    let (gate, router) =
        made_with(world_holding(1).await, support::state(support::unconnected_pool(), support::gate_rules())).await;
    let read = at(&gate, MAP).await;
    assert!(!gate.world.floors().contains(&read.key), "a one-byte budget holds no floor");
    let (status, served) = call(&router, "GET", &format!("/api/floors/{}", key_text(&read.key)), None).await;
    assert_eq!(status, StatusCode::OK);
    let export = gate.world.export(&read.cut).await.unwrap();
    assert_eq!(served, floor_document(&export.bytes).unwrap().to_vec());
}

/// Statement 8, for a sector: the made L-Shape's "Pick stations" sector marks exactly its members, the four pick
/// stations, on the floor and in position order, identically on two reads.
#[tokio::test]
async fn a_sector_marks_exactly_its_members_in_position_order_on_every_read() {
    let (gate, router) = made().await;
    let read = at(&gate, MAP).await;
    let sector = named(&read, "sector", "Pick stations");
    let body = json!({ "location": MAP, "panel": "map", "refs": [sector.to_string()] });
    let (status, first) = answered(&router, "POST", "/api/resolve", Some(body.clone())).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let (_, second) = answered(&router, "POST", "/api/resolve", Some(body)).await;
    assert_eq!(first, second, "identically on two reads");

    let marks = first["refs"][0]["marks"].as_array().expect("marks").clone();
    let loaded = read.loaded.as_ref().as_ref().unwrap();
    let mut members: Vec<String> = loaded.entities[&sector].members.iter().map(ToString::to_string).collect();
    let mut marked: Vec<String> = marks.iter().map(|mark| mark["key"].as_str().unwrap().to_owned()).collect();
    assert_eq!(marks.len(), 4);
    members.sort();
    marked.sort();
    assert_eq!(marked, members, "exactly the sector's members");
    assert!(
        marks.iter().all(|mark| mark["on_floor"] == json!(true) && mark["components"].to_string().contains("station"))
    );
    let centres: Vec<(i64, i64)> = marks
        .iter()
        .map(|mark| (mark["centre_mm"][0].as_i64().unwrap(), mark["centre_mm"][1].as_i64().unwrap()))
        .collect();
    let mut ordered = centres.clone();
    ordered.sort();
    assert_eq!(centres, ordered, "in position order, x then y");
    assert_eq!(first["resolved"].as_array().unwrap().len(), 4, "the cut: root, map, flow and fleet");
}

/// Spec 03 section 3.3, steps 1 and 4, as resolve answers them: a string off the grammar is named; a reference
/// naming nothing fails the whole call, naming each; a pinned reference inside the site breaks the pin rule, `@head`
/// is accepted; a location or a draft the panel does not show is a mismatch.
#[tokio::test]
async fn resolve_refuses_what_it_cannot_mark_and_names_it() {
    let (gate, router) = made().await;
    let read = at(&gate, MAP).await;
    let station = named(&read, "station", "pick station");
    let nobody = DeclaredId::mint().to_string();
    let cases = [
        (json!({ "location": MAP, "panel": "map", "refs": ["not a ref"] }), 422, "ref-malformed"),
        (json!({ "location": MAP, "panel": "map", "refs": [station.to_string(), nobody] }), 422, "ref-unresolved"),
        (json!({ "location": MAP, "panel": "map", "refs": [format!("{station}@1")] }), 422, "ref-pin-conflicts"),
        (
            json!({ "location": format!("{SITE}/flow@main"), "panel": "map", "refs": [station.to_string()] }),
            422,
            "panel-mismatch",
        ),
        (
            json!({ "location": MAP, "panel": "map", "draft": "01KN0TJZANP5N0PGJD8F7A3FDB", "refs": [station.to_string()] }),
            422,
            "panel-mismatch",
        ),
        (
            json!({ "location": "customer/demo/site/nowhere/map@main", "panel": "map", "refs": [station.to_string()] }),
            404,
            "location-unresolved",
        ),
    ];
    for (body, status, kind) in cases {
        let (got, problem) = answered(&router, "POST", "/api/resolve", Some(body.clone())).await;
        assert_eq!((got.as_u16(), problem["type"].as_str()), (status, Some(kind)), "{body}: {problem}");
    }
    let (_, unresolved) = answered(
        &router,
        "POST",
        "/api/resolve",
        Some(cases_body(MAP, &[&station.to_string(), "00000000-0000-4000-8000-000000000000"])),
    )
    .await;
    assert!(unresolved["detail"].as_str().unwrap().contains("00000000-0000-4000-8000-000000000000"), "{unresolved}");
    let (status, headed) =
        answered(&router, "POST", "/api/resolve", Some(cases_body(MAP, &[&format!("{station}@head")]))).await;
    assert_eq!(status, StatusCode::OK, "{headed}");
    assert_eq!(headed["refs"][0]["marks"][0]["key"], json!(station.to_string()));
    let (status, keys) = answered(
        &router,
        "POST",
        "/api/resolve",
        Some(json!({ "panel": "scenario", "refs": ["scenario:01KN0TJZANP5N0PGJD8F7A3FDB"] })),
    )
    .await;
    assert_eq!(
        (status, keys["refs"][0]["marks"][0]["key"].clone()),
        (StatusCode::OK, json!("scenario:01KN0TJZANP5N0PGJD8F7A3FDB"))
    );
}

fn cases_body(location: &str, refs: &[&str]) -> Value {
    json!({ "location": location, "panel": "map", "refs": refs })
}

/// Find: what holds the words in its label, a component's name or an external id, ignoring case, in mark order, at
/// most `limit`, saying whether more matched.
#[tokio::test]
async fn find_answers_matches_in_mark_order() {
    let (_, router) = made().await;
    let (status, found) = answered(&router, "GET", &format!("/api/find/{MAP}?q=PICK%20station"), None).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    let items = found["items"].as_array().unwrap();
    let stations =
        items.iter().filter(|item| item["on_floor"] == json!(true) && item["label"] == json!("pick station")).count();
    assert_eq!(stations, 4, "{found}");
    let placed: Vec<bool> = items.iter().map(|item| item["centre_mm"].is_array()).collect();
    assert!(placed.windows(2).all(|pair| pair[0] >= pair[1]), "placed marks first");
    let (_, two) = answered(&router, "GET", &format!("/api/find/{MAP}?q=pick&limit=2"), None).await;
    assert_eq!((two["items"].as_array().unwrap().len(), two["more"].clone()), (2, json!(true)));
}

/// Statement 10, the API's half: a flow changeset alone moves the cut's flow and gives a new floor key; the map
/// scope's own changeset stays.
#[tokio::test]
async fn a_flow_changeset_alone_gives_a_new_floor_key() {
    let (gate, _) = made().await;
    let before = at(&gate, MAP).await;
    let flow: ScopePath = format!("{SITE}/flow").parse().unwrap();
    let rule = json!({ "flow.return_rule": { "destination": "nearest_eligible", "min_backlog": 3 } });
    let op = Op::Declare {
        id: DeclaredId::mint(),
        name: Some("Return rule, again".into()),
        components: rule.as_object().cloned().unwrap(),
    };
    support::stage(&gate.world, &flow, vec![op], None).await;
    let after = at(&gate, MAP).await;
    assert_ne!(after.key, before.key, "a new floor key");
    assert_eq!(after.cut.seqs[&flow], Seq::new(before.cut.seqs[&flow].get() + 1), "the flow moved");
    let map: ScopePath = format!("{SITE}/map").parse().unwrap();
    assert_eq!(after.cut.seqs[&map], before.cut.seqs[&map], "the map did not");
}

/// The summary (spec 03 section 3.6): the cut, the site's name from its prologue, the floor's key, the frames, the
/// counts most first, and the map at a changeset needing `kin` until world-proto-server's phase 5.
#[tokio::test]
async fn the_summary_names_the_cut_the_floor_and_what_the_site_holds() {
    let scratch = scratch_or_skip!();
    let (gate, router) = made_with(world().await, support::state(scratch.pool.clone(), support::gate_rules())).await;
    let (status, summary) = answered(&router, "GET", &format!("/api/map/{SITE}@main"), None).await;
    assert_eq!(status, StatusCode::OK, "{summary}");
    assert_eq!(summary["location"], json!(MAP), "a site's root opens its map");
    assert_eq!(summary["site"], json!({ "scope": SITE, "label": "Small L-shape" }));
    let read = at(&gate, MAP).await;
    assert_eq!(summary["floor"]["key"], json!(key_text(&read.key)));
    assert_eq!(summary["axes"]["name"], json!("shell"), "the frame the wall knobs name");
    assert_eq!(summary["empty"], json!(false));
    let counts: Vec<u64> =
        summary["counts"].as_array().unwrap().iter().map(|row| row["count"].as_u64().unwrap()).collect();
    assert!(counts.windows(2).all(|pair| pair[0] >= pair[1]) && counts.len() > 5, "{counts:?}");
    assert_eq!(summary["resolved"].as_array().unwrap().len(), 4);
    assert_eq!(summary["made_from"], Value::Null, "no made-from record: this site was not made by Create site");

    let (status, waits) = answered(&router, "GET", &format!("/api/map/{SITE}/map@1"), None).await;
    assert_eq!((status.as_u16(), waits["type"].clone()), (501, json!("wps-not-ready")), "{waits}");
    // Every other scope named still leaves the root at its head: refused as well until phase 5 (entries 35 and 36).
    let (status, fixed) = answered(&router, "GET", &format!("/api/map/{SITE}/map@1?kin=flow@1,fleet@1"), None).await;
    assert_eq!((status.as_u16(), fixed["type"].clone()), (501, json!("wps-not-ready")), "{fixed}");
    let (status, flow) = answered(&router, "GET", &format!("/api/map/{SITE}/flow@main"), None).await;
    assert_eq!((status.as_u16(), flow["type"].clone()), (422, json!("panel-mismatch")));
    let _ = (Pin::Branch(BranchName::main()), &scratch);
    scratch.close().await;
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Spec 03 section 3.8's `floor-compile-failed`: a site whose document foundry's reader refuses says why in words,
/// at its summary and at its floor's key alike.
#[tokio::test]
async fn a_site_whose_document_will_not_load_says_why() {
    let scratch = scratch_or_skip!();
    let gate = support::gate(support::state(scratch.pool.clone(), support::gate_rules())).await;
    let router = studio_api::map::routes().merge(floors::routes()).with_state(gate.clone());
    let (status, summary) = answered(&router, "GET", "/api/map/customer/demo/site/ushape-4@main", None).await;
    assert_eq!((status.as_u16(), summary["type"].clone()), (422, json!("floor-compile-failed")), "{summary}");
    let detail = summary["detail"].as_str().unwrap();
    assert!(detail.starts_with("This map cannot be drawn: the site's document will not load: "), "{detail}");
    let read = at(&gate, "customer/demo/site/ushape-4/map@main").await;
    let (status, floor) = answered(&router, "GET", &format!("/api/floors/{}", key_text(&read.key)), None).await;
    assert_eq!((status.as_u16(), floor["type"].clone()), (422, json!("floor-compile-failed")), "{floor}");
    scratch.close().await;
}

/// Spec 03 section 3.5's `template` row: on a template draft, an entity of its newest ready evaluation marks itself on
/// that evaluation's floor roll, before any site exists; one the evaluation does not hold names nothing.
#[tokio::test]
async fn a_template_drafts_entity_marks_itself_on_its_evaluations_floor() {
    let scratch = scratch_or_skip!();
    make_local_default(&scratch.pool).await.expect("the default project");
    let gate = GateState::new(support::state(scratch.pool.clone(), support::gate_rules()), world().await);
    let router = gate_routes(gate);
    let request = support::from_gate("POST", "/api/template-drafts", "dev", "")
        .header("content-type", "application/json")
        .header("idempotency-key", format!("k{}", DeclaredId::mint()));
    let body = Body::from(json!({ "template": "template/site/l-shape@main" }).to_string());
    let (status, _, started) = support::call(&router, request.body(body).unwrap()).await;
    assert_eq!(status, StatusCode::CREATED);
    let draft = serde_json::from_slice::<Value>(&started).unwrap()["draft"].as_str().unwrap().to_owned();
    support::templates::settled(&router, &draft).await;
    let (_, evaluation) =
        support::json(&router, "GET", &format!("/api/template-drafts/{draft}/evaluations/1"), "dev", None).await;
    let floor_key = evaluation["floor"]["key"].as_str().expect("a floor").to_owned();
    let request =
        support::from_gate("GET", &format!("/api/floors/{floor_key}"), "dev", "").body(Body::empty()).unwrap();
    let (_, _, floor) = support::call(&router, request).await;
    let (_, roll) = FloorAndRoll::read(&floor).expect("a floor document");
    let standing = roll.iter().find(|entry| !entry.is_cell).map(|entry| entry.key.clone()).expect("a station stands");

    let location = format!("template-draft:{draft}");
    let body = json!({ "location": location, "panel": "template", "refs": [standing] });
    let (status, resolved) = support::json(&router, "POST", "/api/resolve", "dev", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{resolved}");
    let mark = &resolved["refs"][0]["marks"][0];
    assert_eq!((mark["key"].as_str(), mark["on_floor"].clone()), (Some(standing.as_str()), json!(true)));
    assert!(mark["centre_mm"].is_array(), "{mark}");
    let body = json!({ "location": location, "panel": "template", "refs": [DeclaredId::mint().to_string()] });
    let (status, refused) = support::json(&router, "POST", "/api/resolve", "dev", Some(body)).await;
    assert_eq!((status.as_u16(), refused["type"].clone()), (422, json!("ref-unresolved")), "{refused}");
    scratch.close().await;
}

/// Statement 11, and entries 35 and 36 of the lead's decisions: until world-proto-server keeps one commit order across a
/// site's scopes (phase 5), a site of several scopes is read only at a branch head; a changeset pin on its map, with
/// `kin` or without, is refused naming that rule, never read with the scopes it leaves out at their heads.
#[tokio::test]
async fn a_changeset_pin_over_a_site_of_several_scopes_is_refused_until_phase_5() {
    let (gate, _) = made().await;
    let location = "customer/demo/site/lshape-6/map@1";
    for kin in [Some("flow@1"), None] {
        let refused = match site_at(&gate, location, kin, "/test").await {
            Ok(read) => panic!("{location} {kin:?} read as a cut of {} scopes", read.cut.seqs.len()),
            Err(refused) => refused.0,
        };
        assert_eq!(refused.type_, ProblemType::WpsNotReady, "{location} {kin:?}: {}", refused.detail);
        assert!(
            refused.detail.contains("only at a branch head") && refused.detail.contains("phase 5"),
            "the rule named: {}",
            refused.detail
        );
    }
    assert!(site_at(&gate, MAP, None, "/test").await.is_ok(), "the map at main reads");
}

/// Spec 05 section 6 and spec 03 section 3.6 (Astra's task-14 review, the third P1): an evaluation carries the axes of
/// the frame its template's wall knobs name, the shell frame, and which way they point, for its stage and its text.
#[tokio::test]
async fn an_evaluation_names_the_frame_its_wall_knobs_name_and_where_its_axes_point() {
    let computed = evaluated(&world().await, "l-shape", &[], "Small L-shape").await;
    let axes = computed.axes.expect("a ready evaluation's axes");
    assert_eq!((axes.name.as_str(), axes.x, axes.y), ("shell", [1, 0], [0, 1]));
}
