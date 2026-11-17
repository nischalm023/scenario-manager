// Spec 02 section 8, statements 14 (markup), 17 (input methods), 18 (storage refused), 22 (one user entry),
// 25 (projection), 27 (tool calls, its DOM half) and 33 (the simplified chat, D117): the chat column, the tab row and
// Settings drawn from the journal fixtures the API's test writer replays, in jsdom.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AppBar } from '../shell/AppBar.tsx';
import { PanelTabs } from '../shell/PanelTabs.tsx';
import { SettingsPanel } from '../shell/SettingsPanel.tsx';
import { Studio } from '../shell/Studio.tsx';
import { reloadPreferences } from '../shell/preferences.ts';
import { parseRoute } from '../shell/routes.ts';
import { entriesOf, snapshotFrom } from '../testing/journal.ts';
import { CONTEXT, renderChat, sessionOn, shellOf, SITE_NAME } from '../testing/render.tsx';
import { ShellContext } from '../shell/context.ts';
import { AgentText } from './AgentText.tsx';
import { spokenSentences } from './sentences.ts';
import type { JournalEntry } from '../protocol/journal.ts';
import type { ActOutcome } from './chatContext.ts';
import { TURN_RUNNING } from './words.ts';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const upTo = (entries: JournalEntry[], seq: number) => entries.filter((entry) => entry.seq <= seq);

