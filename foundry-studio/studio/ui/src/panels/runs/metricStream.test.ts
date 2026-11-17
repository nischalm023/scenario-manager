// A metric stream as the page holds it: the server's messages append rows in place and never revise them, `attached`
// names the descriptor and starts the history afresh, `ended` closes it with the outcome, `lagged` marks it for a fresh
// attach; a close reason reads as words; a socket lost before the server first answers attaches again on its own, and
// one lost after it a few times, but never one the server refused for good (the protocol's `VIEWER_FINAL`), nor a
// ticket the API refused `not-streamed` once the run has ended, nor one closed after `ended`; a new try or a new attempt attaches afresh at
// once; what came is shown a few times a second of wall clock at most. The chart a page draws of the rows is
// `charts.ts`'s, tested beside it.

import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MetricDescriptor, MetricPoint } from '../../protocol/metric_stream.ts';
import { VIEWER_FINAL } from '../../protocol/run_stream_rules.ts';
import type { WatchTicket } from '../../protocol/runs.ts';
import type { Answer } from '../../shell/api.ts';
import { closedWords, freshStream, takeMessage, UNATTACHED_STREAM, useMetricStream } from './metricStream.ts';

const TICKET: WatchTicket = {
  run: '01KZAVPQNES65HV4EZADPCS7E8', attempt: 1, token: 'a-ticket', expires_at: '2026-11-06T12:00:00Z',
  url: 'ws://streams.example/streams/01KZAVPQNES65HV4EZADPCS7E8/1',
  frames_url: 'ws://streams.example/frames/01KZAVPQNES65HV4EZADPCS7E8/1',
  metrics_url: 'ws://streams.example/metrics/01KZAVPQNES65HV4EZADPCS7E8/1',
};

/** The API's answer to each ask for a ticket, which a test may set; a ticket by default. */
const tickets = vi.hoisted(() => ({ asked: 0, answer: null as Answer<unknown> | null }));

vi.mock('../../shell/api.ts', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../shell/api.ts')>()),
  postJson: async () => {
    tickets.asked += 1;
    return tickets.answer ?? { ok: true, status: 200, value: TICKET };
  },
}));

/** The socket the hook opens, which the test drives. */
class Socket {
  static opened: Socket[] = [];
  sent: unknown[] = [];
  closed = false;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onclose: ((event: { reason: string }) => void) | null = null;
  constructor() {
    Socket.opened.push(this);
  }
  send(text: string) {
    this.sent.push(JSON.parse(text));
  }
  close() {
    this.closed = true;
  }
  /** The server's message, as text. */
  says(message: object) {
    this.onmessage?.({ data: JSON.stringify(message) });
  }
}

