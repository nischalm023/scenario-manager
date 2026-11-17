// The open run's player (`OpenRun`), against a stand-in API and a stand-in viewer: the picture is the streaming
// server's, from its frames socket, while the run goes and once it has ended alike; a page that watched the run keeps
// its player through the end, and a page opened after it plays it the same way, never reading the API's recording; the
// player's cursor reaches the metric charts, and moving it renders nothing of the page; the metric panels are offered
// as the picture is, a stored recording that never streamed included; a refused watch is said in words; and the
// picture filling the window is a region named for it, the live view or the playback. A canvas
// draws nothing here, so the viewer is a stand-in whose held window and cursor the test sets, the charts a stand-in
// that shows the cursor it reads, and the inputs a stand-in that counts the page's renders.

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { RunDetail, WatchTicket } from '../../protocol/runs.ts';
import { ShellContext } from '../../shell/context.ts';
import { shellOf } from '../../testing/render.tsx';
import { OpenRun } from './OpenRun.tsx';
import type { Cursor } from './playerCursor.ts';

/** What the stand-in viewer holds: `[from, to]` once a keyframe has come, nothing before; and its cursor. */
const viewer = vi.hoisted(() => ({ held: [] as number[], cursor: 0 }));
/** How often the page has rendered its inputs, which it renders whenever it renders. */
const inputs = vi.hoisted(() => ({ renders: 0 }));

vi.mock('crucible-viewer', () => {
  const replies: Record<string, unknown> = {
    frame: () => true,
    has_floor: () => false,
    camera: () => new Float32Array(6),
    centre_entities: () => [],
    centre_mm: () => new Float64Array(0),
    held_window: () => viewer.held,
    cursor_ms: () => viewer.cursor,
    playing: () => false,
  };
  const standIn = new Proxy({}, { get: (_, name) => (name === 'then' ? undefined : replies[String(name)] ?? (() => undefined)) });
  return {
    openFloorViewer: async () => standIn,
    readPalette: () => ({}),
    attachFrames: () => ({ close: () => undefined }),
  };
});

vi.mock('./MetricCharts.tsx', async () => {
  const { createElement, useSyncExternalStore } = await import('react');
  return {
    MetricCharts: ({ cursor }: { cursor: Cursor }) => {
      const ms = useSyncExternalStore(cursor.subscribe, cursor.get);
      return createElement('section', { className: 'run-metrics', 'data-cursor': ms === null ? 'none' : String(ms) });
    },
  };
});

vi.mock('./Inputs.tsx', async () => {
  const { createElement } = await import('react');
  return {
    Inputs: () => {
      inputs.renders += 1;
      return createElement('div', { className: 'run-inputs' });
    },
  };
});

const RUN = '01KTJ153M7D109TG4G7BBVCZYW';
const SITE = 'customer/demo/site/lshape-6';
const TICKET: WatchTicket = {
  run: RUN, attempt: 1, url: `ws://127.0.0.1:3202/streams/${RUN}/1`, frames_url: `ws://127.0.0.1:3202/frames/${RUN}/1`, metrics_url: `ws://127.0.0.1:3202/metrics/${RUN}/1`,
  token: 'the-ticket', expires_at: '2026-10-27T12:01:00Z',
};
const answer = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });
const nothing = (path: string) => answer({ type: 'not-found', title: 'Nothing', status: 404, detail: `No route ${path}.` }, 404);

/** A run of Peak hour whose attempt was given its streaming server, running unless `more` says otherwise. */
function streamed(more: Partial<RunDetail> = {}): RunDetail {
  return {
    id: RUN, ref: `run:${RUN}`, submission: '01KQ7Z3F0V5Q8M2R1T6W9X4Y7B', position: 1, of: 1, label: 'Peak hour',
    scenario: { id: '01KK8AJ7RYA6GNNQ3QTRQCW7Z8', version: 1, name: 'Peak hour' }, site: SITE, variant: [], seed: 0,
    duration_sim_ms: 10_800_000, report_window: { from_sim_ms: 0, to_sim_ms: null, before_name: 'warm-up' },
    state: 'running', attempt: 1, progress: { sim_ms: 1_200_000, values: [] }, started_at: '2026-10-19T16:58:00Z',
    elapsed_wall_ms: 120_000, recording: { stored: false }, findings: [],
    cut: { unit: SITE, requested: '@main', branch: 'main' }, reads: { [`${SITE}/flow`]: 2, [`${SITE}/map`]: 1 },
    runner: { image: 'local:simrunner@c331dee0f', tag: 'c331dee0f', chosen: 'default' },
    expected: { compile_sha256: null, world_sha256: 'a'.repeat(64), compiler: 'none', workload_sha256: 'b'.repeat(64) },
    submitted_by: 'Developer',
    attempts: [{
      n: 1, state: 'running', requested_at: '2026-10-19T16:57:00Z', job: `run-${RUN.toLowerCase()}-1`, findings: [],
      streaming_server: '01KTJ153M7D109TG4G7BBVCZS1',
    }],
    ...more,
  } as RunDetail;
}

