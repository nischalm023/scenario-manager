// Ruling D121, one renderer: the pictures of floors a page shows as images (the Template list's cards, one template's
// default evaluation) are drawn by the same wasm viewer every stage draws through, never by a second renderer. One
// hidden viewer per page draws each floor document in turn, straight down and fitted, and `snapshotFloor` copies it
// into a PNG data URL, kept in the page by the floor's key and the palette read when its turn came, so a card scrolled
// back into view or a second panel naming the same floor in the same colours draws nothing again, and a theme change
// draws it anew (FloorSnapshot follows the palette). Each picture is measured as `floor-snapshot`.

import type { Viewer } from 'crucible-viewer';
import { readFloor } from './floorRead.ts';

type ViewerModule = typeof import('crucible-viewer');

/** The hidden viewer's size in CSS pixels: the cards' 3 by 1. */
const WIDTH = 600;
const HEIGHT = 200;

interface Painter {
  module: ViewerModule;
  viewer: Viewer;
  gpu: HTMLCanvasElement;
  lines: HTMLCanvasElement;
}

let painter: Promise<Painter> | null = null;
/** Pictures are drawn one at a time, each after the last, on the one viewer. */
let turn: Promise<unknown> = Promise.resolve();
/** The pictures drawn, by floor key and the whole palette each was drawn in. */
const pictures = new Map<string, string>();

/** The hidden viewer, opened on first use: two canvases off screen, laid as a stage lays them, and never shown. */
function painterNow(): Promise<Painter> {
  painter ??= (async () => {
    const module = await import('crucible-viewer');
    const holder = document.createElement('div');
    holder.setAttribute('aria-hidden', 'true');
    holder.style.cssText = `position: fixed; left: -${WIDTH * 4}px; top: 0; width: ${WIDTH}px; height: ${HEIGHT}px; pointer-events: none;`;
    const [gpu, lines] = [document.createElement('canvas'), document.createElement('canvas')];
    for (const canvas of [gpu, lines]) {
      canvas.style.cssText = 'position: absolute; inset: 0; width: 100%; height: 100%;';
      holder.append(canvas);
    }
    // First in the body, since the announcer stays its last element (spec 02 section 3.4).
    document.body.prepend(holder);
    const ratio = window.devicePixelRatio || 1;
    [gpu.width, gpu.height] = [Math.round(WIDTH * ratio), Math.round(HEIGHT * ratio)];
    const viewer = await module.openFloorViewer(gpu, lines);
    viewer.resize(gpu.width, gpu.height, ratio);
    viewer.set_projection('orthographic');
    return { module, viewer, gpu, lines };
  })();
  return painter;
}

/** The picture of the floor document under `key`, in the colours `element` takes when its turn comes. */
export function floorPicture(key: string, element: Element): Promise<string> {
  const picture = turn.then(async () => {
    const started = performance.now();
    const { module, viewer, gpu, lines } = await painterNow();
    // Read once, after the queue: the palette a picture is drawn in is the palette it is kept under.
    const palette = module.readPalette(element);
    const kept = `${key}\n${JSON.stringify(palette)}`;
    const known = pictures.get(kept);
    if (known !== undefined) return known;
    const read = await readFloor(key);
    if (!read.ok) throw new Error(read.failed);
    viewer.set_floor(read.bytes);
    viewer.set_palette(palette);
    viewer.reset_camera();
    const url = module.snapshotFloor(viewer, gpu, lines);
    performance.measure('floor-snapshot', { start: started, end: performance.now(), detail: key });
    pictures.set(kept, url);
    return url;
  });
  turn = picture.catch(() => undefined);
  return picture;
}
