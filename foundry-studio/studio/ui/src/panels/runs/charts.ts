// The run page's and the comparison's charts as data (spec 08 sections 3.10 and 3.12), apart from the component that
// draws them (`Chart.tsx`) so the unit tests hold them: what a chart draws (its series with their ranges, its axes and
// its value range), a metric stream's chart kept up to date as its rows arrive (`StreamChart`), and the words and
// signs its axes, ticks and figures say values in.

import type { Clock } from '../../protocol/clock.ts';
import type { MetricDescriptor, MetricPoint } from '../../protocol/metric_stream.ts';
import type { MetricUnit } from '../../protocol/runner.ts';
import { HOUR_ON, spanWords, valueWords } from './resultWords.ts';

const HOUR_MS = 3_600_000;

/** One line or set of dots; `ranges` holds a value's lowest and highest at an x where repeats were averaged, drawn as
 * a bar in the series' colour. */
export interface ChartSeries {
  label: string;
  values: (number | null)[];
  /** Its colour: its slot in the palette (`paletteSlot`), the first series first. */
  key: number;
  ranges?: { at: number; low: number; high: number }[];
}

export interface ChartSpec {
  x: number[];
  series: ChartSeries[];
  xRange: [number, number];
  xTicks: { at: number; label: string }[];
  xTitle: string;
  yTitle: string;
  /** Where the vertical axis starts and ends. */
  yRange: [number, number];
  /** Readings as a step line; dots alone for a chart by run. */
  shape: 'line' | 'steps' | 'dots';
  yWords(value: number): string;
  /** A value as a tick or a tooltip says it, shorter than `yWords`; `yWords` when absent. */
  yShort?(value: number): string;
  /** An x as a tooltip's heading says it; the number itself when absent. */
  xWords?(x: number): string;
}

/** The vertical axis for values from `low` to `high`: from zero, or from the lowest value when one lies below it, to a
 * tenth of the drawn span above the highest, so no bar runs off. */
export function yRangeOf(low: number, high: number): [number, number] {
  const floor = Math.min(0, low);
  const top = Math.max(floor + 1, high);
  return [floor, top + (top - floor) * 0.1];
}

/** The vertical axis that holds every series' values and both ends of each of its ranges. */
export function seriesRange(series: ChartSeries[]): [number, number] {
  let low = Number.POSITIVE_INFINITY;
  let high = Number.NEGATIVE_INFINITY;
  const hold = (value: number) => {
    low = Math.min(low, value);
    high = Math.max(high, value);
  };
  for (const one of series) {
    for (const value of one.values) if (value !== null) hold(value);
    for (const range of one.ranges ?? []) {
      hold(range.low);
      hold(range.high);
    }
  }
  return yRangeOf(low, high);
}

/** The words at a line's end: its metric's label and its last value, so no line is told from another by colour. */
export const endWords = (series: ChartSeries, value: number, yWords: (value: number) => string): string =>
  `${series.label}: ${yWords(value)}`;

/** A duration's clock as an axis title names it. */
const CLOCK_NAME: Record<Clock, string> = { sim: 'simtime', scheduler: 'scheduler time', wall: 'wall clock' };

/** A unit as an axis title says it, a duration's or a rate's clock named: `clock`, simtime's unless the caller names
 * another. */
export function unitTitle(unit: MetricUnit, clock: Clock = 'sim'): string {
  switch (unit) {
    case 'count': return 'count';
    case 'duration_ms': return `duration, ${CLOCK_NAME[clock]}`;
    case 'distance_mm': return 'metres';
    case 'ratio': return 'ratio';
    case 'rate_per_hour': return `per ${HOUR_ON[clock]}`;
  }
}

/** The x axis's ticks over `hours` of simtime: every hour, or finer while the run is short, numbered in hours. */
function hourTicks(hours: number): { at: number; label: string }[] {
  const step = hours <= 0.5 ? 1 / 12 : hours <= 2 ? 0.25 : hours <= 6 ? 0.5 : 1;
  const ticks: { at: number; label: string }[] = [];
  for (let index = 0; index * step <= hours + 1e-9; index += 1) {
    const at = index * step;
    ticks.push({ at, label: step >= 1 ? `${Math.round(at)} h` : spanWords(Math.round(at * HOUR_MS)) });
  }
  return ticks;
}

/** How many series colours the tokens hold (`--color-series-1` to `--color-series-8`), in fixed order. */
export const PALETTE = 8;

/** A series' slot in the palette, 1 to 8 in fixed order, a slot again past the eighth, whose line a chart dashes. */
export const paletteSlot = (key: number): number => ((key - 1) % PALETTE) + 1;

/** A series' colour as an element's style takes it: its slot's token, which follows the theme on its own. */
export const swatch = (key: number): string => `var(--color-series-${paletteSlot(key)})`;

/** A row's place along the metric's dimensions, as one legend word: the keys joined. */
export const keyWords = (row: MetricPoint): string => (row.keys ?? []).join(' / ');

/** A simtime instant on a metric stream chart's x axis, which counts hours from the run's start. */
export const hoursOf = (ms: number): number => ms / HOUR_MS;