describe('statement 14: each component renders section 3.1\'s skeleton', () => {
  /**
   * Spec 02 section 8, statement 14 (markup): the chat column, a user entry, a turn, agent text, a changeset card and
   * the composer as section 3.1's tree draws them.
   */
  it('draws the chat column, a user entry, a turn with its closed tool calls, agent text, a changeset card and the composer', async () => {
    const { session } = await sessionOn(snapshotFrom(entriesOf('site-conversation'), 1));
    const { container } = renderChat(session);
    const chat = container.querySelector('aside.chat#chat[aria-label="Agent chat"]');
    expect(chat).not.toBeNull();
    expect(chat?.querySelector(':scope > div.chat__header > h2.chat__title#chat-title')?.textContent).toBe(SITE_NAME);
    expect(chat?.querySelector('p.chat__meta > button.btn.btn--small.btn--quiet.chat__dismiss[aria-describedby="chat-dismiss-desc"]')).not.toBeNull();
    expect(chat?.querySelector('p.chat__meta > span.visually-hidden#chat-dismiss-desc')).not.toBeNull();
    const log = chat?.querySelector('ol.chat__log#chat-log[aria-label="Conversation"][tabindex="0"]');
    expect(log?.hasAttribute('aria-live')).toBe(false);
    expect(log?.getAttribute('role')).toBeNull();
    expect(log?.querySelector(':scope > li.entry.entry--user > h3.entry__speaker + div.entry__body > p')).not.toBeNull();
    expect(log?.querySelector(':scope > li.entry.entry--user > h3.entry__speaker > span.visually-hidden')?.textContent).toBe(', for turn 1');
    const turn = log?.querySelector(':scope > li.turn[data-state="done"][aria-labelledby="turn-1"]');
    expect(turn?.querySelector(':scope > h3.turn__header#turn-1[tabindex="-1"] > strong')?.textContent).toBe('Agent');
    const tools = turn?.querySelector(':scope > ol.turn__items > li:first-child > details.tool-calls[data-count="2"]');
    expect(tools?.querySelector(':scope > summary.tool-calls__summary')?.textContent).toBe('2 tool calls');
    expect(tools?.querySelector(':scope > ul.tool-calls__list > li.tool-row[data-state="done"][data-tool="flow.read"] > span.tool-row__glyph[aria-hidden="true"] + span.tool-row__summary + span.tool-row__state')).not.toBeNull();
    expect(tools?.querySelector(':scope > details.turn__details > summary')?.textContent).toBe('Inputs and results of 2 tool calls');
    expect(turn?.querySelector('li.entry.entry--agent[data-kind="text"] > div.entry__body:not([aria-busy])')).not.toBeNull();
    const card = turn?.querySelector('ol.turn__items > li:last-child > div.chat-card.changeset-card[data-state="applied"]');
    expect(card?.querySelector(':scope > h4.chat-card__title[tabindex="-1"]')?.textContent).toBe('Draft changeset applied');
    expect(card?.querySelector(':scope > h4.chat-card__title > span.pill[data-tone="good"]')?.textContent).toBe('applied');
    expect(card?.querySelector('p.changeset-card__base')?.textContent).toBe('Built against flow changeset 3 of customer/demo/site/lshape-6, branch main.');
    expect(card?.querySelector('ol.changeset-card__ops[aria-label] > li.op[data-op="patch"][data-ref] > span.op__kind')?.textContent).toBe('Changed');
    expect(card?.querySelector('span.op__change > span.visually-hidden + del')?.textContent).toBe('20 s');
    expect(log?.querySelector(':scope > li.entry.entry--system[data-kind="draft.applied"] > h3.entry__speaker')?.textContent).toBe('Site changes');
    const composer = chat?.querySelector('form.composer#composer[aria-label="Message the agent"]');
    const chip = composer?.querySelector(':scope > p.composer__context#composer-context > span.pill.pill--wrap.composer__chip');
    expect(chip?.querySelector(':scope > span#composer-context-words + button.composer__chip-remove[aria-label="Leave the page context out"]')).not.toBeNull();
    expect(composer?.querySelector(':scope > label.visually-hidden[for="composer-input"]')).not.toBeNull();
    const input = composer?.querySelector('textarea.composer__input#composer-input[placeholder="Message"]');
    expect(input?.getAttribute('aria-describedby')).toBe('composer-context-words composer-status');
    expect(composer?.querySelector(':scope > p.composer__status#composer-status')).not.toBeNull();
    expect(composer?.querySelector(':scope > div.composer__row > button.btn.btn--primary[type="submit"]')?.textContent).toBe('Send');
  });

  /**
   * Brief chore-chat-fixes, the first defect (D117: the chat says each thing once, with no meta-narration): the
   * fixture's `note` block, the model's reasoning as journals before the fix held it, draws no entry and no words.
   */
  it("draws no entry for the model's reasoning", async () => {
    const { session } = await sessionOn(snapshotFrom(entriesOf('site-conversation'), 1));
    const { container } = renderChat(session);
    const turn = container.querySelector('#chat-log > li.turn[aria-labelledby="turn-1"]');
    expect(turn?.querySelector('li.entry.entry--agent[data-kind="text"]')).not.toBeNull();
    expect(container.querySelector('#chat-log li.entry[data-kind="note"], #chat-log li.entry--note')).toBeNull();
    expect(container.querySelector('#chat-log')?.textContent).not.toContain('Reading the flow at changeset 3');
  });

  /** Spec 02 section 3.1, TableScroll, and section 5's reflow: a reply's table stands in a named, focusable region. */
  it("draws a reply's table inside a TableScroll named by its header cells", () => {
    const text = '| Station | Fill pace |\n| --- | --- |\n| Pick station 3 | 15 s |';
    const { container } = render(
      <ShellContext.Provider value={shellOf()}><ol><AgentText text={text} streaming={false} refs={[]} /></ol></ShellContext.Provider>,
    );
    const region = container.querySelector('div.table-scroll[role="region"][tabindex="0"][aria-label="Station, Fill pace, table"]');
    expect(region?.querySelector(':scope > table > thead > tr > th')?.textContent).toBe('Station');
  });

  /**
   * Brief chore-chat-fixes, the second defect (D60: references name entities, never scope paths): a template or a site
   * the reply names, by the bare path a tool gave or as a `scope:` reference, is a link to its panel route and no
   * chip, spoken as its words; an entity reference that does not resolve still reads "(not found)".
   */
  it("draws a site's or a template's location as a link to its panel route, never a chip", () => {
    const text = 'Start from [Square](ref:template/site/square) or [L-Shape](ref:scope:template/site/l-shape@main); ' +
      '[the demo site](ref:scope:customer/demo/site/lshape-6@main) has [Pick station 3](ref:b6a9a763-687c-4067-909b-460e986e2229).';
    const { container } = render(
      <ShellContext.Provider value={shellOf()}><ol><AgentText text={text} streaming={false} refs={[]} /></ol></ShellContext.Provider>,
    );
    const links = [...container.querySelectorAll('a:not(.entity-chip)')].map((link) => [link.getAttribute('href'), link.textContent]);
    expect(links).toEqual([
      ['/w/template/site/square@main', 'Square'],
      ['/w/template/site/l-shape@main', 'L-Shape'],
      ['/w/customer/demo/site/lshape-6/map@main', 'the demo site'],
    ]);
    const chips = [...container.querySelectorAll('.entity-chip')].map((chip) => [chip.getAttribute('data-ref'), chip.getAttribute('data-state')]);
    expect(chips).toEqual([['b6a9a763-687c-4067-909b-460e986e2229', 'missing']]);
    expect(spokenSentences(text, [])).toEqual(['Start from Square or L-Shape; the demo site has Pick station 3 (not found).']);
  });

  /** Spec 02 section 8, statement 14 (markup): an open question card, a form landmark named by its first question. */
  it('draws an open question card as a form landmark named by its first question, no option checked', async () => {
    const { session } = await sessionOn(snapshotFrom(upTo(entriesOf('new-site-thread'), 9), 1));
    const { container } = renderChat(session);
    const form = container.querySelector('form.chat-card.question-card[data-state="open"][aria-labelledby="question-01M587G5TGB5PY5PCZS6P8RWW0-0"]');
    const texts = form?.querySelectorAll('fieldset.question-card__item > legend > span.question-card__text') ?? [];
    expect([...texts].map((text) => text.id)).toEqual(['question-01M587G5TGB5PY5PCZS6P8RWW0-0', 'question-01M587G5TGB5PY5PCZS6P8RWW0-1']);
    const options = form?.querySelectorAll('div.option > input[type="radio"][aria-describedby]') ?? [];
    expect(options.length).toBe(4);
    expect([...options].some((option) => (option as HTMLInputElement).checked)).toBe(false);
    expect(form?.querySelector('div.option > span > label.option__label + span.option__description')).not.toBeNull();
    expect(form?.querySelector('div.chat-card__actions > button.btn.btn--primary[type="submit"]')?.textContent).toBe('Send');
  });

  /**
   * Spec 02 section 8, statement 14 (markup): a pending approval card, a region whose input comes before its buttons.
   */
  it('draws a pending approval card as a region with its input before its buttons, and the composer\'s Send as "Send"', async () => {
    const { session } = await sessionOn(snapshotFrom(upTo(entriesOf('ended-turns'), 13), 1));
    const { container } = renderChat(session);
    const card = container.querySelector('section.chat-card.approval-card[data-state="pending"][aria-labelledby="approval-01M588MFY0CS240YCSV4E4BFBK-title"]');
    expect(card).not.toBeNull();
    expect(card?.querySelector('dl.approval-card__input > dt')?.textContent).toBe('Change');
    expect(card?.querySelector('details.approval-card__exact > summary')?.textContent).toBe('The exact input');
    const order = [...(card?.querySelectorAll('summary, input, button') ?? [])].map((element) => element.textContent || (element as HTMLInputElement).id);
    expect(order).toEqual(['The exact input', 'approval-01M588MFY0CS240YCSV4E4BFBK-reason', 'Apply', 'Deny']);
    expect(container.querySelector('#composer button[type="submit"]')?.textContent).toBe('Send');
    expect(container.querySelector('#composer-status')?.textContent).toBe('');
    expect(container.querySelector('li.tool-row[data-state="waiting"] > span.tool-row__state')?.textContent).toBe('waiting for you');
  });
});

