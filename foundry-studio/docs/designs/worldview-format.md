# The `.worldview` recording format

A `.worldview` holds one simulation run: everything it was built from, everything it did, and
everything it ended with. One artifact answers every question asked of a run — watch it while it
executes, leave and rejoin, scrub backward and forward, read a crashed run's prefix, rebuild the
workbook — and none of those decodes the file from the start.

This document is the byte-level specification: a second implementation can be written from it alone.
The tests in `worldview/worldview` pin every layout here, and where this document and that code disagree
the code is right and this document is wrong.

Two layers. The **container** is frames, the sections inside a keyframe or a delta, the index block
and the trailer; it carries payload bytes and never reads them. The **vocabulary** is what those
bytes mean: the scenario block, declarations, playback, scalars, heat, tables, motion and the end
block. A consumer that only seeks needs the container; a consumer that draws needs both.

## 1. Conventions

- Little-endian throughout, for every integer the container packs itself.
- Every payload is one MessagePack value. A writer stores it as exactly one zstd frame at level 3 and sets
  flag bit 0; a reader accepts a payload stored without the flag, but never one holding more than that single
  zstd frame — a second frame appended after the first, or any trailing byte, is damage, because decoding the
  first and handing it on would silently change what the frame says.
- Records inside a payload are **positional arrays of fixed arity** — never maps with field names. A
  reader counts positions, so no field is ever inserted, dropped or reordered; a new field is a new
  container version.
- An enum inside a record is **one byte**. Serde's own enum form (a one-entry map keyed by the
  variant name) is not used anywhere.
- A quantity or an identity travels as the integer its newtype wraps: a cell id is its `u32`, an
  instant and a duration are their `u64` milliseconds.
- `nil` means absent, and is written as MessagePack nil (`0xc0`). It never means zero.
- A byte no variant answers to, and a record component a variant has no use for, are both refused on
  decoding. Nothing is repaired: a decoder that rewrote what it read would make two consumers of one
  file disagree.

Three things are packed by hand rather than as MessagePack, because a reader searches them without
decoding anything: the section table, the index records and the trailer.

## 2. Frames

A file is frames and nothing else, then a trailer. A frame is a 20-byte header and one stored
payload, and it is independently decodable: the header says where the frame ends and the checksum
says whether it arrived whole.

| bytes | field | meaning |
|---|---|---|
| 0 | `tag` u8 | 1 scenario · 2 declare · 3 keyframe · 4 delta · 5 index · 6 end |
| 1 | `flags` u8 | bit 0: the payload is zstd-compressed. Every other bit is reserved and zero |
| 2–3 | reserved u16 | zero |
| 4–11 | `t_ms` u64 | the simulated instant the contents begin at |
| 12–15 | `payload_len` u32 | the stored payload's length, compressed |
| 16–19 | `crc32c` u32 | Castagnoli CRC over header bytes 0–15 and then the whole stored payload |

Ceilings: a stored payload is at most 64 MiB and a decompressed one at most 64 MiB. A header declaring
more is damage, and nothing is allocated from what a header claims — a reader grows its buffer as bytes
actually arrive. A writer refuses a body above the decompressed ceiling before it compresses it, since no
compressed size would make such a frame readable.

**The magic.** The first 11 bytes of the scenario frame's payload are uncompressed: `WORLDVIEW\0`
then a version byte, currently **1**. That byte moves only when a format that has reached master
changes; while a format is still being settled its wire changes without it. The zstd frame follows them. One rule therefore opens a file and a
stream alike. There is no file preamble, and no other frame carries the magic.

**Opening a source** reads its first 31 bytes — the header and the magic behind it — and decides among three
answers. A first byte that is not the scenario tag is somebody else's file, said so by name. A header that
decodes but is not a scenario frame, or that declares a payload shorter than the 11 bytes the magic needs, or
bytes that contradict the magic or the version, is a damaged `.worldview`. Fewer than 31 bytes, every one of
them matching, is an **incomplete opening**: the first frame is still being written, nothing is verified yet
and nothing is wrong, so a reader hands back an empty index and looks again later.

