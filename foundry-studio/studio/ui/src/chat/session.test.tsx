// Spec 02 section 3.2 (the same entries draw the same log), section 3.1's Button acts ("Applying" from the 202 until
// the approval's execution ends), section 3.3, step 10 (a message settles once, by its `client_message_id`) and
// section 6's `sendMessage` (a panel act's message): what this browser tab's own acts leave in the log, whichever of
// the journal and the POST's answer arrives first.

import { act, cleanup, fireEvent, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { JournalEntry } from '../protocol/journal.ts';
import { sendPanelMessage } from '../shell/panels.tsx';
import { entriesOf, snapshotFrom } from '../testing/journal.ts';
import { CONTEXT, renderChat, sessionOn } from '../testing/render.tsx';
import type { ActOutcome } from './chatContext.ts';
import type { ChatSession } from './session.ts';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const ENTRIES = entriesOf('site-conversation');
const APPROVAL = '01M5883D20F6Z9NMTBT57BQWWK';
const DRAFT = '01M588260071VJHYRHCBRYT57D';
const upTo = (seq: number) => ENTRIES.filter((entry) => entry.seq <= seq);
const at = (seq: number) => ENTRIES.find((entry) => entry.seq === seq) as JournalEntry;
/** The fixture's first execution of the Apply press, its state and sentence changed. */
const execution = (seq: number, state: string, sentence?: string): JournalEntry =>
  ({ ...at(13), seq, body: { ...(at(13).body as object), execution: state, ...(sentence ? { sentence } : {}) } }) as JournalEntry;
const accepted = () => new Response(JSON.stringify({ approval: { id: APPROVAL } }), { status: 202 });
/** The Apply button in its working words, as the markup holds it. */
const APPLYING = '>Applying</button>';

/** A tab on the fixture up to `seq`, its stream past `stream.live`. */
async function liveTab(seq: number) {
  const tab = await sessionOn(snapshotFrom(upTo(seq), 1));
  tab.sources[0].open();
  tab.sources[0].emit('stream.live', { last_seq: seq });
  return tab;
}

/** The log `session` draws, as markup. */
function logOf(session: ChatSession): string {
  const { container, unmount } = renderChat(session);
  const html = container.querySelector('#chat-log')?.outerHTML ?? '';
  unmount();
  return html;
}

describe('section 3.2: the same entries draw the same log, in the tab that pressed Apply and in any other', () => {
  /** Spec 02 section 3.2: the same entries draw the same log, whichever browser tab pressed Apply. */
  it('draws the execution, the attribution question with its approver\'s buttons and "Applying" alike once the journal holds them', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => accepted()));
    const pressing = await liveTab(12);
    const other = await liveTab(12);
    expect(await pressing.session.apply(DRAFT)).toEqual({ ok: true });
    // Until the journal holds the approval, the press is known only where it was made.
    expect(logOf(pressing.session)).toContain(APPLYING);
    expect(logOf(other.session)).not.toContain(APPLYING);
    const drawn: string[] = [];
    for (const seq of [13, 14, 15, 16]) {
      pressing.sources[0].entry(at(seq));
      other.sources[0].entry(at(seq));
      const reloaded = await sessionOn(snapshotFrom(upTo(seq), 1));
      const log = logOf(other.session);
      expect(logOf(pressing.session), `after ${seq}`).toBe(log);
      expect(logOf(reloaded.session), `reloaded after ${seq}`).toBe(log);
      drawn.push(log);
    }
    expect(drawn[0]).toContain(APPLYING);
    expect(drawn[1]).toContain('It was this change');
    expect(drawn[3]).toContain('data-state="applied"');
  });
});

describe("section 3.1, Button acts: Apply's working words follow the approval's execution", () => {
  /**
   * Spec 02 section 8, statement 26 (button acts), its first half: "Applying" from the 202 until approval.execution
   * ends.
   */
  it('reads "Applying" from the 202 until the execution ends', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => accepted()));
    const tab = await liveTab(12);
    renderChat(tab.session);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Apply to the site' }));
    });
    expect(screen.getByRole('button', { name: 'Applying' }).getAttribute('aria-disabled')).toBe('true');
    act(() => tab.sources[0].entry(execution(13, 'running')));
    expect(screen.getByRole('button', { name: 'Applying' })).not.toBeNull();
    act(() => tab.sources[0].entry(execution(14, 'failed', 'world-proto-server refused it: the fill pace must be at least 5 s.')));
    expect(screen.queryByRole('button', { name: 'Applying' })).toBeNull();
    expect(screen.getByText('The approved change failed: world-proto-server refused it: the fill pace must be at least 5 s.')).not.toBeNull();
  });

  /** Spec 02 section 3.1, Button acts: a 202 that comes after the execution ended leaves no "Applying". */
  it('changes nothing when the 202 arrives after the execution ended', async () => {
    let answer: (response: Response) => void = () => undefined;
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>((resolve) => {
      answer = resolve;
    })));
    const tab = await liveTab(12);
    renderChat(tab.session);
    act(() => {
      fireEvent.click(screen.getByRole('button', { name: 'Apply to the site' }));
    });
    act(() => {
      tab.sources[0].entry(execution(13, 'running'));
      tab.sources[0].entry(execution(14, 'failed', 'world-proto-server refused it.'));
    });
    await act(async () => answer(accepted()));
    expect(screen.queryByRole('button', { name: 'Applying' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Apply to the site' }).getAttribute('aria-disabled')).toBeNull();
  });

  /**
   * Spec 02 section 3.4, A12: the outcome of the person's own Apply is said once, whichever of its 202 and
   * draft.applied comes first.
   */
  it("says the press's A12 itself when the draft landed before the 202 came back", async () => {
    let answer: (response: Response) => void = () => undefined;
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>((resolve) => {
      answer = resolve;
    })));
    const tab = await liveTab(12);
    const applied = tab.session.apply(DRAFT);
    for (const seq of [13, 14, 15, 16]) tab.sources[0].entry(at(seq));
    answer(accepted());
    expect(await applied).toEqual({ ok: true, said: 'Applied as flow changeset 4.' });
  });
});

