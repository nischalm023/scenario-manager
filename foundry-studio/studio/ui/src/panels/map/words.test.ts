// The Map panel's sentences (spec 03 sections 3.6 and 5): the stage's name, the counts' caption, the summary line and
// a mark's kind and place.

import { describe, expect, it } from 'vitest';
import type { MapSummary } from '../../protocol/map.ts';
import { censusCaption, kindOf, markPlace, stageLabel, summaryLine } from './words.ts';

const ROOT = 'customer/demo/site/lshape-6';

const summary = {
  location: `${ROOT}/map@main`, kin: null,
  resolved: [{ scope: ROOT, seq: 1 }, { scope: `${ROOT}/fleet`, seq: 1 }, { scope: `${ROOT}/flow`, seq: 2 }, { scope: `${ROOT}/map`, seq: 3 }],
  site: { scope: ROOT, label: 'Small L-shape' }, floor: { key: 'ab.floor1@x', floor_version: 1, bytes: 10 }, floor_problem: null,
  empty: false, frame: { id: '7794b939-9044-42c0-84d0-64b300c0a89b', unit_mm: 1 },
  axes: { frame: '4fd096ef-c7f0-450f-9109-6eabdf0460b8', name: 'shell', x: [1, 0], y: [0, 1] },
  counts: [
    { label: 'cell + pose', components: ['cell', 'pose'], count: 1940 },
    { label: 'external_ids + pose + station', components: ['external_ids', 'pose', 'station'], count: 8 },
    { label: 'external_ids + queue', components: ['external_ids', 'queue'], count: 8 },
  ],
  made_from: null, exact: true, exact_reason: null,
} satisfies MapSummary;

describe("the Map panel's words", () => {
  it('names the stage by what is drawn and its version', () => {
    expect(stageLabel(summary)).toBe('Small L-shape, map at changeset 3');
  });

  it('captions the counts with the cut, the map first', () => {
    expect(censusCaption(summary.resolved)).toBe(
      'Entities in the site at map changeset 3, fleet 1, flow 2, counted by kind: one row per set of components',
    );
  });

  it('sums up the floor, the highlights and the selection', () => {
    expect(summaryLine(summary, 4, null)).toBe(
      '1,940 floor cells, 8 stations. Axes of the shell frame: x to the right, y up the drawing. 4 entities highlighted by the agent. Nothing selected.',
    );
    expect(summaryLine(summary, 0, 'Pick station 3')).toBe(
      '1,940 floor cells, 8 stations. Axes of the shell frame: x to the right, y up the drawing. No agent highlights. Pick station 3 selected.',
    );
  });

  it("says a mark's kind without its bookkeeping, and its place in metres", () => {
    expect(kindOf(['external_ids', 'pose', 'station'])).toBe('pose + station');
    expect(kindOf(['external_ids'])).toBe('external_ids');
    expect(markPlace({ centre_mm: [35_200, 4_200] })).toBe('x 35.2 m, y 4.2 m');
    expect(markPlace({ centre_mm: null })).toBeNull();
  });
});
