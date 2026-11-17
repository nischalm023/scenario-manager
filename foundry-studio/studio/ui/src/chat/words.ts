// Spec 02 section 3.1's words for each state of the chat's components (tool rows, cards, runs) and section 3.8's
// for what fails, kept in one place so the components and the announcer say the same thing. "working" names a turn
// or tool in progress (lean 10); "run" is kept for simulation runs (ruling D49). Every duration here is simtime.

import type { DraftApplied, DraftStale } from '../protocol/draft.ts';
import type { ContextPanel, PageContext } from '../protocol/page.ts';
import type { ProjectedCall } from '../protocol/projection.ts';
import type { Problem } from '../protocol/problem.ts';
import type { RunState, RunStatus } from '../protocol/run.ts';

export const plural = (count: number, one: string, many = `${one}s`): string => `${count} ${count === 1 ? one : many}`;

/** "Submit 5 runs" read inside a sentence: its first letter lowered unless it starts a name or an acronym. */
export const inSentence = (text: string): string => (/^[A-Z][a-z]/.test(text) ? text[0].toLowerCase() + text.slice(1) : text);

/** The row's `data-state` (spec 01's `ok` is shown as `done`), its glyph and its words. */
export function toolRowState(call: ProjectedCall, deniedBy: string | null): { state: string; glyph: string; words: string } {
  switch (call.state) {
    case 'running': return { state: 'running', glyph: '…', words: call.note ? `working: ${call.note}` : 'working' };
    case 'waiting': return { state: 'waiting', glyph: '?', words: 'waiting for you' };
    case 'ok': return { state: 'done', glyph: '✓', words: 'done' };
    case 'failed': return { state: 'failed', glyph: '✕', words: 'failed' };
    case 'denied': {
      const unattended = (call.problem as { type?: string } | null)?.type === 'approval-required';
      return { state: 'denied', glyph: '⊘', words: unattended ? 'denied: nobody was attached to approve' : `denied by ${deniedBy ?? 'you'}` };
    }
    case 'stopped': return { state: 'stopped', glyph: '■', words: 'stopped' };
    case 'interrupted': return { state: 'interrupted', glyph: '■', words: 'interrupted' };
  }
}

/** ToolCalls' summary, the count alone (D117): a failure shows on its row, and the agent's reply says what it means. */
export const toolCallsCount = (count: number): string => plural(count, 'tool call');

/** "map changeset 1" for a scope path and one of its changesets. */
export const changesetWords = (scope: string, seq: number): string => `${scope.split('/').pop() ?? scope} changeset ${seq}`;

/** "flow changeset 1 and map changeset 1": every scope of a `{ scope: seq }` map, in scope order. */
export function changesetsWords(seqs: Record<string, number>): string {
  const words = Object.keys(seqs).sort().map((scope) => changesetWords(scope, seqs[scope]));
  return words.length <= 1 ? (words[0] ?? '') : `${words.slice(0, -1).join(', ')} and ${words.at(-1)}`;
}

/** "2 h 41 min" of simtime. */
export function simtimeWords(ms: number): string {
  const minutes = Math.floor(ms / 60000);
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (hours === 0) return `${rest} min`;
  return rest === 0 ? `${hours} h` : `${hours} h ${rest} min`;
}

/** A run's name in a sentence: spec 08's short form of its ULID ("Run 01KN0TJZ"), or its place in a sweep. */
export function runName(status: RunStatus): string {
  return status.of > 1 ? `Run ${status.position} of ${status.of}` : `Run ${status.run.slice(0, 8)}`;
}

/** One run's state words and pill tone (section 3.1, RunCard), which the Runs panel says too (spec 08 section 5). */
export function runStateWords(
  status: Pick<RunStatus, 'state' | 'stop_reason' | 'sim_ms'>,
): { words: string; tone: string | undefined; glyph: string | null } {
  const reason = status.stop_reason ?? 'no reason given';
  switch (status.state) {
    case 'queued': return { words: 'queued', tone: 'info', glyph: null };
    case 'running': return { words: 'running', tone: 'info', glyph: '…' };
    case 'finished': return { words: 'finished', tone: 'good', glyph: '✓' };
    case 'cancelled': return { words: `cancelled at ${simtimeWords(status.sim_ms)} of simtime`, tone: undefined, glyph: null };
    case 'failed': return { words: `failed: ${reason}`, tone: 'bad', glyph: '✕' };
    case 'partial': return { words: `stopped early: ${reason}`, tone: 'warn', glyph: null };
  }
}

