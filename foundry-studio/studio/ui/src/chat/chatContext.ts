// Spec 02 sections 3.1 and 3.5: what the chat's components act through. Each act posts to its route (acts.ts) and
// answers in section 3.8's words; on success the component moves focus to the composer (F3), on failure it keeps
// focus on the control pressed and shows the words in `p.chat-card__error` (F4).

import { createContext, useContext } from 'react';
import type { Decision } from '../protocol/approval.ts';
import type { Me } from '../protocol/person.ts';
import type { ProjectedQuestion } from '../protocol/projection.ts';
import type { ItemAnswer } from '../protocol/question.ts';
import type { LogModel } from './journal.ts';
import type { RunCardModel } from './log.ts';

/** An act's outcome: `said` is an A12 the act itself speaks, when the stream heard its outcome before its answer. */
export type ActOutcome = { ok: true; said?: string } | { ok: false; words: string };

export interface ChatApi {
  me: Me | null;
  model: LogModel;
  /** An archived conversation, or another person's new-site thread: cards show their state without controls. */
  readOnly: boolean;
  answer(question: ProjectedQuestion, answers: ItemAnswer[]): Promise<ActOutcome>;
  decide(approval: string, decision: Decision, reason?: string): Promise<ActOutcome>;
  attribute(approval: string, wasThis: boolean): Promise<ActOutcome>;
  apply(draft: string): Promise<ActOutcome>;
  discard(draft: string): Promise<ActOutcome>;
  cancel(card: RunCardModel): Promise<ActOutcome>;
  /** A suggestion pressed, "Send it again", or "Rebuild the draft on the new head": the text goes at once. */
  send(text: string): Promise<void>;
  loadEarlier(): Promise<void>;
}

export const ChatContext = createContext<ChatApi | null>(null);

export function useChatApi(): ChatApi {
  const api = useContext(ChatContext);
  if (api === null) throw new Error('a chat component rendered outside the chat column');
  return api;
}
