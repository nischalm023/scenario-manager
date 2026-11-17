// The run playback's watch (live frames, 26 October 2026) on its five asks of its own, against the stand-ins of
// `testing/playback.ts`: before the run reports progress they are not counted; after, five, then only Retry asks, and
// a Retry that fails leaves them spent; an older read of the run handed to the page after a newer takes neither its
// progress nor its ending back. They are given back when a lost attachment's picture had reached further in
// simtime than any lost before, never for a keyframe alone, and once as the run ends with a picture to play, whatever
// was in flight. So asking stops when the picture gets no further, a page that stops always offers Retry, and after
// the run's ending an ask follows unless no server serves the attempt, and then a server's arrival leads to one.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { RunDetail } from '../../protocol/runs.ts';
import {
  answers, asked, asksAnswered, attached, GOT, holdTheNextAsk, keyframe, refused, replayFailsAt, resetPlayback, run,
  UNAVAILABLE,
} from '../../testing/playback.ts';
import { RunPlayback } from './RunPlayback.tsx';

vi.mock('crucible-viewer', async () => (await import('../../testing/playback.ts')).viewerModule());
vi.mock('../../shell/api.ts', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../shell/api.ts')>()),
  postJson: (await import('../../testing/playback.ts')).postJson,
}));

beforeEach(() => {
  resetPlayback();
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const none = () => document.querySelector('#run-playback-none')?.textContent;
const retry = () => screen.queryByRole('button', { name: 'Retry' });

/** A finished run's replay whose recording holds a delta the viewer refuses at 15 min: every fresh attachment
 * replays from simtime zero, brings the same keyframe, and fails at the same instant; at most ten are played out. */
async function replaysFailingAt15Min() {
  answers.next = GOT;
  render(<RunPlayback served run={run({ state: 'finished' })} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  for (let replay = 0; replay < 10 && replay < attached.length; replay += 1) await replayFailsAt(900_000);
}

/** A runner slow to start: the server holds no frame of the run until it does, however many asks that takes. */
it('asks again as often as it takes until the run reports progress, then a few times only', async () => {
  answers.next = GOT;
  const { rerender } = render(<RunPlayback served run={run({ progress: { sim_ms: 0, values: [] } })} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  for (let refusal = 0; refusal < 8; refusal += 1) await refused();
  expect(attached, 'eight refusals before any progress, eight more asks').toHaveLength(9);
  rerender(<RunPlayback served run={run()} />);
  for (let refusal = 0; refusal < 7; refusal += 1) await refused();
  expect(attached, 'once it reports progress, five more asks at most').toHaveLength(14);
});

/** A week-later page whose server's fetch from the store fails, or whose server restarts: the picture comes back on its
 * own, and a run that ended before its first frame is asked for a few times only too. */
it("asks for a fresh ticket after an ended run's picture is lost, a few times only", async () => {
  answers.next = GOT;
  render(<RunPlayback served run={run({ state: 'failed', progress: { sim_ms: 0, values: [] } })} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  for (let loss = 0; loss < 7; loss += 1) {
    act(() => attached.at(-1)?.options.onError?.('the run store did not answer'));
    await act(() => vi.advanceTimersByTimeAsync(2_010));
  }
  expect(attached, 'the first attach, then five fresh tickets').toHaveLength(6);
});

/** The Runs panel's reads of the run can answer out of order, so the page can be handed an older run after a newer. */
it.each([
  ['reported progress', run(), run({ progress: { sim_ms: 0, values: [] } })],
  ['ended', run({ state: 'failed', progress: { sim_ms: 0, values: [] } }), run({ progress: { sim_ms: 0, values: [] } })],
])('counts its asks once the run has %s, though the page is then handed an older run that says not', async (_, newer, older) => {
  answers.next = GOT;
  const { rerender } = render(<RunPlayback served run={newer} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  rerender(<RunPlayback served run={older} />);
  for (let refusal = 0; refusal < 7; refusal += 1) await refused();
  expect(attached, 'the first attach, then five fresh tickets').toHaveLength(6);
});

it('stops asking when each fresh replay gets no further than the last, though each brings its keyframe', async () => {
  await replaysFailingAt15Min();
  expect(attached, 'the first replay got further than none; then five asks, and no more').toHaveLength(6);
  expect(retry(), 'the stage offers Retry').not.toBeNull();
});

it('stops again at once, its five still spent, when the replay Retry asked for fails the same way', async () => {
  await replaysFailingAt15Min();
  fireEvent.click(retry() as HTMLElement);
  await act(() => vi.advanceTimersByTimeAsync(10));
  expect(attached).toHaveLength(7);
  await replayFailsAt(900_000);
  await asksAnswered(3);
  expect(attached, 'no ask of its own after the failed Retry').toHaveLength(7);
  expect(retry(), 'the stage offers Retry again').not.toBeNull();
});

/** A live run whose server drops the picture now and then, each time after it caught up further. */
it('asks again after every loss while each lost replay had got further than any before', async () => {
  answers.next = GOT;
  render(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  for (let replay = 0; replay < 9; replay += 1) await replayFailsAt(900_000 + replay * 60_000);
  expect(attached, 'every loss asked again').toHaveLength(10);
});

/** A long run whose server hiccups now and then: each hiccup is a few refused asks, and the picture comes back
 * between. */
it('counts its asks afresh once an attachment whose picture reached further than before is lost', async () => {
  answers.next = GOT;
  render(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  for (let loss = 0; loss < 4; loss += 1) await refused();
  keyframe(900_000);
  for (let loss = 0; loss < 7; loss += 1) await refused();
  expect(attached, 'four asks, then the picture, then five more at most').toHaveLength(10);
});

it('says why before any ticket, asks again a few times once the run reports progress, then offers Retry', async () => {
  answers.next = UNAVAILABLE;
  render(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  expect(none()).toBe('No playback: The streaming server did not answer. Asking again.');
  await asksAnswered(7);
  expect(asked, 'the first ask, then five more').toHaveLength(6);
  expect(none()).toBe('No playback: The streaming server did not answer. Retry');
  answers.next = GOT;
  fireEvent.click(retry() as HTMLElement);
  await act(() => vi.advanceTimersByTimeAsync(10));
  expect(attached.map((one) => one.options.token)).toEqual(['the-ticket-7']);
});

it("offers the sentence's Retry again when the ask it made fails, and asks no more on its own", async () => {
  answers.next = UNAVAILABLE;
  render(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  await asksAnswered(7);
  fireEvent.click(retry() as HTMLElement);
  await asksAnswered(3);
  expect(asked, 'the first ask, five of its own, then the one Retry made').toHaveLength(7);
  expect(none()).toBe('No playback: The streaming server did not answer. Retry');
});

/** The run's streaming server went down as it ran, and the page stopped asking; once the run has ended, a replay
 * server can play what the store holds. */
it('asks again as the run ends with a recording to play, after the page stopped asking', async () => {
  answers.next = UNAVAILABLE;
  const { rerender } = render(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  await asksAnswered(7);
  expect(asked).toHaveLength(6);
  answers.next = GOT;
  rerender(<RunPlayback served run={run({ state: 'finished', recording: { stored: true } as RunDetail['recording'] })} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  expect(asked).toHaveLength(7);
  expect(attached).toHaveLength(1);
});

/** The run ends while the page's last ask of its own is in flight, and that ask fails after the ending. */
it('asks again after the run ends while its last ask of its own was in flight, once that ask fails', async () => {
  answers.next = GOT;
  const { rerender } = render(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  for (let loss = 0; loss < 4; loss += 1) await refused();
  const answer = holdTheNextAsk();
  await refused();
  expect(asked, 'the first ask, then five of its own, the last in flight').toHaveLength(6);
  rerender(<RunPlayback served run={run({ state: 'finished' })} />);
  await act(async () => answer(UNAVAILABLE));
  await asksAnswered(1);
  expect(asked, 'a fresh ask after the ending').toHaveLength(7);
  expect(attached.at(-1)?.options.token).toBe('the-ticket-7');
});
