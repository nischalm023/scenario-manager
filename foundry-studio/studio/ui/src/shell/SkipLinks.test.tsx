// Spec 02 section 3.1, SkipLinks: "Skip to the message box" is offered only while the chat draws the message box it
// names, never while the conversation loads nor once it has ended (design/reviews-impl/astra-07c.md, the SkipLinks
// finding), so the link always reaches its target.

import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Snapshot } from '../protocol/snapshot.ts';
import { conversationOf, snapshotFrom } from '../testing/journal.ts';
import { FakeSource, ME } from '../testing/render.tsx';
import { Studio } from './Studio.tsx';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const THREAD = { ...conversationOf([], null), id: '01M5AH0P6Z2CBS21960VQG3RJB' };
const answer = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });

/** The Studio on the Template list, whose chat opens the person's new-site thread, its snapshot as `snapshot` answers. */
async function studioWith(snapshot: () => Promise<Response>) {
  vi.stubGlobal('EventSource', FakeSource);
  vi.stubGlobal('fetch', vi.fn(async (path: string, init?: RequestInit) => {
    if (path === '/api/me') return answer(ME);
    if (path === '/api/conversations' && init?.method === 'POST') return answer({ conversation: THREAD }, 201);
    if (path === `/api/conversations/${THREAD.id}`) return snapshot();
    return answer({ type: 'not-found', title: 'Nothing is here', status: 404, detail: `No route ${path}.` }, 404);
  }));
  await act(async () => {
    render(<MemoryRouter initialEntries={['/templates']}><Routes><Route path="*" element={<Studio />} /></Routes></MemoryRouter>);
  });
}

const messageBoxLink = () => screen.queryByRole('link', { name: 'Skip to the message box' });

describe('spec 02 section 3.1: SkipLinks offers the message box only while the chat draws it', () => {
  it('offers no message box while the conversation loads', async () => {
    await studioWith(() => new Promise<Response>(() => undefined));
    await waitFor(() => expect(screen.getByText('Reading the conversation')).not.toBeNull());
    expect(screen.getByRole('link', { name: 'Skip to the panel' })).not.toBeNull();
    expect(messageBoxLink()).toBeNull();
  });

  it('offers no message box for a conversation that has ended', async () => {
    const discarded: Snapshot = { ...snapshotFrom([], 1), conversation: { ...THREAD, state: 'discarded' } };
    await studioWith(async () => answer(discarded));
    await waitFor(() => expect(screen.getByText("This new site's conversation was discarded; you can read it.")).not.toBeNull());
    expect(document.getElementById('composer-input')).toBeNull();
    expect(messageBoxLink()).toBeNull();
  });

  it('offers the message box once the chat draws it', async () => {
    await studioWith(async () => answer({ ...snapshotFrom([], 1), conversation: THREAD }));
    await waitFor(() => expect(document.getElementById('composer-input')).not.toBeNull());
    expect(messageBoxLink()).not.toBeNull();
  });
});
