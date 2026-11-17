//! Spec 04 section 8's statement 5 on world-proto-server's own answers: every refusal in fixtures/refusals.json,
//! recorded from world-proto cd4923516 by record-refusals.py beside it, reads by its code and details as its kind, each
//! fact taken from the details and none from the sentence, which is kept verbatim; an unknown code, an unknown detail
//! key and a reworded sentence change nothing the reader says; and an answer without the refusal object is no
//! refusal of WPS's.

use std::slice;

use semver::Version;
use serde::Deserialize;
use serde_json::{Value, json};
use studio_protocol::{EntityId, ScopePath, Seq};
use wps_client::refusal::{Reading, RouteKind, Unresolved, generators_moved, read};
use wps_client::wire::WireRanGenerator;
use wps_client::{GeneratorSet, ProblemKind, WpsError};

#[derive(Deserialize)]
struct Recording {
    recorded_from: String,
    answers: Vec<Recorded>,
}

/// One answer as world-proto-server gave it: what was asked, on which route, and its status and body.
#[derive(Deserialize)]
struct Recorded {
    case: String,
    route: String,
    status: u16,
    body: Value,
}

impl Recorded {
    fn route(&self) -> RouteKind {
        match self.route.as_str() {
            "create_scope" => RouteKind::CreateScope,
            "resolve" => RouteKind::Resolve,
            "export" => RouteKind::Export,
            _ => RouteKind::Other,
        }
    }

    fn reading(&self) -> Reading {
        read(self.route(), self.status, self.body.to_string().as_bytes(), 0)
    }

    fn error(&self) -> WpsError {
        match self.reading() {
            Reading::Error(error) => error,
            other => panic!("{}: {other:?}", self.case),
        }
    }

    fn code(&self) -> &str {
        self.body["code"].as_str().expect("a code")
    }

    fn sentence(&self) -> String {
        self.body["error"].as_str().expect("a sentence").to_owned()
    }

    fn details(&self, pointer: &str) -> &Value {
        self.body["details"].pointer(pointer).unwrap_or_else(|| panic!("{}: no details at {pointer}", self.case))
    }
}

fn recorded() -> Vec<Recorded> {
    let recording: Recording = serde_json::from_str(include_str!("fixtures/refusals.json")).expect("the recording");
    assert_eq!(recording.recorded_from, "world-graph-server at world-proto cd4923516");
    recording.answers
}

fn one(code: &str, route: &str) -> Recorded {
    recorded().into_iter().find(|answer| answer.code() == code && answer.route == route).expect("recorded")
}

fn scope(value: &Value) -> Option<ScopePath> {
    value.as_str().map(|text| text.parse().expect("a scope path"))
}

/// Each 422 the client places: its kind, and where in its details the problem stands, a reference's `referrer`,
/// the first identity a deletion leaves named (`targets`), or the details themselves.
fn placed(code: &str) -> (ProblemKind, &'static str) {
    match code {
        "outside_the_initiating_scope" => (ProblemKind::OutsideTheInitiatingScope, ""),
        "registry_writes_itself_alone" => (ProblemKind::RegistryWritesItselfAlone, ""),
        "created_twice" => (ProblemKind::CreatedTwice, ""),
        "base_unstated" => (ProblemKind::BaseUnstated, ""),
        "created_base_not_zero" => (ProblemKind::CreatedBaseNotZero, ""),
        "stated_twice" => (ProblemKind::StatedTwice, ""),
        "expected_twice" => (ProblemKind::ExpectedTwice, ""),
        "duplicate_op" => (ProblemKind::DuplicateOp, ""),
        "schema_invalid" => (ProblemKind::SchemaInvalid, ""),
        "patch_missing_target" => (ProblemKind::PatchMissingTarget, ""),
        "delete_missing_target" => (ProblemKind::DeleteMissingTarget, ""),
        "replace_missing_target" => (ProblemKind::ReplaceMissingTarget, ""),
        "declare_on_deleted" => (ProblemKind::DeclareOnDeleted, ""),
        "owned_elsewhere" => (ProblemKind::OwnedElsewhere, ""),
        "pin_cycle" => (ProblemKind::PinCycle, ""),
        "left_dangling" => (ProblemKind::LeftDangling, "/targets/0"),
        "undeclared" => (ProblemKind::Undeclared, "/referrer"),
        "dangling" => (ProblemKind::Dangling, "/referrer"),
        "pin_within_unit" => (ProblemKind::PinWithinUnit, "/referrer"),
        "head_across_units" => (ProblemKind::HeadAcrossUnits, "/referrer"),
        "branch_across_units" => (ProblemKind::BranchAcrossUnits, "/referrer"),
        "pin_beyond_head" => (ProblemKind::PinBeyondHead, "/referrer"),
        "tag_not_held" => (ProblemKind::TagNotHeld, "/referrer"),
        other => panic!("a recorded 422 this test places nowhere: {other}"),
    }
}

