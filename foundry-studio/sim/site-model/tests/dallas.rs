//! The Walmart Dallas slice, loaded end to end.
//!
//! The unit tests beside each module pin one rule at a time; this file pins the
//! rules *composed*, against the fixture the spec's own example became. Every
//! count below is arithmetic done from the documents and written out in the
//! comment beside it, so a change in what the loader derives shows up here as a
//! number that no longer matches a paper trail — not as a number that used to be
//! different.
//!
//! The fixture under `tests/fixtures/` is the v0.5 example carried into v0.6 by
//! the mechanical transform the spec states: suffixes off the length fields, the
//! rack's four vertical fields folded into one `levels` list, and each
//! boards-access folded into the relay level whose face and reach it stated. It
//! has since lost its exclusion entity too — the charging area's rectangle sits
//! in the highway tiling's own `exclusions` now, which is the same six cells
//! removed by the tiling that owns them. Its primary frame declares
//! `unit_mm: 1`, so every number the example wrote is the number this fixture
//! still says.

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use site_model::components::product::{CellAddress, Queue};
use site_model::components::spatial::Frame;
use site_model::components::structure::AccessCell;
use site_model::{Coord, Document, EntityId, LayerSet, LoadError, Steps, World};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/seed/application-tests/native/fixtures/walmart-dallas")
        .join(name)
}

fn id(text: &str) -> EntityId {
    text.parse().expect("a literal identity in this test")
}

/// The identity a generator derives at one key: the generator, then the chain of
/// segments beneath it. That address *is* what the derived entity is named by
/// (SPEC §3), so a test writes it exactly as a document would.
fn at(generator: impl std::fmt::Display, key: impl std::fmt::Display) -> EntityId {
    format!("{generator}/{key}")
        .parse()
        .expect("a well-formed derivation address in this test")
}

const HIGHWAY: &str = "272542c8-2ea2-441f-8807-8a37aafc8f10";
const AISLE: &str = "cdda8cce-008d-4c20-b5b7-4d5a816e05c8";
const TUNNEL_CORRIDOR: &str = "df6ef2f1-e8e6-46c3-abde-55461a2c6d16";
const APPROACH: &str = "4e9d1209-6065-4a05-89b4-1c552ea6e17d";
const SPINE: &str = "06949220-afd7-4056-b9f5-6dd282ee06e4";
const ROW_A: &str = "d26adfc0-dd23-4cc7-a409-f052e64abfbe";
const ROW_B: &str = "568cbecd-264e-4912-aa4d-e5c4762b03dd";
const TUNNEL_ROW: &str = "dcc757f2-393f-4f89-b25a-3949329f264b";
const ORDER_BUFFER: &str = "8b43cab5-dd0d-428c-9c71-c610c8ad0f98";
const EASYLOOP: &str = "b83f97cc-9ff1-49ec-b547-5c1a1ff88411";
const STAGING: &str = "dea32596-e800-4fd5-8371-d800650fe181";
const ROW_A_ACCESS: &str = "9c4e1f7a-2b58-4d63-8e0a-f31c6b90d472";
const ROW_B_ACCESS: &str = "b7d2e830-45a1-4c96-a5f8-2e64d901c3ab";
const BUFFER_ACCESS: &str = "d3b18c60-72f9-4e84-a1d5-06b4e7c92f81";
const BUFFER_LANE: &str = "3a9f2c17-6d84-4e05-9b21-7c0e5d4a1f63";
const BUFFER_VTM_COHORT: &str = "242b6569-e299-44df-bccf-af11e96751d0";

/// The one relay level every bay of the two aisle rows and the buffer carries:
/// four positions at 400 above the floor, second in a `levels` list whose first
/// entry is the 350 shelf below it.
const RELAY_LEVEL: &str = "relay:1";

fn load() -> (LayerSet, World) {
    site_model::load(&fixture("scenario.json")).expect("the fixture loads with no errors")
}

