// Spec 02 section 3.4, which entry speaks what (A0 to A15) and when: an entry applied live speaks at once; during a
// connection's catch-up the speech is collected, kept through a connection that ends before its `stream.live` (a
// `stream.reset` included), and at the next `stream.live` only the newest A1, A2 to A4 for what still waits, and every
// A5, A8, A9 and A14 are spoken. Each announcement is spoken once per tab, keyed by what it is about, never again on a
// redraw; a load speaks A0 alone, history never. Text deltas, notes, tool rows, `turn.started`, the person's own
// message, `panel.event` and what a snapshot draws are never spoken.

import type { JournalEntry } from '../protocol/journal.ts';
import type { ReplyReading } from '../shell/preferences.ts';
import { askedOf, bodyOf, requestedOf } from './bodies.ts';
import { openTurn, type LogModel } from './journal.ts';
import { replyToSpeak, spokenSentences } from './sentences.ts';
import type { StreamListener } from './stream.ts';
import { appliedWords, inSentence, plural, runEnded, runSentence, staleWords } from './words.ts';

export type Code = 'A0' | 'A1' | 'A2' | 'A3' | 'A4' | 'A5' | 'A6' | 'A7' | 'A8' | 'A9' | 'A10' | 'A11' | 'A12' | 'A13' | 'A14' | 'A15';

export interface Announcement {
  code: Code;
  /** What it is about, so it is said once per tab: a block, a card, a run's attempt, an entry. */
  key: string;
  text: string;
  /** For A2 to A4 in a catch-up: whether what it announces still waits. */
  stillWaits?: (model: LogModel) => boolean;
}

export interface SpeechContext {
  say(sentence: string): void;
  reading(): ReplyReading;
  /** Whether this tab made the act an entry records (its own Dismiss or Apply): another's is heard as theirs. */
  acted(entry: JournalEntry): boolean;
}

/** A0: what a load finds waiting or working; history is never read. */
export function loadSpeech(model: LogModel): string | null {
  const open = openTurn(model);
  if (open === null) return null;
  if (open.state === 'running') return 'The agent is working.';
  if (open.waiting_on === 'approval') {
    const pending = [...model.approvals.values()].find((approval) => approval.turn === open.id && approval.state === 'pending');
    return `Approval needed: the agent asks to ${inSentence((pending !== undefined ? requestedOf(pending)?.summary : undefined) ?? 'act')}.`;
  }
  const items = [...model.questions.values()].filter((question) => question.turn === open.id && question.state === 'open')
    .reduce((count, question) => count + (askedOf(question)?.items.length ?? 1), 0);
  return items > 1 ? `The agent waits for your answer to ${items} questions.` : 'The agent waits for your answer to its question.';
}

function replyOf(model: LogModel, block: string | null | undefined, reading: ReplyReading): Announcement | null {
  if (!block) return null;
  const text = model.entries.map((entry) => bodyOf(entry, 'text.done')).find((done) => done?.block === block) ?? null;
  if (text === null || text.kind !== 'text') return null;
  const spoken = replyToSpeak(spokenSentences(text.text, text.refs ?? []), reading);
  return spoken === null ? null : { code: 'A1', key: `block:${block}`, text: spoken };
}

/** Before a wait, the reply is the turn's last text block so far (lean 5). */
function lastTextBlock(model: LogModel, turn: string | null, before: number): string | null {
  const texts = model.entries.filter((entry) => entry.turn === turn && entry.seq < before && bodyOf(entry, 'text.done')?.kind === 'text');
  return bodyOf(texts.at(-1), 'text.done')?.block ?? null;
}

/** A10 for a wait that ran out: the card the turn waited on expired with it. */
function expiredWords(model: LogModel, turn: string | null): string {
  const approval = [...model.approvals.values()].some((one) => one.turn === turn && one.state === 'expired');
  return approval ? 'The approval expired.' : 'The question expired.';
}

