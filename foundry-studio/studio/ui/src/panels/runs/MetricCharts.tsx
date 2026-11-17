// The run page's metric panels (the metric streams): a board of panels under the playback, one per chosen metric, each
// fed by its own socket to the streaming server (`metricStream`), drawn bucket by bucket as the server sends them, and
// whole for a run that has ended. A panel reads like a monitoring dashboard's: its figures (the whole run's or the
// last, lowest, highest) over the plot, a crosshair and tooltip shared across every panel so the board reads at one
// instant, a legend table for a metric with dimensions, a single figure for a metric only the end block states and a
// bar list for one stated by station or destination. The metrics offered are the server's catalogue
// (`GET /api/metric-streams`); the page starts with a few and adds or removes panels, remembering the choice and the
// panel size per browser, and focus moves to the next panel's Remove, or to the Add control, as a panel leaves. The
// player's cursor is a line on every plot, so the picture and the numbers are read at one instant of simtime; moving it
// redraws the line alone, and the plot itself always holds everything received. The rows the server marks as stating
// the whole run are listed beside the plot and never drawn on it, and a recording the server says stops short of the
// run's end says so in place of the run's ending. A panel whose socket was lost attaches again on its own, and at once
// on request, unless the refusal was final. Every instant is simtime and says so; a duration or a rate names the clock
// its descriptor names. Every figure is a value the server sent: a panel picks a line's last, lowest and highest
// (`charts.ts`), and computes none, so no figure disagrees with the run's own.

import { useEffect, useId, useMemo, useRef, useState } from 'react';
import type { MetricCatalogue, MetricDescriptor, MetricPoint, MetricSource } from '../../protocol/metric_stream.ts';
import { clause, getJson } from '../../shell/api.ts';
import { readStored, writeStored } from '../../shell/storage.ts';
import { Chart } from './Chart.tsx';
import {
  hoursOf, hourWords, keyWords, shortValue, StreamChart, swatch, unitSign, type ChartSpec, type StreamLine,
} from './charts.ts';
import { useMetricStream, type MetricStream } from './metricStream.ts';
import type { Cursor } from './playerCursor.ts';
import { SimSpan } from './RunResults.tsx';
import { isTime, spanWords, valueWords } from './resultWords.ts';
import { OUTCOME_WORDS } from './words.ts';

/** The panels a page starts with: desvis's two defaults and the running total of picks. */
const DEFAULT_METRICS = ['picks_per_hour', 'inducted_totes_per_hour', 'cumulative_picks'];
const CHOICE_KEY = 'studio.runs.metric-charts';
const DENSITY_KEY = 'studio.runs.metric-density';
/** The crosshair shared by every panel of the board. */
const SYNC_KEY = 'run-metrics';

type Density = 'compact' | 'normal' | 'large';
const DENSITIES: { density: Density; label: string; height: number }[] = [
  { density: 'compact', label: 'S', height: 150 },
  { density: 'normal', label: 'M', height: 210 },
  { density: 'large', label: 'L', height: 300 },
];

const SOURCE_WORDS: Record<MetricSource, string> = {
  scalars: 'Time series', robot_state: 'Shares of time in a state, from the robot state channel',
  journals: 'Report series, by state, aisle or station', end_block: 'Figures at the end of the run',
};

/** What this browser kept under `key`, when it reads as `accept` wants; else `fallback`. */
function kept<T>(key: string, fallback: T, accept: (parsed: unknown) => parsed is T): T {
  const raw = readStored('local', key);
  if (raw === null) return fallback;
  try {
    const parsed: unknown = JSON.parse(raw);
    return accept(parsed) ? parsed : fallback;
  } catch {
    return fallback;
  }
}

const keep = (key: string, value: unknown) => writeStored('local', key, JSON.stringify(value));

const isMetrics = (parsed: unknown): parsed is string[] => Array.isArray(parsed) && parsed.every((one) => typeof one === 'string');
const isDensity = (parsed: unknown): parsed is Density => parsed === 'compact' || parsed === 'normal' || parsed === 'large';