/// Statement 5: every recorded 422 is `Refused` with its code's kind, its scope, entity, component and path read
/// from where its details stand, `schema_invalid`'s version beside them, and its sentence verbatim. Before the chore,
/// `dangling` and `left_dangling` read no entity: their entity stands under `referrer` and `targets`.
#[test]
fn every_recorded_422_is_its_kind_where_its_details_place_it() {
    let refusals: Vec<Recorded> = recorded().into_iter().filter(|answer| answer.status == 422).collect();
    assert!(refusals.len() >= 20, "the recording holds the 422s");
    for answer in refusals.iter().filter(|answer| answer.route == "changesets") {
        let WpsError::Refused { problem } = answer.error() else { panic!("{}: refused", answer.case) };
        let (kind, at) = placed(answer.code());
        let place = answer.details(at);
        let entity_key = if answer.code() == "duplicate_op" { "written" } else { "entity" };
        let entity: Option<EntityId> = place[entity_key].as_str().map(|id| id.parse().expect("an identity"));
        assert_eq!(problem.kind, kind, "{}", answer.case);
        assert_eq!(problem.scope, scope(&place["scope"]), "{}", answer.case);
        assert_eq!(problem.entity, entity, "{}", answer.case);
        assert_eq!(problem.component.as_deref(), place["component"].as_str(), "{}", answer.case);
        assert_eq!(problem.path.as_deref(), place["path"].as_str(), "{}", answer.case);
        assert_eq!(problem.version.as_deref(), answer.body["details"]["version"].as_str(), "{}", answer.case);
        assert_eq!(problem.text, answer.sentence(), "{}: the sentence kept verbatim", answer.case);
    }
    let schema = one("schema_invalid", "changesets").error();
    let WpsError::Refused { problem } = schema else { panic!("refused") };
    assert_eq!(
        (problem.component.as_deref(), problem.version.as_deref()),
        (Some("template.description"), Some("0.1.0"))
    );
    assert_eq!(problem.path.as_deref(), Some("/title"));
}

/// Statement 5's 409s: `stale_base` is `HeadMoved` naming each stale scope with its base and head; `operation_reused`
/// its own kind with the identifier, the scope its changeset was initiated on and each scope it wrote, answered as
/// our own fault; `scope_exists` names the scope, on the scope route and in a changeset's `create` alike.
#[test]
fn a_recorded_conflict_is_read_by_its_code() {
    let stale = one("stale_base", "changesets");
    let moved: Vec<(ScopePath, Seq, Seq)> = ["/stale/0", "/stale/1"]
        .map(|at| {
            let one = stale.details(at);
            (
                scope(&one["scope"]).expect("a scope"),
                Seq::new(one["base"].as_u64().unwrap()),
                Seq::new(one["head"].as_u64().unwrap()),
            )
        })
        .into();
    assert_eq!(moved.len(), 2, "the map and the flow both moved");
    assert_eq!(stale.error(), WpsError::HeadMoved { moved });

    let reused = one("operation_reused", "changesets");
    let named = vec![(
        scope(reused.details("/named/0/scope")).unwrap(),
        Seq::new(reused.details("/named/0/changeset").as_u64().unwrap()),
    )];
    let expected = WpsError::OperationReused {
        operation: reused.details("/operation").as_str().unwrap().to_owned(),
        initiated: scope(reused.details("/initiated")).unwrap(),
        named,
    };
    let error = reused.error();
    assert_eq!(error, expected);
    assert_eq!((error.api_problem().type_, error.api_problem().status), ("internal", 500), "the fault is ours");

    let exists = one("scope_exists", "create_scope");
    assert_eq!(
        exists.error(),
        WpsError::ScopeExists { scope: scope(exists.details("/scope")).unwrap(), head: Seq::new(0) }
    );
    let missing = one("parent_missing", "create_scope");
    assert!(
        matches!(missing.error(), WpsError::Invalid { text } if text == missing.sentence()),
        "we create parents first"
    );
}

