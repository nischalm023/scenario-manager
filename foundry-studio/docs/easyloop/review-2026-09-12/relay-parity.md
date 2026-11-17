# Relay terminal maneuver parity

Updated September 15 after Cencora visual review. The earlier parity allowance
for an 850 mm access-position offset was a geometry defect: HTMs stopped beside
the boards. The committed map now declares and binds 684 physical access cells
beneath the boards, matching legacy Cencora coordinates individually.

Each stand has one destination-gated forward ingress and one reverse egress
along the same segment. These directions and gates now agree between loaders.
The existing travel cells are ordinary floor again, allowing along-lane travel.
The world graph contains 31,740 directed arcs, all with legacy counterparts;
the 3,306 legacy-only arcs belong to the VTM aisle and relay-column graph.

Three attribute differences remain explicit in the comparison: RTP flags on
all common arcs, HTM flags on 74 station-footprint arcs, and VTM-zone labels
on 684 relay ingresses. Those zone labels have no effect because neither map
permits VTM traversal on the affected edges and pathfinders only apply the
labels to VTMs. The comparison checks the exact relay endpoints and zone values.

See [Cencora validation](../../cencora-validation.md) for the correction,
reproducible patch audit and current run configuration.