export function MetricCharts({ run, attempt, cursor, live, describedBy }: {
  /** The run's id and its latest attempt, which the panels are of. */
  run: string;
  attempt: number;
  /** The player's cursor in simtime, drawn on every plot; it holds none while there is no player. */
  cursor: Cursor;
  /** Whether the run still goes, for the words beside the panels and for how a panel takes a refused ticket. */
  live: boolean;
  /** What else describes every panel, by element id: a stub attempt's sentence (ruling D110). */
  describedBy?: string;
}) {
  const [catalogue, setCatalogue] = useState<{ state: 'reading' } | { state: 'read'; metrics: MetricDescriptor[] } | { state: 'failed'; reason: string }>({ state: 'reading' });
  useEffect(() => {
    let current = true;
    void getJson<MetricCatalogue>('/api/metric-streams').then((answer) => {
      if (!current) return;
      if (answer.ok && Array.isArray(answer.value?.metrics)) setCatalogue({ state: 'read', metrics: answer.value.metrics });
      else setCatalogue({ state: 'failed', reason: answer.ok ? 'the service answered something else than a catalogue' : clause(answer.reason) });
    });
    return () => {
      current = false;
    };
  }, []);
  const [chosen, setChosen] = useState<string[]>(() => kept(CHOICE_KEY, DEFAULT_METRICS, isMetrics));
  const choose = (metrics: string[]) => {
    setChosen(metrics);
    keep(CHOICE_KEY, metrics);
  };
  const [density, setDensity] = useState<Density>(() => kept(DENSITY_KEY, 'normal', isDensity));
  const size = (next: Density) => {
    setDensity(next);
    keep(DENSITY_KEY, next);
  };
  const height = DENSITIES.find((one) => one.density === density)?.height ?? 210;
  const byName = useMemo(() => new Map(catalogue.state === 'read' ? catalogue.metrics.map((metric) => [metric.metric, metric]) : []), [catalogue]);
  const titleId = useId();
  // Where focus goes once a removed panel has left: the Remove of the panel that took its place, else the one before
  // it, else the Add control, else the board's title.
  const board = useRef<HTMLElement>(null);
  const title = useRef<HTMLHeadingElement>(null);
  const leaving = useRef<number | null>(null);
  useEffect(() => {
    const at = leaving.current;
    leaving.current = null;
    if (at === null || board.current === null) return;
    const removes = board.current.querySelectorAll<HTMLButtonElement>('[data-remove]');
    const next = removes[Math.min(at, removes.length - 1)] ?? board.current.querySelector('select') ?? title.current;
    next?.focus();
  }, [chosen]);
  return (
    <section className="section run-metrics metrics-board" aria-labelledby={titleId} ref={board}>
      <header className="metrics-board__bar">
        <h4 className="section__title" id={titleId} tabIndex={-1} ref={title}>
          Metrics
          <span className="muted run-card__note">{live ? ' computed by the streaming server as the run goes' : ' computed by the streaming server from the recording'}</span>
        </h4>
        <div className="metrics-board__tools">
          <div className="size-switch" role="radiogroup" aria-label="Panel size">
            {DENSITIES.map((one) => (
              <button key={one.density} type="button" role="radio" aria-checked={density === one.density} className="size-switch__item" onClick={() => size(one.density)} title={`${one.density} panels`}>
                {one.label}
              </button>
            ))}
          </div>
          {catalogue.state === 'read' && <AddPanel metrics={catalogue.metrics} chosen={chosen} onAdd={(metric) => choose([...chosen, metric])} />}
          {chosen.join() !== DEFAULT_METRICS.join() && (
            <button className="btn btn--small btn--quiet" type="button" onClick={() => choose(DEFAULT_METRICS)}>Reset</button>
          )}
        </div>
      </header>
      {catalogue.state === 'failed' && <p className="panel__status">The metrics could not be listed: {catalogue.reason}.</p>}
      {chosen.length === 0 && <p className="muted">No panel chosen. Add one above.</p>}
      <div className="metrics-grid" data-density={density}>
        {chosen.map((metric, at) => (
          <MetricPanel
            key={metric} run={run} attempt={attempt} metric={metric} described={byName.get(metric) ?? null} cursor={cursor} height={height} live={live}
            describedBy={describedBy}
            onRemove={() => {
              leaving.current = at;
              choose(chosen.filter((one) => one !== metric));
            }}
          />
        ))}
      </div>
    </section>
  );
}