fn stated(generator: &str, version: &str, digest: &str) -> WireRanGenerator {
    WireRanGenerator { generator: generator.to_owned(), version: version.to_owned(), digest: digest.to_owned() }
}

/// Statement 5's `generators_moved`: each recorded difference read against the set the changeset stated into what
/// runs instead: a stated generator that would not run is gone, one that runs at another module is put at it, and
/// each that runs unstated is added.
#[test]
fn the_recorded_generator_set_409s_read_into_what_runs() {
    let answers: Vec<Recorded> = recorded().into_iter().filter(|answer| answer.code() == "generators_moved").collect();
    let [nothing_runs, moved] = answers.as_slice() else { panic!("two recorded") };
    let tiling = nothing_runs.details("/differences/0/stated");
    let tiling = stated("tiling", tiling["version"].as_str().unwrap(), tiling["digest"].as_str().unwrap());
    let Reading::GeneratorsMoved { details, text } = nothing_runs.reading() else { panic!("moved") };
    assert_eq!(text, nothing_runs.sentence());
    let WpsError::GeneratorsMoved { expected, found } = generators_moved(&details, slice::from_ref(&tiling)) else {
        panic!("the generator set")
    };
    assert_eq!((expected.0.len(), found), (1, GeneratorSet::default()), "tiling would not run");

    let wrong = stated("tiling", &tiling.version, &"0".repeat(64));
    let Reading::GeneratorsMoved { details, .. } = moved.reading() else { panic!("moved") };
    let WpsError::GeneratorsMoved { found, .. } = generators_moved(&details, &[wrong]) else { panic!("the set") };
    let runs = |name: &str| {
        let at = moved.details("/differences").as_array().unwrap().iter().find(|one| one["generator"] == name).unwrap();
        (
            Version::parse(at["runs"]["version"].as_str().unwrap()).unwrap(),
            at["runs"]["digest"].as_str().unwrap().to_owned(),
        )
    };
    assert_eq!(found.0.get("tiling"), Some(&runs("tiling")), "tiling at the module that runs");
    let names: Vec<&str> = found.0.keys().map(String::as_str).collect();
    assert_eq!(names, ["carrier_pool", "cohort", "rack", "stamp", "tiling"], "each that runs unstated, added");
}

/// Section 3.7, step 2: each recorded answer of the resolve route that is not a resolution is what it says of the
/// target: a referring scope that does not exist, a reference no scope declared, a pin-rule refusal naming the owner
/// alone, and a miss naming the owner with the problem the reference has there.
#[test]
fn a_recorded_resolve_refusal_names_the_owner() {
    let expected: [(&str, Option<ProblemKind>); 6] = [
        ("resolve_dangling", Some(ProblemKind::Dangling)),
        ("resolve_tag_not_held", Some(ProblemKind::TagNotHeld)),
        ("resolve_pinned_dangling", Some(ProblemKind::PinnedDangling)),
        ("resolve_pin_beyond_head", Some(ProblemKind::PinBeyondHead)),
        ("resolve_pin_within_unit", None),
        ("resolve_moving_across_units", None),
    ];
    for (code, problem) in expected {
        let answer = one(code, "resolve");
        let owner = scope(answer.details("/owner")).expect("an owner");
        let unresolved = Unresolved::Owned { owner, problem };
        assert_eq!(answer.error(), WpsError::Unresolvable { unresolved, text: answer.sentence() }, "{code}");
    }
    let undeclared = one("resolve_undeclared", "resolve");
    assert_eq!(
        undeclared.error(),
        WpsError::Unresolvable { unresolved: Unresolved::Undeclared, text: undeclared.sentence() }
    );
    let nobody = one("unknown_scope", "resolve");
    assert_eq!(nobody.error(), WpsError::Unresolvable { unresolved: Unresolved::NoReferrer, text: nobody.sentence() });
}

/// Statement 5's other statuses: a read's 404 is `NotFound`, whatever it names, and a 400 a request of ours WPS
/// could not read, `Invalid`, each with its sentence.
#[test]
fn a_recorded_miss_is_not_found_and_an_unreadable_request_is_ours() {
    let misses = [("unknown_scope", "export"), ("entity_not_live", "read"), ("entity_unknown", "read")];
    for (code, route) in misses.into_iter().chain([("component_undefined", "read")]) {
        let answer = one(code, route);
        assert_eq!(answer.error(), WpsError::NotFound { text: answer.sentence() }, "{code}");
    }
    for (code, route) in [("query_unreadable", "export"), ("body_syntax", "changesets")] {
        let answer = one(code, route);
        assert_eq!(answer.error(), WpsError::Invalid { text: answer.sentence() }, "{code}");
    }
}

