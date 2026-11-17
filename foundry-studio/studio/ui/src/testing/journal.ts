// Test support for the chat's unit tests of spec 02 section 8 (and section 3.2, the log as a function of the journal):
// the journal fixtures of studio/protocol/tests/journal (the ones the API's test writer replays), read as entries
// and live items, folded into a tab's model from an empty conversation, or cut into the window a snapshot of the last
// entries would hold, its projection folded by this page's own reducer: a test of parity with the API's projection
// reads the API's instead (tests/browser/log.spec.ts).

import type { Conversation } from '../protocol/conversation.ts';
import type { JournalEntry } from '../protocol/journal.ts';
import type { Projection } from '../protocol/projection.ts';
import type { Snapshot } from '../protocol/snapshot.ts';
import type { LiveItem } from '../protocol/stream.ts';
import { applyEntry, emptyModel, openTurn, type LogModel } from '../chat/journal.ts';
import endedTurns from '../../../../studio/protocol/tests/journal/ended-turns.jsonl?raw';
import newSiteThread from '../../../../studio/protocol/tests/journal/new-site-thread.jsonl?raw';
import siteConversation from '../../../../studio/protocol/tests/journal/site-conversation.jsonl?raw';

export type FixtureLine = { entry: JournalEntry } | { live: LiveItem };

const FIXTURES = { 'site-conversation': siteConversation, 'new-site-thread': newSiteThread, 'ended-turns': endedTurns };
export type FixtureName = keyof typeof FIXTURES;

export function fixture(name: FixtureName): FixtureLine[] {
  return FIXTURES[name].split('\n').filter((line) => line.trim() !== '').map((line) => {
    const read = JSON.parse(line) as JournalEntry | { live: LiveItem };
    return 'live' in read ? { live: read.live } : { entry: read };
  });
}

export const entriesOf = (name: FixtureName): JournalEntry[] =>
  fixture(name).flatMap((line) => ('entry' in line ? [line.entry] : []));

export function conversationOf(entries: JournalEntry[], site: string | null = 'customer/demo/site/lshape-6'): Conversation {
  return {
    id: entries[0]?.conversation ?? '01M5881JF0224GZ9HTHHCD0AG9', project: '01M5880000000000000000000P', site, state: 'live',
    title: null, created_by: 'dev', created_at: '2026-10-18T19:00:00.000Z', bound_at: null, ended_at: null, ended_by: null,
    first_seq: 1, last_seq: entries.at(-1)?.seq ?? 0, model_label: 'Claude Sonnet 5.5',
  };
}

/** A tab that read every entry from the first, as a full replay of the journal draws it. */
export function replayed(entries: JournalEntry[], site: string | null = 'customer/demo/site/lshape-6'): LogModel {
  const model = emptyModel({ ...conversationOf(entries, site), last_seq: 0 });
  for (const entry of entries) applyEntry(model, entry);
  model.conversation = conversationOf(entries, site);
  return model;
}

/** The objects a range of entries names (the API's `Named`): each entry's turn, call, card, draft and run. */
function named(entries: JournalEntry[]) {
  const ids = { turns: new Set<string>(), calls: new Set<string>(), cards: new Set<string>(), drafts: new Set<string>(), runs: new Set<string>() };
  for (const entry of entries) {
    if (entry.turn) ids.turns.add(entry.turn);
    const body = (entry.body ?? {}) as Record<string, unknown>;
    if (typeof body.call === 'string') ids.calls.add(body.call);
    for (const field of ['question', 'approval', 'id']) if (typeof body[field] === 'string') ids.cards.add(body[field] as string);
    if (typeof body.draft === 'string') ids.drafts.add(body.draft);
    if (typeof body.run === 'string') ids.runs.add(body.run);
    if (entry.caused_by) {
      ids.cards.add(entry.caused_by);
      ids.calls.add(entry.caused_by);
    }
  }
  return ids;
}

/** The snapshot a tab would read with only the entries from `fromSeq` on: the API's projection of what they name. */
export function snapshotFrom(entries: JournalEntry[], fromSeq: number): Snapshot {
  const full = replayed(entries);
  const window = entries.filter((entry) => entry.seq >= fromSeq);
  const ids = named(window);
  const ofTurn = (turn: string | null) => turn !== null && ids.turns.has(turn);
  const questions = [...full.questions.values()].filter((q) => ids.cards.has(q.id) || ofTurn(q.turn) || q.state === 'open');
  const approvals = [...full.approvals.values()].filter((a) => ids.cards.has(a.id) || ofTurn(a.turn) || a.state === 'pending');
  const drafts = [...full.drafts.values()].filter((d) => ids.drafts.has(d.id) || ofTurn(d.turn) || d.state === 'drafting' || d.state === 'ready');
  const cardTurns = new Set([...questions, ...approvals, ...drafts].map((card) => card.turn).filter((turn): turn is string => turn !== null));
  const projection: Projection = {
    turns: [...full.turns.values()].filter((t) => ids.turns.has(t.id) || cardTurns.has(t.id)),
    calls: [...full.calls.values()].filter((c) => ids.calls.has(c.call) || ofTurn(c.turn)),
    questions, approvals, drafts,
    runs: [...full.runs.values()].filter((r) => ids.runs.has(r.run)),
  };
  const open = openTurn(full);
  return {
    conversation: conversationOf(entries), entries: window, projection,
    pending: {
      questions: questions.filter((q) => q.state === 'open').map((q) => q.id),
      approvals: approvals.filter((a) => a.state === 'pending').map((a) => a.id),
      drafts: drafts.filter((d) => d.state === 'drafting' || d.state === 'ready').map((d) => d.id),
    },
    active_turn: open === null ? null : { id: open.id, n: open.n, state: open.state, waiting_on: open.waiting_on, streaming: [] },
    last_seq: entries.at(-1)?.seq ?? 0,
  };
}