beforeEach(() => {
  Socket.opened = [];
  Object.assign(tickets, { asked: 0, answer: null });
  vi.stubGlobal('WebSocket', Socket);
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const DESCRIPTOR: MetricDescriptor = {
  metric: 'picks_per_hour', label: 'Picks/hr (rate)', description: 'The slice\'s picks as a rate per simulated hour.',
  unit: 'rate_per_hour', clock: 'sim', aggregation: 'snapshot_gauge', source: 'scalars',
  bucket: { kind: 'trailing', window_sim_ms: 250_000, step_sim_ms: 10_000 },
};

const row = (to: number, value: number | null): MetricPoint => ({ from_sim_ms: to - 10_000, to_sim_ms: to, value });

describe('a metric stream as the page holds it', () => {
  it('appends rows in place, never revises them, and closes with the outcome', () => {
    let stream = takeMessage(UNATTACHED_STREAM, { type: 'attached', v: 1, run: 'r', attempt: 1, descriptor: DESCRIPTOR, reached_sim_ms: 20_000, live: true });
    expect(stream.descriptor?.metric).toBe('picks_per_hour');
    expect(stream.live).toBe(true);
    const held = stream.rows;
    stream = takeMessage(stream, { type: 'rows', v: 1, rows: [row(10_000, 360), row(20_000, 720)] });
    const shown = stream;
    stream = takeMessage(stream, { type: 'rows', v: 1, rows: [row(30_000, null)] });
    expect(stream.rows.map((one) => one.value)).toEqual([360, 720, null]);
    expect(stream.rows, 'the history is never copied').toBe(held);
    expect(stream, 'but each batch is a stream to show anew').not.toBe(shown);
    expect(takeMessage(stream, { type: 'rows', v: 1, rows: [] })).toBe(stream);
    stream = takeMessage(stream, { type: 'ended', v: 1, outcome: 'sim-time-limit' });
    expect(stream.ended).toBe('sim-time-limit');
    expect(stream.live).toBe(false);
    expect(stream.rows).toHaveLength(3);
  });

  it('starts the history afresh at each attach, which sends it again from simtime 0', () => {
    let stream = takeMessage(freshStream(), { type: 'rows', v: 1, rows: [row(10_000, 360)] });
    stream = { ...stream, lost: { why: 'the stream fell silent', next: 'again' } };
    stream = takeMessage(stream, { type: 'attached', v: 1, run: 'r', attempt: 1, descriptor: DESCRIPTOR, reached_sim_ms: 10_000, live: true });
    expect([stream.rows, stream.lost]).toEqual([[], null]);
    expect(() => takeMessage(UNATTACHED_STREAM, { type: 'rows', v: 1, rows: [row(10_000, 1)] }), 'the shown empty stream is no one\'s').toThrow();
  });

  it('marks a lagged stream and words a close reason', () => {
    const lagged = takeMessage(UNATTACHED_STREAM, { type: 'lagged', v: 1 });
    expect(lagged.lagged).toBe(true);
    expect(closedWords('unknown-metric')).toBe('the streaming server does not compute this metric');
    expect(closedWords('token-invalid')).toBe('the streaming server refused the watch ticket');
    expect(closedWords('unsupported-trailing')).toBe('the streaming server does not cut this metric into the windows asked for');
    expect(closedWords('not-recorded')).toBe("the run's recording does not hold what this metric is computed from");
    expect(closedWords('recording-unreadable')).toBe("the run's recording does not read, so this metric cannot be computed from it");
    expect(closedWords('unavailable')).toBe('something the streaming server needs did not answer: its database, its disk or the run store');
    expect(closedWords('no-recording')).toBe('no streaming server can serve this attempt: none holds its stream, and no recording of it is stored');
    expect(closedWords('')).toBe('the streaming server closed the connection');
    expect(closedWords('going away')).toBe('going away');
  });
});

describe('a metric socket as the page shows it', () => {
  it('shows what came at most once in a while of wall clock, every batch in that while together', async () => {
    const { result } = renderHook(() => useMetricStream(TICKET.run, 1, 'picks_per_hour'));
    await act(() => vi.advanceTimersByTimeAsync(10));
    act(() => Socket.opened[0].says({ type: 'attached', v: 1, run: TICKET.run, attempt: 1, descriptor: DESCRIPTOR, reached_sim_ms: 0, live: true }));
    act(() => Socket.opened[0].says({ type: 'rows', v: 1, rows: [row(10_000, 360)] }));
    await act(() => vi.advanceTimersByTimeAsync(300));
    act(() => Socket.opened[0].says({ type: 'rows', v: 1, rows: [row(20_000, 720)] }));
    expect(result.current.descriptor, 'nothing shown yet').toBeNull();
    await act(() => vi.advanceTimersByTimeAsync(150));
    expect(result.current.rows.map((one) => one.value), 'both batches, shown once').toEqual([360, 720]);
  });
});

describe('a lost metric socket', () => {
  const attached = { type: 'attached', v: 1, run: TICKET.run, attempt: 1, descriptor: DESCRIPTOR, reached_sim_ms: 10_000, live: true };
  /** Until the ticket's promise and the hook's show have run. */
  const untilShown = async () => act(async () => {
    await vi.advanceTimersByTimeAsync(500);
  });

  it('attaches again without end until the server first answers, then a few times, then waits for a try', async () => {
    const { result } = renderHook(() => useMetricStream(TICKET.run, 1, 'picks_per_hour'));
    await untilShown();
    for (let lost = 1; lost <= 8; lost += 1) {
      act(() => Socket.opened.at(-1)?.onclose?.({ reason: 'not-held' }));
      await untilShown();
      expect(result.current.lost?.next, 'before any attach, always again').toBe('again');
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2_000);
      });
      expect(Socket.opened).toHaveLength(lost + 1);
    }
    act(() => Socket.opened.at(-1)?.says(attached));
    act(() => Socket.opened.at(-1)?.says({ type: 'rows', v: 1, rows: [row(10_000, 360)] }));
    await untilShown();
    expect(result.current.rows).toHaveLength(1);
    for (let lost = 1; lost <= 5; lost += 1) {
      act(() => Socket.opened.at(-1)?.onclose?.({ reason: '' }));
      await untilShown();
      expect(result.current.lost).toEqual({ why: 'the streaming server closed the connection', next: 'again' });
      expect(result.current.rows, 'what came stays standing').toHaveLength(1);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2_000);
      });
      act(() => Socket.opened.at(-1)?.says(attached));
      act(() => Socket.opened.at(-1)?.says({ type: 'rows', v: 1, rows: [row(10_000, 360)] }));
    }
    const opened = Socket.opened.length;
    act(() => Socket.opened.at(-1)?.onclose?.({ reason: '' }));
    await untilShown();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect([result.current.lost?.next, Socket.opened.length], 'its tries spent, it waits for the page').toEqual(['on-request', opened]);
  });

  it('never attaches again after a refusal the protocol lists as final for a viewer, even before the server first answered', async () => {
    expect(VIEWER_FINAL.has('no-recording'), 'an ended attempt no server can serve').toBe(true);
    for (const reason of VIEWER_FINAL) {
      Socket.opened = [];
      tickets.asked = 0;
      const { result, unmount } = renderHook(() => useMetricStream(TICKET.run, 1, 'picks_per_hour'));
      await untilShown();
      act(() => Socket.opened[0].onclose?.({ reason }));
      await untilShown();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10_000);
      });
      expect([result.current.lost, tickets.asked, Socket.opened.length], reason).toEqual([{ why: closedWords(reason), next: 'final' }, 1, 1]);
      unmount();
    }
  });

  it('asks once for a ticket the API refuses as not streamed for a run that has ended, and goes on asking after any other refusal', async () => {
    const detail = 'Attempt 1 of this run has no live stream: no streaming server was alive when it was dispatched.';
    tickets.answer = { ok: false, status: 409, problem: { type: 'not-streamed', title: 'Not streamed', status: 409, detail }, reason: detail };
    const { result, unmount } = renderHook(() => useMetricStream(TICKET.run, 1, 'picks_per_hour'));
    await untilShown();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect([result.current.lost, tickets.asked, Socket.opened.length]).toEqual([{ why: detail.slice(0, -1), next: 'final' }, 1, 0]);
    unmount();
    tickets.asked = 0;
    tickets.answer = { ok: false, status: null, problem: null, reason: 'the network did not answer' };
    const again = renderHook(() => useMetricStream(TICKET.run, 1, 'picks_per_hour'));
    await untilShown();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(4_100);
    });
    expect([again.result.current.lost, tickets.asked]).toEqual([{ why: 'the network did not answer', next: 'again' }, 3]);
  });

  it('asks again for a ticket refused as not streamed while the run goes, as while its streaming server restarts, and stops once the run has ended', async () => {
    const detail = 'Attempt 1 of this run has no live stream: its streaming server has not been heard from.';
    tickets.answer = { ok: false, status: 409, problem: { type: 'not-streamed', title: 'Not streamed', status: 409, detail }, reason: detail };
    const { result, rerender } = renderHook(({ goes }) => useMetricStream(TICKET.run, 1, 'picks_per_hour', 0, goes), { initialProps: { goes: true } });
    await untilShown();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(4_100);
    });
    expect([result.current.lost?.next, tickets.asked], 'asked again while the run goes').toEqual(['again', 3]);
    rerender({ goes: false });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect([result.current.lost?.next, tickets.asked], 'one more ask, refused for good').toEqual(['final', 4]);
  });

  it('attaches afresh at once on a new try, and a lagged socket says so', async () => {
    const { result, rerender } = renderHook(({ tries }) => useMetricStream(TICKET.run, 1, 'picks_per_hour', tries), { initialProps: { tries: 0 } });
    await untilShown();
    act(() => Socket.opened[0].says(attached));
    act(() => Socket.opened[0].says({ type: 'lagged', v: 1 }));
    act(() => Socket.opened[0].onclose?.({ reason: '' }));
    await untilShown();
    expect(result.current.lost?.why).toBe('the page fell behind the stream');
    rerender({ tries: 1 });
    await untilShown();
    expect(Socket.opened).toHaveLength(2);
    expect(Socket.opened[1].sent, 'not yet open').toEqual([]);
    act(() => Socket.opened[1].onopen?.());
    expect(Socket.opened[1].sent).toEqual([{ type: 'attach', v: 1, metric: 'picks_per_hour', token: 'a-ticket' }]);
  });

  it("attaches afresh at once for the run's new attempt, closing the old attempt's socket", async () => {
    const { result, rerender } = renderHook(({ attempt }) => useMetricStream(TICKET.run, attempt, 'picks_per_hour'), { initialProps: { attempt: 1 } });
    await untilShown();
    act(() => Socket.opened[0].says(attached));
    await untilShown();
    expect(result.current.descriptor?.metric).toBe('picks_per_hour');
    rerender({ attempt: 2 });
    expect(result.current.descriptor, "the old attempt's stream is not shown as the new one's").toBeNull();
    await untilShown();
    expect([Socket.opened.length, Socket.opened[0].closed]).toEqual([2, true]);
  });

  it('opens no other socket once the server said the stream ended, however the socket then closes', async () => {
    const { result } = renderHook(() => useMetricStream(TICKET.run, 1, 'picks_per_hour'));
    await untilShown();
    act(() => Socket.opened[0].says(attached));
    act(() => Socket.opened[0].says({ type: 'ended', v: 1, outcome: 'sim-time-limit' }));
    act(() => Socket.opened[0].onclose?.({ reason: '' }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect([result.current.ended, result.current.lost, Socket.opened.length]).toEqual(['sim-time-limit', null, 1]);
  });
});