**Order.** `t_ms` never decreases over a file. At one instant a keyframe comes before the deltas
stamped with it. The writer refuses a frame that breaks either rule rather than restamping it, and
every reader checks the same rule over what it finds.

**Stamps.** The scenario frame is stamped 0. A declare frame carries the start of the slice it
precedes. The index block and the end frame carry the run's last instant.

## 3. Sections

A keyframe's and a delta's payload opens, after decompression, with a table saying where each part
of it sits, so a consumer takes the ids it knows and steps over the rest.

| bytes | field |
|---|---|
| 0 | `count` u8 |
| then `count ×` | `id` u8, `offset` u32, `len` u32 |
| then | the bodies, in the table's order |

Offsets are relative to the first byte after the table. The ids ascend, are distinct, and the bodies
are one contiguous run ending exactly where the payload does. A gap, an overlap, a repeated id, a
descending id or a trailing byte is damage. An empty section has length 0 and is present; a section
the frame does not carry is absent, which is not the same thing.

Section ids, declared by name in the scenario block: **1 playback · 2 scalars · 3 heat · 4 tables ·
5 motion**. A keyframe carries 1–3. A delta carries 1–5; tables and motion are delta-only.

**In a file, every frame of a kind carries every section of that kind** — a keyframe all three, a delta all
five. A slice in which nothing happened is therefore a delta whose five sections are present and empty, not a
delta with sections left out, and a writer refuses an incomplete one. Leaving a section out is for the wire,
where a server sends a browser only what it asked for; a consumer reading a file never has to tell a section
that says nothing from one that was never written.

**On a wire, an asked-for section a frame does not have is simply absent.** A keyframe carries 1–3, so an
attachment that asked only for `motion` receives keyframes with NO sections at all and deltas with that one —
which is right, and not an error: the request named a section this format defines and this frame kind does not
carry. **`tables` never travels to a browser**: it is the run's journal, the largest thing in a recording, and
nothing a browser draws reads it. A server refuses an attachment that names it rather than sending it.

**On a wire, the index block is a list of instants.** A server sends a browser the records of the file at or
before the instant it attached at, so a viewer knows the shape of the run — which instants carry a keyframe,
where the declarations fall — before a frame has arrived. Only `t` and `tag` mean anything there: `offset` and
`len` describe where a frame sits in the FILE, and the browser is being sent re-encoded frames over a socket,
not the file. A browser seeks by asking to attach again, never by an offset.

## 4. Slices

A slice is a half-open span `[start, start + width)`. A delta is stamped with its slice's start, and
every event offset inside it satisfies `0 ≤ offset < width`. At boundary K the writer emits, in this
order: the delta of the slice ending at K (stamped K − width), the keyframe at K, then the delta of
the slice starting at K. Empty slices still get a delta, so the sequence is dense; a run that ends
inside a slice emits that partial slice.

A keyframe every `keyframe_cadence_slices` slices. Both the width and the cadence are in the
scenario block, and a reader takes them from there and never from configuration.

**Seeking to `t`.** Take the last keyframe with `t_ms ≤ t`, every declaration made at or before `t`,
then the deltas after that keyframe with `t_ms ≤ t`. Apply all but the last whole; the last one's
slice contains `t`, and a reader reads positions out of it without applying it.

## 5. Closing a file

`[scenario][keyframe][delta]…[keyframe][delta]…[index block][end][trailer]`

The **index block** (tag 5) is a packed array of 24-byte records, one for every frame written before
the block — not itself, not the end frame:

| bytes | field |
|---|---|
| 0–7 | `offset` u64 |
| 8–15 | `t_ms` u64 |
| 16 | `tag` u8 |
| 17–19 | padding, zero |
| 20–23 | `len` u32, the whole frame's length, header and payload |