#[test]
fn the_fixture_loads_with_no_errors() {
    let (set, world) = load();
    // catalog, world, fleet, workload, policy — the five the scenario names.
    assert_eq!(set.layers.len(), 5);
    assert_eq!(world.entities.len(), 4370);
    // 5 templates + 34 world declarations + 10 fleet + 2 workload. The world lost
    // its three boards-accesses: the face and the reach each stated are facts
    // about the steel, and they ride on the rack's relay level now. It lost the
    // charging area too — an entity whose whole job was subtracting six highway
    // cells, which the highway's own tiling now states.
    assert_eq!(world.entities.len() - world.derived_count(), 51);
}

#[test]
fn every_generator_expands_to_the_count_the_documents_imply() {
    let (_, world) = load();
    let count = |generator: &str| {
        world
            .counts
            .get(&generator.parse().expect("a literal UUID in this test"))
            .copied()
            .unwrap_or(0)
    };

    // Highway: 1800/900 = 2 columns x 29700/900 = 33 rows = 66 cells, less the
    // six the charging exclusion covers.
    assert_eq!(count(HIGHWAY), 60);
    // Aisle: 16200/900 = 18 x 1800/900 = 2 = 36, less the one suppressed tile.
    assert_eq!(count(AISLE), 35);
    // Tunnel corridor: 16200/900 = 18 x 1300/650 = 2 = 36.
    assert_eq!(count(TUNNEL_CORRIDOR), 36);
    // Approach corridor: 3600/900 = 4 x 900/900 = 1.
    assert_eq!(count(APPROACH), 4);
    // Station spine: 900/900 = 1 x 15300/900 = 17.
    assert_eq!(count(SPINE), 17);
    // Staging sector: 900/900 = 1 x 1800/900 = 2.
    assert_eq!(count(STAGING), 2);

    // Rack rows A and B: 6 instances, each an instance entity + its `rack`
    // internal + 4 relays (one relay level, four positions) + 23 storage levels
    // x 5 wide x 2 deep = 230 slots. 6 x (1 + 1 + 4 + 230) = 1416. The relay
    // level sits second in a 24-entry list, so the counts are what they were and
    // only the keys moved: `relay:1,{i}` and `slot:{level},{col},{depth}` over
    // the levels that are not it.
    assert_eq!(count(ROW_A), 1416);
    assert_eq!(count(ROW_B), 1416);
    // An access derives nothing at all — not a cell, not a link.
    for access in [ROW_A_ACCESS, ROW_B_ACCESS, BUFFER_ACCESS] {
        assert_eq!(count(access), 0);
    }
    // Tunnel row: no relay level, and its first shelf starts at 4200, so its
    // levels list holds the 12 storage levels the old tunnel left above it:
    // 12 x 5 x 2 = 120 slots. 6 x (1 + 1 + 120) = 732.
    assert_eq!(count(TUNNEL_ROW), 732);
    // Order buffer: the same rack template, twice. 2 x 236 = 472.
    assert_eq!(count(ORDER_BUFFER), 472);
    assert_eq!(count(BUFFER_LANE), 6);

    // EasyLoop: 1 instance + 8 template internals (station, two port cells, the
    // queue region, the waiting region, the internal stitch, two presentation
    // points) + the queue's 3 x 2 = 6 cells + the waiting block's 2 x 6 = 12.
    assert_eq!(count(EASYLOOP), 27);
    // Box induction and both extractions: 1 instance + 1 internal each.
    for stamp in [
        "a3e242cd-44de-421d-8ce1-e16e3c5d5ba8",
        "b172c061-646b-416b-ac1b-3ffdce97c08f",
        "c8bbeead-7cb7-4d42-8455-538f0491449a",
    ] {
        assert_eq!(count(stamp), 2);
    }

    // Cohorts and pools mint one entity each: 12 + 2 + 1 robots, 60 + 15 totes.
    assert_eq!(count("e6097dc8-9b60-40fe-a636-cd3a45869e81"), 12);
    assert_eq!(count("e7ffa31d-5d23-452a-ae2f-9c254ecdea9d"), 2);
    assert_eq!(count(BUFFER_VTM_COHORT), 1);
    assert_eq!(count("700da094-b7a6-4c82-abbe-b7b26bcc3e98"), 60);
    assert_eq!(count("e0fe9525-85a1-4e5a-a7e5-cd089bcacf97"), 15);

    // 160 region cells + 4069 from the stamps + 15 robots + 75 carriers.
    assert_eq!(world.derived_count(), 4319);
}

