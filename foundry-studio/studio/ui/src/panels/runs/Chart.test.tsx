// A chart keeps one uPlot while its lines stay the same: new values are set into it, a moved cursor only redraws it,
// and a new line builds another in its place; the value axis is as wide as its widest label; the cursor is drawn as a
// line inside the x axis's range and not past it. uPlot is a stand-in here that counts what is asked of it and keeps
// the options it was built with, since a page without a screen's pixel ratio draws nothing; the test runs the plot's
// draw hooks on a canvas that records them.

import { act, cleanup, render, waitFor } from '@testing-library/react';
import type uPlot from 'uplot';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Chart } from './Chart.tsx';
import type { ChartSpec } from './charts.ts';
import { playerCursor } from './playerCursor.ts';

const plots = vi.hoisted(() => ({ built: 0, destroyed: 0, set: [] as unknown[], redrawn: 0, options: null as unknown }));

vi.mock('uplot', () => ({
  default: class {
    static paths = {};
    constructor(options: unknown) {
      plots.built += 1;
      plots.options = options;
    }
    setData(data: unknown) {
      plots.set.push(data);
    }
    redraw() {
      plots.redrawn += 1;
    }
    setSize() {}
    destroy() {
      plots.destroyed += 1;
    }
  },
}));

const spec = (values: number[], label = 'Picks'): ChartSpec => ({
  x: values.map((_, at) => at), series: [{ label, values, key: 1 }], xRange: [0, 1], xTicks: [], xTitle: 'simtime, hours',
  yTitle: 'picks', yRange: [0, 10], shape: 'line', yWords: (value) => String(value),
});

beforeEach(() => {
  Object.assign(plots, { built: 0, destroyed: 0, set: [], redrawn: 0, options: null });
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

it('keeps one uPlot as values come and the cursor moves, and builds another for a new line', async () => {
  const first = spec([1, 2]);
  const cursor = playerCursor();
  cursor.set(0.1);
  const { rerender } = render(<Chart name="Picks" spec={first} cursor={cursor} />);
  await waitFor(() => expect(plots.built).toBe(1));
  act(() => cursor.set(0.2));
  act(() => cursor.set(0.3));
  act(() => cursor.set(0.3));
  expect([plots.built, plots.redrawn, plots.set.length], 'a moved cursor redraws the plot it has, once per move').toEqual([1, 2, 0]);
  rerender(<Chart name="Picks" spec={spec([1, 2, 3])} cursor={cursor} />);
  expect([plots.built, plots.set], 'new values are set into it').toEqual([1, [[[0, 1, 2], [1, 2, 3]]]]);
  rerender(<Chart name="Picks" spec={spec([1, 2, 3], 'Orders')} cursor={cursor} />);
  await waitFor(() => expect([plots.built, plots.destroyed], 'a new line is another plot').toEqual([2, 1]));
});

it('makes the value axis as wide as its widest label, with room for its title', async () => {
  render(<Chart name="Picks" spec={spec([1, 2])} />);
  await waitFor(() => expect(plots.built).toBe(1));
  const size = (plots.options as uPlot.Options).axes?.[1].size as (plot: unknown, values: string[]) => number;
  const plot = { ctx: { save() {}, restore() {}, font: '', measureText: (text: string) => ({ width: text.length * 6.5 }) } };
  expect(size(plot, ['0', '1,000 per simulated hour', '500'])).toBe(Math.ceil(24 * 6.5) + 28);
});

it("draws the cursor as a line across the plot inside the x axis's range, and none once it passes it", async () => {
  const cursor = playerCursor();
  render(<Chart name="Picks" spec={spec([1, 2])} cursor={cursor} />);
  await waitFor(() => expect(plots.built).toBe(1));
  const lines: [number, number][] = [];
  const canvas = new Proxy({}, { get: (_, name) => (name === 'moveTo' ? (x: number, y: number) => lines.push([x, y]) : () => undefined), set: () => true });
  const plot = { ctx: canvas, bbox: { left: 0, top: 10, width: 100, height: 80 }, scales: { x: { min: 0, max: 1 } }, valToPos: (value: number) => value * 100 };
  const draw = () => (plots.options as uPlot.Options).hooks?.draw?.forEach((hook) => hook?.(plot as unknown as uPlot));
  draw();
  expect(lines, 'no cursor yet').toEqual([]);
  cursor.set(0.5);
  draw();
  expect(lines, "the player's cursor, halfway along").toEqual([[50, 10]]);
  cursor.set(1.25);
  draw();
  expect(lines, 'past the newest bucket and its pad: nothing drawn in the gutter').toEqual([[50, 10]]);
});
