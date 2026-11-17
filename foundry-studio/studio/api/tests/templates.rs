//! Spec 05's template workflow through the API's routes (sections 3.8 to 3.12 and 4.1 to 4.3), against the client's
//! mock seeded by the development seed itself: the Template list and its thumbnails, drafts and their evaluations,
//! section 5.5's acceptances 1 and 4 through the API, and section 9's PATCH statements.

mod support;

use axum::http::StatusCode;
use serde_json::{Map, Value, json};
use studio_protocol::ScopePath;
use support::templates::{draft, get, press, ready, send, settled, thread, until, written};

const SHELLS: [&str; 5] = ["square", "rectangle", "l-shape", "u-shape", "i-shape"];

/// Spec 05 sections 4.1 and 6: the list holds the five shells in the POC's order, each thumbnail evaluating at first
/// and then ready, its floor kept for the page's viewer to draw (ruling D121), its census counted and no problem known;
/// a template state's thumbnail is computed once.
#[tokio::test]
async fn the_list_holds_the_five_shells_in_the_pocs_order_and_fills_their_thumbnails() {
    use runstore::RunStorage;
    let scratch = scratch_or_skip!();
    let (gate, router) = ready(&scratch.pool).await;
    let list = get(&router, "/api/templates").await;
    let refs: Vec<&str> = list["items"].as_array().unwrap().iter().map(|card| card["ref"].as_str().unwrap()).collect();
    let wanted: Vec<String> = SHELLS.iter().map(|shell| format!("template/site/{shell}")).collect();
    assert_eq!(refs, wanted);
    let list = until("every thumbnail", || async {
        let list = get(&router, "/api/templates").await;
        let states = list["items"].as_array().unwrap().iter().map(|card| card["thumbnail"]["state"].clone());
        states.collect::<Vec<_>>().iter().all(|state| state == "ready").then_some(list)
    })
    .await;
    for card in list["items"].as_array().unwrap() {
        let thumbnail = &card["thumbnail"];
        let floor = thumbnail["floor_key"].as_str().expect("the default evaluation's floor, which the page draws");
        assert!(floor.starts_with("template-thumbnails/") && floor.ends_with("/floor.msgpack"), "{floor}");
        assert!(gate.app.runstore.exists(floor).await.unwrap(), "{}: {floor}", card["ref"]);
        assert!(thumbnail.get("svg").is_none(), "no drawing of the API's (D121)");
        assert_eq!(thumbnail["census"]["by_knob"]["aisles"], 4, "{}", card["ref"]);
        assert!(thumbnail["census"]["derived"]["relay"].as_u64().unwrap() > 0, "{}", card["ref"]);
        assert_eq!(thumbnail["problems"], json!([]), "{}", card["ref"]);
        assert_eq!(card["flows"].as_array().unwrap().len(), 8, "the relay flow's material edges");
    }
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM template_thumbnails").fetch_one(&scratch.pool).await.unwrap();
    assert_eq!(rows, 5);
    let one = get(&router, "/api/templates/template/site/l-shape@main").await;
    assert_eq!(
        (one["description"]["title"].as_str(), one["default"]["state"].as_str()),
        (Some("L-Shape"), Some("ready"))
    );
    scratch.close().await;
}

/// Spec 05 section 5.5, acceptance 1 through the API, the flow half excepted: each shell's draft evaluates at its
/// defaults with no error, named "{title}, evaluation 1" with its primary frame made; its document loads in
/// foundry's reader, and its floor compiles and is kept with its plan in the run-object store.
#[tokio::test]
async fn each_shell_evaluates_at_its_defaults_and_its_floor_is_kept() {
    let scratch = scratch_or_skip!();
    let (gate, router) = ready(&scratch.pool).await;
    for shell in SHELLS {
        let (id, read) = draft(&router, shell, None).await;
        assert_eq!(read["latest"], json!({ "n": 1, "state": "ready" }), "{shell}");
        let evaluation = get(&router, &format!("/api/template-drafts/{id}/evaluations/1")).await;
        let errors: Vec<&Value> = evaluation["problems"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|problem| problem["severity"] == "error")
            .collect();
        assert!(errors.is_empty(), "{shell}: {errors:?}");
        let title = read["title"].as_str().unwrap();
        assert_eq!(evaluation["prologue"]["name"], format!("{title}, evaluation 1"));
        assert!(evaluation["prologue"]["primary_frame"].is_string(), "{shell}");
        let floor = evaluation["floor"]["key"].as_str().expect("a floor kept");
        use runstore::RunStorage;
        assert!(gate.app.runstore.exists(floor).await.unwrap(), "{shell}: {floor}");
        let flow: Vec<&Value> = (evaluation["problems"].as_array().unwrap().iter())
            .filter(|problem| problem["code"] == "flow-invalid")
            .collect();
        assert!(flow.is_empty(), "{shell}'s default flow compiles in validation mode: {flow:?}");
    }
    scratch.close().await;
}