#[test]
fn the_charging_exclusion_takes_six_highway_cells_and_nothing_else() {
    let (_, world) = load();
    let highway = id(HIGHWAY);
    // The exclusion runs y = 26000..29000; highway row centres sit at
    // (row + 0.5) x 900, so rows 29, 30 and 31 (26550, 27450, 28350) fall
    // strictly inside while row 28 (25650) and row 32 (29250) do not.
    for row in [29, 30, 31] {
        for col in [0, 1] {
            assert!(
                !world
                    .entities
                    .contains_key(&at(&highway, format!("cell:{col},{row}"))),
                "highway cell:{col},{row} should have been excluded"
            );
        }
    }
    for row in [28, 32] {
        for col in [0, 1] {
            assert!(
                world
                    .entities
                    .contains_key(&at(&highway, format!("cell:{col},{row}"))),
                "highway cell:{col},{row} sits outside the charging area"
            );
        }
    }
    // Six exclusions plus the one authored suppression of an aisle tile.
    assert_eq!(world.suppressions.len(), 7);
    let excluded: Vec<&EntityId> = world
        .suppressions
        .iter()
        .filter_map(|s| s.excluded_by.as_ref())
        .collect();
    assert_eq!(excluded.len(), 6);
    // And each removal is booked against the highway itself: the carve-out is
    // the highway tiling's own, so nothing else could have made it.
    assert!(excluded.iter().all(|by| *by == &highway), "{excluded:?}");
}

#[test]
fn the_suppressed_aisle_tile_is_gone_and_no_arc_reaches_it() {
    let (_, world) = load();
    let hole = at(AISLE, "cell:17,1");
    assert!(!world.entities.contains_key(&hole));
    assert!(
        !world.edges.iter().any(|e| e.from == hole || e.to == hole),
        "the graph must route around a hole, not around a flag"
    );
    // Its neighbour on the same column survives, so the suppression removed one
    // tile rather than a strip.
    assert!(world.entities.contains_key(&at(AISLE, "cell:17,0")));
}

/// The cell each relay binds, and who else is standing on it.
fn binding(world: &World, run: &str, bay: usize, index: usize) -> EntityId {
    let relay = at(
        run,
        format!("instance:{bay}/local:rack/{RELAY_LEVEL},{index}"),
    );
    let bound: AccessCell = world.entities[&relay]
        .require("access_cell")
        .unwrap_or_else(|_| panic!("{run} bay {bay} relay {index} bound no cell"));
    match bound.cell {
        CellAddress::Id(cell) => cell,
        other => panic!("the search binds a cell by its own identity, not {other:?}"),
    }
}

/// Every relay of both aisle rows binds a cell of the aisle they face, and every
/// relay of the buffer binds a cell of the buffer lane.
///
/// Nothing is minted for any of them: the cell an HTM stands on to work a
/// diving board is a cell of the floor it was already driving. Many relays per
/// cell is the expected shape rather than an error — relay pitch is 690 and the
/// aisle's is 900, and no site makes those agree.
#[test]
fn every_relay_binds_a_cell_of_the_floor_it_faces() {
    let (_, world) = load();
    let cells_of = |generator: &str| -> BTreeSet<EntityId> {
        world
            .entities
            .values()
            .filter(|e| e.parent.as_ref() == Some(&id(generator)))
            .map(|e| e.id.clone())
            .collect()
    };
    let aisle = cells_of(AISLE);
    let lane = cells_of(BUFFER_LANE);

    for bay in 0..6 {
        for index in 0..4 {
            for run in [ROW_A, ROW_B] {
                let cell = binding(&world, run, bay, index);
                assert!(
                    aisle.contains(&cell),
                    "{run} bay {bay} relay {index} bound something outside aisle 1"
                );
            }
        }
    }
    for bay in 0..2 {
        for index in 0..4 {
            assert!(lane.contains(&binding(&world, ORDER_BUFFER, bay, index)));
        }
    }

    // 6 bays x 4 relays x 2 rows + 2 bays x 4 relays on the buffer.
    assert_eq!(world.component_census().get("access_cell"), Some(&56));
}