/// The flagged export's refusals, each recorded as WPS answered the request the client makes: an `at` naming a scope
/// no reference at head from the requested scopes can reach, one leaving such a scope out and one past a scope's head
/// are the cut's pin left unresolved, naming the scope its details name; a scope reached at two changesets and a
/// closure past WPS's bound are the export not closed, the problem in the scope the details name.
#[test]
fn the_recorded_export_refusals_name_what_they_found() {
    for code in ["at_out_of_reach", "at_beyond_head"] {
        let answer = one(code, "export");
        let scope = scope(answer.details("/scope")).expect("a scope");
        assert_eq!(answer.error(), WpsError::PinUnresolved { scope, text: answer.sentence() }, "{code}");
    }
    let left_out = one("at_left_out", "export");
    let scope_left = scope(left_out.details("/scopes/0")).expect("a scope");
    assert_eq!(left_out.error(), WpsError::PinUnresolved { scope: scope_left, text: left_out.sentence() });
    let kinds = [("reached_twice", ProblemKind::PinnedTwice), ("closure_too_large", ProblemKind::ClosureTooLarge)];
    for (code, kind) in kinds {
        let answer = one(code, "export");
        let WpsError::ExportNotClosed { problems } = answer.error() else { panic!("{code}: not closed") };
        let [problem] = problems.as_slice() else { panic!("{code}: one problem, not {problems:?}") };
        assert_eq!((problem.kind, &problem.scope), (kind, &scope(answer.details("/scope"))), "{code}");
        assert_eq!(problem.text, answer.sentence(), "{code}: the sentence kept verbatim");
    }
}

/// A flag stated as neither `1` nor `0`, and a parameter the flagged export does not take, are a request of ours that
/// WPS could not read: `export_flag_malformed` and `parameter_unknown` are `Invalid`, each with its sentence.
#[test]
fn a_malformed_flag_and_an_unknown_parameter_are_requests_of_ours() {
    for code in ["export_flag_malformed", "parameter_unknown"] {
        let answer = one(code, "export");
        assert_eq!(answer.status, 400, "{code}");
        assert_eq!(answer.error(), WpsError::Invalid { text: answer.sentence() }, "{code}");
    }
    assert_eq!(one("export_flag_malformed", "export").details("/flag"), &json!("nested"));
    assert_eq!(one("parameter_unknown", "export").details("/parameter"), &json!("flags"));
}

/// WPS's import refuses a flagged export, since a scope it holds in part would read as deletions: the recorded
/// `document_flagged_export` is `Refused` as its own kind, with its sentence.
#[test]
fn an_import_of_a_flagged_export_is_refused_as_its_own_kind() {
    let answer = one("document_flagged_export", "import");
    let WpsError::Refused { problem } = answer.error() else { panic!("refused") };
    assert_eq!((problem.kind, problem.text.as_str()), (ProblemKind::DocumentFlaggedExport, answer.sentence().as_str()));
    let keys = answer.details("/keys").as_array().expect("the keys named");
    assert!(keys.contains(&json!("scopes")) && keys.contains(&json!("referenced")), "{keys:?}");
}

/// The flagged export's `held_twice`, which WPS's index of declared identities makes unreachable, held to the details
/// world-proto's DESIGN.md section 10 gives it on that route: the scope the request names, the entity, and the scopes
/// that hold it.
#[test]
fn an_identity_held_twice_reads_as_its_design_says() {
    let entity = "e1000000-0000-4000-8000-000000000001";
    let details = json!({ "scope": "org/site", "entity": entity, "holders": ["lib/a", "lib/b"] });
    let body = json!({ "error": "a sentence", "code": "held_twice", "details": details }).to_string();
    let Reading::Error(WpsError::ExportNotClosed { problems }) = read(RouteKind::Export, 409, body.as_bytes(), 0)
    else {
        panic!("not closed");
    };
    let [problem] = problems.as_slice() else { panic!("one problem, not {problems:?}") };
    assert_eq!(problem.kind, ProblemKind::HeldTwice);
    assert_eq!(problem.scope, Some("org/site".parse().unwrap()));
    assert_eq!(problem.entity, Some(entity.parse().unwrap()));
}