The records describe one unbroken run of frames from byte zero, and a reader checks that rather than
trusting it: each frame ends where the next begins, every `len` is at least a header — 31 bytes for record 0,
which carries the magic — so offsets strictly increase; record 0 is the one scenario record and no other
record is a scenario; no index or end record appears inside the block; and the order rule holds across the
records. A block is only worth reading instead of the frames themselves if it is exactly what a scan would
have found, so a reader that finds any of this wrong scans instead.

The index block and the end frame are written only by closing a file, once, and a writer refuses to append
either as an ordinary frame.

`Writer::create` makes the folder the file goes in when that folder does not exist yet, so a run told
to write into a new output directory is not told at the end that it cannot.

The **trailer** is the last 24 bytes of a finished file: `WVEND\0\0\0`, the index block's offset
u64, the end frame's offset u64. Opening a file reads those 24 bytes; a valid trailer means one jump
to the block, and no trailer means a scan from byte zero. A stream carries no trailer and no index
block — there is no end to seek back from.

**Recovery.** A scan walks the frames and rebuilds the records. It ends in one of two ways, and they
are not the same. A frame the file ends inside is the torn tail of a run still being written or one
that crashed: it is dropped and what came before it stands. A frame whose bytes are all there and
which still does not decode is damage, and it is raised with its offset rather than treated as the
end of the file.

## 6. The scenario block (tag 1)

One record of 24 positions, behind the magic. It carries everything a reader needs before any
instant of the run.

| # | field | type |
|---|---|---|
| 0 | `container_version` | u8, currently 1 |
| 1 | `run` | str |
| 2 | `attempt` | u32 |
| 3 | `run_name` | str |
| 4 | `commit` | str |
| 5 | `dirty` | bool |
| 6 | `timestamp` | str |
| 7 | `hostname` | str |
| 8 | `slice_width` | duration, ms |
| 9 | `keyframe_cadence_slices` | u32 |
| 10 | `frame` | str: the world document's root 3-D frame, by entity id — the frame every position in the recording is measured in. Empty where the document declares no primary frame |
| 11 | `sections` | `[[id u8, name str]…]` |
| 12 | `world` | bin: the world document as read from disk |
| 13 | `scenario` | bin: the scenario document — the world spec — as read from disk |
| 14 | `data` | `[[reference str, bin]…]`\|nil: the files the scenario document names, keyed by the reference string as written there. **nil on a wire** |
| 15 | `sim_config` | bin: the simulator's configuration, MessagePack the producer writes and reads |
| 16 | `producer` | bin\|nil: whatever else the producer's run model needs to rebuild itself losslessly. **nil on a wire** |
| 17 | `cells` | `[[CellId u32, entity_id str]…]` |
| 18 | `stations` | `[[StationId u32, entity_id str]…]` |
| 19 | `robots` | `[[RobotId u32, name str, class u8, class_entity str\|nil]…]` |
| 20 | `floor` | bin\|nil, **wire only**: the compiled floor a viewer draws |
| 21 | `census` | bin\|nil, **wire only**: which cells each entity of `world` turned into |
| 22 | `metrics` | `[MetricDescriptor…]` |
| 23 | `tables` | `[TableDescriptor…]` |

A consumer resolves every reference against `data` and the filesystem never. Robot class bytes:
**1 HTM · 2 VTM · 3 RTP**.

**A block whose version byte is not the one this crate writes is refused, naming what it said.** There is no
shim in either direction: two readings of one block would leave a reader deciding what a recording says.

**`data` and `producer` are nil on a wire.** A server sending a browser a scenario block states MessagePack nil
for both: the workload files are tens of megabytes of something a browser never draws, and the producer document
is the simulator's own rebuild state, which a browser has no simulator to rebuild into. A FILE states both. A
consumer that finds nil there is holding a wire capture, not a recording, and the readers that rebuild a run
model say so rather than reading an empty document as an empty run.