/// What the sprinkler riser costs, now that the hole is in the floor a relay
/// reaches for rather than in a set of cells the rack minted.
///
/// Row B bay 0's three western relays would each take the aisle tile at
/// x = 17750; it is suppressed, so they fall through to the next-nearest cell
/// that survives — the same row one column west, not the row behind it. That is
/// nearest-within-reach doing exactly what it says, and it is why the reach is
/// stated per relay level rather than assumed.
#[test]
fn a_suppressed_tile_sends_its_relays_one_column_over() {
    let (_, world) = load();
    let west = at(AISLE, "cell:16,1");
    for index in 0..3 {
        assert_eq!(binding(&world, ROW_B, 0, index), west, "relay {index}");
    }
    assert_eq!(binding(&world, ROW_B, 0, 3), at(AISLE, "cell:15,1"));
    assert!(!world.entities.contains_key(&at(AISLE, "cell:17,1")));
}

#[test]
fn the_graph_has_the_arcs_the_flow_rules_and_stitches_imply() {
    let (_, world) = load();
    let directed = world.edges.iter().filter(|e| !e.bidirectional).count();
    let links = world.edges.len() - directed;

    // The three regions that declare a flow:
    // Highway, one_way +y over 30 surviving rows: 28 consecutive row pairs per
    // column x 2 columns = 56 along-axis arcs, plus 30 two-way row changes =
    // 60 cross arcs. 116.
    // Aisle, one_way +x after the policy patch, 35 cells: 17 + 16 = 33 along,
    // 17 row changes x 2 = 34 cross. 67.
    // Tunnel corridor, one_way +x, 36 cells: 17 x 2 = 34 along, 18 x 2 = 36
    // cross. 70.
    //
    // The five that declare none and are therefore two-way, every orthogonal
    // neighbour pair counted twice:
    // approach corridor 4 x 1 -> 3 pairs -> 6; station spine 1 x 17 -> 16 pairs
    // -> 32; staging sector 1 x 2 -> 1 pair -> 2; the EasyLoop's order queue
    // 3 x 2 -> 4 + 3 = 7 pairs -> 14; its waiting block 2 x 6 -> 6 + 10 = 16
    // pairs -> 32.
    // Plus the order buffer's lane, 6 x 1 two-way: 5 pairs -> 10.
    assert_eq!(directed, 116 + 67 + 70 + 6 + 32 + 2 + 14 + 32 + 10);

    // 2 highway<->aisle + 1 approach<->aisle + 2 highway<->tunnel + 1
    // approach<->spine + 2 explicit EasyLoop entries + 1 each for box induction
    // and the two extractions + 2 spine<->staging + 2 waiting<->queue inside the
    // EasyLoop template. A relay's binding is not among them: it names a cell,
    // and naming a cell is not an arc.
    assert_eq!(links, 2 + 1 + 2 + 1 + 2 + 3 + 2 + 2);
    assert_eq!(world.edges.len(), 364);
}

/// Flow restricts travel; it never grants it. A cell_grid that declares no flow
/// rule is two-way, so the station spine every station on this slice hangs off
/// is a lane a robot can actually drive along.
#[test]
fn a_cell_grid_with_no_flow_rule_is_two_way() {
    let (_, world) = load();
    let spine = id(SPINE);
    let (first, second) = (at(&spine, "cell:0,3"), at(&spine, "cell:0,4"));
    assert!(world
        .edges
        .iter()
        .any(|e| e.from == first && e.to == second));
    assert!(world
        .edges
        .iter()
        .any(|e| e.from == second && e.to == first));
    // 17 cells in one column: 16 neighbour pairs, both ways.
    assert_eq!(world.edges.iter().filter(|e| e.owner == spine).count(), 32);
    // And the same inside a stamped template region: the EasyLoop's waiting
    // block is 2 x 6, so 6 + 10 = 16 pairs.
    let waiting = at(EASYLOOP, "instance:0/local:waiting_block");
    assert_eq!(
        world.edges.iter().filter(|e| e.owner == waiting).count(),
        32
    );
}

