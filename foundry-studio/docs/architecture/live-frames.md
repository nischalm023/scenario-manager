# How a run's picture reaches the browser

The run page shows a run's picture, the floor and the robots moving on it, and the run's metrics beside it, both while
the run goes and after it has ended. This page follows the picture from the simulator to the browser: who writes each
frame, what the streaming server reads of it, where its copies are kept, what a run page downloads, and why a frame
never changes once it is sent.

Three clocks appear. Simtime is the run's own: a slice's instants, the keyframe cadence, the live edge (the simtime
instant the picture has reached). Scheduler time is the order in which the simulator's scheduler decides and reports,
which the section on why a frame never changes uses. Wall clock is the machines': sockets, timers, polls. "While the
run goes", "once the run has ended" and "live" are wall clock: before and after the simulation stops and its runner
reports how it ended.

The simulator records a run as frames, a delta for each slice of simtime, 10 seconds by default, and a keyframe every
30 slices, and writes each frame once. The streaming server passes the picture through as the frames were written,
rewriting only the scenario block for the browser, and computes the run's metrics from the same frames. A run page,
whether the run goes or has ended, attaches to the streaming server for the picture, receives the scenario block, the
declarations it needs, at most one keyframe, and the deltas, declarations and end block after it, and decodes them
itself; each metric panel on the page attaches for its own metric, and receives rows the server has computed.

