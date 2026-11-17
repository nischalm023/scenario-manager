// Spec 02 section 8, statements 2 to 5 (speech), 24 (a background tab) and 27's announcer half, with section 3.4's
// words as D117 left them (no turn numbers, A2 without "The agent asks you", A5 one sentence, A10 per ending): what the
// announcer is handed as the journal fixtures arrive, live or in a connection's catch-up.

import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { JournalEntry } from '../protocol/journal.ts';
import type { ResolvedRef } from '../protocol/turn.ts';
import { ShellContext } from '../shell/context.ts';
import type { ReplyReading } from '../shell/preferences.ts';
import { entriesOf, snapshotFrom } from '../testing/journal.ts';
import { sessionOn, shellOf } from '../testing/render.tsx';
import { AgentText } from './AgentText.tsx';
import { Announcer } from './announcer.ts';
import { replyToSpeak, sentencesOf, spokenSentences } from './sentences.ts';
import { speechListener } from './speech.ts';

const upTo = (entries: JournalEntry[], seq: number) => entries.filter((entry) => entry.seq <= seq);
const STATION_3: ResolvedRef = {
  key: 'b6a9a763-687c-4067-909b-460e986e2229', kind: 'station', label: 'Pick station 3', route: '/w/customer/demo/site/lshape-6/map@main?select=b6a9a763-687c-4067-909b-460e986e2229',
};

afterEach(() => {
  cleanup();
});

/** A session on the entries up to `from`, then `rest` applied live after the connection's `stream.live`. */
async function hear(entries: JournalEntry[], from: number, rest: JournalEntry[], reading: ReplyReading = 'd70') {
  const said: string[] = [];
  const listener = speechListener({ say: (sentence) => said.push(sentence), reading: () => reading, acted: () => false });
  const { sources } = await sessionOn(snapshotFrom(upTo(entries, from), 1), listener);
  sources[0].open();
  sources[0].emit('stream.live', { last_seq: from });
  for (const entry of rest) sources[0].entry(entry);
  return { said, sources };
}

describe('statement 2: silence', () => {
  /** Spec 02 section 8, statement 2 (silence). */
  it('speaks nothing for deltas, notes, successful tool rows, turn.started, message.user, panel.event and the unspoken findings', async () => {
    const site = entriesOf('site-conversation');
    const quiet = site.filter((entry) => entry.seq >= 1 && entry.seq <= 6);
    const finding = site.find((entry) => entry.kind === 'run.finding') as JournalEntry;
    const read = { ...finding, seq: 7, body: { ...(finding.body as object), finding: { kind: 'read-differs', sentence: 'Run 1 read below a moved head.' } } };
    const compiler = { ...finding, seq: 8, body: { ...(finding.body as object), finding: { kind: 'compiler-differs', sentence: 'Run 1 used compiler 0.2.0.' } } };
    const said: string[] = [];
    const listener = speechListener({ say: (sentence) => said.push(sentence), reading: () => 'full', acted: () => false });
    const { sources } = await sessionOn(snapshotFrom([], 1), listener);
    sources[0].open();
    sources[0].emit('stream.live', { last_seq: 0 });
    for (const entry of quiet) sources[0].entry(entry);
    sources[0].emit('text.delta', { block: 'b9', offset: 0, delta: 'Station 3 waits on totes' });
    sources[0].entry(read as JournalEntry);
    sources[0].entry(compiler as JournalEntry);
    expect(said).toEqual([]);
  });
});