#[test]
fn the_policy_overlay_reaches_both_pipeline_phases() {
    let (_, world) = load();
    // The patch replaced the aisle's flow before expansion, so the generator
    // compiled eastbound arcs.
    let aisle = &world.entities[&id(AISLE)];
    assert_eq!(aisle.components["flow"]["direction"], "+x");
    let first = at(AISLE, "cell:0,0");
    let second = at(AISLE, "cell:1,0");
    assert!(world
        .edges
        .iter()
        .any(|e| e.from == first && e.to == second));
    assert!(!world
        .edges
        .iter()
        .any(|e| e.from == second && e.to == first));

    // The override reached a derived entity after expansion: the EasyLoop's
    // stamped queue admits four cells this run, where the template's own
    // horseshoe names six.
    let queue = &world.entities[&at(EASYLOOP, "instance:0/local:order_queue")];
    assert_eq!(queue.components["queue"]["path"].as_array().unwrap().len(), 4);

    // And the two order-buffer racks hold orders rather than the template's
    // inventory, addressed through a chained derivation key.
    for instance in ["instance:0", "instance:1"] {
        let rack = &world.entities[&at(ORDER_BUFFER, format!("{instance}/local:rack"))];
        assert_eq!(rack.components["storage"]["holds"], "order");
    }
}

/// EasyLoop 1's queue path, after `policy.json` replaces it wholesale with full
/// chained addresses through the stamp. Every step must land on a cell that
/// exists — and on the cells the template's own `local_ref` form named, since
/// `instance:0/local:order_queue/cell:2,0` reached from the stamp and
/// `cell:2,0` reached from the stamped queue are the same chain of hashes.
///
/// Four steps, not the template's six: the path is what a queue admits on, so
/// the scenario that admits fewer cells is the scenario that names fewer. The
/// two it gives up are the entry leg's; the cells stay tiled floor either way.
#[test]
fn the_easyloop_queue_path_resolves_to_the_cells_the_policy_layer_admits() {
    let (_, world) = load();
    let owner = at(EASYLOOP, "instance:0/local:order_queue");
    let queue: Queue = world.entities[&owner]
        .require("queue")
        .expect("the stamped queue");

    let admitted = ["cell:2,0", "cell:2,1", "cell:1,1", "cell:0,1"];
    assert_eq!(queue.path.len(), admitted.len());
    for (step, key) in queue.path.iter().zip(admitted) {
        let CellAddress::Derived(raw) = step else {
            panic!("a local handle survived expansion");
        };
        let resolved = EntityId::from(raw.parse().expect("a well-formed address"));
        assert_eq!(
            resolved,
            at(&owner, key),
            "the chained address and the direct one name different cells"
        );
        let cell = world
            .entities
            .get(&resolved)
            .expect("a queue step landing on an entity that exists");
        assert!(cell.has("cell"), "a queue step must land on a cell");
    }
}

/// Every derived entity is named by the address that reaches it, and that
/// address starts at a declared entity this world holds.
///
/// The claim used to be a different one: while a derived id was a UUIDv5 hashed
/// down the chain, the id and the chain were two facts that could disagree, and
/// this test recomputed the hash to catch that. They are one fact now — the
/// address is the identity — so what is left to check is that the chain lands
/// somewhere real: a generator nobody declared would leave a whole subtree
/// hanging off nothing.
#[test]
fn every_derived_entity_is_named_by_an_address_rooted_in_a_declared_generator() {
    let (_, world) = load();
    let mut derived = 0;
    for entity in world.entities.values() {
        let Some(address) = entity.id.as_derived() else {
            continue;
        };
        derived += 1;
        assert_eq!(
            entity.id.to_string(),
            format!("{}/{}", address.root(), address.key()),
            "a derived identity spells its own address"
        );
        let root = EntityId::from(address.root());
        assert!(
            world.declared.contains_key(&root),
            "{} hangs off {root}, which no layer declared",
            entity.id
        );
        assert!(!root.is_derived(), "a chain starts at a declared entity");
    }
    assert_eq!(derived, world.derived_count());
}