**`floor` and `census` are nil in every file.** Turning a world document's entities into a floor of
cells is the simulator's map compiler, which no browser has, so a server that serves a recording
compiles the document once per run and puts the answer in the scenario block it sends. A file states
nothing for either, because the world document beside them says the same thing and two statements
that could disagree are worse than one. `floor` is `worldview-render`'s own scene document
(`document::FloorDoc`: the cells dense by id with each one's tile, heading and kind, the cell size,
and the optional edge, rack and underlay layers), and `census` is `[[entity_id str, [CellId u32…]]…]`.
Both are MessagePack this crate carries and never reads, like `sim_config` and `producer`, and both
are defined by the crate that writes and draws them rather than here.

`container_version` restates what the magic ahead of the block already said. The two disagreeing is itself the
fault, so a writer refuses to write a block whose version is not the one it writes, and a reader refuses to
read one whose version is not the one it reads.

The three id tables are state, and the encoder writes each in ascending id. The two descriptor lists are not:
their order fixes a metric's slot and a table's place, and it is written down nowhere else.

**`MetricDescriptor`** — 4 positions: `name` str, `label` str, `description` str, `semantics`.
**`MetricSemantics`** — 3 positions: `unit` u8, `denominator_description` str|nil, `aggregation` u8.
Unit bytes: 1 count · 2 milliseconds · 3 millimetres · 4 ratio · 5 per hour. Aggregation bytes:
1 additive · 2 interval · 3 ratio · 4 snapshot gauge · 5 distinct count. A gauge's value is the
newest reading; everything else accumulates.

**`TableDescriptor`** — 4 positions: `id` u16, `name` str, `columns` `[TableColumn…]`,
`instant_column` u16|nil (which column holds each row's instant; nil for a table whose rows have no
instant at all). **`TableColumn`** — 3 positions: `name` str, `value_type` u8, `encoding` u8. Value
type bytes: 1 unsigned · 2 signed · 3 float · 4 bool · 5 text · 6 instant (ms) · 7 duration (ms).
Encoding bytes: 1, the cell is the MessagePack scalar its value type names, at this column's
position in the row.

Descriptors are **append-only for the run**: a metric's slot in the scalars arrays and a table's id
never move once the run has stated them.

## 7. Declarations (tag 2)

One record of 2 positions: `kind` u8 — **1 metric · 2 table** — and the descriptor that kind names.
A run declares a metric when the thing it measures first exists, and a table when its first row is
written; the descriptor is appended to the run's list.

## 8. Playback (section 1)

**A position** is its own array of 3 positions: `x_mm` i32, `y_mm` i32, `z_mm` i32 — whole millimetres along
the three axes of the world document's root 3-D frame, which the scenario block's `frame` names once. All
three are always stated: the vertical one is how a recording says where a machine that rides a rack is, and a
machine that never leaves its standing surface states that surface's height every time. The unit is the
millimetre whatever step the document's own frame counts in — DFW6's primary frame declares `unit_mm: 0.1`,
and a recording of a DFW6 run still states millimetres. Millimetres are the whole of the format's precision: a
producer holding finer numbers rounds them half away from zero on the way in, and one holding a coordinate
that is not finite, or that rounds past what an i32 carries, writes no file at all.

The numbers in the file are therefore whole millimetres, and what a viewer draws between two of them is that
viewer's own business — the format asks nothing of it. The library's own `RunState::at` answers in whole
millimetres, because a position is three integers and its interpolation lands back on them.

The scenario block's `frame` is never empty. A block naming no frame leaves every millimetre after it measured
from an origin nothing states, so it is refused where it is decoded, on a wire as in a file.

**Cells are advisory.** A record names cells beside its position so a consumer reporting per cell has a cell
to count against. Nothing is placed from one: a reader that resolves a cell to a centre in order to draw a
robot has read the record wrong.

