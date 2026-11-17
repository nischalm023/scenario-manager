//! Spec 04 section 8's statements 1 to 3: what this crate adds to the value types, the kinship classifier, and the
//! spelling a draft gives each reference a consumer states.

mod support;

use serde_json::json;
use studio_protocol::{DeclaredId, EntityId, Kinship, Pin, RefPin, ScopePath, Seq};
use support::{client, path, read_site, site};
use wps_client::kinship::classify;
use wps_client::value::ValueError;
use wps_client::{Author, DraftBuilder, Prologue, Reference, Version};

/// Statement 1: `Version` and `Reference` refuse a branch where a version is due; `Prologue` round-trips WPS's form.
#[test]
fn a_version_is_a_changeset_or_a_tag_and_never_a_branch() {
    assert_eq!("17".parse::<Version>(), Ok(Version::Seq(Seq::new(17))));
    assert_eq!("tag:v3".parse::<Version>().map(|version| version.to_string()), Ok("tag:v3".to_owned()));
    assert_eq!("main".parse::<Version>(), Err(ValueError::Branch("main".to_owned())));
    assert_eq!("head".parse::<Version>(), Err(ValueError::Head));
    let main: Pin = "main".parse().expect("a branch pin");
    assert_eq!(Version::try_from(main), Err(ValueError::Branch("main".to_owned())));

    let id: EntityId = DeclaredId::mint().into();
    let branch = RefPin::Pin("main".parse().expect("a branch pin"));
    assert_eq!(Reference::lateral(id.clone(), branch), Err(ValueError::Branch("main".to_owned())));
    assert_eq!(Reference::lateral(id.clone(), RefPin::Head), Err(ValueError::Head));
    let pinned = Reference::lateral(id.clone(), RefPin::Pin(Pin::Changeset(Seq::new(4)))).expect("a changeset pin");
    assert_eq!(pinned, Reference::Lateral(id, Version::Seq(Seq::new(4))));
}

/// Statement 1: a prologue reads and writes exactly WPS's changeset form (`wp:server/core/src/prologue.rs`).
#[test]
fn a_prologue_round_trips_the_wire_form() {
    let wire = json!({
        "doc": "2d2b5c2e-e7e7-455f-b74c-fa388f0b869c",
        "kind": "world",
        "name": "L shape 6",
        "primary_frame": "23dcb09e-5b10-4b6c-bdb5-8a01b12ebaa6",
        "depends_on": ["9b0c7a64-1d2e-4f3a-8b5c-6d7e8f9a0b1c"]
    });
    let prologue: Prologue = serde_json::from_value(wire.clone()).expect("WPS's prologue");
    assert_eq!(serde_json::to_value(&prologue).expect("serializes"), wire);
    let bare = json!({ "doc": "2d2b5c2e-e7e7-455f-b74c-fa388f0b869c" });
    let prologue: Prologue = serde_json::from_value(bare.clone()).expect("a prologue stating only its id");
    assert_eq!(serde_json::to_value(&prologue).expect("serializes"), bare, "nothing unstated is written");
}

/// Statement 2: the classifier gives every row of section 3.7's table under ruling D76.
#[test]
fn kinship_follows_the_nearest_boundary() {
    let boundaries =
        ["customer/c/site/s", "customer/c/site/s2", "template/site/t", "catalog/agent/a", "catalog/artifact/x"];
    let flagged = |scope: &ScopePath| boundaries.contains(&scope.as_str());
    let rows = [
        ("customer/c/site/s/flow", "customer/c/site/s/map", Kinship::Kin),
        ("customer/c/site/s/flow", "customer/c/site/s", Kinship::Kin),
        ("customer/c/site/s", "customer/c/site/s/map", Kinship::Kin),
        ("customer/c/site/s/flow/x", "customer/c/site/s/map", Kinship::Kin),
        ("template/site/t/flow", "template/site/t/map", Kinship::Kin),
        ("customer/c/site/s/flow", "customer/c/site/s2/map", Kinship::Lateral),
        ("customer/c/site/s", "customer/c/site/s2", Kinship::Lateral),
        ("customer/c/site/s/flow", "customer/c", Kinship::Lateral),
        ("catalog/agent/a", "catalog/artifact/x", Kinship::Lateral),
        ("customer/c/site/s/fleet", "catalog/agent/a", Kinship::Lateral),
    ];
    for (referrer, target, expected) in rows {
        assert_eq!(classify(&path(referrer), &path(target), flagged), expected, "{referrer} → {target}");
    }
    assert_eq!(
        classify(&path("customer/c"), &path("customer/c/site"), flagged),
        Kinship::Lateral,
        "no boundary is shared"
    );
}

/// Statement 3: `DraftBuilder::reference` reproduces section 3.7's spelling table, refuses a branch on a lateral
/// reference, and writes a tag as written.
#[tokio::test]
async fn a_draft_spells_each_reference_as_stated() {
    let client = client().await;
    let root = site(&client, "spelling").await;
    let base = read_site(&client, &root).await;
    let author = Author::new("tester").expect("an author");
    let builder = DraftBuilder::new(&base, root, author, "spelling", "op-spelling");
    let id: EntityId = "0c2f6d5e-8a41-4f7e-9b13-5d2e7a9c4f10".parse().expect("an id");
    assert_eq!(builder.reference(&Reference::Local(id.clone())), "0c2f6d5e-8a41-4f7e-9b13-5d2e7a9c4f10");
    assert_eq!(builder.reference(&Reference::Kin(id.clone())), "0c2f6d5e-8a41-4f7e-9b13-5d2e7a9c4f10@head");
    let lateral = Reference::Lateral(id.clone(), Version::Seq(Seq::new(3)));
    assert_eq!(builder.reference(&lateral), "0c2f6d5e-8a41-4f7e-9b13-5d2e7a9c4f10@3");
    let tagged = Reference::Lateral(id.clone(), "tag:v3".parse().expect("a tag"));
    assert_eq!(
        builder.reference(&tagged),
        "0c2f6d5e-8a41-4f7e-9b13-5d2e7a9c4f10@tag:v3",
        "a tag is written as written"
    );
    let branch = RefPin::Pin("main".parse().expect("a branch"));
    assert!(Reference::lateral(id, branch).is_err(), "a branch on a lateral reference is refused before spelling");
}