/// The legacy lattice folds nothing here, and the warning that used to say
/// otherwise is the receipt for what v0.6 changed.
///
/// Through v0.5 this slice folded twenty-two cells into nine squares of the
/// declared legacy lattice, every one of them a relay's own cell landing beside
/// the aisle cell it was stitched to. A relay does not mint a cell any more; it
/// binds the floor cell an HTM was already standing on, so there is one cell
/// where there were two and the lattice has nothing left to confuse. SPEC §4
/// says the access cell IS the floor cell, and this is that sentence measured.
#[test]
fn the_legacy_lattice_folds_nothing() {
    let (_, world) = load();
    assert!(
        !world
            .warnings
            .iter()
            .any(|warning| warning.to_string().contains("squares that no stitch joins")),
        "{:?}",
        world.warnings
    );

    // The cohort that exercises the served-by-an-access rule is still there and
    // still hard-assigned; it simply does not read as a mistake.
    let cohort = &world.entities[&id(BUFFER_VTM_COHORT)];
    assert_eq!(cohort.components["cohort"]["assignment"]["kind"], "hard");
}

/// This slice's boards sit 690 mm apart and its aisle is tiled at 900, so some
/// tiles take two boards. SPEC §5 expects a cell under each relay stack, so each
/// of those is a warning naming both relays — sixteen of them, and the count is
/// arithmetic rather than observation.
///
/// Row A's twenty-four boards land on aisle row 0, and six of its eighteen
/// columns take two apiece: one pair each. Row B's twenty-four land on row 1 the
/// same way, five columns taking two — and `cell:16,1` taking three, because the
/// suppressed sprinkler tile at `cell:17,1` sends the two boards that wanted it
/// one column west onto a tile that already had one, which is three pairs rather
/// than one. The order buffer's eight boards land on a six-tile lane, two
/// columns taking two apiece. 6 + 8 + 2 = 16.
///
/// The geometry is invented — this slice is illustrative, not surveyed — so
/// these warnings are the format saying what it is for, not a defect in the
/// example. The cencora conversion is where a real site's floor was cut to put a
/// cell under every board, and it warns not at all.
#[test]
fn boards_closer_together_than_the_aisle_tiles_them_warn_in_pairs() {
    let (_, world) = load();
    let sideways: Vec<String> = world
        .warnings
        .iter()
        .map(|warning| warning.to_string())
        .filter(|warning| warning.contains("out of step"))
        .collect();
    assert_eq!(sideways.len(), 16, "{sideways:?}");
    assert_eq!(world.warnings.len(), 16, "{:?}", world.warnings);

    // The three-relay tile is the suppressed sprinkler tile's doing, and it is
    // named by all three of its pairs.
    let crowded = sideways
        .iter()
        .filter(|warning| warning.contains("cdda8cce-008d-4c20-b5b7-4d5a816e05c8/cell:16,1"))
        .count();
    assert_eq!(crowded, 3, "{sideways:?}");
}

