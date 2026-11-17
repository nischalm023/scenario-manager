//! Within `worldlens`'s tests, `census` is the SECOND COUNT: every number the
//! pattern tests assert, worked out again by hand over the same worlds without
//! going through the engine.
//!
//! A test that asserts what the engine printed proves only that the engine is
//! stable. So each pattern of `data/seed/application-tests/lenses/legacy-wps/world.lens` is written out here a second
//! time as a plain scan — the way the simulator's loader writes its predicates
//! today — and `patterns.rs` asserts the identical table through the engine. Two
//! independent walks over one document agreeing on a number is what makes the
//! number evidence; when one of them changes, the disagreement says which.
//!
//! The lenses are counted here a second time as well, as the count of distinct
//! entities each one's anchor would bind.
//!
//! Nothing here uses `worldlens`. That is the point.

mod fixtures;

use fixtures::{documents, Fixture};
use serde_json::{from_value, Value};
use std::collections::BTreeMap;
use site_model::components::product::CellAddress;
use site_model::components::spatial::Pose;
use site_model::pipeline::World;
use site_model::{Entity, EntityId};

/// The count each pattern of `data/seed/application-tests/lenses/legacy-wps/world.lens` comes to on each document,
/// worked out by the scans below.
fn counted(world: &World) -> BTreeMap<&'static str, usize> {
    let mut out = BTreeMap::new();
    let all = || world.entities.values();
    let has = |e: &Entity, name: &str| e.has(name);

    // --- the frame ------------------------------------------------------
    out.insert(
        "primary_frame_scale",
        all().filter(|e| e.raw("frame").is_some_and(|f| states(f.get("unit_mm")) && !states(f.get("parent")))).count(),
    );

    // --- cells ---------------------------------------------------------
    out.insert(
        "document_cell",
        all().filter(|e| has(e, "cell") && e.raw("pose").is_some_and(|p| p.get("frame").is_some())).count(),
    );
    out.insert("charging_cell", all().filter(|e| has(e, "cell") && has(e, "charging")).count());
    out.insert("presentation_marked_cell", all().filter(|e| has(e, "cell") && has(e, "presentation_point")).count());
    out.insert("exchange_cell", all().filter(|e| has(e, "cell") && has(e, "port")).count());
    out.insert("highway_cell", all().filter(|e| has(e, "cell") && has(e, "highway")).count());
    out.insert("flow_highway_cell", {
        let flowed: Vec<&Entity> = all().filter(|e| has(e, "flow") && !e.is_derived()).collect();
        all()
            .filter(|c| has(c, "cell") && c.is_derived())
            .map(|c| flowed.iter().filter(|g| g.id.root() == c.id.root()).count())
            .sum()
    });
    out.insert(
        "open_cell",
        all()
            .filter(|e| {
                has(e, "cell")
                    && !has(e, "charging")
                    && !has(e, "presentation_point")
                    && !has(e, "port")
                    && !has(e, "highway")
            })
            .count(),
    );

    // --- grids and stamps ----------------------------------------------
    out.insert("cell_grid", all().filter(|e| has(e, "region") && has(e, "tiling")).count());
    out.insert("derived_cell_of", {
        all()
            .filter(|c| has(c, "cell") && c.is_derived())
            .filter(|c| c.parent.as_ref().and_then(|p| world.entities.get(p)).is_some_and(|g| has(g, "tiling")))
            .count()
    });
    out.insert("waiting_pool", all().filter(|e| has(e, "waiting") && has(e, "region") && has(e, "tiling")).count());
    out.insert("stamp_instance", {
        all()
            .filter(|i| i.is_derived())
            .filter(|i| i.parent.as_ref().and_then(|p| world.entities.get(p)).is_some_and(|s| has(s, "stamp")))
            .count()
    });
    let easyloop_pairs: Vec<(EntityId, EntityId)> = all()
        .filter(|m| has(m, "stamp") && has(m, "pose"))
        .flat_map(|m| {
            all()
                .filter(|i| has(i, "easyloop"))
                .filter(|i| ancestors(world, &i.id).contains(&m.id))
                .map(|i| (m.id.clone(), i.id.clone()))
                .collect::<Vec<_>>()
        })
        .collect();
    out.insert("easyloop_internal_of", easyloop_pairs.len());
    out.insert("easyloop_machine", {
        let mut machines: Vec<&EntityId> = easyloop_pairs.iter().map(|(m, _)| m).collect();
        machines.sort();
        machines.dedup();
        machines.len()
    });

    out.insert("stitch_pair", {
        let known = |id: &EntityId| world.entities.contains_key(id);
        all()
            .filter(|s| has(s, "stitch"))
            .map(|s| {
                names(s, "stitch", &["a"]).iter().filter(|id| known(id)).count()
                    * names(s, "stitch", &["b"]).iter().filter(|id| known(id)).count()
            })
            .sum()
    });
    // The machine authored cell by cell: one entity carrying its own frame, the
    // envelope and the belt, and membership by that frame. Written out the way
    // a loader would: collect the machines, then ask each posed cell whether
    // its frame chain reaches one.
    let machines: Vec<&EntityId> = all()
        .filter(|e| {
            has(e, "frame") && has(e, "region") && e.raw("easyloop").is_some_and(|g| states(g.get("loop_transit_secs")))
        })
        .map(|e| &e.id)
        .collect();
    let in_a_machine = |e: &Entity| {
        let Some(frame) = e.component::<Pose>("pose").ok().flatten().and_then(|p| p.frame) else {
            return false;
        };
        let mut cursor = Some(frame);
        while let Some(current) = cursor {
            if machines.contains(&&current) {
                return true;
            }
            cursor = world
                .entities
                .get(&current)
                .and_then(|e| e.raw("frame"))
                .and_then(|f| f.get("parent").cloned())
                .and_then(|v| from_value::<EntityId>(v).ok());
        }
        false
    };
    out.insert("machine", machines.len());
    out.insert("machine_cell", all().filter(|e| has(e, "cell") && in_a_machine(e)).count());
    out.insert(
        "machine_pick_place",
        all().filter(|e| has(e, "cell") && has(e, "pick") && has(e, "presentation_point") && in_a_machine(e)).count(),
    );
    out.insert("machine_port", all().filter(|e| has(e, "cell") && has(e, "port") && in_a_machine(e)).count());
    // The ports again, asked for by the pose that puts them in the machine's
    // frame instead of by the `cell` that makes them stand-on-able. Where this
    // count runs ahead of the one above, the difference is the ports a document
    // gave the machine and never made cells -- the ones `machine_port` drops.
    out.insert("machine_port_posed", all().filter(|e| has(e, "port") && has(e, "pose") && in_a_machine(e)).count());
    out.insert(
        "machine_waiting_cell",
        all().filter(|e| has(e, "cell") && has(e, "waiting") && in_a_machine(e)).count(),
    );

    // A lane: a queue on an entity that is neither a station nor a generator of
    // the floor it runs over.
    let is_lane = |e: &Entity| has(e, "queue") && !has(e, "station") && !has(e, "tiling");
    out.insert("lane", all().filter(|e| is_lane(e)).count());
    out.insert("lane_cell", {
        all()
            .filter(|e| is_lane(e))
            .map(|q| {
                names(q, "queue", &["path"])
                    .iter()
                    .filter(|id| world.entities.get(id).is_some_and(|c| has(c, "cell")))
                    .count()
            })
            .sum()
    });

    out.insert(
        "easyloop_part",
        all()
            .filter(|e| {
                e.raw("easyloop").is_some_and(|g| states(g.get("part"))) && has(e, "region") && !has(e, "tiling")
            })
            .count(),
    );
    out.insert("building_envelope", all().filter(|e| has(e, "building") && has(e, "region")).count());

    // --- stations -------------------------------------------------------
    out.insert("placed_station", all().filter(|e| has(e, "station") && has(e, "pose")).count());
    out.insert(
        "pick_station",
        all()
            .filter(|e| {
                has(e, "pose") && e.raw("station").and_then(|s| s.get("kind")).and_then(|k| k.as_str()) == Some("pick")
            })
            .count(),
    );
    out.insert(
        "machine_station",
        all()
            .filter(|e| {
                has(e, "station")
                    && has(e, "pose")
                    && e.raw("easyloop").is_some_and(|g| g.get("loop_transit_secs").is_some())
            })
            .count(),
    );
    out.insert("presentation_cell", {
        all()
            .filter(|c| has(c, "presentation_point"))
            .map(|c| {
                names(c, "presentation_point", &["station"])
                    .iter()
                    .filter(|id| world.entities.get(id).is_some_and(|s| has(s, "station")))
                    .count()
            })
            .sum()
    });
    // The same reference read from the station's side. The two scans part only
    // in which end the pattern anchors at, so they come to the same number.
    let a_station = |id: &EntityId| world.entities.get(id).is_some_and(|s| has(s, "station"));
    out.insert("station_presentation_cell", {
        all()
            .filter(|c| has(c, "presentation_point"))
            .map(|c| names(c, "presentation_point", &["station"]).iter().filter(|id| a_station(id)).count())
            .sum()
    });
    out.insert("order_presentation", {
        all()
            .filter(|c| {
                c.raw("presentation_point").and_then(|p| p.get("for")).and_then(|v| v.as_str()) == Some("order_carrier")
            })
            .map(|c| names(c, "presentation_point", &["station"]).iter().filter(|id| a_station(id)).count())
            .sum()
    });
    out.insert(
        "station_exit",
        all().filter(|e| e.raw("station").is_some_and(|s| states(s.get("exit_direction")))).count(),
    );
    out.insert("station_bins", all().filter(|e| e.raw("station").is_some_and(|s| states(s.get("bins")))).count());
    out.insert("dropoff_cell", port_pairs(world, "sink", "into"));
    out.insert("collection_cell", port_pairs(world, "source", "from"));
    out.insert("belt_designation", {
        let stations: Vec<&Entity> = all().filter(|e| has(e, "station")).collect();
        all()
            .filter(|p| p.raw("port").and_then(|v| v.get("direction")).and_then(|v| v.as_str()) == Some("sink"))
            .filter(|p| {
                p.raw("port")
                    .and_then(|v| v.get("of"))
                    .and_then(|v| v.get(0))
                    .is_some_and(|first| states(first.get("designation")))
            })
            .map(|p| stations.iter().filter(|s| s.id.root() == p.id.root()).count())
            .sum()
    });
    out.insert("port_of_machine", {
        let stations: Vec<&Entity> = all().filter(|e| has(e, "station")).collect();
        all().filter(|p| has(p, "port")).map(|p| stations.iter().filter(|s| s.id.root() == p.id.root()).count()).sum()
    });
    out.insert("station_with_queue", all().filter(|e| has(e, "station") && has(e, "queue")).count());
    out.insert("queue_cell", {
        all()
            .filter(|s| has(s, "station") && has(s, "queue"))
            .map(|s| {
                names(s, "queue", &["path"])
                    .iter()
                    .filter(|id| world.entities.get(id).is_some_and(|c| has(c, "cell")))
                    .count()
            })
            .sum()
    });
    out.insert("queue_into_station", {
        all()
            .filter(|q| has(q, "queue"))
            .map(|q| {
                names(q, "queue", &["into"])
                    .iter()
                    .filter(|id| world.entities.get(id).is_some_and(|s| has(s, "station")))
                    .count()
            })
            .sum()
    });

    // --- steel ----------------------------------------------------------
    out.insert("run_access", {
        all()
            .filter(|a| has(a, "access"))
            .map(|a| {
                let grids = names(a, "access", &["along"])
                    .iter()
                    .filter(|id| world.entities.get(id).is_some_and(|g| has(g, "region")))
                    .count();
                let runs = names(a, "access", &["rack"])
                    .iter()
                    .filter(|id| world.entities.get(id).is_some_and(|r| has(r, "stamp") && has(r, "pose")))
                    .count();
                grids * runs
            })
            .sum()
    });
    out.insert("rack_bay", {
        let runs: Vec<&EntityId> = all().filter(|r| has(r, "stamp")).map(|r| &r.id).collect();
        all()
            .filter(|b| has(b, "rack") && has(b, "pose"))
            .map(|b| {
                let chain = ancestors(world, &b.id);
                runs.iter().filter(|r| chain.contains(**r)).count()
            })
            .sum()
    });
    out.insert("storage_position", {
        all()
            .filter(|s| has(s, "slot"))
            .map(|s| {
                names(s, "slot", &["rack"])
                    .iter()
                    .filter(|id| world.entities.get(id).is_some_and(|b| has(b, "rack")))
                    .count()
            })
            .sum()
    });
    out.insert(
        "order_bay",
        all()
            .filter(|e| {
                has(e, "rack")
                    && e.raw("storage").and_then(|s| s.get("holds")).and_then(|v| v.as_str()) == Some("order")
            })
            .count(),
    );
    out.insert("rack_bay_with_relays", {
        all()
            .filter(|r| has(r, "relay"))
            .map(|r| {
                let bays = names(r, "relay", &["rack"])
                    .iter()
                    .filter(|id| world.entities.get(id).is_some_and(|b| has(b, "rack")))
                    .count();
                let cells = names(r, "access_cell", &["cell"])
                    .iter()
                    .filter(|id| world.entities.get(id).is_some_and(|c| has(c, "cell")))
                    .count();
                bays * cells
            })
            .sum()
    });
    out.insert("unbound_relay", all().filter(|e| has(e, "relay") && !has(e, "access_cell")).count());
    out.insert("cohort_assignment", {
        all()
            .filter(|c| has(c, "cohort"))
            .map(|c| {
                names(c, "cohort", &["assignment", "to"])
                    .iter()
                    .filter(|id| world.entities.get(id).is_some_and(|s| has(s, "sector")))
                    .count()
            })
            .sum()
    });
    out.insert("floor_cohort", {
        all()
            .filter(|c| has(c, "cohort"))
            .map(|c| {
                names(c, "cohort", &["robot_class"])
                    .iter()
                    .filter(|id| {
                        world.entities.get(id).is_some_and(|k| {
                            matches!(
                                k.raw("robot_class").and_then(|v| v.get("role")).and_then(|v| v.as_str()),
                                Some("htm") | Some("rtp")
                            )
                        })
                    })
                    .count()
            })
            .sum()
    });
    out.insert("vtm_cohort", {
        let mut total = 0usize;
        for cohort in all().filter(|c| {
            c.raw("cohort").and_then(|v| v.get("assignment")).and_then(|v| v.get("kind")).and_then(|v| v.as_str())
                == Some("hard")
        }) {
            let classes = names(cohort, "cohort", &["robot_class"])
                .iter()
                .filter(|id| {
                    world.entities.get(id).is_some_and(|k| {
                        k.raw("robot_class").and_then(|v| v.get("role")).and_then(|v| v.as_str()) == Some("vtm")
                    })
                })
                .count();
            if classes == 0 {
                continue;
            }
            for sector in names(cohort, "cohort", &["assignment", "to"])
                .iter()
                .filter_map(|id| world.entities.get(id))
                .filter(|t| has(t, "sector"))
            {
                for run in names(sector, "sector", &["members"])
                    .iter()
                    .filter_map(|id| world.entities.get(id))
                    .filter(|r| has(r, "stamp") && has(r, "pose"))
                {
                    for access in all().filter(|a| has(a, "access")) {
                        if !names(access, "access", &["rack"]).contains(&run.id) {
                            continue;
                        }
                        let aisles = names(access, "access", &["along"])
                            .iter()
                            .filter(|id| world.entities.get(id).is_some_and(|g| has(g, "region")))
                            .count();
                        total += classes * aisles;
                    }
                }
            }
        }
        total
    });
    out.insert("sector_group", all().filter(|e| e.raw("sector").is_some_and(|s| states(s.get("members")))).count());
    out.insert(
        "carrier_instance",
        all()
            .filter(|e| {
                matches!(
                    e.raw("carrier").and_then(|v| v.get("designation")).and_then(|v| v.as_str()),
                    Some("inventory") | Some("order")
                )
            })
            .count(),
    );
    out.insert("sector_member", {
        all()
            .filter(|s| has(s, "sector"))
            .map(|s| {
                names(s, "sector", &["members"])
                    .iter()
                    .filter(|id| world.entities.get(id).is_some_and(|m| m.has("stamp")))
                    .count()
            })
            .sum()
    });
    out
}

