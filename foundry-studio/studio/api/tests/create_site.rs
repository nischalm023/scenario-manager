//! Spec 05 section 4.4's Create site through `POST /api/template-drafts/{d}/site`, against the client's mock of scopes
//! phase 3 seeded by the development seed: one changeset making the site's root and its three sub-scopes, spec 01's
//! statements 13 (a press answers at once) and 16 (the made site binds the thread), the made-from record, a press or
//! a lost answer asked again writing nothing more, one worker per approval and a restarted API's settling of a site
//! that landed (LEAD-DECISIONS entry 54), and the refusals that leave nothing written.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use studio_api::approvals::{claim, executed};
use studio_api::templates::recover;
use studio_api::{GateState, floors, gate_routes};
use studio_protocol::{ApprovalState, ApprovalVia, BranchName, Execution, Pin, ScopePath, Ulid, WriteAccepted};
use support::templates::{draft, get, press, ready, send, settled, thread, written};
use tokio::time::sleep;
use wps_client::wire::{RefAnnotation, WireChangeset};
use wps_client::wps::Wps;
use wps_client::{BlockingCompute, Client, ClientConfig, WorldStore};

/// The roots `GET /api/sites` lists, in its order.
async fn listed(router: &Router) -> Vec<String> {
    let sites = get(router, "/api/sites").await;
    sites["items"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|item| item["site"].as_str().expect("a site").to_owned())
        .collect()
}

/// Each of the site's four scopes at changeset 1, as one changeset leaves them.
fn at_one(root: &str) -> Value {
    json!({ root: 1, format!("{root}/fleet"): 1, format!("{root}/flow"): 1, format!("{root}/map"): 1 })
}

/// Spec 05 section 9's Create site statements on the Rectangle and the L-Shape, with spec 01's statements 13 and 16:
/// while world-proto-server holds the write, the press answers `202` within 1 s of wall clock with the approval's
/// record, as Apply answers (spec 01 section 3.10, rule 6), a second press answers the same approval and the site is
/// unlisted; once it lands, one changeset made the root and its `map`, `flow` and
/// `fleet`, each at changeset 1, the site is listed once under its name, its made-from record keeps every knob's
/// value, the New site thread is bound to it, a press answers `draft-not-open` and the same key replays the `202`,
/// writing nothing more, and foundry's reader loads its export.
#[tokio::test]
async fn create_site_writes_one_changeset_answers_at_once_and_binds_the_thread() {
    let scratch = scratch_or_skip!();
    let (gate, router) = ready(&scratch.pool).await;
    for (shell, slug) in [("rectangle", "rectangle-1"), ("l-shape", "lshape-1")] {
        let conversation = thread(&router).await;
        let (id, _) = draft(&router, shell, Some(&conversation)).await;
        let root = format!("customer/demo/site/{slug}");
        let path: ScopePath = root.parse().expect("a site's root");
        gate.world.wps().hold(&path);
        let key = format!("press-{slug}");
        let pressed = Instant::now();
        let (status, _, accepted) = press(&router, &id, slug, 1, Some(&key)).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
        assert!(pressed.elapsed() < Duration::from_secs(1), "statement 13: {:?}", pressed.elapsed());
        let answer: WriteAccepted =
            serde_json::from_value(accepted.clone()).expect("the approval's record, as Apply's");
        let pressed =
            (answer.approval.via, answer.approval.state, answer.approval.tool.as_str(), answer.draft.as_str());
        assert_eq!(pressed, (ApprovalVia::Button, ApprovalState::Approved, "site.create_from_template", id.as_str()));
        let (status, _, again) = press(&router, &id, slug, 1, None).await;
        assert_eq!((status, &again["approval"]["id"]), (StatusCode::ACCEPTED, &accepted["approval"]["id"]), "rule 6");
        assert!(!listed(&router).await.contains(&root), "unlisted while its changeset waits");

        gate.world.wps().open();
        let read = written(&router, &id).await;
        assert_eq!((read["state"].as_str(), &read["write"]["result"]["site"]), (Some("created"), &json!(root)));
        assert_eq!(read["write"]["result"]["seqs"], at_one(&root), "one changeset wrote all four");
        let approval = get(&router, &format!("/api/approvals/{}", accepted["approval"]["id"].as_str().unwrap())).await;
        assert_eq!(
            (&approval["approval"]["via"], &approval["approval"]["execution"]),
            (&json!("button"), &json!("ok"))
        );
        assert_eq!(listed(&router).await.iter().filter(|site| **site == root).count(), 1);
        let made = get(&router, &format!("/api/made-from/{root}")).await;
        let items = made["items"].as_array().expect("the made-from records");
        assert_eq!((items.len(), &items[0]["site_seqs"], &items[0]["act"]), (1, &at_one(&root), &json!("create")));
        let declared = read["declarations"].as_array().expect("the knob declarations").len();
        assert_eq!(items[0]["knobs"].as_array().expect("its knobs").len(), declared, "every knob's value");

        let bound = get(&router, &format!("/api/conversations/{conversation}")).await;
        assert_eq!(
            (&bound["conversation"]["site"], &bound["conversation"]["title"]),
            (&json!(root), &json!(format!("Site {slug}")))
        );
        let kinds: Vec<&str> =
            bound["entries"].as_array().unwrap().iter().filter_map(|entry| entry["kind"].as_str()).collect();
        assert!(kinds.ends_with(&["approval.execution", "conversation.changed", "approval.execution"]), "{kinds:?}");
        let executed = bound["entries"].as_array().unwrap().last().unwrap()["body"].clone();
        assert_eq!((&executed["execution"], &executed["approver"]["subject"]), (&json!("ok"), &json!("dev")));

        let (status, _, refused) = press(&router, &id, slug, 1, None).await;
        assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("draft-not-open")));
        let (status, headers, replayed) = press(&router, &id, slug, 1, Some(&key)).await;
        assert_eq!(
            (status, &replayed, headers.contains_key("idempotent-replayed")),
            (StatusCode::ACCEPTED, &accepted, true)
        );
        let heads = gate.world.list_scopes(&path).await.expect("the site's scopes");
        assert!(heads.iter().all(|scope| scope.head.get() == 1), "nothing more written: {heads:?}");

        let cut = gate.world.resolve_cut(&path, &Pin::Branch(BranchName::main())).await.expect("the site's cut");
        let export = gate.world.export(&cut).await.expect("the made site's export");
        floors::load(&export.bytes).expect("foundry's reader loads the made site");
    }
    scratch.close().await;
}