describe('statement 27, its DOM half: one closed ToolCalls per turn that says its count alone', () => {
  /**
   * Spec 02 section 8, statement 27 (tool calls), its DOM half: one closed disclosure per turn whose summary says the
   * count and nothing else (D117); a failure shows on its row.
   */
  it('holds one closed disclosure per turn with calls, its summary the count alone even when a call failed', async () => {
    const { session } = await sessionOn(snapshotFrom(entriesOf('ended-turns'), 1));
    const { container } = renderChat(session);
    const turns = [...container.querySelectorAll('li.turn')];
    for (const turn of turns) {
      const tools = turn.querySelectorAll('details.tool-calls');
      expect(tools.length).toBeLessThanOrEqual(1);
      tools.forEach((details) => expect(details.hasAttribute('open')).toBe(false));
    }
    const failed = container.querySelector('li.turn[aria-labelledby="turn-4"] summary.tool-calls__summary');
    expect(failed?.textContent).toBe('1 tool call');
    expect(container.querySelector('li.turn[aria-labelledby="turn-4"] li.tool-row[data-state="failed"] > span.tool-row__state')?.textContent).toBe('failed');
    const denied = container.querySelector('li.turn[aria-labelledby="turn-3"] li.tool-row[data-state="denied"] > span.tool-row__state');
    expect(denied?.textContent).toBe('denied: nobody was attached to approve');
  });

  /** Spec 02 section 8, statement 27 (tool calls), its DOM half: a new turn's calls arrive closed. */
  it('draws the next turn\'s tool calls closed as they arrive live', async () => {
    const entries = entriesOf('site-conversation');
    const { session, sources } = await sessionOn(snapshotFrom(upTo(entries, 16), 1));
    const { container } = renderChat(session);
    (container.querySelector('details.tool-calls') as HTMLDetailsElement).open = true;
    act(() => {
      sources[0].open();
      sources[0].emit('stream.live', { last_seq: 16 });
      for (const entry of entries.filter((one) => one.seq > 16 && one.seq <= 19)) sources[0].entry(entry);
    });
    const second = container.querySelector('li.turn[aria-labelledby="turn-2"] details.tool-calls');
    expect(second?.hasAttribute('open')).toBe(false);
    expect(second?.querySelector('summary')?.textContent).toBe('1 tool call');
  });
});

