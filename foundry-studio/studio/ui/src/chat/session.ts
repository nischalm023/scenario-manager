// Spec 02 section 3.3, steps 10 to 12, and section 3.1's Composer: one browser tab's hold on one conversation, the
// stream client with what only this tab knows (its optimistic messages, its Apply presses), and the acts it posts. A
// message, the composer's or a panel act's (section 6), draws its user entry `sending` with a fresh
// `client_message_id`, and whichever arrives first, the 202 or the `message.user` carrying that id, settles it; once
// the journal holds the message, its POST's answer changes nothing. A refused composer message leaves the log and
// returns its text to the box. Unsent text is held here and kept in `sessionStorage` as well, where a send in flight
// with its key is kept too, so both survive a reload when storage allows and nothing is lost when it refuses (3.8).

import type { Decision } from '../protocol/approval.ts';
import type { JournalEntry } from '../protocol/journal.ts';
import type { PageContext } from '../protocol/page.ts';
import type { Problem } from '../protocol/problem.ts';
import type { ProjectedQuestion } from '../protocol/projection.ts';
import type { ItemAnswer } from '../protocol/question.ts';
import { readStored, writeStored } from '../shell/storage.ts';
import * as acts from './acts.ts';
import type { ActOutcome } from './chatContext.ts';
import { holdsTheStart } from './journal.ts';
import { emptyLocal, type RunCardModel, type SystemItem, type TabLocal } from './log.ts';
import { ConversationStream, type StreamDeps, type StreamListener } from './stream.ts';
import { appliedWords, cardFailureWords, panelSentWords, sendFailureWords } from './words.ts';
import { bodyOf, statusOf } from './bodies.ts';

/** Where a message comes from: the composer, or a panel act that answers the agent (spec 02 section 6). */
export type MessageOrigin = { source: 'composer' } | { source: 'panel'; act: string };

const COMPOSER: MessageOrigin = { source: 'composer' };

interface InFlight {
  key: string;
  clientMessageId: string;
  text: string;
  context: PageContext | null;
  /** How it is sent, in words: a panel act's, else ''. */
  sentAs: string;
  origin: MessageOrigin;
}

export interface ComposerStatus {
  words: string;
  tone: 'bad' | null;
  /** The message a "Try again" resends with its key. */
  retry: InFlight | null;
}

const outcomeOf = (answer: { ok: boolean; problem?: Problem | null; reason?: string }): ActOutcome =>
  answer.ok ? { ok: true } : { ok: false, words: cardFailureWords(answer.problem ?? null, answer.reason ?? 'the service refused it') };

/** The `client_message_id` a `message.user` entry carries, if any. */
const messageIdOf = (entry: JournalEntry): string | undefined => bodyOf(entry, 'message.user')?.client_message_id;

export class ChatSession {
  readonly stream: ConversationStream;
  local: TabLocal = emptyLocal();
  status: ComposerStatus | null = null;
  /** Set while this tab's Dismiss is in flight: the `conversation.changed` it causes is this tab's own act. */
  dismissing = false;
  private readonly browserTab: string;
  private readonly unsentKey: string;
  private readonly inFlightKey: string;
  /** The `client_message_id`s the journal holds: such a message was accepted, whatever its POST answers. */
  private readonly journaled = new Set<string>();
  /** The composer's unsent text, held here first; `sessionStorage` keeps a copy for a reload when it allows. */
  private draft: string | null = null;

  constructor(conversationId: string, browserTab: string, listener: StreamListener, options: { readOnly?: boolean; deps?: StreamDeps } = {}) {
    this.browserTab = browserTab;
    this.unsentKey = `agent-studio.unsent.${conversationId}`;
    this.inFlightKey = `agent-studio.in-flight.${conversationId}`;
    this.stream = new ConversationStream(conversationId, {
      ...listener,
      loaded: (model, why) => {
        model.entries.forEach((entry) => this.settleByEntry(entry));
        listener.loaded?.(model, why);
      },
      applied: (entry, model, phase) => {
        this.settleByEntry(entry);
        if (entry.kind === 'conversation.changed' && this.dismissing) this.local.actedChanges.add(entry.seq);
        listener.applied?.(entry, model, phase);
      },
    }, options);
  }

  get conversationId(): string {
    return this.stream.conversationId;
  }

  start(): void {
    void this.stream.start();
    const inFlight = this.storedInFlight();
    if (inFlight !== null) void this.post(inFlight);
  }

  stop(): void {
    this.stream.stop();
  }

  /** The sentence carried from the conversation this tab followed away from. */
  carry(items: SystemItem[]): void {
    this.local.carried = items;
    this.stream.changed();
  }

  /** While this tab's Dismiss is in flight, the `conversation.changed` it causes is its own act. */
  markDismissing(on: boolean): void {
    this.dismissing = on;
  }

  /** A refused act the composer shows in its status line. */
  fail(words: string): void {
    this.status = { words, tone: 'bad', retry: null };
    this.stream.changed();
  }

  /** "Show earlier entries", then F8's "Start of the conversation" once the first entry is held: true from then on. */
  async loadEarlier(): Promise<boolean> {
    await this.stream.loadEarlier();
    const model = this.stream.model;
    const start = model !== null && holdsTheStart(model);
    this.local.pagedBack = this.local.pagedBack || start;
    this.stream.changed();
    return start;
  }

  unsent(): string {
    return this.draft ?? readStored('session', this.unsentKey) ?? '';
  }

