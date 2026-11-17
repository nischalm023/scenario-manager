# studio-streaming

The streaming server (ruling D174): a run's messages as they happen, received from its runner on an internal websocket
port and served to viewers on an external one, so a run can be watched while it runs and its runner touches no
database. It registers itself in the API's database and heartbeats there, so the API's dispatcher can point each new
attempt at a live one, and it writes each run's top-level figures as they come. Several run at once, for availability:
the dispatcher picks one at random.

Since live frames (26 October 2026) it also carries the run's picture: the frames of the run's worldview recording as
the runner sends them, grouped into segments, held in memory and on its own disk, uploaded to the run-object store
(`runstore`) as each segment closes, and served to a viewer from memory, from disk or from the store, in that order. The messages stay in memory
only: a server that stops loses its streams, which ruling D174 accepts for now; the frames it had locked are on its
disk and in the store. A runner that loses its connection reconnects to the same server, which then takes its stream
up again; one that restarted takes it up from that point on.

Two clocks throughout: **simtime** (`_sim_ms`: the slices' instants, the keyframe cadence) and **wall clock** (`_at`:
the heartbeat, the grace, a cached copy's age). A **frame** is the worldview container's unit
(`worldview/worldview/src/container/frame.rs`): a 20-byte header and a payload, self-delimiting, the same bytes on a
socket as in a file. A **segment** is one keyframe and every delta up to the next keyframe, thirty slices at the
cadence the scenario block states.

## Wire format, v2

The messages are typed in `studio/protocol/src/run_stream.rs`, one type per direction, and read only through it, so
both ends refuse a message that does not read or that states another format than 2. Every message is one websocket
**text** frame holding a JSON object with `"v": 2` and a `"type"`, or, after `started` on a runner's socket, one
**binary** frame holding one bare container frame (ruling D184; the rule below). Values travel by name, never as
fields of their own: a viewer discovers what a run reports. A socket either end closes on a refusal carries it in the
close frame, one of `StreamRefusal`'s reasons; `token-invalid`, `attempt-ended`, `unsupported-version` and `superseded`
are final for a runner, `frame-order` and `frame-too-large` end its frames but not its text, anything else is worth
another try.

### Internal port: runner to server, `ws://<host>/runners`

| type | fields | when |
|---|---|---|
| `hello` | `run`, `attempt`, `token` | first, within 5 s of wall clock; the server admits the attempt's token by the runners' door's own rule (the view `live_runner_tokens`: its Job not known stopped) and answers `welcome {v}` |
| `started` | none | the manifest is accepted and the harness prepared; said again after every reconnect |
| `progress` | `sim_ms`, `values [{metric, label, value}]` | the simtime the run's final slices reach and the counts they state, as its heartbeat reads them, every 5 s of wall clock from `started` on and once more before the ending |
| `ended` | `ending {state, ended_by, stop_reason, stop_detail, sim_ms}` | last, before the runner delivers its recording and report: the ending the attempt left, which the report carries too unless the recording cannot be stored, when the report says the platform failed the attempt |

Binary, after `started`: one message is one bare container frame, in the order a file would hold them. The first is
the scenario block. A keyframe opens a segment and its deltas follow on the same socket with no other segment's
frames between them. The runner sends each frame of the run's recording as dessim writes it, the end block last and
the index never, and a frame is final once written: dessim writes a slice only once its barrier has passed it (PR
#593). The server cannot see a segment's end, so a segment counts as whole, and goes to the upload queue and the
`edge`, when the next keyframe comes, when the socket has sent no frame for 1 s of wall clock, or at the ending; a
segment that grows after that is uploaded again, whole. After a lost connection the runner says `started`, the
scenario block and the open segment from its keyframe again, read back from the recording: a scenario block already
held is passed over, and a keyframe at or before the open segment's own starts that segment over, in memory and on
disk. A `Declare` frame belongs to the segment it is sent with, whether it comes ahead of that segment's keyframe
(stamped with the same instant, as dessim declares a metric a segment's first slice is the first to state) or between
a keyframe and the next delta. The final recording does not travel here: the runner sends it to the API's door
(`PUT /runner/v1/attempts/{run}/{n}/recording`), which stores it under the attempt's key and names it in
`attempts.object_key`.

A binary message before `started` is refused `binary-refused`; one that is not one container frame, or out of the
order above (a frame before the scenario block, a second and different scenario block, a delta or an end block before
any keyframe, an instant running backwards, an index frame, a frame after the end block or after the ending) is
refused `frame-order`; one that
would take the attempt's frames in memory past `STUDIO_STREAMING_OPEN_MAX_BYTES` is refused `frame-too-large`, and so
is every frame of that attempt after it. The runner takes either as the end of its frames: it reconnects, says its
text on, and sends no more frames.

A hello for an attempt whose stream is open takes the stream over: the earlier socket is closed `superseded` and
nothing from it is taken after. The server pings every socket every 10 s and takes one silent for 30 s, pongs included,
for gone; the runner takes a server silent for 30 s for a lost connection too. A runner whose socket goes without
`ended` has 30 s to come back; then its viewers are told `runner-lost`, and the stream takes up again if it does come
back. The runner tries again at most 8 s apart, so an outage the grace covers sees several tries; its queue keeps up
to 1024 messages meanwhile, and a full queue drops the newest, since the simulation never waits on its stream. The
frames do not wait in the queue: the recording's file holds them, and the runner reads on from where it stopped.

### External port: server to viewer, `ws://<host>/streams/{run}/{attempt}`

The viewer's first message is `watch {v, token}`, within 5 s, the token a ticket the API minted for that attempt
(`POST /api/runs/{run}/watch`, which also answers the address); the server admits it by its SHA-256 until it expires
(the view `live_watch_tickets`). It then sends `attached {run, attempt, replayed}`, the stream so far (`started`, the
newest `progress`, and `ended` or `runner-lost` when either is the last word), then the runner's messages as they
come, with an `edge {locked_sim_ms}` after each segment of the picture the server locked: the simtime instant the live
picture reaches, so a page can say how far behind the simulator it trails. The `edge` is live only, never replayed; a
frames viewer is told the edge in its `attached`. Each message is encoded once and shared by every viewer. A viewer
that does not take a message within 10 s of wall clock, or falls more than 256 messages behind, is sent `lagged` and
dropped. The socket closes behind `ended`. `GET /healthz` answers 200.

### External port: frames, `ws://<host>/frames/{run}/{attempt}`

The attach protocol of `studio/viewer/js/src/protocol.ts`, with one field more: the viewer's first message is `attach
{id, from: ms | "live", sections, token}`, within 5 s, `token` the same watch ticket (the ticket's `frames_url` names
this socket); a `run` field, which the path already names, is read and ignored, as is any field not named here. The
server answers `attached {id, run, attempt, from_ms, live_edge_ms}`, then binary frames: the scenario block, the
declarations made at or before `from`, the one keyframe at or before `from` (`"live"` means the newest locked
keyframe; a replay with no `from` starts at the first), then every delta and later declaration in order and never
another keyframe, since the viewer takes one keyframe per attachment and the deltas carry it forward; then the live
frames as they come. `lagged {id, resume_from_ms}` goes to a viewer that stopped reading for 10 s of wall clock, which
is then dropped and re-attaches from that instant; `ended {id, run, outcome}` after the run's last frame, `outcome`
one of the metric stream's (`RunOutcome`): the stop reason the end block states, or `no-end-block` for a copy that
carries none. Frames travel as the bytes the store holds, except the scenario block: a
recording leaves the compiled floor out and a browser has no map compiler, so the server compiles the attempt's world
once (`floor`, as `studio/runs` does), puts the floor and entity census in, and takes the run's data files and the
producer document out. `sections` is read and not applied, and the index frame of a finished recording is not sent.
Each attach writes `last_opened_at`.

Every socket, `/frames` and `/metrics` alike, reads an attempt through one provider (`provider`), which serves the
first copy it finds: the frames in memory while the attempt is open, the disk copy once it has ended, else the
run-object store. A cold attempt (not in memory, no disk copy) is fetched once however many sockets ask, by a task of
its own, the final recording when the API's `attempts.object_key` names one, else the scenario block and the segments
in order, into a part file it renames into place once whole. Every reader of the fetch, the one whose attach started
it included, reads the part file as the fetch says what has landed, `from` honoured as on a whole copy. A fetch that
fails reaches each of its readers as a failure, never as `ended` over what had landed, and the next attach fetches
again. The live segments are the whole run only once the attempt's own server said its uploads finished
(`uploads_finished_at`). Each disk copy keeps its own word on that, judged by what it was copied from and never by
the tables as they stand when it is read: the final recording is whole; a live file is whole once `live.whole` stands
beside it, which this server writes for an attempt it took from its scenario block to its ending, and a fetch writes
for segments whose uploads had all landed. A copy that cannot be vouched for gives way to a fetch of a whole one the
tables name, the final recording or the finished segments; with none to be had it is served as far as it goes, and
both its streams end with the outcome `recording-incomplete`, never with the run's own ending, which the page shows as
such. A fetched copy is checked to end whole before it is renamed into place, so a copy cut short in the store is never
kept.

### External port: metric streams, `ws://<host>/metrics/{run}/{attempt}`

The contract is typed in `studio/protocol/src/metric_stream.rs`; the metrics are `studio/metrics`' catalogue, which
`GET /api/metric-streams` lists: desvis's seventy-four time series from the slices' scalars; three series of time in a
state from the robot state channel (`robot_state`: the D30 and DA50 state shares and the D30 waiting share), each
window made by the span engine once the slice past its end has been taken in; and eight report series from the journals
the deltas carry (the DA50 waiting share, DA50 operations, storage and relay occupancy by aisle, presentations by aisle
and station, orders by DC and station), each window built by the same builder desvis calls once it is final: the barrier
a slice past its end, and for the DA50 waiting share every DA50's interval rows reaching it, since such a row comes only
once the barrier has passed its end. A report series sends the whole run as one more bucket at the end. Twenty-two
more series come only from the end block (`end_block`): desvis's finished Status rows, order extractions by station,
picks by destination, each one bucket spanning the run. One socket carries one metric of one attempt, computed by the
server from the attempt's
recording, the same frames `/frames` serves, so a page draws what the picture shows and computes nothing. The viewer's
first message is `attach {v: 1, metric, token, trailing?}`, within 5 s, `metric` the stable name of one metric the
server computes, `token` the same watch ticket (the ticket's `metrics_url` names this socket), `trailing
{window_sim_ms, step_sim_ms}` an optional ask for causal trailing windows at a step in place of the metric's own
bucketing. The server answers `attached {run, attempt, descriptor, reached_sim_ms, live}`, the descriptor saying the
metric's label, unit, aggregation, bucketing, dimensions and source; then `rows` in batches, every bucket from simtime
0 to `reached_sim_ms`, then each later bucket as the recording grows; then `ended {outcome}` once the run is over and
its last bucket is out, and the socket closes. A bucket travels once, when it is final, and is never revised: a
`scalars` metric's bucket closes once the next frame, the next slice or the end block, says where its slice ends, a
`robot_state` metric's once the slice past its end has been taken in,
a `journals` metric's once the barrier has passed the end of every journal row overlapping it, an `end_block` metric's
only at the run's end; `ended` says how the recording ends, as `/frames` does. A ratio's row carries numerator and
denominator beside the quotient. `lagged` goes to a viewer that stopped reading for 10 s of wall clock, which is
dropped and attaches afresh, receiving the history again. Every failure closes the socket with the protocol's
reason, before anything else when it comes before `attached`: `unknown-metric` for a metric the server does not
compute; `unsupported-trailing` for trailing windows over a running total or an end-block figure, or windows and
steps that are not whole slices of the recording, checked once the scenario block states the slice width;
`not-recorded` for a recording that lacks what the metric is computed from: a scalar its formula reads, the robot
state channel, a journal's table; `recording-unreadable` for a copy that is not a recording, or one whose part the
metric reads does not read; `not-held` for an attempt with nothing to read yet; and `unavailable` when the database,
the disk or the store does not answer. Of these, `not-held` and `unavailable` are worth another try. Nothing of a
metric stream is stored. Every metric of an attempt is computed from one reading of its recording for every metric
socket of the attempt (`metric_streams::hub`): the first socket starts `studio_metrics`' follower over the recording,
which keeps what each family of metrics is computed from; a socket opens its ask's view there, is sent the rows the
view states so far, then each batch the view makes final, and one that falls 1,024 batches behind is told it lagged.
A view that cannot go on, as a part of the recording only its family reads that does not read, closes only its own
sockets. The hub goes once its last socket has left, and one that failed is forgotten at once, so the next socket
starts afresh.

## The T: fan-out and upload

As each locked segment is taken in, the server does two things at once: it sends the segment's frames to every reader
following the attempt, a frames viewer or an attempt's metric hub, each through a queue of its own so a burst after a
runner's lost connection leaves none behind, and it queues the segment for upload. One upload queue per attempt, in
order: the scenario block at `runs/<run>/attempts/<n>/live/scenario.worldview`, segment `k` at
`runs/<run>/attempts/<n>/live/seg-<k, six digits>.worldview` (`runstore::layout::live_scenario_key` and
`live_segment_key`; the padding makes a listing come back in segment order). A failed upload is retried on a backoff
from 1 s doubling to 30 s of wall clock, without end while the server lives; `attempt_streams` is updated after each
success. Once the attempt has ended, the queue's last job, behind every upload, sets `uploads_finished_at` and only
then releases the attempt's memory. Only the streaming server writes under an attempt's `live/` prefix; the final
recording at `runs/<run>/attempts/<n>/recording.worldview` is the API's, stored when the runner sends it to the door.
Nothing here deletes from the store, so the live segments stay beside the final recording.

## The disk cache

`<STUDIO_STREAMING_CACHE_DIR>/<run>/<n>/live.worldview` is an unfinished container file the frames are appended to as
they are taken in, scenario block first, which `Reader::open` reads by scanning; `.../live.whole`, an empty file, says
it holds the whole run; `.../recording.worldview` is the final file once a cold replay fetched it from the store. An
open attempt is in memory and on disk at once; when the attempt ends the memory is dropped once its uploads are done
and the disk copy serves replays. A process that takes an attempt up after an earlier one, a restarted server's or
one that had forgotten the attempt, holds only what its runner sends it from the open segment on: it numbers its
segments on from those the row says are in the store, starting over the last when the segment sent again falls inside
it, so the store keeps the earlier history in order; and since its own copies stop short of the run's start, it never
vouches for its disk copy nor says its uploads finished, and the final recording is the whole run once it is stored. A
sweeper runs every `STUDIO_STREAMING_CACHE_SWEEP_SECS` of wall clock: it deletes the folder of an ended attempt whose
`last_opened_at` (or `cached_at`) is older than `STUDIO_STREAMING_CACHE_TTL_SECS`, then oldest first while the cache's
total passes `STUDIO_STREAMING_CACHE_MAX_BYTES`; never an open attempt's folder, one a socket is reading or a fetch is
writing, or one whose upload queue is not drained, since that copy would be the only one. A deleted copy's `cached_at`
is cleared.

## Bounds

| What | Bound |
|---|---|
| One stream's replay | its `started`, its newest `progress` and its last word |
| Every stream's replay together | at most three messages for every open stream and for each closed one kept. A replayed `progress` or `ended` is a runner's message encoded anew, and can be longer than it: an optional field the runner left out comes back as `null`, and a number in serde_json's own spelling, `1e15` as `1000000000000000.0` |
| Closed streams kept replayable | 50 (`STUDIO_STREAMING_KEEP`); the oldest closed is forgotten first, its frames in memory with it |
| A runner's message | one container frame, 64 MiB of payload at most; its text messages are far smaller |
| An open attempt's frames in memory | 2 GiB (`STUDIO_STREAMING_OPEN_MAX_BYTES`) |
| The disk cache | 20 GiB (`STUDIO_STREAMING_CACHE_MAX_BYTES`), a day since the last open (`STUDIO_STREAMING_CACHE_TTL_SECS`) |

## Configuration

| Variable | Meaning | Default |
|---|---|---|
| `STUDIO_DATABASE_URL` | The API's database, migrated by the API | required |
| `STUDIO_STREAMING_RUNNERS_BIND` | The internal port, where runners connect (`/runners`) | `127.0.0.1:3201` |
| `STUDIO_STREAMING_VIEWERS_BIND` | The external port, where viewers connect (`/streams/{run}/{attempt}`, `/frames/{run}/{attempt}`, `/healthz`) | `127.0.0.1:3202` |
| `STUDIO_STREAMING_RUNNER_URL` | The runners' websocket as a runner reaches it, registered for the dispatcher to write into a job spec | `ws://`, the runners' bind, `/runners`; required when it binds every interface |
| `STUDIO_STREAMING_VIEWER_URL` | The viewers' websocket base as a viewer reaches it, to which a ticket's `/{run}/{attempt}` is added; the API names the frames socket beside it by replacing `/streams` with `/frames` | `ws://`, the viewers' bind, `/streams`; required when it binds every interface |
| `STUDIO_STREAMING_HEARTBEAT_SECS` | How often the registration's heartbeat is written (wall clock) | `2` |
| `STUDIO_STREAMING_KEEP` | How many closed streams stay replayable, the oldest forgotten first | `50` |
| `STUDIO_STREAMING_CACHE_DIR` | The folder the server keeps its disk copies of the frames in, one `<run>/<n>/` per attempt; dev.sh sets `studio/.dev-state/streaming-cache` | required |
| `STUDIO_STREAMING_CACHE_TTL_SECS` | Wall-clock seconds an ended attempt's disk copy outlives its last open before the sweeper may delete it | `86400` |
| `STUDIO_STREAMING_CACHE_MAX_BYTES` | Bytes the disk cache may hold before the sweeper deletes ended attempts' copies, oldest first | `21474836480` |
| `STUDIO_STREAMING_CACHE_SWEEP_SECS` | Wall-clock seconds between sweeps of the disk cache | `300` |
| `STUDIO_STREAMING_OPEN_MAX_BYTES` | Bytes of frames one open attempt may hold in memory; past it the runner's socket is closed `frame-too-large` | `2147483648` |
| `CRUCIBLE_STORAGE_DIR`, `CRUCIBLE_STORAGE_BUCKET` | The run-object store, exactly one of them, as the API reads them; the bucket needs the `gcs` feature (`cargo build -p studio-streaming --features gcs`, which the image builds with) and the deployment's bucket credentials | required |

## Postgres

The tables are migration 0022's, in the API's folder, so the API starts first; migration 0023 adds the picture's
columns, without which the server refuses to start, naming it. A server registers only once its ports are bound, and
unregisters first when it stops. `streaming_servers`: one row per server process, its runners' and
viewers' addresses, `heartbeat_at` and its open streams, written again by every beat; a row whose heartbeat stopped
10 minutes ago is deleted by the next beat of any server. The dispatcher picks a server whose heartbeat is within
`STUDIO_STREAMING_STALE_SECS` (10 s) by the database's clock, and records it on `attempts.streaming_server`.
`attempt_streams`: one row per attempt streamed, the stream's own facts written at most every 5 s and at once when it
closes (the simtime reached and the runner's counters by name, and when and by whom it closed: its runner, or the
server when the runner was lost or the server stopped), each write carrying a revision drawn from one counter per
process, so an older one never overwrites a newer; a restarted process's first write takes over the row, and its
server is the one a watch ticket then names. On the same row, under the server's own upsert (which makes the row at
revision 0 when the stats writer has not yet), what of the picture is where: `scenario_key`, `segments_uploaded`,
`locked_sim_ms`, `uploads_finished_at` (migration 0025), `cached_at`, `last_opened_at`. How the attempt ended is the
API's, on `attempts`.
`stream_watch_tickets`: the viewers' tickets, hashed.

## The live picture is the recording but its index

The live frames are the run's recording as dessim writes it, frame for frame: the journal rows ride in the deltas
written once the barrier has passed their instants, and a metric first stated after the scenario block comes with its
`Declare` frame. The end block, which states the run's result, its summary and the run-level tables, comes last, after
the last delta and before `ended`; the server keeps it in the last segment, so it reaches the live viewers, the disk
copy and the uploaded segments alike, and a page that watched the run gets the end-of-run figures and whole-run tables
a page opening it a week later gets. A run stopped before its first slice has no keyframe, so no segment for its end
block to close: the runner passes that end block over. Only the index block, the container's bookkeeping, never
travels. The picture trails the simulator by one slice of simtime, plus however far dessim's barrier lags its planning
frontier.
