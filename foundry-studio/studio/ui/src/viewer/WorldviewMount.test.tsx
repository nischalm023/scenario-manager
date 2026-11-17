// Spec 03 section 3.6's mount on what the evaluation-latency chore's measurement reads of it, as Astra's review of the
// chore asked (design/reviews-impl/astra-eval.md, with the lead's dispositions of 2026-10-20): a floor's way to the
// screen is marked only in a measurement's session, and `floor:painted` only once a frame presenting it was drawn. A
// canvas draws nothing here, so the viewer is a stand-in that answers each frame as the test says, and the test runs
// the animation frames itself.

import { act, cleanup, render, waitFor } from '@testing-library/react';
import { StrictMode } from 'react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { MEASURING } from '../shell/marks.ts';
import { WorldviewMount } from './WorldviewMount.tsx';

const KEY = 'template-drafts/01KTJ153M7D109TG4G7BBVCZYW/2/floor.msgpack';

/** What each frame the stand-in viewer draws answers: whether it presented one; `true` once the list runs out. And how
 * many viewers were opened. */
const viewer = vi.hoisted(() => ({ presents: [] as boolean[], opened: 0 }));

vi.mock('crucible-viewer', () => {
  const answers: Record<string, unknown> = {
    frame: () => viewer.presents.shift() ?? true,
    has_floor: () => false,
    camera: () => new Float32Array(6),
    centre_entities: () => [],
    centre_mm: () => new Float64Array(0),
  };
  // Every other method does nothing; `then` is left unanswered, so awaiting the stand-in does not take it for a promise.
  const standIn = new Proxy({}, { get: (_, name) => (name === 'then' ? undefined : answers[String(name)] ?? (() => undefined)) });
  return {
    openFloorViewer: async () => {
      viewer.opened += 1;
      return standIn;
    },
    readPalette: () => ({}),
  };
});
vi.mock('./floorRead.ts', () => ({ readFloor: async () => ({ ok: true, bytes: new Uint8Array([1, 2, 3]) }) }));

const flags = globalThis as unknown as Record<string, unknown>;
let frames: FrameRequestCallback[] = [];

beforeEach(() => {
  frames = [];
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => frames.push(callback));
  vi.stubGlobal('cancelAnimationFrame', () => undefined);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  delete flags[MEASURING];
  performance.clearMarks();
  viewer.presents = [];
  viewer.opened = 0;
});

/** Draw the animation frames waiting, each at `now`. */
function step(now: number) {
  act(() => frames.splice(0).forEach((frame) => frame(now)));
}

const named = (name: string) => (performance.getEntriesByName(name) as PerformanceMark[]).map((entry) => entry.detail);

function mount(strict = false) {
  const stage = (
    <WorldviewMount
      source={{ kind: 'evaluation', draft: '01KTJ153M7D109TG4G7BBVCZYW', n: 2 }} floorKey={KEY} waiting={null}
      interactive={false} panel="template" label="Evaluation 2" description="" axes={null} selection={null}
      highlights={[]} highlightSeq={null} onPick={() => undefined} onClearSelection={() => undefined}
      labelOf={async () => null}
    />
  );
  render(strict ? <StrictMode>{stage}</StrictMode> : stage);
}

/** Astra's review, its third finding (`floor:painted` claimed a frame never presented): the mark waits for one. */
it('marks a floor painted only once a frame presenting it was drawn', async () => {
  flags[MEASURING] = true;
  viewer.presents = [false, false];
  mount();
  await waitFor(() => expect(named('floor:drawn')).toEqual([{ key: KEY }]));
  step(16);
  step(32);
  expect(named('floor:painted'), 'two frames drew nothing on the screen').toEqual([]);
  step(48);
  expect(named('floor:painted')).toEqual([{ key: KEY }]);
  step(64);
  expect(named('floor:painted'), 'one mark for the floor, at its first presented frame').toEqual([{ key: KEY }]);
});

/** fwd task 1MQ_yka… (the black canvas): strict mode mounts the stage's effect twice, and a second viewer on the
 * canvas left it black. */
it('opens one viewer on its canvas under strict mode', async () => {
  mount(true);
  await waitFor(() => expect(document.querySelector('.worldview-mount')?.getAttribute('data-state')).toBe('drawn'));
  expect(viewer.opened).toBe(1);
});

/** Astra's review, its fifth finding (the marks accumulated in ordinary use): a person's session keeps none. */
it('keeps no mark of the floor outside a measurement session', async () => {
  mount();
  await waitFor(() => expect(document.querySelector('.worldview-mount')?.getAttribute('data-state')).toBe('drawn'));
  step(16);
  step(32);
  expect(performance.getEntriesByType('mark')).toEqual([]);
});