/// The round trip, which is also the determinism check.
///
/// Emit the expanded form, read it back as a document, derive it again — the
/// cache verification runs on that second load and has to pass — and emit again.
/// The two canonical projections must be the same bytes. That is a stronger
/// statement than "two loads agree with each other": it says the artifact this
/// crate writes is one a loader can consume and reproduce, which is the property
/// a second implementation in another language will be held to.
#[test]
fn the_expanded_form_round_trips_to_the_same_bytes() {
    let (set, world) = load();
    let first =
        site_model::expanded::to_canonical_string(&site_model::expanded::emit(&set, &world));

    // Through the bytes, not around them: the artifact a consumer actually reads
    // is the canonical projection. A comparison that skipped serialization would
    // pass while the real round trip failed.
    let reparsed: Value = serde_json::from_str(&first).expect("the canonical projection is JSON");
    let reloaded = Document::from_value(Path::new("<expanded>"), &reparsed)
        .expect("the expanded form is a document");
    let reloaded_set = LayerSet {
        scenario: reloaded,
        layers: Vec::new(),
    };
    // The cache in the reloaded document is checked against a fresh derivation
    // here; a disagreement would fail this call, not this assertion.
    let rederived = site_model::pipeline::derive(&reloaded_set)
        .expect("the inline expansion agrees with what the records derive");

    assert_eq!(rederived.entities.len(), world.entities.len());
    assert_eq!(rederived.edges.len(), world.edges.len());
    // Nothing was lost on the way out. This is the canary the round trip has
    // always been: the `expect` pins on the blocked highway cell and the
    // suppressed aisle tile still ride on their records, and the world still
    // loads without a word of complaint.
    assert_eq!(rederived.warnings, world.warnings);
    let second = site_model::expanded::to_canonical_string(&site_model::expanded::emit(
        &reloaded_set,
        &rederived,
    ));
    assert_eq!(first.len(), second.len());
    assert!(first == second, "the expanded form did not round trip");
    assert!(!first.contains("$comment"));

    // No geometry in the artifact has a fractional part, which is the property
    // SPEC §5 buys cross-implementation equality with: nothing can print `5750`
    // here and parse as `5750.0` there when every length is a count of steps.
    // The two fractions left are a carrier pool's fill fraction, which is a
    // ratio and not a length, and this fixture's `unit_mm` — which is 1, and
    // therefore prints as an integer too.
    let mut fractional = Vec::new();
    fractions(&reparsed, String::new(), &mut fractional);
    assert!(
        fractional
            .iter()
            .all(|path| path.ends_with(".fill_fraction")),
        "a length kept a fractional part: {fractional:?}"
    );
    assert!(first.contains("\"unit_mm\":1"));
}

/// Every place a number with a fractional part sits, by the path to it.
fn fractions(value: &Value, path: String, out: &mut Vec<String>) {
    match value {
        Value::Number(n) if n.as_i64().is_none() && n.as_u64().is_none() => out.push(path),
        Value::Object(map) => {
            for (key, child) in map {
                fractions(child, format!("{path}.{key}"), out);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                fractions(child, format!("{path}[{index}]"), out);
            }
        }
        _ => {}
    }
}

/// A cache that disagrees with the source is refused, and the refusal names the
/// address where they part rather than the document as a whole.
#[test]
fn a_tampered_inline_expansion_is_refused_at_the_address_that_diverges() {
    let (set, world) = load();
    let mut document = site_model::expanded::emit(&set, &world);

    // Block one derived highway cell in the cache and nowhere else. The records
    // still derive it unblocked, so the two now disagree at exactly one node.
    let highway = id(HIGHWAY);
    let record = document["entities"]
        .as_array_mut()
        .expect("records")
        .iter_mut()
        .find(|r| r["id"] == highway.to_string())
        .expect("the highway's declaration");
    let node = record["derived"]
        .as_object_mut()
        .expect("the highway's derived cells")
        .get_mut("cell:1,7")
        .expect("cell:1,7");
    node["components"]["cell"]["blocked"] = serde_json::json!(false);

    let text = site_model::expanded::to_canonical_string(&document);
    let reparsed: Value = serde_json::from_str(&text).expect("it is JSON");
    let reloaded = Document::from_value(Path::new("<tampered>"), &reparsed).expect("it parses");
    let err = site_model::pipeline::derive(&LayerSet {
        scenario: reloaded,
        layers: Vec::new(),
    })
    .unwrap_err();
    match err {
        LoadError::StaleExpansion { address, .. } => {
            assert_eq!(address, format!("{highway}/cell:1,7"));
        }
        other => panic!("expected a stale cache, got {other}"),
    }
}

