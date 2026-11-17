// Spec 03 section 5's keys on the stage and ruling D119's projection, as what each key does to the camera or the
// selection: arrows pan a tenth of the view, plus and minus zoom, 0 fits the floor, F frames the highlights, Enter
// selects what is under the centre cross, Escape clears the selection, O switches between orthographic and orbit,
// and in orbit Shift with the arrows turns and tilts; the camera's moves made, for every stage a viewer draws. Also
// the sentences the stage's lines say, and where a camera is kept between visits of one panel to one place (spec 03
// section 3.6: `sessionStorage`, never the address).

import type { Projection, Viewer } from 'crucible-viewer';
import { readStored, writeStored } from '../shell/storage.ts';

export type StageAction =
  | { kind: 'pan'; right: number; down: number }
  | { kind: 'zoom'; steps: number }
  | { kind: 'turn'; turn: number; tilt: number }
  | { kind: 'fit' }
  | { kind: 'frame' }
  | { kind: 'select' }
  | { kind: 'clear' }
  | { kind: 'projection' };

/** What `key` (a `KeyboardEvent.key`) does on the stage, or null for a key the stage leaves to the page. */
export function stageAction(key: string, shift: boolean, projection: Projection): StageAction | null {
  const arrows: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
  const arrow = arrows[key];
  if (arrow !== undefined) {
    if (shift && projection === 'orbit') return { kind: 'turn', turn: arrow[0], tilt: arrow[1] === 0 ? 0 : -arrow[1] };
    return { kind: 'pan', right: arrow[0], down: arrow[1] };
  }
  switch (key) {
    case '+': case '=': return { kind: 'zoom', steps: 1 };
    case '-': case '_': return { kind: 'zoom', steps: -1 };
    case '0': return { kind: 'fit' };
    case 'f': case 'F': return { kind: 'frame' };
    case 'Enter': return { kind: 'select' };
    case 'Escape': return { kind: 'clear' };
    case 'o': case 'O': return { kind: 'projection' };
    default: return null;
  }
}

/** Move `viewer`'s camera as `action` says on `stage`: a pan goes a tenth of the view, and a fit keeps an orbit's
 * angles. False for an action that is not the camera's, which the stage's page answers. */
export function moveCamera(viewer: Viewer, stage: HTMLElement, action: StageAction, projection: Projection): boolean {
  switch (action.kind) {
    case 'pan': viewer.pan_by((action.right * stage.clientWidth) / 10, (action.down * stage.clientHeight) / 10); return true;
    case 'zoom': viewer.zoom_by(action.steps); return true;
    case 'turn': viewer.orbit_by(action.turn, action.tilt); return true;
    case 'fit': {
      const [, , , , azimuth, elevation] = Array.from(viewer.camera());
      viewer.reset_camera();
      if (projection === 'orbit') {
        const [x, y, z, distance] = Array.from(viewer.camera());
        viewer.set_camera(new Float32Array([x, y, z, distance, azimuth, elevation]));
      }
      return true;
    }
    default: return false;
  }
}

const MOVE_KEYS = 'arrows pan, plus and minus zoom, 0 fits the floor';
const PROJECTION_KEYS = 'O switches between orthographic and orbit; in orbit, Shift with the arrows turns and tilts';

/** The keys line every mount's stage shows (`WorldviewMount`), which its `aria-describedby` names. */
export const KEYS_LINE = `Keys on the map: ${MOVE_KEYS}, F frames the highlights, Enter selects what is under the centre `
  + `cross, Escape clears the selection, ${PROJECTION_KEYS}.`;

/** The keys line of a stage with nothing to frame or select, a recording played back: the camera's keys alone. */
export const CAMERA_KEYS_LINE = `Keys on the map: ${MOVE_KEYS}, ${PROJECTION_KEYS}.`;

/** "x 12.4 m, y 30.5 m" for a point in whole millimetres. */
export function positionWords([x, y]: readonly [number, number]): string {
  const metres = (mm: number) => (mm / 1000).toFixed(1);
  return `x ${metres(x)} m, y ${metres(y)} m`;
}

type Axes = { name: string; x: readonly [number, number]; y: readonly [number, number] };

/** "the shell frame: x to the right, y up the drawing" for a frame's axes as the stage draws them. */
export function axesWords(axes: Axes): string {
  const way = ([dx, dy]: readonly [number, number]) =>
    dx > 0 ? 'to the right' : dx < 0 ? 'to the left' : dy > 0 ? 'up the drawing' : 'down the drawing';
  return `the ${axes.name} frame: x ${way(axes.x)}, y ${way(axes.y)}`;
}

/** The text twin's sentence for the axes drawn in the stage's corner (spec 03 section 3.6, spec 05 section 6). */
export const axesSentence = (axes: Axes): string => `Axes of ${axesWords(axes)}.`;

/** A camera kept between visits: `Viewer.camera()`'s six numbers and the projection it was looked at in. */
export interface KeptCamera {
  camera: number[];
  projection: Projection;
}

const keptKey = (panel: string, place: string) => `studio.camera.v1:${panel}:${place}`;

export function keptCamera(panel: string, place: string): KeptCamera | null {
  const text = readStored('session', keptKey(panel, place));
  if (text === null) return null;
  try {
    const read = JSON.parse(text) as Partial<KeptCamera>;
    const fine = Array.isArray(read.camera) && read.camera.length === 6 && read.camera.every(Number.isFinite);
    return fine && (read.projection === 'orthographic' || read.projection === 'orbit')
      ? { camera: read.camera as number[], projection: read.projection }
      : null;
  } catch {
    return null;
  }
}

export function keepCamera(panel: string, place: string, kept: KeptCamera): void {
  writeStored('session', keptKey(panel, place), JSON.stringify(kept));
}