describe('statement 3: ruling D70, the reply rule', () => {
  const three = 'Station 3 waits on totes. Its fill pace sets the line. The draft brings it to 15 s.';
  const five = `${three} Apply it when you are ready. The other stations keep their pace.`;

  /** Spec 02 section 8, statement 3 (ruling D70): three sentences whole, five as the first and a count. */
  it('speaks a three-sentence reply whole, and a five-sentence one as its first and a count', () => {
    expect(replyToSpeak(spokenSentences(three), 'd70')).toBe(three);
    expect(replyToSpeak(spokenSentences(five), 'd70')).toBe('Station 3 waits on totes. 4 more sentences in the reply.');
  });

  /** Spec 02 section 8, statement 3 (ruling D70): each setting value as section 3.4 says. */
  it('follows each setting value', () => {
    expect(replyToSpeak(spokenSentences(five), 'full')).toBe(five);
    expect(replyToSpeak(spokenSentences(five), 'first')).toBe('Station 3 waits on totes. 4 more sentences in the reply.');
    expect(replyToSpeak(spokenSentences('One sentence only.'), 'first')).toBe('One sentence only.');
    expect(replyToSpeak(spokenSentences(five), 'off')).toBeNull();
  });

  /** Spec 02 section 8, statement 3 (ruling D70): the segmenter keeps "e.g." and "1.5" inside a sentence. */
  it('keeps "e.g." and "1.5" inside a sentence', () => {
    expect(sentencesOf('Slow stations, e.g. station 3, wait 1.5 times longer. The rest are fine.')).toEqual([
      'Slow stations, e.g. station 3, wait 1.5 times longer.', 'The rest are fine.',
    ]);
  });

  /**
   * Spec 02 section 3.4, the reply rule: each paragraph, list item and heading ends a sentence; a code block reads by
   * its lines.
   */
  it('ends a sentence at each paragraph, list item and heading, reads a code block by its lines and a chip by its label', () => {
    const reply = '# Findings\n\nTwo stations wait\n\n- [Pick station 3](ref:b6a9a763-687c-4067-909b-460e986e2229)\n- Pick station 4\n\n```\nfill_pace: 15\nadmission: 6\n```';
    expect(spokenSentences(reply, [STATION_3])).toEqual(['Findings.', 'Two stations wait.', 'Pick station 3.', 'Pick station 4.', 'A code block of 2 lines.']);
  });

  /** Spec 02 section 3.4, the reply rule: "a chip reads as its label", the label the screen shows. */
  it('reads a chip as the screen draws it: the label its refs resolve, and "(not found)" for one that does not resolve', () => {
    const reply = 'Look at [station three](ref:b6a9a763-687c-4067-909b-460e986e2229) and [the dock](ref:0c2f6d5e-8a41-4f7e-9b13-5d2e7a9c4f10).';
    const missing = { key: '0c2f6d5e-8a41-4f7e-9b13-5d2e7a9c4f10', kind: null, label: 'Dock 1', route: null };
    const { container } = render(
      <ShellContext.Provider value={shellOf()}>
        <ol><AgentText text={reply} streaming={false} refs={[STATION_3, missing]} /></ol>
      </ShellContext.Provider>,
    );
    // A chip's words on screen: its text but the kind, which is hidden from a reader and shown as a badge.
    const shown = [...container.querySelectorAll('.entity-chip')].map((chip) => {
      const words = chip.cloneNode(true) as Element;
      words.querySelector('.entity-chip__kind')?.remove();
      return words.textContent;
    });
    expect(shown).toEqual(['Pick station 3', 'Dock 1 (not found)']);
    expect(spokenSentences(reply, [STATION_3, missing])).toEqual(['Look at Pick station 3 and Dock 1 (not found).']);
  });
});

describe('statement 4: which block is spoken', () => {
  /** Spec 02 section 8, statement 4 (which block). */
  it('speaks the text before the wait before the question, then the block turn.finished names', async () => {
    const thread = entriesOf('new-site-thread');
    const { said } = await hear(thread, 2, thread.filter((entry) => entry.seq > 2));
    expect(said.slice(0, 3)).toEqual([
      'The agent is working.',
      "I'll look at the site templates first.",
      '2 questions: Which shell should the new site use? How many aisles should the storage hold?',
    ]);
    expect(said.at(-1)).toBe('The site is made: Small L-shape, 6 aisles. Its four stock inlets are highlighted on the map.');
  });

  /** Spec 02 section 8, statement 4 (which block): a note never is spoken. */
  it('never speaks a note', async () => {
    const site = entriesOf('site-conversation');
    const { said } = await hear(site, 3, site.filter((entry) => entry.seq > 3 && entry.seq <= 12));
    expect(said.some((sentence) => sentence.includes('Reading the flow at changeset 3'))).toBe(false);
    expect(said).toContain('Station 3 waits on totes for 20 s each, so its fill pace sets the line. The draft brings it to 15 s; apply it when you are ready.');
  });
});

