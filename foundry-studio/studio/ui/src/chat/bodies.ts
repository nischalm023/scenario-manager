// Spec 01 section 3.5's table, each journal kind's body as the browser reads it: studio/protocol's generated
// types, narrowed by the entry's kind and never by a type a caller names. An entry, or an object of the projection,
// holds its body as JSON; `bodyOf` hands an entry's on only when the entry is of the kind asked for, and each
// accessor reads a projected object's field as the kind the projection keeps there (spec 01 section 3.6).

import type { ApprovalDecided, ApprovalExecuted, ApprovalExpired, ApprovalNoted, ApprovalRequested } from '../protocol/approval.ts';
import type { ConversationChanged } from '../protocol/conversation.ts';
import type { DraftApplied, DraftDiscarded, DraftStale, DraftUpdated } from '../protocol/draft.ts';
import type { JournalEntry, JournalKind } from '../protocol/journal.ts';
import type { UserMessage } from '../protocol/message.ts';
import type { PanelCommandEntry } from '../protocol/panel.ts';
import type { PanelEvent } from '../protocol/panel_event.ts';
import type { ProjectedApproval, ProjectedDraft, ProjectedQuestion, ProjectedRun } from '../protocol/projection.ts';
import type { QuestionAnswered, QuestionAsked, QuestionExpired } from '../protocol/question.ts';
import type { RunFinding, RunStatus, SubmissionEnded } from '../protocol/run.ts';
import type { JsonValue } from '../protocol/serde_json/JsonValue.ts';
import type { ToolFinished, ToolProgress, ToolStarted } from '../protocol/tool.ts';
import type { SystemNote, TextDone, TurnFinished, TurnResumed, TurnStarted, TurnWaiting } from '../protocol/turn.ts';

/** Fails to compile when a journal kind has no body below. */
type EveryKind<T extends Record<JournalKind, unknown>> = T;

/** Each journal kind's body. */
export type JournalBodies = EveryKind<{
  'message.user': UserMessage;
  'turn.started': TurnStarted;
  'text.done': TextDone;
  'tool.started': ToolStarted;
  'tool.progress': ToolProgress;
  'tool.finished': ToolFinished;
  'question.asked': QuestionAsked;
  'question.answered': QuestionAnswered;
  'question.expired': QuestionExpired;
  'approval.requested': ApprovalRequested;
  'approval.decided': ApprovalDecided;
  'approval.expired': ApprovalExpired;
  'approval.noted': ApprovalNoted;
  'approval.execution': ApprovalExecuted;
  'draft.updated': DraftUpdated;
  'draft.applied': DraftApplied;
  'draft.discarded': DraftDiscarded;
  'draft.stale': DraftStale;
  'panel.command': PanelCommandEntry;
  'panel.event': PanelEvent;
  'run.status': RunStatus;
  'submission.ended': SubmissionEnded;
  'run.finding': RunFinding;
  'turn.waiting': TurnWaiting;
  'turn.resumed': TurnResumed;
  'turn.finished': TurnFinished;
  'system.note': SystemNote;
  'conversation.changed': ConversationChanged;
}>;

const isObject = (value: JsonValue | undefined): boolean =>
  value !== null && value !== undefined && typeof value === 'object' && !Array.isArray(value);

/**
 * The JSON `value` read as kind `K`'s body. The one place a body is trusted to be its kind's: the API writes each
 * kind's body as its protocol type, and studio/protocol's fixture test reads every fixture body as one.
 */
const asBody = <K extends JournalKind>(value: JsonValue | undefined): JournalBodies[K] | null =>
  (isObject(value) ? (value as unknown as JournalBodies[K]) : null);

/** An entry's body when the entry is of `kind`, else null. */
export const bodyOf = <K extends JournalKind>(entry: JournalEntry | undefined, kind: K): JournalBodies[K] | null =>
  (entry !== undefined && entry.kind === kind ? asBody<K>(entry.body) : null);

export const askedOf = (question: ProjectedQuestion) => asBody<'question.asked'>(question.asked);
export const answeredOf = (question: ProjectedQuestion) => asBody<'question.answered'>(question.answered);
export const requestedOf = (approval: ProjectedApproval) => asBody<'approval.requested'>(approval.requested);
export const decidedOf = (approval: ProjectedApproval) => asBody<'approval.decided'>(approval.decided);
export const notedOf = (approval: ProjectedApproval) => asBody<'approval.noted'>(approval.noted);
export const executedOf = (approval: ProjectedApproval) => asBody<'approval.execution'>(approval.executed);
export const updatedOf = (draft: ProjectedDraft) => asBody<'draft.updated'>(draft.updated);
export const appliedOf = (draft: ProjectedDraft) => (draft.state === 'applied' ? asBody<'draft.applied'>(draft.ended) : null);
export const staleOf = (draft: ProjectedDraft) => (draft.state === 'stale' ? asBody<'draft.stale'>(draft.ended) : null);
export const statusOf = (run: ProjectedRun) => asBody<'run.status'>(run.status);
export const findingsOf = (run: ProjectedRun): RunFinding[] => run.findings.flatMap((finding) => asBody<'run.finding'>(finding) ?? []);