/// The cache is a cache: take it away and the same world comes back, because the
/// records were the source of truth the whole time.
#[test]
fn a_stripped_inline_expansion_re_derives_identically() {
    let (set, world) = load();
    let full = site_model::expanded::emit(&set, &world);
    let expected = site_model::expanded::to_canonical_string(&full);

    let mut stripped = full;
    for record in stripped["entities"].as_array_mut().expect("records") {
        if let Some(object) = record.as_object_mut() {
            object.remove("derived");
            object.remove("edges");
        }
    }
    let text = site_model::expanded::to_canonical_string(&stripped);
    let reparsed: Value = serde_json::from_str(&text).expect("it is JSON");
    let reloaded = Document::from_value(Path::new("<stripped>"), &reparsed).expect("it parses");
    let reloaded_set = LayerSet {
        scenario: reloaded,
        layers: Vec::new(),
    };
    assert!(reloaded_set.scenario.expansions.is_empty());
    let rederived =
        site_model::pipeline::derive(&reloaded_set).expect("nothing to verify, everything to do");
    assert_eq!(
        site_model::expanded::to_canonical_string(&site_model::expanded::emit(
            &reloaded_set,
            &rederived
        )),
        expected
    );
}

#[test]
fn the_expanded_form_states_every_target_exactly_once() {
    let (set, world) = load();
    let document = site_model::expanded::emit(&set, &world);
    let records = document["entities"].as_array().expect("records");

    // 51 declarations + 4 override targets (two buffer racks, the blocked
    // highway cell, the EasyLoop queue the policy layer deepens) + the one
    // authored suppression. The six exclusion removals are not written back:
    // the carve-out is already a record — it rides on the highway's own tiling
    // — and re-derivation subtracts it again.
    assert_eq!(records.len(), 56);

    let mut targets = BTreeSet::new();
    for record in records {
        let key = match (record.get("id"), record.get("derivation")) {
            (Some(id), None) => format!("{id}"),
            (_, Some(derivation)) => format!("{derivation}"),
            _ => panic!("a record addresses nothing"),
        };
        assert!(targets.insert(key.clone()), "two records address {key}");
    }

    // Every derived entity and every edge is somewhere in the trees. Each level
    // is a map from segment to node, so a segment cannot appear twice.
    fn count(node: &Value, nodes: &mut usize, edges: &mut usize) {
        if let Some(children) = node.get("derived").and_then(|d| d.as_object()) {
            for child in children.values() {
                *nodes += 1;
                count(child, nodes, edges);
            }
        }
        if let Some(list) = node.get("edges").and_then(|e| e.as_array()) {
            *edges += list.len();
        }
    }
    let (mut nodes, mut edges) = (0usize, 0usize);
    for record in records {
        count(record, &mut nodes, &mut edges);
    }
    assert_eq!(nodes, 4319);
    assert_eq!(edges, 364);
}

/// A `vendor` tag on a frame is read, carried, and acted on by nothing.
///
/// SPEC §4 names the tag and says a vendor frame is not a kind of frame — just a
/// frame so tagged — so all three halves matter and they are different claims.
/// The typed view reads it, which is what lets a consumer ask whose install
/// these coordinates belong to. The raw component still carries it, which is
/// what SPEC §9 requires of anything a reader parses. And the frame resolves
/// through the transform chain exactly as it did before, because no step of the
/// pipeline branches on the tag.
#[test]
fn a_vendor_tag_on_a_frame_is_read_and_carried_and_changes_nothing() {
    let (set, world) = load();
    let hai = id("f207d86a-173c-4375-a66d-55607ad63a04");
    let frame: Frame = world.entities[&hai]
        .require("frame")
        .expect("the vendor frame's `frame` component");
    assert_eq!(frame.vendor.as_deref(), Some("hai"));
    assert_eq!(world.entities[&hai].components["frame"]["vendor"], "hai");
    // A frame that is not the primary one says nothing about what a step is
    // worth: `unit_mm` lives in exactly one place (SPEC §4).
    assert_eq!(frame.unit_mm, None);
    // Ignored, not misread: the frame still converts, y-flip and all.
    let p = world
        .frames
        .to_primary(&hai, [Coord(126960 + 2450), Coord(143540 - 5750)])
        .expect("the vendor frame resolves to the primary frame");
    assert_eq!(p, [Steps(2450), Steps(5750)]);

    let document = site_model::expanded::emit(&set, &world);
    let record = document["entities"]
        .as_array()
        .expect("source records")
        .iter()
        .find(|r| r["id"] == hai.to_string())
        .expect("the vendor frame's declaration");
    assert_eq!(record["components"]["frame"]["vendor"], "hai");
}
