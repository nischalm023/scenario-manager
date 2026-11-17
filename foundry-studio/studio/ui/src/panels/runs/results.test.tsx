// Spec 08 sections 3.9, 3.10, 3.12 and 5 on an ended run's results and a comparison, against answers the test gives
// (brief I1.10): values in their units; a stub attempt's tiles, tables, picture and metric panels all described by its
// sentence, said once a section; every time in its `.clock`; the inputs copied as the API's text; the list's headline
// column; and a comparison whose heading warns of a difference outside the experiment. The last block holds a test for
// each finding of Astra's review of the results (2026-10-20) the panel answers; a canvas draws nothing here, so the
// chart's stand-in keeps the spec it was handed for those tests to read.

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { CompareReply, MetricRow, MetricsReply, RunInputs } from '../../protocol/results.ts';
import type { WorkloadRead } from '../../protocol/runner.ts';
import type { RunDetail, RunSummary } from '../../protocol/runs.ts';
import { ShellContext } from '../../shell/context.ts';
import { ServicesContext, type NamedSite, type ShellServices } from '../../shell/panels.tsx';
import { shellOf } from '../../testing/render.tsx';
import { endWords, type ChartSpec } from './charts.ts';
import { Comparison } from './Comparison.tsx';
import { Inputs } from './Inputs.tsx';
import { spanWords, valueWords } from './resultWords.ts';
import { OpenRun } from './OpenRun.tsx';
import { RunResults } from './RunResults.tsx';
import { RunsPanel } from './RunsPanel.tsx';

const charted = vi.hoisted(() => [] as { name: string; spec: ChartSpec }[]);

vi.mock('./Chart.tsx', async () => {
  const { createElement } = await import('react');
  return {
    Chart: ({ name, describedBy, spec }: { name: string; describedBy?: string; spec: ChartSpec }) => {
      charted.push({ name, spec });
      return createElement('div', { className: 'chart__canvas', role: 'img', 'aria-label': name, 'aria-describedby': describedBy });
    },
  };
});

/** The viewer, which draws nothing here: a stand-in that holds no frame. */
vi.mock('crucible-viewer', () => {
  const standIn = new Proxy({}, { get: (_, name) => (name === 'then' ? undefined : name === 'held_window' ? () => [] : () => undefined) });
  return { openFloorViewer: async () => standIn, readPalette: () => ({}), attachFrames: () => ({ close: () => undefined }) };
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  charted.length = 0;
});

const RUN = '01KTJ153M7D109TG4G7BBVCZYW';
const OTHER = '01KM5BMQJ3HWJQ2RG6X9TSC526';
const SITE = 'customer/demo/site/lshape-6';
const STUB = 'stub runner: canned results; no simulation ran';
const answer = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });
const missing = (path: string) => answer({ type: 'not-found', title: 'Nothing', status: 404, detail: `No route ${path}.` }, 404);

function inShell(node: React.ReactNode, spoken: string[] = [], address = `/runs/${RUN}`) {
  return <ShellContext.Provider value={shellOf(spoken, address)}>{node}</ShellContext.Provider>;
}

const WHOLE = { from_sim_ms: 0, to_sim_ms: 10_830_000 };

function metric(name: string, more: Partial<MetricRow> = {}): MetricRow {
  return { metric: name, label: name, description: '', unit: 'count', aggregation: 'additive', value: 317, window: WHOLE, basis: 'window_difference', ...more };
}

function summary(id: string, more: Partial<RunSummary> = {}): RunSummary {
  return {
    id, ref: `run:${id}`, submission: '01KQ7Z3F0V5Q8M2R1T6W9X4Y7B', position: 1, of: 1, label: 'Peak hour',
    scenario: { id: '01KK8AJ7RYA6GNNQ3QTRQCW7Z8', version: 1, name: 'Peak hour' }, site: SITE, variant: [], seed: 0,
    duration_sim_ms: 14_400_000, report_window: { from_sim_ms: 0, to_sim_ms: null, before_name: 'warm-up' },
    state: 'finished', attempt: 1, progress: { sim_ms: 10_830_000, values: [] }, started_at: '2026-10-19T16:58:00Z',
    ended_at: '2026-10-19T16:59:00Z', elapsed_wall_ms: 60_000, ended_by: 'simulator', stop_reason: 'SimTimeLimit',
    stop_detail: STUB, recording: { stored: true }, findings: [], ...more,
  } as RunSummary;
}

