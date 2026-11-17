// Spec 01 section 3.6's routes as the browser calls them: same origin (Vite or the gate hands `/api` to the API), JSON
// in and out, and a refusal read as the problem body of section 3.14 so each place can say it in spec 02 section
// 3.8's words. A network failure is a problem with no status, never a thrown error.

import type { Problem } from '../protocol/problem.ts';

export type Answer<T> =
  | { ok: true; status: number; value: T }
  | { ok: false; status: number | null; problem: Problem | null; reason: string };

async function answerOf<T>(response: Response): Promise<Answer<T>> {
  const text = await response.text();
  let body: unknown = null;
  try {
    body = text === '' ? null : JSON.parse(text);
  } catch {
    body = null;
  }
  if (response.ok) return { ok: true, status: response.status, value: body as T };
  const problem = body !== null && typeof body === 'object' && 'type' in body ? (body as Problem) : null;
  const reason = problem?.detail ?? `the service answered ${response.status} ${response.statusText}`.trim();
  return { ok: false, status: response.status, problem, reason };
}

function offline(fault: unknown): Answer<never> {
  const reason = fault instanceof Error ? `the network failed (${fault.message})` : 'the network failed';
  return { ok: false, status: null, problem: null, reason };
}

export async function getJson<T>(path: string): Promise<Answer<T>> {
  try {
    return await answerOf<T>(await fetch(path, { headers: { accept: 'application/json' } }));
  } catch (fault) {
    return offline(fault);
  }
}

/** A POST; `idempotencyKey` for an act that creates or starts work (spec 01 section 3.13). */
export async function postJson<T>(path: string, body: unknown, idempotencyKey?: string): Promise<Answer<T>> {
  const headers: Record<string, string> = { 'content-type': 'application/json', accept: 'application/json' };
  if (idempotencyKey !== undefined) headers['idempotency-key'] = idempotencyKey;
  try {
    return await answerOf<T>(await fetch(path, { method: 'POST', headers, body: JSON.stringify(body ?? {}) }));
  } catch (fault) {
    return offline(fault);
  }
}

/** A reason as a clause inside a sentence: its last full stop dropped. */
export const clause = (reason: string): string => reason.replace(/\.\s*$/, '');