/// The number of objects each lens of `data/seed/application-tests/lenses/legacy-wps/world.lens` projects on one
/// document, worked out by hand: an object is one distinct entity the lens's
/// anchor variable binds, so each scan below counts the entities that satisfy
/// the anchor pattern, once each.
///
/// The fields hung off an object are not counted here. What a field holds is
/// the anchor pattern's rows regrouped, and the rows are already pinned by the
/// table above; what a lens adds on top of them is the grouping, which
/// `tests/lenses.rs` pins on the document small enough to name every number.
fn lens_objects(world: &World) -> BTreeMap<&'static str, usize> {
    let mut out = BTreeMap::new();
    let all = || world.entities.values();
    let has = |e: &Entity, name: &str| e.has(name);

    out.insert(
        "easyloop_station",
        all()
            .filter(|e| {
                has(e, "frame")
                    && has(e, "region")
                    && e.raw("easyloop").is_some_and(|g| states(g.get("loop_transit_secs")))
            })
            .count(),
    );
    out.insert("lane", all().filter(|e| has(e, "queue") && !has(e, "station") && !has(e, "tiling")).count());
    out.insert("rack_bay", {
        let runs: Vec<&EntityId> = all().filter(|r| has(r, "stamp")).map(|r| &r.id).collect();
        all()
            .filter(|b| has(b, "rack") && has(b, "pose"))
            .filter(|b| {
                let chain = ancestors(world, &b.id);
                runs.iter().any(|r| chain.contains(r))
            })
            .count()
    });
    out.insert("placed_station", all().filter(|e| has(e, "station") && has(e, "pose")).count());
    out.insert(
        "pick_station",
        all()
            .filter(|e| {
                has(e, "pose") && e.raw("station").and_then(|s| s.get("kind")).and_then(|k| k.as_str()) == Some("pick")
            })
            .count(),
    );
    out.insert(
        "floor_cohort",
        all()
            .filter(|c| has(c, "cohort"))
            .filter(|c| {
                names(c, "cohort", &["robot_class"]).iter().any(|id| {
                    world.entities.get(id).is_some_and(|k| {
                        matches!(
                            k.raw("robot_class").and_then(|v| v.get("role")).and_then(|v| v.as_str()),
                            Some("htm") | Some("rtp")
                        )
                    })
                })
            })
            .count(),
    );
    out.insert("vtm_cohort", {
        all()
            .filter(|c| {
                c.raw("cohort").and_then(|v| v.get("assignment")).and_then(|v| v.get("kind")).and_then(|v| v.as_str())
                    == Some("hard")
            })
            .filter(|cohort| {
                let climbs = names(cohort, "cohort", &["robot_class"]).iter().any(|id| {
                    world.entities.get(id).is_some_and(|k| {
                        k.raw("robot_class").and_then(|v| v.get("role")).and_then(|v| v.as_str()) == Some("vtm")
                    })
                });
                if !climbs {
                    return false;
                }
                names(cohort, "cohort", &["assignment", "to"])
                    .iter()
                    .filter_map(|id| world.entities.get(id))
                    .filter(|t| has(t, "sector"))
                    .any(|sector| {
                        names(sector, "sector", &["members"])
                            .iter()
                            .filter_map(|id| world.entities.get(id))
                            .filter(|r| has(r, "stamp") && has(r, "pose"))
                            .any(|run| {
                                all().filter(|a| has(a, "access")).any(|access| {
                                    names(access, "access", &["rack"]).contains(&run.id)
                                        && names(access, "access", &["along"])
                                            .iter()
                                            .any(|id| world.entities.get(id).is_some_and(|g| has(g, "region")))
                                })
                            })
                    })
            })
            .count()
    });
    out
}

