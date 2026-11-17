//! Spec 06 section 3.4's flow routes through the API's router, against the client's mock seeded by the development
//! seed, with the L-Shape made at 6 aisles and 54,400 mm staged as Create site writes a site: the read, the
//! parameter list and flow-validate, and the read of a site with no flow.

mod support;

use std::env;
use std::fs;

use axum::http::StatusCode;
use serde_json::{Map, Value, json};
use site_template::{DocId, Prior, SourceKey};
use studio_protocol::{DeclaredId, ScopePath};
use support::Scratch;
use support::l_shape::{FLOWLESS, SITE, call, made_stages, ready};
use support::templates::repo;
use wps_client::Op;

/// Spec 06 section 6.1 through `GET /api/flow/{*location}`: 5 nodes, 8 edges, 13 segments and one rule, compiling
/// to seven roots with no problem, read at the site's heads, each material edge with what serves it and the site with
/// its newest made-from record (section 5's cards and made-from line); then the parameter list and flow-validate,
/// which holds a variant to section 3.1's lower bounds.
#[tokio::test]
async fn the_made_l_shapes_flow_reads_lists_and_validates() {
    let Some(scratch) = Scratch::open().await else { return };
    let (_, router) = ready(&scratch.pool).await;
    let (status, read) = call(&router, "GET", &format!("/api/flow/{SITE}/flow"), None).await;
    assert_eq!(status, StatusCode::OK, "{read}");
    let count = |list: &str| read[list].as_array().map_or(0, Vec::len);
    let counts = (count("nodes"), count("edges"), count("segments"), count("return_rules"));
    assert_eq!(counts, (5, 8, 13, 1));
    assert_eq!(read["problems"], json!([]));
    assert_eq!(read["plan"]["roots"].as_array().map(Vec::len), Some(7));
    assert_eq!(
        (read["scope"].as_str(), read["pin"].as_str()),
        (Some("customer/demo/site/lshape-6/flow"), Some("main"))
    );
    let seqs = read["seqs"].as_object().unwrap();
    let site: Vec<&String> = seqs.keys().filter(|scope| scope.starts_with(SITE)).collect();
    assert_eq!(site.len(), 4, "the root, map, flow and fleet, beside the catalog scopes the fleet pins");
    assert_eq!(read["exact"], json!(true));
    let again = call(&router, "GET", &format!("/api/flow/{SITE}/flow@main"), None).await.1;
    assert_eq!(again, read, "the keep answers a second read the same");
    let served_by = |label: &str| {
        let edges = read["edges"].as_array().unwrap();
        edges.iter().find(|edge| edge["label"] == label).unwrap_or_else(|| panic!("{label}"))["served_by"].clone()
    };
    let delivery = served_by("Inventory storage to Pick stations: stock tote");
    assert_eq!(delivery, json!({ "kind": "root", "task": "PickDemand" }));
    let back = served_by("Put stations to Inventory storage: stock tote");
    assert_eq!(back, json!({ "kind": "clearance" }), "a return is each finished presentation's clearance");
    let leaving = served_by("Pick stations to Outbound orders: order in its bin");
    assert_eq!(leaving, json!({ "kind": "completion" }), "an order leaving in its bin needs no transport");
    assert_eq!(read["made_from"], Value::Null, "the test wrote the site without Create site");
    let template_seqs = json!({ "template/site/l-shape": 3, "template/site/l-shape/flow": 2 });
    sqlx::query(
        "INSERT INTO site_made_from (id, site, state, act, template, template_seqs) \
         VALUES ('01KN0TJZANP5N0PGJD8F7A3MF1', $1, 'made', 'create', 'template/site/l-shape', $2)",
    )
    .bind(SITE)
    .bind(&template_seqs)
    .execute(&scratch.pool)
    .await
    .unwrap();
    let (_, made) = call(&router, "GET", &format!("/api/flow/{SITE}/flow"), None).await;
    assert_eq!(made["made_from"], json!({ "template": "template/site/l-shape", "tag": null, "seq": 3 }));

    let (status, listed) = call(&router, "GET", &format!("/api/flow-parameters/{SITE}/flow"), None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let pace = (listed["parameters"].as_array().unwrap().iter())
        .find(|p| p["label"] == "Put stations, fill pace per station")
        .expect("the fill pace is listed");
    assert_eq!((&pace["sweepable"], &pace["unit"], &pace["value"]), (&json!(true), &json!("per hour"), &json!(240)));
    assert_eq!(pace["all_in"]["text"], "240 an hour fill pace = 211.8 an hour all-in");
    let pick = (listed["parameters"].as_array().unwrap().iter())
        .find(|p| p["label"] == "Pick stations, time per pick")
        .expect("the time per pick is listed");
    assert_eq!((&pick["clock"], &pace["clock"]), (&json!("sim"), &Value::Null), "the protocol's clock discriminator");

    let supply = (read["edges"].as_array().unwrap().iter())
        .find(|edge| edge["label"] == "Inventory storage to Put stations: empty tote")
        .expect("the empty-tote supply")["ref"]
        .clone();
    let validate = format!("/api/flow-validate/{SITE}/flow");
    let moves = json!({ "patches": [{ "target": supply, "pointer": "/flow.material_edge/moves", "value": "units" }] });
    let (status, refused) = call(&router, "POST", &validate, Some(moves)).await;
    assert_eq!(status, StatusCode::OK, "{refused}");
    assert_eq!(refused["plan"], Value::Null);
    let codes: Vec<&str> = refused["problems"].as_array().unwrap().iter().filter_map(|p| p["code"].as_str()).collect();
    assert_eq!(codes, ["not-sweepable"]);
    let inbound = (read["edges"].as_array().unwrap().iter())
        .find(|edge| edge["label"] == "Inbound units to Put stations: units")
        .expect("the inbound units")["ref"]
        .clone();
    let pointer = "/flow.material_edge/induction/units_per_tote";
    let zero = json!({ "patches": [{ "target": inbound, "pointer": pointer, "value": 0 }] });
    let (status, below) = call(&router, "POST", &validate, Some(zero)).await;
    assert_eq!((status, &below["plan"]), (StatusCode::OK, &Value::Null), "a tote of no units compiles to nothing");
    assert_eq!(below["problems"][0]["code"], "schema-unknown", "{below}");
    let cutoff = json!({
        "patches": [{ "target": supply, "pointer": "/flow.material_edge/until_sim_ms", "value": 7150000 }],
        "setup": { "stock_at_start": 0.4, "order_trays": 0 }
    });
    let (status, swept) = call(&router, "POST", &validate, Some(cutoff)).await;
    assert_eq!(status, StatusCode::OK, "{swept}");
    assert_eq!(swept["plan"]["setup"], json!({ "stock_at_start": 0.4, "order_trays": 0 }));
    let settings = swept["plan"]["settings"].as_array().unwrap();
    let rack = settings.iter().find(|s| s["setting"] == "putaway.forbid_rack_after_secs").unwrap();
    assert_eq!(rack["value"], json!(7150.0));
    assert_ne!(swept["plan"]["digest"], read["plan"]["digest"], "the variant and the setup reach the digest");
    scratch.close().await;
}

/// Spec 06 sections 3.3 and 3.4, the read's provenance: the answer names every scope the export holds entities from,
/// the catalog's at the version the fleet pins; with a pinned catalog scope moved past its pin first, the export
/// carries it below its head, and the answer says inexact, which the cut alone overstated (Astra's task-11 read).
#[tokio::test]
async fn a_read_answers_the_exports_provenance_inexact_below_a_catalog_head() {
    let Some(scratch) = Scratch::open().await else { return };
    let (gate, router) = ready(&scratch.pool).await;
    let pinned: ScopePath = "catalog/agent/d30".parse().unwrap();
    let declared = Op::Declare { id: DeclaredId::mint(), name: Some("A later class".into()), components: Map::new() };
    support::stage(&gate.world, &pinned, vec![declared], None).await;
    let (status, read) = call(&router, "GET", &format!("/api/flow/{SITE}/flow"), None).await;
    assert_eq!((status, &read["exact"]), (StatusCode::OK, &json!(false)), "{read}");
    assert_eq!(read["seqs"][pinned.as_str()], json!(1), "the export carries the catalog at the fleet's pin");
    assert_eq!(read["seqs"][format!("{SITE}/flow")], json!(1));
    scratch.close().await;
}

/// Spec 06 section 3.4: a site whose flow sub-scope is empty, and one that has none at all, answer 200 with empty
/// lists, no plan and the warning `flow-empty`, as do their parameter list and flow-validate; a location that is no
/// flow scope is refused, and a draft that does not exist is not found.
#[tokio::test]
async fn a_site_without_a_flow_reads_empty_and_other_locations_are_refused() {
    let Some(scratch) = Scratch::open().await else { return };
    let (_, router) = ready(&scratch.pool).await;
    for site in ["customer/demo/site/bare", FLOWLESS] {
        let (status, bare) = call(&router, "GET", &format!("/api/flow/{site}/flow"), None).await;
        assert_eq!(status, StatusCode::OK, "{site}: {bare}");
        assert_eq!((&bare["nodes"], &bare["plan"]), (&json!([]), &Value::Null));
        let problems = bare["problems"].as_array().unwrap();
        assert_eq!(problems.len(), 1);
        assert_eq!((&problems[0]["code"], &problems[0]["severity"]), (&json!("flow-empty"), &json!("warning")));
        let (status, listed) = call(&router, "GET", &format!("/api/flow-parameters/{site}/flow"), None).await;
        assert_eq!((status, &listed["parameters"]), (StatusCode::OK, &json!([])), "{site}: {listed}");
        let (status, checked) =
            call(&router, "POST", &format!("/api/flow-validate/{site}/flow"), Some(json!({}))).await;
        assert_eq!((status, &checked["plan"]), (StatusCode::OK, &Value::Null), "{site}: {checked}");
        assert_eq!(checked["problems"][0]["code"], "flow-empty");
    }
    let (status, refused) = call(&router, "GET", &format!("/api/flow/{SITE}/map"), None).await;
    assert_eq!(
        (status, &refused["type"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("validation-failed")),
        "{refused}"
    );
    let missing = format!("/api/flow/{SITE}/flow?draft=01K0000000000000000000000A");
    let (status, refused) = call(&router, "GET", &missing, None).await;
    assert_eq!((status, &refused["type"]), (StatusCode::NOT_FOUND, &json!("not-found")), "{refused}");
    scratch.close().await;
}

/// The browser suite's made L-Shape, `studio/ui/tests/browser/fixtures/l-shape-6.json`: what Create site stages for the
/// L-Shape at 6 aisles and 54,400 mm, written with `STUDIO_WRITE_BROWSER_FIXTURE=1` and otherwise held to the
/// engine's output at the fixture's own ids, which the browser suite trades for fresh ones on every site it makes.
#[test]
fn the_browser_suites_made_l_shape_is_the_engines_output() {
    let path = repo().join("studio/ui/tests/browser/fixtures/l-shape-6.json");
    let written: Option<Value> = fs::read_to_string(&path).ok().map(|text| serde_json::from_str(&text).unwrap());
    let mut prior = Prior::new(
        written.as_ref().map_or_else(DocId::mint, |w| w["prologue"]["doc"].as_str().unwrap().parse().unwrap()),
    );
    let system = "template~1site~1l-shape";
    for op in
        written.iter().flat_map(|w| w["stages"].as_array().unwrap()).flat_map(|stage| stage["ops"].as_array().unwrap())
    {
        if let Some(key) = op.pointer(&format!("/components/external_ids/{system}")).and_then(Value::as_str) {
            prior.ids.insert(key.parse::<SourceKey>().unwrap(), op["id"].as_str().unwrap().parse().unwrap());
        }
    }
    let (stages, prologue) = made_stages(&prior);
    let stages: Vec<Value> = stages.into_iter().map(|(scope, ops)| json!({ "scope": scope, "ops": ops })).collect();
    let made = json!({ "prologue": prologue, "stages": stages });
    if env::var_os("STUDIO_WRITE_BROWSER_FIXTURE").is_some() {
        fs::write(&path, serde_json::to_string_pretty(&made).unwrap() + "\n").unwrap();
        return;
    }
    assert_eq!(Some(made), written, "write it again with STUDIO_WRITE_BROWSER_FIXTURE=1 after a seed or engine change");
}
