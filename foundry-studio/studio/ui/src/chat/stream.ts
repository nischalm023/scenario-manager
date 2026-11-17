// Spec 02 section 3.3, the stream client: one per browser tab and conversation. It loads the snapshot (step 1),
// opens the conversation's server-sent events after `last_seq` (step 2), applies each event as section 3.2 says
// (step 3), starts every connection in catch-up until that connection's `stream.live` (step 4), closes a stream silent
// for 45 s (step 5), lets the browser reconnect after a network error (step 6) and reconnects itself, backing off,
// after a refusal (step 7) or a `stream.reset` whose snapshot it cannot read (step 8), and keeps the connection words
// (step 9). The `live` flag it hands panel commands is this connection's state, never a comparison with an older
// snapshot's `last_seq`. Each timer belongs to the start that set it: a stop clears them, and a stale one does nothing.

import type { ConversationChanged } from '../protocol/conversation.ts';
import type { JournalEntry, JournalKind } from '../protocol/journal.ts';
import type { EntriesPage, Snapshot } from '../protocol/snapshot.ts';
import type { TextDelta } from '../protocol/stream.ts';
import { getJson, type Answer } from '../shell/api.ts';
import type { RunProgress } from '../protocol/run.ts';
import { bodyOf } from './bodies.ts';
import { applyDelta, applyEntry, applyProgress, mergeEarlier, modelFromSnapshot, type LogModel } from './journal.ts';

/** Section 3.3, step 9; `null` while the stream is live. */
export type ConnectionState = 'connecting' | 'reconnecting' | 'offline' | 'signed-out' | null;

export type Phase = 'catch-up' | 'live';

/** What the rest of the tab hears from the stream: the announcer (speech.ts) and the panel commands (commands.ts). */
export interface StreamListener {
  /** A snapshot was read: on load, after `stream.reset`, or after a refusal. */
  loaded?(model: LogModel, why: 'load' | 'reset' | 'refusal'): void;
  /** An entry new to this tab was applied. */
  applied?(entry: JournalEntry, model: LogModel, phase: Phase): void;
  /** This connection's `stream.live`: catch-up ends. */
  live?(model: LogModel): void;
  /** The connection words changed. */
  connection?(state: ConnectionState, previous: ConnectionState): void;
  /** A11: no stream for 10 s (`lost`), and the first event after that (`back`). */
  loss?(phase: 'lost' | 'back'): void;
  /** `conversation.changed` with `next` (step 13): the tab follows to the site's next conversation. */
  follow?(changed: ConversationChanged, entry: JournalEntry): void;
  /** A live-only item other than a text delta (`run.progress`, `template.evaluation`), as it arrives. */
  liveItem?(kind: string, body: unknown): void;
}

/** The part of `EventSource` this client uses, so tests can drive it. */
export interface EventSourceLike {
  readonly readyState: number;
  onopen: ((event: Event) => void) | null;
  onerror: ((event: Event) => void) | null;
  addEventListener(type: string, listener: (event: MessageEvent<string>) => void): void;
  close(): void;
}

export interface StreamDeps {
  openSource(url: string): EventSourceLike;
  read<T>(path: string): Promise<Answer<T>>;
}

const defaultDeps: StreamDeps = {
  openSource: (url) => new EventSource(url) as unknown as EventSourceLike,
  read: getJson,
};

const CLOSED = 2;
const STALL_MS = 45_000;
const RECONNECTING_AFTER_MS = 3_000;
const LOSS_SPOKEN_AFTER_MS = 10_000;
const OFFLINE_AFTER_MS = 30_000;
const BACKOFF_MOST_MS = 30_000;

/** Every journaled kind, each a named event the page must listen for (`EventSource` hands on only those). */
const JOURNAL_KINDS: Record<JournalKind, true> = {
  'message.user': true, 'turn.started': true, 'text.done': true, 'tool.started': true, 'tool.progress': true,
  'tool.finished': true, 'question.asked': true, 'question.answered': true, 'question.expired': true,
  'approval.requested': true, 'approval.decided': true, 'approval.expired': true, 'approval.noted': true,
  'approval.execution': true, 'draft.updated': true, 'draft.applied': true, 'draft.discarded': true, 'draft.stale': true,
  'panel.command': true, 'panel.event': true, 'run.status': true, 'submission.ended': true, 'run.finding': true,
  'turn.waiting': true, 'turn.resumed': true, 'turn.finished': true, 'system.note': true, 'conversation.changed': true,
};

export class ConversationStream {
  readonly conversationId: string;
  model: LogModel | null = null;
  connection: ConnectionState = 'connecting';
  /** Whether this connection is still replaying (step 4); panel commands are live only once it is not. */
  catchingUp = true;
  /** Why the conversation cannot be read, in the chat's words, when it cannot. */
  failure: string | null = null;
  version = 0;

