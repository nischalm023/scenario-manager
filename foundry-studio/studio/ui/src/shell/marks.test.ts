// The evaluation-latency chore's performance marks as Astra's review of it asked (design/reviews-impl/astra-eval.md,
// with the lead's dispositions of 2026-10-20): kept in a measurement's session, and in no other.

import { afterEach, expect, it } from 'vitest';
import { MEASURING, mark } from './marks.ts';

const flags = globalThis as unknown as Record<string, unknown>;

afterEach(() => {
  delete flags[MEASURING];
  performance.clearMarks();
});

/** Astra's review, its fifth finding (the marks accumulated in ordinary use): outside a measurement none is kept. */
it('keeps no mark outside a measurement session', () => {
  mark('template:knob-changed', { draft: 'D', knob: 'aisles' });
  mark('floor:painted', { key: 'template-drafts/D/2/floor.msgpack' });
  expect(performance.getEntriesByType('mark')).toEqual([]);
});

/** In the session a measurement opened, each mark is kept with its detail for the measurement to read. */
it('keeps each mark with its detail in a measurement session', () => {
  flags[MEASURING] = true;
  mark('floor:painted', { key: 'template-drafts/D/2/floor.msgpack' });
  const kept = performance.getEntriesByName('floor:painted') as PerformanceMark[];
  expect(kept.map((entry) => entry.detail)).toEqual([{ key: 'template-drafts/D/2/floor.msgpack' }]);
});
