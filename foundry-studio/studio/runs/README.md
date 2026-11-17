# crucible

One server process that runs simulations, records each one as a `.worldview`,
and serves what it recorded.

A submitted specification becomes a run; the simulator executes it on a worker
thread inside this process, up to a configured number of runs at once. There is
one runner mode, in-process — dispatching runs to separate runner processes is
not part of this phase, and neither are image builds, which is why there is no
Dockerfile here. PostgreSQL is the only external service, and `dev.sh` starts
it in a container.

Design: `docs/designs/crucible-server-ui.html`. Recording format:
`docs/designs/worldview-streaming.html`.

## Running it

```sh
./dev.sh start        # PostgreSQL, the builds, the server and the UI; prints the addresses
./dev.sh start --no-ui   # the server alone, for the terminal client
./dev.sh start --bucket <name>   # the same stack storing into a GCS bucket, with your gcloud credentials
studio/runs/dev.sh submit sim/dessim/tests/fixtures/cencora_v3_spec.json --simtime 10 --threads 1 --wait   # from the repository root
./dev.sh status       # the supervisor, and what each unit is doing
./dev.sh logs         # the server's log; `logs ui`, `logs supervisor`
./dev.sh stop         # the container keeps its rows; `stop --db` removes it
```

Everything is named after this checkout — the container, the three ports, the
storage folder — so two clones run side by side without meeting.

`start` holds the terminal until there is something to open, saying what each
unit is doing while it waits, and then prints the UI's address. What it costs
from a fresh clone, on a 16-core Mac whose cargo package cache, `postgres:17`
image and bun cache are already there:

| Step | Cold | Again |
|---|---|---|
| the PostgreSQL container, up and accepting connections | 2 s | under a second, reused |
| the server, built in release | 66 s | under a second |
| the wasm viewer, built and bound | 25 s | skipped while it is newer than its sources |
| Vite | 1 s | 1 s |
| **all of it** | **95 s** | **2 s** |

The two builds are the whole of it and they vary with what else the machine is
doing — 52 s and 21 s on a quieter run of the same clone. A machine that has
none of the caches yet pays for the image pull and the crate downloads on top.
`stop` and a fresh `start` rebuild nothing.

## It shows up in Sutra

The stack publishes itself under `~/.dev-runner/`, which is where Sutra, the
dashboard of dev runners, looks. It reports four units — `postgres` (the
container), `viewer` (the wasm the page loads), `server` and `ui` — each
saying what it is doing as it starts, builds, becomes ready or falls over.
Sutra's terminate button sends one signal to the supervisor `start` left
running, which is exactly what `./dev.sh stop` sends: the server and Vite go
down together, and this checkout's entry is removed. The container stays up,
holding its rows, as it does after any `stop`.

## What a run leaves behind

| What | Where |
|---|---|
| Which runs exist, and how each try at one ended | PostgreSQL; the migrations are compiled into the server and run at startup |
| The specification a run was submitted with | `runs/<run>/spec.json` in storage |
| The recording a run wrote | `runs/<run>/attempts/<n>/recording.worldview` in storage |
| A recording somebody is reading | the temporary folder, under the same key: fetched out of storage the first time it is asked for, and kept there, bounded by a configured size |

Storage is `runstore`: a folder on disk in development
(`CRUCIBLE_STORAGE_DIR`, which `dev.sh` sets), a Google Cloud Storage bucket in
deployment (`CRUCIBLE_STORAGE_BUCKET`, under the `gcs` feature).

A recording is uploaded off the disk it was written to, in parts, the moment
its attempt ends, and the local file stays as the first copy a browser reads.
A replay finds that copy, or a copy fetched earlier, and serves it without a
word to the bucket when its size is the one the registry wrote down at the
ending; only a run this machine has never held is fetched, once, in ranges,
and published whole by rename.

## Running against a bucket

This server is not deployed any more: Agent Studio replaced it, and the des
chart in gm-multi-tenant-apps deploys `studio-api`, `studio-streaming`, the
shell and the runner Jobs instead. There is no image of it — the build that
made one was dropped from `scripts/build-and-push.sh` with the deployment. What
follows is for running it by hand against a bucket, `cargo run -p crucible
--features gcs`.

The binary names no storage backend of its own. Set `CRUCIBLE_STORAGE_BUCKET`
and nothing else about storage: the server refuses to start with both variables
set, with neither, or with either empty, so nothing can quietly send recordings
somewhere else.