/// Whether the document states something at this field: present, and not one of
/// the four ways of writing nothing. This is the `exists` rule, written out again
/// here rather than borrowed from the engine.
fn states(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
        Some(_) => true,
    }
}

/// Every identity the value at `component.path` names, in the spellings SPEC §3
/// gives — written out again here rather than borrowed from the engine.
///
/// A value the reader has no address form for reaches nothing rather than
/// raising: an `[x, y]` position is geometry and no address at all, and an array
/// inside an array is not flattened into one. Of the forms the reader does have,
/// a template-local handle names no identity, because expansion rewrites it and
/// one that survives into a derived world stands for nothing that world holds.
fn names(entity: &Entity, component: &str, path: &[&str]) -> Vec<EntityId> {
    let mut cursor = match entity.raw(component) {
        Some(value) => value,
        None => return Vec::new(),
    };
    for segment in path {
        cursor = match cursor.get(segment) {
            Some(value) => value,
            None => return Vec::new(),
        };
    }
    let items: Vec<&Value> = match cursor {
        Value::Array(list) => list.iter().collect(),
        other => vec![other],
    };
    items
        .into_iter()
        .filter_map(|value| match from_value::<CellAddress>(value.clone()).ok()? {
            CellAddress::Id(id) => Some(id),
            CellAddress::Derived(raw) => raw.parse().ok().map(EntityId::from),
            CellAddress::Local(_) | CellAddress::Handle(_) => None,
            // A position names where a cell stands, not which entity it is.
            CellAddress::At(_) => None,
        })
        .collect()
}

