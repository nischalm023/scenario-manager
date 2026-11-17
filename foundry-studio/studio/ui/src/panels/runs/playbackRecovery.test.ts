// The run playback's recovery (`playbackRecovery.ts`), walked: from each start a watch can have, every event the
// watch can hear is applied in every state reached, breadth first, until no new state comes, and each state and each
// step is checked against the rule. States that differ only in ask numbers, ticket tokens, the simtime reached or the
// words of a loss are one state to the walk: the reducer compares those only with each other, by equality or order,
// and the events are made from the state they are heard in. So the walk ends, and it reaches every state any sequence
// of events can reach, however long.

import { expect, it } from 'vitest';
import type { WatchTicket } from '../../protocol/runs.ts';
import { AGAIN_AT_MOST, attachmentGoesOn, hear, start, stopped, type Heard, type Recovery } from './playbackRecovery.ts';

/** A state reached, with what the walk itself remembers of the way there: whether the run has ever reported progress
 * and whether it has ever ended, the page's own asks since the five were last given back, and, once the run has ended
 * with a picture to play, whether the page has asked by itself since (any ask but Retry's). */
type Walked = {
  state: Recovery; reported: { progressed: boolean; ended: boolean }; own: number; askedSinceEnding: boolean | null;
};

const answersTo = (ask: number): Heard[] => [
  { kind: 'got', ask, ticket: { token: `ticket-${ask}` } as WatchTicket },
  { kind: 'refused', ask, reason: 'The streaming server did not answer.', notStreamed: false },
  { kind: 'refused', ask, reason: 'Attempt 1 of this run has no live stream.', notStreamed: true },
];

/** Every event the watch can hear in `state`: answers to the ask in flight, to the ask before it (one Retry replaced,
 * or already answered) and to the latest once answered (a second answer to one ask, dropped whoever sends it);
 * the attachment begun, accepted, and lost reaching further than any before, no further, or before any keyframe; the
 * pause over, and a pause's timer that fired after its pause ended; Retry wherever a stage or sentence stands; and the
 * run changed in any way a read of it can say, an older read handed after a newer among them: served and no longer
 * served, reporting progress and not, ended and not. */
function events(state: Recovery): Heard[] {
  const heard: Heard[] = [];
  if (state.inFlight !== null) heard.push(...answersTo(state.inFlight));
  else heard.push(...answersTo(state.asked));
  if (state.asked >= 2) heard.push(...answersTo(state.asked - 1), { kind: 'paused', pause: state.asked - 1 });
  heard.push({ kind: 'attaching' }, { kind: 'accepted' });
  for (const reach of [(state.furthest ?? 0) + 60_000, state.furthest, null]) {
    heard.push({ kind: 'lost', why: 'the socket failed', reach });
  }
  if (state.pause !== null) heard.push({ kind: 'paused', pause: state.pause });
  if (state.ticket.kind === 'got' || state.ticket.kind === 'refused') heard.push({ kind: 'retry' });
  for (const served of [true, false]) {
    for (const progressed of [true, false]) {
      for (const ended of [true, false]) {
        const changed = served !== state.served || progressed !== state.progressed || ended !== state.ended;
        if (changed) heard.push({ kind: 'run', served, progressed, ended });
      }
    }
  }
  return heard;
}

const keyOf = ({ state, reported, own, askedSinceEnding }: Walked) => JSON.stringify([
  state.ticket.kind, state.attachment, Math.min(state.asked, 2), state.inFlight === null, state.trouble === null,
  state.pause === null, state.spent, state.furthest === null, state.served, state.progressed, state.ended,
  state.endingHeard, reported.progressed, reported.ended, own, askedSinceEnding,
]);

const retryOffered = (state: Recovery) => (state.ticket.kind === 'got' && state.attachment === 'lost')
  || (state.ticket.kind === 'refused' && state.spent >= AGAIN_AT_MOST);

/** No server serves the attempt, as the page reckons it, and a server's arrival asks. */
const waitsForServer = (state: Recovery) => state.ticket.kind === 'none' && !state.served
  && hear(state, { kind: 'run', served: true, progressed: state.progressed, ended: state.ended }).inFlight !== null;

