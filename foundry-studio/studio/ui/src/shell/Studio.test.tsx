// The model Settings names (`Studio` holds it for `SettingsPanel`): the model of the conversation the chat last showed
// in this page's life. Settings has no chat of its own, and the label is held in memory only, so a page opened on
// Settings names none rather than whatever an earlier page left in the browser.

import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { conversationOf, snapshotFrom } from '../testing/journal.ts';
import { FakeSource, ME } from '../testing/render.tsx';
import { Studio } from './Studio.tsx';

const THREAD = { ...conversationOf([], null), id: '01M5AH0P6Z2CBS21960VQG3RJB', model_label: 'Claude Sonnet 5.5' };
const answer = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });

afterEach(() => {
  cleanup();
  sessionStorage.clear();
  vi.unstubAllGlobals();
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
    render(<MemoryRouter initialEntries={[address]}><Routes><Route path="*" element={<Studio />} /></Routes></MemoryRouter>);
  });
}

const named = () => document.querySelector('#settings .settings__model')?.textContent ?? null;

describe('Settings names the model of the conversation the chat last showed, held in memory', () => {
  it('names the model of the chat open before the person went to Settings', async () => {
    await studioOn('/templates');
    await waitFor(() => expect(document.getElementById('composer-input')).not.toBeNull());
    fireEvent.click(screen.getByRole('link', { name: 'Settings' }));
    await waitFor(() => expect(document.getElementById('settings')).not.toBeNull());
    expect(named()).toBe('Claude Sonnet 5.5');
  });

  it('names none on a page opened on Settings, whatever the browser holds from an earlier page', async () => {
    sessionStorage.setItem('studio.model-label', 'An earlier conversation');
    await studioOn('/settings');
    await waitFor(() => expect(document.getElementById('settings')).not.toBeNull());
    expect(named()).toBeNull();
  });
});
