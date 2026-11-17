// The run page's metric panels: focus goes to the next panel's Remove as a panel leaves, then the one before it, then
// the Add control, never to `body`; a panel added is kept for the next page, with the panel size; Attach again opens a
// fresh socket once the hook's own tries are spent, and is not offered after a final refusal, which the panel says in
// words, a ticket refused as not streamed being final only once the run has ended; a metric only the end block states shows its value as the panel's one figure, or once its stream ended without
// it, why it has none; a report series' whole-run rows are listed beside its plot, never drawn on it; a series panel
// leads with the whole run's value the stream sends, else the latest window's, and makes no mean; a duration's clock is
// named; and a panel of a recording the server says stops short of the run's end says so in place of the run's ending.
// The metric sockets are stand-ins the test drives.

import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { MetricDescriptor } from '../../protocol/metric_stream.ts';
import { MetricCharts } from './MetricCharts.tsx';
import { playerCursor } from './playerCursor.ts';

const RUN = '01KZAVPQNES65HV4EZADPCS7E8';
const descriptor = (metric: string, more: Partial<MetricDescriptor> = {}): MetricDescriptor => ({
  metric, label: metric, description: `The ${metric}.`, unit: 'count', aggregation: 'additive', source: 'scalars',
  bucket: { kind: 'slice', width_sim_ms: 10_000 }, ...more,
});
const CATALOGUE = ['a', 'b', 'c', 'run_total_picks'].map((metric) => descriptor(metric));

/** How many tickets the panels asked for, and the API's answer when a test sets one: a ticket by default. */
const tickets = vi.hoisted(() => ({ asked: 0, refusal: null as unknown }));

vi.mock('../../shell/api.ts', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../shell/api.ts')>()),
  getJson: async () => ({ ok: true, status: 200, value: { metrics: CATALOGUE } }),
  postJson: async () => {
    tickets.asked += 1;
    return tickets.refusal ?? {
      ok: true, status: 200,
      value: { run: RUN, attempt: 1, url: 'ws://s/streams', frames_url: 'ws://s/frames', metrics_url: 'ws://s/metrics', token: 't', expires_at: '2026-11-06T12:00:00Z' },
    };
  },
}));

