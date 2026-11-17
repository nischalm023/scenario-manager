# Foundry Studio

Foundry Studio is where GreyOrange designs and measures a warehouse before it
is built. A site is laid out from a template, its robot fleet and operating
flow are set, a scenario gives it work, and a discrete-event simulation of the
result comes back as a recording that can be watched, compared and exported to
a workbook. This repository holds the product's browser application and API,
the simulator they run, and the services around them. It is being renamed from
`greymatter-foundry` to `foundry-studio`; GitHub redirects the old name.

The code is Rust and TypeScript, with Python for repository scripts. The Java
queue simulator this repository began with is retired and recoverable from git
history.

## Where things are

| Path | What it is |
| --- | --- |
| `dev.sh` | One command for the local stack: a PostgreSQL container, the world-proto server built from a checkout beside this one and seeded, the Studio API with its runners' listener, and the browser application |
| `studio/` | The product folder. `api`: the Studio API service (templates and sites, scenarios and experiments, runs, the journal the chat reads). `ui`: the browser application. The shell templates and catalog items the stack is seeded with live in `data/seed/studio`. `viewer` and `viewer/js`: the floor viewer compiled to WebAssembly and its TypeScript side. `runs`: the Crucible run server that executes submitted simulations and serves their recordings, with its own starter `studio/runs/dev.sh`. Beside them the Studio crates: `protocol` (the types the API and the application share), `wps-client` (the world-proto client), `site-template` (the template evaluation engine), `scenario-spec` (the workload generator), `run-invocation` (the run loops and the invoker) |
| `sim/dessim` | The simulator: the model, the scheduler, the tasks robots perform |
| `sim/` (the rest) | What the simulator stands on: the warehouse site model (`site-model`), the warehouse graph (`desgraph`) and its JSON map format (`desmap`), the pathfinder, simulated time and shared ids (`desunits`), and dessim's three macro crates |
| `worldview/` | The `.worldview` recording format, its floor model and its renderer |
| `runstore/` | Run storage: the objects a run produces, as the services keep and serve them |
| `workbook/` | The workbook export of a run's results |
| `desvis/` | The desktop simulator and viewer: runs a simulation and draws it, live or from a recording |
| `docs/`, `specs/`, `notes/`, `perf/` | Designs, specifications, working notes and performance profiles; some documents still describe the retired Java system |
| `scripts/`, `.githooks/`, `.github/` | Repository tooling: the checkers, the pre-commit gate and CI |

## Getting started

- `./dev.sh start` builds and starts the whole stack and prints its addresses;
  `./dev.sh status`, `./dev.sh --logs [unit]` and `./dev.sh --stop` follow, and
  `./dev.sh --stop --db` also resets the databases. It needs Rust, bun, Docker
  and jq, and a world-proto checkout beside this one (`WORLD_PROTO_DIR`, by
  default `../world-proto`).
- `desvis/README.md`: run a simulation on your desktop, play a recording back,
  compare two runs.
- `studio/runs/README.md`: the Crucible run server on its own.

The test gate is `cargo nextest run` at the root (`cargo install cargo-nextest
--locked` once); `CLAUDE.md` says what it runs and what the nightly slow gate
keeps out of it.

Before your first commit, enable the pre-commit gate once per clone:

```bash
git config core.hooksPath .githooks
```

`CLAUDE.md` holds the conventions this codebase is written to, including the
line-by-line review every pull request goes through.