function finished(): RunDetail {
  return {
    ...summary(RUN),
    cut: { unit: SITE, requested: '@main', branch: 'main' }, reads: { [`${SITE}/flow`]: 1, [`${SITE}/map`]: 1 },
    runner: { image: 'local:simrunner@c331dee0f', tag: 'c331dee0f', chosen: 'default' },
    expected: { compile_sha256: null, world_sha256: 'a'.repeat(64), compiler: 'none', workload_sha256: 'b'.repeat(64) },
    submitted_by: 'Developer',
    attempts: [{
      n: 1, state: 'finished', requested_at: '2026-10-19T16:57:00Z', job: `run-${RUN.toLowerCase()}-1`, findings: [],
      compile: { digest: null, compiler: 'none' }, exact: true, stub: true, stalled: false,
    }],
  } as RunDetail;
}

const COUNTED = metric('counted_a', { label: 'Counted A' });
const TIMED = metric('timed_b', { label: 'Timed B', unit: 'duration_ms', value: 2_853_000 });
const GAUGE = metric('gauge_c', { label: 'Gauge C', unit: 'ratio', aggregation: 'snapshot_gauge', value: 65, basis: 'end_block' });

const INPUTS: RunInputs = {
  run: RUN, complete: false, text: 'Inputs of run 01KTJ153\nOrders per hour: 350 orders an hour (varied by the sweep, value 4 of 5)',
  rows: [
    { section: 'scenario', field: '/workload/generator/orders_per_hour', label: 'Orders per hour', value: '350 orders an hour', canonical: '350', from: 'varied by the sweep, value 4 of 5', held: '300 orders an hour', varied: true, number: 350 },
    { section: 'scenario', field: '/run_controls/duration_sim_ms', label: 'Duration', value: '4 hours of simtime', canonical: '14400000', from: 'Peak hour, version 1', varied: false, clock: 'sim', number: 4 },
    { section: 'provenance', field: 'compiler', label: 'Compiler', value: 'none', canonical: '"none"', from: 'the runner', varied: false },
  ],
};

function results(fetched: string[] = []) {
  return vi.fn(async (path: string) => {
    fetched.push(path);
    if (path === `/api/runs/${RUN}/metrics?attempt=1`) {
      const reply: MetricsReply = { attempt: 1, window: WHOLE, items: [COUNTED, TIMED, GAUGE], headlines: [COUNTED.metric, TIMED.metric, GAUGE.metric] };
      return answer(reply);
    }
    if (path === `/api/runs/${RUN}/inputs`) return answer(INPUTS);
    if (path === `/api/runs/${RUN}/watch`) return answer({ type: 'not-streamed', title: 'Not streamed', status: 409, detail: 'No live stream.' }, 409);
    return missing(path);
  });
}

/** Every text that reads as a time and sits outside a `.clock`. */
function loose(container: HTMLElement): string[] {
  const times: string[] = [];
  const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    if (/\d+ (s|min|h)\b|\d\d:\d\d/.test(node.textContent ?? '') && node.parentElement?.closest('.clock') === null) times.push(node.textContent ?? '');
  }
  return times;
}

describe('the result words', () => {
  it('says values and spans of simtime in their units', () => {
    expect([spanWords(10_830_000), spanWords(0), spanWords(2_853_000)]).toEqual(['3 h 30 s', '0 s', '47 min 33 s']);
    expect([valueWords('count', 1_133_941), valueWords('duration_ms', 2_853_000), valueWords('duration_ms', 13_133.7)])
      .toEqual(['1,133,941', '47 min 33 s', '13.1 s']);
    expect([valueWords('distance_mm', 16_130_817), valueWords('ratio', 0.4321), valueWords('count', null)]).toEqual(['16,130.8 m', '0.43', 'no value']);
  });
});