**A tote location** is three positions — `kind` u8, the id the kind names u32, the rack level u16 —
nested as one element wherever a record names a location:
0 at a cell · 1 extracted · 2 in a rack (uses both the cell and the level) · 3 at a relay ·
4 on a robot (the id is a robot's) · 5 on a conveyor. A location carrying a component its kind has
no use for is refused.

### Keyframe — 5 positions

| # | field | type |
|---|---|---|
| 0 | `robots` | `[[RobotId, class u8, at, heading_mrad i32\|nil, carrying ToteId\|nil, cell CellId, toward CellId\|nil]…]` |
| 1 | `totes` | `[[ToteId, tote_type u8, designation u8, location, contents_units u64]…]` |
| 2 | `reserved_relay_cells` | `[CellId…]` |
| 3 | `stations` | `[[StationId, pipeline_depth u32]…]` |
| 4 | `transits_in_flight` | `[ToteTransit…]` |

Tote types: 0 tray · 1 tote · 2 carton. Designations: 0 inventory · 1 order. A tote whose location is
*extracted* has left the run: the encoder leaves it out of every keyframe, and a decoder refuses one that says
otherwise. A robot's `heading_mrad` is which way it
faces at the keyframe's own instant — the heading the slice beginning there starts with.

A robot's `at` is where it is, and the whole answer to that. `cell` is the cell it is in — what a consumer
counting robots per cell counts it against — and `toward` the cell it is travelling to when the keyframe's
instant catches it between two, nil when it has arrived.

These arrays are **state**, so the encoder writes them in ascending id — robots, totes, cells, stations, and
transits by tote, then start, then end. Two producers that gathered the same instant in different orders write
the same bytes.

### Delta — 9 positions

| # | field | type |
|---|---|---|
| 0 | `movers` | `[[RobotId, start_heading_mrad i32\|nil, steps, cell CellId]…]` |
| 1 | `headings` | `[[RobotId, heading_mrad i32\|nil]…]` |
| 2 | `tote_creations` | `[[offset, ToteId, tote_type u8, designation u8, location]…]` |
| 3 | `tote_transitions` | `[[offset, ToteId, location]…]` |
| 4 | `tote_transits` | `[ToteTransit…]` |
| 5 | `tote_contents` | `[[offset, ToteId, units u64]…]` |
| 6 | `relay_transitions` | `[[CellId, offset, reserved bool]…]` |
| 7 | `station_changes` | `[[offset, StationId, pipeline_depth u32]…]` |
| 8 | `station_display` | bin: the producer's own station panel records for the slice |

`ToteTransit` — 5 positions: `ToteId`, `from` CellId, `to` CellId, `start`, `end`. Its instants are
absolute, not offsets, so every slice a journey overlaps can place the tote on its own.

A **step** is 4 positions: `at`, a position; `dt` duration; `kind` u8 — **1 move · 2 reverse · 3 wait ·
4 rotate** — and `cell` CellId, the leg's destination. `at` is where the step leaves the mover and `dt` is the
time it spent on that step inside this slice; the step's source is wherever the mover was before it. A leg the
planner curved arrives at one cell along an arc and is recorded as several steps all naming that cell, each at
its own point on it, and a slice whose end cuts a leg part way states the point it cut at — both are ordinary
steps, because the coordinates say where the mover is and the cell never has to. Between two steps a reader
draws the straight line over the later one's `dt`.

A step of no duration that also changes the position is refused: it says the mover crossed that distance in no
time. One of no duration that leaves the position where it was is ordinary — it is how a turn or a wait of no
measured length is written. A mover that states no step at all is refused too: a mover is a robot that MOVED,
so one saying nothing about where it went would still carry the robot's occupancy to the cell it names.

A mover's `cell` is the cell the slice leaves it in. It is stated rather than taken from the last step because
the slice's end can cut a leg: the step names the cell the mover was travelling to and this names the one it
is still in, so a reader that replayed every delta counts the mover against the same cell as one that sought
to the next keyframe. The two differing is exactly what says the mover is still under way, and it is where the
next keyframe's `toward` comes from.

A robot that did not move has **no entry** among the movers. A mover's `start_heading_mrad` is its heading
when the slice opened; nil there says nothing, and a reader keeps the heading it was already holding.

`headings` states which way robots face at the slice's **start**, and lists every robot whose heading there is
not the one the reader was holding — a robot that turned where it stands as much as one that walked. Nil in
this array does say something: the producer no longer tracks a heading for that robot, and a reader forgets
the one it held. A keyframe at that same instant states the same headings, which is what lets a reader that
seeked and a reader that replayed agree about a robot that never moved.

`headings` is state, and the encoder writes it in ascending robot id. Every other array here is **events**:
the producer's order — by offset, then by the order the run emitted them — is the order they happened in, and
nothing is sorted, not even by id. A tote born and extracted inside one slice appears in that delta's
creations and transitions, and in no keyframe.

## 9. Scalars (section 2)

One position, an array of one slot per declared metric in declaration order: `[values: [f64|nil…]]`,
the same shape in a keyframe and in a delta.

A keyframe's slot carries what the metric's aggregation class says it is at that instant — a running
sum for a counter, the newest reading for a gauge. A delta's slot carries the slice's own value. A
slot is **nil when the producer has no value for that metric in that slice**, which is not zero: a
reader must rebuild exactly the metrics that were stated.

An array shorter than the descriptor list means the trailing slots had not been declared when the frame was
written; a slot is only ever appended, so a frame written early stays readable. An array **longer** than the
descriptor list is damage: nothing says what its last values count or how they combine.

## 10. Heat (section 3)

One position, an array of layers. A keyframe layer is 2 positions — `name` str, `counts`
`[[CellId, u64]…]` — and carries the counts at that instant. A delta layer is the same two
positions, and carries what the slice added, for the cells it touched only.

Layer names are the producer's own (`nav_visits_htm`, for one). These are counters with no order of their
own, so the encoder writes layers by name and counts by cell id. A count that passes what a `u64` holds is
damage — no run counts a cell eighteen quintillion times — and a reader says so rather than stopping the
counter at the top.

## 11. Tables (section 4, deltas only)

A bare array, with nothing wrapped around it, of 3-position entries: `table_id` u16, `row_count`
u32, `rows` bin. Entries are in ascending table id, and a table with no rows in a slice has no entry.

`rows` is the concatenation of positional MessagePack rows in the table's declared column order,
back to back with no outer container. `row_count` is authoritative: a decoder reads exactly that
many rows and stops. Two blocks of one table therefore **splice** — appending the bytes and adding the counts gives exactly what
encoding both at once would have given — so a server can cut a chunk out of a recording without decoding a
row. Blocks of two different tables do not splice, and a spliced count that passes what it holds is damage;
both numbers come out of a file, so both are checked.

A delta carries the rows that are due once B has passed its slice's end: every row that no earlier frame
stated whose instant of finality lies before that end. That instant is the row's own, but for a leg or a VTM
cycle, which states a span and is final at its end; the row's instant column still holds its start. A row is
never stated before its slice; one the run reports after its slice went out rides in the next delta written,
where it became known, so rows due at one instant keep the order the run reported them in. A table with no
instant column cannot be cut into slices, and rides in the end block instead.

## 12. Motion (section 5, deltas only)

A bare array of 2-position entries: `RobotId`, `spans`. Robots ascend by id; spans are in start
order; a robot that made no move in the slice has no entry.

A **span** is 6 positions: `source` CellId, `destination` CellId, `start`, `end`, `distance_mm` f64,
`profile`|nil. A **profile** is 13 positions, all f64, in this order: `entry_velocity_mm_s`,
`exit_velocity_mm_s`, `peak_velocity_mm_s`, `accel_time_s`, `cruise_time_s`, `decel_time_s`,
`accel_distance_mm`, `cruise_distance_mm`, `decel_distance_mm`, `total_time_s`, `total_distance_mm`,
`acceleration_mm_s2`, `deceleration_mm_s2`.

A step list says which cells a mover reached and when, which is everything playback needs and nothing a speed
can be read from. These spans are what the traffic overlay bins distance and time by. Every number the planner
computed for the move travels, and a consumer reads what it needs rather than recomputing it: the phase times
sum to `total_time_s` and the phase distances to `total_distance_mm` only modulo the rounding of the
arithmetic that produced them, and a phase of zero time leaves no way to recover the accelerations at all. Any
phase may be zero — a move with no cruise, or no motion, is this same shape with zeros in it. A nil profile
means the consumer reads the move as one constant speed.

## 13. The end block (tag 6)

One record of 7 positions.

| # | field | type |
|---|---|---|
| 0 | `stop_reason` | str |
| 1 | `stop_detail` | str\|nil — what the reason points at, when it points at something |
| 2 | `result` | bin: the run's result, the producer's own MessagePack |
| 3 | `summary` | bin: the summary statistics the workbook cannot derive from the tables |
| 4 | `cumulative_scalars` | `[f64\|nil…]`, positional per the descriptors, as a keyframe's |
| 5 | `tables` | `[[table_id u16, row_count u32, rows bin]…]` |
| 6 | `frames_written` | u64 |

The `summary` document holds what the run states once and no table can be cut into slices: the stripe plan,
the rack-zone capacities, the station and robot timelines, the station gap statistics, the per-robot duty
totals, the station types, the relay utilisation timelines, the per-station presentation rates and
utilisations. Everything with an instant is a table, each row stated in the delta written once B has passed
the instant that makes it final, which for a leg or a VTM cycle is its end. A table whose rows carry no
instant at all rides in `tables`, in the same three fields a slice's rows use; so do the rows the run reported
after its last slice went out, which became known only at its end.

## 14. Rebuilding a state

A state stands at an instant, and that instant is where the next slice begins: a keyframe puts it at
the keyframe's own instant, and taking in a slice moves it to where that slice ends. So a delta's
offsets are always measured from where the state already stands.

A state takes in a whole slice only when that slice begins where the state stands; one that begins anywhere
else is refused, because a slice's offsets are measured from where the state already is. Nothing is changed
until everything that can be refused has been refused, so a delta a state cannot take leaves it exactly as it
was.

Taking in one whole slice, in this order: first the headings, which are true from the slice's start, so they
belong to a state standing there whether or not the slice is then applied — a reader that only looks inside
the slice takes them and nothing else; then the tote moves, merged from three arrays into the order they
happened — a creation, then a transit coming to rest, then a transition, for records sharing an instant, and
records of one kind sharing an instant keep the order the run emitted them in; then contents, relay
reservations and station depths, where the last record for a thing stands; then each mover, which lands at its
last step's position, in the cell the mover states, heading for the last step's cell when that is another one.
A transit is keyed by
the instant it comes to rest: one that ended before the slice opened has already landed, and one reaching past
the slice's end has not landed at all — its tote is still travelling, and the journey is carried forward, once
however many slices it spans. A tote whose location becomes *extracted* is dropped from the state. Which robot
carries which tote is stated twice — on the tote as its location, on the robot as what it carries — and the
two are kept in step, so a state reached by applying deltas holds what the next keyframe states.

Reading a place **inside** a slice, without applying it: take the slice's headings, then walk each mover's
steps from the slice's start. A waiting or turning step gives its own position, and a travelling one the point
the instant has reached along the straight line from the position the step started at — the state's, or the
step before it — to its own, over the step's `dt`. The library answers in whole millimetres, since a position
is three integers. The mover counts against the cell it was in when the step began, because that is the cell
it has not yet left. Totes move by the same
merged order, up to that instant.

Scalars accumulate or replace by their aggregation class; a slot the slice says nothing about keeps
what it held, and one nothing has ever said stays absent. Heat counts add.

A delta that moves a robot no keyframe ever placed is refused, and so is a delta that does not begin where the
state stands: a run is read forward. Every sum over numbers a frame states — the step durations a walk adds
up, the counts a heat layer raises — is checked, and one that passes what it can hold is damage rather than a
wrapped number and a mover drawn where it never was.