/// Spec 05 section 9, a retry after a lost answer, and the asks again its write makes: a `customer` another writer
/// makes after the body was read refuses it whole, so the body is built again, kept and sent once more; both answers
/// lost, the client's resend's too, leave the outcome unknown until the API sends the kept body again, which WPS
/// answers by replaying the site it made; and the start's recovery, run while a press's worker writes, leaves the
/// write to that worker. Each site is one changeset, made once.
#[tokio::test]
async fn a_write_asked_again_finds_the_landed_site_and_writes_nothing_more() {
    let scratch = scratch_or_skip!();
    let (gate, router) = ready(&scratch.pool).await;
    let mock = &gate.world.wps().inner;
    let (raced, _) = draft(&router, "square", None).await;
    mock.make_scope_before_next_changeset(&"customer".parse().expect("a slug"), false);
    let (status, _, accepted) = press(&router, &raced, "raced-1", 1, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let read = written(&router, &raced).await;
    assert_eq!(read["state"], "created", "{read}");

    let (lost, _) = draft(&router, "square", None).await;
    mock.lose_next_answer();
    mock.lose_next_answer();
    let (status, _, accepted) = press(&router, &lost, "lost-1", 1, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let read = written(&router, &lost).await;
    let seqs = &read["write"]["result"]["seqs"];
    assert_eq!((read["state"].as_str(), seqs), (Some("created"), &at_one("customer/demo/site/lost-1")));

    let (left, _) = draft(&router, "u-shape", None).await;
    let root: ScopePath = "customer/demo/site/left-1".parse().expect("a site's root");
    gate.world.wps().hold(&root);
    let (status, _, _) = press(&router, &left, "left-1", 1, None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    recover(&gate).await.expect("the start's settling");
    gate.world.wps().open();
    let read = written(&router, &left).await;
    assert_eq!((read["state"].as_str(), &read["write"]["result"]["seqs"]), (Some("created"), &at_one(root.as_str())));
    for site in ["customer/demo/site/raced-1", "customer/demo/site/lost-1", root.as_str()] {
        let made = get(&router, &format!("/api/made-from/{site}")).await;
        assert_eq!(made["items"].as_array().map(Vec::len), Some(1), "{site} is made once");
        let history = gate.world.history(&site.parse().expect("a root"), None, 10).await.expect("the root's log");
        assert_eq!(history.len(), 1, "{site}'s root holds one changeset");
    }
    scratch.close().await;
}

/// Spec 05 section 4.4's refusals, each writing nothing: a taken site path answers `site-exists` with no approval
/// recorded; a root another writer makes before the changeset applies returns the draft to `open` with
/// `site-exists`, its approval `failed`, and the draft then makes a site under another reference; and evaluation 1
/// after a later one started is `evaluation-stale`.
#[tokio::test]
async fn a_refused_create_site_writes_nothing_and_reopens_the_draft() {
    let scratch = scratch_or_skip!();
    let (gate, router) = ready(&scratch.pool).await;
    let taken = support::made_site(&gate.world, "customer/demo/site/taken", "Taken").await;
    let (id, _) = draft(&router, "rectangle", None).await;
    let (status, _, refused) = press(&router, &id, "taken", 1, None).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("site-exists")), "{refused}");
    let read = get(&router, &format!("/api/template-drafts/{id}")).await;
    assert_eq!((read["state"].as_str(), &read["write"]), (Some("open"), &Value::Null), "nothing recorded");
    let approvals: i64 = sqlx::query_scalar("SELECT count(*) FROM approvals").fetch_one(&scratch.pool).await.unwrap();
    assert_eq!(approvals, 0, "no approval recorded");
    assert_eq!(gate.world.list_scopes(&taken).await.expect("its scopes").len(), 4);

    let raced: ScopePath = "customer/demo/site/raced".parse().expect("a site's root");
    gate.world.wps().inner.make_scope_before_next_changeset(&raced, true);
    let (status, _, accepted) = press(&router, &id, "raced", 1, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let read = written(&router, &id).await;
    assert_eq!(
        (read["state"].as_str(), read["write"]["problem"]["type"].as_str()),
        (Some("open"), Some("site-exists"))
    );
    let approval = get(&router, &format!("/api/approvals/{}", accepted["approval"]["id"].as_str().unwrap())).await;
    assert_eq!(approval["approval"]["execution"], "failed");
    assert!(gate.world.list_scopes(&raced).await.expect("its scopes").iter().all(|scope| scope.head.get() == 0));
    let (status, _, _) = press(&router, &id, "rectangle-2", 1, None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(written(&router, &id).await["state"], "created");

    let (stale, _) = draft(&router, "rectangle", None).await;
    let (status, _, _) = send(&router, "POST", &format!("/api/template-drafts/{stale}/evaluate"), None, None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    settled(&router, &stale).await;
    let (status, _, refused) = press(&router, &stale, "stale", 1, None).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("evaluation-stale")), "{refused}");
    scratch.close().await;
}

/// Spec 05 section 4.4, step 2: a generator published since the evaluation refuses the press with `generators-moved`
/// and starts the next evaluation, which records the set and is created stating it, since the site's `tiling` names
/// that generator; a registry moving between the check and the apply refuses the changeset, nothing written, the
/// draft `open` again with the next evaluation started.
#[tokio::test]
async fn a_moved_generator_set_refuses_the_press_and_starts_the_next_evaluation() {
    let scratch = scratch_or_skip!();
    let (gate, router) = ready(&scratch.pool).await;
    let (id, _) = draft(&router, "rectangle", None).await;
    gate.world.wps().inner.publish_generator("tiling-cells", "1.0.0", "sha256-tiling-1");
    gate.world.wps().inner.anchor_generator("tiling", "tiling-cells");
    gate.world.registry().revalidate(gate.world.wps()).await.expect("the registry read again");
    let (status, _, refused) = press(&router, &id, "moved", 1, None).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("generators-moved")), "{refused}");
    let read = settled(&router, &id).await;
    assert_eq!((read["state"].as_str(), &read["latest"]["n"]), (Some("open"), &json!(2)));
    let (status, _, accepted) = press(&router, &id, "moved", 2, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    assert_eq!(written(&router, &id).await["state"], "created");
    let stated: Value = sqlx::query_scalar("SELECT sent -> 'expect_generators' FROM approvals WHERE id = $1")
        .bind(accepted["approval"]["id"].as_str())
        .fetch_one(&scratch.pool)
        .await
        .expect("the kept plan");
    assert_eq!(stated["tiling-cells"][0], "1.0.0", "{stated}");

    let (next, _) = draft(&router, "rectangle", None).await;
    gate.world.wps().inner.move_registry_next();
    let (status, _, _) = press(&router, &next, "moved-again", 1, None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let read = written(&router, &next).await;
    assert_eq!(
        (read["state"].as_str(), read["write"]["problem"]["type"].as_str()),
        (Some("open"), Some("generators-moved"))
    );
    assert_eq!(settled(&router, &next).await["latest"]["n"], 2);
    assert!(!listed(&router).await.contains(&"customer/demo/site/moved-again".to_owned()));
    scratch.close().await;
}

/// The changeset Create site keeps with `draft` while it writes.
async fn kept(pool: &PgPool, draft: &str) -> WireChangeset {
    let kept: Value = sqlx::query_scalar("SELECT write_body FROM template_drafts WHERE id = $1")
        .bind(draft)
        .fetch_one(pool)
        .await
        .expect("the kept changeset");
    serde_json::from_value(kept).expect("a changeset")
}

/// The `approval.execution` states journaled for `approval` in `conversation`, oldest first.
async fn executions(router: &Router, conversation: &str, approval: &str) -> Vec<String> {
    let read = get(router, &format!("/api/conversations/{conversation}")).await;
    let entries = read["entries"].as_array().expect("the entries");
    let of_it = entries.iter().filter(|entry| entry["kind"] == "approval.execution" && entry["caused_by"] == approval);
    of_it.map(|entry| entry["body"]["execution"].as_str().unwrap_or_default().to_owned()).collect()
}

/// LEAD-DECISIONS entry 54 against the interleaving of design/reviews-impl/astra-07c.md's first finding: a press's
/// worker holds its approval, so the start's recovery, run beside it, does nothing; a `customer` another writer makes
/// refuses the kept body, which the one worker builds again and lands. Two changesets went under the approval's
/// operation (the refused body and the one built again), the site is made once with its thread bound, the approval
/// journals `running` then `ok`, and a late worker finds it settled: its claim is refused, and `running` after `ok`
/// moves nothing. Before, the recovery's worker sent the kept body beside the press's, twice as many changesets.
#[tokio::test]
async fn one_approval_has_one_worker_through_a_prefix_refusal() {
    let scratch = scratch_or_skip!();
    let (gate, router) = ready(&scratch.pool).await;
    let conversation = thread(&router).await;
    let (id, _) = draft(&router, "rectangle", Some(&conversation)).await;
    let root: ScopePath = "customer/demo/site/once-1".parse().expect("a site's root");
    gate.world.wps().hold(&root);
    let (status, _, accepted) = press(&router, &id, "once-1", 1, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let approval = accepted["approval"]["id"].as_str().expect("the approval").to_owned();
    recover(&gate).await.expect("the start's settling");
    sleep(Duration::from_millis(300)).await;
    gate.world.wps().inner.make_scope_before_next_changeset(&"customer".parse().expect("a slug"), false);
    gate.world.wps().open();

    let read = written(&router, &id).await;
    assert_eq!(read["state"], "created", "{read}");
    assert_eq!(gate.world.wps().posted(&approval), 2, "the refused body and the one built again, by one worker");
    let made = get(&router, &format!("/api/made-from/{root}")).await;
    assert_eq!(made["items"].as_array().map(Vec::len), Some(1));
    let bound = get(&router, &format!("/api/conversations/{conversation}")).await;
    assert_eq!(bound["conversation"]["site"], json!(root.as_str()));
    assert_eq!(executions(&router, &conversation, &approval).await, ["running", "ok"]);

    let approval: Ulid = approval.parse().expect("a ULID");
    assert!(!claim(&gate.app, &approval).await.expect("a claim"), "a settled execution has no worker");
    let mut tx = scratch.pool.begin().await.expect("a transaction");
    let late = executed(&mut tx, &approval, Execution::Running, None, None).await.expect("a move");
    tx.commit().await.expect("committed");
    assert!(late.is_none(), "running after ok moves nothing");
    let read = get(&router, &format!("/api/approvals/{approval}")).await;
    assert_eq!(read["approval"]["execution"], "ok");
    assert_eq!(executions(&router, &conversation, approval.as_str()).await, ["running", "ok"]);
    scratch.close().await;
}

/// Spec 04 section 3.9 and LEAD-DECISIONS entry 54, against design/reviews-impl/astra-07c.md's second finding:
/// world-proto-server committed the site, and the process that sent it stopped before recording it (here its worker
/// waits on a held answer for good). A restarted API's client holds only what a start loads, and every schema read
/// fails, yet its start's recovery sends the kept changeset as it was, reads no schema, is answered WPS's replay, and
/// makes the site: the draft `created`, one made-from record, the approval `ok`. Before, sending read `frame`'s
/// schema first, and the failed read passed for a refusal: the draft went back to `open` beside its standing site.
#[tokio::test]
async fn a_restarted_api_settles_a_landed_site_without_reading_a_schema() {
    let scratch = scratch_or_skip!();
    let (gate, router) = ready(&scratch.pool).await;
    let (id, _) = draft(&router, "l-shape", None).await;
    let root: ScopePath = "customer/demo/site/restart-1".parse().expect("a site's root");
    gate.world.wps().hold(&root);
    let (status, _, accepted) = press(&router, &id, "restart-1", 1, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    gate.world.wps().inner.post_changeset(&kept(&scratch.pool, &id).await).await.expect("it lands, its sender waiting");

    let restarted = Client::start(gate.world.wps().sharing(), ClientConfig::default(), Arc::new(BlockingCompute))
        .await
        .expect("the restarted API's client");
    restarted.wps().fail_schema_reads();
    let again = GateState::new(support::state(scratch.pool.clone(), support::gate_rules()), restarted);
    recover(&again).await.expect("the start's settling");
    let read = written(&gate_routes(again.clone()), &id).await;
    assert_eq!((read["state"].as_str(), &read["write"]["result"]["site"]), (Some("created"), &json!(root)), "{read}");
    assert_eq!(again.world.wps().schema_reads(), 0, "the kept changeset went as it was, no schema read");
    let made = get(&router, &format!("/api/made-from/{root}")).await;
    assert_eq!(made["items"].as_array().map(Vec::len), Some(1));
    let approval = get(&router, &format!("/api/approvals/{}", accepted["approval"]["id"].as_str().unwrap())).await;
    assert_eq!(approval["approval"]["execution"], "ok");
    scratch.close().await;
}

/// LEAD-DECISIONS entry 54 and spec 01 section 3.17: a root whose changeset landed while Create site still writes it
/// is no site yet. `GET /api/sites` leaves it out and opening its conversation answers `not-found`, so nobody can open
/// one before the landing binds the New site thread; once it lands, the site is listed and opening its conversation
/// answers that thread. Before, the list held the root as soon as world-proto-server did.
#[tokio::test]
async fn a_root_still_being_made_is_no_site_until_its_landing_binds_the_thread() {
    let scratch = scratch_or_skip!();
    let (gate, router) = ready(&scratch.pool).await;
    let conversation = thread(&router).await;
    let (id, _) = draft(&router, "square", Some(&conversation)).await;
    let root: ScopePath = "customer/demo/site/making-1".parse().expect("a site's root");
    gate.world.wps().hold(&root);
    let (status, _, accepted) = press(&router, &id, "making-1", 1, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    gate.world.wps().inner.post_changeset(&kept(&scratch.pool, &id).await).await.expect("the root lands first");
    assert!(!listed(&router).await.contains(&root.to_string()), "no site while its draft writes");
    let (status, _, opened) = send(&router, "POST", "/api/conversations", Some(json!({ "site": root })), None).await;
    assert_eq!((status, opened["type"].as_str()), (StatusCode::NOT_FOUND, Some("not-found")), "{opened}");

    gate.world.wps().open();
    assert_eq!(written(&router, &id).await["state"], "created");
    assert!(listed(&router).await.contains(&root.to_string()));
    let (status, _, opened) = send(&router, "POST", "/api/conversations", Some(json!({ "site": root })), None).await;
    assert_eq!((status, &opened["conversation"]["id"]), (StatusCode::OK, &json!(conversation)), "{opened}");
    scratch.close().await;
}

/// LEAD-DECISIONS entry 54: the thread's binding is part of the landing's one transaction, so a site that holds a
/// conversation already (nothing can open one on a root still being made, so here a row written by hand) fails the
/// landing whole: the draft still writes, no made-from record, the thread unbound, the approval not `ok`. Before, the
/// binding was rolled back alone, and the site recorded as made without it.
#[tokio::test]
async fn a_site_holding_a_conversation_fails_its_landing_whole() {
    let scratch = scratch_or_skip!();
    let (gate, router) = ready(&scratch.pool).await;
    let conversation = thread(&router).await;
    let (id, _) = draft(&router, "square", Some(&conversation)).await;
    let root: ScopePath = "customer/demo/site/held-1".parse().expect("a site's root");
    gate.world.wps().hold(&root);
    let (status, _, accepted) = press(&router, &id, "held-1", 1, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    support::conversation(&scratch.pool, Some(root.as_str()), "live", "other", 1).await;
    gate.world.wps().open();
    sleep(Duration::from_millis(1500)).await;

    let approval = accepted["approval"]["id"].as_str().expect("the approval");
    assert_eq!(gate.world.wps().posted(approval), 1, "it landed");
    let read = get(&router, &format!("/api/template-drafts/{id}")).await;
    assert_eq!(read["state"], "writing", "{read}");
    let made = get(&router, &format!("/api/made-from/{root}")).await;
    assert_eq!(made["items"], json!([]));
    let bound = get(&router, &format!("/api/conversations/{conversation}")).await;
    assert_eq!(bound["conversation"]["site"], Value::Null);
    let read = get(&router, &format!("/api/approvals/{approval}")).await;
    assert_eq!(read["approval"]["execution"], "running");
    scratch.close().await;
}

/// Spec 05 section 4.3, step 6.12 (LEAD-DECISIONS entry 54): an evaluation runs the client's own check of the site its
/// plan would make before it says ready, so a `frame` schema world-proto-server published since the template was read
/// (the engine validates against the read it keeps) shows as the evaluation's error under each frame's source key, and
/// Create site refuses that evaluation before anything is approved or sent. Before, the evaluation was ready without a
/// problem and the press taken, world-proto-server's refusal coming only once the write was under way.
#[tokio::test]
async fn an_evaluation_checks_the_site_it_would_make() {
    let scratch = scratch_or_skip!();
    let (gate, router) = ready(&scratch.pool).await;
    let (id, _) = draft(&router, "rectangle", None).await;
    let strict = json!({ "type": "object", "required": ["never"] });
    let parent = RefAnnotation { path: "/parent".to_owned(), expects: Some("frame".to_owned()) };
    gate.world.wps().inner.publish("frame", "0.8.0", strict, vec![parent]);
    gate.world.registry().refresh(gate.world.wps(), "frame").await.expect("frame read again");
    let (status, _, _) = send(&router, "POST", &format!("/api/template-drafts/{id}/evaluate"), None, None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(settled(&router, &id).await["latest"], json!({ "n": 2, "state": "ready" }));

    let evaluation = get(&router, &format!("/api/template-drafts/{id}/evaluations/2")).await;
    let problems = evaluation["problems"].as_array().expect("its problems");
    let refused: Vec<&Value> = problems.iter().filter(|problem| problem["code"] == "schema-invalid").collect();
    assert!(!refused.is_empty(), "{problems:?}");
    let keys: Vec<String> = sqlx::query_scalar("SELECT key FROM template_draft_ids WHERE draft = $1")
        .bind(&id)
        .fetch_all(&scratch.pool)
        .await
        .expect("the draft's source keys");
    for problem in refused {
        let text = problem["text"].as_str().unwrap_or_default();
        assert!(text.starts_with("Error: The site these knobs make would be refused: "), "{problem}");
        assert_eq!(problem["severity"], "error");
        let key = problem["ref"].as_str().unwrap_or_default();
        assert!(keys.iter().any(|known| known == key), "under the frame's source key: {problem}");
    }
    let (status, _, answer) = press(&router, &id, "checked-1", 2, None).await;
    assert_eq!((status, answer["type"].as_str()), (StatusCode::CONFLICT, Some("evaluation-stale")), "{answer}");
    scratch.close().await;
}