describe('statement 25: a snapshot that begins inside a turn draws it as a full replay does', () => {
  /**
   * Spec 02 section 8, statement 25 (projection), with this page's own reducer making the window's projection; the
   * same against the API's projection is tests/browser/log.spec.ts's.
   */
  it("draws turn 2's heading, rows and cards from the projection alone", async () => {
    const entries = entriesOf('site-conversation');
    const full = await sessionOn(snapshotFrom(entries, 1));
    const window = await sessionOn(snapshotFrom(entries, 25));
    const drawn = (session: typeof full.session) => {
      const { container, unmount } = renderChat(session);
      const turn = container.querySelector('li.turn[aria-labelledby="turn-2"]');
      const parts = {
        header: turn?.querySelector('h3.turn__header')?.outerHTML,
        rows: turn?.querySelector('ul.tool-calls__list')?.outerHTML,
        cards: [...(turn?.querySelectorAll('.chat-card') ?? [])].map((card) => card.outerHTML),
      };
      unmount();
      return parts;
    };
    const replayedTurn = drawn(full.session);
    expect(replayedTurn.cards.length).toBe(4);
    expect(drawn(window.session)).toEqual(replayedTurn);
  });
});

describe('statement 22: one user entry', () => {
  /** Spec 02 section 8, statement 22 (one user entry). */
  it('settles the optimistic entry on a message.user that arrives before the 202, which then changes nothing', async () => {
    let answer: (response: Response) => void = () => undefined;
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>((resolve) => {
      answer = resolve;
    })));
    const entries = entriesOf('site-conversation');
    const { session, sources } = await sessionOn(snapshotFrom(upTo(entries, 16), 1));
    const { container } = renderChat(session);
    act(() => {
      sources[0].open();
      sources[0].emit('stream.live', { last_seq: 16 });
    });
    let sent: Promise<ActOutcome> = Promise.resolve({ ok: false, words: 'not sent yet' });
    act(() => {
      sent = session.send('Run the peak hour scenario five times on the new flow.', null);
    });
    expect(container.querySelectorAll('li.entry--user[data-state="sending"]')).toHaveLength(1);
    const posted = JSON.parse(String((vi.mocked(fetch).mock.calls[0][1] as RequestInit).body)) as { client_message_id: string };
    const message = entries.find((entry) => entry.seq === 17) as JournalEntry;
    act(() => sources[0].entry({ ...message, body: { ...(message.body as object), client_message_id: posted.client_message_id } }));
    const texts = () => [...container.querySelectorAll('li.entry--user .entry__body > p:first-child')].map((p) => p.textContent);
    expect(texts().filter((text) => text?.startsWith('Run the peak hour'))).toHaveLength(1);
    expect(container.querySelector('li.entry--user[data-state="sending"]')).toBeNull();
    await act(async () => {
      answer(new Response(JSON.stringify({ turn: 'x', seq: 17, client_message_id: posted.client_message_id }), { status: 202 }));
      await sent;
    });
    expect(texts().filter((text) => text?.startsWith('Run the peak hour'))).toHaveLength(1);
  });
});

