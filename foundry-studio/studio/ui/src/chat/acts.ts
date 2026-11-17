// Spec 02 section 3.3, step 10, and spec 01 section 3.6: every act the chat makes is a POST to its route, with the
// body and answer studio/protocol types. A message carries a fresh `Idempotency-Key` per action, reused only when
// that action is retried, and a `client_message_id` minted with its optimistic entry; a cancel carries a fresh key
// per press too (spec 08 section 3.7); answers, decisions, stop, apply and discard need no key. The routes task 3 has
// not mounted answer 404, which the composer and the cards say in section 3.8's words: nothing here is faked.

import type { ApprovalAnswer, Decision, PostAttribution, PostDecision } from '../protocol/approval.ts';
import type { MessageAccepted, PostMessage } from '../protocol/message.ts';
import type { ItemAnswer, PostAnswers } from '../protocol/question.ts';
import { postJson, type Answer } from '../shell/api.ts';

export const sendMessage = (conversation: string, post: PostMessage, key: string): Promise<Answer<MessageAccepted>> =>
  postJson<MessageAccepted>(`/api/conversations/${conversation}/messages`, post, key);

export const answerQuestion = (question: string, answers: ItemAnswer[]) =>
  postJson<unknown>(`/api/questions/${question}/answers`, { answers } satisfies PostAnswers);

export const decide = (approval: string, decision: Decision, reason?: string) =>
  postJson<ApprovalAnswer>(`/api/approvals/${approval}/decision`, (reason ? { decision, reason } : { decision }) satisfies PostDecision);

export const attribute = (approval: string, wasThis: boolean) =>
  postJson<ApprovalAnswer>(`/api/approvals/${approval}/attribution`, { was_this: wasThis } satisfies PostAttribution);

export const stopTurn = (turn: string) => postJson<unknown>(`/api/turns/${turn}/stop`, {});

/** `202 { approval }`: the press is the approval (spec 01 section 3.10, rule 6). */
export const applyDraft = (draft: string) => postJson<ApprovalAnswer>(`/api/drafts/${draft}/apply`, {});

export const discardDraft = (draft: string) => postJson<unknown>(`/api/drafts/${draft}/discard`, {});

export const cancelRun = (run: string) => postJson<unknown>(`/api/runs/${run}/cancel`, {}, freshKey());

export const cancelSubmission = (submission: string) =>
  postJson<unknown>(`/api/submissions/${submission}/cancel`, {}, freshKey());

/** A fresh random v4 per user action (spec 01 section 3.13); never derived from anything. */
export const freshKey = (): string => crypto.randomUUID();
