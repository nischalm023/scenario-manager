//! Spec 01 section 3.6's `GET /api/sites` against the client's in-memory mock: every `customer/*/site/*` root that has
//! landed a changeset, named by its root's prologue, newest change first, with its live conversation; a root standing
//! at changeset 0 unlisted until a changeset writes it (spec 04 statement 14's listing half); opening the conversation
//! of a site the list leaves out answered `not-found` (brief I1.06b); and a failing world-proto-server answered as its
//! problem.

mod support;

use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;
use studio_api::projects::make_local_default;
use studio_api::{GateState, gate_routes};
use studio_protocol::{DeclaredId, ScopePath, SiteList};
use tokio::time::sleep;
use wps_client::draft::Op;
use wps_client::{MockWps, WorldStore};

/// The service with its default project, over a mock world holding no site yet.
async fn ready(pool: &PgPool) -> (GateState<MockWps>, Router) {
    make_local_default(pool).await.expect("the default project");
    let gate = GateState::new(support::state(pool.clone(), support::gate_rules()), support::world().await);
    (gate.clone(), gate_routes(gate))
}

async fn sites(router: &Router, query: &str) -> (StatusCode, Value) {
    support::json(router, "GET", &format!("/api/sites{query}"), "u-1", None).await
}

/// The roots a list answer names, in its order.
fn roots(answer: &Value) -> Vec<String> {
    let list: SiteList = serde_json::from_value(answer.clone()).expect("a site list");
    list.items.into_iter().map(|item| item.site.to_string()).collect()
}

async fn open(router: &Router, site: &str) -> (StatusCode, Value) {
    support::json(router, "POST", "/api/conversations", "u-1", Some(json!({ "site": site }))).await
}

fn declare(components: Value) -> Op {
    Op::Declare { id: DeclaredId::mint(), name: None, components: components.as_object().cloned().expect("an object") }
}

fn below(root: &ScopePath, sub: &str) -> ScopePath {
    root.join(&sub.parse().expect("a slug")).expect("a sub-scope")
}

#[tokio::test]
async fn the_list_holds_each_made_site_by_its_name_newest_change_first() {
    let scratch = scratch_or_skip!();
    let (gate, router) = ready(&scratch.pool).await;
    let world = &gate.world;
    for (root, name) in [("customer/demo/site/lshape-6", "L-Shape 6"), ("customer/demo/site/ushape-4", "U-Shape 4")] {
        support::made_site(world, root, name).await;
        sleep(Duration::from_millis(5)).await;
    }
    support::made_site(world, "customer/other/site/a", "Other A").await;
    let template: ScopePath = "template/site/l-shape".parse().expect("a template's root");
    support::made_scopes(world, &template).await;
    support::stage(world, &template, vec![declare(json!({ "frame": {} }))], None).await;
    let (status, opened) = open(&router, "customer/demo/site/lshape-6").await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, all) = sites(&router, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(roots(&all), ["customer/other/site/a", "customer/demo/site/ushape-4", "customer/demo/site/lshape-6"]);
    let lshape = &all["items"][2];
    let root = "customer/demo/site/lshape-6";
    let heads = json!({ root: 1, format!("{root}/fleet"): 0, format!("{root}/flow"): 0, format!("{root}/map"): 1 });
    assert_eq!(lshape["name"], "L-Shape 6");
    assert_eq!(lshape["branches"], json!([{ "name": "main", "heads": heads }]));
    assert_eq!(lshape["tags"], json!([]));
    let root_path: ScopePath = root.parse().expect("a root");
    let landed = world.history(&root_path, None, 1).await.expect("the root's log")[0].applied_at.clone();
    let changed_at: DateTime<Utc> = serde_json::from_value(lshape["changed_at"].clone()).expect("a wall-clock time");
    assert_eq!(changed_at, DateTime::parse_from_rfc3339(&landed).expect("RFC 3339"), "the newest head's");
    let conversation = json!({ "id": opened["conversation"]["id"], "waiting": null, "working": false });
    assert_eq!(lshape["conversation"], conversation, "a conversation just opened has no turn running or waiting");
    assert_eq!(all["items"][1]["conversation"], Value::Null, "nobody opened U-Shape 4's");

    let (_, demo) = sites(&router, "?customer=demo").await;
    assert_eq!(roots(&demo), ["customer/demo/site/ushape-4", "customer/demo/site/lshape-6"]);
    let (_, nobody) = sites(&router, "?customer=nobody").await;
    assert_eq!(roots(&nobody), Vec::<String>::new());
    for refused in ["?customer=Not%20a%20slug", "?customer=demo/site"] {
        let (status, problem) = sites(&router, refused).await;
        assert_eq!((status, problem["type"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("validation-failed")));
    }
    scratch.close().await;
}

/// A root made by name and never written stands at changeset 0 and is no site, even with its `map` written; the
/// changeset that writes the root lists it by its prologue's name. Create site writes all four scopes in one
/// changeset, so a site it makes is never seen in between.
#[tokio::test]
async fn a_root_standing_at_changeset_0_is_unlisted_until_a_changeset_writes_it() {
    let scratch = scratch_or_skip!();
    let (gate, router) = ready(&scratch.pool).await;
    let root: ScopePath = "customer/demo/site/unwritten".parse().expect("a root");
    support::made_scopes(&gate.world, &root).await;
    let frame = DeclaredId::mint();
    let frame_op = Op::Declare {
        id: frame,
        name: None,
        components: json!({ "frame": {} }).as_object().cloned().expect("an object"),
    };
    support::stage(&gate.world, &below(&root, "map"), vec![frame_op], None).await;

    let (_, listed) = sites(&router, "").await;
    assert_eq!(roots(&listed), Vec::<String>::new(), "the map has landed, the root has not");
    let (status, problem) = open(&router, root.as_str()).await;
    assert_eq!((status, problem["type"].as_str()), (StatusCode::NOT_FOUND, Some("not-found")), "{problem}");

    support::stage(&gate.world, &root, Vec::new(), Some(support::prologue("Written", frame))).await;
    let (_, listed) = sites(&router, "").await;
    assert_eq!((roots(&listed), &listed["items"][0]["name"]), (vec![root.to_string()], &json!("Written")));
    let (status, _) = open(&router, root.as_str()).await;
    assert_eq!(status, StatusCode::CREATED);
    scratch.close().await;
}

/// Spec 04 section 3.8's table on a route: a log world-proto-server does not answer is `wps-unavailable`.
#[tokio::test]
async fn a_failing_world_proto_server_answers_its_problem() {
    let scratch = scratch_or_skip!();
    let (gate, router) = ready(&scratch.pool).await;
    let root = support::made_site(&gate.world, "customer/demo/site/silent", "Silent").await;
    gate.world.wps().make_log_unreadable(&root, true);
    let (status, problem) = sites(&router, "").await;
    assert_eq!((status, problem["type"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("wps-unavailable")));
    let (status, problem) = open(&router, root.as_str()).await;
    assert_eq!((status, problem["type"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("wps-unavailable")));
    scratch.close().await;
}
