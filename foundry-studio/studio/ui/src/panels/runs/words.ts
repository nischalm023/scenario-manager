// The Runs panel's words (spec 08 section 5): a run's state as the RunCard says it (spec 02 section 3.1, the chat's
// words), with the queue's own words while it waits ("3rd in the queue", "starting"); the queue in one sentence; how
// far a run simulated; and a wall-clock span. Every time the panel shows names its clock, the components wrapping
// each in `.clock[data-clock]`.

import { plural, runStateWords, simtimeWords } from '../../chat/words.ts';
import type { RunOutcome } from '../../protocol/metric_stream.ts';
import type { RunQueue, RunSummary } from '../../protocol/runs.ts';

/** "1st", "2nd", "3rd", "11th", "22nd". */
export function ordinal(n: number): string {
  const tens = n % 100;
  if (tens >= 11 && tens <= 13) return `${n}th`;
  return `${n}${({ 1: 'st', 2: 'nd', 3: 'rd' } as Record<number, string>)[n % 10] ?? 'th'}`;
}

/** A wall-clock span: "under a second", "12 s", "2 min", "1 h 5 min". */
export function wallWords(ms: number): string {
  if (ms < 1000) return 'under a second';
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds} s`;
  const minutes = Math.round(seconds / 60);
  return minutes < 60 ? `${minutes} min` : `${Math.floor(minutes / 60)} h ${minutes % 60} min`;
}

/** A run's state in the RunCard's words, and while it waits the queue's: its place for a seat, or "starting". */
export function stateWords(run: Pick<RunSummary, 'state' | 'waiting' | 'stop_reason' | 'progress'>): ReturnType<typeof runStateWords> {
  if (run.state !== 'queued') return runStateWords({ state: run.state, stop_reason: run.stop_reason, sim_ms: run.progress.sim_ms });
  const place = run.waiting?.place_in_queue;
  const words = run.waiting?.for === 'runner' ? 'starting' : place !== undefined ? `${ordinal(place)} in the queue` : 'queued';
  return { words, tone: 'info', glyph: null };
}

/** "Queue: 2 going, none waiting; at most 5 go at a time in Agent Studio." */
export function queueWords(queue: RunQueue): string {
  const waiting = queue.waiting.length === 0 ? 'none waiting' : `${queue.waiting.length} waiting`;
  return `Queue: ${plural(queue.going.length, 'run')} going, ${waiting}; at most ${queue.limit} go at a time in Agent Studio.`;
}

/** How far a run simulated, in simtime: "20 min of 3 h", or "3 h" once it reached its duration. */
export function simulatedWords(run: Pick<RunSummary, 'progress' | 'duration_sim_ms'>): string {
  const reached = run.progress.sim_ms >= run.duration_sim_ms;
  return reached ? simtimeWords(run.duration_sim_ms) : `${simtimeWords(run.progress.sim_ms)} of ${simtimeWords(run.duration_sim_ms)}`;
}

/** A run's short name: the first eight characters of its id, as spec 08's sentences name it. */
export const shortId = (id: string): string => id.slice(0, 8);

/** Why an incomplete recording stops where it does: the server's copy is the live segments the store held of an
 * attempt whose streaming server stopped before its uploads finished. */
export const INCOMPLETE_WHY = 'the streaming server stopped before it stored the rest of the run';

/** How a run's recording ends, as the streaming server ends its charts and its picture alike, in the page's words. */
export const OUTCOME_WORDS: Record<RunOutcome, string> = {
  complete: 'every order was completed',
  'user-stop': 'a person stopped it',
  watchdog: "the simulator's watchdog stopped it",
  'wall-timeout': 'it reached its wall-clock bound',
  'sim-time-limit': 'it reached the simtime it was to simulate',
  error: 'the simulator stopped on an error',
  'recording-incomplete': `the recording is incomplete: ${INCOMPLETE_WHY}`,
  'no-end-block': 'its recording has no end block',
};
