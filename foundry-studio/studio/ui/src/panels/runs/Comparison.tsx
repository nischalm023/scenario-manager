// Spec 08 section 5's comparison (mockup 10; brief I1.10, task 35), below the list with its runs ticked: `h3`
// Comparison; What differs, every input that differs before any number, the experiment's design apart from what
// differs outside it, whose heading then warns that the results cannot be read as the varied input's effect alone, a
// differing changeset linking its scope's history; the chart (one numeric input varied by design: the first one or
// two headline metrics against it, repeats as their mean with each series' range and n; else a dot chart by run); the
// table, each run's change against the reference run, or, when seeds repeat, each value's mean against the reference
// value's, the reference the person may pick; and Not compared, the runs whose latest attempt did not finish. Every
// metric says the window its values cover, and the varied input's values keep their unit and clock. Runs are named
// by their place in the comparison, since the runs of one plan share their ids' first characters, the millisecond
// they were minted. It reads what the agent's `run.compare` reads, so the agent states nothing the page does not
// show; a stub's numbers carry its sentence; the run it measures against goes to the panel, whose chat follows it.

import { useEffect, useMemo, useState } from 'react';
import type { CompareReply, ComparedMetric, InputField, InputValue, SameField } from '../../protocol/results.ts';
import { clause, getJson } from '../../shell/api.ts';
import { useShell } from '../../shell/context.ts';
import { routeFor } from '../../shell/routes.ts';
import { ShellLink } from '../../shell/ShellLink.tsx';
import { TableScroll } from '../../shell/TableScroll.tsx';
import { Chart } from './Chart.tsx';
import { seriesRange, unitTitle, type ChartSeries, type ChartSpec } from './charts.ts';
import { STUB_WORDS, changeWords, isTime, listWords, spanWords, valueWords } from './resultWords.ts';
import { ClockedValue, MetricValue, SimSpan } from './RunResults.tsx';
import { shortId } from './words.ts';

type Read = { state: 'reading' } | { state: 'read'; value: CompareReply } | { state: 'failed'; reason: string };

/** The scenario field a repeat varies (spec 07 section 3.6), and the prefix of a read scope's provenance field. */
const SEED = '/run_controls/seed';
const READ = 'read:';

function Value({ value, clock }: { value: string | null; clock?: InputField['clock'] }) {
  if (value === null) return <>not recorded</>;
  return clock === undefined ? <>{value}</> : <span className="clock" data-clock={clock}>{value}</span>;
}

/** One run's value of a differing input; a changeset of a scope it read links that scope's history there (spec 03
 * section 3.2's History route), so the reader can see what changed between the numbers. */
function Differs({ field, one }: { field: InputField; one: InputValue }) {
  const scope = field.field.startsWith(READ) ? field.field.slice(READ.length) : null;
  if (scope === null || one.value === null || one.changeset === undefined) return <Value value={one.value} clock={field.clock} />;
  const history = routeFor({ view: 'world', location: { scope, pin: { kind: 'changeset', seq: one.changeset } }, show: 'history' });
  return <ShellLink to={history}>{one.value}</ShellLink>;
}

/** The window a compared metric's values cover (spec 08 section 3.9, every value states its window): a gauge's last
 * reading, an additive metric over the report window, any other class over the whole run. */
function MetricWindow({ metric }: { metric: ComparedMetric }) {
  const window = metric.values[0]?.window;
  if (window === undefined) return null;
  if (metric.aggregation === 'snapshot_gauge') return <>at <SimSpan ms={window.to_sim_ms} />, its last reading</>;
  if (metric.aggregation === 'additive') return <>over <SimSpan ms={window.from_sim_ms} to={window.to_sim_ms} /></>;
  return <>whole run, <SimSpan ms={window.from_sim_ms} to={window.to_sim_ms} /></>;
}

function windowWords(metric: ComparedMetric): string {
  const window = metric.values[0]?.window;
  if (window === undefined) return '';
  const span = `${spanWords(window.from_sim_ms)} to ${spanWords(window.to_sim_ms)} of simtime`;
  if (metric.aggregation === 'snapshot_gauge') return `at ${spanWords(window.to_sim_ms)} of simtime, its last reading`;
  return metric.aggregation === 'additive' ? `over ${span}` : `whole run, ${span}`;
}

const tickWords = (value: number): string => value.toLocaleString('en-US', { maximumFractionDigits: 20 });