/// The two refusals no request reaches today, held to the details world-proto's DESIGN.md section 10 gives them:
/// `registry_kept_moving` is `RegistryMoved` naming its scope, never `HeadMoved`, and `ended_by_postgres` is told
/// apart for its one resend.
#[test]
fn the_unreachable_conflicts_read_as_their_design_says() {
    let conflict = |code: &str, details: Value| {
        let body = json!({ "error": "a sentence", "code": code, "details": details }).to_string();
        read(RouteKind::Other, 409, body.as_bytes(), 0)
    };
    let kept = conflict("registry_kept_moving", json!({ "scope": "customer/c/site/s", "attempts": 2 }));
    let scope: ScopePath = "customer/c/site/s".parse().unwrap();
    assert_eq!(kept, Reading::Error(WpsError::RegistryMoved { scope, text: "a sentence".to_owned() }));
    let ended = conflict("ended_by_postgres", json!({ "scopes": ["customer/c/site/s"] }));
    assert_eq!(ended, Reading::EndedByPostgres { text: "a sentence".to_owned() });
}

/// The rule world-proto keeps: a code this client does not know is ignored and read as its status says, `Other` for a
/// 422 and `ServerFault` for a 409; a detail key it does not know changes nothing; nor does a reworded sentence, kept
/// verbatim; and a known code missing a fact it names is a fault on WPS's side, never guessed.
#[test]
fn unknown_codes_keys_and_sentences_change_nothing() {
    let dangling = one("dangling", "changesets");
    let mut anew = dangling.body.clone();
    anew["error"] = json!("worded anew in a later commit");
    anew["details"]["named_later"] = json!({ "a": "fact this client does not know" });
    let read_anew = read(RouteKind::Other, 422, anew.to_string().as_bytes(), 0);
    let Reading::Error(WpsError::Refused { problem }) = read_anew else { panic!("refused") };
    let Reading::Error(WpsError::Refused { problem: recorded }) = dangling.reading() else { panic!("refused") };
    assert_eq!(problem.text, "worded anew in a later commit");
    assert_eq!((problem.kind, &problem.entity, &problem.path), (recorded.kind, &recorded.entity, &recorded.path));

    let unknown = |status: u16| {
        let (error, code) = ("a refusal of a later commit", "refused_some_new_way");
        let body = json!({ "error": error, "code": code, "details": { "scope": "a" } });
        read(RouteKind::Other, status, body.to_string().as_bytes(), 0)
    };
    let Reading::Error(WpsError::Refused { problem }) = unknown(422) else { panic!("refused") };
    assert_eq!((problem.kind, problem.text.as_str()), (ProblemKind::Other, "a refusal of a later commit"));
    assert!(matches!(unknown(409), Reading::Error(WpsError::ServerFault { status: 409, .. })));
    assert!(matches!(unknown(404), Reading::Error(WpsError::NotFound { .. })));

    let unnamed = json!({ "error": "stale", "code": "stale_base", "details": {} }).to_string();
    let read_unnamed = read(RouteKind::Other, 409, unnamed.as_bytes(), 0);
    assert!(matches!(read_unnamed, Reading::Error(WpsError::ServerFault { status: 409, .. })), "{read_unnamed:?}");
}

/// World-proto's DESIGN.md section 10: an answer of 400 or above without the refusal object is hyper's, to a request
/// it could not read, or no server's at all, so no sentence of it is read as a refusal: a 4xx is a request of ours
/// gone wrong, a 5xx a fault on that side, a 502 to 504 the server unavailable.
#[test]
fn an_answer_without_the_refusal_object_is_no_refusal() {
    let bare = |status: u16, body: &str| read(RouteKind::Other, status, body.as_bytes(), 0);
    let sentence_only = json!({ "error": "scope customer/c already exists" }).to_string();
    assert!(matches!(bare(409, &sentence_only), Reading::Error(WpsError::Invalid { .. })), "no code, no refusal");
    assert!(matches!(bare(400, ""), Reading::Error(WpsError::Invalid { .. })), "hyper's own 400");
    assert!(matches!(bare(500, "<html>"), Reading::Error(WpsError::ServerFault { status: 500, .. })));
    assert!(matches!(bare(503, "<html>busy</html>"), Reading::Error(WpsError::Unavailable { attempts: 1, .. })));
}
