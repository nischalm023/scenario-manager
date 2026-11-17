// A metric stream's chart as the run page keeps it (`StreamChart`), fed the stream's rows as they arrive: each bucket
// at its end in hours of simtime, the x axis growing by quarter hours and the value axis naming the descriptor's clock;
// one line per place along the dimensions in order of its first row; the whole-run rows kept apart, told by their mark
// alone; each row read once however many batches bring it; what each line and the chart come to; a slot that comes late
// or is stated twice put right; and a chart of more values than an engine takes as one call's arguments read without a
// throw. Then the value axis of a chart of series with ranges, the comparison's.

import { describe, expect, it } from 'vitest';
import type { MetricDescriptor, MetricPoint } from '../../protocol/metric_stream.ts';
import { seriesRange, StreamChart } from './charts.ts';

const HOUR_MS = 3_600_000;

const DESCRIPTOR: MetricDescriptor = {
  metric: 'picks_per_hour', label: 'Picks/hr (rate)', description: 'The slice\'s picks as a rate per simulated hour.',
  unit: 'rate_per_hour', clock: 'sim', aggregation: 'snapshot_gauge', source: 'scalars',
  bucket: { kind: 'trailing', window_sim_ms: 250_000, step_sim_ms: 10_000 },
};

const STATES: MetricDescriptor = {
  ...DESCRIPTOR, metric: 'da50_state_share', label: 'DA50 state, % of fleet time', unit: 'ratio', aggregation: 'ratio', source: 'journals',
  bucket: { kind: 'trailing', window_sim_ms: 900_000, step_sim_ms: 300_000 }, dimensions: [{ name: 'state', description: 'The state.' }],
};

const row = (to: number, value: number | null): MetricPoint => ({ from_sim_ms: to - 10_000, to_sim_ms: to, value });
const at = (to: number, state: string, value: number | null): MetricPoint => ({ from_sim_ms: to - 900_000, to_sim_ms: to, keys: [state], value });

/** A chart of `rows`, every one taken. */
function chartOf(descriptor: MetricDescriptor, rows: MetricPoint[]): StreamChart {
  const chart = new StreamChart(descriptor, rows);
  chart.catchUp();
  return chart;
}