function Change({ metric, change }: { metric: ComparedMetric; change: number | null }) {
  if (change === null) return <>no value</>;
  const clock = metric.clock ?? 'sim';
  if (!isTime(metric.unit)) return <>{changeWords(metric.unit, change, clock)}</>;
  return <ClockedValue clock={clock}>{changeWords(metric.unit, change, clock)}</ClockedValue>;
}

interface Stat {
  mean: number | null;
  low: number | null;
  high: number | null;
  n: number;
}

/** One point of the chart: a value of the varied input, or a run's place, with each comparable metric's mean, low,
 * high and n; `label` says the value in its words and unit ("15 minutes of simtime"), `tick` its number alone, the
 * axis title naming the unit. */
interface Group {
  x: number;
  label: string;
  tick: string;
  runs: string[];
  stats: Stat[];
}

interface Grouped {
  varied: InputField | null;
  metrics: ComparedMetric[];
  groups: Group[];
}

/** The finished runs grouped by the one numeric input the experiment varied, or one group a run when none was. */
function grouped(reply: CompareReply): Grouped {
  const left = new Set(reply.not_compared.map((one) => one.run));
  const runs = reply.runs.filter((run) => !left.has(run.id)).map((run) => run.id);
  const metrics = reply.metrics.filter((metric) => metric.comparable);
  // The repeats' seeds are what the mean is taken over, so they are not the input the chart is drawn against.
  const design = reply.differs.filter((field) => field.by_experiment && field.section !== 'provenance' && field.field !== SEED);
  const varied = design.length === 1 && design[0].values.every((one) => one.number !== undefined) ? design[0] : null;
  const place = (run: string) => reply.runs.findIndex((one) => one.id === run) + 1;
  const members = new Map<number, string[]>();
  for (const run of runs) {
    const x = varied === null ? place(run) : (varied.values.find((one) => one.run === run)?.number ?? place(run));
    members.set(x, [...(members.get(x) ?? []), run]);
  }
  const groups = [...members.entries()].sort(([a], [b]) => a - b).map(([x, at]) => ({
    x,
    label: varied === null ? `run ${place(at[0])}` : (varied.values.find((one) => one.run === at[0])?.value ?? tickWords(x)),
    tick: varied === null ? `run ${place(at[0])}` : tickWords(x),
    runs: at,
    stats: metrics.map((metric) => {
      const values = at.map((run) => metric.values.find((one) => one.run === run)?.value ?? null).filter((value): value is number => value !== null);
      if (values.length === 0) return { mean: null, low: null, high: null, n: 0 };
      return { mean: values.reduce((sum, value) => sum + value, 0) / values.length, low: Math.min(...values), high: Math.max(...values), n: values.length };
    }),
  }));
  return { varied, metrics, groups };
}

