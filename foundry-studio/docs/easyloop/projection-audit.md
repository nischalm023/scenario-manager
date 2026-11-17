# EasyLoop projection boundary

September 8, 2026: read-only design audit. The changes below are proposed, not
implemented by the throughput/reporting pass.

`worldmap::machine::Machine` is a private loader intermediate selected specifically
by `go.easyloop`. It groups ports, waiting locations and order presentation cells
while translating the world document into simulation map data. It is neither a
generic world entity nor a runtime state manager. Its name is broader than its
actual predicate and purpose.

Worldlens currently returns predicate bindings from variable names to entity IDs.
The consumer still reads component values, checks cardinalities, resolves graph
IDs and builds its Rust projection. That boundary is reasonable; the present
conversion path is unnecessarily indirect:

```text
Worldlens bindings → Machine → synthetic Stations → ConveyorMachine
```

The loader reconstructs the conveyor layout from compatibility stations. Transit
time and accepted container designation remain on those stations rather than on
the resulting layout; the fulfillment task looks transit time back up there.
The runtime layout also drops the source equipment entity identity.

A more explicit boundary would construct validated layouts directly:

- A private `EasyLoopProjection` adapter recognizes today's `go.easyloop`
  components and preserves source identity for diagnostics.
- `ConveyorLayout` carries dropoff/collection ports, transit time and tote
  capacity. The conveyor's eight-tote capacity and the retained HTM queue's
  capacity are separate equipment facts.
- `ConveyorPickingLayout` relates that conveyor to the retained inventory-HTM
  queue, shared order waiting places and explicit order presentation endpoints.
- Existing `Station` consumers receive compatibility records derived from the
  validated layout, without a reverse reconstruction step.

The adapter belongs in `worldmap/projections/easyloop.rs`, consumer layouts in
`desmap`, and live custody/admission in the existing conveyance services. Generic
movement, retrieval, storage and item-transfer tasks remain reusable. A generic
conveyor should not be forced to have two operator presentation locations.

Two recognition details also need attention during that change: constrain the
pick-place pattern to `presentation_point.for == "order_carrier"`, and use a
consistent anchor predicate for all member patterns. Required ports and topology
should produce specific projection errors, rather than failing later with a
generic missing-location error.

This is a consumer-projection cleanup, not approval to start the parked world
format migration. It should have map parity checks and preserve current physical
behavior before replacing the existing path.