describe('statement 17: input methods', () => {
  /** Spec 02 section 8, statement 17 (input methods). */
  it('does not send on Enter while an input method is composing, and sends on Enter after it', async () => {
    const posts = vi.fn(async () => new Response('{}', { status: 202 }));
    vi.stubGlobal('fetch', posts);
    const { session } = await sessionOn(snapshotFrom(entriesOf('site-conversation'), 1));
    renderChat(session);
    const input = screen.getByLabelText('Message the agent', { selector: 'textarea' });
    fireEvent.change(input, { target: { value: 'ステーション' } });
    fireEvent.compositionStart(input);
    fireEvent.keyDown(input, { key: 'Enter', isComposing: true });
    expect(posts).not.toHaveBeenCalled();
    expect((input as HTMLTextAreaElement).value).toBe('ステーション');
    fireEvent.compositionEnd(input);
    await act(async () => {
      fireEvent.keyDown(input, { key: 'Enter' });
    });
    expect(posts).toHaveBeenCalledTimes(1);
  });
});

describe('statement 18: browser storage refused', () => {
  beforeEach(() => {
    const refuse = () => {
      throw new DOMException('The operation is insecure.', 'SecurityError');
    };
    vi.spyOn(window, 'localStorage', 'get').mockImplementation(refuse);
    vi.spyOn(window, 'sessionStorage', 'get').mockImplementation(refuse);
    reloadPreferences();
  });

  /** Spec 02 section 8, statement 18 (storage refused), with section 3.8's "It is still in the box." */
  it('keeps a failed message\'s text in the box, though storage keeps nothing', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ type: 'internal', title: 'Internal', status: 500, detail: 'The service failed.' }), { status: 500 })));
    const { session } = await sessionOn(snapshotFrom(entriesOf('site-conversation'), 1));
    renderChat(session);
    const input = screen.getByLabelText('Message the agent', { selector: 'textarea' }) as HTMLTextAreaElement;
    fireEvent.change(input, { target: { value: 'Run the peak hour scenario.' } });
    await act(async () => {
      fireEvent.keyDown(input, { key: 'Enter' });
    });
    expect(screen.getByText('Your message was not sent: The service failed. It is still in the box.')).not.toBeNull();
    expect(input.value).toBe('Run the peak hour scenario.');
  });

  /** Spec 02 section 8, statement 18 (storage refused): the page renders, the defaults apply, nothing throws. */
  it('renders the page with the defaults, and nothing throws', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ type: 'not-found', title: 'Not found', status: 404, detail: 'nothing' }), { status: 404 })));
    await act(async () => {
      render(
        <MemoryRouter initialEntries={['/']}>
          <Routes><Route path="*" element={<Studio />} /></Routes>
        </MemoryRouter>,
      );
    });
    expect(screen.getByRole('heading', { level: 2, name: 'Sites' })).not.toBeNull();
    cleanup();
    // The choices live on the Settings page of their own, so the defaults are read there.
    await act(async () => {
      render(
        <MemoryRouter initialEntries={['/settings']}>
          <Routes><Route path="*" element={<Studio />} /></Routes>
        </MemoryRouter>,
      );
    });
    expect((screen.getByLabelText('Up to three sentences, else the first') as HTMLInputElement).checked).toBe(true);
    expect((screen.getByLabelText('Follow the system') as HTMLInputElement).checked).toBe(true);
    expect(screen.getByRole('switch', { name: 'Switch to it' }).getAttribute('aria-checked')).toBe('true');
    fireEvent.click(screen.getByLabelText('Dark'));
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
  });
});

