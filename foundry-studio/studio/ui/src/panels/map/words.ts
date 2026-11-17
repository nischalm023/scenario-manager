// Spec 03 sections 3.6 and 5, the Map panel's sentences: the stage's stable name, the counts table's caption, the
// summary line its description reads, and how a mark is named in the text twin. Every count is discovered from the
// summary's rows, never a fixed list of kinds; a position is in the site's primary frame.

import type { MapSummary, Mark } from '../../protocol/map.ts';
import type { ScopeSeq } from '../../protocol/panel.ts';
import { axesSentence, positionWords } from '../../viewer/stage.ts';

/** 1940 as "1,940". */
export const grouped = (count: number): string => count.toLocaleString('en-US');

/** The map scope's changeset in the cut a summary read. */
export function mapSeq(summary: MapSummary): number | null {
  return summary.resolved.find((row) => row.scope.endsWith('/map'))?.seq ?? null;
}

/** "Small L-shape, map at changeset 1": what is drawn and its version, the stage's name. */
export function stageLabel(summary: MapSummary): string {
  const seq = mapSeq(summary);
  return seq === null ? summary.site.label : `${summary.site.label}, map at changeset ${seq}`;
}

/** "map changeset 1, flow 1, fleet 1": the site's sub-scopes in the cut, the map first. */
export function cutWords(resolved: ScopeSeq[]): string {
  const named = resolved
    .map((row) => ({ name: row.scope.split('/').pop() ?? row.scope, seq: row.seq, depth: row.scope.split('/').length }))
    .filter((row) => row.depth > 4)
    .sort((a, b) => (a.name === 'map' ? -1 : b.name === 'map' ? 1 : a.name.localeCompare(b.name)));
  return named.map((row, n) => (n === 0 ? `${row.name} changeset ${row.seq}` : `${row.name} ${row.seq}`)).join(', ');
}

export const censusCaption = (resolved: ScopeSeq[]): string =>
  `Entities in the site at ${cutWords(resolved)}, counted by kind: one row per set of components`;

/** How many entities carry `component` among the counts' rows. */
const carrying = (summary: MapSummary, component: string): number =>
  summary.counts.filter((row) => row.components.includes(component)).reduce((sum, row) => sum + row.count, 0);

/** "1,100 floor cells, 6 stations. Axes of the shell frame: …. 4 entities highlighted by the agent. Nothing selected." */
export function summaryLine(summary: MapSummary, highlighted: number, selected: string | null): string {
  const holds = [[carrying(summary, 'cell'), 'floor cell'], [carrying(summary, 'station'), 'station']]
    .filter(([count]) => (count as number) > 0)
    .map(([count, word]) => `${grouped(count as number)} ${word}${count === 1 ? '' : 's'}`);
  const held = holds.length > 0 ? `${holds.join(', ')}.` : `${grouped(summary.counts.reduce((sum, row) => sum + row.count, 0))} entities.`;
  const agent = highlighted === 0 ? 'No agent highlights.' : `${grouped(highlighted)} ${highlighted === 1 ? 'entity' : 'entities'} highlighted by the agent.`;
  return `${held} ${axesSentence(summary.axes)} ${agent} ${selected === null ? 'Nothing selected.' : `${selected} selected.`}`;
}

/** A mark's kind as a chip shows it: its components, the bookkeeping `external_ids` left out. */
export function kindOf(components: string[]): string {
  const shown = components.filter((name) => name !== 'external_ids');
  return (shown.length > 0 ? shown : components).join(' + ');
}

/** "x 12.4 m, y 30.5 m", or nothing for a mark that stands nowhere. */
export const markPlace = (mark: Pick<Mark, 'centre_mm'>): string | null =>
  mark.centre_mm === null ? null : positionWords(mark.centre_mm);

/** The five figures a person weighs a floor by, from what made the site and what it holds; a zero is left out. */
export function mapFigures(summary: MapSummary): { label: string; value: number }[] {
  const knobs = summary.made_from?.knobs ?? [];
  const sum = (wanted: (key: string) => boolean) => knobs.filter((knob) => wanted(knob.key)).reduce((total, knob) => total + knob.value, 0);
  const counted = (label: string) => summary.counts.find((row) => row.label === label)?.count ?? 0;
  return [
    { label: 'Aisles', value: knobs.find((knob) => knob.key === 'aisles')?.value ?? 0 },
    { label: 'Pick stations', value: sum((key) => key.startsWith('pick')) },
    { label: 'Putaway stations', value: sum((key) => key.startsWith('putaway')) },
    { label: 'Storage slots', value: counted('slot') },
    { label: 'Robots', value: counted('robot') },
  ].filter((one) => one.value > 0);
}