  keepUnsent(text: string): void {
    this.draft = text;
    writeStored('session', this.unsentKey, text === '' ? null : text);
  }

  private storedInFlight(): InFlight | null {
    try {
      const stored = readStored('session', this.inFlightKey);
      const read = stored === null ? null : (JSON.parse(stored) as Partial<InFlight>);
      return read === null ? null : ({ ...read, origin: read.origin ?? COMPOSER } as InFlight);
    } catch {
      return null;
    }
  }

  /** Send a message: the optimistic entry, then the POST with a fresh key; a panel act's names its act. */
  async send(text: string, context: PageContext | null, origin: MessageOrigin = COMPOSER): Promise<ActOutcome> {
    const sentAs = origin.source === 'panel' ? panelSentWords(context, origin.act) : '';
    const message: InFlight = { key: acts.freshKey(), clientMessageId: crypto.randomUUID(), text, context, sentAs, origin };
    if (origin.source === 'composer') this.keepUnsent('');
    return this.post(message);
  }

  /** "Try again": the same message, its key and its `client_message_id` kept. */
  async retry(): Promise<ActOutcome> {
    const message = this.status?.retry;
    if (!message) return { ok: false, words: 'There is no message to send again.' };
    this.keepUnsent('');
    return this.post(message);
  }

  private async post(message: InFlight): Promise<ActOutcome> {
    this.status = null;
    this.local.optimistic = [
      ...this.local.optimistic.filter((held) => held.clientMessageId !== message.clientMessageId),
      { clientMessageId: message.clientMessageId, text: message.text, context: message.context, sentAs: message.sentAs, state: 'sending' },
    ];
    writeStored('session', this.inFlightKey, JSON.stringify(message));
    this.stream.changed();
    const act = message.origin.source === 'panel' ? { act: message.origin.act } : {};
    const answer = await acts.sendMessage(this.conversationId, {
      text: message.text, context: message.context, source: message.origin.source, browser_tab: this.browserTab,
      client_message_id: message.clientMessageId, ...act,
    }, message.key);
    writeStored('session', this.inFlightKey, null);
    // The journal holding the message settles it (step 10): a POST that then fails, its answer lost on the way back,
    // reports nothing, since the message was accepted.
    if (answer.ok || this.journaled.has(message.clientMessageId)) {
      this.local.optimistic = this.local.optimistic.map((held) => (held.clientMessageId === message.clientMessageId ? { ...held, state: 'sent' } : held));
      this.stream.changed();
      return { ok: true };
    }
    this.local.optimistic = this.local.optimistic.filter((held) => held.clientMessageId !== message.clientMessageId);
    this.stream.changed();
    // A panel act's refusal is its panel's to show beside the control pressed (F4); the composer's is the box's.
    if (message.origin.source === 'panel') return outcomeOf(answer);
    const failure = sendFailureWords(answer.problem, answer.reason);
    this.status = { words: failure.words, tone: 'bad', retry: failure.retry ? message : null };
    if (this.unsent() === '') this.keepUnsent(message.text);
    this.stream.changed();
    return { ok: false, words: failure.words };
  }

  /**
   * Section 3.3, step 10: the `message.user` that carries an optimistic entry's id settles it, and a failure this tab
   * already reported for that message is withdrawn, since the message was accepted after all.
   */
  private settleByEntry(entry: JournalEntry): void {
    const id = messageIdOf(entry);
    if (id === undefined) return;
    this.journaled.add(id);
    this.local.optimistic = this.local.optimistic.filter((held) => held.clientMessageId !== id);
    if (this.status?.retry?.clientMessageId === id) this.status = null;
  }

  async answer(question: ProjectedQuestion, answers: ItemAnswer[]): Promise<ActOutcome> {
    return outcomeOf(await acts.answerQuestion(question.id, answers));
  }

  async decide(approval: string, decision: Decision, reason?: string): Promise<ActOutcome> {
    return outcomeOf(await acts.decide(approval, decision, reason));
  }

  async attribute(approval: string, wasThis: boolean): Promise<ActOutcome> {
    return outcomeOf(await acts.attribute(approval, wasThis));
  }

  /**
   * Apply: the 202's approval is recorded as this tab's press (log.ts's `applyOf` draws "Applying" from it until the
   * journal holds the approval). When the draft already landed before the 202 came back, the stream heard it as no act
   * of this tab's, so its A12 is said here instead.
   */
  async apply(draft: string): Promise<ActOutcome> {
    const answer = await acts.applyDraft(draft);
    if (!answer.ok) return outcomeOf(answer);
    const approval = answer.value.approval.id;
    this.local.presses.set(approval, draft);
    this.stream.changed();
    const landed = this.stream.model?.entries.find((entry) => entry.kind === 'draft.applied' && entry.caused_by === approval);
    return landed === undefined ? { ok: true } : { ok: true, said: appliedWords(bodyOf(landed, 'draft.applied')) };
  }

  async discard(draft: string): Promise<ActOutcome> {
    return outcomeOf(await acts.discardDraft(draft));
  }

  async cancel(card: RunCardModel): Promise<ActOutcome> {
    if (card.of > 1) return outcomeOf(await acts.cancelSubmission(card.submission));
    const first = card.runs[0];
    const run = first !== undefined ? statusOf(first)?.run : undefined;
    return run === undefined ? { ok: false, words: 'This run is not known yet.' } : outcomeOf(await acts.cancelRun(run));
  }

  async stopTurn(turn: string): Promise<ActOutcome> {
    return outcomeOf(await acts.stopTurn(turn));
  }
}
