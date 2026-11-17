// Test support for the unit tests of the run page's playback (`RunPlayback`) and its watch (`playbackWatch.ts`):
// stand-ins for `crucible-viewer`'s viewer and frames attachment and for the API's watch ticket, which a test file
// mocks those modules with, the state the stand-ins keep for a test to drive and read, and the steps the tests share.
// A canvas draws nothing in jsdom, so the viewer holds a window from a keyframe until it is cleared, and its cursor as
// asked; the API mints a fresh token at each ask, as the API does.

import { act } from '@testing-library/react';
import { vi } from 'vitest';
import type { RunDetail, WatchTicket } from '../protocol/runs.ts';
import type { Answer } from '../shell/api.ts';

/** What the page hands `attachFrames`: the ticket's token, and what a test says as the server would. */
export type AttachOptions = {
  token?: string;
  onAttached?(said: { from_ms: number; live_edge_ms: number }): void;
  onFrame?(tag: string, bytes: number): void;
  onLagged?(resume_from_ms: number): void;
  onEnded?(outcome: string): void;
  onError?(message: string): void;
  onClose?(): void;
};

/** Every attachment made, in order, and whether the page closed it. */
export const attached: { url: string; options: AttachOptions; closed: boolean }[] = [];
/** Every ask for a ticket, by its path. */
export const asked: string[] = [];
/** The API's answer to the next ask. */
export const answers = { next: null as Answer<WatchTicket> | null };
/** The viewer: how many opened and cleared, its window and cursor, and what drove it. */
export const viewers = { opened: 0, cleared: 0, window: [] as number[], cursor: 0, driven: [] as string[] };

/** `crucible-viewer` as the playback uses it. */
export function viewerModule() {
  const replies: Record<string, unknown> = {
    frame: () => true,
    has_floor: () => false,
    camera: () => new Float32Array(6),
    centre_entities: () => [],
    centre_mm: () => new Float64Array(0),
    held_window: () => viewers.window,
    cursor_ms: () => viewers.cursor,
    playing: () => false,
    clear: () => {
      viewers.cleared += 1;
      viewers.window = [];
    },
    play: () => {
      viewers.driven.push('play');
    },
    pause: () => {
      viewers.driven.push('pause');
    },
    set_cursor: (ms: number) => {
      viewers.driven.push(`cursor at ${ms}`);
      viewers.cursor = ms;
    },
  };
  const standIn = new Proxy({}, { get: (_, name) => (name === 'then' ? undefined : replies[String(name)] ?? (() => undefined)) });
  return {
    openFloorViewer: async () => {
      viewers.opened += 1;
      return standIn;
    },
    readPalette: () => ({}),
    attachFrames: (url: string, _viewer: unknown, options: AttachOptions) => {
      const attachment = { url, options, closed: false };
      attached.push(attachment);
      return { close: () => (attachment.closed = true) };
    },
  };
}

/** The answer the next ask waits for, once `holdTheNextAsk` is called. */
let held: Promise<Answer<WatchTicket>> | null = null;

/** The next ask waits, in flight, until the function this hands back is called with its answer. */
export function holdTheNextAsk(): (answer: Answer<WatchTicket>) => void {
  let give: (answer: Answer<WatchTicket>) => void = () => undefined;
  held = new Promise((resolve) => (give = resolve));
  return give;
}

/** `postJson` as the watch asks for a ticket: `answers.next`, or the held answer, its token suffixed with the ask's
 * number. */
export async function postJson(path: string) {
  asked.push(path);
  const number = asked.length;
  const waited = held;
  held = null;
  const next = waited === null ? answers.next : await waited;
  return next?.ok ? { ...next, value: { ...next.value, token: `${next.value.token}-${number}` } } : next;
}

/** Before each test: nothing attached or asked, a fresh viewer, and the browser's frame, size and media stand-ins. */
export function resetPlayback() {
  attached.length = 0;
  asked.length = 0;
  held = null;
  Object.assign(viewers, { opened: 0, cleared: 0, window: [], cursor: 0, driven: [] });
  vi.stubGlobal('requestAnimationFrame', () => 0);
  vi.stubGlobal('cancelAnimationFrame', () => undefined);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
}

