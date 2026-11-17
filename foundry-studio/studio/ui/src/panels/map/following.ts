// Spec 03 section 3.6, "Following a branch": a map on a branch hears every scope of its cut through
// `GET /api/scope-events` (spec 01 section 3.6), since a flow or fleet changeset changes the floor too; while that
// stream is down and the browser tab is visible the panel reads its summary again every 30 s of wall clock instead.

import { useEffect } from 'react';

/** How often a page whose stream is down reads the head again (wall clock). */
const DOWN_READ_MS = 30_000;

/** Call `moved` whenever one of `scopes` moves; `scopes` empty follows nothing. */
export function useFollowing(scopes: string[], moved: () => void): void {
  const named = scopes.join('\n');
  useEffect(() => {
    if (named === '') return;
    const query = named.split('\n').map((scope) => `scope=${encodeURIComponent(scope)}`).join('&');
    const source = new EventSource(`/api/scope-events?${query}`);
    let down = false;
    source.addEventListener('scope.moved', () => moved());
    source.addEventListener('open', () => {
      if (down) moved();
      down = false;
    });
    source.addEventListener('error', () => {
      down = true;
    });
    const timer = setInterval(() => {
      if (down && document.visibilityState === 'visible') moved();
    }, DOWN_READ_MS);
    return () => {
      source.close();
      clearInterval(timer);
    };
  }, [named, moved]);
}
