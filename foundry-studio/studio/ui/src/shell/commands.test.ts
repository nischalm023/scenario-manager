// Spec 03 section 3.3's three checks, in the unit: only a live entry of a turn this browser tab started moves it; a
// person's navigation since the turn began, focus in the panel, or "Offer a link in the chat" holds a `show` back as
// a suggestion with the "suggests" sentence; and highlights are applied and cleared by `data-ref`.

import { afterEach, describe, expect, it } from 'vitest';
import type { JournalEntry } from '../protocol/journal.ts';
import { entriesOf, snapshotFrom } from '../testing/journal.ts';
import { sessionOn, TAB } from '../testing/render.tsx';
import type { ChatSession } from '../chat/session.ts';
import { applyHighlights, commandListener, highlightedKeys, noteUserNavigation } from './commands.ts';
import { parseRoute } from './routes.ts';

const thread = entriesOf('new-site-thread').map((entry) => (entry.kind === 'turn.started'
  ? { ...entry, body: { ...(entry.body as object), browser_tab: TAB } } : entry)) as JournalEntry[];
const show = thread.find((entry) => entry.kind === 'panel.command') as JournalEntry;
const before = thread.filter((entry) => entry.seq < show.seq);

async function tab(options: { offersLink?: boolean; tabId?: string } = {}) {
  const moved: string[] = [];
  const said: string[] = [];
  let session: ChatSession | null = null;
  const listener = commandListener({
    browserTab: () => options.tabId ?? TAB, route: () => parseRoute('/templates'),
    navigate: (to) => moved.push(to), say: (sentence) => said.push(sentence), offersLinkForShow: () => options.offersLink ?? false,
  }, () => session);
  const opened = await sessionOn(snapshotFrom(before.slice(0, 1), 1), listener);
  session = opened.session;
  const source = opened.sources[0];
  source.open();
  for (const entry of before.slice(1)) source.entry(entry);
  return { moved, said, source, session: opened.session };
}

afterEach(() => {
  document.body.innerHTML = '';
});

describe('spec 03 section 3.3: which browser tab acts', () => {
  /** Spec 03 section 8, statement 4 (which browser tab moves): check 1, only a live entry moves the tab. */
  it('moves nothing during catch-up, and moves the initiating tab once live', async () => {
    const replayed = await tab();
    replayed.source.entry(show);
    expect(replayed.moved).toEqual([]);
    const live = await tab();
    live.source.emit('stream.live', { last_seq: show.seq - 1 });
    live.source.entry(show);
    expect(live.moved).toEqual(['/w/customer/demo/site/lshape-6/map@1?highlights=26']);
    expect(live.said).toEqual(['The agent opened the Map panel: Small L-shape, 6 aisles, map at changeset 1. Highlighted 4 entities: where stock totes arrive for picking.']);
  });

  /** Spec 03 section 8, statement 4 (which browser tab moves): check 2, only the initiating tab. */
  it('moves no other browser tab', async () => {
    const other = await tab({ tabId: 'a-different-browser-tab' });
    other.source.emit('stream.live', { last_seq: show.seq - 1 });
    other.source.entry(show);
    expect(other.moved).toEqual([]);
    expect(other.said).toEqual([]);
  });

  /** Spec 03 section 8, statements 4 and 12: check 3 holds a show back as a link. */
  it("holds a show back as a suggestion after the person's own navigation, with focus in the panel, or by the setting", async () => {
    for (const hold of ['navigated', 'focus', 'setting'] as const) {
      const held = await tab({ offersLink: hold === 'setting' });
      if (hold === 'navigated') noteUserNavigation();
      if (hold === 'focus') {
        document.body.innerHTML = '<main class="panel" id="panel"><button>in the panel</button></main>';
        document.querySelector('button')?.focus();
      }
      held.source.emit('stream.live', { last_seq: show.seq - 1 });
      held.source.entry(show);
      expect(held.moved).toEqual([]);
      expect(held.said).toEqual(['The agent suggests the Map panel; the link is in the chat.']);
      expect(held.session.local.heldBack.has(show.seq)).toBe(true);
      document.body.innerHTML = '';
    }
  });
});

describe('spec 03 section 3.3: highlights by data-ref', () => {
  /** Spec 03 section 3.3, applying highlights by data-ref. */
  it("marks every element whose data-ref is a group's key, and clears the mark when the address names none", async () => {
    const { session, source } = await tab();
    source.entry(show);
    document.body.innerHTML = '<main id="panel"><span data-ref="ffe868cd-a382-4dc7-8c27-cc41bf3433a9">Pick stations</span><span data-ref="b6a9a763-687c-4067-909b-460e986e2229">other</span></main>';
    const panel = document.getElementById('panel') as HTMLElement;
    applyHighlights(panel, highlightedKeys(session.stream.model, show.seq));
    expect([...panel.querySelectorAll('[data-highlight="agent"]')].map((element) => element.textContent)).toEqual(['Pick stations']);
    applyHighlights(panel, highlightedKeys(session.stream.model, undefined));
    expect(panel.querySelector('[data-highlight]')).toBeNull();
  });
});
