// The Scenario panel's words (spec 07 sections 4.2 and 5): values in the field catalogue's shown unit and back, who
// set a field, a conflict's sentence, the readiness line and the count beside Run.

import { describe, expect, it } from 'vitest';
import type { FieldEntry, SetBy } from '../../protocol/scenario.ts';
import type { Readiness } from '../../protocol/workload.ts';
import {
  conflictWords, readinessWords, runCountWords, runLabel, setByWords, sharedSetter, shownValue, storedValue, sweepValues,
} from './words.ts';

const duration: FieldEntry = {
  pointer: '/run_controls/duration_sim_ms', label: 'Duration', short_label: 'duration', unit: 'hours of simtime',
  per_unit: 3_600_000, clock: 'sim', sweepable: false, options: [], hint: '',
};
const rate: FieldEntry = { ...duration, pointer: '/workload/generator/orders_per_hour', label: 'Orders per hour', short_label: 'rate', unit: 'orders an hour', per_unit: 1, clock: null, sweepable: true };
const agent: SetBy = { by: 'agent', author: { subject: 'agent', name: 'Agent' }, turn: { id: '01KN0TJZANP5N0PGJD8F7A3FDB', n: 6 }, at: '2026-10-19T10:00:00Z', revision: 2 };
const you: SetBy = { by: 'user', author: { subject: 'dev', name: 'Developer' }, turn: null, at: '2026-10-19T10:00:00Z', revision: 1 };
const site = 'customer/demo/site/lshape-6';

describe('spec 07 section 5: the Scenario panel words', () => {
  it('shows a duration in hours of simtime and stores it in milliseconds', () => {
    expect(shownValue(duration, 10_800_000)).toBe('3');
    expect(storedValue(duration, '2')).toEqual({ ok: true, value: 7_200_000 });
    expect(storedValue(rate, 'many')).toEqual({ ok: false, why: 'Orders per hour needs a number; "many" is not one.' });
  });

  it('says who set a field, once for all when one setter set them all', () => {
    expect(setByWords(agent, 'dev')).toBe('set by the agent in turn 6');
    expect(setByWords(you, 'dev')).toBe('set by you');
    expect(sharedSetter([you, you], 'dev')).toBe('set by you');
    expect(sharedSetter([you, agent], 'dev')).toBeNull();
  });

  it('words a conflict with the other value in its unit', () => {
    expect(conflictWords(duration, 7_200_000, agent, 'dev')).toBe('Changed by the agent in turn 6 to 2 hours of simtime; yours was not saved.');
  });

  it("reads the readiness line's state and what follows it from the cut and the workload", () => {
    const ready: Readiness = {
      checked_at: '2026-10-19T10:00:00Z', ready: true, problems: [],
      cut: { unit: site, requested: 'main', branch: 'main', exact: true, exact_reason: null,
        seqs: { [site]: 1, [`${site}/map`]: 1, [`${site}/flow`]: 2, [`${site}/fleet`]: 1 } },
      workload: { orders: 900, lines: 2700, skus: 400, preload_fraction: 0.25 },
    };
    expect(readinessWords(ready, site)).toEqual({ state: 'Ready to run', detail: '900 orders at map 1, flow 2, fleet 1' });
    const error = { code: 'unsupported-value', severity: 'error' as const, text: 'Random arrivals need a runner image.' };
    expect(readinessWords({ ...ready, ready: false, problems: [error, error] }, site)).toEqual({ state: 'Not ready', detail: '2 problems' });
  });

  it('counts the runs beside Run with the limit on how many go at once, and says nothing beside one run', () => {
    expect(runLabel(5)).toBe('Run 5 runs');
    expect(runLabel(1)).toBe('Run 1 run');
    expect(runCountWords(5, 1)).toBe('5 values, 5 runs; at most five go at a time.');
    expect(runCountWords(4, 3)).toBe('4 values, 12 runs; at most five go at a time.');
    expect(runCountWords(1, 3)).toBe('3 runs; at most five go at a time.');
    expect(runCountWords(1, 1)).toBeNull();
    expect(sweepValues(rate, '200, 250, 300, 350, 400')).toEqual({ ok: true, values: [200, 250, 300, 350, 400] });
    expect(sweepValues(rate, '200')).toEqual({ ok: false, why: 'A sweep takes 2 to 12 values; this has 1.' });
  });
});