fn ancestors(world: &World, id: &EntityId) -> Vec<EntityId> {
    let mut out = Vec::new();
    let mut cursor = world.entities.get(id).and_then(|e| e.parent.clone());
    while let Some(current) = cursor {
        if out.contains(&current) {
            break;
        }
        cursor = world.entities.get(&current).and_then(|e| e.parent.clone());
        out.push(current);
    }
    out
}

fn port_pairs(world: &World, direction: &str, field: &str) -> usize {
    world
        .entities
        .values()
        .filter(|p| p.has("cell"))
        .filter(|p| p.raw("port").and_then(|v| v.get("direction")).and_then(|v| v.as_str()) == Some(direction))
        .map(|p| {
            names(p, "port", &[field])
                .iter()
                .filter(|id| world.entities.get(id).is_some_and(|s| s.has("station")))
                .count()
        })
        .sum()
}

/// One row per pattern of `data/seed/application-tests/lenses/legacy-wps/world.lens`: the count it comes to on
/// cencora-v3, walmart-dallas, minimal-putaway2 and the minimal EasyLoop map,
/// in that order. Every number was worked out twice — once by the scans in
/// `census.rs` and once by the engine in `patterns.rs` — and the two must agree.
///
/// Six rows read zero on all four, and those are the six whose whole claim IS
/// the zero: nobody writes `queue.into`, no station carries `queue` (the two
/// station-queue rows: a station's queue is a lane of its own), every relay of every document stands on
/// a cell, none of the four declares charging equipment, and no document says
/// which station an order carrier's head serves — walmart-dallas and the minimal
/// EasyLoop map each state two such heads and neither names a station on one.
/// Nine more read zero on the three reader fixtures and
/// answer only on the minimal EasyLoop map — `machine` and the five that read
/// what is posed in its frame, which want a machine authored cell by cell
/// rather than stamped from a template; and `building`, `highway` and a
/// `easyloop` naming its `part`, three components the other three documents
/// never write. `lane` and `lane_cell` answer wherever a station has a lane.
pub const EXPECTED: [(&str, [usize; 4]); 52] = [
    ("belt_designation", [0, 3, 0, 0]),
    ("building_envelope", [0, 0, 0, 1]),
    ("carrier_instance", [0, 75, 0, 10]),
    ("cell_grid", [146, 9, 1, 8]),
    ("cohort_assignment", [19, 1, 1, 2]),
    ("collection_cell", [0, 1, 0, 0]),
    ("derived_cell_of", [8089, 178, 132, 223]),
    ("document_cell", [8773, 184, 132, 251]),
    ("dropoff_cell", [0, 1, 0, 0]),
    ("easyloop_internal_of", [0, 1, 0, 0]),
    ("easyloop_machine", [0, 1, 0, 0]),
    ("easyloop_part", [0, 0, 0, 1]),
    ("exchange_cell", [0, 5, 0, 2]),
    ("floor_cohort", [1, 1, 1, 1]),
    ("flow_highway_cell", [0, 131, 0, 0]),
    ("highway_cell", [0, 0, 0, 32]),
    ("lane", [26, 0, 2, 6]),
    ("lane_cell", [136, 0, 4, 23]),
    ("machine", [0, 0, 0, 1]),
    ("machine_cell", [0, 0, 0, 28]),
    ("machine_pick_place", [0, 0, 0, 2]),
    ("machine_port", [0, 0, 0, 2]),
    ("machine_port_posed", [0, 0, 0, 2]),
    ("machine_station", [0, 1, 0, 0]),
    ("machine_waiting_cell", [0, 0, 0, 12]),
    ("open_cell", [8747, 179, 130, 215]),
    ("order_bay", [0, 2, 0, 5]),
    ("order_presentation", [0, 0, 0, 0]),
    ("charging_cell", [0, 0, 0, 0]),
    ("pick_station", [10, 1, 1, 0]),
    ("placed_station", [26, 4, 2, 3]),
    ("port_of_machine", [0, 5, 0, 0]),
    ("presentation_cell", [26, 0, 2, 3]),
    ("presentation_marked_cell", [26, 0, 2, 2]),
    ("primary_frame_scale", [1, 1, 1, 1]),
    ("queue_cell", [0, 0, 0, 0]),
    ("queue_into_station", [0, 0, 0, 0]),
    ("rack_bay", [228, 20, 6, 11]),
    ("rack_bay_with_relays", [1368, 56, 6, 11]),
    ("run_access", [38, 4, 1, 3]),
    ("sector_group", [19, 1, 1, 2]),
    ("sector_member", [38, 1, 1, 3]),
    ("stamp_instance", [372, 24, 6, 11]),
    ("station_bins", [0, 0, 2, 1]),
    ("station_exit", [26, 0, 2, 3]),
    ("station_presentation_cell", [26, 0, 2, 3]),
    ("station_with_queue", [0, 0, 0, 0]),
    ("stitch_pair", [420, 10, 0, 16]),
    ("storage_position", [10032, 3940, 60, 55]),
    ("unbound_relay", [0, 0, 0, 0]),
    ("vtm_cohort", [38, 1, 1, 3]),
    ("waiting_pool", [0, 1, 0, 0]),
];