/** What a line's stated values come to, each a value the server sent: its last by simtime and when (in hours from
 * the run's start), its lowest and highest, and how many. No mean: one of windows can disagree with the run's own
 * figures, and means nothing for a running total. */
export interface SeriesStats {
  last: number;
  lastAt: number;
  min: number;
  max: number;
  count: number;
}

/** One line of a metric stream's chart, a place along the metric's dimensions: its value in each of the chart's time
 * slots, null where it stated none, and what its stated values come to, kept as each value is stated. */
export class StreamLine implements ChartSeries {
  readonly label: string;
  readonly key: number;
  readonly values: (number | null)[];
  private count = 0;
  private min = Number.POSITIVE_INFINITY;
  private max = Number.NEGATIVE_INFINITY;
  private last: number | null = null;
  private lastEndMs = Number.NEGATIVE_INFINITY;

  constructor(label: string, key: number, slots: number) {
    this.label = label;
    this.key = key;
    this.values = Array.from({ length: slots }, () => null);
  }

  /** What its stated values come to; null while none is stated. */
  get stats(): SeriesStats | null {
    if (this.last === null) return null;
    return { last: this.last, lastAt: hoursOf(this.lastEndMs), min: this.min, max: this.max, count: this.count };
  }

  /** States `value` in the slot at `index`, whose end is `endMs`. A slot stated twice keeps the later value and counts
   * the figures again from the column; the stream never revises a bucket, so that costs nothing in practice. */
  state(index: number, value: number | null, endMs: number, ends: readonly number[]): void {
    const was = this.values[index];
    this.values[index] = value;
    if (was !== null) {
      this.recount(ends);
      return;
    }
    if (value === null) return;
    this.count += 1;
    this.min = Math.min(this.min, value);
    this.max = Math.max(this.max, value);
    if (endMs >= this.lastEndMs) {
      this.last = value;
      this.lastEndMs = endMs;
    }
  }

  /** Adds an empty slot at `index`, for a time slot the chart gained. */
  open(index: number): void {
    if (index === this.values.length) this.values.push(null);
    else this.values.splice(index, 0, null);
  }

  private recount(ends: readonly number[]): void {
    [this.count, this.min, this.max, this.last, this.lastEndMs] = [0, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY, null, Number.NEGATIVE_INFINITY];
    this.values.forEach((value, index) => {
      if (value === null) return;
      this.count += 1;
      this.min = Math.min(this.min, value);
      this.max = Math.max(this.max, value);
      this.last = value;
      this.lastEndMs = ends[index];
    });
  }

  /** Its lowest and highest stated values, and how many, for the chart's own. */
  get extent(): { low: number; high: number; count: number } {
    return { low: this.min, high: this.max, count: this.count };
  }
}

/** What a metric stream's chart says in words: how many values it holds, the lowest and highest, the last stated and
 * the newest time slot's end. */
export interface ChartSummary {
  count: number;
  low: number;
  high: number;
  last: number;
  endMs: number;
}

/**
 * A metric stream's chart (the metric streams), kept up to date as the stream's rows arrive: each bucket at its end,
 * in hours of simtime from the run's start, one line per place along the metric's dimensions in order of its first
 * row, a single line in the first colour, what each line comes to, the chart's lowest and highest value, and the rows
 * the server marks as stating the whole run, which a page lists beside the chart and never draws. The mark, not the
 * span, says which those are: a run no longer than one window has a whole-run row exactly as wide as a window.
 *
 * The chart reads the stream's own rows, which the stream only ever appends to, and takes each row once: a page calls
 * `catchUp` as rows arrive, which costs a batch its own rows however long the run. A fresh attach brings a fresh array
 * of rows, and so a fresh chart.
 */
export class StreamChart {
  readonly descriptor: MetricDescriptor;
  /** Each time slot's end, in hours of simtime from the run's start, ascending: the chart's x column. */
  readonly x: number[] = [];
  readonly lines: StreamLine[] = [];
  /** The rows the server marks as stating the whole run. */
  readonly whole: MetricPoint[] = [];
  /** Each time slot's end in simtime, as `x` holds it in hours. */
  private readonly ends: number[] = [];
  private readonly slotOf = new Map<number, number>();
  private readonly lineOf = new Map<string, StreamLine>();
  /** The stream's own rows, which it only ever appends to. */
  private readonly rows: readonly MetricPoint[];
  private readonly dimensioned: boolean;
  private taken = 0;
  private count = 0;
  private low = Number.POSITIVE_INFINITY;
  private high = Number.NEGATIVE_INFINITY;
  private last: number | null = null;

  constructor(descriptor: MetricDescriptor, rows: readonly MetricPoint[]) {
    this.descriptor = descriptor;
    this.rows = rows;
    this.dimensioned = (descriptor.dimensions ?? []).length > 0;
  }

  /** Takes the rows the stream appended since the last call. */
  catchUp(): void {
    for (; this.taken < this.rows.length; this.taken += 1) this.take(this.rows[this.taken]);
  }