describe("an ended run's results", () => {
  it("describes every stub number by the stub's sentence, said once a section, and keeps every time in its clock", async () => {
    const answered = results();
    const ticket = { run: RUN, attempt: 1, token: 't', expires_at: '2026-10-19T17:00:00Z', url: 'ws://s/streams', frames_url: 'ws://s/frames', metrics_url: 'ws://s/metrics' };
    vi.stubGlobal('fetch', vi.fn(async (path: string) => (path === `/api/runs/${RUN}/watch` ? answer(ticket) : answered(path))));
    // The picture's stage needs these; the metric panels' sockets are not under test.
    vi.stubGlobal('requestAnimationFrame', () => 0);
    vi.stubGlobal('cancelAnimationFrame', () => undefined);
    vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
    vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
    vi.stubGlobal('WebSocket', undefined);
    const { container } = render(inShell(<OpenRun run={finished()} reread={() => {}} />));
    await waitFor(() => expect(container.querySelectorAll('.metric-tile')).toHaveLength(3));
    const tiles = [...container.querySelectorAll('.metric-tile')];
    expect(tiles.map((tile) => tile.querySelector('.metric-tile__label')?.textContent)).toEqual(['Counted A', 'Timed B', 'Gauge C']);
    expect(tiles.every((tile) => tile.getAttribute('aria-describedby') === 'run-stub-summary')).toBe(true);
    expect(document.getElementById('run-stub-summary')?.textContent).toBe(STUB);
    const tables = [...container.querySelectorAll('table.census, details table.list-table')];
    expect(tables.length).toBeGreaterThanOrEqual(1);
    expect(tables.filter((table) => table.closest('.run-inputs') === null).every((table) => table.getAttribute('aria-describedby')?.includes('run-stub'))).toBe(true);
    expect(container.querySelector('.run-card--player .run-playback'), 'the playback, from the streaming server').not.toBeNull();
    await waitFor(() => expect(container.querySelector('.run-playback .worldview-mount__stage')?.getAttribute('aria-describedby')?.split(' ')).toContain('run-stop-detail'));
    const panels = [...container.querySelectorAll('figure.mpanel')];
    expect(panels.length, 'the metric panels, from the same recording').toBeGreaterThan(0);
    expect(panels.every((panel) => panel.getAttribute('aria-describedby') === 'run-stop-detail')).toBe(true);
    expect(document.getElementById('run-stop-detail')?.textContent).toBe(STUB);
    expect(screen.getByRole('link', { name: 'Recording (.worldview), canned by the stub runner' }).getAttribute('aria-describedby')).toBe('run-stop-detail');
    await waitFor(() => expect(screen.getByRole('button', { name: 'Copy the inputs as text' })).not.toBeNull());
    expect(loose(container)).toEqual([]);
    expect(container.querySelector('[aria-live], [role="log"], [role="status"], [role="alert"]')).toBeNull();
  });

  it("copies the API's own text of the inputs and says so through the announcer", async () => {
    vi.stubGlobal('fetch', results());
    const written: string[] = [];
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText: async (text: string) => void written.push(text) } });
    const spoken: string[] = [];
    render(inShell(<Inputs run={RUN} attempt={1} manifestIn />, spoken));
    const copy = await screen.findByRole('button', { name: 'Copy the inputs as text' });
    expect(screen.getAllByText('Orders per hour').length, 'the fact and its row in the table').toBe(2);
    expect(screen.queryByText('Compiler'), 'provenance stays in its own list').toBeNull();
    fireEvent.click(copy);
    await waitFor(() => expect(written).toEqual([INPUTS.text]));
    expect(spoken).toEqual(['Copied the inputs of run 01KTJ153 as text.']);
  });

  it("lists a Status total the API answers over the whole run after a warm-up as the whole run's", async () => {
    const report = { from_sim_ms: 1_800_000, to_sim_ms: 10_830_000 };
    const counted = { ...COUNTED, window: report };
    const picks = metric('run_total_picks', { label: 'Picks, whole run', value: 1_234, basis: 'end_block' });
    const reply: MetricsReply = { attempt: 1, window: report, items: [counted, picks], headlines: [counted.metric, picks.metric] };
    vi.stubGlobal('fetch', vi.fn(async (path: string) => (path === `/api/runs/${RUN}/metrics?attempt=1` ? answer(reply) : missing(path))));
    const run = { ...finished(), report_window: { from_sim_ms: 1_800_000, to_sim_ms: null, before_name: 'warm-up' } } as RunDetail;
    const { container } = render(inShell(<RunResults run={run} />));
    await waitFor(() => expect(container.querySelectorAll('.metric-tile')).toHaveLength(1));
    expect(container.querySelector('.metric-tile__label')?.textContent).toBe('Counted A');
    expect(screen.getByText(/is a whole-run value/).textContent).toContain('Picks, whole run is a whole-run value, including the warm-up');
    const row = [...container.querySelectorAll('table.all-metrics tr')].find((tr) => tr.textContent?.includes('run_total_picks'));
    expect(row?.querySelector('td.num')?.textContent).toBe('1,234');
    expect(row?.textContent).toContain('whole run, including the warm-up');
  });

  it('states a stored duration in the clock its row names, on the summary and in the comparison', async () => {
    const wall = metric('run_wall_clock', { label: 'Wall clock, whole run', unit: 'duration_ms', clock: 'wall', aggregation: 'interval', value: 600, basis: 'end_block' });
    const timed = { ...TIMED, clock: 'sim' as const };
    const reply: MetricsReply = { attempt: 1, window: WHOLE, items: [timed, wall], headlines: [timed.metric, wall.metric] };
    vi.stubGlobal('fetch', vi.fn(async (path: string) => (path === `/api/runs/${RUN}/metrics?attempt=1` ? answer(reply) : missing(path))));
    const { container, unmount } = render(inShell(<RunResults run={finished()} />));
    await waitFor(() => expect(container.querySelectorAll('.metric-tile')).toHaveLength(2));
    const clockOf = (cell: Element | null | undefined) => {
      const clocked = cell?.querySelector('.clock');
      return [clocked?.getAttribute('data-clock'), clocked?.textContent];
    };
    const tiles = [...container.querySelectorAll('.metric-tile__value')];
    expect(tiles.map(clockOf)).toEqual([['sim', '47 min 33 s simtime'], ['wall', '0.6 s']]);
    const cell = (name: string) => [...container.querySelectorAll('table.all-metrics tr')].find((tr) => tr.textContent?.includes(name))?.querySelector('td.num');
    expect([clockOf(cell('timed_b')), clockOf(cell('run_wall_clock'))]).toEqual([['sim', '47 min 33 s simtime'], ['wall', '0.6 s']]);
    expect(loose(container)).toEqual([]);
    unmount();

    const compared: CompareReply = {
      runs: [
        { id: RUN, label: 'Peak hour, 200 orders per hour', state: 'finished', attempt: 1, stub: true },
        { id: OTHER, label: 'Peak hour, 250 orders per hour', state: 'finished', attempt: 1, stub: true },
      ],
      differs: [{ field: '/workload/generator/orders_per_hour', section: 'scenario', label: 'Orders per hour', by_experiment: true, warns: false, changed: [],
        values: [{ run: RUN, value: '200 orders an hour', number: 200 }, { run: OTHER, value: '250 orders an hour', number: 250 }] }],
      same: [], not_compared: [], reference: RUN,
      metrics: [{ metric: wall.metric, label: wall.label, unit: 'duration_ms', clock: 'wall', aggregation: 'interval', comparable: true,
        values: [{ run: RUN, value: 600, window: WHOLE }, { run: OTHER, value: 900, window: WHOLE }] }],
    };
    vi.stubGlobal('fetch', vi.fn(async (path: string) => (path.startsWith('/api/runs/compare?') ? answer(compared) : missing(path))));
    const shown = render(inShell(<Comparison ids={[RUN, OTHER]} />, [], '/runs/compare'));
    await waitFor(() => expect(charted.length).toBeGreaterThan(0));
    expect(charted.at(-1)?.spec.yTitle).toBe('duration, wall clock');
    const clocks = [...shown.container.querySelectorAll('table .clock')].filter((one) => /\d s$/.test(one.textContent ?? ''));
    expect(clocks.length).toBeGreaterThan(0);
    expect(clocks.every((one) => one.getAttribute('data-clock') === 'wall' && one.querySelector('.clock__tag') === null)).toBe(true);
  });
});

