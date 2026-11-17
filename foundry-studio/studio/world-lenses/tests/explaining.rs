//! Within `worldlens`'s tests, `explaining` is THE ACCOUNT A NEAR MISS GETS:
//! for an entity that does not take part in a pattern, which clause let it go
//! and what it was looking at.
//!
//! A count says how many things a document has; it never says why the thing in
//! front of somebody is not one of them. That question — asked of a cell that
//! should be a pick place, of a station a document forgot to name — is the
//! second half of what this engine is for, so the failures below are chosen from
//! the real documents rather than built to be explainable.
//!
//! Each test names the entity by searching for it, not by pasting an identity:
//! a document that stops containing the near miss should fail the search
//! loudly rather than explain a different entity's absence.
//!
//! `membership` is asked here too, because it is the same question — does this
//! entity take part — put the other way round, and the two answers have to be
//! the one answer.

mod fixtures;

use fixtures::{document, Fixture};
use std::collections::{BTreeMap, BTreeSet};
use worldlens::{Explanation, Library, Matcher, Verdict};
use site_model::EntityId;

const LIBRARY: &str = foundry_world_lenses::LEGACY_WPS_LENS;

fn library() -> Library {
    Library::parse(LIBRARY).expect("the shipped pattern library parses")
}

/// The account as a person reads it: one line per clause, verdict and clause,
/// in the order the explanation reports them. A test that compares this against
/// a written-out list is asserting the whole account and not a sample of it.
fn account(explanation: &Explanation) -> Vec<String> {
    explanation
        .clauses
        .iter()
        .map(|clause| {
            let verdict = match clause.verdict {
                Verdict::Holds => "holds",
                Verdict::Fails => "FAILS",
                Verdict::Skipped => "skipped",
            };
            format!("{verdict} {}", clause.text)
        })
        .collect()
}

