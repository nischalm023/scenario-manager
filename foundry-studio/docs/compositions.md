# Task compositions and shared state

A composition selects a simulation's top-level tasks. The JSON-only
[`sim/dessim/compositions`](../sim/dessim/compositions/) folder contains
[`cencora.json`](../sim/dessim/compositions/cencora.json) and
[`walmart.json`](../sim/dessim/compositions/walmart.json). Rust parsing and startup
live under [`src/composition`](../sim/dessim/src/composition/); the hardcoded
`fn cencora` and `ModelState` are removed.

A scenario selects `"composition": "walmart"`. Cencora is the default for existing
scenarios; Walmart fixtures select their own composition explicitly. A conveyor
map with the Cencora preset is refused. Named presets are embedded in the binary;
editing one requires a rebuild. An inline composition can instead live directly
in a scenario:

```json
{
  "composition": {
    "tasks": [
      { "type": "InitializeFleet" },
      { "type": "FillInventory", "after": ["InitializeFleet"] },
      { "type": "LoadOrders", "after": ["FillInventory"] },
      { "type": "FleetConservation", "after": ["FillInventory"] },
      { "type": "FulfillOrders", "after": ["LoadOrders"] }
    ]
  }
}
```

These are roots; parents construct children as work arrives. `after` requires a
root to finish before the dependent root is constructed. Missing dependencies,
cycles, duplicate run-wide roots, and waiting for a perpetual service are errors.
Tasks declare whether they hold simulation completion. The scheduler finishes
when finite work has drained, even when monitoring roots remain alive.

## Roots register; children use ordinary constructors

Import `task` and place `#[task]` on a root struct. The attribute uses the struct's
name as its JSON identity. `linkme` collects static type/factory descriptors;
registration performs no construction, and linker order does not determine
execution order. There is no central list of child types or child invocations.

A JSON-selected root implements the synchronous trait method:

```rust
fn create(ctx: &TaskContext) -> Result<Self, PlanError>
```

The root reads its parameters and resolves specific service handles. Dependencies
in `after` delay this call until setup finishes; no dependencies means immediate
construction. Task execution and reservation waits remain asynchronous.

Children need no attribute or registry entry. They use `new(...)` with whatever
typed arguments they need, and the parent calls `ctx.add_task(child)`. A child
that reads composition defaults can obtain `ctx.task_context::<Child>(config)`,
read its parameters, construct normally, and call `finish()` to reject unread
fields before adding it. There is no mandatory `Spec`, `Config`, or `Input` type.

## Task parameters

Each root accepts optional `config`. A composition can also declare defaults for
root or child task types:

```json
{
  "task_defaults": {
    "PickDemand": { "sequencing": "sequenced", "queue_factor": 3 }
  },
  "tasks": [
    { "type": "InitializeFleet" },
    { "type": "FillInventory", "after": ["InitializeFleet"] },
    { "type": "LoadOrders", "after": ["FillInventory"] },
    { "type": "PickDemand", "after": ["LoadOrders"], "config": { "queue_factor": 4 } }
  ]
}
```

Instance fields override type defaults; omitted parameters inherit existing
scenario settings or the task's built-in default. The example uses sequenced
picking and queue factor 4. Explicit task parameters take precedence over legacy
settings, including settings passed on the command line. Null is not a universal
inheritance marker: each parameter must satisfy its declared type.

| Root | Optional parameters |
| --- | --- |
| `FulfillOrders` | `pick_secs`, `extraction_secs`, `buffer_dwell_secs` |
| `PickDemand` | `embargo_secs`, `sequencing`, `pace_to_station`, `queue_factor`, `max_inflight_picks` |
| `PutawaySupplyDemand` | `direct_bias_secs` |
| `PutawayInduction` | `max_presentations_per_hour` |
| `OrderCloseOut` | `buffer_dwell_secs` |

Registered root configuration is validated up front. Unregistered child defaults
must be objects, but their fields are validated only when that child is
constructed. A misspelled child name that is never constructed cannot currently
be detected globally. Keep child defaults close to the workflow that uses them.

The named presets preserve scenario-specific rates and delays by omitting
instance overrides. Equipment geometry, conveyor capacity and physical timing
remain at the map/settings boundary. Initial fleet, inventory fill and order
loading are finite setup tasks, not manager configuration. `ConfigureTtp` and a
general service-config section are absent.

## Synchronous managers, including late construction

`ctx.state::<Manager>()?` returns an `Arc<Manager>`, constructing it on first use.
Each simulation owns a `States` resolver; its clones share one instance per
manager type. Separate simulations are isolated without a `Scope::Run` argument.
A child spawned later can request a manager no startup task needed.

Factories implement `StateManager::create(&States) -> Result<Self, StateError>`.
They can synchronously resolve dependencies. Recursive dependency cycles report
the construction chain. A failed construction is not cached; after a missing
input is published, a later request can retry. Async initialization, if a future
manager needs it, must start in the background and expose readiness to dependent
operations rather than make construction asynchronous.

`States::configure<C>(value)` publishes immutable construction input once;
`configuration<C>()` reads it immediately or reports a missing input. It does not
wait for a future configuration task. Physical setup and its readiness belong to
scheduled work with explicit completion dependencies.

Factories wire empty services and read declared equipment. They do not register
fleet or inventory as a side effect. Running tasks retain specific typed handles
and pass clones to their children. The runner uses `RunReports` to inspect
already-created managers for reporting and teardown; inspection does not
instantiate services merely to count them.

## Current behavior and validation

Cencora selects its original pick, putaway and return task families. Walmart
selects `FulfillOrders`, which uses actual order totes, inventory conveyor custody,
paired item transfer, retained inventory HTMs, buffering and extraction. The
shared VTM/relay, pickup, navigation and storage tasks serve both workflows.

The Walmart implementation assigns concurrent order workers to distinct waiting
berths. Inventory queue movement and collection overlap picking. Outbound
dispatch runs independently after the final pick; the root awaits all inventory
cleanup and outbound work. Detailed shared main-belt sequencing remains future
work. See the
[implemented task hierarchy](easyloop/tasks.html#L431),
[current scope](easyloop/implementation.md), and
[workload checks](easyloop/workload-checks.md).

Focused checks cover synchronous late service creation, run isolation, cycle
refusal, root dependencies, ordinary typed children, real pickup/return timing,
parameter equivalence and Walmart physical custody. Historical full Cencora
parity remains a separate gate: the existing `zero_gap` picking-fixture deadlock
also reproduces at the preceding service checkpoint. Passing focused regressions
is not a claim that that gate is complete.
