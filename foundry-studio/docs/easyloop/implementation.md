# Current EasyLoop task scope

The Walmart composition initializes its fleet and inventory, then loads orders
through `LoadOrders`. `FulfillOrders` creates fixed-presentation order streams
using each EasyLoop's inventory CONWIP ceiling. The old half-fleet worker limit
is removed. The full Walmart map now declares 230 HTMs, per Daniel's September 12
instruction; physical robot and relay acceptance remain separate from logical
work generation.

Streams take orders from one shared pending set. `ClaimNextOrder` makes a
revocable claim no earlier than the prior order's final pick timestamp. Source
arrival gates apply unless the throughput-control scenario releases all work at
T0. Logical assignment ends at the final pick while outbound work continues.

Each EasyLoop has two independent six-member areas, each with one fixed order
presentation. `AreaAdmission` owns temporal membership and a return-berth right;
unfinished orders retain that right while presenting or recirculating. Another
order can enter only after a committed departure creates capacity. Physical
membership ends at the area-exit timestamp, not at the final pick. The first
order route still visits its waiting cell unconditionally; a direct-first route
when sequencing permits remains pending.

Inventory preparation uses `ceil(queue_factor × F_max / pick_period) + 2`:
42/43/42/42 at queue factor 3 for the current full map. Its credit spans retrieval
release through matching pick completion. This is distinct from the physical
eight-tote conveyor capacity, which lasts through collection.

After unloading, `AdvanceQueue` moves the retained empty inventory HTM through
all eight one-way holding cells. That movement overlaps the conveyor tote's
presentation and picking. `TransferItems` requires both named totes to remain
correctly presented through the shared operator's dwell; it moves actual stock
into the order tote at dwell completion.

`SupplyAndCollect` launches `CollectAndStoreTote` immediately after unloading
and queue launch. The retained HTM can collect the next ready return even while
its original inventory tote's order container remains unavailable. It need not
collect the tote it delivered. The conveyance separately enforces at most eight
inventory totes and eight retained HTMs, including presentation and collection-ready
inventory totes.

After the final pick, `DispatchOrder` owns buffering, any extraction deadline,
extraction and returning the empty order tote. A completed order does not make
an unnecessary return to the waiting block. The root awaits every inventory
cleanup and outbound dispatch before declaring the workload complete.
Buffer release and container extraction currently follow the implemented
placeholder policy; [the Walmart requirements](remaining-flows.md) remain open.

This structure permits overlapping orders, inventory deliveries and returns.
It does not yet model detailed shared main-belt stops or lateral-transfer
mechanics, and tote animation along the conveyor remains deferred. The
configured `pick_secs` is the duration of one item-transfer operation; a transfer
may move multiple units of the requested SKU, without multiplying dwell by
quantity. See [workload checks](workload-checks.md) for the measured scope and
validation evidence; the earlier serial timing baseline is retained there for
comparison.

Walmart uses floor-running VTMs with unspecified make/model. The current world
already declares floor locomotion, but the runtime still uses shared VTM motion
behavior. The distinct floor-VTM implementation remains future work; historical
DA50 report labels do not identify the Walmart machine. The current fleet is
230 HTMs plus 40 inventory and eight provisional buffer/depot VTMs. The ongoing
[controlled fleet comparisons](review-2026-09-12/fleet-isolation.md) distinguish
VTM delay, HTM availability and time consumed by loaded holds.