The bucket is reached with whatever identity the host gives the process, and
no credential file. The client's chain, in order: a service-account key named
by `GOOGLE_SERVICE_ACCOUNT`, `GOOGLE_SERVICE_ACCOUNT_PATH` or
`GOOGLE_SERVICE_ACCOUNT_KEY`; application default credentials from
`GOOGLE_APPLICATION_CREDENTIALS` or `~/.config/gcloud/`; and otherwise the
GCE metadata server, which is what a GKE pod with Workload Identity, a Cloud
Run service or a GCE VM exposes. Agent Studio's pods reach the same store
through the same `runstore` crate and take that last step, carrying **no**
`GOOGLE_*` variable and no mounted key. Two cautions: the
client does not read `external_account` credential files, so Workload Identity
has to be the metadata-server binding (`iam.gke.io/gcp-service-account` on the
pod's Kubernetes service account) and not a credential-config file; and a
`~/.config/gcloud/application_default_credentials.json` inside the container
would silently win over the metadata server, so none is mounted.

The identity needs to create, read, list and delete objects in the bucket and
nothing more: `roles/storage.objectUser` on the bucket. Delete is used when a
submission's registry row fails after its spec was stored.

At startup the server probes the store, with one listing under `runs/`, and
refuses to start if the probe fails, naming the bucket and the reason. A wrong
IAM binding is therefore a crash loop with the cause in the pod's log, not a
run that fails to store minutes later. Write rights are proved by the first
upload, whose failure names the key.

Mount a volume at `CRUCIBLE_TEMP_DIR` (`/var/lib/crucible/temp` in the image),
writable by uid 10001, and set `CRUCIBLE_CACHE_BYTES` to at most about 80% of
it so staging files and compiled floors fit beside the cached recordings. A
mounted volume hides the `chown` the image did, so the pod sets
`securityContext.fsGroup: 10001` to make it writable. A persistent volume
keeps the cache across restarts; an `emptyDir` costs one fetch per run after
each restart.

Run one replica: the registry sweeps every `running` attempt to `partial` at
startup, and the cache is per machine.

An upload that is cut off, by a pod being stopped mid-way, leaves parts in the
bucket that no listing names and that are billed until deleted; the server
aborts the uploads it sees fail, but not one it is killed in. Give the bucket
a lifecycle rule that abandons incomplete multipart uploads after a day.

## What it answers

`submit`, `list`, `status` and `cancel`, as JSON control messages over one
websocket at `/ws`; the message types are in `src/protocol.rs` and the
browser's half of them is generated into `studio/viewer/js/src/protocol.ts`
by `cargo test -p crucible --test ts_bindings`.

`attach`, `detach` and `series` serve a recording to a browser: `attach`
answers with the filtered scenario block, the index up to the asked instant,
the keyframe at or before it and every delta after that, as binary worldview
frames; `series` answers with a metric's points, assembled from the recording's
scalars sections. A run can be opened once its attempt has ended — finished,
cancelled, or failed with something recorded. Watching a run WHILE it executes
is not in this branch; `docs/designs/crucible-server-ui.html` section 06 says
what it waits on.

## The recording is headless desvis's recording

A run goes through `dessim::harness` — the same function `desvis --headless`
calls, not a copy of it — and the recording is written by phase 1's writer from
the finished run model. Both callers reach the harness the same way: every
command-line value is an override applied only when somebody passed the flag,
so a specification's own `settings` block stands for both, whatever it states.
For the same specification, the workbook built from this server's stored object
is cell-identical to the one built from desvis's own recording.

## Configuration

| Variable | What | Default |
|---|---|---|
| `BIND` | host and port to answer on | `127.0.0.1:3001` |
| `DATABASE_URL` | the registry's database | required |
| `CRUCIBLE_TEMP_DIR` | where a recording is written before it is stored, and where one is fetched back to be read | the system temporary folder |
| `CRUCIBLE_CACHE_BYTES` | how much of that folder the fetched recordings and compiled floors may take | 2 GB |
| `CRUCIBLE_ATTACH_LAG_SECONDS` · `CRUCIBLE_SERIES_POINTS` | how long a frame waits for a browser's socket before its attachment is dropped; the most points one metric's series comes back as | 10 s · 2,000 |
| `CRUCIBLE_RUNS_AT_ONCE` | simulations at once; more queue | available cores − 1 |
| `CRUCIBLE_MAX_MESSAGE_BYTES` | largest control message accepted | 32 MiB |
| `CRUCIBLE_STORAGE_DIR` / `CRUCIBLE_STORAGE_BUCKET` | where finished recordings go | one of the two is required |

## Tests

```sh
cargo test -p crucible                                    # the registry tests skip without a database
DATABASE_URL=postgres://... cargo test -p crucible        # all of them
```

The registry tests create a schema of their own inside the named database and
drop it afterwards, so they run in parallel against the container `dev.sh`
starts.

The bucket backend is tested in `runstore` against a Google Cloud Storage
emulator, which `dev.sh` starts in a container and names, or against a real
bucket with your own credentials (`runstore/README.md` says how, and
which two tests the emulator cannot run):

```sh
./dev.sh gcs-emulator            # starts the emulator; prints the variables to export
cargo test -p runstore --features gcs
./dev.sh gcs-emulator stop
```

The browser walk drives the UI headlessly against a stack that is up — submit,
watch the progress count up, open the run, play, scrub, seek, reload, every
overlay and view, a second floor, the DFW6 site with its frame times, a
cancelled run's recording, and a spec picked off the machine — asserting each
step before photographing it:

```sh
./dev.sh start                                 # prints the UI's address
node ../../scripts/worldview/ui_gate.mjs --ui <ui-address> --shots <results-dir>
```

The terminal client has a gate of its own, run against a server that is up:

```sh
./dev.sh start --no-ui                         # prints the websocket address
../../scripts/worldview/cli_gate.sh <server-ws-url> <results-dir>
```

It submits a fixture, watches it to its end, lists, asks for a status and for a
metric's series, attaches twice and cancels a second longer run, writing every
command and its output to `<results-dir>/cli-transcript.txt`. It fails if any
command fails or prints the greeting the server opens a socket with. Nothing
else in the gates runs this client, so without it a protocol change the browser
absorbs breaks the client unseen.
