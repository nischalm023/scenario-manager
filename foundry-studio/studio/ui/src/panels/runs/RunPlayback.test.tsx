// The run page's playback (live frames, 26 October 2026) against a stand-in API, viewer and frames attachment
// (`testing/playback.ts`): a run with a stream gets a watch ticket and attaches to its `frames_url` with its token,
// once, says "attaching" until the server answers, then "live, as of" the instant the picture reaches while the run
// goes, and "replay" once the frames end or the run has ended, a warning in its place when the server says its copy
// of the recording stops short of the run's end; until an attachment made again brings its keyframe, the controls
// keep the last window, play and the cursor inert, and drop it once none can come; the cursor stated and told the page
// is the instant the picture shows, within the window held; and focus on a Retry or a playback control that leaves
// goes to Play, the stage, or before any stage the sentence, never to the page, while focus the person moved elsewhere
// first stays where they put it.
// How the page asks again is its watch's (`playbackWatch.test.tsx`, `playbackWatch.budget.test.tsx`).

import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import {
  answers, asked, asksAnswered, attached, badge, closeTheSocket, cursorLine, FRAMES, GOT, holdTheNextAsk, keyframe,
  lagged, refused, resetPlayback, run, RUN, stageOf, UNAVAILABLE, viewers,
} from '../../testing/playback.ts';
import { RunPlayback } from './RunPlayback.tsx';

vi.mock('crucible-viewer', async () => (await import('../../testing/playback.ts')).viewerModule());
vi.mock('../../shell/api.ts', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../shell/api.ts')>()),
  postJson: (await import('../../testing/playback.ts')).postJson,
}));

beforeEach(resetPlayback);

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it('gets a ticket, attaches to its frames address with its token once, and words the picture live', async () => {
  answers.next = GOT;
  render(<RunPlayback served run={run()} />);
  await waitFor(() => expect(attached).toHaveLength(1));
  expect(asked).toEqual([`/api/runs/${RUN}/watch`]);
  expect([attached[0].url, attached[0].options.token]).toEqual([FRAMES, 'the-ticket-1']);
  expect(badge()).toBe('Attaching to the stream');
  expect(document.querySelector('.worldview-mount')?.getAttribute('data-state')).toBe('loading');
  act(() => attached[0].options.onAttached?.({ from_ms: 0, live_edge_ms: 900_000 }));
  expect(badge()).toBe('Live, as of 15 min simtime, trailing the simulator by 5 min');
  act(() => attached[0].options.onFrame?.('scenario', 1_024));
  expect(document.querySelector('.worldview-mount')?.getAttribute('data-state')).toBe('drawn');
  act(() => attached[0].options.onEnded?.('sim-time-limit'));
  expect(badge()).toBe('Replay');
  expect(attached).toHaveLength(1);
});

it('warns of a replay whose recording the server says stops short of the run\'s end', async () => {
  answers.next = GOT;
  render(<RunPlayback served run={run({ state: 'finished' })} />);
  await waitFor(() => expect(attached).toHaveLength(1));
  act(() => attached[0].options.onAttached?.({ from_ms: 0, live_edge_ms: 3_600_000 }));
  act(() => attached[0].options.onEnded?.('recording-incomplete'));
  expect(badge()).toBe('Replay of an incomplete recording: the streaming server stopped before it stored the rest of the run');
  expect(document.querySelector('#run-playback-mode .pill')?.getAttribute('data-tone')).toBe('warn');
});

it('words an ended run as a replay, and an error as an interruption', async () => {
  answers.next = GOT;
  render(<RunPlayback served run={run({ state: 'finished' })} />);
  await waitFor(() => expect(attached).toHaveLength(1));
  act(() => attached[0].options.onAttached?.({ from_ms: 0, live_edge_ms: 10_830_000 }));
  expect(badge()).toBe('Replay');
  act(() => attached[0].options.onError?.('no such attempt'));
  expect(badge()).toBe('Interrupted: no such attempt');
  expect(document.querySelector('.worldview-mount')?.getAttribute('data-state')).toBe('failed');
  expect(asked).toHaveLength(1);
});

/** Asking again after frames had arrived once took the window away until the fresh keyframe, and with it Play, Speed,
 * the cursor and Follow, so focus on any of them fell to the page. */
