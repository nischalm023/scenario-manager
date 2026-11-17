// The Docs section's panel when its code fails to load, as when a tab opened before a redeploy asks for a chunk the
// new build renamed: the panel says so and offers Try again, which reloads the page, and the rest of the app stands.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DocsPanel } from './DocsPanel.tsx';

vi.mock('./DocsSection.tsx', () => {
  throw new Error('Failed to fetch dynamically imported module');
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('the Docs panel whose section fails to load', () => {
  it('says the documents could not load, keeps the rest of the app, and Try again reloads the page', async () => {
    const reload = vi.spyOn(window.history, 'go').mockImplementation(() => undefined);
    await act(async () => {
      render(<main><p>The rest of the app</p><DocsPanel doc="docs/architecture/live-frames" /></main>);
    });
    expect(await screen.findByText(/^The documents could not load: /)).not.toBeNull();
    expect(screen.getByText('The rest of the app')).not.toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(reload).toHaveBeenCalledWith(0);
  });
});