/** The control that adds a panel: a word to filter the catalogue by, the metrics left grouped by where they come
 * from. */
function AddPanel({ metrics, chosen, onAdd }: { metrics: MetricDescriptor[]; chosen: string[]; onAdd: (metric: string) => void }) {
  const [filter, setFilter] = useState('');
  const [adding, setAdding] = useState('');
  const offered = metrics.filter((metric) => !chosen.includes(metric.metric) && (filter === '' || `${metric.label} ${metric.description}`.toLowerCase().includes(filter.toLowerCase())));
  const groups = (['scalars', 'robot_state', 'journals', 'end_block'] as MetricSource[]).map((source) => ({ source, metrics: offered.filter((metric) => metric.source === source) })).filter((group) => group.metrics.length > 0);
  return (
    <form
      className="metrics-board__add"
      onSubmit={(event) => {
        event.preventDefault();
        if (adding !== '' && !chosen.includes(adding)) onAdd(adding);
        setAdding('');
      }}
    >
      <input className="metrics-board__filter" type="search" value={filter} onChange={(event) => setFilter(event.target.value)} placeholder="Find a metric" aria-label="Filter the metrics offered" />
      <select value={adding} onChange={(event) => setAdding(event.target.value)} aria-label="The metric to add a panel of">
        <option value="">{offered.length === 0 ? 'No metric matches' : `Add a panel (${offered.length})`}</option>
        {groups.map((group) => (
          <optgroup key={group.source} label={SOURCE_WORDS[group.source]}>
            {group.metrics.map((metric) => <option key={metric.metric} value={metric.metric}>{metric.label}</option>)}
          </optgroup>
        ))}
      </select>
      <button className="btn btn--small" type="submit" disabled={adding === ''}>Add</button>
    </form>
  );
}

