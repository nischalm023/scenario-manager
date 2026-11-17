// Spec 02 sections 3.1 and 3.2: the log a browser tab draws, a pure function of the journal it holds (`LogModel`)
// and of what only this tab knows (its optimistic messages, the `show`s it held back, its own Dismiss, and an Apply
// press whose execution the journal does not hold yet). Each journal entry either makes an item in journal order or
// changes one already made: a user entry, then its turn with the turn's items (one closed ToolCalls first, the
// changeset cards last), system entries, and run cards. Nothing here renders; the components draw these items.

import type { ConversationChanged } from '../protocol/conversation.ts';
import type { DraftApplied } from '../protocol/draft.ts';
import type { UserMessage } from '../protocol/message.ts';
import type { PageContext } from '../protocol/page.ts';
import type { Panel } from '../protocol/panel.ts';
import type { Person } from '../protocol/person.ts';
import type {
  ProjectedApproval, ProjectedCall, ProjectedDraft, ProjectedQuestion, ProjectedRun, ProjectedTurn,
} from '../protocol/projection.ts';
import type { ResolvedRef } from '../protocol/turn.ts';
import { bodyOf, decidedOf, executedOf, requestedOf, statusOf, updatedOf } from './bodies.ts';
import { holdsTheStart, openTurn, turnAt, type LogModel } from './journal.ts';
import { parseLocation } from '../shell/grammar.ts';
import { changesetWords, changesetsWords, inSentence, panelSentWords, plural, runEnded, runSentence } from './words.ts';

/** A message this tab sent and the journal has not yet carried back (section 3.3, step 10). */
export interface Optimistic {
  clientMessageId: string;
  text: string;
  /** The page context sent with it, null when the person left it out. */
  context: PageContext | null;
  /** How it was sent, in words (a panel act's), or '' for a message from the composer. */
  sentAs: string;
  state: 'sending' | 'sent';
}

/** What only this browser tab knows, beside the journal. */
export interface TabLocal {
  optimistic: Optimistic[];
  /** `panel.command` entries whose `show` this tab held back (spec 03 section 3.3, check 3): their suggestion. */
  heldBack: Set<number>;
  /** `conversation.changed` entries this tab caused, which it does not announce as another's act. */
  actedChanges: Set<number>;
  /**
   * The approvals this tab's Apply presses made, each with the draft it applies, from the press's 202 (section 3.1,
   * Button acts). Until the journal holds an approval's first execution, its card reads "Applying" from this alone;
   * after that the journal alone draws the card, and this says only that the outcome is this tab's own act (A12).
   */
  presses: Map<string, string>;
  /** System entries carried from a conversation this tab followed away from (`conversation.changed` with `next`). */
  carried: SystemItem[];
  /** Whether this tab read earlier entries with "Show earlier entries" (F8). */
  pagedBack: boolean;
}

export const emptyLocal = (): TabLocal => ({
  optimistic: [], heldBack: new Set(), actedChanges: new Set(), presses: new Map(), carried: [], pagedBack: false,
});

export interface CommandLinks {
  seq: number;
  panel: Panel;
  route: string;
  asItWas: string | null;
  asItWasWords: string | null;
}

export interface SystemItem {
  type: 'system';
  key: string;
  kind: string;
  speaker: string;
  text: string;
}

export interface RunCardModel {
  submission: string;
  of: number;
  /** Each run of the submission the journal has named, by position. */
  runs: ProjectedRun[];
}

export interface UserItem {
  type: 'user';
  key: string;
  seq: number | null;
  author: Person | null;
  text: string;
  turnN: number | null;
  /** How the message was sent, said under it ("Sent as your answer"); '' for a plain message from the composer. */
  sentAs: string;
  /** The page context the agent was sent with the message, behind an "i" under it (ruling D190); null for none. */
  page: PageContext | null;
  sending: boolean;
}

export type TurnItem =
  | { type: 'tools'; key: string; calls: ProjectedCall[]; links: Map<string, CommandLinks> }
  | { type: 'text'; key: string; text: string; streaming: boolean; refs: ResolvedRef[] }
  | { type: 'question'; key: string; question: ProjectedQuestion }
  | { type: 'approval'; key: string; approval: ProjectedApproval }
  | { type: 'run'; key: string; card: RunCardModel }
  | { type: 'suggestion'; key: string; links: CommandLinks }
  | SystemItem
  | ({ type: 'draft'; key: string; draft: ProjectedDraft } & DraftApply);

