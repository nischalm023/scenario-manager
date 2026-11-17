// A user entry's page context (`LogView`, ruling D190): the page the agent was sent with a message stands behind an "i"
// under the bubble, never as the bubble's own text, and a message the person sent without it has no "i".

import { cleanup, fireEvent, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { JournalEntry } from '../protocol/journal.ts';
import type { PageContext } from '../protocol/page.ts';
import { entriesOf, snapshotFrom } from '../testing/journal.ts';
import { renderChat, sessionOn } from '../testing/render.tsx';

afterEach(cleanup);

const SENT: PageContext = {
  route: '/w/customer/demo/site/lshape-6/map@main?select=b6a9a763-687c-4067-909b-460e986e2229', panel: 'map',
  title: 'Map: Small L-shape, 6 aisles, following branch main', seen: [{ scope: 'customer/demo/site/lshape-6/map', seq: 3 }],
  selection: { key: 'b6a9a763-687c-4067-909b-460e986e2229', label: 'Pick station 3' }, highlights: null,
};

/** The site conversation as far as its first message, that message sent with `context`. */
function sentWith(context: PageContext | null): JournalEntry[] {
  return entriesOf('site-conversation').filter((entry) => entry.seq <= 2).map((entry) => (entry.kind === 'message.user'
    ? { ...entry, body: { ...(entry.body as object), context } } as JournalEntry : entry));
}

describe("a user entry's page context (ruling D190)", () => {
  it('stands behind an "i" under the bubble with the page, its selection and what it read, not in the bubble', async () => {
    const { session } = await sessionOn(snapshotFrom(sentWith(SENT), 1));
    const { container } = renderChat(session);
    const entry = container.querySelector('li.entry--user');
    const about = screen.getByRole('button', { name: 'About the page sent with this message' });
    expect(entry?.contains(about)).toBe(true);
    // The bubble is `.entry__body`, drawn on its own background; the "i" stands in the line after it.
    const bubble = entry?.querySelector('.entry__body');
    expect(bubble?.contains(about), 'the "i" out of the bubble').toBe(false);
    expect(about.closest('.entry__sent')?.previousElementSibling).toBe(bubble);
    const tip = document.getElementById(about.getAttribute('aria-controls') ?? '');
    expect(tip?.hidden).toBe(true);
    expect(tip?.textContent).toBe('Page Map: Small L-shape, 6 aisles, following branch mainSelected Pick station 3It read map changeset 3');
    expect(entry?.textContent).not.toContain('Sent with');
    fireEvent.click(about);
    expect(tip?.hidden).toBe(false);
  });

  it('has no "i" for a message sent without the page context', async () => {
    const { session } = await sessionOn(snapshotFrom(sentWith(null), 1));
    const { container } = renderChat(session);
    expect(screen.queryByRole('button', { name: 'About the page sent with this message' })).toBeNull();
    expect(container.querySelector('li.entry--user .entry__sent')).toBeNull();
  });
});