The sections below describe the mechanism, and a change to the mechanism updates them. The sizes one run measured
before the run page changed are in [the dated section at the end](#measured-on-the-dev-stack-29-october-2026).

## The life of a frame

![The life of a frame: in the runner's container, dessim writes the recording and simrunner sends each frame to the streaming server, end block last. The streaming server appends every frame to its disk copy, uploads each closed segment to the run-object store, sends each browser the rewritten scenario block, the declarations, at most one keyframe, and the deltas, declarations and end block after it, as written, and sends each metric panel the rows it computes from the same frames. Once the run has ended, the API stores the finished file in the run-object store. The steps are listed below the figure.](live-frames/life-of-a-frame.svg "The life of a frame")

**The picture's frames reach the browser as dessim wrote them, all but the scenario block; the metrics reach it as
rows the streaming server computed from those frames.** Amber arrows carry the recording's own bytes, in this figure
and the next, and blue arrows carry JSON. In order:

1. In the runner's container, dessim appends each slice's frames to `recording.worldview` as soon as the slice is
   final, in scheduler time (see [why a frame never changes once sent](#why-a-frame-never-changes-once-sent)).
2. simrunner reads whole frames as the file grows, told how far it has grown by the harness's recording tap, and sends
   each frame to the streaming server's `/runners` websocket, on its internal port, as one binary message. It sends
   the end block last and passes the index over.
3. The streaming server checks each frame's header, checksum and order, and holds each open attempt's frames in
   memory, grouped into segments: a keyframe opens a segment and the deltas behind it join it. A declaration joins the
   segment open when the server takes it in; dessim writes one just ahead of the first slice that states its metric,
   whether or not that slice has a keyframe, or ahead of the end block. One written ahead of a keyframe opens that
   keyframe's segment instead when a second of quiet (step 4) closed the segment before it. The last segment holds the
   end block too. A run stopped before its first slice has no keyframe, and its end block opens the attempt's one
   segment.
4. It appends every frame to its disk copy, `live.worldview`. It uploads the scenario block to the run-object store as
   it takes the block in, and each segment as the segment closes: when it takes in the next keyframe, when the
   runner's socket has sent no binary frame for 1 second of wall clock, or when the runner says `ended`. A segment
   closed by the quiet that then grows is closed again at its next close, and uploaded again, whole.
5. On `/frames/<run>/<attempt>`, on its external port, it sends each browser that attaches the scenario block,
   rewritten: it adds the floor and entity census compiled from the block's world document, which a browser cannot
   compile, and takes out the run's data files and the producer's rebuild document, which nothing a browser draws
   reads. Then it sends the declarations stamped at or before the instant the browser attaches from, the keyframe at
   or before that instant (or the first it holds, when it holds none so early), and every delta, declaration and end
   block after it, each as written, then each new frame as the runner sends it. It never sends a second keyframe,
   since the deltas alone carry the picture forward, and never the index.
6. On `/metrics/<run>/<attempt>`, for each socket that names one metric, it computes that metric from the same frames
   and sends its rows, each bucket once, as soon as the frames it has read make the bucket final.
7. In the browser, the viewer hands each frame to its wasm, which decodes it, draws the canvas and keeps the decoded
   picture in memory. The metric panels draw the rows they are sent and compute none.

Once the simulation stops, the runner gives its stream to the streaming server at most 10 seconds of wall clock to send
what it still holds, `ended` last; then, whether or not all of it went out, it sends the finished file to the API
(`PUT /runner/v1/attempts/{run}/{n}/recording`), which stores it beside the live segments and names it on the
attempt's row.
The streaming server never uploads the finished file; it fetches it from the store when a browser asks for an ended
attempt it holds no copy of (see [where the copies live](#where-the-copies-live)).

## What the streaming server reads of a frame

For the picture, the streaming server reads each frame's header (its kind, its simtime instant and its length) and
checks its checksum, to keep the frames in order and to group them. Of the bodies it decodes only the scenario
block's, for the slice width and to rewrite it, the end block's, for the outcome its `ended` message names, and, when
it opens a finished file, the index block's, which says where each frame starts. Every other frame goes to the browser
as the bytes the store holds.

For the metrics, the streaming server decodes the bodies. One reading of an attempt's recording serves every metric
socket of that attempt: the first socket starts `studio_metrics`' follower over the frames, which reads the scenario
block's metric descriptors and stations, each declaration, the robot-state channel of the first keyframe, the scalar
metrics, journal rows and robot-state channel of each delta, and the end block's stop reason, summary and tables. It
decompresses every keyframe and delta whole, but parses none of the picture's own channels: playback, motion and heat.
`GET /api/metric-streams` lists the metrics it computes.

## Where the copies live

The run-object store's two copies of a run's recording, the server's live segments and the finished file, outlive the
run; so does the runner's file where its output folder is kept, as on the dev stack. The others hold the recording for
a span of wall clock. On the dev stack the stored copies are under `studio/.dev-state/` and the store is a folder; in
a cluster the store is to be a bucket.

| Copy | Where | Made by, and when | What it holds |
|---|---|---|---|
| The runner's file | `runner-out/run-<run, in lower case>-<attempt>/recording.worldview` | dessim, a slice at a time as the run goes; the original | every frame, then the index, the end block and the trailer |
| The server's memory | the streaming server's process | the streaming server, in wall clock from the scenario block until the uploads behind the runner's `ended` finish, or, for a runner lost without one, until the server forgets the closed stream | every frame it took, in segments |
| The server's disk copy | `streaming-cache/<run>/<attempt>/live.worldview` | the streaming server, appending each frame as it takes it in | every frame it took, the end block among them; no index, no trailer |
| The server's fetched copy | `streaming-cache/<run>/<attempt>/recording.worldview`, or a `live.worldview` rebuilt from the segments | the streaming server, when a socket asks for an ended attempt it holds no copy of | the finished file whole, or the live segments joined |
| The store's live segments | `objects/runs/<run>/attempts/<attempt>/live/` | the streaming server: the scenario block as it takes it in, each segment as it closes | `scenario.worldview`, then `seg-000000.worldview`, `seg-000001.worldview`, …; the last ends with the end block when the runner sent one |
| The store's finished file | `objects/runs/<run>/attempts/<attempt>/recording.worldview` | the API, once the runner sends it the finished file | the runner's file, whole |
| The browser's copy | memory | the viewer, from `/frames` | decoded: the scenario, the declarations, at most one keyframe's state and every delta after it, and the end block's instant |

The streaming server's disk copies are a cache, swept every 5 minutes of wall clock. A sweep deletes an ended
attempt's folder once a day of wall clock has gone by since a socket last opened it, or, never opened, since the
server cached it, and deletes the oldest first while the cache holds more than 20 GiB; never a folder a socket is
reading or whose uploads have not finished. The three figures are defaults the server's environment can change. A
browser saves a copy of its own only when someone presses **Download the recording**, which reads the store's
finished file through the API.

### When a copy stops short

A runner that loses its connection reconnects to the same streaming server and sends `started`, the scenario block and
the open segment from its keyframe again; the server passes over a scenario block it holds and starts the open segment
over, in memory and on disk. A server process that takes an attempt up after an earlier one, a restarted server's or
one that had forgotten the attempt, holds only what the runner sends it from the open segment on: it creates
`live.worldview` afresh, and numbers its segments on from those the attempt's row says were uploaded, starting the
last of them over when the segment sent again begins inside it, so the store keeps the earlier history in order.

Each socket reads an attempt through one provider, which serves the first copy it finds: the frames in memory, a fetch
already landing, a disk copy known to hold the whole run, else a fresh fetch from the store. A disk copy holds the
whole run when it is the fetched finished file, or a `live.worldview` with `live.whole` beside it, which the server
writes only for an attempt it took from its scenario block to its ending, or a fetch writes for segments whose uploads
had all landed. A disk copy that cannot be vouched for gives way to a fetch of a whole one, the finished file or the
finished segments, and with none to be had is served as far as it goes. The frames in memory come first all the same,
even those of an attempt the process took up partway. A copy that stops short of the run ends both its `/frames` and
its `/metrics` sockets with the outcome `recording-incomplete`, which the page shows as such.

An open attempt's frames in memory are bounded, by default at 2 GiB. The streaming server refuses the frame that would
pass the bound, `frame-too-large`, and every frame of the attempt after it, so the picture and the live segments stop
there; the finished file, which the runner sends to the API, still holds the whole run.

## What a run page downloads

![A running run's page: the browser gets the page and the viewer's wasm, reads the run every 2 seconds of wall clock, its inputs and the list of metrics the server computes, and asks the API for a watch ticket for each socket it opens but a reattachment after lagged. On the progress socket it sends watch and receives the run's numbers. On the picture socket, on a new ticket, it attaches from simtime 0 and receives the rewritten scenario block, the declarations, at most one keyframe, and the later deltas, declarations and end block the server holds, then each new frame but a keyframe, then ended. On each metric panel's socket it attaches for one metric and receives that metric's rows. The same exchange is told in words below the figure.](live-frames/running-run-page.svg "What a running run's page downloads")

**A run page takes its picture over one websocket and each metric panel's rows over a websocket of its own, whether
the run goes or has ended.** A dot in the figure marks a call passing through the page origin's `/api`; grey arrows
carry the page's own code.

While the run goes, the page loads itself and the viewer's wasm, and reads through the API: the run, again every 2
seconds of wall clock while the Runs panel is open (with the panel's list of runs and the queue), the run's inputs,
and the list of metrics the streaming server computes. It asks the API for a watch ticket each time it opens a socket,
but for the picture socket's reattachment after `lagged` (below), which keeps the ticket it has. A ticket names one
streaming server's three sockets for the run's attempt, `/streams`, `/frames` and `/metrics`, and the token the server
admits a socket by, for 60 seconds of wall clock. It names the server the attempt streamed to while that server's
heartbeat is fresh; when it is gone and the attempt has ended with a copy in the store, another live server picked at
random and kept for the attempt, which reads the attempt from the store.

- On the progress socket, `/streams/<run>/<attempt>`, opened while the run's state is running, the page sends `watch`
  and receives the runner's progress: the simtime its final slices reach and the counts they state, never the picture.
- On the picture socket, `/frames/<run>/<attempt>`, the page attaches from simtime 0. The server answers `attached`
  with the live edge, then sends the frames of step 5 above, one binary message per frame, and `ended` once the run's
  last frame is out.
- On each metric panel's socket, `/metrics/<run>/<attempt>`, the page attaches for one metric. The server answers
  `attached` with the metric's descriptor, then sends rows: every bucket so far, then each bucket as it becomes final,
  then `ended`. The page starts with three panels (picks per hour, inducted totes per hour, and the running total of
  picks) and remembers in each browser the panels someone adds or removes.

Each picture socket the page opens on a new ticket is a full download: it starts from simtime 0 and receives
everything up to the live edge, and every browser tab attaches on its own. The frames are compact per slice, but the
download grows with the length of the run and the size of the site. A page the server drops for reading too slowly,
with `lagged`, clears its picture and attaches again from the simtime instant the server names, holding from then on
only what follows the keyframe at or before it; when its ticket has expired by then, the page asks for a new one and
downloads the run again from simtime 0.

Once the run has ended, the page plays the run from the picture socket and draws the metric panels from their sockets
in the same way, and opens no progress socket. It reads the run's summary, the figures the runner's report stored,
from `GET /api/runs/{run}/metrics?attempt=<n>`. It downloads the finished file only when someone presses **Download
the recording**. A page opened while the run goes keeps its player through the run's ending: its picture socket
receives the end block and `ended`, and the camera, the cursor and the frames received stay. Its metric panels attach
afresh at the ending, each receiving its metric's history again.

Either way the page holds the picture in memory, and Play, pause and seek send nothing over the network. On the dev
stack the page's origin is Vite's dev server, which serves the page and hands `/api` to the API, and the sockets go to
the streaming server's external port at the address the ticket names; in a cluster the login gate is to take the place
of Vite's proxy.

## What a recording is made of

![A recording in file order, here one of a run longer than 30 slices: the scenario block, a keyframe at slice 0 and the deltas of slices 0 to 29, a declaration, here of a metric slice 30 is the first to state, a keyframe at slice 30 and the deltas after it, then, written when the simulation stops, the index, the end block and the trailer. Everything but the index and the trailer is sent live; the store keeps the scenario block and one file per segment under live/, the declaration with the segment open when the server takes it in and the end block in the last segment, and the finished file holds every block.](live-frames/recording-layout.svg "What a recording is made of")

**A keyframe restates the whole state every 30 slices; the end block travels live, last, and only the index and the
trailer stay in the file.**

A slice is 10 seconds of simtime by default, a width the scenario block states, and the last ends at the run's stop. A
delta holds one slice: the robots' playback and motion, heat, the slice's scalar metrics, the journal rows due with
it, and the robot-state channel when the scenario declares one. A keyframe restates the whole state at the start of
its slice, which is what makes seeking cheap: a reader rebuilds any instant from the keyframe at or before it and the
deltas after it. A declaration names a metric ahead of the first slice that states it, keyframe or not, or ahead of
the end block when only the run's final values state it. The end block holds how the run stopped, its result and
summary, every metric's final value and the run-level tables. When the simulation stops, dessim writes the index,
which says where each frame before it starts in the file, then the end block, then the trailer, the file's last 24
bytes, which says where the index and the end block start. A copy without a trailer, as the server's disk copy is, is
read by scanning it from its first byte.

The streaming server closes a segment on the next keyframe, so a segment in the store holds one keyframe and the
deltas behind it, with any declaration the server took in while it was open, and the last ends with the end block
when the runner sent one; a run stopped before its first slice has one segment, its end block alone. A segment closed
by a second of wall clock without a frame and then grown is closed again at its next close, and uploaded again whole.
A run slower than a slice per second of wall clock closes its open segment after nearly every slice, and so uploads
each segment about once per slice it holds, some 30 times, and more when a lost connection starts it over.

The byte-level specification is [the `.worldview` recording format](../designs/worldview-format.md); its [section on
slices](../designs/worldview-format.md#4-slices) gives the order of the frames around a keyframe.

## Why a frame never changes once sent

The scheduler keeps [a global barrier](../../sim/dessim/docs/global-barrier.md), B: an instant of simtime that the
scheduler advances only while the executor is quiescent, and before which no new claim may start. A slice is final
once B has passed its end: as the observer reads the advance of B past a slice's end, in scheduler time, every fact
that could land in the slice has already been reported, and the observer hands the slice to the run's recording, once
([the barrier's consumers](../../sim/dessim/docs/global-barrier.md#consumers)). The slice that reaches the run's
simtime limit waits instead, with every slice B has not passed, for the event stream to close, in scheduler time, when
the run's stop is known: then each slice that begins before the stop is made final, the last ending at the stop, and a
slice that begins at or after the stop is dropped. A fact that writes into a slice B has passed stops the run, and so
does a fact stamped behind B. That is what lets the streaming server and the browser treat a received frame as final,
with no revision channel and no dirty flags. A frame can be sent again, after a lost connection or in a segment
uploaded again, but always as the same bytes.

The price is a lag: until the event stream closes, a slice reaches the recording only once B has passed its end, and
B trails the simtime of the newest activity the scheduler has already accepted. The live edge an attach names is the
simtime instant of the newest frame the server holds other than the end block.

## Measured on the dev stack, 29 October 2026

This section records one run on one stack, measured before pull requests 600 and 610 changed the run page, the runner
and the streaming server; it describes that branch, not the mechanism above, and its numbers will change with the
format, the fixtures and the code. Then, the runner sent neither the end block nor the index, so the server's copies
held no end block, and the page opened no metric socket.

The dev stack was brought up on 29 October 2026 from the PR #597 branch at `74e9f406a` (on main at `e37cc96a7`). The
run was a calibration of 5 minutes of simtime on a small rectangular site, started by a script making the Runs tab's
own API calls: create an experiment on the site, which the API turned into a plan naming the runner and the run id,
then submit the plan. It ran in under a second of wall clock.

| Copy | Bytes |
|---|--:|
| The runner's file | 379,429 |
| The server's disk copy | 376,511 |
| The store's live segments: `scenario`, `seg-000000`, `seg-000001` | 61,532 + 291,633 + 23,346 = 376,511 |
| The store's finished file | 379,429 |
| The browser's copy, received in 45 messages | 384,092 |

The server's disk copy was 2,918 bytes shorter than the runner's file because it held no index, no end block and no
trailer. The browser received the scenario block rewritten, 91,996 bytes against 61,532 stored, and not the keyframe
at slice 30, 22,883 bytes: 91,996 + 376,511 − 61,532 − 22,883 = 384,092. The browser's memory use was not measured.

The run page of that branch attached a finished run over the picture socket, as the page does again since pull request
610. Opening the run page fetched the viewer's wasm, `crucible_viewer_bg.wasm`, 6,182,561 bytes, and a watch ticket of
304 bytes, and the server answered `attached` with `live_edge_ms: 310000`, an instant of simtime. For this finished
run, a 30-second capture of wall clock after Play showed no new socket and no frame; only the Runs panel's polls,
every 2 seconds of wall clock, went out.

| Run, its length in simtime | What an attach downloaded |
|---|--:|
| This run, 5 minutes, small rectangular site | 384,092 bytes |
| A DFW6 run, 30 minutes (the server's copy) | about 34 MB |
| A run of 12 hours on a 12-aisle site | 95 MB |

On the Cencora and DFW6 fixtures the live edge stood about a minute of simtime behind the newest activity. On that
stack the API answered on port 24228, Vite on 24231, and the streaming server's runners' and viewers' listeners on
24233 and 24234.