describe('statement 33: the simplified chat (D117)', () => {
  const site = entriesOf('site-conversation');
  const thread = entriesOf('new-site-thread');
  const buttons = (root: Element | null | undefined) => [...(root?.querySelectorAll('button') ?? [])].map((button) => button.textContent);

  /**
   * Spec 02 section 8, statement 33 (the simplified chat): the header holds the title and Dismiss; a turn heading
   * reads "Agent" alone and a running turn ends in "Working"; a ToolCalls summary says its count alone; no user entry
   * reads "Sent without the page context"; Stop shows only while the turn runs, and the status only a refused send.
   */
  it('draws the header, a running turn and the composer with nothing D117 cut', async () => {
    const { session, sources } = await sessionOn(snapshotFrom(upTo(site, 6), 1));
    const spoken: string[] = [];
    const { container } = renderChat(session, shellOf(spoken));
    const header = container.querySelector('div.chat__header');
    expect(header?.querySelector('h2#chat-title')?.textContent).toBe(SITE_NAME);
    expect([...(header?.querySelectorAll('button, a') ?? [])].map((control) => control.textContent)).toEqual(['Dismiss']);
    expect(header?.textContent).not.toMatch(/Model|Claude|Answer|Approve/);
    const turn = container.querySelector('li.turn[data-state="running"]');
    expect(turn?.querySelector('h3.turn__header')?.textContent).toBe('Agent');
    expect(turn?.querySelector('ol.turn__items > li:last-child')?.outerHTML).toBe('<li class="turn__working">Working</li>');
    expect(turn?.querySelector('summary.tool-calls__summary')?.textContent).toBe('1 tool call');
    expect(container.querySelector('li.entry--user .entry__sent')).toBeNull();
    expect(container.textContent).not.toContain('Sent without the page context');
    const composer = container.querySelector('form#composer');
    expect(composer?.querySelector('textarea')?.getAttribute('placeholder')).toBe('Message');
    expect(buttons(composer)).toEqual(['×', 'Stop', 'Send']);
    expect(composer?.querySelector('#composer-status')?.textContent).toBe('');
    // Send while the agent works shows and speaks the turn-running words, which go when the turn stops working.
    fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    expect(composer?.querySelector('#composer-status')?.textContent).toBe(TURN_RUNNING);
    expect(spoken).toEqual([TURN_RUNNING]);
    act(() => {
      sources[0].open();
      sources[0].emit('stream.live', { last_seq: 6 });
      for (const entry of site.filter((one) => one.seq > 6 && one.seq <= 12)) sources[0].entry(entry);
    });
    expect(container.querySelector('li.turn__working')).toBeNull();
    expect(buttons(composer)).toEqual(['×', 'Send']);
    expect(composer?.querySelector('#composer-status')?.textContent).toBe('');
  });

  /** Spec 02 section 8, statement 33 (the simplified chat): an open question card is its questions and Send. */
  it('draws an open question card with no title, topic pill, text field or footnote, and the waiting composer silent', async () => {
    const { session } = await sessionOn(snapshotFrom(upTo(thread, 9), 1));
    const { container } = renderChat(session);
    const form = container.querySelector('form.question-card');
    expect(form?.querySelector('h4, .pill, input[type="text"], textarea, .chat-card__footnote')).toBeNull();
    expect([...(form?.querySelectorAll('legend') ?? [])].map((legend) => legend.textContent))
      .toEqual(['Which shell should the new site use?', 'How many aisles should the storage hold?']);
    expect([...(form?.querySelectorAll('label.option__label') ?? [])].map((label) => label.textContent)).toEqual(['L (recommended)', 'U', '6', '8']);
    expect(buttons(form)).toEqual(['Send']);
    expect(form?.textContent).not.toMatch(/The agent asks you|Other|message box/);
    expect(container.querySelector('li.turn__working')).toBeNull();
    expect(buttons(container.querySelector('form#composer'))).toEqual(['×', 'Send']);
    expect(container.querySelector('#composer-status')?.textContent).toBe('');
    expect(container.querySelector('li.tool-row[data-tool="ask_user"] > span.tool-row__state')?.textContent).toBe('waiting for you');
  });

  /** Spec 02 section 8, statement 33 (the simplified chat): Send with an item unanswered says so under it. */
  it('marks an unanswered item "Choose an answer." and counts what still needs one', async () => {
    const { session } = await sessionOn(snapshotFrom(upTo(thread, 9), 1));
    const spoken: string[] = [];
    const { container } = renderChat(session, shellOf(spoken));
    fireEvent.click(screen.getByLabelText('L (recommended)'));
    fireEvent.submit(container.querySelector('form.question-card') as HTMLFormElement);
    expect([...container.querySelectorAll('p.question-card__error')].map((error) => error.textContent)).toEqual(['Choose an answer.']);
    expect(document.activeElement?.id).toBe('question-01M587G5TGB5PY5PCZS6P8RWW0-1-0');
    expect(spoken).toEqual(['One question still needs an answer.']);
  });

  /**
   * Spec 02 section 8, statement 33 (the simplified chat): the page context is one chip, absent without one; its × leaves
   * the context out of the next message, puts focus in the input, and the chip comes back after that send.
   */
  it('shows the page context as one chip whose × leaves it out of the next message only', async () => {
    const posted: { context: unknown }[] = [];
    vi.stubGlobal('fetch', vi.fn(async (_path: string, init?: RequestInit) => {
      posted.push(JSON.parse(String(init?.body)) as { context: unknown });
      return new Response(JSON.stringify({ turn: 'x', seq: 40, client_message_id: 'c' }), { status: 202 });
    }));
    const { session } = await sessionOn(snapshotFrom(site, 1));
    const selected = { ...CONTEXT, selection: { key: 'b6a9a763-687c-4067-909b-460e986e2229', label: 'Pick station 3' } };
    const { container } = renderChat(session, shellOf(), selected);
    expect(container.querySelectorAll('#composer-context .composer__chip')).toHaveLength(1);
    expect(container.querySelector('#composer-context-words')?.textContent)
      .toBe('Map: customer/demo/site/lshape-6, following branch main, Pick station 3 selected');
    const input = screen.getByLabelText('Message the agent', { selector: 'textarea' });
    fireEvent.click(screen.getByRole('button', { name: 'Leave the page context out' }));
    expect(container.querySelector('#composer-context')).toBeNull();
    expect(document.activeElement).toBe(input);
    expect(input.getAttribute('aria-describedby')).toBe('composer-status');
    fireEvent.change(input, { target: { value: 'Which aisle is the slowest?' } });
    await act(async () => {
      fireEvent.keyDown(input, { key: 'Enter' });
    });
    expect(posted.map((body) => body.context)).toEqual([null]);
    expect(container.querySelector('#composer-context-words')?.textContent).toContain('Pick station 3 selected');
    cleanup();
    const bare = renderChat((await sessionOn(snapshotFrom(site, 1))).session, shellOf(), null);
    expect(bare.container.querySelector('#composer-context')).toBeNull();
    expect(bare.container.querySelector('#composer-input')?.getAttribute('aria-describedby')).toBe('composer-status');
  });

  /** Spec 02 section 8, statement 33 (the simplified chat): a disabled tab's reason is its tooltip alone. */
  it("says a disabled tab's reason only in its tooltip, shown on keyboard focus and hidden by Escape", () => {
    render(
      <ShellContext.Provider value={shellOf([], '/')}>
        <PanelTabs route={parseRoute('/')} current={null} site={null} agentOpened={null} />
      </ShellContext.Provider>,
    );
    expect([...document.querySelectorAll('#panel-tabs a .panel-tabs__label')].map((tab) => tab.textContent))
      .toEqual(['Maps', 'Inventory & orders', 'Agents', 'Process flow', 'Stations', 'Scenarios', 'Sim runs', 'Scripting']);
    expect(document.getElementById('panel-tabs')?.textContent).not.toMatch(/\(open a site first\)|\(after the PoC\)/i);
    for (const [label, why] of [['Process flow', 'Open a site first'], ['Scripting', 'After the PoC']]) {
      const tab = screen.getByRole('link', { name: new RegExp(`^${label}`) });
      expect(tab.getAttribute('aria-disabled')).toBe('true');
      expect(tab.getAttribute('tabindex')).toBe('0');
      expect(tab.hasAttribute('href')).toBe(false);
      const tooltip = document.getElementById(tab.getAttribute('aria-describedby') ?? '');
      expect(tooltip?.getAttribute('role')).toBe('tooltip');
      expect(tooltip?.textContent).toBe(why);
      expect(tooltip?.hidden).toBe(true);
      act(() => tab.focus());
      expect(tooltip?.hidden).toBe(false);
      fireEvent.keyDown(document, { key: 'Escape' });
      expect(tooltip?.hidden).toBe(true);
    }
  });

  /** Spec 02 section 8, statement 33 (the simplified chat): the Settings page names the model and the key rule. */
  it('names the open conversation\'s model and the key rule on the Settings page, and no model where none is open', () => {
    const { unmount } = render(
      <ShellContext.Provider value={shellOf()}><SettingsPanel modelLabel="Claude Sonnet 5.5" /></ShellContext.Provider>,
    );
    expect(document.querySelector('#settings .settings__model')?.textContent).toBe('Claude Sonnet 5.5');
    expect(document.querySelector('#settings .settings__help')?.textContent).toBe('Enter sends; Shift and Enter starts a new line.');
    unmount();
    render(<ShellContext.Provider value={shellOf()}><SettingsPanel modelLabel={null} /></ShellContext.Provider>);
    expect(document.querySelector('#settings .settings__model')).toBeNull();
    expect(document.querySelector('#settings .settings__help')).not.toBeNull();
  });

  /** The gear in the sidebar is a link to that page, not a popover. */
  it('links the sidebar gear to /settings and keeps no Settings popover in the app bar', () => {
    const { container } = render(
      <ShellContext.Provider value={shellOf()}><AppBar pagePhrase="the Map panel" /></ShellContext.Provider>,
    );
    expect(container.querySelector('a.app-bar__settings')?.getAttribute('href')).toBe('/settings');
    expect(container.querySelector('[popover]')).toBeNull();
  });
});

