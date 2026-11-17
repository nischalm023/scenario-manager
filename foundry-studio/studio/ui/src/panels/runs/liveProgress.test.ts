// A running attempt's live numbers: the watch speaks the stream's format, and a socket the server closes is said in
// words, never by the refusal's code. The API and the socket are stand-ins.

import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { WatchTicket } from '../../protocol/runs.ts';
import { useLiveProgress } from './liveProgress.ts';

const RUN = '01KTJ153M7D109TG4G7BBVCZYW';
const TICKET: WatchTicket = {
  run: RUN, attempt: 1, url: `ws://127.0.0.1:3202/streams/${RUN}/1`, frames_url: `ws://127.0.0.1:3202/frames/${RUN}/1`, metrics_url: `ws://127.0.0.1:3202/metrics/${RUN}/1`,
  token: 'the-ticket', expires_at: '2026-10-27T12:01:00Z',
};

vi.mock('../../shell/api.ts', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../shell/api.ts')>()),
  postJson: async () => ({ ok: true, status: 200, value: TICKET }),
}));

/** The socket the hook opens, which the test drives. */
class Socket {
  static opened: Socket[] = [];
  sent: unknown[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: ((event: { reason: string }) => void) | null = null;
  constructor() {
    Socket.opened.push(this);
  }
  send(text: string) {
    this.sent.push(JSON.parse(text));
  }
  close() {}
}

beforeEach(() => {
  Socket.opened = [];
  vi.stubGlobal('WebSocket', Socket);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

it("watches in the stream's format, and says a refusal in words rather than its code", async () => {
  const { result } = renderHook(() => useLiveProgress(RUN, 1, true));
  await waitFor(() => expect(Socket.opened).toHaveLength(1));
  const socket = Socket.opened[0];
  act(() => socket.onopen?.());
  expect(socket.sent).toEqual([{ type: 'watch', v: 2, token: 'the-ticket' }]);
  act(() => socket.onclose?.({ reason: 'unsupported-version' }));
  expect(result.current).toEqual({ state: 'lost', why: 'the streaming server speaks another version of the stream' });
});
