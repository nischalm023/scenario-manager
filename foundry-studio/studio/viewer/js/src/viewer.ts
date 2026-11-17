// The wasm viewer, ready to use.
//
// wasm-bindgen writes the class and its declarations; this adds the one thing
// it cannot, which is fetching the module itself. A page has to do that once
// before any `Viewer` exists, and getting it wrong shows up as a constructor
// that throws about a null pointer — so `openViewer` does both steps in the
// one call a shell actually wants, and remembers that the first is done.
//
// A page that shows a site rather than a run (Agent Studio's Map panel, spec 03
// section 3.6) also lays a 2D canvas over the viewer's for the lines it draws
// — outlines, the frame's axes, the centre cross — and hands it the colours
// those take from the page: `openFloorViewer` and `readPalette`. It chooses
// the camera's `Projection` too (ruling D119): `set_projection("orthographic")`,
// straight down, or `"orbit"`, which `orbit_by` turns and tilts by keys. `snapshotFloor` copies a drawn floor
// into a picture, for a page that shows floors as images.

import init, { Viewer } from "./wasm/crucible_viewer.js";
import type { FloorPalette } from "./wasm/crucible_viewer.js";

export { Viewer } from "./wasm/crucible_viewer.js";
export type { DiffSets, FloorPalette, FrameAxes, HighlightGroup, Projection } from "./wasm/crucible_viewer.js";

/** The one load of the wasm module, shared by every canvas on the page. */
let loading: Promise<unknown> | undefined;

/**
 * Load the wasm module, if this page has not already, and open a viewer on
 * `canvas`.
 *
 * `wasmUrl` says where the module is when the page does not serve it beside
 * this file — which is what a bundler that fingerprints assets does.
 */
export async function openViewer(canvas: HTMLCanvasElement, wasmUrl?: string | URL): Promise<Viewer> {
  loading ??= wasmUrl === undefined ? init() : init({ module_or_path: wasmUrl });
  await loading;
  return await Viewer.new(canvas);
}

/**
 * Open a viewer on `canvas` that strokes its lines on `lines`, a 2D canvas the
 * page lays exactly over it (`pointer-events: none`, `aria-hidden`). The
 * viewer sizes `lines` with its own canvas from then on.
 */
export async function openFloorViewer(
  canvas: HTMLCanvasElement,
  lines: HTMLCanvasElement,
  wasmUrl?: string | URL,
): Promise<Viewer> {
  const viewer = await openViewer(canvas, wasmUrl);
  viewer.attach_lines(lines);
  return viewer;
}

/** Spec 03 section 5's tokens, one per colour the viewer chooses. */
const TOKENS: Readonly<Record<keyof FloorPalette, string>> = {
  floor: "--color-map-floor",
  lines: "--color-text-primary",
  agent: "--color-agent",
  selection: "--color-selection",
};

/**
 * The system colours each role takes under `(forced-colors: active)`, which a
 * canvas never receives by itself (reviews/accessibility.md finding 4): the
 * agent's and the selection's outlines share one, told apart by dash and width.
 */
const SYSTEM: Readonly<Record<keyof FloorPalette, string>> = {
  floor: "Canvas",
  lines: "CanvasText",
  agent: "Highlight",
  selection: "Highlight",
};

/**
 * The palette for `Viewer.set_palette`, as `element` sees it now: the page's
 * tokens, or the system colours while forced colours are active. Each comes
 * back as its computed `rgb(…)`, which the viewer reads. A page calls this when
 * it mounts the viewer and again when the theme or the forced-colours mode
 * changes; a token the page does not define is an error naming it.
 */
export function readPalette(element: Element): FloorPalette {
  const forced = window.matchMedia("(forced-colors: active)").matches;
  const declared = getComputedStyle(element);
  const probe = element.ownerDocument.createElement("span");
  probe.style.setProperty("forced-color-adjust", "none");
  element.appendChild(probe);
  try {
    const resolve = (role: keyof FloorPalette): string => {
      if (!forced && declared.getPropertyValue(TOKENS[role]).trim() === "") {
        throw new Error(`readPalette: the page defines no ${TOKENS[role]}, the viewer's ${role} colour`);
      }
      probe.style.color = forced ? SYSTEM[role] : `var(${TOKENS[role]})`;
      return getComputedStyle(probe).color;
    };
    return { floor: resolve("floor"), lines: resolve("lines"), agent: resolve("agent"), selection: resolve("selection") };
  } finally {
    probe.remove();
  }
}

/**
 * A picture of the floor `viewer` holds, as a PNG data URL: one frame drawn now and copied, within the same task, from
 * `gpu` with `lines`, the 2D canvas laid over it, composited on top. A page that shows many floors as pictures (the
 * Template list's cards, ruling D121) draws each through one hidden viewer and keeps the pictures, rather than holding a
 * GPU canvas per card. The copy is made before the browser presents the frame, which is when a WebGL canvas may clear,
 * and only of a frame this call presented: one the viewer did not present (no size, no surface handed over, no floor)
 * is refused in words rather than copied as the floor drawn before or a blank.
 */
export function snapshotFloor(viewer: Viewer, gpu: HTMLCanvasElement, lines: HTMLCanvasElement): string {
  if (!viewer.frame(0)) throw new Error("snapshotFloor: the viewer presented no frame of its floor, so there is no picture to copy");
  const picture = gpu.ownerDocument.createElement("canvas");
  [picture.width, picture.height] = [gpu.width, gpu.height];
  const context = picture.getContext("2d");
  if (context === null) throw new Error("snapshotFloor: a 2D canvas gives no context to composite on");
  context.drawImage(gpu, 0, 0);
  context.drawImage(lines, 0, 0, gpu.width, gpu.height);
  return picture.toDataURL("image/png");
}

/** What a run's frames arrive as: one worldview frame per binary message. */
export type Frame = Uint8Array;
