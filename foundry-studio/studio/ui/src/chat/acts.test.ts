// Spec 08 section 3.7: every POST carries an Idempotency-Key, so the RunCard's Cancel the run and Cancel the sweep
// each send a fresh one per press, which the API's cancel routes require.

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cancelRun, cancelSubmission } from './acts.ts';

afterEach(() => vi.unstubAllGlobals());

describe("the run card's cancels", () => {
  it('send a fresh Idempotency-Key with each press', async () => {
    const keys: [string, string | undefined][] = [];
    vi.stubGlobal('fetch', vi.fn(async (path: string, init?: RequestInit) => {
      keys.push([path, (init?.headers as Record<string, string> | undefined)?.['idempotency-key']]);
      return new Response(JSON.stringify({ outcome: 'cancelled', state: 'cancelled' }), { status: 202 });
    }));
    await cancelRun('01KTJ153M7D109TG4G7BBVCZYW');
    await cancelRun('01KTJ153M7D109TG4G7BBVCZYW');
    await cancelSubmission('01KQ7Z3F0V5Q8M2R1T6W9X4Y7B');
    expect(keys.map(([path]) => path)).toEqual([
      '/api/runs/01KTJ153M7D109TG4G7BBVCZYW/cancel', '/api/runs/01KTJ153M7D109TG4G7BBVCZYW/cancel',
      '/api/submissions/01KQ7Z3F0V5Q8M2R1T6W9X4Y7B/cancel',
    ]);
    const sent = keys.map(([, key]) => key);
    expect(sent.every((key) => key !== undefined && /^[A-Za-z0-9_-]{1,255}$/.test(key))).toBe(true);
    expect(new Set(sent).size).toBe(3);
  });
});