/// The two presentation points walmart-dallas's EasyLoop template stamps carry
/// the marker and say nothing about whose they are. Anchored at one of them, the
/// pattern that follows the reference names the reference as what failed, and
/// says the field reached nothing — which is the document's gap, stated as a
/// clause.
#[test]
fn a_point_that_names_no_station_fails_at_the_reference() {
    let Fixture { world, .. } = document("walmart-dallas");
    let anchor: &EntityId = world
        .entities
        .values()
        .find(|e| {
            e.has("presentation_point") && e.raw("presentation_point").is_some_and(|p| p.get("station").is_none())
        })
        .map(|e| &e.id)
        .expect("walmart-dallas stamps presentation points that name no station");

    let library = library();
    let explanation =
        Matcher::new(&world.entities).explain(library.get("presentation_cell").expect("the pattern"), anchor);
    assert!(!explanation.holds());
    let failure = explanation.first_failure().expect("something failed");
    assert_eq!(failure.text, r#"ref(c, s, "presentation_point.station")"#);
    let note = failure.note.as_deref().expect("the clause says what it saw");
    assert!(note.contains("names no entity"), "{note}");
    // The anchor's own condition held; only the reference did not. And the
    // station's own condition is reported as unasked rather than left out: an
    // account that names three clauses of a three-clause pattern is the whole
    // account, which is what makes the absence of a `FAILS` beside `s: station`
    // mean something.
    assert_eq!(
        account(&explanation),
        ["holds c: presentation_point", "skipped s: station", r#"FAILS ref(c, s, "presentation_point.station")"#,],
        "{explanation}"
    );
    assert_eq!(explanation.clauses[0].verdict, Verdict::Holds);
}

/// A station of another kind fails on the field test, and the note quotes the
/// value the document actually states beside the value the clause wants.
#[test]
fn a_station_of_the_wrong_kind_fails_on_the_field_it_states() {
    let Fixture { world, .. } = document("cencora-v3");
    let anchor: &EntityId = world
        .entities
        .values()
        .find(|e| {
            e.has("pose")
                && e.raw("station").and_then(|s| s.get("kind")).and_then(|k| k.as_str()).is_some_and(|k| k != "pick")
        })
        .map(|e| &e.id)
        .expect("cencora states stations that are not pick stations");

    let library = library();
    let explanation = Matcher::new(&world.entities).explain(library.get("pick_station").expect("the pattern"), anchor);
    assert!(!explanation.holds());
    let failure = explanation.first_failure().expect("something failed");
    assert_eq!(failure.text, r#"s: station(kind == "pick")"#);
    let note = failure.note.as_deref().expect("the clause says what it saw");
    assert!(note.contains("where the clause wants"), "{note}");
    assert!(note.contains(r#"kind == "pick""#), "{note}");
}

/// The commonest miss of all — the entity simply does not carry the component —
/// names the component it lacks and lists what it carries instead, because the
/// next question after "why is this not charging" is always "what is it then".
#[test]
fn a_cell_that_is_not_charging_is_told_what_it_carries() {
    let Fixture { world, .. } = document("cencora-v3");
    let anchor: &EntityId = world
        .entities
        .values()
        .find(|e| e.has("cell") && !e.has("charging"))
        .map(|e| &e.id)
        .expect("a plain floor cell");

    let library = library();
    let explanation = Matcher::new(&world.entities).explain(library.get("charging_cell").expect("the pattern"), anchor);
    let failure = explanation.first_failure().expect("something failed");
    assert_eq!(failure.text, "c: charging");
    let note = failure.note.as_deref().expect("the clause says what it saw");
    assert!(note.contains("it carries no `charging`"), "{note}");
    assert!(note.contains("cell"), "and it says what it does carry: {note}");
}

/// Anchored at an entity that does take part, every clause holds and the
/// explanation is the pattern read back with the entities filled in.
#[test]
fn a_match_explains_as_a_match() {
    let Fixture { world, .. } = document("cencora-v3");
    let library = library();
    let matcher = Matcher::new(&world.entities);
    let pattern = library.get("presentation_cell").expect("the pattern");
    let anchor = matcher.matches(pattern)[0].get("c").expect("the anchor of the first binding").clone();
    let explanation = matcher.explain(pattern, &anchor);
    assert!(explanation.holds(), "{explanation}");
    assert!(explanation.clauses.iter().all(|clause| clause.verdict == Verdict::Holds), "{explanation}");
    // And the entity's memberships name the same pattern at the same variable.
    let memberships = matcher.membership(&library, &anchor);
    assert!(memberships.iter().any(|m| m.pattern == "presentation_cell" && m.variable == "c"), "{memberships:?}");
}

/// An identity no entity of the world has is not a silent zero: the explanation
/// says that is what happened, which is what a person who mistyped an identity
/// needs to read.
#[test]
fn an_identity_the_world_does_not_have_says_so() {
    let Fixture { world, .. } = document("minimal-putaway2");
    let library = library();
    let missing: EntityId = "00000000-0000-4000-8000-000000000000".parse().expect("a well-formed identity");
    let explanation =
        Matcher::new(&world.entities).explain(library.get("charging_cell").expect("the pattern"), &missing);
    assert!(!explanation.holds());
    assert_eq!(explanation.clauses.len(), 1);
    assert_eq!(explanation.clauses[0].note.as_deref(), Some("no entity of this world has that identity"));
}

/// An entity that takes part explains as taking part even when the first
/// candidate for a later variable is a dead end.
///
/// Cencora's sectors name two runs each, and only one of the two is reached
/// from the `-y` face. Sector `151ce687-f087-5999-a75e-cbd1194c461a` is the case
/// Codex's review found: its lower-identity run `9e632a50-…` is accessed from
/// `+y`, so an account that binds `r` to the first run it reaches and never
/// reconsiders goes looking for a `-y` access of that run, finds none, and
/// calls a real match a failure. The claim asserted here is the general one —
/// every sector the pattern binds explains as holding — with the two-faced
/// shape asserted separately so the test cannot go quiet if cencora's sectors
/// stop having it.
#[test]
fn a_dead_end_candidate_does_not_turn_a_match_into_a_failure() {
    let Fixture { world, .. } = document("cencora-v3");
    let library = Library::parse(
        r#"pattern minus_y_run_of_sector {
            t: sector;
            r: stamp;
            a: access(face == "-y");
            ref(t, r, "sector.members");
            ref(a, r, "access.rack");
        }"#,
    )
    .expect("the pattern parses");
    let pattern = library.get("minus_y_run_of_sector").expect("the pattern");
    let matcher = Matcher::new(&world.entities);

    let bound: BTreeSet<EntityId> = matcher.matches(pattern).iter().filter_map(|b| b.get("t")).cloned().collect();
    assert!(!bound.is_empty(), "cencora states sectors whose runs are accessed from `-y`");
    for sector in &bound {
        let explanation = matcher.explain(pattern, sector);
        assert!(explanation.holds(), "{sector} takes part, and the account says otherwise:\n{explanation}");
        let membership = matcher.membership(&library, sector);
        assert!(
            membership.iter().any(|m| m.pattern == "minus_y_run_of_sector" && m.variable == "t"),
            "{sector}: {membership:?}"
        );
    }

    // The shape that makes this a test rather than a tautology: at least one of
    // those sectors names runs of two different faces, so the `-y` one is not
    // the only thing `r` could have bound to.
    let face_of: BTreeMap<String, &str> = world
        .entities
        .values()
        .filter_map(|e| {
            let access = e.raw("access")?;
            Some((access.get("rack")?.as_str()?.to_string(), access.get("face")?.as_str()?))
        })
        .collect();
    let two_faced = world.entities.values().filter(|e| bound.contains(&e.id)).any(|e| {
        let faces: BTreeSet<&str> = e
            .raw("sector")
            .and_then(|s| s.get("members"))
            .and_then(|m| m.as_array())
            .map(|members| members.iter().filter_map(|m| face_of.get(m.as_str()?).copied()).collect())
            .unwrap_or_default();
        faces.len() > 1
    });
    assert!(two_faced, "cencora states a sector whose two runs are accessed from different faces");
}

/// A walk that failed says which chain it followed and where it ran out, in
/// place of the one line every relation without an arm of its own gets.
///
/// "Why is this cell not in that machine" is the question the five `machine_*`
/// patterns are asked most, and the answer a person needs is how far the frame
/// chain got: a cell on the floor reaches its floor frame and the site frame
/// above it, and neither of those is an EasyLoop.
#[test]
fn a_failed_walk_names_the_chain_it_followed_and_where_it_ended() {
    let Fixture { world, .. } = document("minimal-easyloop");
    let library = library();
    let matcher = Matcher::new(&world.entities);
    let pattern = library.get("machine_cell").expect("the pattern");
    let inside: BTreeSet<EntityId> = matcher.matches(pattern).iter().filter_map(|b| b.get("c")).cloned().collect();
    let outside = world
        .entities
        .values()
        .find(|e| e.has("cell") && e.has("pose") && !inside.contains(&e.id))
        .expect("the map poses cells outside the machine as well as in it");

    let explanation = matcher.explain(pattern, &outside.id);
    assert!(!explanation.holds(), "{} is not one of the machine's cells", outside.id);
    let failure = explanation.first_failure().expect("something failed");
    assert_eq!(failure.text, r#"ref_walk(c, m, "pose.frame", "frame.parent")"#);
    let note = failure.note.as_deref().expect("the clause says what it saw");
    assert!(note.contains("`pose.frame`, `frame.parent`"), "the chain it followed: {note}");
    assert!(note.contains(&outside.id.to_string()), "the entity it started at: {note}");
    // Where it ended is the one frame with no parent, the site: a floor cell's
    // chain climbs floor to site and stops, and the note names that entity.
    let top = world
        .entities
        .values()
        .find(|e| e.has("frame") && e.raw("frame").and_then(|frame| frame.get("parent")).is_none())
        .expect("the frame chain has a top");
    assert!(note.contains(&format!("ends at {}", top.id)), "where it ended: {note}");
}

/// `membership` answers the same question `matches` does, on a pattern a
/// relation decides. Fixing a variable to one entity is a restriction on that
/// variable's own step in the join; written into the candidate list instead it
/// would be what every other clause reads, and an entity would take part in a
/// pattern merely by having been asked about.
///
/// `machine_cell` is the case: the minimal EasyLoop map poses twenty-eight of
/// its cells in the machine's frame and the rest on the floor, so a cell's
/// membership turns on the walk and on nothing the cell itself carries.
#[test]
fn membership_names_only_the_cells_the_walk_reaches() {
    let Fixture { world, .. } = document("minimal-easyloop");
    let library = library();
    let matcher = Matcher::new(&world.entities);
    let pattern = library.get("machine_cell").expect("the pattern");
    let inside: BTreeSet<EntityId> = matcher.matches(pattern).iter().filter_map(|b| b.get("c")).cloned().collect();
    let cells = world.entities.values().filter(|e| e.has("cell")).count();
    assert_eq!(inside.len(), 28, "the machine's own cells");
    assert!(inside.len() < cells, "and the map states {cells} cells in all");

    let belongs = |id: &EntityId| {
        matcher.membership(&library, id).iter().any(|m| m.pattern == "machine_cell" && m.variable == "c")
    };
    let member = inside.iter().next().expect("one of them");
    assert!(belongs(member), "{member} is posed in the machine's frame");
    let elsewhere = world
        .entities
        .values()
        .find(|e| e.has("cell") && !inside.contains(&e.id))
        .expect("a cell posed outside the machine");
    assert!(!belongs(&elsewhere.id), "{} is posed on the floor", elsewhere.id);
}

/// Every condition of the anchor is reported, not just the first one that
/// fails. A cell that is neither charging nor a presentation point fails both,
/// and an account that stopped at the first would leave a reader to guess
/// whether the second was asked.
#[test]
fn every_condition_of_a_variable_gets_a_verdict() {
    let Fixture { world, .. } = document("cencora-v3");
    let library = Library::parse("pattern p { c: cell, charging, presentation_point; }").expect("it parses");
    let anchor: &EntityId = world
        .entities
        .values()
        .find(|e| e.has("cell") && !e.has("charging") && !e.has("presentation_point"))
        .map(|e| &e.id)
        .expect("a plain floor cell");
    let explanation = Matcher::new(&world.entities).explain(library.get("p").expect("the pattern"), anchor);
    assert_eq!(
        account(&explanation),
        ["holds c: cell", "FAILS c: charging", "FAILS c: presentation_point"],
        "{explanation}"
    );
}

/// Every relation is reported too, and the one that let the variable go is the
/// one that fails. A sector's members are stamps, so the reference reaches
/// candidates; a sector states no pose, so the walk up the frame chain reaches
/// nothing and is what stops it — and the reference is reported as unanswered
/// rather than dropped.
#[test]
fn every_relation_gets_a_verdict_and_the_failing_one_is_named() {
    let Fixture { world, .. } = document("cencora-v3");
    let library = Library::parse(
        r#"pattern p { t: sector; m: stamp; ref(t, m, "sector.members"); ref_walk(t, m, "pose.frame", "frame.parent"); }"#,
    )
    .expect("it parses");
    let anchor: &EntityId = world
        .entities
        .values()
        .find(|e| e.raw("sector").and_then(|s| s.get("members")).is_some_and(|m| !m.as_array().unwrap().is_empty()))
        .map(|e| &e.id)
        .expect("cencora states sectors that name their runs");
    let explanation = Matcher::new(&world.entities).explain(library.get("p").expect("the pattern"), anchor);
    assert_eq!(
        account(&explanation),
        [
            "holds t: sector",
            "skipped m: stamp",
            r#"skipped ref(t, m, "sector.members")"#,
            r#"FAILS ref_walk(t, m, "pose.frame", "frame.parent")"#,
        ],
        "{explanation}"
    );
    let reference = &explanation.clauses[2];
    assert_eq!(reference.note.as_deref(), Some("one of its variables never bound"));
}

/// An optional variable that nothing satisfies is reported as unasked rather
/// than as a failure — that is what optional means — and it is reported, beside
/// the required clause that did fail.
#[test]
fn an_optional_variable_that_binds_nothing_is_reported_as_unasked() {
    let Fixture { world, .. } = document("walmart-dallas");
    let library = Library::parse(
        r#"pattern p {
            c: presentation_point;
            s: station;
            d?: no_such_component;
            ref(c, s, "presentation_point.station");
        }"#,
    )
    .expect("it parses");
    let anchor: &EntityId = world
        .entities
        .values()
        .find(|e| {
            e.has("presentation_point") && e.raw("presentation_point").is_some_and(|p| p.get("station").is_none())
        })
        .map(|e| &e.id)
        .expect("walmart-dallas stamps presentation points that name no station");
    let explanation = Matcher::new(&world.entities).explain(library.get("p").expect("the pattern"), anchor);
    assert_eq!(
        account(&explanation),
        [
            "holds c: presentation_point",
            "skipped s: station",
            r#"FAILS ref(c, s, "presentation_point.station")"#,
            "skipped d: no_such_component",
        ],
        "{explanation}"
    );
    // The account of an entity that fails one clause is still a failure, and the
    // optional variable does not make it one.
    assert!(!explanation.holds());
}
