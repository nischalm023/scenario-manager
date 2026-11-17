# The render dump — what desvis would upload, written to files

A render dump is what desvis's renderer would hand the GPU for one simulated
instant, written to files instead. It exists for one job: showing that two
desvis binaries draw the same picture from the same run without anybody
comparing screenshots. `desvis --recording <file> --dump-render <ticks> <dir>`
writes one, and `scripts/worldview/parity_b.sh` (harness B) writes one from
each side and diffs the bytes.

The dump is a diagnostic, not a recording format: nothing reads it back into
desvis, and it carries no version negotiation. The schema string in the
manifest changes whenever a field moves, and both sides must agree on it.

## What a dump run does

`--dump-render` opens the recording, builds the same playback app the GUI
builds, and then, for each listed instant in the order given:

1. puts the app in the view realtime playback puts it in — **State** view with
   the heat overlay **off**, playback active — and moves the cursor to the
   instant, which is `DesvisApp::update_grid_for_cursor`, the same walk over
   the step lists the GUI runs every frame;
2. builds the bot instances, the tote instances, the per-cell instances and the
   relay reservation flags from the app as it now stands;
3. writes each as a file of rows and appends its layout to the manifest.

One app serves every instant, because that is how a session behaves: a marker
desvis placed in an earlier slice stays where it stands until a later slice
moves it. Dumping the instants in a different order therefore gives different
answers, and both sides must use the same list.

Three overlay arrays sit beside the four. `slice_heat` is the overlay the app
holds after the cursor moves — the per-slice count off the step cells that
`update_grid_for_cursor` puts there — dumped at every instant. `traffic` is the
two tint columns the traffic overlay paints, which the dump asks for by putting
the app briefly into the view that draws them; it reads the motion spans, the
newest thing a recording carries. `heat` is the run-total overlay a recording
shows when it OPENS, taken before any cursor move; it belongs to the instant
the run ended at and to no other, so that instant is appended to the list when
the list leaves it out, and the manifest states it as `run_end_ms`. The run's
end is the summary's own sim time, which is also what the recording's end frame
is stamped with. The recording additionally carries per-slice heat increments;
desvis draws none of them yet — they are for the browser in phase 2.

## The files

```
<dir>/manifest.json
<dir>/t<instant_ms>.bots.bin
<dir>/t<instant_ms>.totes.bin
<dir>/t<instant_ms>.cells.bin
<dir>/t<instant_ms>.reserved.bin
<dir>/t<instant_ms>.slice_heat.bin
<dir>/t<instant_ms>.traffic.bin
<dir>/t<run_end_ms>.heat.bin
```

Every `.bin` is rows of fixed stride with **no padding anywhere**: each field
starts immediately after the one before it, little-endian, and an `f32` is the
bit pattern the renderer computed — nothing is scaled or re-ordered on the way
out.

Two values are stated at the format's resolution rather than the renderer's,
because that is all a recording carries. A **heading** is rounded to whole
milliradians on both sides with the writer's own arithmetic,
`(f64::from(radians) * 1_000.0).round() as i32` (the round-2a ruling). A NaN is
refused BEFORE that cast, since `f32 as i32` turns a NaN into a zero that reads
as a heading along +x. A **position** — the `world_x` and `world_y` of a bot row
and of a tote row — is rounded to whole millimetres on both sides before it
becomes an f32 (the 2026-09-18 ruling); `RenderFrame::world_mm` is a plain
divide by a thousand, so the dump rounds the render metre to the nearest
thousandth, which rounds the millimetre itself. `world_z` is untouched: it is
the rack-top proxy each renderer works out for itself and it is already the same
number on both sides. Everything else compares at full width.

## What the comparison proves, and on which inputs

**Master, given its recorded positions and headings at the precision a
`.worldview` states them, draws byte-identically to the branch.** That is the
whole claim and it is exact: no tolerance, no epsilon, no array left out of the
diff.

