// The Runs panel's words: a waiting run's place as an ordinal, "starting" while its runner starts, each ending in the
// RunCard's words, the queue in one sentence, and how far a run simulated (spec 08 section 5; spec 02 section 3.1).

import { describe, expect, it } from 'vitest';
import type { RunQueue, RunSummary } from '../../protocol/runs.ts';
import { ordinal, queueWords, simulatedWords, stateWords, wallWords } from './words.ts';

const run = (state: RunSummary['state'], more: Partial<RunSummary> = {}) =>
  ({ state, progress: { sim_ms: 9_660_000, values: [] }, ...more }) as Pick<RunSummary, 'state' | 'waiting' | 'stop_reason' | 'progress'>;

describe('the Runs panel words', () => {
  it('numbers a place in the queue as an ordinal', () => {
    expect([1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 101].map(ordinal)).toEqual(
      ['1st', '2nd', '3rd', '4th', '11th', '12th', '13th', '21st', '22nd', '23rd', '101st'],
    );
  });

  it("says a queued run's wait, running, and each ending as the RunCard does", () => {
    expect(stateWords(run('queued', { waiting: { for: 'seat', place_in_queue: 3 } })).words).toBe('3rd in the queue');
    expect(stateWords(run('queued', { waiting: { for: 'runner' } })).words).toBe('starting');
    expect(stateWords(run('running'))).toEqual({ words: 'running', tone: 'info', glyph: '…' });
    expect(stateWords(run('finished'))).toEqual({ words: 'finished', tone: 'good', glyph: '✓' });
    expect(stateWords(run('cancelled')).words).toBe('cancelled at 2 h 41 min of simtime');
    expect(stateWords(run('failed', { stop_reason: 'watchdog' })).words).toBe('failed: watchdog');
    expect(stateWords(run('partial', { stop_reason: 'OOMKilled' })).words).toBe('stopped early: OOMKilled');
  });

  it('says the queue in one sentence, how far a run simulated, and wall-clock spans', () => {
    const queue = (going: number, waiting: number) =>
      ({ limit: 5, going: new Array(going).fill({}), waiting: new Array(waiting).fill({}) }) as unknown as RunQueue;
    expect(queueWords(queue(1, 0))).toBe('Queue: 1 run going, none waiting; at most 5 go at a time in Agent Studio.');
    expect(queueWords(queue(5, 1))).toBe('Queue: 5 runs going, 1 waiting; at most 5 go at a time in Agent Studio.');
    const simulated = (sim_ms: number) => simulatedWords({ progress: { sim_ms, values: [] }, duration_sim_ms: 10_800_000 });
    expect([simulated(0), simulated(1_200_000), simulated(10_800_000)]).toEqual(['0 min of 3 h', '20 min of 3 h', '3 h']);
    expect([wallWords(400), wallWords(12_000), wallWords(3_900_000)]).toEqual(['under a second', '12 s', '1 h 5 min']);
  });
});
