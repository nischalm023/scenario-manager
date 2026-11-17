// Spec 08 section 5's accessibility contract on the Runs panel, against answers the test gives: the open run's row
// marked by `aria-current="page"` alone; the queue in one sentence, as the API's queue reads, whatever the list holds,
// or why it could not be read, the last queue read marked out of date; every time value in its `.clock`, only simtime
// tagged; older runs shown by
// following the list's `next`; a native `progress` named in simtime whose value moves at most once a minute
// of wall clock; Cancel keeping focus while it reads "Stopping…", then handing focus to the run's `h3` as it leaves;
// and no live region in the panel.

import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { RunDetail, RunSummary } from '../../protocol/runs.ts';
import { ShellContext } from '../../shell/context.ts';
import { shellOf } from '../../testing/render.tsx';
import { OpenRun } from './OpenRun.tsx';
import { RunsPanel } from './RunsPanel.tsx';

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const RUN = '01KTJ153M7D109TG4G7BBVCZYW';
const OTHER = '01KM5BMQJ3HWJQ2RG6X9TSC526';
const SITE = 'customer/demo/site/lshape-6';
const answer = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });

function summary(id: string, more: Partial<RunSummary> = {}): RunSummary {
  return {
    id, ref: `run:${id}`, submission: '01KQ7Z3F0V5Q8M2R1T6W9X4Y7B', position: 1, of: 1, label: 'Peak hour',
    scenario: { id: '01KK8AJ7RYA6GNNQ3QTRQCW7Z8', version: 1, name: 'Peak hour' }, site: SITE, variant: [], seed: 0,
    duration_sim_ms: 10_800_000, report_window: { from_sim_ms: 0, to_sim_ms: null, before_name: 'warm-up' },
    state: 'running', attempt: 1, progress: { sim_ms: 1_200_000, values: [] }, started_at: '2026-10-19T16:58:00Z',
    elapsed_wall_ms: 120_000, recording: { stored: false }, findings: [], ...more,
  } as RunSummary;
}

function detail(more: Partial<RunDetail> = {}): RunDetail {
  return {
    ...summary(RUN),
    cut: { unit: SITE, requested: '@main', branch: 'main' }, reads: { [`${SITE}/flow`]: 2, [`${SITE}/map`]: 1 },
    runner: { image: 'local:simrunner@c331dee0f', tag: 'c331dee0f', chosen: 'default' },
    expected: { compile_sha256: null, world_sha256: 'a'.repeat(64), compiler: 'none', workload_sha256: 'b'.repeat(64) },
    submitted_by: 'Developer',
    attempts: [{ n: 1, state: 'running', requested_at: '2026-10-19T16:57:00Z', job: `run-${RUN.toLowerCase()}-1`, findings: [] }],
    ...more,
  } as RunDetail;
}

function inShell(node: React.ReactNode, spoken: string[] = []) {
  return <ShellContext.Provider value={shellOf(spoken, `/runs/${RUN}`)}>{node}</ShellContext.Provider>;
}

