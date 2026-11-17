// The folded chat (`ChatDrawer`, folded and opened by `Studio`): a folded chat draws nothing that can take focus, so
// every keyboard path into it opens it first or reaches its tab, and its tab names a card that waits on the person
// while the card is out of sight. And the drawer's width, which its edge says and moves within the window it stands in.

import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { conversationOf, entriesOf, snapshotFrom } from '../testing/journal.ts';
import { FakeSource, ME } from '../testing/render.tsx';
import { ChatDrawer } from './ChatDrawer.tsx';
import { Studio } from './Studio.tsx';

/** The new-site thread as far as its turn waiting on the person's answer to a question. */
const ASKED = entriesOf('new-site-thread').filter((entry) => entry.seq <= 9);
const THREAD = conversationOf(ASKED, null);
const answer = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });

beforeEach(() => localStorage.setItem('studio.chat', 'folded'));

afterEach(() => {
  cleanup();
  localStorage.clear();
  vi.unstubAllGlobals();
});

/** The Template list, whose chat opens the person's new-site thread, folded and holding the question. */
async function foldedOnTemplates() {
  vi.stubGlobal('EventSource', FakeSource);
  vi.stubGlobal('fetch', vi.fn(async (path: string, init?: RequestInit) => {
    if (path === '/api/me') return answer(ME);
    if (path === '/api/conversations' && init?.method === 'POST') return answer({ conversation: THREAD }, 201);
    if (path === `/api/conversations/${THREAD.id}`) return answer({ ...snapshotFrom(ASKED, 1), conversation: THREAD });
    return answer({ type: 'not-found', title: 'Nothing is here', status: 404, detail: `No route ${path}.` }, 404);
  }));
  await act(async () => {
    render(<MemoryRouter initialEntries={['/templates']}><Routes><Route path="*" element={<Studio />} /></Routes></MemoryRouter>);
  });
  await waitFor(() => expect(document.getElementById('composer-input')).not.toBeNull());
  expect(document.querySelector('.shell')?.getAttribute('data-chat-folded')).toBe('true');
}

const folded = () => document.querySelector('.shell')?.getAttribute('data-chat-folded') === 'true';

describe('a folded chat', () => {
  it("names the question that waits on the person on its tab, and the name goes once it opens", async () => {
    await foldedOnTemplates();
    const tab = screen.getByRole('button', { name: 'Open the agent chat: a question waits for you' });
    expect(tab.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(tab);
    expect(folded()).toBe(false);
    expect(tab.getAttribute('aria-label')).toBe('Fold the agent chat away');
  });

  it('opens before "Skip to the message box" puts focus in its message box', async () => {
    await foldedOnTemplates();
    fireEvent.click(screen.getByRole('link', { name: 'Skip to the message box' }));
    await waitFor(() => expect(document.activeElement?.id).toBe('composer-input'));
    expect(folded()).toBe(false);
  });

  it('is visited by F6 at its tab, which stands where its message box and log stand once open', async () => {
    await foldedOnTemplates();
    fireEvent.keyDown(document, { key: 'F6' });
    expect(document.activeElement?.id).toBe('chat-toggle');
    fireEvent.keyDown(document, { key: 'F6' });
    expect(document.activeElement?.closest('#panel-tabs, #panel')).not.toBeNull();
    fireEvent.keyDown(document, { key: 'F6', shiftKey: true });
    expect(document.activeElement?.id).toBe('chat-toggle');
  });
});

describe("the drawer's width", () => {
  /** The window resized to `width` CSS px, as the browser tells it. */
  const resized = (width: number) => act(() => {
    vi.stubGlobal('innerWidth', width);
    window.dispatchEvent(new Event('resize'));
  });

  it('follows the window, the edge saying the width shown and the most it may be, and comes back as the window grows', () => {
    vi.stubGlobal('innerWidth', 1920);
    localStorage.setItem('studio.chat-width', '800');
    render(<ChatDrawer folded={false} waiting={null} toggle={() => undefined}>{(edge) => <section id="chat">{edge}</section>}</ChatDrawer>);
    const edge = screen.getByRole('separator', { name: 'Width of the agent chat' });
    const said = () => [
      edge.getAttribute('aria-valuenow'), edge.getAttribute('aria-valuemax'),
      document.querySelector<HTMLElement>('.chat-drawer')?.style.getPropertyValue('--chat-width'),
    ];
    expect(said()).toEqual(['800', '800', '800px']);
    resized(1280);
    expect(said(), 'half the 1,280 px window').toEqual(['640', '640', '640px']);
    resized(1920);
    expect(said(), 'the width the person set, shown again').toEqual(['800', '800', '800px']);
    resized(1280);
    fireEvent.keyDown(edge, { key: 'ArrowRight' });
    expect(said(), 'narrowed from the width shown').toEqual(['624', '640', '624px']);
    expect(localStorage.getItem('studio.chat-width')).toBe('624');
    fireEvent.keyDown(edge, { key: 'End' });
    expect(said(), 'its widest in this window').toEqual(['640', '640', '640px']);
  });
});