/**
 * A run's change of state in one sentence, for the "Runs" entry and A8. Spec 08 writes the sentence an entry is to
 * carry; `run.status` carries none yet (spec 01 section 3.5), so the browser words it from the body.
 */
export function runSentence(status: RunStatus): string {
  return `${runName(status)} ${status.state === 'queued' ? 'is queued' : status.state === 'running' ? 'is running' : runStateWords(status).words}.`;
}

export const runEnded = (state: RunState): boolean => state !== 'queued' && state !== 'running';

const PANEL_NAME: Record<ContextPanel, string> = {
  home: 'Home', templates: 'Template list', template: 'Template panel', map: 'Map panel', flow: 'Flow panel',
  history: 'History', scenario: 'Scenario panel', runs: 'Runs panel',
};

/** The page context as its one chip reads it (D117): its title, then the selection, never "No selection". */
export function pageContextWords(context: PageContext): string {
  return context.selection ? `${context.title}, ${context.selection.label} selected` : context.title;
}

/** A panel act's message, as its entry's context line reads it: "Sent from the Template panel by Choose template". */
export function panelSentWords(context: PageContext | null, act: string | null | undefined): string {
  const panel = context !== null ? PANEL_NAME[context.panel] : 'panel';
  return act ? `Sent from the ${panel} by ${act}` : `Sent from the ${panel}`;
}

/** Section 3.8's `turn-running` words, for a send the API refused and for Send pressed while the agent works. */
export const TURN_RUNNING = 'The agent is working. Stop it or wait; your message stays in the box.';

/** What waits on the person in the chat, for a folded chat's tab: "a question waits for you"; null for nothing. */
export function waitingWords({ questions, approvals }: { questions: number; approvals: number }): string | null {
  const cards = [
    questions === 0 ? null : questions === 1 ? 'a question' : `${questions} questions`,
    approvals === 0 ? null : approvals === 1 ? 'an approval' : `${approvals} approvals`,
  ].filter((words) => words !== null);
  if (cards.length === 0) return null;
  return `${cards.join(' and ')} ${questions + approvals === 1 ? 'waits' : 'wait'} for you`;
}

/** Section 3.8's words for a refused or failed message, which leaves the text in the box. */
export function sendFailureWords(problem: Problem | null, reason: string): { words: string; retry: boolean } {
  // The generated ProblemType lists only the types the API's routes answer so far, so the rest compare as text.
  switch (problem?.type as string | undefined) {
    case 'turn-running': return { words: TURN_RUNNING, retry: false };
    case 'idempotency-key-in-flight': return { words: 'Still working on your last press.', retry: false };
    case 'agent-busy': return { words: 'The agent service is busy; try again in a minute. Your message stays in the box.', retry: false };
    case 'conversation-too-long':
      return { words: 'This conversation is too long for the agent, even summarised. Dismiss it to start a new one for this site.', retry: false };
    default:
      return { words: `Your message was not sent: ${reason.replace(/\.\s*$/, '')}. It is still in the box.`, retry: true };
  }
}

/** Section 3.8's words for a refused answer, decision or press on a card. */
export function cardFailureWords(problem: Problem | null, reason: string): string {
  const type = problem?.type as string | undefined;
  if (type === 'question-closed' || type === 'turn-not-waiting') return 'Already answered in another tab.';
  if (type === 'approval-closed') return 'This approval is no longer pending.';
  if (type === 'idempotency-key-in-flight') return 'Still working on your last press.';
  return `Not sent: ${reason.replace(/\.\s*$/, '')}.`;
}

/** A12 for an Apply whose draft landed: "Applied as flow changeset 2." */
export const appliedWords = (applied: DraftApplied | null): string => `Applied as ${changesetsWords(applied?.seqs ?? {})}.`;

/** The `detail` of a problem body (spec 01 section 3.14) an entry carries as JSON. */
const detailOf = (problem: unknown): string | undefined => {
  const detail = problem !== null && typeof problem === 'object' ? (problem as { detail?: unknown }).detail : undefined;
  return typeof detail === 'string' ? detail : undefined;
};

/** A stale draft's sentence by its reason (spec 04 section 3.8), on its card and in A6. */
export function staleWords(stale: DraftStale | null): string {
  switch (stale?.reason) {
    case 'refused': return `world-proto-server refused it: ${detailOf(stale.problem) ?? 'no reason was given'}`;
    case 'generators-moved': return 'The generators that derive this site changed after the preview; check and approve again.';
    default: return detailOf(stale?.problem) ?? 'The site moved on after this draft was built; nothing was applied.';
  }
}