describe('the Runs panel', () => {
  it("puts the open run on its own page under a switcher of the site's runs, and holds no live region", async () => {
    vi.stubGlobal('fetch', vi.fn(async (path: string) => {
      if (path === `/api/runs/${RUN}`) return answer(detail());
      if (path === `/api/runs?site=${encodeURIComponent(SITE)}`) return answer({ items: [summary(RUN), summary(OTHER, { state: 'finished' })] });
      if (path === '/api/runs/queue') return answer({ limit: 5, going: [summary(RUN)], waiting: [] });
      return answer({ type: 'not-found', title: 'Nothing', status: 404, detail: `No route ${path}.` }, 404);
    }));
    const { container } = render(inShell(<RunsPanel open={RUN} />));
    await waitFor(() => expect(screen.getByRole('heading', { level: 3 }).textContent).toBe(`Peak hour ${RUN}`));
    expect(container.querySelector('table.runs-table'), 'the list stays on its own page').toBeNull();
    const pick = await screen.findByRole('combobox', { name: 'Open another run' }) as HTMLSelectElement;
    expect(pick.value).toBe(RUN);
    await waitFor(() => expect([...pick.options].map((option) => option.value)).toEqual([RUN, OTHER]));
    expect(screen.getByRole('link', { name: 'All runs' }).getAttribute('href')).toContain('/runs');
    expect(screen.getByRole('progressbar', { name: 'Simulated so far, of 3 h simtime' })).not.toBeNull();
    expect(container.querySelector('[aria-live], [role="log"], [role="status"], [role="alert"]')).toBeNull();
  });

  it("says the queue as the API's queue reads, not as the runs the list holds", async () => {
    const waiting = '01KTJ153M7D109TG4G7BBVCZY1';
    vi.stubGlobal('fetch', vi.fn(async (path: string) => {
      if (path === `/api/runs?site=${encodeURIComponent(SITE)}`) return answer({ items: [summary(RUN)] });
      if (path === '/api/runs/queue') return answer({ limit: 5, going: [summary(RUN), summary(OTHER)], waiting: [summary(waiting, { state: 'queued' })] });
      return answer({ type: 'not-found', title: 'Nothing', status: 404, detail: `No route ${path}.` }, 404);
    }));
    const { container } = render(inShell(<RunsPanel site={SITE} />));
    await waitFor(() => expect(container.querySelector('#runs-queue')?.textContent)
      .toBe('Queue: 2 runs going, 1 waiting; at most 5 go at a time in Agent Studio.'));
  });

  it('says why the queue could not be read, and marks the last queue read out of date while reads of it fail', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-10-30T14:02:00'));
    let reads = 0;
    vi.stubGlobal('fetch', vi.fn(async (path: string) => {
      if (path === `/api/runs?site=${encodeURIComponent(SITE)}`) return answer({ items: [summary(RUN)] });
      if (path === '/api/runs/queue') {
        reads += 1;
        if (reads === 2) return answer({ limit: 5, going: [summary(RUN)], waiting: [] });
        return answer({ type: 'unavailable', title: 'Unavailable', status: 503, detail: 'The runs store did not answer.' }, 503);
      }
      return answer({ type: 'not-found', title: 'Nothing', status: 404, detail: `No route ${path}.` }, 404);
    }));
    const { container } = render(inShell(<RunsPanel site={SITE} />));
    const line = () => container.querySelector('#runs-queue')?.textContent;
    expect(line()).toBe('Reading the queue');
    await act(() => vi.advanceTimersByTimeAsync(10));
    expect(line()).toBe('The queue could not be read: The runs store did not answer.');
    await act(() => vi.advanceTimersByTimeAsync(2_000));
    expect(line()).toBe('Queue: 1 run going, none waiting; at most 5 go at a time in Agent Studio.');
    await act(() => vi.advanceTimersByTimeAsync(2_000));
    expect(line()).toBe('out of date Queue: 1 run going, none waiting; at most 5 go at a time in Agent Studio. '
      + 'As read at 14:02; reading it again failed: The runs store did not answer.');
    expect(container.querySelector('#runs-queue .clock[data-clock="wall"]')?.textContent).toBe('14:02');
  });

  it("names no customer or site on a site's list, where every row would name the page's own, nor filters by them", async () => {
    const elsewhere = summary(OTHER, { site: 'customer/acme/site/west' });
    vi.stubGlobal('fetch', vi.fn(async (path: string) => {
      if (path === '/api/runs?mine=true') return answer({ items: [summary(RUN), elsewhere] });
      if (path === `/api/runs?site=${encodeURIComponent(SITE)}`) return answer({ items: [summary(RUN)] });
      if (path === '/api/runs/queue') return answer({ limit: 5, going: [], waiting: [] });
      return answer({ type: 'not-found', title: 'Nothing', status: 404, detail: `No route ${path}.` }, 404);
    }));
    const heads = () => [...document.querySelectorAll('table.runs-table thead th')].map((head) => head.textContent);
    const rows = () => [...document.querySelectorAll('table.runs-table tbody tr')].map((row) => row.getAttribute('data-ref'));
    const { rerender } = render(inShell(<RunsPanel site={SITE} filter="mine" />));
    await waitFor(() => expect(rows()).toHaveLength(2));
    expect(heads()).toEqual(['Compare', 'Run', 'Customer', 'Site', 'Scenario', 'Status', 'Simulated time', 'Started', 'Duration']);
    fireEvent.click(screen.getByRole('button', { name: 'Filter by customer' }));
    fireEvent.click(screen.getByRole('radio', { name: 'acme' }));
    expect(rows()).toEqual([`run:${OTHER}`]);
    rerender(inShell(<RunsPanel site={SITE} filter="site" />));
    await waitFor(() => expect(rows()).toEqual([`run:${RUN}`]));
    expect(heads()).toEqual(['Compare', 'Run', 'Scenario', 'Status', 'Simulated time', 'Started', 'Duration']);
  });

  it('puts every time value in its clock, tagging simtime once a phrase and leaving wall clock untagged', async () => {
    const running = detail({ progress: { sim_ms: 1_200_000, values: [{ metric: 'picks', label: 'Picks', value: 274 }] } });
    vi.stubGlobal('fetch', vi.fn(async (path: string) => {
      if (path === `/api/runs/${RUN}`) return answer(running);
      if (path === `/api/runs?site=${encodeURIComponent(SITE)}`) return answer({ items: [summary(RUN)] });
      if (path === '/api/runs/queue') return answer({ limit: 5, going: [summary(RUN)], waiting: [] });
      return answer({ type: 'not-found', title: 'Nothing', status: 404, detail: `No route ${path}.` }, 404);
    }));
    const { container } = render(inShell(<RunsPanel open={RUN} />));
    await waitFor(() => expect(container.querySelector('.metric-tiles')).not.toBeNull());
    // Every time the panel shows, the table's cells, the tiles and the progress label among them, is in a `.clock`.
    const times: string[] = [];
    const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
      if (/\d+ (s|min|h)\b|\d\d:\d\d/.test(node.textContent ?? '') && node.parentElement?.closest('.clock') === null) {
        times.push(node.textContent ?? '');
      }
    }
    expect(times).toEqual([]);
    expect(container.querySelectorAll('.clock[data-clock="wall"] .clock__tag')).toHaveLength(0);
    expect(container.querySelectorAll('.clock[data-clock="wall"]').length).toBeGreaterThanOrEqual(1);
    const tile = container.querySelector('.metric-tile');
    expect([...(tile?.querySelectorAll('.clock') ?? [])].map((clock) => clock.textContent)).toEqual(['20 min', '3 h simtime']);
  });

  it("follows the list's next to show older runs, and keeps focus on its button once every run shows", async () => {
    const [first, second, third] = ['01KTJ153M7D109TG4G7BBVCZY1', '01KTJ153M7D109TG4G7BBVCZY2', '01KTJ153M7D109TG4G7BBVCZY3'];
    const list = `/api/runs?site=${encodeURIComponent(SITE)}`;
    vi.stubGlobal('fetch', vi.fn(async (path: string) => {
      if (path === list) return answer({ items: [summary(first), summary(second)], next: second });
      if (path === `${list}&after=${second}`) return answer({ items: [summary(third)] });
      if (path === '/api/runs/queue') return answer({ limit: 5, going: [], waiting: [] });
      return answer({ type: 'not-found', title: 'Nothing', status: 404, detail: `No route ${path}.` }, 404);
    }));
    const { container } = render(inShell(<RunsPanel site={SITE} />));
    const rows = () => [...container.querySelectorAll('table.runs-table tbody tr')].map((row) => row.getAttribute('data-ref'));
    await waitFor(() => expect(rows()).toEqual([`run:${first}`, `run:${second}`]));
    const older = screen.getByRole('button', { name: 'Show older runs' });
    older.focus();
    fireEvent.click(older);
    await waitFor(() => expect(rows()).toEqual([`run:${first}`, `run:${second}`, `run:${third}`]));
    const all = screen.getByRole('button', { name: 'All runs shown' });
    expect(all.getAttribute('aria-disabled')).toBe('true');
    expect(document.activeElement).toBe(all);
  });

  it('moves the progress bar at most once a minute of wall clock, however often the run reports', () => {
    vi.useFakeTimers();
    const { rerender } = render(inShell(<OpenRun run={detail()} reread={() => {}} />));
    const bar = () => screen.getByRole('progressbar') as HTMLProgressElement;
    expect(bar().value).toBe(1_200_000);
    rerender(inShell(<OpenRun run={detail({ progress: { sim_ms: 2_400_000, values: [] } })} reread={() => {}} />));
    act(() => vi.advanceTimersByTime(59_000));
    expect(bar().value).toBe(1_200_000);
    act(() => vi.advanceTimersByTime(1_000));
    expect(bar().value).toBe(2_400_000);
  });

  it('keeps focus on Cancel while it reads Stopping…, and hands it to the run heading once the run ends', async () => {
    const posted: string[] = [];
    vi.stubGlobal('fetch', vi.fn(async (path: string, init?: RequestInit) => {
      // The playback's own ask for a watch ticket is not the press under test.
      if (path.endsWith('/watch')) return answer({ type: 'not-streamed', title: 'Not streamed', status: 409, detail: 'No live stream.' }, 409);
      if (init?.method === 'POST') posted.push(path);
      return answer({ outcome: 'stopping', state: 'running' }, 202);
    }));
    const spoken: string[] = [];
    const { rerender } = render(inShell(<OpenRun run={detail()} reread={() => {}} />, spoken));
    const cancel = screen.getByRole('button', { name: 'Cancel the run' });
    cancel.focus();
    fireEvent.click(cancel);
    await waitFor(() => expect(screen.getByRole('button', { name: 'Stopping…' })).toBe(document.activeElement));
    // The page also asks for a watch ticket while the run runs; the press itself posts the cancel alone.
    expect(posted.filter((path) => !path.endsWith('/watch'))).toEqual([`/api/runs/${RUN}/cancel`]);
    const cancelled = detail({ state: 'cancelled', progress: { sim_ms: 1_260_000, values: [] }, ended_by: 'person' });
    rerender(inShell(<OpenRun run={cancelled} reread={() => {}} />, spoken));
    expect(screen.queryByRole('button', { name: /Stopping|Cancel/ })).toBeNull();
    expect(document.activeElement).toBe(screen.getByRole('heading', { level: 3 }));
    expect(screen.getByText('cancelled at 21 min of simtime')).not.toBeNull();
    expect(spoken).toEqual([]);
  });
});
