// The Scenario panel's words (spec 07 sections 3.2, 4.2 and 5): a field's value as the panel shows it, in the unit
// the field catalogue names (hours of simtime for a duration held in milliseconds), and back; who last set a field;
// the readiness line's state and what follows it; a conflict's sentence; and the count beside Run. None of it reads
// the page, so the unit tests hold it.

import type { FieldEntry, SetBy } from '../../protocol/scenario.ts';
import type { Readiness } from '../../protocol/workload.ts';
import type { JsonValue } from '../../protocol/serde_json/JsonValue.ts';

/** A stored value in the field's shown unit, as an input holds it: `""` for null. */
export function shownValue(field: FieldEntry, value: JsonValue | undefined): string {
  if (value === null || value === undefined) return '';
  if (typeof value === 'number') return String(Number((value / field.per_unit).toFixed(6)));
  return String(value);
}

/** An input's text as the value it stores, or why it cannot be one: a closed value as written, else a number in
 * the shown unit, or nothing for the wave interval. */
export function storedValue(field: FieldEntry, text: string): { ok: true; value: JsonValue } | { ok: false; why: string } {
  const trimmed = text.trim();
  if (field.options.length > 0) return { ok: true, value: trimmed };
  if (trimmed === '') {
    return field.pointer.endsWith('wave_every_sim_ms') ? { ok: true, value: null } : { ok: false, why: `${field.label} needs a number.` };
  }
  const number = Number(trimmed.replaceAll(',', ''));
  if (!Number.isFinite(number)) return { ok: false, why: `${field.label} needs a number; "${trimmed}" is not one.` };
  const stored = number * field.per_unit;
  return { ok: true, value: field.per_unit === 1 ? stored : Math.round(stored) };
}

/** A value with its unit, as a sentence says it: "2 hours of simtime", "300 orders an hour". */
export function valueWords(field: FieldEntry, value: JsonValue | undefined): string {
  if (value === null || value === undefined) return 'nothing';
  const option = field.options.find((choice) => choice.value === value);
  if (option !== undefined) return option.words.split(':')[0].toLowerCase();
  const shown = shownValue(field, value);
  return field.unit === '' ? shown : `${shown} ${field.unit}`;
}

/** Who last set a field, as its hint says it: "set by you", "set by the agent in turn 6", "set by Dana Ruiz". */
export function setByWords(set: SetBy | undefined, me: string | null): string {
  if (set === undefined) return '';
  if (set.by === 'agent') return set.turn ? `set by the agent in turn ${set.turn.n}` : 'set by the agent';
  return set.author.subject === me ? 'set by you' : `set by ${set.author.name}`;
}

/** The one setter of every field, when there is one: the status line says it once instead of every hint. */
export function sharedSetter(sets: (SetBy | undefined)[], me: string | null): string | null {
  const words = new Set(sets.map((set) => setByWords(set, me)));
  if (words.size !== 1) return null;
  const [only] = words;
  return only === '' ? null : only;
}

/** A conflict as the field says it: "Changed by the agent in turn 6 to 2 hours of simtime; yours was not saved". */
export function conflictWords(field: FieldEntry, value: JsonValue | undefined, set: SetBy | null | undefined, me: string | null): string {
  const who = set ? setByWords(set, me).replace(/^set by /, '') : 'someone';
  return `Changed by ${who} to ${valueWords(field, value)}; yours was not saved.`;
}

const SUB_SCOPES = ['map', 'flow', 'fleet'] as const;

/** The readiness line's state, said once, and what follows it: "Ready to run" and "900 orders at map 1, flow 2,
 * fleet 1", or "Not ready" and "2 problems". */
export function readinessWords(readiness: Readiness, site: string): { state: string; detail: string } {
  const errors = readiness.problems.filter((problem) => problem.severity === 'error').length;
  if (!readiness.ready) return { state: 'Not ready', detail: `${errors} problem${errors === 1 ? '' : 's'}` };
  const seqs = readiness.cut?.seqs ?? {};
  const heads = SUB_SCOPES.map((sub) => `${sub} ${seqs[`${site}/${sub}`] ?? 0}`).join(', ');
  const orders = readiness.workload?.orders ?? 0;
  return { state: 'Ready to run', detail: `${orders.toLocaleString('en-US')} order${orders === 1 ? '' : 's'} at ${heads}` };
}

/** "5 runs", "1 run". */
export const runsWords = (runs: number): string => `${runs} run${runs === 1 ? '' : 's'}`;

/** "Run 5 runs", "Run 1 run". */
export const runLabel = (runs: number): string => `Run ${runsWords(runs)}`;

/** The note beside Run when it makes more than one run: the count, and spec 08's limit on how many go at once,
 * which promises no start, since other runs may hold the seats. */
export function runCountWords(values: number, repeats: number): string | null {
  const runs = values * repeats;
  if (runs <= 1) return null;
  return `${values > 1 ? `${values} values, ` : ''}${runsWords(runs)}; at most five go at a time.`;
}

/** A sweep's values as typed, "200, 250, 300", read for the field: each stored value, or why one cannot be. */
export function sweepValues(field: FieldEntry, text: string): { ok: true; values: JsonValue[] } | { ok: false; why: string } {
  const parts = text.split(/[,;]/).map((part) => part.trim()).filter((part) => part !== '');
  const values: JsonValue[] = [];
  for (const part of parts) {
    const read = storedValue(field, part);
    if (!read.ok) return read;
    values.push(read.value);
  }
  if (values.length < 2 || values.length > 12) return { ok: false, why: `A sweep takes 2 to 12 values; this has ${values.length}.` };
  return { ok: true, values };
}

export const capitalised = (words: string): string => words.charAt(0).toUpperCase() + words.slice(1);

/** The id of a field's input, from its pointer: `sc-run_controls-duration_sim_ms`. */
export const fieldId = (pointer: string): string => `sc-${pointer.split('/').filter(Boolean).join('-')}`;
