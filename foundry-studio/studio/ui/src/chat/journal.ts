// Spec 02 section 3.2, the log as a pure function of the journal: what a browser tab holds of one conversation (the
// entries it has, oldest first, each `seq` once) and the state of everything they name, folded as the API's
// projection folds it (studio/api/src/journal/projection.rs). A snapshot or a page of earlier entries brings its
// projection, and per id the tab keeps the object with the larger `as_of_seq` (spec 01 section 3.6); an entry the
// tab already folded past changes nothing, so the same entries from a snapshot or from the stream draw the same log.

import type { Conversation } from '../protocol/conversation.ts';
import type { JournalEntry } from '../protocol/journal.ts';
import type {
  CallState, DraftState, ProjectedApproval, ProjectedCall, ProjectedDraft, ProjectedQuestion, ProjectedRun, ProjectedTurn,
  Projection,
} from '../protocol/projection.ts';
import type { RunProgress } from '../protocol/run.ts';
import type { EntriesPage, Snapshot } from '../protocol/snapshot.ts';
import type { TextDelta } from '../protocol/stream.ts';
import { bodyOf } from './bodies.ts';

export interface LogModel {
  conversation: Conversation;
  /** The entries held, ascending by `seq`. */
  entries: JournalEntry[];
  turns: Map<string, ProjectedTurn>;
  calls: Map<string, ProjectedCall>;
  questions: Map<string, ProjectedQuestion>;
  approvals: Map<string, ProjectedApproval>;
  drafts: Map<string, ProjectedDraft>;
  runs: Map<string, ProjectedRun>;
  /** Blocks still streaming (live only): their text so far. */
  streaming: Map<string, string>;
  /** Each run's latest `run.progress` (live only), with the wall-clock time it arrived. */
  progress: Map<string, RunProgress & { heardAt: number }>;
  lastSeq: number;
}

const byId = <T,>(items: T[], id: (item: T) => string): Map<string, T> => new Map(items.map((item) => [id(item), item]));

export function emptyModel(conversation: Conversation): LogModel {
  return {
    conversation, entries: [], turns: new Map(), calls: new Map(), questions: new Map(), approvals: new Map(),
    drafts: new Map(), runs: new Map(), streaming: new Map(), progress: new Map(), lastSeq: conversation.last_seq,
  };
}

/** Keep per id the projected object with the larger `as_of_seq`. */
function mergeInto<T extends { as_of_seq: number }>(held: Map<string, T>, incoming: T[], id: (item: T) => string): void {
  for (const item of incoming) {
    const mine = held.get(id(item));
    if (mine === undefined || mine.as_of_seq < item.as_of_seq) held.set(id(item), item);
  }
}

export function mergeProjection(model: LogModel, projection: Projection): void {
  mergeInto(model.turns, projection.turns, (turn) => turn.id);
  mergeInto(model.calls, projection.calls, (call) => call.call);
  mergeInto(model.questions, projection.questions, (question) => question.id);
  mergeInto(model.approvals, projection.approvals, (approval) => approval.id);
  mergeInto(model.drafts, projection.drafts, (draft) => draft.id);
  mergeInto(model.runs, projection.runs, (run) => run.run);
}

/** Section 3.3, step 1: the snapshot's entries, its projection and the blocks still streaming. */
export function modelFromSnapshot(snapshot: Snapshot): LogModel {
  const model = emptyModel(snapshot.conversation);
  model.entries = [...snapshot.entries].sort((a, b) => a.seq - b.seq);
  mergeProjection(model, snapshot.projection);
  for (const block of snapshot.active_turn?.streaming ?? []) model.streaming.set(block.block, block.text);
  model.lastSeq = Math.max(snapshot.last_seq, model.entries.at(-1)?.seq ?? 0);
  return model;
}

/** "Show earlier entries": the page merges in without changing an item already drawn. */
export function mergeEarlier(model: LogModel, page: EntriesPage): void {
  const held = new Set(model.entries.map((entry) => entry.seq));
  const earlier = page.entries.filter((entry) => !held.has(entry.seq));
  model.entries = [...earlier, ...model.entries].sort((a, b) => a.seq - b.seq);
  mergeProjection(model, page.projection);
}

export const holds = (model: LogModel, seq: number): boolean => model.entries.some((entry) => entry.seq === seq);

/** Whether the tab holds the conversation's first entry, so nothing earlier can be read. */
export function holdsTheStart(model: LogModel): boolean {
  const first = model.entries[0];
  return first === undefined ? model.conversation.last_seq < model.conversation.first_seq : first.seq <= model.conversation.first_seq;
}

/** Change the object under `id` when `seq` is past what it reflects; a made object starts at `seq`. */
function change<T extends { as_of_seq: number }>(held: Map<string, T>, id: string | null | undefined, seq: number, edit: (item: NoInfer<T>) => NoInfer<T>): boolean {
  if (id === null || id === undefined) return false;
  const item = held.get(id);
  if (item === undefined || item.as_of_seq >= seq) return false;
  held.set(id, { ...edit(item), as_of_seq: seq });
  return true;
}