/// The scans above come to the table, on all four documents.
#[test]
fn the_hand_count_matches_the_table() {
    let mut mismatches = Vec::new();
    for (column, Fixture { name, world, .. }) in documents().into_iter().enumerate() {
        let counted = counted(&world);
        for (pattern, wanted) in EXPECTED {
            let found = counted.get(pattern).copied();
            if found != Some(wanted[column]) {
                mismatches.push(format!("{name}: {pattern}: actual {found:?}, expected {}", wanted[column]));
            }
        }
        assert_eq!(counted.len(), EXPECTED.len(), "{name}: the table covers every scan");
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// One row per lens of `data/seed/application-tests/lenses/legacy-wps/world.lens`: the number of objects it projects
/// on cencora-v3, walmart-dallas, minimal-putaway2 and the minimal EasyLoop map.
/// Worked out twice, like the pattern table above.
///
/// Six of the seven names are a pattern's name as well, which is the point of
/// the two keywords: `library.lens("rack_bay")` and `library.get("rack_bay")`
/// answer two different questions about the same subject.
///
/// The EasyLoop rows are where the four documents part company.
/// `easyloop_station` answers only on the minimal map, whose machine is
/// authored cell by cell; `lane` answers wherever a station or a machine
/// states one; and `pick_station` answers everywhere but there,
/// because that map's three stations induct, put away and extract, and none of
/// them is a pick station.
pub const LENSES: [(&str, [usize; 4]); 7] = [
    ("easyloop_station", [0, 0, 0, 1]),
    ("floor_cohort", [1, 1, 1, 1]),
    ("lane", [26, 0, 2, 6]),
    ("pick_station", [10, 1, 1, 0]),
    ("placed_station", [26, 4, 2, 3]),
    ("rack_bay", [228, 20, 6, 11]),
    ("vtm_cohort", [19, 1, 1, 2]),
];

/// The anchor scans above come to the lens table, on all four documents.
#[test]
fn the_hand_count_matches_the_lens_table() {
    for (column, Fixture { name, world, .. }) in documents().into_iter().enumerate() {
        let counted = lens_objects(&world);
        for (lens, wanted) in LENSES {
            assert_eq!(counted.get(lens).copied(), Some(wanted[column]), "{name}: {lens} counted by hand");
        }
        assert_eq!(counted.len(), LENSES.len(), "{name}: the table covers every anchor scan");
    }
}
