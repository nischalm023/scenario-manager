// Spec 05 section 3.8's routes as the template panels call them: JSON in and out on the same origin, every write with
// a fresh Idempotency-Key (spec 01 section 3.13), a refusal read as the problem body so a panel can say it in words.
// The shell's `getJson` and `postJson` serve reads and posts; PATCH and DELETE are the draft's own.

import type { Problem } from '../../protocol/problem.ts';
import type { Answer } from '../../shell/api.ts';

export { getJson } from '../../shell/api.ts';

/** A fresh key per press: a retry of the same press reuses it, a new press never does. */
export const freshKey = (): string => crypto.randomUUID();

export async function sendJson<T>(
  method: 'POST' | 'PATCH' | 'DELETE',
  path: string,
  body?: unknown,
  key: string = freshKey(),
): Promise<Answer<T>> {
  const headers: Record<string, string> = { accept: 'application/json', 'idempotency-key': key };
  if (body !== undefined) headers['content-type'] = 'application/json';
  let response: Response;
  try {
    response = await fetch(path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  } catch (fault) {
    const reason = fault instanceof Error ? `the network failed (${fault.message})` : 'the network failed';
    return { ok: false, status: null, problem: null, reason };
  }
  const text = await response.text();
  let read: unknown = null;
  try {
    read = text === '' ? null : JSON.parse(text);
  } catch {
    read = null;
  }
  if (response.ok) return { ok: true, status: response.status, value: read as T };
  const problem = read !== null && typeof read === 'object' && 'type' in read ? (read as Problem) : null;
  const reason = problem?.detail ?? `the service answered ${response.status} ${response.statusText}`.trim();
  return { ok: false, status: response.status, problem, reason };
}
