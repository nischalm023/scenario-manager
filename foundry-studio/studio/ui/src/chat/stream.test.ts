// Spec 02 section 3.3, steps 4, 7 and 8, and section 3.4's A11: a `stream.reset` whose snapshot cannot be read goes
// through the reconnect backoff with the connection words, until it reads or the sign-in expires; what a catch-up
// collected survives a reset and is spoken once at the next `stream.live`; and a stopped stream says nothing.

import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Snapshot } from '../protocol/snapshot.ts';
import type { Answer } from '../shell/api.ts';
import { entriesOf, snapshotFrom } from '../testing/journal.ts';
import { FakeSource, TAB } from '../testing/render.tsx';
import { ChatSession } from './session.ts';
import { speechListener } from './speech.ts';
import type { StreamDeps, StreamListener } from './stream.ts';

afterEach(() => {
  vi.useRealTimers();
});

const ENDED = entriesOf('ended-turns');
const upTo = (seq: number) => ENDED.filter((entry) => entry.seq <= seq);

/**
 * A tab on `snapshot()` whose snapshot reads are refused with `refusal()` while it names a status, each read first
 * waiting for `gate()` when that holds one.
 */
function tabOn(snapshot: () => Snapshot, refusal: () => number | null, listener: StreamListener = {}, gate: () => Promise<void> | null = () => null) {
  const sources: FakeSource[] = [];
  const reads: string[] = [];
  const deps: StreamDeps = {
    openSource: (url) => {
      const source = new FakeSource(url);
      sources.push(source);
      return source;
    },
    read: async <T,>(path: string): Promise<Answer<T>> => {
      reads.push(path);
      await gate();
      const status = refusal();
      if (status !== null) return { ok: false, status, problem: null, reason: `the service answered ${status}` };
      return { ok: true, status: 200, value: snapshot() as T };
    },
  };
  const session = new ChatSession(snapshot().conversation.id, TAB, listener, { deps });
  return { session, stream: session.stream, sources, reads };
}

/** A tab on the fixture up to turn 4's failure, its first connection past `stream.live`. */
async function liveTab(refusal: () => number | null, listener: StreamListener = {}, gate?: () => Promise<void> | null) {
  const tab = tabOn(() => snapshotFrom(upTo(33), 1), refusal, listener, gate);
  await tab.stream.start();
  tab.sources[0].open();
  tab.sources[0].emit('stream.live', { last_seq: 33 });
  return tab;
}

describe('section 3.3, step 8: a stream.reset whose snapshot cannot be read', () => {
  /**
   * Spec 02 section 3.3, steps 7 to 9: a reset's unreadable snapshot is retried with backoff and the connection words.
   */
  it('reads it again after 1 s, 2 s and 4 s with the connection words, and reconnects once it reads', async () => {
    vi.useFakeTimers();
    let refused: number | null = null;
    const tab = await liveTab(() => refused);
    refused = 503;
    tab.sources[0].emit('stream.reset', { reason: 'slow-client' });
    await vi.advanceTimersByTimeAsync(0);
    expect(tab.sources).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(3000);
    expect(tab.stream.connection).toBe('reconnecting');
    expect(tab.reads).toHaveLength(4);
    refused = null;
    await vi.advanceTimersByTimeAsync(4000);
    expect(tab.reads).toHaveLength(5);
    expect(tab.sources).toHaveLength(2);
    expect(tab.sources[1].url).toContain('after=33');
    tab.sources[1].open();
    tab.sources[1].emit('stream.live', { last_seq: 33 });
    expect(tab.stream.connection).toBeNull();
    tab.session.stop();
  });

  /** Spec 02 section 3.3, step 7: a 401 ends the retries and the chat says the sign-in expired. */
  it('stops retrying once the sign-in has expired', async () => {
    vi.useFakeTimers();
    let refused: number | null = null;
    const tab = await liveTab(() => refused);
    refused = 401;
    tab.sources[0].emit('stream.reset', { reason: 'unknown-cursor' });
    await vi.advanceTimersByTimeAsync(60_000);
    expect(tab.stream.connection).toBe('signed-out');
    expect(tab.reads).toHaveLength(2);
    expect(tab.sources).toHaveLength(1);
    tab.session.stop();
  });
});

describe("section 3.3, step 4: what a connection's catch-up collected survives a stream.reset", () => {
  /**
   * Spec 02 section 3.3, step 4: a connection that ends before its stream.live keeps what it collected for the next.
   */
  it('speaks the failure collected before the reset once, at the next connection\'s stream.live', async () => {
    const said: string[] = [];
    const listener = speechListener({ say: (sentence) => said.push(sentence), reading: () => 'd70', acted: () => false });
    let held = 29;
    const tab = tabOn(() => snapshotFrom(upTo(held), 1), () => null, listener);
    await tab.stream.start();
    tab.sources[0].open();
    for (const entry of ENDED.filter((one) => one.seq > 29 && one.seq <= 33)) tab.sources[0].entry(entry);
    held = 33;
    tab.sources[0].emit('stream.reset', { reason: 'slow-client' });
    await vi.waitFor(() => expect(tab.sources).toHaveLength(2));
    tab.sources[1].open();
    tab.sources[1].emit('stream.live', { last_seq: 33 });
    expect(said).toEqual([
      'The agent is working.',
      'A tool call failed.',
      'The model stayed unavailable after 3 retries, so the turn failed. Send it again when it is back.',
    ]);
    tab.sources[1].emit('stream.live', { last_seq: 33 });
    expect(said).toHaveLength(3);
    tab.session.stop();
  });
});

describe('section 3.4, A11: a stopped stream says nothing', () => {
  /** Spec 02 section 3.4, A11: a stopped stream announces no loss. */
  it('announces no loss and sets no connection words once stopped, though its connection had dropped', async () => {
    vi.useFakeTimers();
    const heard: string[] = [];
    const tab = await liveTab(() => null, { loss: (phase) => heard.push(phase), connection: (state) => heard.push(`words ${state}`) });
    heard.length = 0;
    tab.sources[0].fail(0);
    tab.session.stop();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(heard).toEqual([]);
    expect(tab.sources).toHaveLength(1);
  });

  /** Spec 02 section 3.3: work a stopped start began opens no connection after a new start. */
  it('lets a retry begun before a stop and a new start open nothing, so the new start holds the one connection', async () => {
    vi.useFakeTimers();
    let gate: Promise<void> | null = null;
    let release: () => void = () => undefined;
    const tab = await liveTab(() => null, {}, () => gate);
    gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    tab.sources[0].fail(2);
    await vi.advanceTimersByTimeAsync(1000);
    expect(tab.reads).toHaveLength(2);
    tab.session.stop();
    const held = gate;
    gate = null;
    await tab.stream.start();
    expect(tab.sources).toHaveLength(2);
    release();
    await held;
    await vi.advanceTimersByTimeAsync(60_000);
    expect(tab.sources).toHaveLength(2);
    tab.session.stop();
  });
});