/** What one entry, applied to `model`, would have this tab say. */
export function speechFor(entry: JournalEntry, model: LogModel, reading: ReplyReading, acted: boolean): Announcement[] {
  const said: Announcement[] = [];
  const turn = entry.turn !== null ? model.turns.get(entry.turn) : undefined;
  switch (entry.kind) {
    case 'question.asked': {
      const asked = bodyOf(entry, 'question.asked');
      if (asked === null) break;
      const reply = replyOf(model, lastTextBlock(model, entry.turn, entry.seq), reading);
      if (reply !== null) said.push(reply);
      // D117 took the card's "The agent asks you" heading, so A2 says the questions alone.
      const text = asked.items.length === 1
        ? `Question: ${asked.items[0].text}`
        : `${asked.items.length} questions: ${asked.items.map((item) => item.text).join(' ')}`;
      said.push({ code: 'A2', key: `question:${asked.question}`, text, stillWaits: (now) => now.questions.get(asked.question)?.state === 'open' });
      break;
    }
    case 'approval.requested': {
      const requested = bodyOf(entry, 'approval.requested');
      if (requested === null) break;
      const reply = replyOf(model, lastTextBlock(model, entry.turn, entry.seq), reading);
      if (reply !== null) said.push(reply);
      said.push({
        code: 'A3', key: `approval:${requested.approval}`, text: `Approval needed: the agent asks to ${inSentence(requested.summary)}.`,
        stillWaits: (now) => now.approvals.get(requested.approval)?.state === 'pending',
      });
      break;
    }
    case 'turn.finished': {
      const finished = bodyOf(entry, 'turn.finished');
      const reply = replyOf(model, finished?.reply_block ?? turn?.reply_block, reading);
      if (reply !== null) said.push(reply);
      if (finished?.outcome === 'stopped') {
        const text = finished.end_reason === 'wait-expired' ? expiredWords(model, entry.turn) : 'The agent stopped.';
        said.push({ code: 'A10', key: `turn:${entry.seq}`, text });
      }
      if (finished?.outcome === 'interrupted') said.push({ code: 'A10', key: `turn:${entry.seq}`, text: 'The agent was interrupted by a server restart.' });
      break;
    }
    case 'tool.finished':
      // Never the row's words (ruling D81), and the summary says only the count (D117).
      if (bodyOf(entry, 'tool.finished')?.outcome === 'failed') said.push({ code: 'A5', key: `failed:${entry.seq}`, text: 'A tool call failed.' });
      break;
    case 'draft.updated': {
      const updated = bodyOf(entry, 'draft.updated');
      if (updated === null || updated.state !== 'ready') break;
      said.push({
        code: 'A4', key: `ready:${updated.draft}`, text: `Draft ready to review: ${plural(updated.ops_total, 'change')}, not applied.`,
        stillWaits: (now) => now.drafts.get(updated.draft)?.state === 'ready',
      });
      break;
    }
    case 'draft.applied': {
      // A12: the outcome of this person's own Apply, once its changeset lands.
      if (acted) said.push({ code: 'A12', key: `applied:${entry.seq}`, text: appliedWords(bodyOf(entry, 'draft.applied')) });
      break;
    }
    case 'draft.stale':
      said.push({ code: 'A6', key: `stale:${entry.seq}`, text: staleWords(bodyOf(entry, 'draft.stale')) });
      break;
    case 'run.status': {
      const status = bodyOf(entry, 'run.status');
      if (status === null) break;
      if (status.of === 1 || (runEnded(status.state) && (status.state === 'failed' || status.state === 'partial'))) {
        said.push({ code: 'A8', key: `run:${status.run}:${status.attempt}:${status.state}`, text: runSentence(status) });
      }
      break;
    }
    case 'run.finding': {
      const finding = bodyOf(entry, 'run.finding');
      if (finding?.finding.kind !== 'plan-differs') break;
      said.push({ code: 'A8', key: `finding:${finding.run}:${finding.attempt}:plan-differs`, text: finding.finding.sentence ?? `${finding.run} compiled another plan than the check.` });
      break;
    }
    case 'submission.ended': {
      const ended = bodyOf(entry, 'submission.ended');
      if (ended === null) break;
      const counts = (['finished', 'cancelled', 'failed', 'partial'] as const)
        .filter((state) => ended.counts[state] > 0).map((state) => `${ended.counts[state]} ${state}`).join(', ');
      said.push({ code: 'A8', key: `submission:${entry.seq}`, text: `All ${ended.of} runs of the sweep have ended: ${counts}.` });
      break;
    }
    case 'system.note': {
      const text = bodyOf(entry, 'system.note')?.text;
      if (text) said.push({ code: 'A9', key: `note:${entry.seq}`, text });
      break;
    }
    case 'approval.noted': {
      const noted = bodyOf(entry, 'approval.noted');
      if (noted?.note) said.push({ code: 'A13', key: `noted:${noted.approval}`, text: noted.note });
      break;
    }
    case 'approval.execution': {
      const executed = bodyOf(entry, 'approval.execution');
      if (executed === null) break;
      const key = `execution:${executed.approval}:${executed.execution}`;
      if (executed.execution === 'failed') said.push({ code: 'A14', key, text: `The approved change failed: ${executed.sentence ?? 'no reason was given'}` });
      if (executed.execution === 'unknown') said.push({ code: 'A14', key, text: executed.sentence ?? 'Not yet known whether this change landed; checking again.' });
      if (executed.execution === 'ok' && wasUnknown(model, executed.approval, entry.seq)) said.push({ code: 'A14', key, text: 'The approved change landed.' });
      break;
    }
    case 'conversation.changed': {
      const changed = bodyOf(entry, 'conversation.changed');
      if (acted || changed === null) break;
      const text = changed.state === 'live'
        ? `This conversation now belongs to ${changed.title ?? changed.site ?? 'its site'}.`
        : `${changed.by.name} dismissed this conversation; a new one continues this site.`;
      said.push({ code: 'A15', key: `changed:${entry.seq}`, text });
      break;
    }
    default:
      break;
  }
  return said;
}

