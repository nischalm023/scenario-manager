// Spec 02 section 3.1's Home and section 3.3, step 13: New site goes to the Template list, whose chat opens the
// person's new-site thread; the route opens its conversation once, and nothing else posts it.

import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { conversationOf, snapshotFrom } from '../testing/journal.ts';
import { FakeSource, ME } from '../testing/render.tsx';
import { Studio } from './Studio.tsx';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const answer = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });

describe('spec 02 section 3.3, step 13: the route opens its conversation', () => {
  /** Spec 02 section 8, statement 30 (Home and Dismiss), its Home half: New site opens the new-site thread, once. */
  it('opens the new-site thread once, from the Template list\'s chat, when New site is pressed', async () => {
    const thread = { ...conversationOf([], null), id: '01M5AH0P6Z2CBS21960VQG3RJB' };
    const posted: string[] = [];
    vi.stubGlobal('EventSource', FakeSource);
    vi.stubGlobal('fetch', vi.fn(async (path: string, init?: RequestInit) => {
      if (init?.method === 'POST') posted.push(path);
      if (path === '/api/me') return answer(ME);
      if (path === '/api/sites') return answer({ items: [] });
      if (path === '/api/conversations' && init?.method === 'POST') return answer({ conversation: thread }, 201);
      if (path === `/api/conversations/${thread.id}`) return answer({ ...snapshotFrom([], 1), conversation: thread });
      return answer({ type: 'not-found', title: 'Nothing is here', status: 404, detail: `No route ${path}.` }, 404);
    }));
    await act(async () => {
      render(<MemoryRouter initialEntries={['/']}><Routes><Route path="*" element={<Studio />} /></Routes></MemoryRouter>);
    });
    fireEvent.click(screen.getByRole('button', { name: 'New site' }));
    await waitFor(() => expect(screen.getByRole('heading', { level: 2, name: 'New site' })).not.toBeNull());
    await waitFor(() => expect(document.getElementById('composer-input')).not.toBeNull());
    expect(posted.filter((path) => path === '/api/conversations')).toHaveLength(1);
  });
});
