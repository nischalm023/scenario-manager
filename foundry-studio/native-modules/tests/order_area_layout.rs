//! Admission boundaries follow the declared circulation graph, including
//! mirrored stations. External holds and inventory traffic must not consume
//! an order block's six resident/return rights.

use std::path::Path;

use desgraph::HtmTraversal;
use dessim::worldmap;
use rustc_hash::FxHashSet;

#[test]
fn each_presentation_has_an_independent_area_and_external_holds() {
    for (fixture, expected_machines) in [("minimal_easyloop.json", 1), ("walmart_dfw6.json", 4)] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(fixture);
        let world = worldmap::load(&path).unwrap();
        assert_eq!(world.map.conveyor_machines.len(), expected_machines);
        for machine in &world.map.conveyor_machines {
            let mut claimed = FxHashSet::default();
            for presentation in &machine.presentations {
                let area = presentation.order_area.as_ref().expect("one external entry/egress per block");
                let resident: FxHashSet<_> = area.resident_cells.iter().copied().collect();
                assert_eq!(presentation.waiting_cells.len(), 6);
                assert_eq!(resident.len(), 19, "six waits, two six-cell lanes, and presentation");
                assert!(resident.contains(&presentation.cell));
                assert!(presentation.waiting_cells.iter().all(|cell| resident.contains(cell)));
                assert!(!resident.contains(&area.entry));
                assert!(!resident.contains(&area.egress));
                assert!(!resident.contains(&machine.dropoff_cell.unwrap()));
                assert!(!resident.contains(&machine.collection_cell.unwrap()));
                assert!(machine.inventory_queue.as_ref().unwrap().path.iter().all(|cell| !resident.contains(cell)));
                assert!(
                    area.resident_cells.iter().all(|&cell| claimed.insert(cell)),
                    "blocks must not share residents"
                );
                assert!(claimed.insert(area.entry) && claimed.insert(area.egress), "each block owns its door holds");
                for (from, to, edge) in world.map.graph.edges() {
                    if edge.traversability.htm_traversal != HtmTraversal::Both {
                        continue;
                    }
                    if !resident.contains(&from) && resident.contains(&to) {
                        assert_eq!(from, area.entry, "admission cannot be bypassed");
                    }
                    if resident.contains(&from) && !resident.contains(&to) {
                        assert_eq!(to, area.egress, "membership ends only at the dedicated egress");
                    }
                }
            }
        }
    }
}