/** What is wrong with `walked`, a state reached, if anything. */
function faultsOf({ state, own, askedSinceEnding }: Walked): string[] {
  const faults: string[] = [];
  if (state.inFlight !== null && state.inFlight !== state.asked) faults.push('an ask in flight that is not the latest');
  if (state.pause !== null && state.pause !== state.asked) faults.push('a pause with an ask made since it began');
  const idle = state.trouble !== null && state.inFlight === null && !attachmentGoesOn(state);
  if (state.pause !== null && !(idle && state.spent < AGAIN_AT_MOST)) faults.push('a pause while something else will answer the loss');
  if (idle && (state.pause !== null) !== (state.spent < AGAIN_AT_MOST)) faults.push('an interruption that neither pauses with asks left nor stops with none');
  // The first property: the page's own asks, counted once the run has ever reported progress or ended, are five at
  // most between the givings back.
  if (own > AGAIN_AT_MOST) faults.push(`${own} asks of the page's own since the five were given back`);
  // The second: no state is stuck.
  const waits = state.attachment === 'playing' || state.inFlight !== null || attachmentGoesOn(state)
    || state.pause !== null || retryOffered(state);
  if (!waits && state.ticket.kind !== 'none') faults.push('stuck: nothing plays, asks, attaches, pauses or offers Retry');
  if (!waits && state.ticket.kind === 'none' && !waitsForServer(state)) {
    faults.push('stuck with no server: a server to serve the attempt asks nothing');
  }
  // After the run ends with a picture to play, an ask follows unless no server serves the attempt, and then a
  // server's arrival leads to one: until the page asks by itself, an ask in flight, the pause or an attachment going
  // on will lead to an ask or a picture that plays, or the page waits for a server.
  const answering = state.inFlight !== null || state.pause !== null || attachmentGoesOn(state) || waitsForServer(state);
  if (askedSinceEnding === false && !answering) faults.push('the run ended with a picture to play, and nothing will ask for it');
  return faults;
}

/** What is wrong with the step from `was` by `heard` into `into`, if anything; `ending`: whether `heard` is the run's
 * ending with a picture to play, by the walk's own memory of the run. */
function stepFaultsOf(was: Recovery, heard: Heard, into: Recovery, ending: boolean): string[] {
  const faults: string[] = [];
  if (into.asked > was.asked + 1) faults.push('two asks in one step');
  if (into.asked > was.asked && was.inFlight !== null && heard.kind !== 'retry') faults.push(`a second ask in flight, by ${heard.kind}`);
  if (into.asked > was.asked && !['paused', 'retry', 'run'].includes(heard.kind)) faults.push(`an ask made by ${heard.kind}`);
  // The third property: a picture that plays is replaced only after it is lost.
  if (was.attachment === 'playing' && heard.kind !== 'lost' && (into.ticket !== was.ticket || into.attachment !== 'playing')) {
    faults.push(`a picture playing replaced or stopped by ${heard.kind}`);
  }
  // The run's ending with a picture to play gives the five back, once, and leaves no page stopped.
  if (ending !== (into.endingHeard && !was.endingHeard)) faults.push('an ending heard otherwise than once, as the run ends with a picture to play');
  if (ending && (into.spent !== 0 || stopped(into))) faults.push('an ending that leaves the five spent or the page stopped');
  return faults;
}