  /** What the chart says in words; null while it holds no value. */
  get summary(): ChartSummary | null {
    if (this.last === null) return null;
    return { count: this.count, low: this.low, high: this.high, last: this.last, endMs: this.ends[this.ends.length - 1] };
  }

  /** The chart as uPlot draws it, from the chart's own columns, so new values reach the plot without a copy. The x
   * axis reaches a quarter hour past the newest slot, so a live chart grows in steps and not on every slice; the value
   * axis names the clock the descriptor names. */
  spec(): ChartSpec {
    const { unit, clock, bucket, aggregation } = this.descriptor;
    const end = this.ends.length === 0 ? 0 : this.ends[this.ends.length - 1];
    const hours = Math.max(0.25, Math.ceil((end / HOUR_MS) / 0.25) * 0.25);
    return {
      x: this.x,
      series: this.lines,
      xRange: [0, hours],
      xTicks: hourTicks(hours),
      xTitle: 'simtime, hours',
      yTitle: unitTitle(unit, clock),
      yRange: yRangeOf(this.low, this.high),
      shape: bucket.kind === 'trailing' || aggregation !== 'additive' ? 'line' : 'steps',
      yWords: (value: number) => valueWords(unit, value, clock),
      yShort: (value: number) => shortValue(unit, value),
      xWords: hourWords,
    };
  }

  private take(row: MetricPoint): void {
    if (row.whole_run === true) {
      this.whole.push(row);
      return;
    }
    const index = this.slot(row.to_sim_ms);
    const line = this.line(this.dimensioned ? keyWords(row) : this.descriptor.label);
    const restated = line.values[index] !== null;
    line.state(index, row.value, row.to_sim_ms, this.ends);
    if (restated) {
      this.recount();
    } else if (row.value !== null) {
      this.count += 1;
      this.low = Math.min(this.low, row.value);
      this.high = Math.max(this.high, row.value);
    }
    if (row.value !== null) this.last = row.value;
  }

  /** The index of the time slot ending at `endMs`, opened in every line when it is new. Each bucket comes once it is
   * final, so a new slot is the newest; one that comes after a later one still goes in its place. */
  private slot(endMs: number): number {
    const known = this.slotOf.get(endMs);
    if (known !== undefined) return known;
    let index = this.ends.length;
    while (index > 0 && this.ends[index - 1] > endMs) index -= 1;
    this.ends.splice(index, 0, endMs);
    this.x.splice(index, 0, hoursOf(endMs));
    for (const line of this.lines) line.open(index);
    for (let later = index; later < this.ends.length; later += 1) this.slotOf.set(this.ends[later], later);
    return index;
  }

  /** The line of a place along the dimensions, made, with a slot for each the chart has, at its first row. */
  private line(label: string): StreamLine {
    const known = this.lineOf.get(label);
    if (known !== undefined) return known;
    const made = new StreamLine(label, this.dimensioned ? this.lines.length + 1 : 1, this.x.length);
    this.lines.push(made);
    this.lineOf.set(label, made);
    return made;
  }

  /** The chart's count, lowest and highest again from its lines, after a slot was stated twice. */
  private recount(): void {
    [this.count, this.low, this.high] = [0, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY];
    for (const line of this.lines) {
      const { low, high, count } = line.extent;
      this.count += count;
      this.low = Math.min(this.low, low);
      this.high = Math.max(this.high, high);
    }
  }
}

/** A value as a tick or a tooltip says it: compact, its unit as a sign ("1.2k", "45 s", "0.42", "3.1 m", "128/h"). */
export function shortValue(unit: MetricUnit, value: number): string {
  const compact = (n: number, digits = 1): string => {
    const abs = Math.abs(n);
    if (abs >= 1_000_000) return `${(n / 1_000_000).toLocaleString('en-US', { maximumFractionDigits: 1 })}M`;
    if (abs >= 10_000) return `${(n / 1000).toLocaleString('en-US', { maximumFractionDigits: 1 })}k`;
    return n.toLocaleString('en-US', { maximumFractionDigits: digits });
  };
  switch (unit) {
    case 'count': return compact(value);
    case 'duration_ms': return Math.abs(value) < 60_000 ? `${compact(value / 1000)} s` : spanWords(value);
    case 'distance_mm': return `${compact(value / 1000)} m`;
    case 'ratio': return compact(value, 2);
    case 'rate_per_hour': return `${compact(value)}/h`;
  }
}

/** The unit as a short sign beside a panel's title, a duration's or a rate's clock named, since the panel's ticks say
 * none; a count has none. */
export function unitSign(unit: MetricUnit, clock: Clock = 'sim'): string {
  switch (unit) {
    case 'count': return '';
    case 'duration_ms': return CLOCK_NAME[clock];
    case 'distance_mm': return 'm';
    case 'ratio': return 'ratio';
    case 'rate_per_hour': return `per ${HOUR_ON[clock]}`;
  }
}

/** An x in hours of simtime, said as a span from the run's start. */
export const hourWords = (x: number): string => spanWords(Math.round(x * HOUR_MS));