function MetricPanel({ run, attempt, metric, described, cursor, height, live, describedBy, onRemove }: {
  run: string;
  attempt: number;
  metric: string;
  /** Whether the run still goes, which decides whether a ticket refused as not streamed is final. */
  live: boolean;
  /** The catalogue's descriptor, shown while the socket has not yet spoken. */
  described: MetricDescriptor | null;
  cursor: Cursor;
  height: number;
  describedBy?: string;
  onRemove: () => void;
}) {
  // A socket lost past the hook's own tries, not refused for good, is attached afresh on request: a new try is a new
  // key.
  const [tries, setTries] = useState(0);
  const stream = useMetricStream(run, attempt, metric, tries, live);
  const descriptor = stream.descriptor ?? described;
  // The chart of this attach's rows, which the stream appends in place: it takes the rows new since it last caught up
  // when their count moves, so a batch costs only its own rows however long the run, and the cursor costs nothing.
  const chart = useMemo(() => (stream.descriptor === null ? null : new StreamChart(stream.descriptor, stream.rows)), [stream.descriptor, stream.rows]);
  const count = stream.rows.length;
  const drawn = useMemo(() => {
    if (chart === null) return null;
    chart.catchUp();
    return { spec: chart.spec(), said: sentence(chart) };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chart, count]);
  const spec = drawn?.spec ?? null;
  const said = drawn?.said ?? '';
  // The player's cursor on the plot's x axis, in hours of simtime from the run's start.
  const onPlot = useMemo<Cursor>(() => ({
    get: () => {
      const ms = cursor.get();
      return ms === null ? null : hoursOf(ms);
    },
    subscribe: cursor.subscribe,
  }), [cursor]);
  const [hovered, setHovered] = useState<number | null>(null);
  const id = useId();
  const name = descriptor?.label ?? metric;
  const kind: 'series' | 'stat' | 'bars' = descriptor === null || descriptor.bucket.kind !== 'overall' ? 'series' : (descriptor.dimensions ?? []).length > 0 ? 'bars' : 'stat';
  const dimensioned = (descriptor?.dimensions ?? []).length > 0;
  const whole = chart?.whole ?? [];
  const sign = descriptor === null ? '' : unitSign(descriptor.unit, descriptor.clock);
  return (
    <figure className="mpanel" aria-labelledby={`${id}-title`} aria-describedby={describedBy} data-kind={kind} data-state={panelState(stream, descriptor)}>
      <header className="mpanel__head">
        <div className="mpanel__name">
          <figcaption className="mpanel__title" id={`${id}-title`}>{name}</figcaption>
          {sign !== '' && <span className="mpanel__unit">{sign}</span>}
          <StateChip stream={stream} descriptor={descriptor} />
        </div>
        <div className="mpanel__actions">
          {descriptor !== null && <span className="mpanel__info" title={legendWords(stream, descriptor)} aria-hidden="true">i</span>}
          <button className="mpanel__remove" type="button" onClick={onRemove} aria-label={`Remove the panel of ${name}`} title="Remove" data-remove>×</button>
        </div>
      </header>
      <p className="visually-hidden" id={`${id}-note`}>{descriptor === null ? '' : legendWords(stream, descriptor)}</p>
      {kind === 'series' && chart !== null && spec !== null && !dimensioned && <Figures line={chart.lines[0]} whole={wholeValue(whole)} spec={spec} hovered={hovered} descriptor={chart.descriptor} />}
      {kind === 'series' && spec !== null && spec.x.length > 0 && (
        <Chart
          name={said} describedBy={describedBy === undefined ? `${id}-note` : `${describedBy} ${id}-note`} spec={spec} cursor={onPlot}
          interactive height={height} syncKey={SYNC_KEY} onHover={setHovered}
        />
      )}
      {kind === 'series' && spec !== null && spec.x.length === 0 && stream.lost === null && (
        <div className="mpanel__empty" style={{ minHeight: height }} aria-busy={stream.live ? 'true' : undefined}>
          <span className="muted">{stream.live ? 'Waiting for the first bucket' : stream.ended !== null ? 'No bucket was stated' : 'Attaching'}</span>
        </div>
      )}
      {kind === 'series' && chart !== null && spec !== null && dimensioned && chart.lines.length > 0 && <LegendTable lines={chart.lines} spec={spec} hovered={hovered} />}
      {kind === 'series' && descriptor !== null && whole.length > 0 && (
        <details className="mpanel__whole">
          <summary>Whole run</summary>
          <BarList descriptor={descriptor} rows={whole} />
        </details>
      )}
      {kind === 'stat' && descriptor !== null && <StatTile descriptor={descriptor} rows={whole} stream={stream} height={height} />}
      {kind === 'bars' && descriptor !== null && (whole.length > 0 ? <BarList descriptor={descriptor} rows={whole} /> : <Waiting stream={stream} height={height} />)}
      {stream.descriptor === null && stream.lost === null && kind === 'series' && (
        <div className="mpanel__empty" style={{ minHeight: height }} aria-busy="true"><span className="muted">Attaching to the metric stream</span></div>
      )}
      {stream.lost !== null && (
        <p className="mpanel__foot">
          Interrupted: {stream.lost.why}.{' '}
          {stream.lost.next === 'again' && 'Attaching again in a moment.'}
          {stream.lost.next === 'on-request' && <button className="btn btn--small" type="button" onClick={() => setTries((n) => n + 1)}>Attach again</button>}
        </p>
      )}
    </figure>
  );
}

/** Where the panel stands, for its colour: live, ended, waiting on the end, or interrupted. */
function panelState(stream: MetricStream, descriptor: MetricDescriptor | null): string {
  if (stream.lost !== null || stream.lagged) return 'lost';
  if (stream.ended !== null) return 'ended';
  if (stream.live) return 'live';
  if (descriptor?.source === 'end_block' && stream.rows.length === 0) return 'waiting';
  return 'idle';
}