describe('a metric stream chart', () => {
  it('charts each bucket at its end in hours of simtime, the axis growing by quarter hours', () => {
    const spec = chartOf(DESCRIPTOR, [row(10_000, 360), row(20_000, 720), row(1_800_000, 400)]).spec();
    expect(spec.x).toEqual([10_000 / HOUR_MS, 20_000 / HOUR_MS, 0.5]);
    expect(spec.series.map((one) => [one.label, one.key, one.values])).toEqual([['Picks/hr (rate)', 1, [360, 720, 400]]]);
    expect(spec.xRange).toEqual([0, 0.5]);
    expect(spec.yRange, 'from zero to a tenth above the highest').toEqual([0, 792]);
    expect(spec.shape).toBe('line');
    expect(spec.yTitle).toBe('per simulated hour');
    const empty = chartOf({ ...DESCRIPTOR, aggregation: 'additive', bucket: { kind: 'slice', width_sim_ms: 10_000 } }, []).spec();
    expect(empty.xRange).toEqual([0, 0.25]);
    expect(empty.yRange).toEqual([0, 1.1]);
    expect(empty.shape).toBe('steps');
    expect(empty.xTicks.map((tick) => tick.label)).toEqual(['0 s', '5 min', '10 min', '15 min']);
  });

  it('names the clock of a duration or a rate on its value axis and in its values, simtime or the wall clock', () => {
    const wallSlice: MetricDescriptor = {
      ...DESCRIPTOR, metric: 'wall_clock_per_slice', label: 'Wall clock/slice', unit: 'duration_ms', clock: 'wall', aggregation: 'additive',
      bucket: { kind: 'slice', width_sim_ms: 10_000 },
    };
    expect(chartOf(wallSlice, [row(10_000, 2_500)]).spec().yTitle).toBe('duration, wall clock');
    expect(chartOf({ ...wallSlice, clock: 'sim' }, []).spec().yTitle).toBe('duration, simtime');
    const wallRate = chartOf({ ...DESCRIPTOR, clock: 'wall' }, [row(10_000, 360)]).spec();
    expect([wallRate.yTitle, wallRate.yWords(360)]).toEqual(['per wall-clock hour', '360 per wall-clock hour']);
    expect(chartOf(DESCRIPTOR, []).spec().yWords(360)).toBe('360 per simulated hour');
  });

  it('draws one line per place in order of its first row, and keeps the whole-run rows apart', () => {
    const rows = [at(900_000, 'Waiting', 60), at(900_000, 'Laden travel', 40), at(1_200_000, 'Waiting', 50), at(1_200_000, 'Laden travel', 50),
      { from_sim_ms: 0, to_sim_ms: 1_500_000, keys: ['Waiting'], value: 55, whole_run: true },
      { from_sim_ms: 0, to_sim_ms: 1_500_000, keys: ['Laden travel'], value: 45, whole_run: true }];
    const chart = chartOf(STATES, rows);
    const spec = chart.spec();
    expect(spec.x).toEqual([0.25, 1_200_000 / HOUR_MS]);
    expect(spec.series.map((one) => [one.label, one.key, one.values])).toEqual([['Waiting', 1, [60, 50]], ['Laden travel', 2, [40, 50]]]);
    expect(chart.whole.map((one) => one.value)).toEqual([55, 45]);
  });

  it('tells a whole-run row by its mark alone, so a run one window long charts its window and lists its whole run', () => {
    const window = at(900_000, 'Waiting', 60);
    const whole: MetricPoint = { ...window, value: 61, whole_run: true };
    expect([window.from_sim_ms, window.to_sim_ms]).toEqual([whole.from_sim_ms, whole.to_sim_ms]);
    const chart = chartOf(STATES, [window, whole]);
    expect(chart.spec().series.map((one) => one.values)).toEqual([[60]]);
    expect(chart.whole).toEqual([whole]);
  });

  it('reads each row once, however many batches the stream appends it in, and hands its own columns to the plot', () => {
    const rows: MetricPoint[] = [];
    const read: number[] = [];
    const watched = new Proxy(rows, {
      get: (target, key, receiver) => {
        if (typeof key === 'string' && /^\d+$/.test(key)) read.push(Number(key));
        return Reflect.get(target, key, receiver);
      },
    });
    const chart = new StreamChart(DESCRIPTOR, watched);
    rows.push(row(10_000, 360), row(20_000, 720));
    chart.catchUp();
    rows.push(row(30_000, 500));
    chart.catchUp();
    chart.catchUp();
    expect(read, 'each row once').toEqual([0, 1, 2]);
    const spec = chart.spec();
    expect(spec.series[0].values).toEqual([360, 720, 500]);
    expect(spec.x, 'the chart\'s own column, never a copy').toBe(chart.x);
  });

  it('keeps what each line comes to, its last by simtime, and what the chart comes to for its sentence', () => {
    const chart = chartOf(STATES, [at(900_000, 'Waiting', 60), at(900_000, 'Laden travel', 40), at(1_200_000, 'Waiting', 50), at(1_200_000, 'Laden travel', null)]);
    expect(chart.lines.map((line) => line.stats)).toEqual([
      { last: 50, lastAt: 1_200_000 / HOUR_MS, min: 50, max: 60, count: 2 },
      { last: 40, lastAt: 0.25, min: 40, max: 40, count: 1 },
    ]);
    expect(chart.summary).toEqual({ count: 3, low: 40, high: 60, last: 50, endMs: 1_200_000 });
    expect(chart.spec().yRange).toEqual([0, 66]);
    expect(new StreamChart(STATES, []).summary).toBeNull();
  });

  it('puts a slot that comes after a later one in its place, and keeps the later slot its line\'s last', () => {
    const chart = chartOf(DESCRIPTOR, [row(10_000, 360), row(30_000, 500), row(20_000, 720)]);
    expect(chart.x).toEqual([10_000 / HOUR_MS, 20_000 / HOUR_MS, 30_000 / HOUR_MS]);
    expect(chart.lines[0].values).toEqual([360, 720, 500]);
    expect(chart.lines[0].stats).toEqual({ last: 500, lastAt: 30_000 / HOUR_MS, min: 360, max: 720, count: 3 });
  });

  it('keeps the later of two values stated for one slot, and counts what the line and the chart come to again', () => {
    const chart = chartOf(DESCRIPTOR, [row(10_000, 360), row(20_000, 720), row(10_000, 100)]);
    expect(chart.lines[0].values).toEqual([100, 720]);
    expect(chart.lines[0].stats).toEqual({ last: 720, lastAt: 20_000 / HOUR_MS, min: 100, max: 720, count: 2 });
    expect(chart.summary).toEqual({ count: 2, low: 100, high: 720, last: 100, endMs: 20_000 });
  });

  it('reads a chart of more values than an engine takes as one call\'s arguments without a throw', () => {
    const rows = Array.from({ length: 150_000 }, (_, n) => row((n + 1) * 10_000, n % 1_000));
    const chart = chartOf(DESCRIPTOR, rows);
    expect(chart.summary?.count).toBe(150_000);
    expect([chart.summary?.low, chart.summary?.high, chart.lines[0].stats?.max]).toEqual([0, 999, 999]);
    expect(chart.spec().yRange[1]).toBeCloseTo(1_098.9);
  });
});

describe("a chart's value axis", () => {
  it('holds every value and both ends of every range, from zero or the lowest below it', () => {
    expect(seriesRange([{ label: 'a', key: 1, values: [-5, 10, null], ranges: [{ at: 0, low: -8, high: 12 }] }])).toEqual([-8, 14]);
    expect(seriesRange([{ label: 'a', key: 1, values: [3, 4] }])).toEqual([0, 4.4]);
  });
});
