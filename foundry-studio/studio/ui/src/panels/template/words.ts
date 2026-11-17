// Spec 05 section 6's words for knobs, evaluations and censuses: a knob's range and default as its hint says them,
// who set a value, what a value its default expression gave fits (D120), a census as tiles and as one sentence.
// Lengths are whole millimetres, written with thousands separators; counts are whole numbers (ruling D74).

import type { Census, EvaluationProblem, KnobDeclaration } from '../../protocol/template.ts';
import type { KnobSetBy } from '../../protocol/template_draft.ts';

export const grouped = (value: number): string => value.toLocaleString('en-US');

const unitOf = (knob: KnobDeclaration): string => (knob.unit === 'mm' ? ' mm' : knob.unit === 'deg' ? ' degrees' : '');

/** A knob's value in words: an option's label, else the number with its unit. */
export function valueWords(knob: KnobDeclaration, value: number): string {
  const option = knob.options.find((one) => one.value === value);
  return option ? option.label : `${grouped(value)}${unitOf(knob)}`;
}

/** "4,250 to 255,000 mm, in steps of 850 mm", or "1 VTM or 2 VTMs". */
export function rangeWords(knob: KnobDeclaration): string {
  if (knob.options.length > 0) {
    const labels = knob.options.map((option) => option.label);
    return labels.length === 2 ? labels.join(' or ') : `${labels.slice(0, -1).join(', ')} or ${labels[labels.length - 1]}`;
  }
  const unit = unitOf(knob);
  const span = `${grouped(knob.min ?? 0)} to ${grouped(knob.max ?? 0)}${unit}`;
  return (knob.step ?? 1) > 1 ? `${span}, in steps of ${grouped(knob.step ?? 1)}${unit}` : span;
}

/** A knob's template default: its value, or that an expression over the other knobs gives it. */
export function defaultWords(knob: KnobDeclaration): string {
  return typeof knob.default === 'number' ? valueWords(knob, knob.default) : 'follows the other knobs';
}

/** A description's sentences, and the one naming knobs in braces, which says what a default expression fits. */
function sentences(knob: KnobDeclaration): { plain: string[]; fits: string | undefined } {
  const all = knob.description?.split(/(?<=\.)\s+/) ?? [];
  return { plain: all.filter((sentence) => !sentence.includes('{')), fits: all.find((sentence) => sentence.includes('{')) };
}

/** The hint's parts, for a tooltip that lays them out: the range, the preset's default, and what the knob changes. */
export function hintParts(knob: KnobDeclaration): { range: string; fallback: string; changes: string } {
  return { range: rangeWords(knob), fallback: defaultWords(knob), changes: sentences(knob).plain.join(' ') };
}

/** The knob's hint: its range, the template's default, and what it changes (a wall knob's names its wall). */
export function hintWords(knob: KnobDeclaration): string {
  const description = sentences(knob).plain.map((sentence) => ` ${sentence}`).join('');
  return `${rangeWords(knob)}; template default ${defaultWords(knob)}.${description}`;
}

/** "Fits 5 aisles and the stations on its walls: 48,450 mm": the line of a knob its default expression gave, from
 * its description's sentence naming knobs in braces, each filled in at `values`; else "Follows the other knobs". */
export function derivedWords(knob: KnobDeclaration, values: Record<string, number>, knobs: KnobDeclaration[]): string {
  const fits = sentences(knob).fits?.replace(/\.$/, '').replace(/\{([a-z][a-z0-9_]*)\}/g, (braced, key: string) => {
    const named = knobs.find((one) => one.key === key);
    return named !== undefined && values[key] !== undefined ? valueWords(named, values[key]) : braced;
  });
  return `${fits ?? 'Follows the other knobs'}: ${valueWords(knob, values[knob.key])}`;
}

/** Whether `value` is one of the knob's valid values, and if not, the error its field shows. */
export function invalidWords(knob: KnobDeclaration, value: number | null): string | null {
  if (value === null || !Number.isInteger(value)) return `${knob.label} is a whole number: ${rangeWords(knob)}.`;
  if (knob.options.length > 0) {
    return knob.options.some((option) => option.value === value) ? null : `${knob.label} is ${rangeWords(knob)}.`;
  }
  const [min, max, step] = [knob.min ?? 0, knob.max ?? 0, knob.step ?? 1];
  return value < min || value > max || (value - min) % step !== 0 ? `${knob.label} is ${rangeWords(knob)}.` : null;
}

/** The "set by" line; null for a knob at the template's default, which carries none. */
export function setByWords(setBy: KnobSetBy | undefined, me: string | null): string | null {
  if (setBy === undefined || setBy.by === 'template') return null;
  if (setBy.by === 'agent') return setBy.turn !== null ? `Set by the agent in turn ${setBy.turn}` : 'Set by the agent';
  if (setBy.author !== null && setBy.author.subject === me) return 'Set by you';
  return `Set by ${setBy.author?.name ?? 'someone'}`;
}

const DERIVED: [string, string][] = [['relay', 'relays'], ['robot', 'robots'], ['slot', 'storage slots'], ['cell', 'floor cells']];

/** The census as tiles: each count knob's instances by its label, then what the floor compile derived. */
export function figures(census: Census, knobs: KnobDeclaration[]): { key: string; value: number; label: string }[] {
  const tiles = Object.entries(census.by_knob)
    .filter(([, count]) => count > 0)
    .map(([key, count]) => ({ key: `by_knob/${key}`, value: count, label: knobs.find((knob) => knob.key === key)?.label ?? key }));
  for (const [component, label] of DERIVED) {
    const count = census.derived[component];
    if (count !== undefined) tiles.push({ key: `derived/${component}`, value: count, label });
  }
  return tiles;
}

/** What a person weighs a floor by, in one order everywhere: the floor's shape in numbers, not the compile's internals
 * (cells, poses, tilings), which say nothing to whoever is choosing or shaping a map. A zero is left out. `key` names
 * the figure in its tile's reference (`census/{key}`), which an agent's highlight may already name it by. */
export function headlineFigures(census: Census): { key: string; label: string; value: number }[] {
  const sum = (wanted: (key: string) => boolean): number =>
    Object.entries(census.by_knob).filter(([key]) => wanted(key)).reduce((total, [, count]) => total + count, 0);
  return [
    { key: 'aisles', label: 'Aisles', value: census.by_knob.aisles ?? 0 },
    { key: 'pick-stations', label: 'Pick stations', value: sum((key) => key.startsWith('pick')) },
    { key: 'putaway-stations', label: 'Putaway stations', value: sum((key) => key.startsWith('putaway')) },
    { key: 'storage-slots', label: 'Storage slots', value: census.derived.slot ?? 0 },
    { key: 'robots', label: 'Robots', value: census.derived.robot ?? 0 },
  ].filter((one) => one.value > 0);
}

export function problemWords(problems: EvaluationProblem[]): string {
  return problems.length === 0 ? 'no problems' : problems.length === 1 ? '1 problem' : `${problems.length} problems`;
}

/** One sentence naming what an evaluation holds by its census, as a picture of it is named: its counts and problems. */
export function censusSentence(subject: string, census: Census | null, knobs: KnobDeclaration[], problems: EvaluationProblem[]): string {
  const counted = census === null ? [] : figures(census, knobs).map((tile) => `${grouped(tile.value)} ${tile.label.toLowerCase()}`);
  const what = counted.length > 0 ? `: ${counted.join(', ')}` : '';
  return `${subject}${what}; ${problemWords(problems)}.`;
}