function StateChip({ stream, descriptor }: { stream: MetricStream; descriptor: MetricDescriptor | null }) {
  const state = panelState(stream, descriptor);
  const words = state === 'live' ? 'live' : state === 'ended' ? 'ended' : state === 'waiting' ? 'at the end' : state === 'lost' ? 'interrupted' : null;
  if (words === null) return null;
  return <span className="mpanel__chip" data-state={state}><span className="mpanel__dot" aria-hidden="true" />{words}</span>;
}

/** A panel's large figure, in its `.clock` with the descriptor's clock where it is a duration. */
function Big({ descriptor, className, children }: { descriptor: MetricDescriptor | null; className: string; children: string }) {
  const timed = descriptor !== null && isTime(descriptor.unit);
  return <span className={timed ? `${className} clock` : className} data-clock={timed ? descriptor.clock : undefined}>{children}</span>;
}

/** The one value a metric without dimensions states over the whole run, once the stream has sent it. */
function wholeValue(rows: MetricPoint[]): number | null {
  return rows.find((row) => row.value !== null)?.value ?? null;
}

/** The figures over a single-series plot: the value under the crosshair; else the whole run's where the stream sent
 * one, else the latest window's; then the lowest and highest window. */
function Figures({ line, whole, spec, hovered, descriptor }: {
  line: StreamLine | undefined; whole: number | null; spec: ChartSpec; hovered: number | null; descriptor: MetricDescriptor;
}) {
  const stats = line?.stats ?? null;
  const said = spec.yShort ?? spec.yWords;
  if (line === undefined || stats === null) return <div className="mpanel__figures"><span className="mpanel__big muted">—</span></div>;
  const at = hovered !== null && line.values[hovered] !== null && line.values[hovered] !== undefined ? { value: line.values[hovered] as number, x: spec.x[hovered] } : null;
  const lead = at !== null ? { value: at.value, when: `at ${hourWords(at.x)}` }
    : whole !== null ? { value: whole, when: 'over the whole run' }
      : { value: stats.last, when: `last, at ${hourWords(stats.lastAt)}` };
  return (
    <div className="mpanel__figures">
      <div className="mpanel__lead" data-hovered={at !== null ? 'true' : undefined}>
        <Big descriptor={descriptor} className="mpanel__big">{said(lead.value)}</Big>
        <span className="mpanel__when">{lead.when}</span>
      </div>
      <dl className="mpanel__minor">
        <div><dt>min</dt><dd>{said(stats.min)}</dd></div>
        <div><dt>max</dt><dd>{said(stats.max)}</dd></div>
      </dl>
    </div>
  );
}

/** The legend as a table, one row per line: its colour, its name, the value under the crosshair or its last, and its
 * highest. */
