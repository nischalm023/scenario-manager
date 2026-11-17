# studio-api

The Agent Studio API service: the one door through which a person in the browser
and the agent act on Agent Studio's data (spec 01, the API service and the agent
loop). This build answers `GET /healthz` and `GET /api/me`, reads a person's
identity from the login gate's headers, keeps its tables in Postgres with
embedded migrations, refuses every request as `application/problem+json`, and
holds the idempotency layer the write routes of later tasks take.

It keeps a site's conversations and their journal (spec 01 sections 3.5, 3.6 and
3.17): `POST /api/conversations` opens a site's live conversation or the
person's new-site thread, `GET /api/conversations?site=` lists a site's,
`GET /api/conversations/{c}` answers the snapshot with its projection,
`GET /api/conversations/{c}/entries` pages of entries, and
`GET /api/conversations/{c}/stream` the server-sent events;
`POST /api/conversations/{c}/dismiss` archives or discards one. With
`STUDIO_ENV=local` it makes the default project, Demo, at start, and mounts the
test writer, `POST /api/dev/conversations/{c}/entries`, which takes an entry's
`{ kind, body, turn?, caused_by?, dedup_key? }` or a live item's
`{ live: { kind, body } }`; the journal fixtures in
`studio/protocol/tests/journal/` replay through it.

It listens twice: the gate-facing listener, which only the login gate reaches,
and the runners' listener, which reads no identity and serves only the runners'
door (spec 01 section 3.16): `/runner/v1/` routes, each opened by one attempt's
token.