/// Spec 05 section 5.5, acceptance 4 through the API for the L-Shape (lead decided; the engine's own tests hold it for
/// all five): each knob at its minimum and at its maximum, the others at their defaults, and each orientation,
/// evaluates; it fails only checks and outlines held at their range (D120), and a site it cannot load only alongside
/// one of those; without an error it is ready with its floor.
#[tokio::test]
async fn the_l_shape_evaluates_at_each_knobs_bounds_and_each_orientation() {
    let scratch = scratch_or_skip!();
    let (_, router) = ready(&scratch.pool).await;
    let (id, read) = draft(&router, "l-shape", None).await;
    let mut values = Vec::new();
    for knob in read["declarations"].as_array().unwrap() {
        let key = knob["key"].as_str().unwrap().to_owned();
        match knob["options"].as_array().filter(|options| !options.is_empty()) {
            Some(options) => {
                values.extend(options.iter().map(|option| (key.clone(), option["value"].as_i64().unwrap())))
            }
            None => {
                let (min, max, step) =
                    (knob["min"].as_i64().unwrap(), knob["max"].as_i64().unwrap(), knob["step"].as_i64().unwrap());
                values.extend([(key.clone(), min), (key.clone(), max - (max - min) % step)]);
            }
        }
    }
    let (mut revision, mut moved): (u64, Option<String>) = (1, None);
    for (key, value) in values {
        let mut knobs = Map::new();
        if let Some(previous) = moved.take().filter(|previous| *previous != key) {
            knobs.insert(previous.clone(), Value::Null);
        }
        knobs.insert(key.clone(), json!(value));
        moved = Some(key.clone());
        let body = json!({ "revision": revision, "knobs": knobs });
        let (status, _, set) = send(&router, "PATCH", &format!("/api/template-drafts/{id}"), Some(body), None).await;
        assert_eq!(status, StatusCode::OK, "{key} at {value}: {set}");
        revision = set["revision"].as_u64().unwrap();
        let n = set["evaluation"]["n"].as_u64().unwrap();
        settled(&router, &id).await;
        let evaluation = get(&router, &format!("/api/template-drafts/{id}/evaluations/{n}")).await;
        let codes: Vec<&str> = evaluation["problems"]
            .as_array()
            .unwrap()
            .iter()
            .map(|problem| problem["code"].as_str().unwrap())
            .collect();
        let checked = evaluation["problems"].as_array().unwrap().iter().any(|p| {
            ["check-failed", "derived-out-of-range"].contains(&p["code"].as_str().unwrap()) && p["severity"] == "error"
        });
        for code in &codes {
            let allowed =
                ["check-failed", "derived-out-of-range"].contains(code) || (*code == "floor-compile-failed" && checked);
            assert!(allowed, "{key} at {value}: {}", evaluation["problems"]);
        }
        if !checked {
            assert_eq!(evaluation["state"], "ready", "{key} at {value}: {}", evaluation["problems"]);
            assert!(evaluation["floor"]["key"].is_string(), "{key} at {value}");
        }
    }
    scratch.close().await;
}