describe('statement 5: a load', () => {
  /** Spec 02 section 8, statement 5 (load). */
  it('speaks A0 alone for a snapshot holding a pending approval', async () => {
    const said: string[] = [];
    const listener = speechListener({ say: (sentence) => said.push(sentence), reading: () => 'd70', acted: () => false });
    const { sources } = await sessionOn(snapshotFrom(upTo(entriesOf('ended-turns'), 13), 1), listener);
    sources[0].open();
    sources[0].emit('stream.live', { last_seq: 13 });
    expect(said).toEqual(['Approval needed: the agent asks to apply 1 change to the flow.']);
  });
});

describe('statement 27, the announcer half: a failure speaks A5, never a row', () => {
  /** Spec 02 section 8, statement 27 (tool calls), its announcer half. */
  it('hands the announcer "A tool call failed." and never the row\'s words', async () => {
    const ended = entriesOf('ended-turns');
    const { said } = await hear(ended, 29, ended.filter((entry) => entry.seq > 29 && entry.seq <= 31));
    expect(said).toEqual(['The agent is working.', 'A tool call failed.']);
    expect(said.join(' ')).not.toContain('Reading the flow failed');
  });
});

describe('section 3.4, A10: how a turn ended early', () => {
  /** Spec 02 section 3.4, A10: an expired wait says which card expired; a restart says so; no turn number. */
  it('says "The question expired.", "The approval expired." and "The agent was interrupted by a server restart."', async () => {
    const ended = entriesOf('ended-turns');
    const between = (from: number, to: number) => ended.filter((entry) => entry.seq > from && entry.seq <= to);
    // Each load first speaks its A0 for the turn it finds open; what follows is the turn's ending.
    const after = async (from: number, rest: JournalEntry[]) => (await hear(ended, from, rest)).said.slice(1);
    expect(await after(5, between(5, 8))).toEqual(['The question expired.']);
    expect(await after(19, between(19, 22))).toEqual(['The approval expired.']);
    expect(await after(37, between(37, 38))).toEqual(['The agent was interrupted by a server restart.']);
    const finished = ended.find((entry) => entry.seq === 8) as JournalEntry;
    const stopped = { ...finished, body: { ...(finished.body as object), end_reason: 'stopped' } } as JournalEntry;
    expect(await after(5, [...between(5, 7), stopped])).toEqual(['The agent stopped.']);
  });
});

describe("section 3.3, step 4: a connection's catch-up", () => {
  /** Spec 02 section 3.3, step 4, and section 3.4's catch-up set. */
  it('collects what the replay would say and speaks the catch-up set once, at stream.live', async () => {
    const site = entriesOf('site-conversation');
    const said: string[] = [];
    const listener = speechListener({ say: (sentence) => said.push(sentence), reading: () => 'd70', acted: () => false });
    const { sources } = await sessionOn(snapshotFrom(upTo(site, 16), 1), listener);
    sources[0].open();
    for (const entry of site.filter((one) => one.seq > 16)) sources[0].entry(entry);
    expect(said).toEqual([]);
    sources[0].emit('stream.live', { last_seq: 39 });
    // The approval, the drafts and the stale draft no longer wait, so A3, A4 and A6 drop; A13 is never in the set.
    expect(said).toEqual([
      'Run 1 of 5 compiled another plan than the one the check approved; its results may not show the approved flow.',
      'All 5 runs of the sweep have ended: 4 finished, 1 failed.',
      'Four of the five runs finished and one failed; run 1 compiled another plan than the check approved.',
    ]);
    sources[0].emit('stream.live', { last_seq: 39 });
    expect(said).toHaveLength(3);
  });
});

describe('statement 24: a background tab', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    document.getElementById('announcer')?.remove();
  });

  /** Spec 02 section 8, statement 24 (background tab). */
  it('appends nothing while hidden, counts in the title, and sums up in one sentence on return', () => {
    const region = document.createElement('div');
    region.id = 'announcer';
    document.body.append(region);
    const announcer = new Announcer(document);
    announcer.setTitle('Agent Studio, the Map panel');
    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
    announcer.say('Question: Which aisle should I look at first?');
    announcer.say('Run 01M5884Z failed: Watchdog.');
    expect(region.children).toHaveLength(0);
    expect(document.title.startsWith('(2) ')).toBe(true);
    visibility.mockReturnValue('visible');
    document.dispatchEvent(new Event('visibilitychange'));
    expect([...region.children].map((line) => line.textContent)).toEqual([
      'While this tab was in the background: question: Which aisle should I look at first?; run 01M5884Z failed: Watchdog.',
    ]);
    expect(document.title).toBe('Agent Studio, the Map panel');
  });
});