const isOpenCall = (state: CallState) => state === 'running' || state === 'waiting';

function fold(model: LogModel, entry: JournalEntry): void {
  const { seq, turn } = entry;
  switch (entry.kind) {
    case 'turn.started': {
      const started = bodyOf(entry, 'turn.started');
      if (started === null || turn === null || (model.turns.get(turn)?.as_of_seq ?? -1) >= seq) return;
      model.turns.set(turn, {
        id: turn, n: started.n, state: 'running', author: started.author, attended: started.attended, browser_tab: started.browser_tab,
        message_seq: started.message_seq, waiting_on: null, end_reason: null, reply_block: null, budget: null, as_of_seq: seq,
      });
      return;
    }
    case 'turn.waiting': {
      const waiting = bodyOf(entry, 'turn.waiting');
      if (waiting === null) return;
      change<ProjectedTurn>(model.turns, turn, seq, (t) => ({ ...t, state: 'waiting', waiting_on: waiting.on, reply_block: waiting.reply_block ?? t.reply_block }));
      return;
    }
    case 'turn.resumed':
      change<ProjectedTurn>(model.turns, turn, seq, (t) => ({ ...t, state: 'running', waiting_on: null }));
      return;
    case 'turn.finished': {
      const finished = bodyOf(entry, 'turn.finished');
      if (finished === null) return;
      change<ProjectedTurn>(model.turns, turn, seq, (t) => ({
        ...t, state: finished.outcome, waiting_on: null, end_reason: finished.end_reason ?? null,
        budget: finished.budget ?? null, reply_block: finished.reply_block ?? t.reply_block,
      }));
      return;
    }
    case 'tool.started': {
      const started = bodyOf(entry, 'tool.started');
      if (started === null || (model.calls.get(started.call)?.as_of_seq ?? -1) >= seq) return;
      model.calls.set(started.call, {
        call: started.call, turn, tool: started.tool, class: started.class, state: 'running', summary: started.summary, note: null,
        problem: null, as_of_seq: seq,
      });
      return;
    }
    case 'tool.progress': {
      const progress = bodyOf(entry, 'tool.progress');
      if (progress !== null) change<ProjectedCall>(model.calls, progress.call, seq, (call) => ({ ...call, note: progress.note }));
      return;
    }
    case 'tool.finished': {
      const finished = bodyOf(entry, 'tool.finished');
      if (finished === null || isOpenCall(finished.outcome)) return;
      change<ProjectedCall>(model.calls, finished.call, seq, (call) => ({
        ...call, state: finished.outcome, summary: finished.summary, problem: finished.problem ?? null, note: null,
      }));
      return;
    }
    case 'question.asked': {
      const asked = bodyOf(entry, 'question.asked');
      if (asked === null || (model.questions.get(asked.question)?.as_of_seq ?? -1) >= seq) return;
      model.questions.set(asked.question, {
        id: asked.question, call: asked.call, turn, state: 'open', asked: entry.body, answered: null, as_of_seq: seq,
      });
      return;
    }
    case 'question.answered': {
      const answered = bodyOf(entry, 'question.answered');
      if (answered !== null) change<ProjectedQuestion>(model.questions, answered.question, seq, (question) => ({ ...question, state: 'answered', answered: entry.body }));
      return;
    }
    case 'question.expired':
      change<ProjectedQuestion>(model.questions, bodyOf(entry, 'question.expired')?.question, seq, (question) => ({ ...question, state: 'expired' }));
      return;
    case 'approval.requested': {
      const requested = bodyOf(entry, 'approval.requested');
      if (requested === null || (model.approvals.get(requested.approval)?.as_of_seq ?? -1) >= seq) return;
      change<ProjectedCall>(model.calls, requested.call, seq, (call) => ({ ...call, state: 'waiting' }));
      model.approvals.set(requested.approval, {
        id: requested.approval, call: requested.call, turn, state: 'pending', execution: null, requested: entry.body, decided: null,
        executed: null, noted: null, as_of_seq: seq,
      });
      return;
    }
    case 'approval.decided': {
      const decided = bodyOf(entry, 'approval.decided');
      if (decided === null) return;
      const approval = model.approvals.get(decided.approval);
      if (change<ProjectedApproval>(model.approvals, decided.approval, seq, (a) => ({ ...a, state: decided.decision === 'approve' ? 'approved' : 'denied', decided: entry.body }))) {
        change<ProjectedCall>(model.calls, approval?.call, seq, (call) => ({ ...call, state: decided.decision === 'approve' ? 'running' : 'denied' }));
      }
      return;
    }
    case 'approval.expired':
      change<ProjectedApproval>(model.approvals, bodyOf(entry, 'approval.expired')?.approval, seq, (a) => ({ ...a, state: 'expired' }));
      return;
    case 'approval.noted':
      change<ProjectedApproval>(model.approvals, bodyOf(entry, 'approval.noted')?.approval, seq, (a) => ({ ...a, noted: entry.body }));
      return;
    case 'approval.execution': {
      const executed = bodyOf(entry, 'approval.execution');
      if (executed === null) return;
      // A button's approval, which no `approval.requested` precedes, first appears here, approved.
      if (!model.approvals.has(executed.approval)) {
        model.approvals.set(executed.approval, {
          id: executed.approval, call: null, turn, state: 'approved', execution: null, requested: null, decided: null, executed: null,
          noted: null, as_of_seq: seq - 1,
        });
      }
      change<ProjectedApproval>(model.approvals, executed.approval, seq, (a) => ({ ...a, execution: executed.execution, executed: entry.body }));
      return;
    }
    case 'draft.updated':
    case 'draft.applied':
    case 'draft.discarded':
    case 'draft.stale': {
      const updated = bodyOf(entry, 'draft.updated');
      const id = updated?.draft ?? (bodyOf(entry, 'draft.applied') ?? bodyOf(entry, 'draft.discarded') ?? bodyOf(entry, 'draft.stale'))?.draft;
      if (id === undefined) return;
      const state: DraftState = updated !== null ? updated.state : entry.kind === 'draft.applied' ? 'applied' : entry.kind === 'draft.discarded' ? 'discarded' : 'stale';
      const held = model.drafts.get(id);
      if (held !== undefined && held.as_of_seq >= seq) return;
      const open = state === 'drafting' || state === 'ready';
      model.drafts.set(id, {
        id, turn: held?.turn ?? turn, state, updated: open ? entry.body : held?.updated ?? null, ended: open ? null : entry.body, as_of_seq: seq,
      });
      return;
    }
    case 'run.status':
    case 'run.finding': {
      const id = (bodyOf(entry, 'run.status') ?? bodyOf(entry, 'run.finding'))?.run;
      if (id === undefined) return;
      const held = model.runs.get(id);
      if (held !== undefined && held.as_of_seq >= seq) return;
      const run = held ?? { run: id, status: null, findings: [], as_of_seq: seq };
      model.runs.set(id, entry.kind === 'run.status'
        ? { ...run, status: entry.body, as_of_seq: seq }
        : { ...run, findings: [...run.findings, entry.body], as_of_seq: seq });
      return;
    }
    case 'text.done': {
      const done = bodyOf(entry, 'text.done');
      if (done !== null) model.streaming.delete(done.block);
      return;
    }
    default:
      return;
  }
}