function walk() {
  const reached = new Map<string, { walked: Walked; from: string | null; by: Heard | null }>();
  let frontier: string[] = [];
  for (const served of [true, false]) {
    for (const progressed of [true, false]) {
      for (const ended of [true, false]) {
        const walked = {
          state: start({ served, progressed, ended }), reported: { progressed, ended }, own: 0,
          askedSinceEnding: ended && served ? true : null,
        };
        const key = keyOf(walked);
        reached.set(key, { walked, from: null, by: null });
        frontier.push(key);
      }
    }
  }
  const faults: string[] = [];
  const path = (key: string | null): string[] => {
    const step = key === null ? undefined : reached.get(key);
    return step === undefined || step.by === null ? [] : [...path(step.from), JSON.stringify(step.by)];
  };
  let depth = 0;
  for (const key of frontier) faults.push(...faultsOf(reached.get(key)!.walked).map((fault) => `${fault}, at a start`));
  while (frontier.length > 0 && faults.length === 0) {
    const next: string[] = [];
    for (const from of frontier) {
      const { state, reported, own, askedSinceEnding } = reached.get(from)!.walked;
      for (const heard of events(state)) {
        const into = hear(state, heard);
        const heardLoss = heard.kind === 'lost' && into.attachment === 'lost' && state.attachment !== 'lost';
        const further = heardLoss && heard.reach !== null && (state.furthest === null || heard.reach > state.furthest);
        const since = heard.kind === 'run'
          ? { progressed: reported.progressed || heard.progressed, ended: reported.ended || heard.ended } : reported;
        const ending = heard.kind === 'run' && since.ended && heard.served && !state.endingHeard;
        const ownAsk = heard.kind === 'paused' && into.asked > state.asked && (reported.progressed || reported.ended);
        const byItself = into.asked > state.asked && heard.kind !== 'retry';
        const walked = {
          state: into, reported: since, own: further || ending ? 0 : own + (ownAsk ? 1 : 0),
          askedSinceEnding: ending ? byItself : askedSinceEnding === null ? null : askedSinceEnding || byItself,
        };
        const after = [...path(from), JSON.stringify(heard)].join(' ');
        faults.push(...stepFaultsOf(state, heard, into, ending).map((fault) => `${fault}, after ${after}`));
        const key = keyOf(walked);
        if (reached.has(key)) continue;
        reached.set(key, { walked, from, by: heard });
        faults.push(...faultsOf(walked).map((fault) => `${fault}, after ${after}`));
        next.push(key);
      }
    }
    frontier = next;
    if (next.length > 0) depth += 1;
  }
  return { faults, reached: [...reached.values()].map((step) => step.walked), depth };
}

it('keeps the rule in every state any sequence of events reaches, and in every step between', () => {
  const { faults, reached, depth } = walk();
  expect(faults.slice(0, 3)).toEqual([]);
  // The walk got as far as the rule reaches: five asks spent and stopped, with a ticket and without; a picture
  // playing; no server; five spent with the run's ending heard; and the run ended as an ask was in flight whose
  // answer found no server, with no ask since: a server's arrival asks.
  expect(reached.some(({ state }) => stopped(state) && state.ticket.kind === 'got' && state.spent === AGAIN_AT_MOST)).toBe(true);
  expect(reached.some(({ state }) => stopped(state) && state.ticket.kind === 'refused')).toBe(true);
  expect(reached.some(({ state }) => state.attachment === 'playing')).toBe(true);
  expect(reached.some(({ state }) => state.ticket.kind === 'none')).toBe(true);
  expect(reached.some(({ state }) => stopped(state) && state.endingHeard && state.spent === AGAIN_AT_MOST)).toBe(true);
  expect(reached.some(({ state, askedSinceEnding }) => askedSinceEnding === false && state.ticket.kind === 'none')).toBe(true);
  // A walk that stopped short of spending the five from a start (a refusal, then five pauses and their refusals)
  // would say nothing of the bound.
  expect(depth).toBeGreaterThanOrEqual(2 * AGAIN_AT_MOST + 1);
});

/** The Runs panel's reads of the run can answer out of order: one made before the run reported progress, handed to
 * the page after one made since, once made every ask of the page's own free, for good. */
it("counts the page's own asks once the run has reported progress, whatever an older read of the run says later", () => {
  const read = { kind: 'run', served: true, ended: false } as const;
  let state = hear(start({ served: true, progressed: false, ended: false }), { ...read, progressed: true });
  state = hear(state, { ...read, progressed: false });
  const refusal = (ask: number): Heard => ({ kind: 'refused', ask, reason: 'The streaming server did not answer.', notStreamed: false });
  state = hear(state, refusal(state.asked));
  for (let pause = 0; pause < AGAIN_AT_MOST; pause += 1) {
    state = hear(state, { kind: 'paused', pause: state.asked });
    state = hear(state, refusal(state.asked));
  }
  expect(stopped(state), 'five asks of its own, then only Retry').toBe(true);
});
