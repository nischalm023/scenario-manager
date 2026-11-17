// The run playback's watch (live frames, 26 October 2026), through the playback, against a stand-in API, viewer and
// frames attachment (`testing/playback.ts`), on how a loss is answered: a run no server can serve is one sentence and
// no socket, asked again once it is given its streaming server; a lost attachment, or an ask for a fresh ticket refused
// for a moment, is asked again 2 s on, by the same player and on the same viewer; Retry asks at once, the stage offering
// it after a closed socket too; an attachment that stands goes on through the page's reckoning of a server changing
// and through an ask Retry made as it was accepted, failed or answered; and a refusal means what the page reckons as
// it arrives, however React schedules the page's effects, of which only a kept one's answer is heard. How many asks the page makes on its own, and what gives them
// back, is `playbackWatch.budget.test.tsx`; every sequence of what the watch hears is walked in
// `playbackRecovery.test.ts`.

import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { StrictMode, useLayoutEffect } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { RunDetail } from '../../protocol/runs.ts';
import {
  answers, asked, attached, badge, GOT, holdTheNextAsk, NOT_STREAMED, OFFLINE, refused, resetPlayback, run, RUN, TICKET,
  UNAVAILABLE, viewers,
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

const none = () => document.querySelector('#run-playback-none')?.textContent;

it('says so, and opens no socket, when the run has no live stream', async () => {
  answers.next = NOT_STREAMED;
  render(<RunPlayback served={false} run={run()} />);
  await waitFor(() => expect(none()).toBe('No streaming server is up for this run, so there is no playback to show.'));
  expect(asked).toEqual([`/api/runs/${RUN}/watch`]);
  expect(attached).toEqual([]);
});

/** In development React runs a page's effects twice, the first time cleaned up at once, so the first ask is sent twice. */
it("attaches with the ticket the ask React kept brought, not the one its cleaned-up first run's request brought", async () => {
  answers.next = GOT;
  render(<StrictMode><RunPlayback served run={run()} /></StrictMode>);
  await waitFor(() => expect(attached).toHaveLength(1));
  expect(asked).toHaveLength(2);
  expect(attached[0].options.token).toBe('the-ticket-2');
});

it('asks again once a run waiting for a seat is given its streaming server', async () => {
  answers.next = NOT_STREAMED;
  const { rerender } = render(<RunPlayback served={false} run={run({ state: 'queued' })} />);
  await waitFor(() => expect(none()).toBe('No streaming server is up for this run, so there is no playback to show.'));
  answers.next = GOT;
  const dispatched = [{ n: 1, state: 'queued', streaming_server: '01KTJ153M7D109TG4G7BBVCZS1' }] as unknown as RunDetail['attempts'];
  rerender(<RunPlayback served run={run({ state: 'queued', attempts: dispatched })} />);
  await waitFor(() => expect(attached).toHaveLength(1));
  expect(asked).toHaveLength(2);
});

/** The Runs panel hands the page its run anew every 2 s, which must not put the retry off. */
it('asks for a fresh ticket 2 s after an interruption, however often the page is handed the run anew', async () => {
  vi.useFakeTimers();
  answers.next = GOT;
  const { rerender } = render(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  expect(attached).toHaveLength(1);
  act(() => attached[0].options.onError?.("this attempt's picture is nowhere this server can reach"));
  for (let read = 0; read < 5; read += 1) {
    rerender(<RunPlayback served run={run()} />);
    await act(() => vi.advanceTimersByTimeAsync(500));
  }
  expect(asked).toHaveLength(2);
  expect(attached).toHaveLength(2);
});

/** Each ask once made a new player: a new viewer and socket every 2 s, and focus in the old one fell to the page. */
it('asks again on the same player: one viewer, its stage keeping focus, each fresh ticket attached in turn', async () => {
  vi.useFakeTimers();
  answers.next = GOT;
  render(<RunPlayback served run={run({ progress: { sim_ms: 0, values: [] } })} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  const stage = document.querySelector<HTMLElement>('.worldview-mount__stage');
  stage?.focus();
  for (let refusal = 0; refusal < 3; refusal += 1) await refused();
  expect(viewers.opened).toBe(1);
  expect(viewers.cleared, 'each attachment starts the viewer afresh').toBe(4);
  expect(document.activeElement).toBe(stage);
  expect(attached.map((one) => [one.options.token, one.closed])).toEqual([
    ['the-ticket-1', true], ['the-ticket-2', true], ['the-ticket-3', true], ['the-ticket-4', false],
  ]);
});

/** A streaming server restarting goes stale, so the API answers `not-streamed` for its attempt for a moment. */
it.each([
  ['answers not-streamed for an attempt a server serves', NOT_STREAMED, 'Attempt 1 of this run has no live stream'],
  ['answers 503', UNAVAILABLE, 'The streaming server did not answer'],
  ['cannot be reached', OFFLINE, 'the network failed (fetch failed)'],
])('keeps its player through a fresh ticket refused for a moment, and asks again 2 s on, when the API %s', async (_, refusal, why) => {
  vi.useFakeTimers();
  answers.next = GOT;
  render(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  const stage = document.querySelector('.worldview-mount__stage');
  answers.next = refusal;
  await refused();
  expect(asked, 'the page asked again, and was refused').toHaveLength(2);
  expect(document.querySelector('.worldview-mount__stage'), 'the same player stands').toBe(stage);
  expect(badge()).toBe(`Interrupted: ${why}`);
  answers.next = GOT;
  await act(() => vi.advanceTimersByTimeAsync(2_010));
  expect(attached.map((one) => one.options.token), 'asked again, and attached').toEqual(['the-ticket-1', 'the-ticket-3']);
  expect(viewers.opened).toBe(1);
});

it("asks for a fresh ticket when someone presses the stage's Retry, since the one it held lives a minute", async () => {
  answers.next = GOT;
  render(<RunPlayback served run={run({ state: 'finished' })} />);
  await waitFor(() => expect(attached).toHaveLength(1));
  act(() => attached[0].options.onError?.('the watch ticket is not valid'));
  answers.next = { ok: true, status: 200, value: { ...TICKET, token: 'a-fresh-ticket' } };
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  await waitFor(() => expect(attached).toHaveLength(2));
  expect(asked).toHaveLength(2);
  expect(attached[1].options.token).toBe('a-fresh-ticket-2');
  expect(viewers.opened, 'the same player attaches with it').toBe(1);
});

it('offers Retry on the stage once the streaming server closes the socket', async () => {
  answers.next = GOT;
  render(<RunPlayback served run={run({ state: 'finished' })} />);
  await waitFor(() => expect(attached).toHaveLength(1));
  act(() => attached[0].options.onFrame?.('scenario', 1_024));
  expect(screen.queryByRole('button', { name: 'Retry' })).toBeNull();
  act(() => attached[0].options.onClose?.());
  expect(badge()).toBe('Interrupted: the streaming server closed the connection');
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  await waitFor(() => expect(attached).toHaveLength(2));
});

/** The page reckons a server from the run's attempts and recording, re-read every 2 s, while a picture plays. */
it("keeps its attachment, and asks nothing, as the page's reckoning of a server changes while a ticket stands", async () => {
  vi.useFakeTimers();
  answers.next = GOT;
  const { rerender } = render(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  rerender(<RunPlayback served={false} run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  rerender(<RunPlayback served run={run()} />);
  await act(() => vi.advanceTimersByTimeAsync(2_010));
  expect(asked).toHaveLength(1);
  expect(attached.map((one) => one.closed)).toEqual([false]);
});

/** Retry pressed in the moment a fresh attachment is accepted, before its floor is drawn, while the stage still shows
 * the earlier failure and its Retry. */
it('lets an accepted attachment play on, uninterrupted, when an ask Retry made as it was accepted fails', async () => {
  vi.useFakeTimers();
  answers.next = GOT;
  render(<RunPlayback served run={run({ state: 'finished' })} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  act(() => attached[0].options.onError?.('the run store did not answer'));
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  await act(() => vi.advanceTimersByTimeAsync(10));
  act(() => attached[1].options.onAttached?.({ from_ms: 0, live_edge_ms: 3_600_000 }));
  answers.next = UNAVAILABLE;
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  await act(() => vi.advanceTimersByTimeAsync(2_010));
  expect(asked, 'the first ask, then the two Retry made').toHaveLength(3);
  expect(badge()).toBe('Replay');
  expect(attached.map((one) => one.closed)).toEqual([true, false]);
});

/** As above, but the accepted attachment is lost while Retry's ask is in flight, and Retry's ticket then brings a
 * picture that plays: the loss heard while an ask was in flight once started the 2 s pause too, whose ask, in flight
 * as that picture was accepted, replaced it. */
it("lets the picture Retry's ticket brought play on when the attachment before it was lost as Retry asked", async () => {
  vi.useFakeTimers();
  answers.next = GOT;
  render(<RunPlayback served run={run({ state: 'finished' })} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  act(() => attached[0].options.onError?.('the run store did not answer'));
  await act(() => vi.advanceTimersByTimeAsync(2_010));
  act(() => attached[1].options.onAttached?.({ from_ms: 0, live_edge_ms: 3_600_000 }));
  const retried = holdTheNextAsk();
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  act(() => attached[1].options.onError?.('the viewer refused a frame: a delta out of order'));
  await act(() => vi.advanceTimersByTimeAsync(1_000));
  await act(async () => retried(GOT));
  await act(() => vi.advanceTimersByTimeAsync(10));
  const late = holdTheNextAsk();
  await act(() => vi.advanceTimersByTimeAsync(1_000));
  act(() => attached[2].options.onAttached?.({ from_ms: 0, live_edge_ms: 3_600_000 }));
  act(() => attached[2].options.onFrame?.('scenario', 1_024));
  await act(async () => late(GOT));
  await act(() => vi.advanceTimersByTimeAsync(2_010));
  expect(asked, 'the first ask, the one 2 s after the first loss, and Retry').toHaveLength(3);
  expect(attached.map((one) => one.closed)).toEqual([true, true, false]);
  expect(badge()).toBe('Replay');
});

/** Retry pressed while the fresh attachment made after a loss is attaching: the server accepts that attachment after
 * the page took Retry's ticket and before the player, rendering, closed it and began the next. */
it("takes no word of the attachment Retry's ticket replaces for the one that ticket begins", async () => {
  vi.useFakeTimers();
  answers.next = GOT;
  render(<RunPlayback served run={run({ state: 'finished' })} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  act(() => attached[0].options.onError?.('the run store did not answer'));
  await act(() => vi.advanceTimersByTimeAsync(2_010));
  const retried = holdTheNextAsk();
  fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
  await act(async () => {
    retried(GOT);
    for (let tick = 0; tick < 10; tick += 1) await Promise.resolve();
    attached[1].options.onAttached?.({ from_ms: 0, live_edge_ms: 3_600_000 });
  });
  expect(attached.map((one) => one.closed)).toEqual([true, true, false]);
  expect(badge(), 'interrupted until the attachment Retry asked for is accepted').toBe('Interrupted: the run store did not answer');
});

/** Polls until `holds`, on the wall clock, as a page with no test's hand on React's scheduling does. */
async function until(holds: () => boolean) {
  for (let wait = 0; wait < 600 && !holds(); wait += 1) await new Promise((later) => setTimeout(later, 10));
  expect(holds()).toBe(true);
}

/** In a page, React runs a render's effects in a later task when the render took long, and an answer can arrive
 * between: the first ask's `not-streamed` came after the render that handed the page its server and before that
 * render's effects, was taken for "no server can serve it", and the page asked no more. React's own scheduling is
 * left to run here, and the render is made long. */
it("asks again when the page reckons a server as the first ask is in flight, whenever that ask's refusal comes", async () => {
  const actEnvironment = Reflect.get(globalThis, 'IS_REACT_ACT_ENVIRONMENT');
  Reflect.set(globalThis, 'IS_REACT_ACT_ENVIRONMENT', false);
  const host = document.body.appendChild(document.createElement('div'));
  const root = createRoot(host);
  try {
    const answer = holdTheNextAsk();
    /** As it commits the render that hands the page its server, the API refuses the first ask, the render running
     * long enough that React leaves the render's effects for a later task. */
    function Refusing({ served }: { served: boolean }) {
      useLayoutEffect(() => {
        if (!served) return;
        for (const late = performance.now() + 20; performance.now() < late;) { /* a long render */ }
        answer(NOT_STREAMED);
      }, [served]);
      return null;
    }
    const page = (served: boolean) => <><RunPlayback served={served} run={run({ state: 'queued' })} /><Refusing served={served} /></>;
    root.render(page(false));
    await until(() => asked.length === 1);
    answers.next = GOT;
    root.render(page(true));
    await until(() => none() !== "Asking for the run's stream.");
    expect(none()).toBe('No playback: Attempt 1 of this run has no live stream. Asking again.');
    await until(() => attached.length === 1);
    expect(asked).toHaveLength(2);
  } finally {
    root.unmount();
    host.remove();
    Reflect.set(globalThis, 'IS_REACT_ACT_ENVIRONMENT', actEnvironment);
  }
});
