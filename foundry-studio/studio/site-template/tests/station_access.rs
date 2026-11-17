//! Template stations must remain connected after the simulator applies queue lane discipline: a laden HTM can
//! reach each presentation from every relay and leave it for every relay, including rotated and mirrored shells.

mod support;

use desgraph::graph::WarehouseGraph;
use desgraph::{CellId, HtmTraversal};
use dessim::worldmap::load_from_bytes;
use rustc_hash::{FxHashMap, FxHashSet};
use site_template::{DocId, KnobDecl, KnobForm, KnobValues, Mint, Prior, evaluate};

fn reachable(graph: &WarehouseGraph, from: CellId, goal: CellId, backwards: bool) -> FxHashSet<CellId> {
    let mut adjacent: FxHashMap<CellId, Vec<CellId>> = FxHashMap::default();
    for (a, b, edge) in graph.edges() {
        if edge.traversability.htm_traversal == HtmTraversal::Both
            && edge.dest_only.is_none_or(|gate| gate.admits(goal))
        {
            let (a, b) = if backwards { (b, a) } else { (a, b) };
            adjacent.entry(a).or_default().push(b);
        }
    }
    let mut seen = FxHashSet::default();
    let mut pending = vec![from];
    while let Some(cell) = pending.pop() {
        if seen.insert(cell) {
            pending.extend(adjacent.get(&cell).into_iter().flatten());
        }
    }
    seen
}

#[test]
fn stations_are_reachable_from_the_relays_and_can_return_to_them() {
    for shell in ["rectangle", "square", "l-shape", "u-shape", "i-shape"] {
        let read = support::read_template(shell);
        let root = read.scopes.iter().find(|scope| scope.scope == read.template).unwrap();
        let orientations = root.entities.iter().find_map(|entity| {
            let knob = KnobDecl::read(entity.id, entity.components.get("template.knob")?).unwrap();
            match knob.form {
                KnobForm::Options(options) if knob.key.as_str() == "orientation" => {
                    Some(options.into_iter().map(|option| option.value.0).collect::<Vec<_>>())
                }
                _ => None,
            }
        });
        let mut cases = vec![KnobValues::new()];
        for orientation in orientations.into_iter().flatten() {
            cases.push(KnobValues::new().with("orientation", orientation));
        }
        if shell == "square" {
            cases.push(KnobValues::new().with("pick_minus_x", 1).with("pick_plus_x", 1));
        }
        for knobs in cases {
            let evaluation = evaluate(&read, &knobs, &Prior::new(DocId::mint()), &mut Mint);
            assert!(!evaluation.has_errors(), "{shell}: {:?}", evaluation.problems);
            let loaded = load_from_bytes(&evaluation.document(&read).to_bytes()).expect("the shell loads");
            let map = loaded.map;
            assert!(!map.relays.is_empty(), "{shell} has relays");
            for (station, goal) in map.stations.iter().flat_map(|s| s.presentation_cells().map(move |goal| (s, goal))) {
                let incoming = reachable(&map.graph, goal, goal, true);
                for relay in &map.relays {
                    assert!(
                        incoming.contains(&relay.htm_access),
                        "{shell} {:?}: relay {:?} cannot reach station {:?}",
                        knobs,
                        relay.id,
                        station.id
                    );
                    let outgoing = reachable(&map.graph, goal, relay.htm_access, false);
                    assert!(
                        outgoing.contains(&relay.htm_access),
                        "{shell} {:?}: station {:?} cannot return to relay {:?}",
                        knobs,
                        station.id,
                        relay.id
                    );
                }
            }
        }
    }
}
