// Spec 03 section 3.3, "Which browser tab acts": a random v4 minted at first load and kept in `sessionStorage`, so a
// reload keeps it. A duplicated browser tab copies `sessionStorage`, so each page asks on a `BroadcastChannel` whether
// another live page holds its id, and mints a new one when one answers. The id is settled once that question has had
// its wait, and the shell opens the page's conversation only then; an answer that comes later still mints a new id
// and tells it, and the shell makes the conversation's session again, so its posts and its command handling carry
// the new id. The id rides on every POST that starts a turn and comes back on `turn.started`.

import { useEffect, useState } from 'react';
import { readStored, writeStored } from './storage.ts';

const KEY = 'agent-studio.browser-tab';
const CHANNEL = 'agent-studio.browser-tabs';
/** How long a page waits, in wall-clock ms, for another page to say it holds the same id. */
const PROBE_WAIT_MS = 250;

export type TabMessage = { kind: 'probe'; id: string; from: string } | { kind: 'held'; id: string; to: string };

/** The part of `BroadcastChannel` the probe uses, so a test can join two pages by one channel. */
export interface ChannelLike {
  onmessage: ((event: MessageEvent<TabMessage>) => void) | null;
  postMessage(message: TabMessage): void;
}

export interface BrowserTabDeps {
  read(): string | null;
  write(id: string): void;
  /** The channel to the other pages, or null where the browser has none: a duplicate then keeps the copied id. */
  channel(): ChannelLike | null;
  waitMs: number;
}

/** One page's browser-tab id. */
export class BrowserTab {
  private readonly deps: BrowserTabDeps;
  /** This page's own name on the channel, so it never answers its own probe. */
  private readonly page = crypto.randomUUID();
  private current: string | null = null;
  private done = false;
  private settling: Promise<string> | null = null;
  private readonly listeners = new Set<(id: string) => void>();

  constructor(deps: BrowserTabDeps) {
    this.deps = deps;
  }

  /** The id once settled, else null. */
  id(): string | null {
    return this.done ? this.current : null;
  }

  /** Read or mint the id, probe, and answer once no other page claimed it within the wait, or with the new one. */
  settle(): Promise<string> {
    this.settling ??= new Promise<string>((resolve) => {
      const id = this.deps.read() ?? this.mint();
      this.current = id;
      const finish = () => {
        if (this.done) return;
        this.done = true;
        resolve(this.current ?? id);
      };
      const channel = this.deps.channel();
      if (channel === null) {
        finish();
        return;
      }
      channel.onmessage = (event) => this.heard(channel, event.data, finish);
      channel.postMessage({ kind: 'probe', id, from: this.page });
      setTimeout(finish, this.deps.waitMs);
    });
    return this.settling;
  }

  /** Hear of each change a late answer makes, after the id settled. */
  onChange(listener: (id: string) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private heard(channel: ChannelLike, message: TabMessage, finish: () => void): void {
    if (message.kind === 'probe' && message.id === this.current && message.from !== this.page) {
      channel.postMessage({ kind: 'held', id: message.id, to: message.from });
    } else if (message.kind === 'held' && message.to === this.page && message.id === this.current) {
      const fresh = this.mint();
      this.current = fresh;
      if (this.done) this.listeners.forEach((listener) => listener(fresh));
      else finish();
    }
  }

  private mint(): string {
    const fresh = crypto.randomUUID();
    this.deps.write(fresh);
    return fresh;
  }
}

function openChannel(): ChannelLike | null {
  try {
    return new BroadcastChannel(CHANNEL) as unknown as ChannelLike;
  } catch {
    return null;
  }
}

const shared = new BrowserTab({
  read: () => readStored('session', KEY), write: (id) => writeStored('session', KEY, id), channel: openChannel,
  waitMs: PROBE_WAIT_MS,
});

/** This page's browser tab. */
export const browserTab = (): BrowserTab => shared;

/** This page's browser-tab id: null until it settles, then each id a late answer gives it. */
export function useBrowserTab(): string | null {
  const [id, setId] = useState<string | null>(() => shared.id());
  useEffect(() => {
    let current = true;
    void shared.settle().then((settled) => {
      if (current) setId(settled);
    });
    const stop = shared.onChange(setId);
    return () => {
      current = false;
      stop();
    };
  }, []);
  return id;
}