/** The socket each panel opens, which the test drives. */
class Socket {
  static opened: Socket[] = [];
  sent: { metric?: string }[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onclose: ((event: { reason: string }) => void) | null = null;
  constructor() {
    Socket.opened.push(this);
    queueMicrotask(() => this.onopen?.());
  }
  send(text: string) {
    this.sent.push(JSON.parse(text));
  }
  close() {}
}

beforeEach(() => {
  Socket.opened = [];
  tickets.asked = 0;
  tickets.refusal = null;
  vi.stubGlobal('WebSocket', Socket);
});

afterEach(() => {
  cleanup();
  localStorage.clear();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it('adds a panel from the catalogue and keeps the choice, and keeps the panel size, for the next page', async () => {
  localStorage.setItem('studio.runs.metric-charts', JSON.stringify(['a']));
  const { container, unmount } = render(<MetricCharts run={RUN} attempt={1} cursor={playerCursor()} live />);
  const offered = await screen.findByRole('combobox', { name: 'The metric to add a panel of' });
  fireEvent.change(offered, { target: { value: 'c' } });
  fireEvent.click(screen.getByRole('button', { name: 'Add' }));
  expect([...container.querySelectorAll('.mpanel__title')].map((title) => title.textContent)).toEqual(['a', 'c']);
  fireEvent.click(screen.getByRole('radio', { name: 'L' }));
  expect(container.querySelector('.metrics-grid')?.getAttribute('data-density')).toBe('large');
  unmount();
  const { container: again } = render(<MetricCharts run={RUN} attempt={1} cursor={playerCursor()} live />);
  expect([...again.querySelectorAll('.mpanel__title')].map((title) => title.textContent), 'the kept choice').toEqual(['a', 'c']);
  expect(again.querySelector('.metrics-grid')?.getAttribute('data-density'), 'the kept size').toBe('large');
});

it("attaches a panel afresh at once on Attach again, once the hook's own tries are spent", async () => {
  vi.useFakeTimers();
  localStorage.setItem('studio.runs.metric-charts', JSON.stringify(['a']));
  render(<MetricCharts run={RUN} attempt={1} cursor={playerCursor()} live />);
  const attached = { type: 'attached', v: 1, run: RUN, attempt: 1, descriptor: descriptor('a'), reached_sim_ms: 0, live: true };
  for (let loss = 0; loss < 6; loss += 1) {
    await act(() => vi.advanceTimersByTimeAsync(10));
    act(() => Socket.opened.at(-1)?.onmessage?.({ data: JSON.stringify(attached) }));
    act(() => Socket.opened.at(-1)?.onclose?.({ reason: 'silent' }));
    await act(() => vi.advanceTimersByTimeAsync(2_500));
  }
  expect(Socket.opened, 'the first socket and five more on its own').toHaveLength(6);
  fireEvent.click(screen.getByRole('button', { name: 'Attach again' }));
  await act(() => vi.advanceTimersByTimeAsync(10));
  expect(Socket.opened, 'one more, at once').toHaveLength(7);
});

it('offers no Attach again after a final refusal, and says it in words', async () => {
  vi.useFakeTimers();
  localStorage.setItem('studio.runs.metric-charts', JSON.stringify(['a']));
  const { container } = render(<MetricCharts run={RUN} attempt={1} cursor={playerCursor()} live={false} />);
  await act(() => vi.advanceTimersByTimeAsync(10));
  act(() => Socket.opened.at(-1)?.onclose?.({ reason: 'no-recording' }));
  await act(() => vi.advanceTimersByTimeAsync(10_000));
  expect(container.querySelector('.mpanel__foot')?.textContent).toBe(
    'Interrupted: no streaming server can serve this attempt: none holds its stream, and no recording of it is stored. ',
  );
  expect(screen.queryByRole('button', { name: 'Attach again' })).toBeNull();
  expect(Socket.opened, 'no other socket').toHaveLength(1);
});

it("asks again for a ticket refused as not streamed while the run goes, but not once it has ended", async () => {
  vi.useFakeTimers();
  localStorage.setItem('studio.runs.metric-charts', JSON.stringify(['a']));
  const detail = 'Attempt 1 of this run has no live stream: its streaming server has not been heard from.';
  tickets.refusal = { ok: false, status: 409, problem: { type: 'not-streamed', title: 'Not streamed', status: 409, detail }, reason: detail };
  const { unmount } = render(<MetricCharts run={RUN} attempt={1} cursor={playerCursor()} live />);
  await act(() => vi.advanceTimersByTimeAsync(4_100));
  expect(tickets.asked, 'the first ask and two more, 2 s apart').toBe(3);
  unmount();
  tickets.asked = 0;
  render(<MetricCharts run={RUN} attempt={1} cursor={playerCursor()} live={false} />);
  await act(() => vi.advanceTimersByTimeAsync(4_100));
  expect(tickets.asked, 'refused for good').toBe(1);
});

it("moves focus to the next panel's Remove as a panel leaves, then the one before, then the Add control", async () => {
  localStorage.setItem('studio.runs.metric-charts', JSON.stringify(['a', 'b', 'c']));
  render(<MetricCharts run={RUN} attempt={1} cursor={playerCursor()} live />);
  const remove = (metric: string) => screen.getByRole('button', { name: `Remove the panel of ${metric}` });
  fireEvent.click(remove('b'));
  expect(document.activeElement).toBe(remove('c'));
  fireEvent.click(remove('c'));
  expect(document.activeElement).toBe(remove('a'));
  await screen.findByRole('combobox');
  fireEvent.click(remove('a'));
  expect(document.activeElement).toBe(screen.getByRole('combobox'));
});

it('shows the value a metric only the end block states as its one figure', async () => {
  localStorage.setItem('studio.runs.metric-charts', JSON.stringify(['run_total_picks']));
  const { container } = render(<MetricCharts run={RUN} attempt={1} cursor={playerCursor()} live={false} />);
  await waitFor(() => expect(Socket.opened[0]?.sent).toHaveLength(1));
  const said = (message: object) => act(() => Socket.opened[0].onmessage?.({ data: JSON.stringify(message) }));
  said({ type: 'attached', v: 1, run: RUN, attempt: 1, descriptor: descriptor('run_total_picks', { label: 'Picks, whole run', source: 'end_block', bucket: { kind: 'overall' } }), reached_sim_ms: 10_800_000, live: false });
  said({ type: 'rows', v: 1, rows: [{ from_sim_ms: 0, to_sim_ms: 10_800_000, value: 1276, whole_run: true }] });
  await waitFor(() => expect(container.querySelector('.mpanel__huge')?.textContent).toBe('1,276'));
  expect(container.querySelector('.mpanel__stat .mpanel__when')?.textContent).toBe('1,276 over the whole run, to 3 h simtime');
  expect(container.querySelector('.mpanel__empty'), 'nothing waits once the end block stated it').toBeNull();
});

it('says why a metric only the end block states has no value once its stream ended without it', async () => {
  localStorage.setItem('studio.runs.metric-charts', JSON.stringify(['run_total_picks']));
  const endBlock = descriptor('run_total_picks', { label: 'Picks, whole run', source: 'end_block', bucket: { kind: 'overall' } });
  const ending = async (outcome: string) => {
    Socket.opened = [];
    const { container, unmount } = render(<MetricCharts run={RUN} attempt={1} cursor={playerCursor()} live={false} />);
    await waitFor(() => expect(Socket.opened[0]?.sent).toHaveLength(1));
    const said = (message: object) => act(() => Socket.opened[0].onmessage?.({ data: JSON.stringify(message) }));
    said({ type: 'attached', v: 1, run: RUN, attempt: 1, descriptor: endBlock, reached_sim_ms: 0, live: false });
    await waitFor(() => expect(container.querySelector('.mpanel__empty')?.textContent).toBe('Stated once the run ends'));
    said({ type: 'ended', v: 1, outcome });
    await waitFor(() => expect(container.querySelector('.mpanel__empty')?.getAttribute('aria-busy')).toBeNull());
    const words = container.querySelector('.mpanel__empty')?.textContent;
    unmount();
    return words;
  };
  expect(await ending('no-end-block')).toBe('No value: its recording has no end block');
  expect(await ending('recording-incomplete')).toBe('No value: the recording is incomplete: the streaming server stopped before it stored the rest of the run');
  expect(await ending('sim-time-limit')).toBe('No value: the end block does not state it');
});

it("lists a report series' whole-run rows beside its plot and never draws them, even over a run one window long", async () => {
  localStorage.setItem('studio.runs.metric-charts', JSON.stringify(['a']));
  const { container } = render(<MetricCharts run={RUN} attempt={1} cursor={playerCursor()} live={false} />);
  await waitFor(() => expect(Socket.opened[0]?.sent).toHaveLength(1));
  const said = (message: object) => act(() => Socket.opened[0].onmessage?.({ data: JSON.stringify(message) }));
  const states = descriptor('a', {
    label: 'State share', unit: 'ratio', aggregation: 'ratio', source: 'journals', dimensions: [{ name: 'state', description: 'The state.' }],
    bucket: { kind: 'trailing', window_sim_ms: 900_000, step_sim_ms: 300_000 },
  });
  said({ type: 'attached', v: 1, run: RUN, attempt: 1, descriptor: states, reached_sim_ms: 900_000, live: false });
  said({ type: 'rows', v: 1, rows: [
    { from_sim_ms: 0, to_sim_ms: 900_000, keys: ['Waiting'], value: 60 },
    { from_sim_ms: 0, to_sim_ms: 900_000, keys: ['Waiting'], value: 61, whole_run: true },
  ] });
  await waitFor(() => expect(container.querySelector('.mpanel__whole .barlist')?.textContent).toBe('Waiting61'));
  expect(screen.getByRole('img').getAttribute('aria-label')).toBe('State share: 1 values to 15 min of simtime, between 60 and 60, the last 60.');
  expect(container.querySelector('.mpanel__legend tbody')?.textContent, 'the legend reads the plotted window').toBe('Waiting6060');
  expect(container.querySelector('.mpanel__legend thead')?.textContent, 'no mean the browser made').not.toContain('mean');
});

it("leads a series panel with the whole run's value once the stream sends one, else the latest window's, and makes no mean", async () => {
  localStorage.setItem('studio.runs.metric-charts', JSON.stringify(['a']));
  const { container } = render(<MetricCharts run={RUN} attempt={1} cursor={playerCursor()} live={false} />);
  await waitFor(() => expect(Socket.opened[0]?.sent).toHaveLength(1));
  const said = (message: object) => act(() => Socket.opened[0].onmessage?.({ data: JSON.stringify(message) }));
  said({ type: 'attached', v: 1, run: RUN, attempt: 1, descriptor: descriptor('a'), reached_sim_ms: 20_000, live: false });
  said({ type: 'rows', v: 1, rows: [{ from_sim_ms: 0, to_sim_ms: 10_000, value: 10 }, { from_sim_ms: 10_000, to_sim_ms: 20_000, value: 30 }] });
  const lead = () => [container.querySelector('.mpanel__big')?.textContent, container.querySelector('.mpanel__lead .mpanel__when')?.textContent];
  await waitFor(() => expect(lead()[0]).toBe('30'));
  expect(lead()[1]).toMatch(/^last, at /);
  expect([...container.querySelectorAll('.mpanel__minor dt')].map((term) => term.textContent)).toEqual(['min', 'max']);
  said({ type: 'rows', v: 1, rows: [{ from_sim_ms: 0, to_sim_ms: 20_000, value: 40, whole_run: true }] });
  await waitFor(() => expect(lead()).toEqual(['40', 'over the whole run']));
});

it("names a wall-clock duration's clock on its panel", async () => {
  localStorage.setItem('studio.runs.metric-charts', JSON.stringify(['a']));
  const { container } = render(<MetricCharts run={RUN} attempt={1} cursor={playerCursor()} live={false} />);
  await waitFor(() => expect(Socket.opened[0]?.sent).toHaveLength(1));
  const said = (message: object) => act(() => Socket.opened[0].onmessage?.({ data: JSON.stringify(message) }));
  said({ type: 'attached', v: 1, run: RUN, attempt: 1, descriptor: descriptor('a', { unit: 'duration_ms', clock: 'wall' }), reached_sim_ms: 10_000, live: false });
  said({ type: 'rows', v: 1, rows: [{ from_sim_ms: 0, to_sim_ms: 10_000, value: 2_500 }] });
  await waitFor(() => expect(container.querySelector('.mpanel__big')?.getAttribute('data-clock')).toBe('wall'));
  expect(container.querySelector('.mpanel__unit')?.textContent).toBe('wall clock');
});

it('says how the run ended in the words of its outcome', async () => {
  localStorage.setItem('studio.runs.metric-charts', JSON.stringify(['a']));
  render(<MetricCharts run={RUN} attempt={1} cursor={playerCursor()} live={false} />);
  await waitFor(() => expect(Socket.opened[0]?.sent).toHaveLength(1));
  const said = (message: object) => act(() => Socket.opened[0].onmessage?.({ data: JSON.stringify(message) }));
  said({ type: 'attached', v: 1, run: RUN, attempt: 1, descriptor: descriptor('a'), reached_sim_ms: 10_000, live: false });
  said({ type: 'ended', v: 1, outcome: 'sim-time-limit' });
  const legend = await screen.findByText(/the run ended/);
  expect(legend.textContent).toBe('The a. One value per 10 s of simtime; the run ended: it reached the simtime it was to simulate.');
});

it("says a panel's recording is incomplete when the server ends its stream so, in place of the run's ending", async () => {
  localStorage.setItem('studio.runs.metric-charts', JSON.stringify(['a']));
  render(<MetricCharts run={RUN} attempt={1} cursor={playerCursor()} live={false} />);
  await waitFor(() => expect(Socket.opened[0]?.sent).toHaveLength(1));
  const said = (message: object) => act(() => Socket.opened[0].onmessage?.({ data: JSON.stringify(message) }));
  said({ type: 'attached', v: 1, run: RUN, attempt: 1, descriptor: descriptor('a'), reached_sim_ms: 10_000, live: false });
  said({ type: 'rows', v: 1, rows: [{ from_sim_ms: 0, to_sim_ms: 10_000, value: 3 }] });
  said({ type: 'ended', v: 1, outcome: 'recording-incomplete' });
  const legend = await screen.findByText(/the recording is incomplete/);
  expect(legend.textContent).toBe(
    'The a. One value per 10 s of simtime; the recording is incomplete: the streaming server stopped before it stored the rest of the run.',
  );
});