describe('section 3.3, step 10: a message settles once, by its client_message_id', () => {
  const TEXT = 'Run the peak hour scenario five times on the new flow.';
  /** The fixture's message of turn 2, carrying the id this tab's POST sent. */
  const echo = (clientMessageId: string): JournalEntry =>
    ({ ...at(17), body: { ...(at(17).body as object), client_message_id: clientMessageId } }) as JournalEntry;
  const sentId = () => (JSON.parse(String((vi.mocked(fetch).mock.calls[0][1] as RequestInit).body)) as { client_message_id: string }).client_message_id;
  const shownTexts = (container: HTMLElement) =>
    [...container.querySelectorAll('li.entry--user .entry__body > p:first-child')].filter((p) => p.textContent === TEXT);

  /** Spec 02 section 3.3, step 10: once the journal holds a message, its POST's answer changes nothing. */
  it('reports nothing when the POST fails after the journal already holds the message', async () => {
    let fail: (fault: Error) => void = () => undefined;
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>((_, reject) => {
      fail = reject;
    })));
    const tab = await liveTab(16);
    const { container } = renderChat(tab.session);
    let sent: Promise<ActOutcome> = Promise.resolve({ ok: false, words: 'not sent yet' });
    act(() => {
      sent = tab.session.send(TEXT, null);
    });
    act(() => tab.sources[0].entry(echo(sentId())));
    await act(async () => {
      fail(new TypeError('Failed to fetch'));
      await sent;
    });
    expect(await sent).toEqual({ ok: true });
    expect(tab.session.status).toBeNull();
    expect(container.querySelector('#composer-status')?.textContent).not.toContain('was not sent');
    expect(shownTexts(container)).toHaveLength(1);
  });

  /** Spec 02 section 3.3, step 10: the journal settles a message whose POST this tab had reported failed. */
  it('withdraws a failure it reported once the journal brings the message after all', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => {
      throw new TypeError('Failed to fetch');
    }));
    const tab = await liveTab(16);
    const { container } = renderChat(tab.session);
    await act(async () => {
      await tab.session.send(TEXT, null);
    });
    expect(container.querySelector('#composer-status')?.textContent).toContain('Your message was not sent');
    act(() => tab.sources[0].entry(echo(sentId())));
    expect(container.querySelector('#composer-status')?.textContent).not.toContain('was not sent');
    expect(screen.queryByRole('button', { name: 'Try again' })).toBeNull();
    expect(shownTexts(container)).toHaveLength(1);
  });
});

describe("section 6, sendMessage: a panel act's message", () => {
  const posted = () => JSON.parse(String((vi.mocked(fetch).mock.calls[0][1] as RequestInit).body)) as { source: string; act?: string };

  /**
   * Spec 02 section 6, sendMessage, and spec 03 section 3.3's rule that a panel act starting a turn returns focus to
   * the composer.
   */
  it('posts as the panel with its act, draws "Sent from the Map panel by Choose template", and moves focus to the composer', async () => {
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>(() => undefined)));
    const tab = await liveTab(16);
    const { container } = renderChat(tab.session);
    const focused: string[] = [];
    act(() => {
      void sendPanelMessage(tab.session, 'Use the L shell.', 'Choose template', CONTEXT, { focusComposer: () => focused.push('composer'), say: () => undefined });
    });
    expect(posted()).toMatchObject({ source: 'panel', act: 'Choose template' });
    expect(container.querySelector('li.entry--user[data-state="sending"] .entry__sent-as')?.textContent).toBe('Sending');
    expect(tab.session.local.optimistic[0].sentAs).toBe('Sent from the Map panel by Choose template');
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ turn: 'x', seq: 17, client_message_id: 'c' }), { status: 202 })));
    await sendPanelMessage(tab.session, 'Use the L shell.', 'Choose template', CONTEXT, { focusComposer: () => focused.push('composer'), say: () => undefined });
    expect(focused).toEqual(['composer']);
  });

  /**
   * Spec 02 section 6, sendMessage, and section 3.5's F4: a refusal is spoken and handed to the panel; the composer is
   * left as it was.
   */
  it('says a refusal and rejects with its words, leaving the composer as it was', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ type: 'not-found', title: 'Nothing is here', status: 404, detail: 'Agent Studio serves no route POST /api/conversations/c/messages.' }), { status: 404 })));
    const tab = await liveTab(16);
    renderChat(tab.session);
    const said: string[] = [];
    const sent = sendPanelMessage(tab.session, 'Use the L shell.', 'Choose template', CONTEXT, { focusComposer: () => said.push('focus'), say: (sentence) => said.push(sentence) });
    await expect(sent).rejects.toThrow('Not sent: Agent Studio serves no route POST /api/conversations/c/messages.');
    expect(said).toEqual(['Not sent: Agent Studio serves no route POST /api/conversations/c/messages.']);
    expect(tab.session.status).toBeNull();
    expect(screen.queryByRole('button', { name: 'Try again' })).toBeNull();
  });
});