It runs simulation runs (spec 08): `POST /api/runs/plan` prepares and checks a
submission plan from an experiment, read with `GET /api/runs/plan/{p}`;
`POST /api/runs` executes a ready plan, the press being its approval;
`GET /api/runs`, `GET /api/runs/{r}`, `GET /api/runs/queue` and
`GET /api/runs/{r}/manifest` read them; `POST /api/runs/{r}/cancel` and
`POST /api/submissions/{s}/cancel` cancel without asking; `GET
/api/runner-images` names the runner image. Its run loops (`studio/run-invocation`)
hold at most `STUDIO_RUNS_AT_ONCE` seats. Every attempt runs the runner image
`STUDIO_RUNNER_IMAGE` (`studio/Dockerfile`'s `simrunner` target): with
`STUDIO_ENV=local` as a container of this machine's engine (docker or podman),
its output folder mounted from `STUDIO_RUNNER_OUT_DIR`; with `cluster` as a
Kubernetes Job (`studio/kube-invoker`).

It reads and writes the world graph only through `studio/wps-client` (spec 04).
At start it probes the world-proto-server `STUDIO_WPS_URL` names, writing
nothing, and loads the client's schema cache; a server it cannot reach, or one
it cannot use, refuses the start. While it serves, the schema cache is
revalidated every 60 s (wall clock). `GET /api/sites?customer=` lists every
`customer/*/site/*` root that has landed a changeset, named by its prologue,
newest change first, with its live conversation, and `POST /api/conversations`
answers `not-found` for a site the list leaves out.
`GET /api/scope-events?scope=&scope=` relays up to 16 scopes' changesets as
server-sent `scope.moved` events, with a `ping` every 15 s. Computations such as
the flagged export's check run on the compute pool.

## Running it

```sh
STUDIO_ENV=local \
STUDIO_DATABASE_URL=postgres://postgres@127.0.0.1:5432/studio \
STUDIO_DEV_IDENTITY='dev-1,dev@example.com,Dev One' \
STUDIO_MODEL_ID=claude-sonnet-5-5 STUDIO_MODEL_LABEL='Claude Sonnet 5.5' \
STUDIO_VERTEX_PROJECT=my-project STUDIO_VERTEX_REGION=global \
STUDIO_RUNNER_IMAGE=studio-simrunner:dev-<key> \
cargo run -p studio-api
```

The service brings the database up to its tables before it serves. `STUDIO_RUNNER_IMAGE` names a runner image the
container engine must hold built from the source this checkout holds; `./dev.sh start` builds it, as
`studio-simrunner:dev-` and the checkout's key, and sets the rest.

## Configuration

Read once from the environment at startup. A value that does not parse, or a
combination spec 01 forbids, refuses the start; a variable set but blank is a
mistake, never an absence. The table names variables, never secret values.
Spec 01 section 3.1 lists more (the turn budgets, the registrar and service
subjects, the runners' in-cluster address); each arrives with the code that
reads it.

| Variable | What it holds | Default |
|---|---|---|
| `STUDIO_BIND` | Address and port the gate-facing listener answers on | `127.0.0.1:3100` |
| `STUDIO_RUNNER_BIND` | Address and port the runners' listener answers on; the deployment opens it to runner pods only | `127.0.0.1:3101` |
| `STUDIO_DATABASE_URL` | The Postgres database of the des chart | required |
| `STUDIO_ENV` | `local` or `cluster`; decides which identity source and model transport may be used | required |
| `STUDIO_IDENTITY_HEADERS` | The four header names the gate forwards, in order: stable subject, email, display name, Keycloak roles | `x-auth-request-user,x-auth-request-email,x-auth-request-preferred-username,x-auth-request-groups` (unconfirmed; the gate is Nithin's) |
| `STUDIO_DEVELOPER_ROLE` | The Keycloak role that makes a person a developer | `studio-developer` |
| `STUDIO_DEVELOPERS` | Subjects joined by `,` treated as developers, for local work | empty; set with `cluster` refuses the start |
| `STUDIO_DEV_IDENTITY` | `subject,email,name` taken for every request, for local work without a gate | unset; set with `cluster` refuses the start |
| `STUDIO_WPS_URL` | Base address of world-proto-server | `http://127.0.0.1:7413` (WPS's own default) with `local`; required with `cluster` |
| `STUDIO_WPS_ROUTES` | How WPS spells scope routes: `path` (`/scopes/<path>/-/<operation>`), the one shape world-proto-server serves since c384e59; any other value refuses the start; spec 04 section 3.12 | `path` |
| `STUDIO_WPS_REVISION` | The world-proto commit the server was built from, which dev.sh sets from the checkout it builds; the start-up probe refuses a server whose `features` disagree with what that revision is known to have landed, and without `features` takes the revision's (spec 04 section 3.12) | unset: the probe alone decides |
| `STUDIO_MODEL_TRANSPORT` | `vertex`, or `anthropic` (local only; `cluster` refuses it) | `vertex` |
| `STUDIO_MODEL_ID` | The model id as the transport names it | required |
| `STUDIO_MODEL_LABEL` | The model's name as the chat header shows it | required |
| `STUDIO_VERTEX_PROJECT`, `STUDIO_VERTEX_REGION` | GCP project and location of the Claude endpoint | required with `vertex` |
| `ANTHROPIC_API_KEY` | The local development key, read only by the `anthropic` transport | required with `anthropic` |
| `STUDIO_MODEL_EFFORT` | `low`, `medium`, `high`, `xhigh` or `max` | `medium` |
| `STUDIO_MODEL_THINKING_DISPLAY` | `omitted` asks the model for no thought summaries; `updates` asks Gemini for them, which reach only the transcript and `model_calls.raw`, never the chat (D117) | `omitted` |
| `STUDIO_MODEL_MAX_TOKENS` | Output ceiling of one model request, above 0 | `32000` |
| `STUDIO_COMPUTE_WORKERS` | Threads of the compute pool, apart from those serving routes, streams and turns; at least 1 (spec 01 section 3.15) | `2` |
| `STUDIO_COMPUTE_QUEUE` | Computations that may wait for a pool thread before the next is refused, asked to come back in 2 s (wall clock); 0 or more | `64` |
| `STUDIO_BUILD_COMMIT` | The commit the API was built from, which every floor key names beside the floor layout (spec 03 section 3.6); an image sets it, having no checkout to describe | unset: `git describe --always --dirty` of the checkout the API starts in, else `unversioned-{start ms}` |
| `CRUCIBLE_STORAGE_DIR`, `CRUCIBLE_STORAGE_BUCKET` | The run-object store (`runstore`), exactly one of them: a folder that exists, or a bucket; template evaluations keep their plans and floors there under `template-drafts/` and `template-thumbnails/` (spec 05 section 3.7), and each run attempt its job spec, manifest and recording under `runs/<run>/attempts/<n>/` (spec 08 section 3.5), the recording sent by its runner to the API, which stores it; dev.sh sets the folder under `studio/.dev-state/objects`; the local invoker mounts each attempt's job spec from the folder | required |
| `STUDIO_RUNS_AT_ONCE` | Seats held at most across the deployment: attempts whose runner may run (spec 08 section 3.4); at least 1 | `5` |
| `STUDIO_RUN_DEADLINE_SECS` | A runner's wall-clock deadline, after which it is killed and its run fails; at least 1 | `21600` |
| `STUDIO_START_TIMEOUT_SECS` | Wall-clock seconds a dispatched run may wait for its runner before it fails | `900` |
| `STUDIO_CANCEL_GRACE_SECS` | Wall-clock seconds a running run's cancel may go unconfirmed before it ends `cancelled`, keeping any recording its runner sent before then | `60` |
| `STUDIO_STREAMING_STALE_SECS` | Wall-clock seconds, by the database's clock, after its last heartbeat that a streaming server (`studio/streaming`, the `streaming_servers` table) still counts as alive: the dispatcher picks one alive server at random, writes it into the job spec and onto the attempt's row, and launches the attempt unstreamed when none is; a watch ticket is refused `not-streamed` once its server is not alive | `10` |
| `STUDIO_RUNNER_CPU_MILLIS`, `STUDIO_RUNNER_MEMORY_MIB` | A runner's resources, which both invokers apply and whose whole cores they hand the runner as `STUDIO_RUNNER_THREADS`: a Job as its requests and limits, equal; a container as `--cpus` and `--memory` | `1000`; unset until measured |
| `STUDIO_RUNNER_URL` | The runners' listener as a runner reaches it, written into every job spec | `http://` and `STUDIO_RUNNER_BIND` with `local`; required with `cluster` |
| `STUDIO_RUNNER_OUT_DIR` | With `local`, the folder each attempt's container gets an empty folder of its own in, mounted at the image's output path, where its recording stays; set with `cluster`, refuses the start | `studio-runner-out` in the system's temporary folder; dev.sh sets `studio/.dev-state/runner-out` |
| `STUDIO_RUNNER_IMAGE` | The runner image every attempt runs, the one image plans name, whose closed values are taken to be this build's: with `local` the tag dev.sh builds per checkout (`studio-simrunner:dev-<key>`), which the engine must hold built from the source this checkout holds, its commit and any uncommitted changes as `scripts/source-identity.sh` names them and the image's `agent-studio/source` label keeps, else the start is refused, plans naming its id and the commit and branch built into it; with `cluster` a full reference with its digest as `scripts/build-and-push.sh` prints it, from the build that made the API's image, whose commit and branch it names | required |
| `STUDIO_RUNNER_WPS_URL` | world-proto-server as a runner reaches it, written into every attempt's pins: from a container the host's alias, `http://host.docker.internal:<port>`, which dev.sh sets | `STUDIO_WPS_URL` |
| `STUDIO_CONTAINER_ENGINE` | With `local`, the engine's executable that runs the runner's containers; set with `cluster`, refuses the start | `docker`, else `podman`, on `PATH` |
| `STUDIO_CONTAINER_USER` | With `local`, the `uid:gid` every runner container runs as, where the image's own user could not write the mounted output folder (Linux); dev.sh sets it there; set with `cluster`, refuses the start | the image's user |
| `STUDIO_CONTAINER_STACK` | With `local`, the stack this API belongs to, which every runner container it starts carries as its `agent-studio/stack` label; it adopts only containers of its own stack, and dev.sh, which sets its checkout's key, stops only them; set with `cluster`, refuses the start | unset: containers carry no stack |
| `STUDIO_RUNNER_NAMESPACE` | With `cluster`, the namespace the Jobs and their Secrets are created in; the API's service account needs `studio/deploy/runner-rbac.yaml`'s Role there | the API pod's own namespace |
| `STUDIO_RUNNER_SERVICE_ACCOUNT` | With `cluster`, the service account every runner pod runs as; it needs no Kubernetes permission | the namespace's default |
| `STUDIO_RUNNER_PASSED` | Names joined by `,` of runner settings copied from the API's own environment into every runner, container or pod: `DES_HEARTBEAT`, `DES_DIGEST` and `DES_DIGEST_SORTED` (dessim's progress line and event digests) and `STUDIO_RUNNER_HOLD_MS` (a hold after the manifest, for tests), the only variables a runner reads that its image and its invoker do not set. Any other name, or one the environment does not set, refuses the start, so no database URL or store credential can reach a runner (ruling D174) | unset |
| `STUDIO_BUILD_BRANCH` | With `cluster`, the branch the images were built from, which the runner image's record names; the API's image sets it | unset |
| `DESVIS_RUN_BRANCH`, `DESVIS_RUN_COMMIT`, `DESVIS_RUN_DIRTY` | The code identity dessim's harness records when the scenario check prepares a run inside the API, which asks git without them; the API's and the runner's images carry their own, baked in at their build, since neither stands in a checkout | unset: git's |

The model's settings are read and checked at startup; no turn calls the model yet.

### Runs in a cluster

With `STUDIO_ENV=cluster` every approved attempt is one Kubernetes Job (`studio/kube-invoker`) running
`STUDIO_RUNNER_IMAGE` in `STUDIO_RUNNER_NAMESPACE`, its job spec and token in a Secret of the Job's name that the Job
owns, `backoffLimit` 0, `activeDeadlineSeconds` the run deadline, and 90 s between its SIGTERM and its SIGKILL, in
which the runner stops the simulation, keeps its recording and reports `failed` by the platform. The runner delivers
its results over the door (`PUT /runner/v1/attempts/{run}/{n}/results`), beats every 5 s (`.../heartbeat`), and reads
its cancel off `.../control`; the API never reads a pod's disk. Its recording goes to the API first
(`.../recording`), which stores it in the run store, so no runner holds a credential for the store. When the job spec
names a streaming server (`streaming`, picked by the dispatcher from `streaming_servers`), the runner also streams its
progress to it live (`studio/streaming`); a server that does not answer leaves the run alone. The API's service
account needs `studio/deploy/runner-rbac.yaml`'s Role in that namespace. A runner pod needs no permission and no
Secret but its own Job's, which holds its token and job spec: its environment is that token, its thread count and the
runner settings `STUDIO_RUNNER_PASSED` names, each set by name, and nothing else. Watch the runs with
`GET /api/runs/stats`, `GET /api/runs/queue` and each run's `heartbeat_at`, `platform_status` and `stalled`, all read
off the attempts' rows.

The API takes each recording onto its own disk, in its temporary folder (`TMPDIR`), before it stores it: one at a
time per upload in flight, each up to 8 GiB, a 16-hour run of the DFW6 site recording 669 MB. Give the API pod an
`emptyDir` at that folder with an ephemeral-storage request of `STUDIO_RUNS_AT_ONCE` times the largest recording its
runs make, and a limit 8 GiB above that, so one recording at the cap fits beside the rest. An upload the disk cannot
take is answered 500, which the runner asks again within its four minutes.

Locally the same image runs as a container per attempt (`studio/api/src/runs/container.rs`), named by the Job,
labelled `agent-studio=runner` and with its stack (`STUDIO_CONTAINER_STACK`), its job spec mounted read-only from the
run-object store's folder and its output folder from `STUDIO_RUNNER_OUT_DIR`, so `docker ps` shows the runs and the
folder keeps what each left, the recording included, with what its runner printed beside it in `<job>.log`. The
containers outlive the API: a restarted API adopts its stack's ones still holding seats, at the deadlines they
started with, and `./dev.sh --stop --runners` removes that stack's.

## The streaming server

`studio/streaming` (`studio-streaming`, the Dockerfile's `streaming` target) is a process beside the API (ruling
D174): a run's messages as they happen, received from the runner on an internal websocket port and served to viewers
on an external one. It registers itself in `streaming_servers` (migration 0022, the API's, so the API starts first)
and heartbeats there; the dispatcher picks a live one per attempt. A viewer asks the API for a ticket,
`POST /api/runs/{run}/watch`, which answers the server's address for the run's latest attempt and a token the server
admits for a minute; the server reads the API's tables only through migration 0022's views of who may open a socket.
Several servers run at once; one that dies mid-run loses its streams. `studio/streaming/README.md` has the wire
format, the bounds on what a server holds, and its configuration (`STUDIO_DATABASE_URL`, the two binds, and how a
runner and a viewer reach it, which a bind on every interface must state).

Since live frames (26 October 2026) the same server also carries the run's picture: the frames of the run's worldview
recording as the runner sends them, each segment uploaded to the run-object store as it closes, kept on the server's
disk and served to the browser on `/frames/{run}/{attempt}` beside `/streams`, with the same ticket (`frames_url` on the
ticket). The runner's final recording still comes to the API's door, and once `attempts.object_key` names it a replay
uses it. The frames add to the server's configuration: `STUDIO_STREAMING_CACHE_DIR` (required; dev.sh sets
`studio/.dev-state/streaming-cache`), `STUDIO_STREAMING_CACHE_TTL_SECS`, `STUDIO_STREAMING_CACHE_MAX_BYTES`,
`STUDIO_STREAMING_CACHE_SWEEP_SECS`, `STUDIO_STREAMING_OPEN_MAX_BYTES`, and the run-object store's own
`CRUCIBLE_STORAGE_DIR` or `CRUCIBLE_STORAGE_BUCKET` as the API reads them (the bucket needs `studio-streaming/gcs`,
which the image builds with); migration 0023 adds its columns to `attempt_streams`.

## Testing it

```sh
cargo test -p studio-api
```

The tests that need Postgres take the database `STUDIO_TEST_DATABASE_URL`
names, each in a schema of its own that it creates, migrates and drops. With
the variable unset they say they are skipped and pass; a database it names but
cannot reach fails them. A throwaway one:

```sh
docker run -d --rm --name studio-test-pg -e POSTGRES_HOST_AUTH_METHOD=trust -p 127.0.0.1:55432:5432 postgres:16
STUDIO_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:55432/postgres cargo test -p studio-api
docker stop studio-test-pg
```

`tests/runs_container*.rs` (a run's life, its stops, an API restart and its
manifests) run real runners as containers of the runner image against a live
world-proto-server: `STUDIO_TEST_WPS_URL` (a dev.sh stack's,
whose seed holds the shell templates), `STUDIO_TEST_RUNNER_IMAGE` (the image
dev.sh builds, `studio-simrunner:dev-<key>`) and an engine that answers, docker or
podman on `PATH`; the fake runners are images the tests build of a shell
script over `STUDIO_TEST_SHELL_IMAGE` (`busybox:1.36`), each attempt running
the image its plan names, as a real one does. Any of these missing, those
tests say they are skipped. A container reaches the test's listener through
`host.docker.internal`.