describe('lean 6: a message that denies the waiting approval', () => {
  /**
   * Spec 02 lean 6, as the lead settled it: a message typed while an approval waits denies it with no sentence before
   * the send, and the approval card says so after, "denied by your message", without repeating the message.
   */
  it('reads "denied by your message" on the card, the message its reason only in its own entry', async () => {
    const ended = entriesOf('ended-turns').filter((entry) => entry.seq <= 13);
    const turn = '01M588KYBGZ6T51P6SJ2H2AA1B';
    const approval = '01M588MFY0CS240YCSV4E4BFBK';
    const author = { subject: 'dev', name: 'Developer' };
    const later = (seq: number, kind: string, body: object, causedBy: string | null = null): JournalEntry =>
      ({ ...ended[0], seq, kind, turn, caused_by: causedBy, body }) as JournalEntry;
    const entries = [
      ...ended,
      later(14, 'message.user', { author, text: 'Not before Monday.', context: null, source: 'composer', browser_tab: null }),
      later(15, 'approval.decided', { approval, decision: 'deny', author, reason: 'Not before Monday.' }, approval),
      later(16, 'turn.resumed', { by: 'message' }, approval),
    ];
    const { session } = await sessionOn(snapshotFrom(entries, 1));
    const { container } = renderChat(session);
    const card = container.querySelector(`#approval-${approval}`);
    expect(card?.getAttribute('data-state')).toBe('denied');
    expect(card?.querySelector('h4 > span.pill[data-tone="bad"]')?.textContent).toBe('denied by your message');
    expect(card?.textContent).not.toContain('Not before Monday.');
    const entry = [...container.querySelectorAll('li.entry--user')].find((one) => one.textContent?.includes('Not before Monday.'));
    expect(entry?.querySelector('.entry__sent-as')?.textContent).toBe('Sent as the reason for denying: apply 1 change to the flow');
  });
});
