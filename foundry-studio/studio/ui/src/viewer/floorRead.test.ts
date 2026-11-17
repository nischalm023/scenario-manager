// A mount's floor read (spec 03 sections 3.6 and 3.8): a failed request, a refused key and a body cut off on the way
// each answer the sentence the stage shows with Retry, never a read that does not end.

import { describe, expect, it } from 'vitest';
import { readFloor } from './floorRead.ts';

const answering = (answer: Partial<Response> | Error): typeof fetch =>
  (() => (answer instanceof Error ? Promise.reject(answer) : Promise.resolve(answer as Response))) as typeof fetch;

describe('a floor read', () => {
  it('answers the bytes of a floor the API serves', async () => {
    const read = await readFloor('k', answering({ ok: true, status: 200, arrayBuffer: () => Promise.resolve(new Uint8Array([1, 2]).buffer) }));
    expect(read).toEqual({ ok: true, bytes: new Uint8Array([1, 2]) });
  });

  it('says why when the request fails or the key is refused', async () => {
    expect(await readFloor('k', answering(new TypeError('Failed to fetch')))).toEqual({ ok: false, failed: 'The floor could not be read: the network failed.' });
    expect(await readFloor('k', answering({ ok: false, status: 404 }))).toEqual({ ok: false, failed: 'The floor could not be read: the floor answered 404.' });
  });

  it('says why when the body is cut off after the status', async () => {
    const cut = answering({ ok: true, status: 200, arrayBuffer: () => Promise.reject(new TypeError('network error')) });
    expect(await readFloor('k', cut)).toEqual({ ok: false, failed: 'The floor could not be read: the network failed.' });
  });
});
