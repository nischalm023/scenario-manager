// An ended run's results in words (spec 08 sections 3.9 and 3.10): a metric's value in its unit (whole counts with
// separators, durations in hours, minutes and seconds, distances in metres to one decimal, a ratio as the number
// stated, a rate per the hour of its clock), a span of simtime to the second, a window's whole hours, a metric's kind,
// a list, and a stub attempt's sentence. None of it reads the page or names a metric, so the unit tests hold it.

import type { Clock } from '../../protocol/clock.ts';
import type { MetricRow, SimWindow } from '../../protocol/results.ts';
import type { MetricUnit } from '../../protocol/runner.ts';
import type { RunDetail } from '../../protocol/runs.ts';

const HOUR_MS = 3_600_000;

/** The stub sentence simrunner writes (spec 08 section 3.5), which goes with every number of a stub attempt. */
export const STUB_WORDS = 'stub runner: canned results; no simulation ran';

/** A span or instant of simtime to the second: "3 h 30 s", "47 min 33 s", "0 s". */
export function spanWords(ms: number): string {
  const seconds = Math.round(ms / 1000);
  const parts: [number, string][] = [[Math.floor(seconds / 3600), 'h'], [Math.floor(seconds / 60) % 60, 'min'], [seconds % 60, 's']];
  const said = parts.filter(([count]) => count > 0).map(([count, unit]) => `${count} ${unit}`);
  return said.length === 0 ? '0 s' : said.join(' ');
}

const grouped = (value: number, digits: number): string =>
  value.toLocaleString('en-US', { maximumFractionDigits: digits, minimumFractionDigits: 0 });

/** A change against a reference, signed: "+148", "0", "-2 min 5 s"; a rate per the hour of `clock`. */
export function changeWords(unit: MetricUnit, change: number, clock: Clock = 'sim'): string {
  const said = valueWords(unit, Math.abs(change), clock);
  return change > 0 ? `+${said}` : change < 0 ? `-${said}` : said;
}

/** The hour a rate is per, on each clock. */
export const HOUR_ON: Record<Clock, string> = { sim: 'simulated hour', scheduler: 'hour of scheduler time', wall: 'wall-clock hour' };

/** A value in its unit, no clock tag: a duration under a minute to a tenth of a second, a rate per the hour of
 * `clock`, simtime's unless the caller names another. */
export function valueWords(unit: MetricUnit, value: number | null, clock: Clock = 'sim'): string {
  if (value === null) return 'no value';
  switch (unit) {
    case 'count': return grouped(value, 1);
    case 'duration_ms': return Math.abs(value) < 60_000 ? `${grouped(value / 1000, 1)} s` : spanWords(value);
    case 'distance_mm': return `${grouped(value / 1000, 1)} m`;
    case 'ratio': return grouped(value, 2);
    case 'rate_per_hour': return `${grouped(value, 1)} per ${HOUR_ON[clock]}`;
  }
}

/** Whether a unit's values are times, so a page puts them in the clock a stored result's row or a metric stream's
 * descriptor names. */
export const isTime = (unit: MetricUnit): boolean => unit === 'duration_ms';

/** The kind of a metric, as the All metrics table says it: "count, additive". */
export function kindWords(row: Pick<MetricRow, 'unit' | 'aggregation' | 'denominator'>): string {
  const unit = { count: 'count', duration_ms: 'duration', distance_mm: 'distance', ratio: 'ratio', rate_per_hour: 'rate' }[row.unit];
  const aggregation = row.aggregation.replace('_', ' ');
  return row.denominator ? `${unit}, ${aggregation}, of ${row.denominator}` : `${unit}, ${aggregation}`;
}

/** "hours 1 and 2", "hours 0 to 2", "hour 0", or nothing when the window holds no whole hour. */
export function wholeHours(window: SimWindow): string {
  const first = Math.ceil(window.from_sim_ms / HOUR_MS);
  const last = Math.floor(window.to_sim_ms / HOUR_MS) - 1;
  if (last < first) return '';
  if (last === first) return `hour ${first}`;
  return last === first + 1 ? `hours ${first} and ${last}` : `hours ${first} to ${last}`;
}

/** "a, b and c". */
export function listWords(items: string[]): string {
  if (items.length <= 1) return items.join('');
  return `${items.slice(0, -1).join(', ')} and ${items[items.length - 1]}`;
}

/** The stub's sentence for a stub attempt: the ending's own detail, which simrunner wrote. */
export function stubSentence(run: RunDetail): string | null {
  return run.attempts.at(-1)?.stub === true ? (run.stop_detail ?? STUB_WORDS) : null;
}
