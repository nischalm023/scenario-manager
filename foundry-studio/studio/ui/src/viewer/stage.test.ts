// The stage's keys (spec 03 section 5) and ruling D119's projection key, the camera's moves they make, its words, and
// the camera it keeps between visits (spec 03 section 3.6).

import type { Viewer } from 'crucible-viewer';
import { beforeEach, describe, expect, it } from 'vitest';
import { axesWords, keepCamera, keptCamera, moveCamera, positionWords, stageAction } from './stage.ts';

describe('the stage keys', () => {
  it('pans with the arrows, and in orbit turns and tilts with Shift and the arrows', () => {
    expect(stageAction('ArrowRight', false, 'orthographic')).toEqual({ kind: 'pan', right: 1, down: 0 });
    expect(stageAction('ArrowUp', true, 'orthographic')).toEqual({ kind: 'pan', right: 0, down: -1 });
    expect(stageAction('ArrowLeft', true, 'orbit')).toEqual({ kind: 'turn', turn: -1, tilt: 0 });
    expect(stageAction('ArrowUp', true, 'orbit')).toEqual({ kind: 'turn', turn: 0, tilt: 1 });
    expect(stageAction('ArrowDown', false, 'orbit')).toEqual({ kind: 'pan', right: 0, down: 1 });
  });

  it('zooms, fits, frames, selects, clears and switches the projection', () => {
    expect(stageAction('+', false, 'orthographic')).toEqual({ kind: 'zoom', steps: 1 });
    expect(stageAction('=', false, 'orthographic')).toEqual({ kind: 'zoom', steps: 1 });
    expect(stageAction('-', false, 'orthographic')).toEqual({ kind: 'zoom', steps: -1 });
    expect(stageAction('0', false, 'orbit')).toEqual({ kind: 'fit' });
    expect(stageAction('F', true, 'orthographic')).toEqual({ kind: 'frame' });
    expect(stageAction('Enter', false, 'orthographic')).toEqual({ kind: 'select' });
    expect(stageAction('Escape', false, 'orthographic')).toEqual({ kind: 'clear' });
    expect(stageAction('o', false, 'orthographic')).toEqual({ kind: 'projection' });
    expect(stageAction('Tab', false, 'orthographic')).toBeNull();
    expect(stageAction('x', false, 'orthographic')).toBeNull();
  });
});

describe("the camera's moves", () => {
  it("pans a tenth of the stage, fits keeping an orbit's angles, and leaves the selection's keys to the page", () => {
    const said: string[] = [];
    let camera = [1, 2, 3, 4, 0.5, 0.25];
    const viewer = {
      pan_by: (right: number, down: number) => said.push(`pan ${right} ${down}`),
      camera: () => new Float32Array(camera),
      reset_camera: () => (camera = [9, 9, 9, 40, 0, 1]),
      set_camera: (to: Float32Array) => (camera = Array.from(to)),
    } as unknown as Viewer;
    const stage = { clientWidth: 800, clientHeight: 600 } as HTMLElement;
    expect(moveCamera(viewer, stage, { kind: 'pan', right: 1, down: -1 }, 'orthographic')).toBe(true);
    expect(said).toEqual(['pan 80 -60']);
    moveCamera(viewer, stage, { kind: 'fit' }, 'orbit');
    expect(camera).toEqual([9, 9, 9, 40, 0.5, 0.25]);
    expect(moveCamera(viewer, stage, { kind: 'select' }, 'orbit')).toBe(false);
  });
});

describe("the stage's words", () => {
  it('says a position in metres to a tenth', () => {
    expect(positionWords([12_449, 30_500])).toBe('x 12.4 m, y 30.5 m');
  });

  it("says which way a frame's axes point on the drawing", () => {
    expect(axesWords({ name: 'shell', x: [1, 0], y: [0, 1] })).toBe('the shell frame: x to the right, y up the drawing');
    expect(axesWords({ name: 'shell', x: [-1, 0], y: [0, -1] })).toBe('the shell frame: x to the left, y down the drawing');
  });
});

describe('the camera kept between visits', () => {
  beforeEach(() => sessionStorage.clear());

  it('reads back as written, per panel and place', () => {
    const kept = { camera: [1, 2, 0, 30, -1.57, 0.75], projection: 'orbit' as const };
    keepCamera('map', 'scope:customer/demo/site/lshape-6/map@main', kept);
    expect(keptCamera('map', 'scope:customer/demo/site/lshape-6/map@main')).toEqual(kept);
    expect(keptCamera('template', 'scope:customer/demo/site/lshape-6/map@main')).toBeNull();
  });

  it('reads nothing from an entry it did not write', () => {
    sessionStorage.setItem('studio.camera.v1:map:here', '{"camera":[1,2],"projection":"orbit"}');
    expect(keptCamera('map', 'here')).toBeNull();
    sessionStorage.setItem('studio.camera.v1:map:here', 'not json');
    expect(keptCamera('map', 'here')).toBeNull();
  });
});