export const RUN = '01KTJ153M7D109TG4G7BBVCZYW';
export const FRAMES = `ws://127.0.0.1:3202/frames/${RUN}/1`;
export const TICKET: WatchTicket = {
  run: RUN, attempt: 1, url: `ws://127.0.0.1:3202/streams/${RUN}/1`, frames_url: FRAMES, metrics_url: `ws://127.0.0.1:3202/metrics/${RUN}/1`, token: 'the-ticket',
  expires_at: '2026-10-27T12:01:00Z',
};
export const GOT: Answer<WatchTicket> = { ok: true, status: 200, value: TICKET };
export const NOT_STREAMED: Answer<WatchTicket> = {
  ok: false, status: 409, reason: 'Attempt 1 of this run has no live stream.',
  problem: { type: 'not-streamed', title: 'Not streamed', status: 409, detail: 'Attempt 1 of this run has no live stream.' } as never,
};
export const UNAVAILABLE: Answer<WatchTicket> = {
  ok: false, status: 503, reason: 'The streaming server did not answer.',
  problem: { type: 'unavailable', title: 'Unavailable', status: 503, detail: 'The streaming server did not answer.' } as never,
};
export const OFFLINE: Answer<WatchTicket> = { ok: false, status: null, problem: null, reason: 'the network failed (fetch failed)' };

/** Peak hour, running at 20 min simtime, unless `more` says otherwise. */
export function run(more: Partial<RunDetail> = {}): RunDetail {
  return {
    id: RUN, ref: `run:${RUN}`, label: 'Peak hour', state: 'running', attempt: 1,
    progress: { sim_ms: 1_200_000, values: [] }, recording: { stored: false }, findings: [], attempts: [],
    ...more,
  } as unknown as RunDetail;
}

export const badge = () => document.querySelector('#run-playback-mode .pill')?.textContent;
export const cursorLine = () => document.querySelector('#run-playback-mode')?.textContent;
export const stageOf = () => document.querySelector('.worldview-mount__stage');

/** The server refuses the newest attachment, as it does before it holds the run's first frame, and 2 s pass. */
export async function refused() {
  act(() => attached.at(-1)?.options.onError?.("this attempt's picture is nowhere this server can reach"));
  await act(() => vi.advanceTimersByTimeAsync(2_010));
}

/** The page's own asks, `times` of them 2 s apart, each answered as `answers.next` says. */
export async function asksAnswered(times: number) {
  for (let ask = 0; ask < times; ask += 1) await act(() => vi.advanceTimersByTimeAsync(2_010));
}

/** The newest attachment's keyframe, as `attachFrames` pushes it: the viewer's window opens from `from_ms` to
 * `to_ms`. */
export function keyframe(to_ms: number, from_ms = 0) {
  viewers.window = [from_ms, to_ms];
  act(() => attached.at(-1)?.options.onFrame?.('keyframe', 512));
}

/** The newest attachment accepted and its keyframe in, the viewer's window open to `to_ms`, then the viewer refusing
 * the frame after, and 2 s pass. */
export async function replayFailsAt(to_ms: number) {
  act(() => attached.at(-1)?.options.onAttached?.({ from_ms: 0, live_edge_ms: 3_600_000 }));
  keyframe(to_ms);
  act(() => attached.at(-1)?.options.onError?.('the viewer refused a frame: a delta out of order'));
  await act(() => vi.advanceTimersByTimeAsync(2_010));
}

/** The server closes the newest attachment's socket, inside a test's `act`. A browser fires a socket's `close` as it
 * fires a dialog's, so React renders what its listener changes at once, ahead of updates made outside any event and
 * not yet rendered; a stand-in element's `close` does the same here. */
export function closeTheSocket() {
  const socket = document.createElement('span');
  socket.addEventListener('close', () => attached.at(-1)?.options.onClose?.());
  socket.dispatchEvent(new Event('close'));
}

/** The server drops the newest attachment as too slow: `attachFrames` clears the viewer and attaches again. */
export function lagged() {
  viewers.window = [];
  act(() => attached.at(-1)?.options.onLagged?.(600_000));
}