The qualification is load-bearing, and the master patch is where it is applied.
Master's observer keeps a robot's position as full-precision f64 millimetres and
its heading as the f32 it derived; a `.worldview` states whole millimetres and
whole milliradians. Both binaries then interpolate the same way — the same f32
progress, the same f64 lerp — but between different endpoints, so they land up
to half a millimetre apart, and a heading derived from the direction between two
positions follows. Rounding the RESULT on both sides narrows that to one
millimetre and cannot close it. So the master patch rounds the INPUTS instead:
right after `read_profile` and before `DesvisApp::from_profile` it walks every
`profile.time_slices[*].bot_locations`, rounds both axes of `start_pos` and of
every `steps[*].pos` with f64 `round()`, and puts every `start_heading_rad`
through the recording's own wire arithmetic — `(f64::from(rad) * 1_000.0).round()
as i32`, then that i32 divided by a thousand and cast back to f32. Nothing else
in master's profile is touched: every cell, counter, motion span and journal row
stays as master recorded it.

What the comparison therefore does NOT prove is that master and the branch agree
on master's full-precision numbers. They do not, and cannot: the ruling is that
a recording carries whole millimetres, so a viewer reading one has whole
millimetres to draw from. The harness states that premise out loud instead of
hiding it in a tolerance.

**A NaN in any `f32` field is an error**: the dump stops and names
the array, the row and the field, because a NaN draws nothing and does not even
compare equal to itself.

The manifest is one JSON object: the schema string, the recording's path, the
instants, and one entry per file naming the array, its stride, its row count
and every field with its type and byte offset.

## The arrays, field by field

Row order is **ascending entity id** for the two actor arrays and **cell id**
for the three grid arrays (row *i* is cell *i*, including ids the map does not
use, which draw nothing). The GPU's own upload order is not the file's:
`build_bot_instances` puts HTMs first so the two glyph draws stay contiguous,
and both binaries walk a hash map to place robots, so neither order is stable
enough to compare. The arrays appear in the manifest in upload order —
bots, totes, cells, reserved — with the heat arrays after them.

### `bots` — stride 28

| offset | field | type | meaning |
|---|---|---|---|
| 0 | `bot_id` | u32 | the run's robot id; not uploaded, it identifies the row |
| 4 | `world_x` | f32 | render metres, from the reported millimetres, rounded to a whole one |
| 8 | `world_y` | f32 | |
| 12 | `world_z` | f32 | floor clearance, or the rack top for a VTM standing on steel |
| 16 | `bot_type` | u32 | 0 = VTM, 1 = HTM, 2 = RTP |
| 20 | `highlight` | u32 | always 0 in a dump: nothing is selected |
| 24 | `heading_mrad` | i32 | whole milliradians, 0 along +x, turning toward +y |

### `totes` — stride 44

| offset | field | type | meaning |
|---|---|---|---|
| 0 | `tote_id` | u32 | |
| 4 | `contents_units` | u64 | physical units at the cursor |
| 12 | `world_x` | f32 | render metres, rounded to a whole millimetre like a bot's |
| 16 | `world_y` | f32 | |
| 20 | `world_z` | f32 | shelf, relay tab, VTM slab or HTM deck |
| 24 | `tote_type` | u32 | 0 = in rack, 1 = at relay or cell, 2 = on VTM, 3 = on HTM, 4 = on conveyor |
| 28 | `designation` | u32 | 1 for an ORDER tote, 0 for inventory |
| 32 | `heading_mrad` | i32 | whole milliradians: a carried tote shares its HTM's yaw |
| 36 | `pitch` | f32 | |
| 40 | `physical_type` | u32 | 0 = tote, 1 = tray, 2 = carton |

These are the rows `worldview_render::ToteInstance` holds. The buffer the GPU
actually receives is `draw_instances`' expansion of them into a shell and its
physical units, which is a pure function of these rows and carries no tote id.

### `cells` — stride 48, row *i* = cell *i*

The `CellInstance` the grid pipeline uploads, exactly as `bytemuck` would cast
it, `pad` included so the row is the GPU's own 48-byte stride. This is the
DENSE PREFIX of the instance buffer, one row per cell id, which is the part
`update_heatmap` rewrites every frame. The buffer's tail — the static underlay
the steel footprints, the equipment strips and the building's ground compose —
is built once when the grid resources are created and never rewritten, so it is
not dumped.

| offset | field | type | meaning |
|---|---|---|---|
| 0 | `min_x` | f32 | low corner of the cell's floor tile, render metres |
| 4 | `min_y` | f32 | |
| 8 | `size_x` | f32 | tile size in metres |
| 12 | `size_y` | f32 | |
| 16 | `cell_kind` | u32 | `cell_kind_to_u32` |
| 20 | `rack_levels` | u32 | |
| 24 | `heatmap_vtm` | f32 | 0 in a dump — the overlay is off in the dumped view |
| 28 | `heatmap_htm` | f32 | 0, likewise |
| 32 | `reserved` | u32 | 1 where the relay cell is held at this instant |
| 36 | `depth_bias` | f32 | which coplanar tier the ground surface draws on |
| 40 | `heading_rad` | f32 | tile rotation about its centre |
| 44 | `pad` | u32 | always 0; the stride's last word |

### `reserved` — stride 1, row *i* = cell *i*

One byte per cell, 1 where the relay cell is held at the instant, 0 otherwise.
It duplicates the `cells` column so a difference in the reservation set is
visible on its own rather than buried in the geometry.

**The two sides derive this set differently on purpose.** A `.worldview` seeds
the held set from the run's first slice and moves it through the transitions,
because master's observer re-snapshots its running set every time it CREATES a
slice — and for a slice created out of order that snapshot is from a later
instant, so master's recorded `start_reserved` does not follow from the
transitions beside it. The master patch therefore carries the set forward the
same way (`carried_reserved_cells`) instead of reading each slice's own
snapshot. Harness B compares the carried sets; the difference between a carried
set and master's re-snapshot is round 2b's deviation 4 and is not what this
harness measures.

### `slice_heat` and `heat` — stride 8, row *i* = cell *i*

| offset | field | type |
|---|---|---|
| 0 | `heatmap_vtm` | f32 |
| 4 | `heatmap_htm` | f32 |

Both are the normalised tints the shader reads: the cell's visit count divided
by `max(vtm_max, htm_max, 1)` over the whole overlay. `slice_heat` is the
overlay the app holds at the instant, `heat` the run total it opened on.

### `traffic` — stride 8, row *i* = cell *i*

| offset | field | type | meaning |
|---|---|---|---|
| 0 | `wait` | f32 | the waiting overlay's intensity, or −1 where nothing was measured |
| 4 | `speed` | f32 | the moving-speed overlay's, or −1 |

`TrafficCache::intensity` for `HtmWait` and for `HtmSpeed`, over the cumulative
prefix up to the instant. −1 is the overlay's own "no exposure here", not a
missing value. This is the only array fed by the motion section, so it is the
one that shows whether the per-move speed profiles survived the round trip.

## Master's side

Master's desvis gains the same mode from `scripts/worldview/master-dump-render.patch`,
which harness B applies to the pinned master worktree's detached HEAD — it
creates, moves and deletes no ref — builds into its own target directory (so the
pristine binary harness A uses is never overwritten), and reverts, verifying the
hand-back before it reports success. The patched module is the branch's file
with five deliberate differences: the charter says it is a patch, it reads
master's own container and builds the app the way master builds it, it states
master's recorded positions and headings at the recording's precision as
described above, it carries the reserved relay set forward, and the flag is
spelled the way master's own playback flag is spelled.

To change the patch, edit `scripts/worldview/master-dump-render.patch` itself —
its `render_dump.rs` hunk creates the file, so every line of it is a `+` line and
the hunk's count is the number of those lines. Recompute `@@ -0,0 +1,N @@` and
the `index 000000000..<blob>` from the file the hunk builds, and check it with
`git -C <pinned worktree> apply --check`, which changes nothing. The pinned
worktree is never edited by hand: harness B is the only thing that applies the
patch to it, and it reverts and verifies the hand-back. Those spellings are why the branch's
"no legacy naming left" grep gate excludes the patch and the harness script by
name; every other file in the tree is held to it.
