// Spec 07 section 3.8's routes as the Scenario panel calls them, beside the shell's `getJson` and `postJson`: an edit
// is a PATCH and a delete a DELETE, and every refusal is read as spec 01 section 3.14's problem body so the panel can
// say it in spec 02 section 3.8's words. A network failure is a problem with no status, never a thrown error.

import type { Problem } from '../../protocol/problem.ts';
import type { Answer } from '../../shell/api.ts';

async function send<T>(method: 'PATCH' | 'DELETE', path: string, body?: unknown): Promise<Answer<T>> {
  try {
    const response = await fetch(path, {
      method,
      headers: { 'content-type': 'application/json', accept: 'application/json' },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    const text = await response.text();
    let read: unknown = null;
    try {
      read = text === '' ? null : JSON.parse(text);
    } catch {
      read = null;
    }
    if (response.ok) return { ok: true, status: response.status, value: read as T };
    const problem = read !== null && typeof read === 'object' && 'type' in read ? (read as Problem) : null;
    return { ok: false, status: response.status, problem, reason: problem?.detail ?? `the service answered ${response.status}` };
  } catch (fault) {
    const reason = fault instanceof Error ? `the network failed (${fault.message})` : 'the network failed';
    return { ok: false, status: null, problem: null, reason };
  }
}

export const patchJson = <T>(path: string, body: unknown): Promise<Answer<T>> => send<T>('PATCH', path, body);
export const deleteJson = <T>(path: string): Promise<Answer<T>> => send<T>('DELETE', path);

/** A fresh key per press (spec 01 section 3.13). */
export const pressKey = (): string => crypto.randomUUID();
