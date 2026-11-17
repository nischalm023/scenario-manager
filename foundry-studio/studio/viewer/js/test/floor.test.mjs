// The wasm viewer's floor-only source in a headless Chrome (spec 03 section 8,
// statement 7, the viewer's half): the made L-Shape's floor document drawn
// with no run, a station picked before the cell it stands on, outlines that
// leave the drawn floor as it was, the named frame's axes in the corner, the
// camera kept across a second floor, and the system colours handed over under
// emulated forced colours.
//
// It compiles its own fixture with the crate's `floor_of` example, serves this
// package over HTTP (a module and its wasm will not load from a file), and
// drives Chrome over its debugging protocol with scripts/worldview's helper.
// It needs `npm run build:wasm` to have run; `npm test` builds the TypeScript.
// It draws through WebGPU, or through the WebGL2 fallback with
// CRUCIBLE_VIEWER_BACKEND=webgl.

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, extname, join, resolve, sep } from 'node:path';
import { after, before, test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { openChrome } from '../../../../scripts/worldview/headless_chrome.mjs';

const pkg = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repo = resolve(pkg, '..', '..', '..');
const world = join(repo, 'studio/viewer/tests/fixtures/l_shape/world.json');
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm' };
const BACKEND = process.env.CRUCIBLE_VIEWER_BACKEND ?? 'webgpu';

/** The made site's stations, by the component the world document gives them, and their lanes. */
const entities = JSON.parse(readFileSync(world, 'utf8')).entities;
const stations = entities.filter((entity) => 'station' in entity.components).map((entity) => entity.id);
const lanes = entities.filter((entity) => 'queue' in entity.components).map((entity) => entity.id);
/** Whether `keys` holds `station` with only lanes, which stand on fewer cells, before it. */
const stationLeads = (keys, station) => keys.includes(station) && keys.slice(0, keys.indexOf(station)).every((key) => lanes.includes(key));

let scratch;
let server;
let chrome;

/** Evaluate in the page, where `harness` holds the viewer and its helpers. */
const page = (expression) => chrome.evaluate(`(async () => { const { viewer, frames } = window.harness; ${expression} })()`);

before(async () => {
  scratch = await mkdtemp(join(tmpdir(), 'crucible-viewer-floor-'));
  const floor = join(scratch, 'l-shape.floor');
  // The compiler's warnings about other crates are noise here; its output is
  // shown only when the fixture could not be made.
  try {
    execFileSync('cargo', ['run', '-q', '-p', 'crucible-viewer', '--example', 'floor_of', '--', world, floor], {
      cwd: repo,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
  } catch (failure) {
    throw new Error(`the floor fixture could not be compiled:\n${failure.stderr}`);
  }
  server = createServer(async (request, response) => {
    const path = decodeURIComponent(new URL(request.url, 'http://here').pathname);
    const file = path === '/l-shape.floor' ? floor : resolve(pkg, `.${path}`);
    const body = file === floor || file.startsWith(pkg + sep) ? await readFile(file).catch(() => null) : null;
    if (body === null) {
      response.writeHead(404).end();
      return;
    }
    response.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream' });
    response.end(body);
  });
  await new Promise((listening) => server.listen(0, '127.0.0.1', listening));
  chrome = await openChrome({ width: 1000, height: 800, quiet: true, webgpu: BACKEND === 'webgpu' });
  await chrome.navigate(`http://127.0.0.1:${server.address().port}/test/floor.html?floor=/l-shape.floor`);
  await chrome.until('window.harness', 'the viewer to open and take the floor');
  const failed = await chrome.evaluate('window.harness.failed ?? null');
  if (failed) throw new Error(`the page could not draw the floor: ${failed}`);
  assert.equal(await chrome.evaluate('window.harness.viewer.backend()'), BACKEND, 'the backend asked for');
});

after(async () => {
  await chrome?.close();
  server?.close();
  if (scratch) await rm(scratch, { recursive: true, force: true });
});

test('a floor with no run draws its cells and no movers', async () => {
  const { drawn } = await page('return frames(3);');
  assert.equal(drawn[0], 0, 'no robots');
  assert.equal(drawn[1], 0, 'no containers');
  assert.ok(drawn[2] > 1000, `the made L-Shape's cells, and ${drawn[2]} were submitted`);
});

test("a station's cell answers the station before the cell", async () => {
  assert.equal(stations.length, 6);
  for (const station of stations) {
    const said = await page(`
      const framed = viewer.frame_entities([${JSON.stringify(station)}]);
      await frames(2);
      const { clientWidth: width, clientHeight: height } = window.harness.stage;
      return { framed, centre: viewer.centre_entities(), picked: viewer.pick_entities(width / 2, height / 2) };`);
    assert.equal(said.framed, 1, `${station} stands on cells`);
    assert.ok(stationLeads(said.centre, station), `the station first, after only its lane: ${said.centre}`);
    assert.ok(said.centre.length >= 2 && said.centre.at(-1) !== station, `and the cell itself last: ${said.centre}`);
    assert.deepEqual(said.picked, said.centre, 'the pointer at the centre picks what the centre holds');
  }
});

test('outlines leave the drawn cell count as it was', async () => {
  const said = await page(`
    const plain = await frames(2);
    const counts = Array.from(viewer.set_highlights([{ marks: ${JSON.stringify([stations[0], stations[1], 'no-such-entity'])} }]));
    const selected = viewer.set_selection([${JSON.stringify(stations[2])}]);
    const marked = await frames(2);
    viewer.set_highlights([]);
    viewer.set_selection([]);
    return { plain, counts, selected, marked, cleared: await frames(2) };`);
  assert.deepEqual(said.counts, [2], 'two of the three keys stand on cells');
  assert.equal(said.selected, 1, 'the one key selected stands on cells');
  assert.deepEqual(said.marked.drawn, said.plain.drawn, 'the same cells drawn with outlines over them');
  assert.ok(said.marked.outlined[0] > 0 && said.marked.outlined[1] > 0, `outlined ${said.marked.outlined}`);
  assert.deepEqual(said.cleared.outlined, [0, 0, 0]);
});

test("the named frame's axes are drawn in the corner", async () => {
  const said = await page(`
    viewer.set_axes(null);
    await frames(2);
    const before = window.harness.inkInCorner(120);
    viewer.set_axes({ name: "shell", x: [1, 0], y: [0, 1] });
    await frames(2);
    return { before, after: window.harness.inkInCorner(120) };`);
  assert.equal(said.before, 0, 'nothing in the corner before a frame is named');
  assert.ok(said.after > 50, `the axes and the name are drawn: ${said.after} pixels`);
});

test('a second floor keeps the camera where it stands', async () => {
  const said = await page(`
    viewer.pan_by(40, 0);
    viewer.zoom_by(1);
    await frames(1);
    const moved = Array.from(viewer.camera());
    const floor = await (await fetch("/l-shape.floor")).arrayBuffer();
    viewer.set_floor(new Uint8Array(floor));
    await frames(1);
    return { moved, kept: Array.from(viewer.camera()) };`);
  assert.deepEqual(said.kept, said.moved);
});

test('under emulated forced colours the palette handed over is the system colours', async () => {
  const system = `(keyword) => {
    const probe = document.createElement("span");
    probe.style.setProperty("forced-color-adjust", "none");
    probe.style.color = keyword;
    document.body.append(probe);
    const resolved = getComputedStyle(probe).color;
    probe.remove();
    return resolved;
  }`;
  const tokens = await page('const palette = window.harness.palette(); viewer.set_palette(palette); return palette;');
  assert.equal(tokens.agent, 'rgb(105, 65, 198)', "the page's --color-agent");
  await chrome.call('Emulation.setEmulatedMedia', { features: [{ name: 'forced-colors', value: 'active' }] });
  try {
    const said = await page(`
      const system = ${system};
      const palette = window.harness.palette();
      viewer.set_palette(palette);
      return {
        forced: matchMedia("(forced-colors: active)").matches,
        palette,
        held: viewer.palette(),
        expected: { floor: system("Canvas"), lines: system("CanvasText"), agent: system("Highlight"), selection: system("Highlight") },
      };`);
    assert.equal(said.forced, true);
    assert.deepEqual(said.palette, said.expected, 'Canvas, CanvasText and Highlight, not the tokens');
    assert.deepEqual(said.held, said.palette, 'and the viewer holds them');
  } finally {
    await chrome.call('Emulation.setEmulatedMedia', { features: [] });
    await page('viewer.set_palette(window.harness.palette()); return null;');
  }
});

/** The stage back at the page's own size, fitted. */
const restoreStage = () => page('Object.assign(window.harness.stage.style, { width: "", height: "", display: "" }); return window.harness.fit();');

test('a resize and a change of device pixel ratio keep the outline on the cell it outlines', async () => {
  // The cell the first station's presentation stands on, as the roll names it: the last key over the centre.
  const cell = await page(`
    viewer.set_highlights([]);
    viewer.frame_entities([${JSON.stringify(stations[0])}]);
    await frames(1);
    const cell = viewer.centre_entities().at(-1);
    viewer.set_selection([cell]);
    return cell;`);
  const plain = await page('return (await frames(1)).drawn;');
  try {
    for (const [ratio, width, height] of [
      [2, 800, 600],
      [2, 500, 300],
      [1, 640, 480],
    ]) {
      await chrome.call('Emulation.setDeviceMetricsOverride', { width: 1000, height: 800, deviceScaleFactor: ratio, mobile: false });
      const said = await page(`
        Object.assign(window.harness.stage.style, { width: "${width}px", height: "${height}px" });
        const fitted = window.harness.fit();
        viewer.frame_entities([${JSON.stringify(cell)}]);
        const { drawn } = await frames(2);
        // The selection's solid ink, #0a4a5e, on the lines canvas, in device pixels.
        const { ink } = window.harness;
        let box = null;
        for (let y = 0; y < ink.height; y += 1) {
          for (let x = 0; x < ink.width; x += 1) {
            const at = (y * ink.width + x) * 4;
            const near = Math.abs(ink.data[at] - 10) + Math.abs(ink.data[at + 1] - 74) + Math.abs(ink.data[at + 2] - 94);
            if (ink.data[at + 3] > 200 && near < 24) {
              box = box ? [Math.min(box[0], x), Math.min(box[1], y), Math.max(box[2], x), Math.max(box[3], y)] : [x, y, x, y];
            }
          }
        }
        const middle = box && [(box[0] + box[2]) / 2, (box[1] + box[3]) / 2];
        const picked = middle ? viewer.pick_entities(middle[0] / fitted.ratio, middle[1] / fitted.ratio) : [];
        return { fitted, drawn, box, middle, picked };`);
      const where = `at ${width}x${height} CSS px and ${ratio} device px each`;
      assert.equal(said.fitted.ratio, ratio, where);
      assert.deepEqual(said.fitted.lines, said.fitted.device, `the lines canvas sized with the GPU's ${where}`);
      assert.deepEqual(said.drawn, plain, `every cell drawn ${where}`);
      assert.ok(said.box, `the selection is outlined ${where}`);
      const [cx, cy] = said.fitted.device.map((side) => side / 2);
      assert.ok(Math.abs(said.middle[0] - cx) <= 2 * ratio && Math.abs(said.middle[1] - cy) <= 2 * ratio, `the outline around the framed cell, mid-view ${where}: ${said.box}`);
      assert.ok(said.picked.includes(cell), `the outline's middle picks the cell it outlines ${where}: ${said.picked}`);
    }
  } finally {
    await chrome.call('Emulation.clearDeviceMetricsOverride');
    await restoreStage();
    await page('viewer.set_selection([]); return null;');
  }
});

test('a canvas hidden at zero size and shown again draws as before, its camera kept whole', async () => {
  try {
    const said = await page(`
      const { stage } = window.harness;
      viewer.frame_entities([${JSON.stringify(stations[1])}]);
      const shown = await frames(2);
      stage.style.display = "none";
      const hidden = window.harness.fit();
      const whileHidden = await frames(2);
      // What a page may still ask of a hidden stage: highlights framed, keys pressed.
      const framed = viewer.frame_entities([${JSON.stringify(stations[2])}]);
      viewer.pan_by(40, 0);
      viewer.pan_by(-40, 0);
      viewer.zoom_by(1);
      viewer.zoom_by(-1);
      const cameraHidden = Array.from(viewer.camera());
      const centreHidden = Array.from(viewer.centre_mm());
      stage.style.display = "";
      window.harness.fit();
      const drawnAgain = await frames(2);
      return { shown, hidden, whileHidden, framed, cameraHidden, centreHidden, drawnAgain, centre: viewer.centre_entities() };`);
    assert.deepEqual(said.hidden.device, [0, 0], 'the stage hidden');
    assert.deepEqual(said.whileHidden.drawn, [0, 0, 0], 'nothing drawn on a canvas of no size');
    assert.equal(said.framed, 1);
    assert.ok(said.cameraHidden.every(Number.isFinite), `a camera moved while hidden is still a camera: ${said.cameraHidden}`);
    assert.equal(said.centreHidden.length, 2, 'and the centre of the view still meets the floor');
    assert.deepEqual(said.drawnAgain.drawn, said.shown.drawn, 'shown again, every cell drawn as before');
    assert.ok(stationLeads(said.centre, stations[2]), `the station framed while hidden is mid-view once shown: ${said.centre}`);
  } finally {
    await restoreStage();
  }
});

// Ruling D119: a page chooses the projection; orthographic looks straight down
// and nothing tilts it, a shifted drag included; orbit tilts the camera into
// perspective about the same centre, a drag and orbit_by turning it within its
// reach; back to orthographic is straight down and north up again. The camera
// reads [x, y, z, distance, azimuth, elevation].
test('the projection is orthographic or orbit, as the page chooses, about one centre', async () => {
  const said = await page(`
    const camera = () => Array.from(viewer.camera());
    const drag = async (dx, dy, shift) => {
      viewer.pointer_down(300, 300, false);
      viewer.pointer_move(300 + dx, 300 + dy, shift);
      viewer.pointer_up();
      await frames(2);
    };
    const unchosen = viewer.projection() ?? null;
    viewer.set_projection("orthographic");
    await drag(40, -40, true);
    const ortho = { projection: viewer.projection(), camera: camera() };
    viewer.set_projection("orbit");
    await frames(2);
    const orbit = { projection: viewer.projection(), camera: camera(), drawn: (await frames(1)).drawn };
    viewer.orbit_by(1, 0);
    viewer.orbit_by(0, 100);
    await frames(1);
    const turned = camera();
    await drag(60, -60, false);
    const dragged = camera();
    viewer.set_projection("orthographic");
    await frames(2);
    const back = { projection: viewer.projection(), camera: camera() };
    let refused = null;
    try { viewer.set_projection("isometric"); } catch (failure) { refused = String(failure); }
    return { unchosen, ortho, orbit, turned, dragged, back, refused };`);
  const straightDown = Math.PI / 2;
  const close = (a, b) => Math.abs(a - b) < 1e-4;
  assert.equal(said.unchosen, null, 'no projection until the page chooses');
  assert.equal(said.ortho.projection, 'orthographic');
  assert.ok(close(said.ortho.camera[5], straightDown), `straight down after a shifted drag: ${said.ortho.camera}`);
  assert.equal(said.orbit.projection, 'orbit');
  assert.ok(said.orbit.camera[5] < 1.2, `tilted into perspective: ${said.orbit.camera}`);
  assert.ok(close(said.orbit.camera[0], said.ortho.camera[0]) && close(said.orbit.camera[1], said.ortho.camera[1]), 'about the same centre');
  assert.ok(close(said.orbit.camera[3], said.ortho.camera[3]), 'at the same distance');
  assert.ok(said.orbit.drawn[2] > 1000, 'the floor still drawn');
  assert.ok(!close(said.turned[4], said.orbit.camera[4]), 'orbit_by turns');
  assert.ok(said.turned[5] < 1.2, `and tilts no further than the orbit reaches: ${said.turned}`);
  assert.ok(!close(said.dragged[5], said.turned[5]) || !close(said.dragged[4], said.turned[4]), 'a drag turns it');
  assert.equal(said.back.projection, 'orthographic');
  assert.ok(close(said.back.camera[5], straightDown) && close(said.back.camera[4], -Math.PI / 2), 'straight down, north up');
  assert.match(said.refused ?? '', /isometric/);
});

// Ruling D121: a floor drawn into a picture, the GPU canvas with the lines over
// it, as the Template list's cards take theirs: a PNG data URL as large as the
// canvas, holding the floor's own pixels, not one flat colour.
test('a snapshot is a PNG of the drawn floor, the lines composited over it', async () => {
  const said = await page(`
    const { snapshotFloor } = await import("../dist/index.js");
    viewer.set_projection("orthographic");
    viewer.reset_camera();
    viewer.set_axes({ name: "shell", x: [1, 0], y: [0, 1] });
    const url = snapshotFloor(viewer, window.harness.stage.querySelector(".gpu"), window.harness.stage.querySelector(".lines"));
    const image = new Image();
    image.src = url;
    await image.decode();
    const copy = document.createElement("canvas");
    [copy.width, copy.height] = [image.width, image.height];
    const context = copy.getContext("2d");
    context.drawImage(image, 0, 0);
    const pixels = context.getImageData(0, 0, copy.width, copy.height).data;
    const colours = new Set();
    for (let at = 0; at < pixels.length; at += 4 * 97) colours.add(pixels[at] + "," + pixels[at + 1] + "," + pixels[at + 2]);
    const gpu = window.harness.stage.querySelector(".gpu");
    viewer.set_axes(null);
    return { head: url.slice(0, 22), size: [image.width, image.height], canvas: [gpu.width, gpu.height], colours: colours.size };`);
  assert.equal(said.head, 'data:image/png;base64,');
  assert.deepEqual(said.size, said.canvas, 'as large as the canvas');
  assert.ok(said.colours > 3, `the floor's own pixels, ${said.colours} colours sampled`);
});

// D121's pictures are copies of a frame the viewer presented of the floor it
// was given: when it presents none (told it has no size here, its canvas still
// holding the last frame's pixels, as a surface the browser would not hand over
// also leaves it), the snapshot is refused in words, never a copy of the floor
// drawn before or of a blank (reviews-impl/astra-t14.md, the tenth P1).
test('a snapshot of a frame the viewer did not present is refused, not copied', async () => {
  try {
    const said = await page(`
      const { snapshotFloor } = await import("../dist/index.js");
      const { stage } = window.harness;
      await frames(1);
      viewer.resize(0, 0, window.devicePixelRatio || 1);
      try {
        return { copied: snapshotFloor(viewer, stage.querySelector(".gpu"), stage.querySelector(".lines")).slice(0, 22) };
      } catch (refused) {
        return { refused: String(refused) };
      }`);
    assert.equal(said.copied, undefined, `nothing copied: ${said.copied}`);
    assert.match(said.refused, /presented no frame/);
  } finally {
    await restoreStage();
  }
});