function LegendTable({ lines, spec, hovered }: { lines: StreamLine[]; spec: ChartSpec; hovered: number | null }) {
  const said = spec.yShort ?? spec.yWords;
  return (
    <table className="mpanel__legend">
      <thead>
        <tr><th scope="col"><span className="visually-hidden">line</span></th><th scope="col" className="num">{hovered !== null ? 'at cursor' : 'last'}</th><th scope="col" className="num">max</th></tr>
      </thead>
      <tbody>
        {lines.map((one) => {
          const stats = one.stats;
          const shown = hovered !== null ? one.values[hovered] ?? null : stats?.last ?? null;
          return (
            <tr key={one.label}>
              <th scope="row"><span className="mpanel__key" style={{ background: swatch(one.key) }} aria-hidden="true" />{one.label}</th>
              <td className="num">{shown === null ? '—' : said(shown)}</td>
              <td className="num">{stats === null ? '—' : said(stats.max)}</td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

/** One figure for the whole run, large, with when it was stated; before the end, what it waits on. */
function StatTile({ descriptor, rows, stream, height }: { descriptor: MetricDescriptor; rows: MetricPoint[]; stream: MetricStream; height: number }) {
  const row = rows.find((one) => one.value !== null) ?? null;
  if (row === null) return <Waiting stream={stream} height={height} />;
  return (
    <div className="mpanel__stat" style={{ minHeight: height }}>
      <Big descriptor={descriptor} className="mpanel__huge">{shortValue(descriptor.unit, row.value as number)}</Big>
      <span className="mpanel__when">{valueWords(descriptor.unit, row.value, descriptor.clock)} over the whole run, to <SimSpan ms={row.to_sim_ms} /></span>
    </div>
  );
}

/** A metric only the end block states, without its value: stated once the run ends; or, once the stream has ended
 * without it, no value and why, from how the recording ended. */
function Waiting({ stream, height }: { stream: MetricStream; height: number }) {
  const ended = stream.ended;
  const words = ended === null ? (stream.lost !== null ? 'Not stated' : 'Stated once the run ends')
    : ended === 'no-end-block' || ended === 'recording-incomplete' ? `No value: ${OUTCOME_WORDS[ended]}`
      : 'No value: the end block does not state it';
  return (
    <div className="mpanel__empty" style={{ minHeight: height }} aria-busy={stream.lost === null && ended === null ? 'true' : undefined}>
      <span className="muted">{words}</span>
    </div>
  );
}

/** Whole-run rows by place along the dimensions as a bar list, highest first, each bar against the highest. */
function BarList({ descriptor, rows }: { descriptor: MetricDescriptor; rows: MetricPoint[] }) {
  const stated = rows.filter((row): row is MetricPoint & { value: number } => row.value !== null).sort((a, b) => b.value - a.value);
  const top = stated.reduce((highest, row) => Math.max(highest, Math.abs(row.value)), 0);
  const dimensions = (descriptor.dimensions ?? []).map((dimension) => dimension.name).join(', ');
  return (
    <ol className="barlist" aria-label={`${descriptor.label} by ${dimensions}, over the whole run`}>
      {stated.map((row) => (
        <li key={keyWords(row)} className="barlist__row">
          <span className="barlist__label">{keyWords(row)}</span>
          <span className="barlist__track" aria-hidden="true"><span className="barlist__bar" style={{ width: `${top === 0 ? 0 : (Math.abs(row.value) / top) * 100}%` }} /></span>
          <span className="barlist__value">{shortValue(descriptor.unit, row.value)}</span>
        </li>
      ))}
      {rows.length > stated.length && <li className="barlist__row muted">{rows.length - stated.length} without a value</li>}
    </ol>
  );
}

/** What a bucket is, in the descriptor's own words, and how the run's recording ended once the server said so. */
function legendWords(stream: MetricStream, descriptor: MetricDescriptor): string {
  const bucket = descriptor.bucket;
  const cut = bucket.kind === 'slice'
    ? `one value per ${spanWords(bucket.width_sim_ms)} of simtime`
    : bucket.kind === 'trailing'
      ? `trailing ${spanWords(bucket.window_sim_ms)} of simtime, every ${spanWords(bucket.step_sim_ms)}`
      : 'one value for the whole run, from the end block';
  const when = stream.ended === 'recording-incomplete' ? `; ${OUTCOME_WORDS[stream.ended]}`
    : stream.ended !== null ? `; the run ended: ${OUTCOME_WORDS[stream.ended]}` : stream.live ? '; live' : '';
  return `${descriptor.description} ${cut[0].toUpperCase()}${cut.slice(1)}${when}.`;
}

/** The sentence a screen reader hears for the canvas. */
function sentence(chart: StreamChart): string {
  const said = chart.summary;
  if (said === null) return 'No value stated yet.';
  const { label, unit, clock } = chart.descriptor;
  const words = (value: number) => valueWords(unit, value, clock);
  return `${label}: ${said.count} values to ${spanWords(said.endMs)} of simtime, between ${words(said.low)} and ${words(said.high)}, the last ${words(said.last)}.`;
}
