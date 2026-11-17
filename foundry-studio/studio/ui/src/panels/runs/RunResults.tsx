// Spec 08 section 5's results of an ended run (mockup 09; brief I1.10, task 34), below its heading: Summary, the
// headline metrics as tiles over the report window, beside whatever the page puts `beside` it (its compact player);
// then whatever the page puts `between` (its inputs and metric panels); then every declared metric in "All metrics",
// grouped by family. The headline metrics are the API's choice (section 3.9's rule), so no metric is named here; their
// code names are spelled out by `metricName`. A stub attempt's numbers carry the stub's sentence once a section, every
// tile and table described by it (ruling D110); an unfinished attempt's say how far it got. Every time sits in its
// `.clock`.

import { useEffect, useState, type ReactNode } from 'react';
import type { DurationClock } from '../../protocol/clock.ts';
import type { MetricRow, MetricsReply, SimWindow } from '../../protocol/results.ts';
import type { MetricUnit } from '../../protocol/runner.ts';
import type { RunDetail } from '../../protocol/runs.ts';
import { clause, getJson } from '../../shell/api.ts';
import { TableScroll } from '../../shell/TableScroll.tsx';
import { metricFamily, metricName } from './metricWords.ts';
import { isTime, kindWords, listWords, spanWords, stubSentence, valueWords, wholeHours } from './resultWords.ts';

/** A span or instant of simtime in its `.clock`, tagged where it closes its phrase. */
export function SimSpan({ ms, to, tagged = true }: { ms: number; to?: number; tagged?: boolean }) {
  return (
    <span className="clock" data-clock="sim">
      {spanWords(ms)}{to !== undefined && ` to ${spanWords(to)}`}{tagged && <>{' '}<span className="clock__tag">simtime</span></>}
    </span>
  );
}

/** A metric's value in its unit, a duration in the clock the API states for it (simtime's for an attempt stored
 * before results named one), tagged when that is simtime, and a rate per the hour of that clock. */
export function MetricValue({ unit, value, clock = 'sim' }: { unit: MetricUnit; value: number | null; clock?: DurationClock }) {
  if (value === null || !isTime(unit)) return <>{valueWords(unit, value, clock)}</>;
  return <ClockedValue clock={clock}>{valueWords(unit, value, clock)}</ClockedValue>;
}

/** Words of a time in its `.clock`: simtime's tagged, the wall clock's untagged as every wall-clock time on a page. */
export function ClockedValue({ clock, children }: { clock: DurationClock; children: ReactNode }) {
  if (clock === 'wall') return <span className="clock" data-clock="wall">{children}</span>;
  return <span className="clock" data-clock="sim">{children} <span className="clock__tag">simtime</span></span>;
}

const sameWindow = (a: SimWindow, b: SimWindow): boolean => a.from_sim_ms === b.from_sim_ms && a.to_sim_ms === b.to_sim_ms;

type Read<T> = { state: 'reading' } | { state: 'read'; value: T } | { state: 'failed'; reason: string };

export function RunResults({ run, highlights, beside, between }: { run: RunDetail; highlights?: number; beside?: ReactNode; between?: ReactNode }) {
  const [metrics, setMetrics] = useState<{ key: string; read: Read<MetricsReply> } | null>(null);
  const key = `${run.id}/${run.attempt}/${highlights ?? ''}`;
  useEffect(() => {
    let current = true;
    const query = `attempt=${run.attempt}${highlights !== undefined ? `&highlights=${highlights}` : ''}`;
    void (async () => {
      const answer = await getJson<MetricsReply>(`/api/runs/${run.id}/metrics?${query}`);
      if (!current) return;
      if (!answer.ok || !Array.isArray(answer.value?.items) || !Array.isArray(answer.value.headlines)) {
        setMetrics({ key, read: { state: 'failed', reason: answer.ok ? 'the service answered something else than metrics' : clause(answer.reason) } });
        return;
      }
      setMetrics({ key, read: { state: 'read', value: answer.value } });
    })();
    return () => {
      current = false;
    };
  }, [run.id, run.attempt, highlights, key]);
  const read: Read<MetricsReply> = metrics !== null && metrics.key === key ? metrics.read : { state: 'reading' };
  const stub = stubSentence(run);
  const split = (summary: ReactNode) => (beside === undefined ? summary : <div className="run-split">{beside}{summary}</div>);
  if (read.state === 'reading') return <>{split(<section className="section"><h4 className="section__title">Summary</h4><p className="panel__status" aria-busy="true">Reading the results</p></section>)}{between}</>;
  if (read.state === 'failed') return <>{split(<section className="section"><h4 className="section__title">Summary</h4><p className="panel__status">The results could not be read: {read.reason}.</p></section>)}{between}</>;
  const reply = read.value;
  const unfinished = run.state !== 'finished';
  const reached = run.progress.sim_ms;
  return (
    <>
      {split(<Summary run={run} reply={reply} stub={stub} unfinished={unfinished} reached={reached} />)}
      {between}
      {reply.items.length > 0 && <AllMetrics reply={reply} before={run.report_window.before_name || 'start'} describedBy={stub !== null ? 'run-stub-summary' : undefined} />}
    </>
  );
}