describe('the list and the comparison', () => {
  it("heads its columns in plain words, Duration for the wall-clock span, and shows no key-numbers column", async () => {
    const headlined = summary(RUN, { headline: COUNTED });
    const other = summary(OTHER, { headline: { ...COUNTED, value: 290, window: { from_sim_ms: 1_800_000, to_sim_ms: 10_830_000 } } });
    vi.stubGlobal('fetch', vi.fn(async (path: string) => {
      if (path === `/api/runs?site=${encodeURIComponent(SITE)}`) return answer({ items: [headlined, headlined, other].map((one, n) => ({ ...one, id: `${one.id.slice(0, 25)}${n}` })) });
      if (path === '/api/runs/queue') return answer({ limit: 5, going: [], waiting: [] });
      return missing(path);
    }));
    const { container } = render(inShell(<RunsPanel site={SITE} />, [], `/runs?site=${encodeURIComponent(SITE)}`));
    await waitFor(() => expect(container.querySelector('table.runs-table')).not.toBeNull());
    const heads = [...container.querySelectorAll('table.runs-table thead th')].map((head) => head.textContent);
    expect(heads.join(' ')).toContain('Duration');
    expect(heads).not.toContain('Key numbers');
    expect(loose(container)).toEqual([]);
  });

  it('warns in its heading of a difference outside the experiment and lists the run that did not finish', async () => {
    const reply: CompareReply = {
      runs: [
        { id: RUN, label: 'Peak hour, 200 orders per hour', state: 'finished', attempt: 1, stub: true },
        { id: OTHER, label: 'Peak hour, 250 orders per hour', state: 'finished', attempt: 1, stub: true },
        { id: '01KZAVPQNES65HV4EZADPCS7E8', label: 'Peak hour', state: 'failed', attempt: 1, stub: false },
      ],
      differs: [
        { field: '/workload/generator/orders_per_hour', section: 'scenario', label: 'Orders per hour', by_experiment: true, warns: false, changed: [],
          values: [{ run: RUN, value: '200 orders an hour', number: 200 }, { run: OTHER, value: '250 orders an hour', number: 250 }] },
        { field: `read:${SITE}/flow`, section: 'provenance', label: `${SITE}/flow`, by_experiment: false, warns: true, changed: ['Pick time'],
          values: [{ run: RUN, value: 'changeset 2' }, { run: OTHER, value: 'changeset 3' }] },
      ],
      same: [{ field: '/run_controls/duration_sim_ms', section: 'scenario', label: 'Duration', value: '4 hours of simtime', clock: 'sim' }],
      not_compared: [{ run: '01KZAVPQNES65HV4EZADPCS7E8', state: 'failed', stop_reason: 'the watchdog fired' }],
      reference: RUN,
      metrics: [{ metric: COUNTED.metric, label: COUNTED.label, unit: 'count', aggregation: 'additive', comparable: true,
        values: [{ run: RUN, value: 300, window: WHOLE }, { run: OTHER, value: 340, window: WHOLE }] }],
    };
    vi.stubGlobal('fetch', vi.fn(async (path: string) => (path.startsWith('/api/runs/compare?') ? answer(reply) : missing(path))));
    const { container } = render(inShell(<Comparison ids={[RUN, OTHER, '01KZAVPQNES65HV4EZADPCS7E8']} />, [], '/runs/compare'));
    await waitFor(() => expect(screen.getByRole('heading', { level: 3, name: 'Comparison' })).not.toBeNull());
    expect(screen.getByRole('heading', { level: 4, name: /What differs: results cannot be read as the effect of the varied input alone/ })).not.toBeNull();
    expect(container.textContent).toContain('it changed Pick time');
    expect(screen.getByRole('img').getAttribute('aria-label')).toBe(
      'Counted A against orders per hour. Counted A, over 0 s to 3 h 30 s of simtime: 300 at 200 orders an hour and 340 at 250 orders an hour',
    );
    const rows = [...container.querySelectorAll('table.list-table tbody tr')].map((row) => [...row.children].map((cell) => cell.textContent));
    expect(rows).toEqual([
      ['Run 1, Peak hour, 200 orders per hour (reference)', '200 orders an hour', '300', '0'],
      ['Run 2, Peak hour, 250 orders per hour', '250 orders an hour', '340', '+40'],
    ]);
    expect(container.textContent).toContain('changeset 2 on run 1 and changeset 3 on run 2');
    expect(container.textContent).toContain('failed, the watchdog fired');
    expect(document.getElementById('compare-stub')?.textContent).toBe(STUB);
    expect(loose(container)).toEqual([]);
  });

  it("tables each value of a varied input with its repeats' mean, range and n when seeds repeat", async () => {
    const runs = ['01KTJ153M7D109TG4G7BBVCZY1', '01KTJ153M7D109TG4G7BBVCZY2', '01KTJ153M7D109TG4G7BBVCZY3', '01KTJ153M7D109TG4G7BBVCZY4'];
    const rates = [200, 200, 250, 250];
    const counts = [300, 310, 340, 360];
    const reply: CompareReply = {
      runs: runs.map((id, at) => ({ id, label: `Peak hour, ${rates[at]} orders per hour, seed ${at % 2}`, state: 'finished', attempt: 1, stub: false })),
      differs: [
        { field: '/workload/generator/orders_per_hour', section: 'scenario', label: 'Orders per hour', by_experiment: true, warns: false, changed: [],
          values: runs.map((run, at) => ({ run, value: `${rates[at]} orders an hour`, number: rates[at] })) },
        { field: '/run_controls/seed', section: 'scenario', label: 'Simulation seed', by_experiment: true, warns: false, changed: [],
          values: runs.map((run, at) => ({ run, value: String(at % 2), number: at % 2 })) },
      ],
      same: [], not_compared: [], reference: runs[0],
      metrics: [{ metric: COUNTED.metric, label: COUNTED.label, unit: 'count', aggregation: 'additive', comparable: true, values: runs.map((run, at) => ({ run, value: counts[at], window: WHOLE })) }],
    };
    vi.stubGlobal('fetch', vi.fn(async (path: string) => (path.startsWith('/api/runs/compare?') ? answer(reply) : missing(path))));
    const { container } = render(inShell(<Comparison ids={runs} />, [], '/runs/compare'));
    await waitFor(() => expect(screen.getByRole('heading', { level: 3, name: 'Comparison' })).not.toBeNull());
    expect(screen.getByRole('img').getAttribute('aria-label')).toBe(
      'Counted A against orders per hour. Counted A, over 0 s to 3 h 30 s of simtime: 305 at 200 orders an hour (the mean of 2, from 300 to 310) and 350 at 250 orders an hour (the mean of 2, from 340 to 360)',
    );
    const rows = [...container.querySelectorAll('table.list-table tbody tr')].map((row) => [...row.children].map((cell) => cell.textContent));
    expect(rows).toEqual([
      ['200 orders an hour (2 runs), reference', '305', '300 to 310, n 2', '0'], ['250 orders an hour (2 runs)', '350', '340 to 360, n 2', '+45'],
    ]);
  });
});