/** Apply one journaled entry: in sequence order, dropped when its `seq` is held (API.md:138). */
export function applyEntry(model: LogModel, entry: JournalEntry): boolean {
  if (holds(model, entry.seq)) return false;
  const after = model.entries.findIndex((held) => held.seq > entry.seq);
  if (after < 0) model.entries.push(entry);
  else model.entries.splice(after, 0, entry);
  model.lastSeq = Math.max(model.lastSeq, entry.seq);
  fold(model, entry);
  return true;
}

/** A `text.delta` placed by `offset`: written over the block from there for its own length, the rest kept. */
export function applyDelta(model: LogModel, delta: TextDelta): void {
  const text = model.streaming.get(delta.block) ?? '';
  const done = model.entries.some((entry) => bodyOf(entry, 'text.done')?.block === delta.block);
  if (done) return;
  model.streaming.set(delta.block, text.slice(0, delta.offset) + delta.delta + text.slice(delta.offset + delta.delta.length));
}

export function applyProgress(model: LogModel, progress: RunProgress, heardAt: number): void {
  model.progress.set(progress.run, { ...progress, heardAt });
}

/** The turn still running or waiting, the latest started if several are open. */
export function openTurn(model: LogModel): ProjectedTurn | null {
  let open: ProjectedTurn | null = null;
  for (const turn of model.turns.values()) {
    if ((turn.state === 'running' || turn.state === 'waiting') && (open === null || turn.message_seq > open.message_seq)) open = turn;
  }
  return open;
}

/** The cards that wait on the person: the questions still open and the approvals still pending. */
export function waitingCards(model: LogModel): { questions: number; approvals: number } {
  let [questions, approvals] = [0, 0];
  for (const question of model.questions.values()) if (question.state === 'open') questions += 1;
  for (const approval of model.approvals.values()) if (approval.state === 'pending') approvals += 1;
  return { questions, approvals };
}

/** The turn whose span holds `seq`: started after its message and not finished before `seq`. */
export function turnAt(model: LogModel, seq: number): ProjectedTurn | null {
  for (const turn of model.turns.values()) {
    const open = turn.state === 'running' || turn.state === 'waiting';
    if (turn.message_seq < seq && (open || turn.as_of_seq >= seq)) return turn;
  }
  return null;
}

export { byId };