  private readonly deps: StreamDeps;
  private readonly listener: StreamListener;
  private readonly readOnly: boolean;
  private source: EventSourceLike | null = null;
  private stall: ReturnType<typeof setTimeout> | null = null;
  private lossTimers: ReturnType<typeof setTimeout>[] = [];
  private retry: ReturnType<typeof setTimeout> | null = null;
  private backoffMs = 0;
  private lossSpoken = false;
  private stopped = false;
  /** Each start and stop begins a new generation; work an older one began finds itself stale and stops. */
  private generation = 0;
  private readonly subscribers = new Set<() => void>();

  constructor(conversationId: string, listener: StreamListener, options: { readOnly?: boolean; deps?: StreamDeps } = {}) {
    this.conversationId = conversationId;
    this.listener = listener;
    this.readOnly = options.readOnly ?? false;
    this.deps = options.deps ?? defaultDeps;
  }

  subscribe = (subscriber: () => void): (() => void) => {
    this.subscribers.add(subscriber);
    return () => this.subscribers.delete(subscriber);
  };

  getVersion = (): number => this.version;

  /** Anything drawn from the model changed. */
  changed(): void {
    this.version += 1;
    this.subscribers.forEach((subscriber) => subscriber());
  }

  async start(): Promise<void> {
    this.generation += 1;
    this.stopped = false;
    if (await this.load('load')) this.connect();
  }

  stop(): void {
    this.generation += 1;
    this.stopped = true;
    this.closeSource();
    if (this.retry !== null) clearTimeout(this.retry);
    this.retry = null;
    this.clearLossTimers();
  }

  /** Whether work begun in `generation` still belongs to this stream: no stop or new start came since. */
  private current(generation: number): boolean {
    return generation === this.generation && !this.stopped;
  }

  /** Step 1: the snapshot. False when it cannot be read, which the chat then says. */
  private async load(why: 'load' | 'reset' | 'refusal'): Promise<boolean> {
    const generation = this.generation;
    const answer = await this.deps.read<Snapshot>(`/api/conversations/${this.conversationId}`);
    if (this.stopped || generation !== this.generation) return false;
    if (!answer.ok) {
      if (answer.status === 401) this.setConnection('signed-out');
      this.failure = answer.status === 404 ? 'There is no such conversation.' : answer.reason;
      this.changed();
      return false;
    }
    this.failure = null;
    this.model = modelFromSnapshot(answer.value);
    this.listener.loaded?.(this.model, why);
    this.changed();
    return true;
  }

  /** "Show earlier entries": the 200 entries before the first held, merged without changing what is drawn. */
  async loadEarlier(): Promise<void> {
    const first = this.model?.entries[0]?.seq;
    if (this.model === null || first === undefined) return;
    const generation = this.generation;
    const after = Math.max(0, first - 201);
    const answer = await this.deps.read<EntriesPage>(`/api/conversations/${this.conversationId}/entries?after=${after}&limit=200`);
    if (!answer.ok || this.model === null || !this.current(generation)) return;
    mergeEarlier(this.model, answer.value);
    this.changed();
  }

  /** Step 2: the stream after the cursor; every `open`, the browser's own reconnects included, begins catch-up. */
  private connect(): void {
    if (this.readOnly || this.stopped || this.model === null) return;
    this.closeSource();
    const source = this.deps.openSource(`/api/conversations/${this.conversationId}/stream?after=${this.model.lastSeq}`);
    this.source = source;
    source.onopen = () => {
      this.catchingUp = true;
      this.armStall();
    };
    source.onerror = () => this.lost(source);
    const on = (kind: string, handle: (data: unknown) => void) => {
      source.addEventListener(kind, (event) => {
        if (source !== this.source) return;
        this.heard();
        let data: unknown;
        try {
          data = JSON.parse(event.data);
        } catch {
          return;
        }
        handle(data);
      });
    };
    for (const kind of Object.keys(JOURNAL_KINDS)) on(kind, (data) => this.applyJournaled(data as JournalEntry));
    on('text.delta', (data) => this.withModel((model) => applyDelta(model, data as TextDelta)));
    on('run.progress', (data) => {
      this.withModel((model) => applyProgress(model, data as RunProgress, Date.now()));
      this.listener.liveItem?.('run.progress', data);
    });
    on('template.evaluation', (data) => this.listener.liveItem?.('template.evaluation', data));
    on('ping', () => undefined);
    on('stream.live', () => {
      this.catchingUp = false;
      this.backoffMs = 0;
      if (this.model !== null) this.listener.live?.(this.model);
      this.changed();
    });
    on('stream.reset', () => void this.reset());
  }

  private withModel(change: (model: LogModel) => void): void {
    if (this.model === null) return;
    change(this.model);
    this.changed();
  }