export interface TurnItemList {
  type: 'turn';
  key: string;
  turn: ProjectedTurn;
  items: TurnItem[];
  error: { reason: string; resend: string | null } | null;
}

export type LogItem =
  | { type: 'earlier'; key: 'earlier'; start: boolean }
  | { type: 'welcome'; key: 'welcome' }
  | UserItem
  | TurnItemList
  | SystemItem
  | { type: 'run'; key: string; card: RunCardModel }
  | { type: 'execution'; key: string; approval: ProjectedApproval };

const PANEL_SPEAKER: Record<string, string> = {
  'knob.changed': 'Template panel', 'flow.draft_changed': 'Flow panel', 'scenario.changed': 'Scenario panel',
  'experiment.prepared': 'Scenario panel', 'runs.submitted': 'Runs panel',
};

const PANEL_WORD: Record<Panel, string> = { map: 'Map', flow: 'Flow', template: 'Template', scenario: 'Scenario', runs: 'Runs' };
export const panelWord = (panel: Panel): string => PANEL_WORD[panel];

/** "as it was at map changeset 1": the frozen route's location in words. */
function asItWasWords(route: string | null): string | null {
  if (route === null) return null;
  try {
    const location = parseLocation(decodeURIComponent(new URL(route, 'http://studio.invalid').pathname.replace(/^\/w\//, '')));
    return location.pin.kind === 'changeset' ? changesetWords(location.scope, location.pin.seq) : null;
  } catch {
    return null;
  }
}

export function commandLinks(seq: number, command: { panel: Panel; route: string; as_it_was: string | null }): CommandLinks {
  return { seq, panel: command.panel, route: command.route, asItWas: command.as_it_was, asItWasWords: asItWasWords(command.as_it_was) };
}

/** How a message was sent, unless plainly from the composer: as an answer, as a denial's reason, from a panel. */
function sentAsWords(model: LogModel, message: UserMessage, seq: number, turnId: string | null): string {
  if (turnId !== null) {
    const turn = model.turns.get(turnId);
    if (turn !== undefined && turn.message_seq < seq) {
      const wait = [...model.entries].reverse().find((entry) => entry.seq < seq && entry.turn === turnId && entry.kind === 'turn.waiting');
      const on = bodyOf(wait, 'turn.waiting');
      if (on?.on === 'approval') {
        const approval = model.approvals.get(on.id);
        const summary = (approval !== undefined ? requestedOf(approval)?.summary : undefined) ?? 'the approval';
        return `Sent as the reason for denying: ${inSentence(summary)}`;
      }
      return 'Sent as your answer';
    }
  }
  return message.source === 'panel' ? panelSentWords(message.context, message.act) : '';
}

/** The "Site changes" entry: no person (`draft.applied` names none) and no turn number (D117). */
function draftChangeWords(model: LogModel, draftId: string, applied: DraftApplied | null): string {
  if (applied === null) return 'The draft was discarded.';
  const draft = model.drafts.get(draftId);
  const updated = draft !== undefined ? updatedOf(draft) : null;
  const changes = updated ? `${plural(updated.ops_total, 'change')}, ` : '';
  return `The draft was applied: ${changes}as ${changesetsWords(applied.seqs)}.`;
}

function changedWords(changed: ConversationChanged): string {
  if (changed.state === 'live') return `This conversation now belongs to ${changed.title ?? changed.site ?? 'its site'}.`;
  if (changed.state === 'archived') return `${changed.by.name} dismissed this conversation; a new one continues this site.`;
  return `${changed.by.name} discarded this new site's conversation.`;
}

/** Whose yes an approval carries out: its execution's approver, else the person who decided it. */
export function approverOf(approval: ProjectedApproval): Person | null {
  return executedOf(approval)?.approver ?? decidedOf(approval)?.author ?? null;
}

/**
 * Whether a message denied this approval (spec 01 section 3.10, rule 3): the wait on it ended `by: "message"`. The
 * composer says nothing before such a send, so the card says it after (lean 6; the lead's "Denied by your message").
 */
export function deniedByMessage(model: LogModel, approval: ProjectedApproval): boolean {
  if (approval.state !== 'denied' || approval.turn === null) return false;
  const waited = model.entries.find((entry) => entry.turn === approval.turn && bodyOf(entry, 'turn.waiting')?.id === approval.id);
  const resumed = waited && model.entries.find((entry) => entry.seq > waited.seq && entry.turn === approval.turn && entry.kind === 'turn.resumed');
  return bodyOf(resumed, 'turn.resumed')?.by === 'message';
}

/** Whether an approved act's execution has a line to show: failed, not yet known, or not attributable. */
export function executionShown(approval: ProjectedApproval): boolean {
  const executed = executedOf(approval);
  return executed !== null && (executed.unattributable !== undefined || executed.execution === 'failed' || executed.execution === 'unknown');
}

/** How a draft's Apply stands: the latest button approval the journal says applies it, and "Applying". */
export interface DraftApply {
  execution: ProjectedApproval | null;
  applying: boolean;
}

/**
 * Section 3.1, Button acts: "Applying" from the 202 until the approval's execution ends. The journal decides it once it
 * holds an execution naming the draft; a press of this tab's whose approval the journal does not hold yet stands in
 * until then, so a 202 that arrives after the execution ended changes nothing.
 */
export function applyOf(model: LogModel, local: TabLocal, draft: string): DraftApply {
  let execution: ProjectedApproval | null = null;
  for (const approval of model.approvals.values()) {
    const latest = execution === null || approval.as_of_seq > execution.as_of_seq;
    if (approval.requested === null && executedOf(approval)?.draft === draft && latest) execution = approval;
  }
  const unheard = [...local.presses].some(([approval, pressed]) => pressed === draft && !model.approvals.has(approval));
  const running = execution?.execution === 'not_started' || execution?.execution === 'running';
  return { execution, applying: unheard || running };
}

export function drawLog(model: LogModel, local: TabLocal): LogItem[] {
  const asked = new Set(model.entries.flatMap((entry) => {
    const card = bodyOf(entry, 'question.asked')?.question ?? bodyOf(entry, 'approval.requested')?.approval;
    return card === undefined ? [] : [card];
  }));
  const heldAsked = (id: string) => asked.has(id);
  const items: LogItem[] = [];
  const turns = new Map<string, TurnItemList>();
  const runCards = new Map<string, RunCardModel>();
  const executions = new Set<string>();
  const settled = new Set<string>();
  const drawnCards = new Set<string>();

  if (model.entries.length > 0 && !holdsTheStart(model)) items.push({ type: 'earlier', key: 'earlier', start: false });
  else if (local.pagedBack) items.push({ type: 'earlier', key: 'earlier', start: true });
  items.push(...local.carried);
  if (model.entries.length === 0 && local.optimistic.length === 0) items.push({ type: 'welcome', key: 'welcome' });

  const toolsOf = (turn: TurnItemList) => {
    let tools = turn.items.find((item) => item.type === 'tools');
    if (tools === undefined) {
      tools = { type: 'tools', key: 'tools', calls: [], links: new Map() };
      turn.items.unshift(tools);
    }
    return tools as Extract<TurnItem, { type: 'tools' }>;
  };
  const turnItem = (turnId: string): TurnItemList | null => {
    const held = turns.get(turnId);
    if (held !== undefined) return held;
    const turn = model.turns.get(turnId);
    if (turn === undefined) return null;
    const made: TurnItemList = { type: 'turn', key: `turn-${turnId}`, turn, items: [], error: null };
    turns.set(turnId, made);
    items.push(made);
    // A turn the held entries begin inside still draws its rows and the cards asked before them (the projection).
    const calls = [...model.calls.values()].filter((call) => call.turn === turnId);
    if (calls.length > 0) toolsOf(made).calls.push(...calls);
    for (const question of model.questions.values()) {
      if (question.turn === turnId && !heldAsked(question.id)) {
        drawnCards.add(question.id);
        made.items.push({ type: 'question', key: question.id, question });
      }
    }
    for (const approval of model.approvals.values()) {
      if (approval.turn === turnId && approval.requested !== null && !heldAsked(approval.id)) {
        drawnCards.add(approval.id);
        made.items.push({ type: 'approval', key: approval.id, approval });
      }
    }
    return made;
  };

  for (const entry of model.entries) {
    const { seq } = entry;
    if (entry.kind === 'message.user') {
      const message = bodyOf(entry, 'message.user');
      if (message === null) continue;
      if (message.client_message_id) settled.add(message.client_message_id);
      items.push({
        type: 'user', key: `user-${seq}`, seq, author: message.author, text: message.text,
        turnN: entry.turn ? model.turns.get(entry.turn)?.n ?? null : null,
        sentAs: sentAsWords(model, message, seq, entry.turn), page: message.context, sending: false,
      });
      continue;
    }
    const placed = entry.turn ?? (entry.kind.startsWith('run.') || entry.kind === 'submission.ended' ? turnAt(model, seq)?.id ?? null : null);
    const turn = placed !== null ? turnItem(placed) : null;
    /** An item a turn holds, or the log itself when no turn does. */
    const add = (item: SystemItem | { type: 'run'; key: string; card: RunCardModel }) => {
      if (turn !== null) turn.items.push(item);
      else items.push(item);
    };

    switch (entry.kind) {
      case 'text.done': {
        const done = bodyOf(entry, 'text.done');
        // A `note` is the model's reasoning, which only a journal from before the service stopped writing it holds:
        // the chat never draws it (D117).
        if (done !== null && done.kind === 'text' && turn !== null) {
          turn.items.push({ type: 'text', key: done.block, text: done.text, streaming: false, refs: done.refs ?? [] });
        }
        break;
      }
      case 'tool.started':
        if (turn !== null) toolsOf(turn);
        break;
      case 'question.asked': {
        const question = model.questions.get(bodyOf(entry, 'question.asked')?.question ?? '');
        // One card per question, however often a faulty writer journals its asking.
        if (question !== undefined && turn !== null && !drawnCards.has(question.id)) {
          drawnCards.add(question.id);
          turn.items.push({ type: 'question', key: question.id, question });
        }
        break;
      }
      case 'approval.requested': {
        const approval = model.approvals.get(bodyOf(entry, 'approval.requested')?.approval ?? '');
        if (approval !== undefined && turn !== null && !drawnCards.has(approval.id)) {
          drawnCards.add(approval.id);
          turn.items.push({ type: 'approval', key: approval.id, approval });
        }
        break;
      }
      case 'approval.execution': {
        const id = bodyOf(entry, 'approval.execution')?.approval ?? '';
        const approval = model.approvals.get(id);
        if (approval === undefined || approval.requested !== null || executions.has(id)) break;
        executions.add(id);
        // A button's line joins its draft's card when the log draws that card (filtered below), else stands alone.
        if (executionShown(approval)) items.push({ type: 'execution', key: `execution-${id}`, approval });
        break;
      }
      case 'draft.updated': {
        const draft = model.drafts.get(bodyOf(entry, 'draft.updated')?.draft ?? '');
        const owner = draft?.turn ? turnItem(draft.turn) : turn;
        if (draft !== undefined && owner !== null && !owner.items.some((item) => item.type === 'draft' && item.draft.id === draft.id)) {
          owner.items.push({ type: 'draft', key: `draft-${draft.id}`, draft, execution: null, applying: false });
        }
        break;
      }
      case 'draft.applied': case 'draft.discarded': {
        const applied = bodyOf(entry, 'draft.applied');
        const draft = applied?.draft ?? bodyOf(entry, 'draft.discarded')?.draft;
        if (draft === undefined) break;
        items.push({ type: 'system', key: `system-${seq}`, kind: entry.kind, speaker: 'Site changes', text: draftChangeWords(model, draft, applied) });
        break;
      }
      case 'panel.command': {
        const command = bodyOf(entry, 'panel.command');
        const call = entry.caused_by;
        if (command === null) break;
        if (turn !== null && call !== null) toolsOf(turn).links.set(call, commandLinks(seq, command));
        if (local.heldBack.has(seq) && turn !== null) turn.items.push({ type: 'suggestion', key: `suggestion-${seq}`, links: commandLinks(seq, command) });
        break;
      }
      case 'panel.event': {
        const event = bodyOf(entry, 'panel.event');
        if (event !== null) items.push({ type: 'system', key: `system-${seq}`, kind: 'panel.event', speaker: PANEL_SPEAKER[event.kind] ?? 'Agent Studio', text: event.text });
        break;
      }
      case 'run.status': {
        const status = bodyOf(entry, 'run.status');
        if (status === null) break;
        let card = runCards.get(status.submission);
        if (card === undefined) {
          card = { submission: status.submission, of: status.of, runs: [] };
          runCards.set(status.submission, card);
          add({ type: 'run', key: `run-${status.submission}`, card });
        }
        if (runEnded(status.state) && (status.of === 1 || status.state === 'failed' || status.state === 'partial')) {
          items.push({ type: 'system', key: `system-${seq}`, kind: 'run.status', speaker: 'Runs', text: runSentence(status) });
        }
        break;
      }
      case 'submission.ended': {
        const ended = bodyOf(entry, 'submission.ended');
        if (ended === null) break;
        const counts = (['finished', 'cancelled', 'failed', 'partial'] as const)
          .filter((state) => ended.counts[state] > 0).map((state) => `${ended.counts[state]} ${state}`).join(', ');
        items.push({ type: 'system', key: `system-${seq}`, kind: 'submission.ended', speaker: 'Runs', text: `All ${ended.of} runs of the sweep have ended: ${counts}` });
        break;
      }
      case 'system.note': {
        const note = bodyOf(entry, 'system.note');
        if (note === null) break;
        if (turn !== null && turn.turn.state === 'failed') turn.error = { reason: note.text, resend: null };
        else add({ type: 'system', key: `system-${seq}`, kind: 'system.note', speaker: 'Agent Studio', text: note.text });
        break;
      }
      case 'conversation.changed': {
        const changed = bodyOf(entry, 'conversation.changed');
        if (changed === null || local.actedChanges.has(seq)) break;
        items.push({ type: 'system', key: `system-${seq}`, kind: 'conversation.changed', speaker: 'Agent Studio', text: changedWords(changed) });
        break;
      }
      default:
        break;
    }
  }

  // Every run the journal named joins its submission's card, in position order.
  for (const run of model.runs.values()) {
    const status = statusOf(run);
    const card = status ? runCards.get(status.submission) : undefined;
    if (card !== undefined) card.runs.push(run);
  }
  for (const card of runCards.values()) card.runs.sort((a, b) => (statusOf(a)?.position ?? 0) - (statusOf(b)?.position ?? 0));

  for (const draft of model.drafts.values()) {
    const owner = draft.turn !== null ? turns.get(draft.turn) : undefined;
    if (owner !== undefined && !owner.items.some((item) => item.type === 'draft' && item.draft.id === draft.id)) {
      owner.items.push({ type: 'draft', key: `draft-${draft.id}`, draft, execution: null, applying: false });
    }
  }

  // A call that asked a question still open waits for the person (section 4.3), as one an approval holds does.
  const cards = new Set<string>();
  const asking = new Set([...model.questions.values()].filter((question) => question.state === 'open').map((question) => question.call));
  const open = openTurn(model);
  for (const turn of turns.values()) {
    for (const item of turn.items) {
      if (item.type === 'tools') item.calls = item.calls.map((call) => (call.state === 'running' && asking.has(call.call) ? { ...call, state: 'waiting' } : call));
    }
    // Live blocks still streaming belong to the open turn, after its journaled items.
    if (open !== null && turn.turn.id === open.id) {
      for (const [block, text] of model.streaming) {
        if (!turn.items.some((item) => item.type === 'text' && item.key === block)) {
          turn.items.push({ type: 'text', key: block, text, streaming: true, refs: [] });
        }
      }
    }
    // The changeset cards stay the turn's last, after the words that explain them; a button's execution joins its card.
    const drafts = turn.items.filter((item) => item.type === 'draft');
    drafts.forEach((item) => cards.add(item.draft.id));
    turn.items = [...turn.items.filter((item) => item.type !== 'draft'), ...drafts.map((item) => ({ ...item, ...applyOf(model, local, item.draft.id) }))];
    if (turn.turn.state === 'interrupted' && turn.error === null) {
      turn.error = { reason: 'The server restarted while the agent worked on this turn.', resend: null };
    }
    if (turn.turn.state === 'failed' && turn.error === null) {
      turn.error = { reason: `The turn failed${turn.turn.end_reason ? `: ${turn.turn.end_reason}` : ''}.`, resend: null };
    }
    if (turn.error !== null) {
      const message = model.entries.find((entry) => entry.seq === turn.turn.message_seq);
      turn.error.resend = bodyOf(message, 'message.user')?.text ?? null;
    }
  }

  for (const optimistic of local.optimistic) {
    if (settled.has(optimistic.clientMessageId)) continue;
    items.push({
      type: 'user', key: `optimistic-${optimistic.clientMessageId}`, seq: null, author: null, text: optimistic.text,
      turnN: null, sentAs: optimistic.state === 'sending' ? 'Sending' : optimistic.sentAs, page: optimistic.context,
      sending: optimistic.state === 'sending',
    });
  }
  return items.filter((item) => item.type !== 'execution' || !cards.has(executedOf(item.approval)?.draft ?? ''));
}