const ENDED: Partial<RunDetail> = { state: 'finished', recording: { stored: true } as RunDetail['recording'] };

/** The page in the shell, the API answering the watch with `watch` and nothing else. */
function opened(watch: () => Response) {
  vi.stubGlobal('fetch', vi.fn(async (path: string) => (path === `/api/runs/${RUN}/watch` ? watch() : nothing(path))));
  return (run: RunDetail) => <ShellContext.Provider value={shellOf([], `/runs/${RUN}`)}><OpenRun run={run} reread={() => {}} /></ShellContext.Provider>;
}

beforeEach(() => {
  vi.stubGlobal('requestAnimationFrame', () => 0);
  vi.stubGlobal('cancelAnimationFrame', () => undefined);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  // The live numbers' own socket is not the player under test.
  vi.stubGlobal('WebSocket', undefined);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("a page opened while its run streams, once the run ends", () => {
  it('keeps its player, which is the one player on the page', async () => {
    viewer.held = [0, 600_000];
    const page = opened(() => answer(TICKET));
    const { container, rerender } = render(page(streamed()));
    await waitFor(() => expect(container.querySelector('#run-playback-mode')?.textContent).toContain('Cursor at'));
    const view = container.querySelector('.run-live .run-playback');
    rerender(page(streamed(ENDED)));
    expect(container.querySelector('.run-live .run-playback')).toBe(view);
    expect(container.querySelectorAll('.run-playback')).toHaveLength(1);
  });
});

describe('a page opened after its run ended', () => {
  it("plays it from the streaming server's frames, never reads the API's recording, and draws its cursor on the charts", async () => {
    viewer.held = [0, 600_000];
    const page = opened(() => answer(TICKET));
    const { container } = render(page(streamed(ENDED)));
    await waitFor(() => expect(container.querySelector('#run-playback-mode')?.textContent).toContain('Cursor at'));
    expect(container.querySelector('.run-card--player .run-playback'), 'beside the Summary').not.toBeNull();
    expect(container.querySelector('.run-live'), 'and no live card').toBeNull();
    await waitFor(() => expect(container.querySelector('.run-metrics')?.getAttribute('data-cursor')).toBe('0'));
    const asked = vi.mocked(fetch).mock.calls.map(([path]) => String(path));
    expect(asked.filter((path) => path.includes('/recording')), 'the picture is the streaming server\'s').toEqual([]);
  });

  it('offers its metric panels as it offers its picture: from a stored recording that never streamed too', async () => {
    viewer.held = [0, 600_000];
    const page = opened(() => answer(TICKET));
    const neverStreamed = { ...streamed().attempts[0], state: 'finished', streaming_server: undefined } as RunDetail['attempts'][number];
    const { container, rerender } = render(page(streamed({ ...ENDED, attempts: [neverStreamed] })));
    await waitFor(() => expect(container.querySelector('#run-playback-mode')?.textContent).toContain('Cursor at'));
    expect(container.querySelector('.run-metrics'), 'the panels read the copy the picture reads').not.toBeNull();
    rerender(page(streamed({ state: 'finished', attempts: [neverStreamed] })));
    expect(container.querySelector('.run-card--player.run-card--empty'), 'nothing stored, nothing to play').not.toBeNull();
    expect(container.querySelector('.run-metrics'), 'and nothing to chart').toBeNull();
  });

  it("moves the panels' cursor as the player moves without rendering the page again", async () => {
    [viewer.held, viewer.cursor] = [[0, 600_000], 0];
    const page = opened(() => answer(TICKET));
    const { container } = render(page(streamed(ENDED)));
    await waitFor(() => expect(container.querySelector('.run-metrics')?.getAttribute('data-cursor')).toBe('0'));
    const rendered = inputs.renders;
    viewer.cursor = 120_000;
    await waitFor(() => expect(container.querySelector('.run-metrics')?.getAttribute('data-cursor')).toBe('120000'));
    expect(inputs.renders, 'the cursor reached the panels, and the page did not render for it').toBe(rendered);
  });

  it('says why there is no playback when the watch is refused', async () => {
    const page = opened(() => answer({ type: 'unavailable', title: 'Unavailable', status: 503, detail: 'The streaming server did not answer.' }, 503));
    const { container } = render(page(streamed(ENDED)));
    await waitFor(() => expect(container.querySelector('.run-card--player')?.textContent).toContain('No playback: The streaming server did not answer.'));
  });
});

describe('the picture filling the window', () => {
  it.each([
    ['live view while the run goes', streamed(), 'Live view of Peak hour, filling the window'],
    ['playback once the run has ended', streamed(ENDED), 'Playback of Peak hour, filling the window'],
  ])('is a region named for the run\'s %s', async (_, detail, name) => {
    const page = opened(() => answer(TICKET));
    render(page(detail));
    fireEvent.click(await screen.findByRole('button', { name: 'Fill the window' }));
    expect(screen.getByRole('region', { name })).not.toBeNull();
  });
});