/** Whether an approval's execution stood `unknown` before `seq`, so its settling is spoken (A14). */
function wasUnknown(model: LogModel, approval: string, seq: number): boolean {
  return model.entries.some((entry) => {
    const executed = bodyOf(entry, 'approval.execution');
    return entry.seq < seq && executed?.approval === approval && executed.execution === 'unknown';
  });
}

/** Section 3.4's catch-up set: the newest A1, A2 to A4 for what still waits, every A5, A8, A9 and A14. */
export function catchUpSet(collected: Announcement[], model: LogModel): Announcement[] {
  const newestReply = collected.filter((one) => one.code === 'A1').at(-1);
  return collected.filter((one) => {
    if (one.code === 'A1') return one === newestReply;
    if (one.code === 'A2' || one.code === 'A3' || one.code === 'A4') return one.stillWaits?.(model) ?? false;
    return one.code === 'A5' || one.code === 'A8' || one.code === 'A9' || one.code === 'A14';
  });
}

/** The stream listener that speaks: A0 on a load, each entry live, the catch-up set at `stream.live`, A11. */
export function speechListener(context: SpeechContext): StreamListener {
  const spoken = new Set<string>();
  let collected: Announcement[] = [];
  const say = (announcement: Announcement) => {
    if (spoken.has(announcement.key)) return;
    spoken.add(announcement.key);
    context.say(announcement.text);
  };
  return {
    loaded: (model, why) => {
      if (why === 'load') collected = [];
      // A0 on a load or a reset, unless what was collected still says that wait in its own words at `stream.live`.
      const waitHeard = collected.some((one) => (one.code === 'A2' || one.code === 'A3') && (one.stillWaits?.(model) ?? false));
      const now = why === 'refusal' || waitHeard ? null : loadSpeech(model);
      if (now !== null) context.say(now);
      // What the snapshot holds is history, never spoken now or on a later redraw, except what a connection that ended
      // before its `stream.live` collected: the next `stream.live` speaks that (section 3.3, step 4).
      const pending = new Set(collected.map((one) => one.key));
      for (const entry of model.entries) {
        for (const announcement of speechFor(entry, model, context.reading(), true)) {
          if (!pending.has(announcement.key)) spoken.add(announcement.key);
        }
      }
    },
    applied: (entry, model, phase) => {
      const said = speechFor(entry, model, context.reading(), context.acted(entry));
      if (phase === 'live') said.forEach(say);
      else collected.push(...said);
    },
    live: (model) => {
      catchUpSet(collected, model).forEach(say);
      collected.forEach((announcement) => spoken.add(announcement.key));
      collected = [];
    },
    loss: (phase) => context.say(phase === 'lost' ? 'Lost the connection to the agent. Reconnecting.' : 'Reconnected.'),
  };
}