export function Comparison({ ids, reference, onReference }: {
  ids: string[]; reference?: string;
  /** Told the run the comparison measures against once the API names it, the asked one or the first if it fails. */
  onReference?(run: string): void;
}) {
  const shell = useShell();
  const [read, setRead] = useState<{ key: string; read: Read } | null>(null);
  const key = `${ids.join(',')}|${reference ?? ''}`;
  useEffect(() => {
    let current = true;
    const query = `ids=${ids.map(encodeURIComponent).join(',')}${reference !== undefined ? `&reference=${encodeURIComponent(reference)}` : ''}`;
    void getJson<CompareReply>(`/api/runs/compare?${query}`).then((answer) => {
      if (!current) return;
      if (answer.ok && Array.isArray(answer.value?.differs)) {
        setRead({ key, read: { state: 'read', value: answer.value } });
        onReference?.(answer.value.reference);
      } else {
        setRead({ key, read: { state: 'failed', reason: answer.ok ? 'the service answered something else than a comparison' : clause(answer.reason) } });
        onReference?.(reference ?? ids[0]);
      }
    });
    return () => {
      current = false;
    };
  }, [ids, reference, key, onReference]);
  const shown: Read = read !== null && read.key === key ? read.read : { state: 'reading' };
  const groups = useMemo(() => (read !== null && read.read.state === 'read' ? grouped(read.read.value) : null), [read]);
  const chart = useMemo(() => (groups === null ? null : chartOf(groups)), [groups]);
  if (shown.state === 'failed') return <p className="panel__status">The runs could not be compared: {shown.reason}.</p>;
  if (shown.state === 'reading' || groups === null) return <p className="panel__status" aria-busy="true">Comparing the runs</p>;
  const compared = shown.value;
  const labelOf = (run: string) => compared.runs.find((one) => one.id === run)?.label ?? shortId(run);
  const placeOf = (run: string) => compared.runs.findIndex((one) => one.id === run) + 1;
  const left = new Set(compared.not_compared.map((one) => one.run));
  const counted = compared.runs.filter((run) => !left.has(run.id));
  const stub = counted.some((run) => run.stub);
  const describedBy = stub ? 'compare-stub' : undefined;
  const warns = compared.differs.some((field) => field.warns);
  const repeats = groups.groups.some((group) => group.runs.length > 1);
  // With repeats the table measures each value's mean against the reference value's, so the picker offers values: a
  // pick of another repeat of one value would change nothing.
  const byValue = repeats && groups.varied !== null;
  const base = groups.groups.find((group) => group.runs.includes(compared.reference)) ?? groups.groups[0];
  const pick = (to: string) => shell.navigate(routeFor({ view: 'compare', ids, reference: to }), 'within');
  return (
    <section className="section">
      <h3 className="section__title" id="compare-title" tabIndex={-1}>Comparison</h3>
      <p>
        {counted.length} of {compared.runs.length} runs compared.{' '}
        {compared.differs.length === 0 ? 'Every input is the same in all of them.'
          : warns ? 'They differ outside the experiment as well as in what it varied.'
            : 'They differ only in what the experiment varied, so the results below can be read as its effect.'}
      </p>
      {stub && <p className="field__hint" id="compare-stub">{STUB_WORDS}</p>}
      <h4 className="section__title" id="compare-differs">
        {warns ? <>What differs: <span className="pill" data-tone="warn">results cannot be read as the effect of the varied input alone</span></> : 'What differs'}
      </h4>
      <dl className="provenance">
        <dt>Differs, varied by the experiment</dt>
        <dd><Differing fields={compared.differs.filter((field) => field.by_experiment)} placeOf={placeOf} /></dd>
        <dt>Differs outside the experiment</dt>
        <dd><Differing fields={compared.differs.filter((field) => !field.by_experiment)} placeOf={placeOf} /></dd>
        <dt>Same in all</dt>
        <dd>{compared.same.length === 0 ? 'nothing' : <Same fields={compared.same} />}</dd>
      </dl>
      {chart === null
        ? <p className="muted">No headline metric is comparable across these runs, so nothing is drawn.</p>
        : (
          <figure className="chart" aria-labelledby="compare-chart-title">
            <figcaption className="chart__title" id="compare-chart-title">{chart.title}</figcaption>
            <p className="chart__legend" id="compare-chart-note">
              {chart.drawn.map((metric, at) => (
                <span key={metric.metric}>
                  {at > 0 && '; '}<span className={`chart__key chart__key--${at + 1}`} aria-hidden="true" />{metric.label}, series {at + 1},{' '}
                  <MetricWindow metric={metric} />
                </span>
              ))}
              . The table below holds the same numbers.
            </p>
            <Chart name={chart.sentence} describedBy={[describedBy, 'compare-chart-note'].filter(Boolean).join(' ')} spec={chart.spec} />
          </figure>
        )}
      {byValue && groups.varied !== null
        ? <ByValue grouped={groups} varied={groups.varied} base={base} describedBy={describedBy} />
        : <ByRun compared={compared} metrics={groups.metrics} placeOf={placeOf} labelOf={labelOf} describedBy={describedBy} />}
      {compared.metrics.some((metric) => !metric.comparable) && (
        <ul>
          {compared.metrics.filter((metric) => !metric.comparable).map((metric) => (
            <li key={metric.metric}>{metric.label}: not comparable: {metric.reason}</li>
          ))}
        </ul>
      )}
      {counted.length > 1 && (
        <p>
          <label htmlFor="compare-reference">Measure changes against </label>
          {byValue ? (
            <select className="select" id="compare-reference" value={base.runs[0]} onChange={(event) => pick(event.target.value)} style={{ width: 'auto' }}>
              {groups.groups.map((group) => <option key={group.x} value={group.runs[0]}>{group.label}, {runsWords(group.runs.length)}</option>)}
            </select>
          ) : (
            <select className="select" id="compare-reference" value={compared.reference} onChange={(event) => pick(event.target.value)} style={{ width: 'auto' }}>
              {counted.map((run) => <option key={run.id} value={run.id}>Run {placeOf(run.id)}, {run.label}</option>)}
            </select>
          )}
        </p>
      )}
      <h4 className="section__title">Not compared</h4>
      {compared.not_compared.length === 0 ? <p>None: every run finished.</p> : (
        <ul>
          {compared.not_compared.map((one) => (
            <li key={one.run}>
              <ShellLink to={routeFor({ view: 'run', id: one.run })}>Run {placeOf(one.run)}</ShellLink>, {labelOf(one.run)}: {one.state}
              {one.stop_reason && <>, {one.stop_reason}</>}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** One row a run: the differing inputs, each headline metric's value and its change against the reference run. */
function ByRun({ compared, metrics, placeOf, labelOf, describedBy }: {
  compared: CompareReply; metrics: ComparedMetric[]; placeOf(run: string): number; labelOf(run: string): string; describedBy?: string;
}) {
  const left = new Set(compared.not_compared.map((one) => one.run));
  const inputs = compared.differs.filter((field) => field.section !== 'provenance');
  const reference = compared.reference;
  return (
    <TableScroll name="Comparison">
      <table className="list-table" aria-describedby={describedBy}>
        <caption>
          The comparison as a table: each run's headline metrics, each over the window its heading names, and its change
          against the reference run, run {placeOf(reference)}, {labelOf(reference)}.
        </caption>
        <thead>
          <tr>
            <th scope="col">Run</th>
            {inputs.map((field) => <th scope="col" key={field.field}>{field.label}</th>)}
            {metrics.map((metric) => <MetricHeads key={metric.metric} metric={metric} />)}
          </tr>
        </thead>
        <tbody>
          {compared.runs.filter((run) => !left.has(run.id)).map((run) => (
            <tr key={run.id}>
              <th scope="row">
                <ShellLink to={routeFor({ view: 'run', id: run.id })}>Run {placeOf(run.id)}</ShellLink>, {run.label}
                {run.id === reference && ' (reference)'}
              </th>
              {inputs.map((field) => (
                <td key={field.field}><Value value={field.values.find((one) => one.run === run.id)?.value ?? null} clock={field.clock} /></td>
              ))}
              {metrics.map((metric) => {
                const value = metric.values.find((one) => one.run === run.id)?.value ?? null;
                const base = metric.values.find((one) => one.run === reference)?.value ?? null;
                return [
                  <td className="num" key={`${metric.metric}-value`}><MetricValue unit={metric.unit} clock={metric.clock} value={value} /></td>,
                  <td className="num" key={`${metric.metric}-change`}><Change metric={metric} change={value === null || base === null ? null : value - base} /></td>,
                ];
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </TableScroll>
  );
}

const runsWords = (n: number): string => (n === 1 ? '1 run' : `${n} runs`);

/** One row a value of the varied input when seeds repeat: each metric's mean over its repeats, their range and n, and
 * the mean's change against the reference value's mean, mean against mean. */
function ByValue({ grouped: { metrics, groups }, varied, base, describedBy }: {
  grouped: Grouped; varied: InputField; base: Group; describedBy?: string;
}) {
  return (
    <TableScroll name="Comparison">
      <table className="list-table" aria-describedby={describedBy}>
        <caption>
          The comparison as a table: for each value of {varied.label.toLowerCase()}, the mean of its repeats' headline
          metrics, each over the window its heading names, their range and how many ran, and the mean's change against
          the reference value's mean, <Value value={base.label} clock={varied.clock} /> (n {base.runs.length}).
        </caption>
        <thead>
          <tr>
            <th scope="col">{varied.label}</th>
            {metrics.map((metric) => <MetricHeads key={metric.metric} metric={metric} range />)}
          </tr>
        </thead>
        <tbody>
          {groups.map((group) => (
            <tr key={group.x}>
              <th scope="row"><Value value={group.label} clock={varied.clock} /> ({runsWords(group.runs.length)}){group === base && ', reference'}</th>
              {metrics.map((metric, at) => {
                const stat = group.stats[at];
                const from = base.stats[at].mean;
                return [
                  <td className="num" key={`${metric.metric}-mean`}><MetricValue unit={metric.unit} clock={metric.clock} value={stat.mean} /></td>,
                  <td className="num" key={`${metric.metric}-range`}>
                    {stat.low === null || stat.high === null ? 'no value'
                      : <><MetricValue unit={metric.unit} clock={metric.clock} value={stat.low} /> to <MetricValue unit={metric.unit} clock={metric.clock} value={stat.high} />, n {stat.n}</>}
                  </td>,
                  <td className="num" key={`${metric.metric}-change`}><Change metric={metric} change={stat.mean === null || from === null ? null : stat.mean - from} /></td>,
                ];
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </TableScroll>
  );
}

function MetricHeads({ metric, range = false }: { metric: ComparedMetric; range?: boolean }) {
  return (
    <>
      <th scope="col" className="num">
        {range ? `${metric.label}, mean` : metric.label}<br /><span className="muted"><MetricWindow metric={metric} /></span>
      </th>
      {range && <th scope="col" className="num">Range<span className="visually-hidden"> of {metric.label}</span></th>}
      <th scope="col" className="num">Change<span className="visually-hidden"> in {metric.label}</span></th>
    </>
  );
}

/** The differing inputs of one kind, each with every run's value, or "nothing". */
function Differing({ fields, placeOf }: { fields: InputField[]; placeOf(run: string): number }) {
  if (fields.length === 0) return <>nothing</>;
  return (
    <ul className="compare-differs">
      {fields.map((field) => (
        <li key={field.field}>
          {field.warns && <><span className="pill" data-tone="warn">outside the experiment</span>{' '}</>}
          {field.label}:{' '}
          {field.values.map((one, at) => (
            <span key={one.run}>{at > 0 && (at === field.values.length - 1 ? ' and ' : ', ')}<Differs field={field} one={one} /> on run {placeOf(one.run)}</span>
          ))}
          {field.changed.length > 0 && <>; it changed {listWords(field.changed)}</>}
        </li>
      ))}
    </ul>
  );
}

function Same({ fields }: { fields: SameField[] }) {
  return (
    <>
      {fields.map((field, at) => (
        <span key={field.field}>{at > 0 && '; '}{field.label} <Value value={field.value} clock={field.clock} /></span>
      ))}
    </>
  );
}

/** Section 3.12's chart from the groups: the first one or two comparable metrics of one unit and clock, a line
 * against the varied input with each series' range at each value where repeats were averaged, else dots by run; its
 * sentence names each metric's window and each value in its unit. */
function chartOf({ varied, metrics, groups }: Grouped): { title: string; sentence: string; drawn: ComparedMetric[]; spec: ChartSpec } | null {
  if (metrics.length === 0 || groups.length === 0) return null;
  const drawn = metrics.slice(0, metrics[1]?.unit === metrics[0].unit && metrics[1]?.clock === metrics[0].clock ? 2 : 1);
  const clock = drawn[0].clock ?? 'sim';
  const repeats = groups.some((group) => group.runs.length > 1);
  const xs = groups.map((group) => group.x);
  const span = xs.length > 1 ? xs[xs.length - 1] - xs[0] : 1;
  const yWords = (value: number) => valueWords(drawn[0].unit, value, clock);
  const against = varied === null ? 'by run' : `against ${varied.label.toLowerCase()}`;
  const said = drawn.map((metric, at) => {
    const values = groups.map((group) => {
      const stat = group.stats[at];
      const value = stat.mean === null ? 'no value' : yWords(stat.mean);
      return stat.n > 1 ? `${value} at ${group.label} (the mean of ${stat.n}, from ${yWords(stat.low as number)} to ${yWords(stat.high as number)})` : `${value} at ${group.label}`;
    });
    return `${metric.label}, ${windowWords(metric)}: ${listWords(values)}`;
  });
  const names = listWords(drawn.map((metric) => metric.label));
  const series: ChartSeries[] = drawn.map((metric, at) => ({
    label: metric.label,
    values: groups.map((group) => group.stats[at].mean),
    key: (at + 1) as 1 | 2,
    ranges: groups.flatMap((group) => {
      const stat = group.stats[at];
      return stat.n > 1 && stat.low !== null && stat.high !== null ? [{ at: group.x, low: stat.low, high: stat.high }] : [];
    }),
  }));
  return {
    title: `${names} ${against}${repeats ? ', the mean of each value\'s repeats with its range' : ''}`,
    sentence: `${names} ${against}. ${said.join('. ')}`,
    drawn,
    spec: {
      x: xs,
      series,
      xRange: [xs[0] - span * 0.1, xs[xs.length - 1] + span * 0.1],
      xTicks: groups.map((group) => ({ at: group.x, label: group.tick })),
      xTitle: varied === null ? 'run' : `${varied.label.toLowerCase()}${varied.unit !== undefined ? `, ${varied.unit}` : ''}`,
      yTitle: unitTitle(drawn[0].unit, clock),
      yRange: seriesRange(series),
      shape: varied === null ? 'dots' : 'line',
      yWords,
    },
  };
}