const ELSEWHERE = 'customer/demo/site/rect-2';
const THIRD = '01KZAVPQNES65HV4EZADPCS7E8';

/** A comparison of `runs` by design in one input, whose values `values` say, each run's counts as given. */
function sweep(runs: string[], values: { value: string; number: number }[], counts: number[][], more: Partial<CompareReply> = {}): CompareReply {
  return {
    runs: runs.map((id, at) => ({ id, label: `Peak hour, ${values[at].value}`, state: 'finished', attempt: 1, stub: false })),
    differs: [{
      field: '/workload/generator/orders_per_hour', section: 'scenario', label: 'Orders per hour', by_experiment: true, warns: false, changed: [],
      unit: 'orders an hour', values: runs.map((run, at) => ({ run, ...values[at] })),
    }],
    same: [], not_compared: [], reference: runs[0],
    metrics: counts.map((series, at) => ({
      metric: `counted_${at}`, label: `Counted ${at}`, unit: 'count', aggregation: 'additive', comparable: true,
      values: runs.map((run, n) => ({ run, value: series[n], window: WHOLE })),
    })),
    ...more,
  };
}

function services(named: (NamedSite | null)[]): ShellServices {
  return {
    announce: () => undefined, sendMessage: async () => undefined, browserTab: () => 'tab', focusInPanel: () => false,
    focusPanelTitle: () => undefined, offersLinkForShow: () => false, subscribe: () => () => undefined,
    nameSite: (site) => void named.push(site),
  };
}