/// Spec 05 section 9: a PATCH naming a knob set after its revision answers `knob-changed`, one naming other knobs
/// merges, and evaluation numbers stay gapless and ordered under two writers.
#[tokio::test]
async fn knob_changes_merge_refuse_a_knob_set_since_and_number_evaluations_without_gaps() {
    let scratch = scratch_or_skip!();
    let (_, router) = ready(&scratch.pool).await;
    let (id, _) = draft(&router, "rectangle", None).await;
    let uri = format!("/api/template-drafts/{id}");
    let patch = |revision: u64, knobs: Value| {
        let router = router.clone();
        let uri = uri.clone();
        async move { send(&router, "PATCH", &uri, Some(json!({ "revision": revision, "knobs": knobs })), None).await }
    };
    let (status, _, set) = patch(1, json!({ "aisles": 5 })).await;
    assert_eq!((status, set["revision"].clone(), set["evaluation"]["n"].clone()), (StatusCode::OK, json!(2), json!(2)));
    let (status, _, refused) = patch(1, json!({ "aisles": 3 })).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("knob-changed")));
    assert_eq!(refused["problems"][0]["knob"], "aisles");
    assert!(refused["detail"].as_str().unwrap().starts_with("Aisles was set to 5 by"), "{refused}");
    let (status, _, merged) = patch(1, json!({ "bays": 3 })).await;
    assert_eq!((status, merged["revision"].clone()), (StatusCode::OK, json!(3)));
    let (status, _, invalid) = patch(3, json!({ "aisles": 31 })).await;
    assert_eq!((status, invalid["problems"][0]["knob"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("aisles")));

    let writers: Vec<_> = ["htms", "station_pitch", "width", "depth"]
        .into_iter()
        .zip([25, 5100, 41650, 28050])
        .map(|(knob, value)| tokio::spawn(patch(3, json!({ knob: value }))))
        .collect();
    let mut numbers = Vec::new();
    for writer in writers {
        let (status, _, set) = writer.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{set}");
        numbers.push(set["evaluation"]["n"].as_u64().unwrap());
    }
    numbers.sort();
    assert_eq!(numbers, [4, 5, 6, 7], "each writer numbered its own evaluation, none skipped");
    let read = settled(&router, &id).await;
    assert_eq!((read["revision"].clone(), read["knobs"]["aisles"].clone()), (json!(7), json!(5)));
    let held: Vec<(i32, String)> =
        sqlx::query_as("SELECT n, state FROM template_evaluations WHERE draft = $1 ORDER BY n")
            .bind(&id)
            .fetch_all(&scratch.pool)
            .await
            .unwrap();
    assert_eq!(held.iter().map(|(n, _)| *n).collect::<Vec<_>>(), (1..=7).collect::<Vec<_>>());
    assert!(held[1..6].iter().all(|(_, state)| state != "in-progress"), "{held:?}");
    scratch.close().await;
}

/// Spec 05 section 9: a PATCH while the draft is `writing` answers `draft-not-open`, as do evaluating it again and
/// abandoning it (section 3.8), and nothing is numbered; once abandoned, a PATCH and Create site answer the same, and
/// its thread stays unbound.
#[tokio::test]
async fn a_draft_that_is_writing_or_abandoned_refuses_changes_as_not_open() {
    let scratch = scratch_or_skip!();
    let (gate, router) = ready(&scratch.pool).await;
    let (id, _) = draft(&router, "i-shape", None).await;
    let uri = format!("/api/template-drafts/{id}");
    let root: ScopePath = "customer/demo/site/writing".parse().expect("a site's root");
    gate.world.wps().hold(&root);
    let (status, _, accepted) = press(&router, &id, "writing", 1, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");

    let knobs = json!({ "revision": 1, "knobs": { "htms": 25 } });
    let (status, _, refused) = send(&router, "PATCH", &uri, Some(knobs.clone()), None).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("draft-not-open")), "{refused}");
    assert_eq!(refused["detail"], "This draft is writing: wait until its site is written.");
    for (method, path) in [("POST", format!("{uri}/evaluate")), ("DELETE", uri.clone())] {
        let (status, _, refused) = send(&router, method, &path, None, None).await;
        assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("draft-not-open")), "{method}");
    }
    let read = get(&router, &uri).await;
    assert_eq!((read["state"].as_str(), read["revision"].clone()), (Some("writing"), json!(1)));
    assert_eq!((read["latest"]["n"].clone(), read["write"]["evaluation"].clone()), (json!(1), json!(1)));
    gate.world.wps().open();
    assert_eq!(written(&router, &id).await["state"], "created");

    let conversation = thread(&router).await;
    let (abandoned, _) = draft(&router, "i-shape", Some(&conversation)).await;
    let uri = format!("/api/template-drafts/{abandoned}");
    let (status, _, _) = send(&router, "DELETE", &uri, None, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, refused) = send(&router, "PATCH", &uri, Some(knobs), None).await;
    assert_eq!(
        (status, refused["detail"].as_str()),
        (StatusCode::CONFLICT, Some("This draft is abandoned: start a new draft from the template."))
    );
    let (status, _, refused) = press(&router, &abandoned, "abandoned", 1, None).await;
    assert_eq!((status, refused["type"].as_str()), (StatusCode::CONFLICT, Some("draft-not-open")), "{refused}");
    let read = get(&router, &format!("/api/conversations/{conversation}")).await;
    assert_eq!(read["conversation"]["site"], Value::Null, "an abandoned draft binds nothing");
    scratch.close().await;
}