  private applyJournaled(entry: JournalEntry): void {
    if (this.model === null || !applyEntry(this.model, entry)) return;
    this.listener.applied?.(entry, this.model, this.catchingUp ? 'catch-up' : 'live');
    this.changed();
    const changed = bodyOf(entry, 'conversation.changed');
    if (changed !== null) {
      this.model.conversation = { ...this.model.conversation, state: changed.state, site: changed.site, title: changed.title };
      if (changed.next !== undefined) this.listener.follow?.(changed, entry);
      this.changed();
    }
  }

  /** Any event: the stream is alive, the connection words go, the stall timer starts again. */
  private heard(): void {
    this.clearLossTimers();
    if (this.lossSpoken) {
      this.lossSpoken = false;
      this.listener.loss?.('back');
    }
    this.setConnection(null);
    this.armStall();
  }

  /** Step 5: 45 s without any event closes the stream and reconnects as after a refusal. */
  private armStall(): void {
    if (this.stall !== null) clearTimeout(this.stall);
    const generation = this.generation;
    this.stall = setTimeout(() => {
      if (!this.current(generation)) return;
      this.closeSource();
      this.catchingUp = true;
      this.startLossTimers();
      this.scheduleReconnect(1000);
    }, STALL_MS);
  }

  /** Steps 6, 7 and 9: a browser that retries by itself gets the words; a refused stream is reconnected here. */
  private lost(source: EventSourceLike): void {
    if (source !== this.source || this.stopped) return;
    this.catchingUp = true;
    this.startLossTimers();
    if (source.readyState === CLOSED) {
      this.closeSource();
      this.retryLater();
    }
  }

  /**
   * Steps 7 to 9: no stream is open and the browser will not open one, so the loss words and A11 run while the client
   * reads the snapshot again after 1 s, 2 s, 4 s, up to 30 s, and reconnects from it; an expired sign-in ends that.
   */
  private retryLater(): void {
    if (this.stopped || this.connection === 'signed-out') return;
    this.catchingUp = true;
    this.startLossTimers();
    this.backoffMs = this.backoffMs === 0 ? 1000 : Math.min(this.backoffMs * 2, BACKOFF_MOST_MS);
    this.scheduleReconnect(this.backoffMs);
  }

  private startLossTimers(): void {
    if (this.lossTimers.length > 0) return;
    const generation = this.generation;
    const after = (ms: number, then: () => void) => setTimeout(() => {
      if (this.current(generation)) then();
    }, ms);
    this.lossTimers = [
      after(RECONNECTING_AFTER_MS, () => this.setConnection('reconnecting')),
      after(LOSS_SPOKEN_AFTER_MS, () => {
        this.lossSpoken = true;
        this.listener.loss?.('lost');
      }),
      after(OFFLINE_AFTER_MS, () => this.setConnection('offline')),
    ];
  }

  private clearLossTimers(): void {
    this.lossTimers.forEach(clearTimeout);
    this.lossTimers = [];
  }

  private scheduleReconnect(afterMs: number): void {
    if (this.retry !== null || this.stopped) return;
    const generation = this.generation;
    this.retry = setTimeout(() => {
      this.retry = null;
      if (!this.current(generation)) return;
      void this.load('refusal').then((read) => {
        if (read) this.connect();
        else if (this.current(generation)) this.retryLater();
      });
    }, afterMs);
  }

  /** Step 8: close, read the snapshot, redraw, reopen; a snapshot it cannot read is retried as after a refusal. */
  private async reset(): Promise<void> {
    const generation = this.generation;
    this.closeSource();
    if (await this.load('reset')) this.connect();
    else if (this.current(generation)) this.retryLater();
  }

  private closeSource(): void {
    if (this.stall !== null) clearTimeout(this.stall);
    this.stall = null;
    this.source?.close();
    this.source = null;
  }

  private setConnection(state: ConnectionState): void {
    if (state === this.connection || (this.connection === 'signed-out' && state !== null)) return;
    const previous = this.connection;
    this.connection = state;
    this.listener.connection?.(state, previous);
    this.changed();
  }
}

/** One listener that hands every event to each of `listeners` in turn. */
export function combineListeners(...listeners: StreamListener[]): StreamListener {
  return {
    loaded: (model, why) => listeners.forEach((one) => one.loaded?.(model, why)),
    applied: (entry, model, phase) => listeners.forEach((one) => one.applied?.(entry, model, phase)),
    live: (model) => listeners.forEach((one) => one.live?.(model)),
    connection: (state, previous) => listeners.forEach((one) => one.connection?.(state, previous)),
    loss: (phase) => listeners.forEach((one) => one.loss?.(phase)),
    follow: (changed, entry) => listeners.forEach((one) => one.follow?.(changed, entry)),
    liveItem: (kind, body) => listeners.forEach((one) => one.liveItem?.(kind, body)),
  };
}
