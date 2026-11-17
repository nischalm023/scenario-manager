// Test support for the chat's unit tests of spec 02 section 8: a shell around the component under test (section 6's
// services as a test needs them: who is signed in, this browser tab's id, the announcer's sentences collected), a
// stand-in `EventSource` the test drives event by event (section 3.3's stream client), and a session on a snapshot
// rendered in the real ChatColumn.

import { useSyncExternalStore } from 'react';
import { render } from '@testing-library/react';
import { ChatColumn } from '../chat/ChatColumn.tsx';
import { ChatSession } from '../chat/session.ts';
import type { EventSourceLike, StreamDeps, StreamListener } from '../chat/stream.ts';
import type { ChatHandle } from '../chat/useChat.ts';
import type { JournalEntry } from '../protocol/journal.ts';
import type { PageContext } from '../protocol/page.ts';
import type { Me } from '../protocol/person.ts';
import type { Snapshot } from '../protocol/snapshot.ts';
import type { Answer } from '../shell/api.ts';
import { ShellContext, type Shell } from '../shell/context.ts';
import { parseRoute } from '../shell/routes.ts';

export const ME: Me = { subject: 'dev', email: 'dev@example.com', name: 'Developer', developer: true, runner_image: null };
export const TAB = '87202cd6-c458-4b48-95b6-cc86614adfd3';

export function shellOf(spoken: string[] = [], address = '/w/customer/demo/site/lshape-6/map@main'): Shell {
  return {
    route: parseRoute(address), address, me: ME, browserTab: TAB,
    navigate: () => undefined,
    announce: (sentence) => spoken.push(sentence),
    focusComposer: () => document.getElementById('composer-input')?.focus(),
    focusPanelTitle: () => document.getElementById('panel-title')?.focus(),
  };
}

export class FakeSource implements EventSourceLike {
  readyState = 0;
  onopen: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  readonly url: string;
  private readonly listeners = new Map<string, ((event: MessageEvent<string>) => void)[]>();

  constructor(url: string) {
    this.url = url;
  }

  addEventListener(type: string, listener: (event: MessageEvent<string>) => void): void {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }

  close(): void {
    this.readyState = 2;
  }

  open(): void {
    this.readyState = 1;
    this.onopen?.(new Event('open'));
  }

  emit(kind: string, data: unknown): void {
    const event = new MessageEvent<string>(kind, { data: JSON.stringify(data) });
    (this.listeners.get(kind) ?? []).forEach((listener) => listener(event));
  }

  entry(entry: JournalEntry): void {
    this.emit(entry.kind, entry);
  }

  /** A network error the browser retries by itself (`CONNECTING`), or a refusal it does not (`CLOSED`). */
  fail(readyState: 0 | 2): void {
    this.readyState = readyState;
    this.onerror?.(new Event('error'));
  }
}

export function fakeDeps(snapshot: () => Snapshot): { deps: StreamDeps; sources: FakeSource[] } {
  const sources: FakeSource[] = [];
  const deps: StreamDeps = {
    openSource: (url) => {
      const source = new FakeSource(url);
      sources.push(source);
      return source;
    },
    read: async <T,>(path: string): Promise<Answer<T>> => {
      if (path.includes('/entries')) return { ok: false, status: 404, problem: null, reason: 'no earlier entries in this test' };
      return { ok: true, status: 200, value: snapshot() as T };
    },
  };
  return { deps, sources };
}

/** A started session on `snapshot`, its stream's sources, and a way to wait for the snapshot to be read. */
export async function sessionOn(snapshot: Snapshot, listener: StreamListener = {}): Promise<{ session: ChatSession; sources: FakeSource[] }> {
  const { deps, sources } = fakeDeps(() => snapshot);
  const session = new ChatSession(snapshot.conversation.id, TAB, listener, { deps });
  await session.stream.start();
  return { session, sources };
}

function Harness({ session, context, siteName }: { session: ChatSession; context: PageContext | null; siteName: string | null }) {
  const version = useSyncExternalStore(session.stream.subscribe, session.stream.getVersion);
  const handle: ChatHandle = {
    session, viewing: null, failure: null, archived: [], version, view: () => undefined,
    dismiss: async () => ({ ok: false, words: 'not in this test' }),
  };
  return <ChatColumn chat={handle} context={context} siteName={siteName} />;
}

export const CONTEXT: PageContext = {
  route: '/w/customer/demo/site/lshape-6/map@main', panel: 'map', title: 'Map: customer/demo/site/lshape-6, following branch main',
  seen: [], selection: null, highlights: null,
};

/** The site the fixtures' conversations belong to, by the name the shell gives the chat's title. */
export const SITE_NAME = 'Small L-shape, 6 aisles';

export function renderChat(session: ChatSession, shell: Shell = shellOf(), context: PageContext | null = CONTEXT, siteName: string | null = SITE_NAME) {
  return render(
    <ShellContext.Provider value={shell}>
      <Harness session={session} context={context} siteName={siteName} />
    </ShellContext.Provider>,
  );
}