it('keeps the last window and focus on Play from asking again until the fresh keyframe, play and the cursor inert', async () => {
  vi.useFakeTimers();
  answers.next = GOT;
  render(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  keyframe(900_000);
  await act(() => vi.advanceTimersByTimeAsync(250));
  const play = screen.getByRole('button', { name: 'Play' });
  play.focus();
  await refused();
  await act(() => vi.advanceTimersByTimeAsync(250));
  expect(attached.map((one) => one.options.token), 'asked again').toEqual(['the-ticket-1', 'the-ticket-2']);
  expect(viewers.window, 'the viewer cleared for the fresh attachment').toEqual([]);
  expect(document.activeElement).toBe(play);
  expect(play.getAttribute('aria-disabled')).toBe('true');
  viewers.driven = [];
  fireEvent.click(play);
  fireEvent.change(screen.getByRole('slider'), { target: { value: '300000' } });
  expect(viewers.driven, 'nothing driven while the viewer holds no window').toEqual([]);
  keyframe(960_000);
  expect(play.getAttribute('aria-disabled'), 'live again at the keyframe, before any read').toBeNull();
  fireEvent.click(play);
  expect(viewers.driven.at(-1)).toBe('play');
});

/** The viewer is read only as a keyframe opens its window and every quarter second, so the controls learn of a clear
 * from the stream, or Play and the cursor would drive a viewer holding no window until the next read. */
it.each([
  ['the page asks again on a fresh ticket', refused],
  ['the server drops a slow attachment', lagged],
])('makes Play and the cursor inert as soon as the viewer is cleared, before any read, when %s', async (_, cleared) => {
  vi.useFakeTimers();
  answers.next = GOT;
  render(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  keyframe(900_000);
  await act(() => vi.advanceTimersByTimeAsync(250));
  const [play, cursor] = [screen.getByRole('button', { name: 'Play' }), screen.getByRole('slider')];
  await cleared();
  expect(viewers.window, 'the viewer cleared').toEqual([]);
  expect([play.getAttribute('aria-disabled'), cursor.getAttribute('aria-disabled')]).toEqual(['true', 'true']);
  viewers.driven = [];
  fireEvent.click(play);
  fireEvent.change(cursor, { target: { value: '300000' } });
  expect(viewers.driven, 'nothing driven').toEqual([]);
});

/** Five fresh attachments refused after frames arrived: the page asks no more on its own. */
it('drops the kept window once the page stops asking, so Play and the cursor do not stand inert over nothing', async () => {
  vi.useFakeTimers();
  answers.next = GOT;
  render(<RunPlayback served run={run({ state: 'finished' })} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  keyframe(900_000);
  await act(() => vi.advanceTimersByTimeAsync(250));
  for (let loss = 0; loss < 5; loss += 1) await refused();
  expect(screen.getByRole('button', { name: 'Play' }).getAttribute('aria-disabled'), 'a keyframe may still come').toBe('true');
  act(() => attached.at(-1)?.options.onError?.('the run store did not answer'));
  expect(attached).toHaveLength(6);
  expect(screen.queryByRole('button', { name: 'Play' })).toBeNull();
  expect(screen.queryByRole('slider')).toBeNull();
  expect(cursorLine()).toBe('Interrupted: the run store did not answer');
});

/** The server drops the fifth fresh attachment as slow and closes its socket before the page renders the drop: the
 * player's update from the drop, made outside any event, React renders after the socket's close, and in the browser
 * that undid the drop of the kept window made as the close rendered, 1 to 3 ms of wall clock later. */
it('keeps the kept window dropped when the page stops asking at a socket the server closes', async () => {
  vi.useFakeTimers();
  answers.next = GOT;
  render(<RunPlayback served run={run({ state: 'finished' })} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  keyframe(900_000);
  await act(() => vi.advanceTimersByTimeAsync(250));
  for (let loss = 0; loss < 5; loss += 1) await refused();
  act(() => {
    viewers.window = [];
    attached.at(-1)?.options.onLagged?.(600_000);
    closeTheSocket();
  });
  await act(() => vi.advanceTimersByTimeAsync(250));
  expect(attached).toHaveLength(6);
  expect(screen.queryByRole('button', { name: 'Play' })).toBeNull();
  expect(cursorLine()).toBe('Interrupted: the streaming server closed the connection');
});

/** A fresh attachment to a run that stopped before its first slice: the server sends its end and no keyframe. */
it('drops the kept window when a fresh attachment ends without its keyframe', async () => {
  vi.useFakeTimers();
  answers.next = GOT;
  render(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  keyframe(900_000);
  await act(() => vi.advanceTimersByTimeAsync(250));
  await refused();
  expect(screen.getByRole('button', { name: 'Play' }).getAttribute('aria-disabled')).toBe('true');
  act(() => attached.at(-1)?.options.onAttached?.({ from_ms: 0, live_edge_ms: 0 }));
  act(() => attached.at(-1)?.options.onEnded?.('sim-time-limit'));
  expect(screen.queryByRole('button', { name: 'Play' })).toBeNull();
  expect(cursorLine()).toBe('Replay');
});

/** Scrubbed back with Follow off, then dropped as slow: the server resumes from a keyframe later than the cursor. */
it('states the instant the picture shows when the window it attached again for starts later than the cursor', async () => {
  vi.useFakeTimers();
  answers.next = GOT;
  const told: number[] = [];
  render(<RunPlayback served run={run()} onCursor={(ms) => told.push(ms)} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  keyframe(900_000);
  await act(() => vi.advanceTimersByTimeAsync(250));
  fireEvent.change(screen.getByRole('slider'), { target: { value: '300000' } });
  lagged();
  keyframe(960_000, 600_000);
  expect(cursorLine()).toContain('Cursor at 10 min simtime, of 10 min simtime to 16 min simtime.');
  expect(told.at(-1), 'the charts draw the same instant').toBe(600_000);
  expect(viewers.cursor, 'the viewer keeps the cursor as asked').toBe(300_000);
});

it("hands focus on the stage's Retry to the stage once the attachment it asked for draws", async () => {
  answers.next = GOT;
  render(<RunPlayback served run={run({ state: 'finished' })} />);
  await waitFor(() => expect(attached).toHaveLength(1));
  act(() => attached[0].options.onError?.('the watch ticket is not valid'));
  const retry = screen.getByRole('button', { name: 'Retry' });
  retry.focus();
  fireEvent.click(retry);
  await waitFor(() => expect(attached).toHaveLength(2));
  expect(document.activeElement, 'Retry keeps focus while the fresh attachment is made').toBe(retry);
  act(() => attached[1].options.onFrame?.('scenario', 1_024));
  expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
  expect(document.activeElement).toBe(stageOf());
});

const find = () => screen.getByRole('searchbox', { name: 'Find' });

/** The person leaves the stage's Retry while the page's own ask is due: for the page's blank space, for a search field
 * beside the stage (as the Map panel's Find stands beside its map), or for the field and on to blank space. */
it.each([
  ['on blank page', (retry: HTMLElement) => retry.blur(), () => document.body],
  ['in a search field', () => find().focus(), find],
  ['on blank page after a search field', () => {
    find().focus();
    find().blur();
  }, () => document.body],
])("leaves focus where the person put it, %s, as the stage's Retry they left goes", async (_, leave, where) => {
  vi.useFakeTimers();
  answers.next = GOT;
  render(<><input type="search" aria-label="Find" /><RunPlayback served run={run({ state: 'finished' })} /></>);
  await act(() => vi.advanceTimersByTimeAsync(10));
  act(() => attached[0].options.onError?.('the run store did not answer'));
  const retry = screen.getByRole('button', { name: 'Retry' });
  retry.focus();
  act(() => leave(retry));
  await act(() => vi.advanceTimersByTimeAsync(2_010));
  act(() => attached[1].options.onFrame?.('scenario', 1_024));
  expect(screen.queryByRole('button', { name: 'Retry' }), 'the picture came back on its own').toBeNull();
  expect(document.activeElement).toBe(where());
});

it("hands focus on the sentence's Retry to the stage of the player that takes its place", async () => {
  vi.useFakeTimers();
  answers.next = UNAVAILABLE;
  render(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  await asksAnswered(7);
  const retry = screen.getByRole('button', { name: 'Retry' });
  retry.focus();
  answers.next = GOT;
  fireEvent.click(retry);
  await act(() => vi.advanceTimersByTimeAsync(10));
  expect(attached).toHaveLength(1);
  expect(document.activeElement).toBe(stageOf());
});

/** The page's fifth ask of its own is in flight before any ticket, so the sentence offers Retry, and the run ends: the
 * five are given back, and Retry leaves with no stage drawn to take focus. */
it("hands focus on the sentence's Retry to the sentence as the run's ending takes Retry away, then to the stage", async () => {
  vi.useFakeTimers();
  answers.next = UNAVAILABLE;
  const { rerender } = render(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  await asksAnswered(4);
  const answer = holdTheNextAsk();
  await asksAnswered(1);
  expect(asked, 'the first ask, then five of its own, the last in flight').toHaveLength(6);
  const retry = screen.getByRole('button', { name: 'Retry' });
  retry.focus();
  rerender(<RunPlayback served run={run({ state: 'finished' })} />);
  expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
  const sentence = document.querySelector('#run-playback-none');
  expect(sentence?.textContent).toBe('No playback: The streaming server did not answer. Asking again.');
  expect(document.activeElement).toBe(sentence);
  await act(async () => answer(GOT));
  await act(() => vi.advanceTimersByTimeAsync(10));
  expect(attached).toHaveLength(1);
  expect(document.activeElement).toBe(stageOf());
});

it('hands focus on Follow the live edge to Play as the run ends', async () => {
  vi.useFakeTimers();
  answers.next = GOT;
  const { rerender } = render(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  keyframe(900_000);
  screen.getByRole('checkbox', { name: 'Follow the live edge' }).focus();
  rerender(<RunPlayback served run={run({ state: 'finished' })} />);
  expect(screen.queryByRole('checkbox')).toBeNull();
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Play' }));
});

it('hands focus on Play to the stage as the kept window goes', async () => {
  vi.useFakeTimers();
  answers.next = GOT;
  render(<RunPlayback served run={run({ state: 'finished' })} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  keyframe(900_000);
  await act(() => vi.advanceTimersByTimeAsync(250));
  screen.getByRole('button', { name: 'Play' }).focus();
  for (let loss = 0; loss < 5; loss += 1) await refused();
  act(() => attached.at(-1)?.options.onError?.('the run store did not answer'));
  expect(screen.queryByRole('button', { name: 'Play' })).toBeNull();
  expect(document.activeElement).toBe(stageOf());
});