/// D120 through the API, the brief's Done when: raising Aisles on the Square leaves Side unset and grows it with no fit
/// error, each knob of the evaluation marked set, default or derived; Side set smaller than the fit holds, the fit
/// error under it; set back to `null`, it is unset again and follows the aisles.
#[tokio::test]
async fn the_squares_side_follows_its_aisles_until_someone_sets_it() {
    let scratch = scratch_or_skip!();
    let (_, router) = ready(&scratch.pool).await;
    let (id, _) = draft(&router, "square", None).await;
    let uri = format!("/api/template-drafts/{id}");
    let mut revision = 1;
    let mut set = |knobs: Value| {
        let (router, uri) = (router.clone(), uri.clone());
        revision += 1;
        let body = json!({ "revision": revision - 1, "knobs": knobs });
        async move {
            let (status, _, answer) = send(&router, "PATCH", &uri, Some(body), None).await;
            assert_eq!(status, StatusCode::OK, "{answer}");
            let read = settled(&router, uri.trim_start_matches("/api/template-drafts/")).await;
            let n = answer["evaluation"]["n"].as_u64().unwrap();
            (read, get(&router, &format!("{uri}/evaluations/{n}")).await)
        }
    };
    let (read, grown) = set(json!({ "aisles": 6 })).await;
    assert_eq!(read["knobs"], json!({ "aisles": 6 }), "only what was set is stored as set");
    assert_eq!((grown["knobs"]["side"].clone(), grown["problems"].clone()), (json!(56100), json!([])));
    let sources = &grown["sources"];
    assert_eq!(
        (&sources["aisles"], &sources["side"], &sources["bays"]),
        (&json!("set"), &json!("derived"), &json!("default"))
    );

    // The site those knobs make cannot load either (its aisles overrun the floor), so its compile fails beside it.
    let (_, short) = set(json!({ "side": 40800 })).await;
    assert_eq!(short["sources"]["side"], "set");
    let problems: Vec<(Option<&str>, &str)> = short["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|problem| (problem["knob"].as_str(), problem["code"].as_str().unwrap()))
        .collect();
    assert_eq!(problems[0], (Some("side"), "check-failed"), "{problems:?}");
    let text = "Error: The aisle row needs 15,300 mm more floor than the shell's width gives.";
    assert_eq!(short["problems"][0]["text"], text);
    assert!(problems[1..].iter().all(|problem| *problem == (None, "floor-compile-failed")), "{problems:?}");

    let (read, follows) = set(json!({ "side": null })).await;
    assert_eq!(
        (read["knobs"].clone(), read["set_by"]["side"]["by"].clone()),
        (json!({ "aisles": 6 }), json!("template"))
    );
    assert_eq!(
        (follows["knobs"]["side"].clone(), follows["sources"]["side"].clone()),
        (json!(56100), json!("derived"))
    );
    assert_eq!(follows["problems"], json!([]));
    scratch.close().await;
}