function Summary({ run, reply, stub, unfinished, reached }: {
  run: RunDetail; reply: MetricsReply; stub: string | null; unfinished: boolean; reached: number;
}) {
  const describedBy = stub !== null ? 'run-stub-summary' : undefined;
  const window = reply.window;
  const before = run.report_window.before_name || 'start';
  const narrowed = window.from_sim_ms > 0 || run.report_window.to_sim_ms !== null;
  const headline = reply.headlines.map((metric) => reply.items.find((row) => row.metric === metric)).filter((row): row is MetricRow => row !== undefined);
  const tiles = headline.filter((row) => !narrowed || sameWindow(row.window, window));
  const moved = headline.filter((row) => !tiles.includes(row));
  return (
    <section className="section run-summary">
      <h4 className="section__title">
        Summary
        <span className="muted run-card__note">
          {' '}over <SimSpan ms={window.from_sim_ms} to={window.to_sim_ms} />{wholeHours(window) && `, ${wholeHours(window)}`}
          {unfinished && <>, up to <SimSpan ms={reached} />; the run did not finish</>}
        </span>
      </h4>
      {reply.items.length === 0 ? (
        <p className="panel__status">
          {run.recording.stored ? 'The Worldview service has not reported summary metrics for this attempt.'
            : <>This attempt stored no results: it stopped at <SimSpan ms={reached} /> and no metrics were stored.</>}
        </p>
      ) : (
        <>
          {stub !== null && <p className="field__hint" id="run-stub-summary">{stub}</p>}
          <ul className="metric-tiles" aria-label="Headline metrics">
            {tiles.map((row) => (
              <li className="metric-tile" key={row.metric} aria-describedby={describedBy} title={row.description || row.metric}>
                <span className="metric-tile__label">{metricName(row.metric, row.label)}</span>
                <span className="metric-tile__value"><MetricValue unit={row.unit} value={row.value} clock={row.clock} /></span>
                <span className="metric-tile__note">
                  {sameWindow(row.window, window) ? 'the report window' : <WindowNote row={row} window={window} before={before} />}
                  {unfinished && <>; up to <SimSpan ms={reached} />, the run did not finish</>}
                </span>
              </li>
            ))}
          </ul>
          {moved.length > 0 && (
            <p className="muted">
              {listWords(moved.map((row) => metricName(row.metric, row.label)))} {moved.length === 1 ? 'is a whole-run value' : 'are whole-run values'}, including
              the {before}, so {moved.length === 1 ? 'it is' : 'they are'} listed under All metrics rather than beside values for the
              report window.
            </p>
          )}
        </>
      )}
    </section>
  );
}

/** Every declared metric, grouped by family so a hundred rows read as a dozen lists; the code name stays beside each. */
function AllMetrics({ reply, before, describedBy }: { reply: MetricsReply; before: string; describedBy?: string }) {
  const families = new Map<string, MetricRow[]>();
  for (const row of reply.items) {
    const family = metricFamily(row.metric);
    families.set(family, [...(families.get(family) ?? []), row]);
  }
  const stated = reply.items.filter((row) => row.value !== null && row.value !== 0).length;
  return (
    <details className="run-card run-card--fold">
      <summary>
        <h4 className="run-card__title">
          All metrics ({reply.items.length})
          <span className="muted run-card__note"> every metric the run declared, {stated} with a value other than zero</span>
        </h4>
      </summary>
      <div className="run-card__body">
        <TableScroll name="All metrics">
          <table className="list-table all-metrics" aria-describedby={describedBy}>
            <caption>Every metric the run declared, grouped by what it measures, with the window each value covers and the name the simulator declares it by.</caption>
            <thead>
              <tr><th scope="col">Metric</th><th scope="col" className="num">Value</th><th scope="col">Kind</th><th scope="col">Window</th><th scope="col">Declared as</th></tr>
            </thead>
            {[...families.entries()].map(([family, rows]) => (
              <tbody key={family}>
                <tr className="all-metrics__family"><th scope="rowgroup" colSpan={5}>{family}</th></tr>
                {rows.map((row) => (
                  <tr key={row.metric} data-zero={row.value === 0 ? 'true' : undefined}>
                    <th scope="row">{metricName(row.metric, row.label)}</th>
                    <td className="num"><MetricValue unit={row.unit} value={row.value} clock={row.clock} /></td>
                    <td>{kindWords(row)}</td>
                    <td><WindowNote row={row} window={reply.window} before={before} /></td>
                    <td><code>{row.metric}</code></td>
                  </tr>
                ))}
              </tbody>
            ))}
          </table>
        </TableScroll>
      </div>
    </details>
  );
}

/** The window a value covers (spec 08 section 3.9, every value states its window). */
function WindowNote({ row, window, before }: { row: MetricRow; window: SimWindow; before: string }): ReactNode {
  if (row.aggregation === 'snapshot_gauge') return <>at <SimSpan ms={row.window.to_sim_ms} />, its last reading</>;
  if (sameWindow(row.window, window)) return <>over <SimSpan ms={row.window.from_sim_ms} to={row.window.to_sim_ms} /></>;
  return window.from_sim_ms > 0 ? <>whole run, including the {before}</> : <>whole run</>;
}