/** The runs ticked for comparison in the list, by id. */
const ticked = (container: HTMLElement): string[] =>
  [...container.querySelectorAll('table.runs-table tbody tr')]
    .filter((row) => row.querySelector<HTMLInputElement>('input[type="checkbox"]')?.checked)
    .map((row) => row.getAttribute('data-ref')?.slice('run:'.length) ?? '');

describe("Astra's review of the results, the panel's findings", () => {
  /** F1 as the lead amended it: with repeats, the picker offers the varied input's values, each with its n, and the
   * caption names the reference value and its n; the changes stay mean against mean. Also the numeric time input's
   * unit and clock: group labels and row headings say "15 minutes of simtime" in a simtime clock, the axis title names
   * the unit and its ticks the numbers; and F8: each series carries its ranges and the axis reaches their ends. */
  it("offers values as the reference when seeds repeat, keeps a time input's unit and clock, and ranges every series", async () => {
    const runs = ['01KTJ153M7D109TG4G7BBVCZY1', '01KTJ153M7D109TG4G7BBVCZY2', '01KTJ153M7D109TG4G7BBVCZY3', '01KTJ153M7D109TG4G7BBVCZY4'];
    const waves = [15, 15, 30, 30].map((number) => ({ value: `${number} minutes of simtime`, number }));
    const reply = sweep(runs, waves, [[300, 310, 340, 360], [10, 20, 30, 50]], { reference: runs[1] });
    reply.differs[0] = { ...reply.differs[0], field: '/workload/generator/wave_every_sim_ms', label: 'Wave interval', clock: 'sim', unit: 'minutes of simtime' };
    reply.differs.push({ field: '/run_controls/seed', section: 'scenario', label: 'Simulation seed', by_experiment: true, warns: false, changed: [],
      values: runs.map((run, at) => ({ run, value: String(at % 2), number: at % 2 })) });
    vi.stubGlobal('fetch', vi.fn(async (path: string) => (path.startsWith('/api/runs/compare?') ? answer(reply) : missing(path))));
    const { container } = render(inShell(<Comparison ids={runs} reference={runs[1]} />, [], '/runs/compare'));
    await waitFor(() => expect(screen.getByRole('heading', { level: 3, name: 'Comparison' })).not.toBeNull());
    const picker = screen.getByLabelText('Measure changes against') as HTMLSelectElement;
    expect([...picker.options].map((option) => option.textContent)).toEqual(['15 minutes of simtime, 2 runs', '30 minutes of simtime, 2 runs']);
    expect(picker.value).toBe(runs[0]);
    expect(container.querySelector('table.list-table caption')?.textContent).toContain("the reference value's mean, 15 minutes of simtime (n 2)");
    const heads = [...container.querySelectorAll('table.list-table tbody th')];
    expect(heads.map((head) => head.querySelector('.clock[data-clock="sim"]')?.textContent)).toEqual(['15 minutes of simtime', '30 minutes of simtime']);
    expect([...container.querySelectorAll('table.list-table tbody tr')].map((row) => row.children[3].textContent)).toEqual(['0', '+45']);
    const { name, spec } = charted.at(-1)!;
    expect(spec.xTicks.map((tick) => tick.label)).toEqual(['15', '30']);
    expect(spec.xTitle).toBe('wave interval, minutes of simtime');
    expect(name).toContain('305 at 15 minutes of simtime (the mean of 2, from 300 to 310)');
    expect(spec.series.map((one) => one.ranges)).toEqual([
      [{ at: 15, low: 300, high: 310 }, { at: 30, low: 340, high: 360 }], [{ at: 15, low: 10, high: 20 }, { at: 30, low: 30, high: 50 }],
    ]);
    expect(spec.yRange[1], 'the axis holds every range').toBeGreaterThan(360);
  });

  /** F7: each metric says the window its values cover, in its column heading and in the chart's sentence, a gauge
   * its last reading; F12: a differing changeset links its scope's History route at that changeset. */
  it("says each metric's window and links a differing changeset's history", async () => {
    const reply = sweep([RUN, OTHER], [{ value: '200 orders an hour', number: 200 }, { value: '250 orders an hour', number: 250 }], [[300, 340]]);
    reply.metrics.push({ metric: GAUGE.metric, label: GAUGE.label, unit: 'ratio', aggregation: 'snapshot_gauge', comparable: true,
      values: [{ run: RUN, value: 65, window: WHOLE }, { run: OTHER, value: 70, window: WHOLE }] });
    reply.differs.push({ field: `read:${SITE}/flow`, section: 'provenance', label: `${SITE}/flow`, by_experiment: false, warns: true, changed: [],
      values: [{ run: RUN, value: 'changeset 2', changeset: 2 }, { run: OTHER, value: 'changeset 3', changeset: 3 }] });
    vi.stubGlobal('fetch', vi.fn(async (path: string) => (path.startsWith('/api/runs/compare?') ? answer(reply) : missing(path))));
    const { container } = render(inShell(<Comparison ids={[RUN, OTHER]} />, [], '/runs/compare'));
    await waitFor(() => expect(screen.getByRole('heading', { level: 3, name: 'Comparison' })).not.toBeNull());
    const heads = [...container.querySelectorAll('table.list-table thead th')].map((head) => head.textContent ?? '');
    expect(heads.find((head) => head.startsWith('Counted 0'))).toContain('over 0 s to 3 h 30 s simtime');
    expect(heads.find((head) => head.startsWith('Gauge C'))).toContain('at 3 h 30 s simtime, its last reading');
    expect(charted.at(-1)?.name).toContain('Counted 0, over 0 s to 3 h 30 s of simtime: 300 at 200 orders an hour');
    expect(screen.getByRole('link', { name: 'changeset 2' }).getAttribute('href')).toBe(`/w/${SITE}/flow@2?panel=history`);
    expect(screen.getByRole('link', { name: 'changeset 3' }).getAttribute('href')).toBe(`/w/${SITE}/flow@3?panel=history`);
    expect(loose(container)).toEqual([]);
  });

  /** F9: a line's end names its metric beside its value, so two lines are told apart without their colours. */
  it("labels a line's end with its metric", () => {
    expect(endWords({ label: 'Counted A', values: [107], key: 2 }, 107, (value) => valueWords('count', value))).toBe('Counted A: 107');
  });

  /** F10: the list and the chat column follow the run the comparison measures against as the API names it, here
   * another site's, not the first id the address names. */
  it('follows the run the comparison measures against for the list and the chat column', async () => {
    const named: (NamedSite | null)[] = [];
    const fetched: string[] = [];
    const reply = sweep([RUN, OTHER], [{ value: '200 orders an hour', number: 200 }, { value: '250 orders an hour', number: 250 }], [[300, 340]], { reference: OTHER });
    vi.stubGlobal('fetch', vi.fn(async (path: string) => {
      fetched.push(path);
      if (path.startsWith('/api/runs/compare?')) return answer(reply);
      if (path === `/api/runs/${OTHER}`) return answer({ ...finished(), id: OTHER, site: ELSEWHERE, reads: { [`${ELSEWHERE}/map`]: 4 } });
      if (path === `/api/runs?site=${encodeURIComponent(ELSEWHERE)}`) return answer({ items: [summary(OTHER, { site: ELSEWHERE })] });
      if (path === '/api/runs/queue') return answer({ limit: 5, going: [], waiting: [] });
      return missing(path);
    }));
    render(
      <ServicesContext.Provider value={services(named)}>
        {inShell(<RunsPanel compare={{ ids: [RUN, OTHER] }} />, [], `/runs/compare?ids=${RUN},${OTHER}`)}
      </ServicesContext.Provider>,
    );
    await waitFor(() => expect(named.at(-1)).toEqual({ site: ELSEWHERE, seen: [{ scope: `${ELSEWHERE}/map`, seq: 4 }] }));
    await waitFor(() => expect(fetched).toContain(`/api/runs?site=${encodeURIComponent(ELSEWHERE)}`));
    expect(fetched).not.toContain(`/api/runs/${RUN}`);
  });

  /** F11: the ticked runs follow the address's comparison when it changes, as Back changes it, since the shell keeps
   * one Runs panel across the views. */
  it("ticks the runs the address compares whenever the address changes them", async () => {
    const reply = (ids: string[]) => sweep(ids, ids.map((_, at) => ({ value: `${200 + at * 50} orders an hour`, number: 200 + at * 50 })), [ids.map(() => 300)]);
    vi.stubGlobal('fetch', vi.fn(async (path: string) => {
      if (path.startsWith('/api/runs/compare?')) return answer(reply(decodeURIComponent(/ids=([^&]+)/.exec(path)?.[1] ?? '').split(',')));
      if (path === `/api/runs/${RUN}`) return answer(finished());
      if (path === `/api/runs?site=${encodeURIComponent(SITE)}`) return answer({ items: [RUN, OTHER, THIRD].map((id) => summary(id)) });
      if (path === '/api/runs/queue') return answer({ limit: 5, going: [], waiting: [] });
      return missing(path);
    }));
    const { container, rerender } = render(inShell(<RunsPanel compare={{ ids: [RUN, OTHER] }} />, [], '/runs/compare'));
    await waitFor(() => expect(ticked(container)).toEqual([RUN, OTHER]));
    rerender(inShell(<RunsPanel compare={{ ids: [RUN, THIRD] }} />, [], '/runs/compare'));
    await waitFor(() => expect(ticked(container)).toEqual([RUN, THIRD]));
  });

  /** F13's panel half, since ruling D192 dropped the list's headline column: a stub run's status cell says the stub's
   * sentence, and the list no longer asks for headlines with the address's `highlights`. */
  it("says a stub run's ending sentence in its status cell and asks for the list without the address's highlights", async () => {
    vi.stubGlobal('fetch', vi.fn(async (path: string) => {
      if (path === `/api/runs?site=${encodeURIComponent(SITE)}`) return answer({ items: [summary(RUN, { headline: COUNTED }), summary(OTHER)] });
      if (path === '/api/runs/queue') return answer({ limit: 5, going: [], waiting: [] });
      return missing(path);
    }));
    const { container } = render(inShell(<RunsPanel site={SITE} highlights={412} />, [], `/runs?site=${encodeURIComponent(SITE)}&highlights=412`));
    await waitFor(() => expect(container.querySelector('table.runs-table')).not.toBeNull());
    const statuses = [...container.querySelectorAll('table.runs-table .pill--status')].map((pill) => pill.closest('td')?.lastChild?.textContent);
    expect(statuses).toEqual([STUB, STUB]);
  });

  /** F14's panel half: the generated workload's generator, seed, whole digest and request follow the input rows. */
  it("shows the generated workload after the input rows", async () => {
    const workload: WorkloadRead = {
      spec: { v: 1, generator: { orders_per_hour: 350, lines_per_order: 3, sku_count: 400, inventory_fill_pct: 80, units_per_stored_tote: 1, release: 'all_at_start', seed: 7 },
        duration_sim_ms: 14_400_000, slots: 96, brings_stock_in: true },
      seed: 7, generator: '0.1.0', sha256: 'c'.repeat(64), bytes: 2048,
    };
    vi.stubGlobal('fetch', vi.fn(async (path: string) => (path === `/api/runs/${RUN}/inputs` ? answer({ ...INPUTS, workload }) : missing(path))));
    const { container } = render(inShell(<Inputs run={RUN} attempt={1} manifestIn />));
    await screen.findByRole('button', { name: 'Copy the inputs as text' });
    const table = container.querySelector('table.list-table');
    const read = container.querySelector('dl.provenance');
    expect(read?.textContent).toContain(`from generator 0.1.0, seed 7: 2,048 bytes, sha256 ${'c'.repeat(64)}`);
    expect(read?.textContent).toContain('"brings_stock_in":true');
    expect(table !== null && read !== null && (table.compareDocumentPosition(read) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0).toBe(true);
  });
});
