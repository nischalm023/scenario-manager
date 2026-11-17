# Butler terminology for the EasyLoop task proposal

[Open the parent–child diagrams](tasks.html#terminology).

Compared against Butler `develop` at **`7064a99964960a7e46b7928d7d4a008e6b2159ce`**, after the
September 7 fast-forward pull from `1a00ce2cda`. The pull added 1,309 commits;
the relevant definitions and workflows were reread afterward. Source links below
are pinned to that revision, rather than a moving branch. The local source
checkout is `/Users/daniel/vt/greyorange/butler_server`.

The diagrams describe a proposed simulator composition. Butler’s persistent
transport records and robot subtask lists are useful vocabulary and design
evidence; they are not identical to task-model-v2 parent/child schedules.

## Vocabulary and scope

| Proposed simulator term | Butler evidence | What is shared, and what differs |
|---|---|---|
| **Tote**, **order tote**, **inventory tote** | [carrier_location_info: carrier types and locations](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/pick/src/models/carrier_location_info.erl#L76); [tote_task: child-task types and destinations](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/fleet/task/models/tote_task.erl#L71) | Butler really does use `carrier`, including `tote` and `packing_box`. The concrete MHS task vocabulary is tote-based. Use “tote” in these current-flow diagrams and keep the robot distinct. This naming does not prescribe tote dimensions or EasyLoop-specific behavior. |
| **TransportTote** | [tote_task: child-task types and destinations](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/fleet/task/models/tote_task.erl#L71); [tote_move_task: transport legs](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/fleet/task/models/tote_move_task.erl#L103); [relay_pps_task: relay/PPS task types](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/fleet/task/models/relay_pps_task.erl#L100) | A named tote moves to a destination through equipment capabilities. Butler’s `tote_task` tracks a selected child task and a destination type; `tote_move_task` and `relay_pps_task` describe more specific legs. The proposal combines the former `PositionCarrier` and `TransportCarrier` names into one destination-oriented family. Its robot detail diagram ends with the tote aboard; add unloading only when the destination requires a handoff. |
| **Navigate** | [relay_pps_subtasks: navigation to the PPS destination](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/fleet/butler/relay_pps_subtasks.erl#L4989); [relay_pps_subtasks: goto_barcode creation](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/fleet/butler/relay_pps_subtasks.erl#L6075); [bot_transport_task: assignment and destination model](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/fleet/task/models/bot_transport_task.erl#L59) | Corresponds to robot movement subtasks such as `goto_barcode`, with context-specific routing such as `goto_pps_destination`. `bot_transport_task` also owns assignment and external-trigger waiting, so it would be misleading to use that name for a single route primitive. “Navigate” avoids requiring barcode-based coordinates in the simulator. |
| **LoadTote / UnloadTote** | [rm_tote_load_unload_subtasks: load/unload dispatch](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/fleet/butler/rm_tote_load_unload_subtasks.erl#L26); [relay_pps_subtasks: HTM load/unload dispatch](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/fleet/butler/relay_pps_subtasks.erl#L103) | Reuses the verbs in `load_tote`, `unload_tote`, `htm_load_tote`, and `htm_unload_tote`. Direction is from the robot’s perspective. Loading puts the tote on the robot; unloading removes it. These labels replace ambiguous whole-tote “transfer” nodes. Presenting an order tote does neither. |
| **ConveyorMove** | [conveyor_move_task: internal and notified movement types](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/fleet/task/models/conveyor_move_task.erl#L77) | Reuses the `conveyor_move_task` intent vocabulary: `entry_to_pick`, `pick_to_pick`, `pick_to_exit`, etc. Butler distinguishes internal task types from externally notified types. Its request is not a simulation of belt motion; our shared conveyor service still needs occupancy, starts/stops, side transfers, and effects on all riders. |
| **PresentTotes** | [waiting_for_pick_start_node: source arrival and pick prerequisites](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/pick/src/flows/plan_transport/waiting_for_pick_start_node.erl#L19); [waiting_for_dest_agent_node: EasyLoop destination branch](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/pick/src/flows/plan_transport/waiting_for_dest_agent_node.erl#L47); [wait_for_easy_loop_dest_node: carrier-at-active-bin readiness](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/pick/src/flows/plan_transport/wait_for_easy_loop_dest_node.erl#L73) | New simulator parent coordinating the inventory and order tote presentations. Butler expresses source arrival and destination readiness in its pick workflow. No identical “position the pair” task was established. This parent schedules placement work; it must not be mistaken for a passive `wait_for_*` state. |
| **TransferItems** | [wait_for_pick_confirm_node: operator confirmation workflow](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/pick/src/flows/hsm/entity_transport/wait_for_pick_confirm_node.erl#L56) | Retain the generic simulator primitive for timed item movement. Butler’s pick-confirmation workflow also validates operator input and updates picking state. It is broader than this primitive, so renaming the primitive to a UI wait or confirmation node would misrepresent it. |
| **MoveToWaitingBlock / AdvanceQueue** | [block: waiting-block locations](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/layout/models/block.erl#L299); [block: sequenced queues](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/layout/models/block.erl#L407); [relay_pps_subtasks: waiting_for_sequence](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/fleet/butler/relay_pps_subtasks.erl#L6879) | Reuse `waiting_block`, `sequenced_queue`, and `waiting_for_sequence`. The proposed tasks compose movement and signal-driven waits. Butler’s queue-manager/state-machine boundaries do not force one v2 task per state or cell. The order waiting block and empty-HTM under-conveyor queue remain different resources. |
| **SupplyAndCollectTotes** | [relay_pps_subtasks: post-conveyor-drop continuation](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/fleet/butler/relay_pps_subtasks.erl#L6057); [relay_pps_subtasks: completion and possible robot unassignment](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/fleet/butler/relay_pps_subtasks.erl#L6093) | New simulator parent retaining one HTM through delivery and subsequent collection. Butler has post-drop continuation and follow-on robot assignment, but can also complete the delivery task and leave a robot unassigned. It is not an equivalent custody guarantee. Daniel’s no-early-exit ruling remains binding here. |

## Names that remain simulator compositions

`FulfillOrder`, `PickOrder`, `AcquireTote`, `PickupTote`, `RetrieveTote`,
`SupplyTote`, `CollectTote`, `SupplyAndCollectTotes`, `DispatchTote`,
`PresentTotes`, `ClearPresentations`, `MoveToWaitingBlock`, and `AdvanceQueue`
are proposed descriptive names, not claims that Butler defines those classes.
`ReturnToStorage` describes the return composition. Existing simulator storage
tasks `VtmForward` and `VtmReturn` retain their names; no verbatim Butler API
mapping is asserted for them.

`PickItem` remains the proposed parent of presentation, item transfer, and
clearance. Sharing a name with the current simulator task does not preserve its
old behavior of recording a pick at inventory dropoff.

## What alignment does not decide

- It does not adopt Butler’s complete task tree, database representation, GUI
  workflow, or external-controller protocol as the simulator architecture.
- It does not change Daniel’s physical brief: both totes present for picking,
  order totes stay aboard HTMs, shared waiting reaches either side, one-way
  empty-HTM continuation, eight total inventory totes, and no early exit.
- It does not make names or APIs approved implementation contracts. The HTML
  remains a design for review, with task-model-v2 scheduling underneath.
- It does not implement box induction. Butler’s `packing_box` terminology is
  available when that source is implemented; the current diagrams use totes.

The pull also added a `conveyor_to_pps` relay task variant for a Split 2:4
station and an `at_presentation_point` state. These are useful evidence that
transport intents and presentation readiness vary by configuration. Their
station layout is not adopted for Daniel’s EasyLoop. See [relay_pps_task: relay/PPS task types](https://github.com/greyorange/butler_server/blob/7064a99964960a7e46b7928d7d4a008e6b2159ce/apps/mhs/src/fleet/task/models/relay_pps_task.erl#L100).
