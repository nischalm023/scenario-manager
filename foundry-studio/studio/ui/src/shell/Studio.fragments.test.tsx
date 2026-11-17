// An address's fragment through the shell (`Studio`), on two small served documents: a link the person follows to a
// heading of another document, or of the open one by its file's name, and an address the shell rewrites to its
// canonical form, each reach the address with its fragment, and the heading it names takes keyboard focus (`DocPage`).

import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useEffect } from 'react';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { conversationOf, snapshotFrom } from '../testing/journal.ts';
import { FakeSource, ME } from '../testing/render.tsx';
import { Studio } from './Studio.tsx';

vi.mock('../docs/manifest.ts', () => ({
  PAGES: new Map([
    ['docs/a.md', '# A\n\n[Later in this document](a.md#later), [B\'s heading](b.md#there)\n\n## Early\n\nText.\n\n## Later\n\nText.'],
    ['docs/b.md', '# B\n\n## Before\n\nText.\n\n## There\n\nText.'],
  ]),
  FIGURES: new Map(),
}));

const THREAD = { ...conversationOf([], null), id: '01M5AH0P6Z2CBS21960VQG3RJB' };
const answer = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });

/** The address the router shows, its fragment included. */
let shown = '';
function Address() {
  const { pathname, search, hash } = useLocation();
  useEffect(() => {
    shown = `${pathname}${search}${hash}`;
  }, [pathname, search, hash]);
  return null;
}

// jsdom lays nothing out and has no `scrollIntoView`; where the heading scrolls to is the browser test's to check.
beforeEach(() => {
  Element.prototype.scrollIntoView = () => undefined;
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  Reflect.deleteProperty(Element.prototype, 'scrollIntoView');
});

async function studioOn(address: string) {
  vi.stubGlobal('EventSource', FakeSource);
  vi.stubGlobal('fetch', vi.fn(async (path: string, init?: RequestInit) => {
    if (path === '/api/me') return answer(ME);
    if (path === '/api/conversations' && init?.method === 'POST') return answer({ conversation: THREAD }, 201);
    if (path === `/api/conversations/${THREAD.id}`) return answer({ ...snapshotFrom([], 1), conversation: THREAD });
    return answer({ type: 'not-found', title: 'Nothing is here', status: 404, detail: `No route ${path}.` }, 404);
  }));
  await act(async () => {
    render(<MemoryRouter initialEntries={[address]}><Routes><Route path="*" element={<><Studio /><Address /></>} /></Routes></MemoryRouter>);
  });
}

const focused = () => document.activeElement?.id;

describe('the shell keeps an address\'s fragment, and the heading it names takes focus', () => {
  it('following a link to a heading of another document opens that document at the heading', async () => {
    await studioOn('/docs/docs/a');
    fireEvent.click(await screen.findByRole('link', { name: 'B\'s heading' }));
    await waitFor(() => expect(shown).toBe('/docs/docs/b#there'));
    await waitFor(() => expect(focused()).toBe('there'));
  });

  it('following a link to a heading of the open document by its file\'s name goes to that heading', async () => {
    await studioOn('/docs/docs/a');
    fireEvent.click(await screen.findByRole('link', { name: 'Later in this document' }));
    await waitFor(() => expect(shown).toBe('/docs/docs/a#later'));
    await waitFor(() => expect(focused()).toBe('later'));
  });

  it('rewriting an address to its canonical form keeps its fragment', async () => {
    await studioOn('/docs/docs/a?stray=1#later');
    await waitFor(() => expect(shown).toBe('/docs/docs/a#later'));
    await waitFor(() => expect(focused()).toBe('later'));
  });
});
