// Spec 08 section 5, the open run's page (mockup 08): its `h3` and its status line, the ending's detail beneath (a stub
// attempt's sentence among them, ruling D110, so no figure of a stub attempt shows without it). While it is queued or
// running, its picture as the streaming server sends it (`RunPlayback`, from its `/frames`) beside its progress tiles
// and a native `progress` in simtime whose value moves at most once a minute of wall clock, and Cancel, which never
// asks, keeps focus reading "Stopping…" until the ending and hands focus to the `h3` as it leaves. Once it has ended
// (brief I1.10): its picture from the same socket, so a page opened a week later reads the run as one that watched it,
// played small beside the Summary (`RunResults`), filling the window on request, unless the page watched it run, whose
// player then stays where it stood with its camera, cursor and frames. Under the picture and the Summary: its inputs as
// values (`Inputs`), what the run was triggered with, then the metric panels (`MetricCharts`), the player's cursor
// drawn on every plot; once it has ended, all its metrics; folded, the provenance of every pin. Every time value sits
// in its `.clock` with its clock, the simtime tag shown once per phrase and wall clock, the default, untagged
// (LEAD-DECISIONS entry 56).

import { useEffect, useRef, useState } from 'react';
import { runEnded, simtimeWords } from '../../chat/words.ts';
import type { CancelAnswer, RunDetail } from '../../protocol/runs.ts';
import { clause, postJson } from '../../shell/api.ts';
import { useShell } from '../../shell/context.ts';
import { useFillWindow } from '../../shell/fillWindow.ts';
import { pressKey } from '../scenario/api.ts';
import { Inputs } from './Inputs.tsx';
import { MetricCharts } from './MetricCharts.tsx';
import { useLiveProgress } from './liveProgress.ts';
import { playerCursor } from './playerCursor.ts';
import { stubSentence } from './resultWords.ts';
import { RunPlayback } from './RunPlayback.tsx';
import { RunResults } from './RunResults.tsx';
import { stateWords, wallWords } from './words.ts';

/** A simtime value in its `.clock`, `tagged` where the phrase it closes needs the clock's tag, which a screen reader
 * hears as a word of its own. */
export function Sim({ ms, tagged = true }: { ms: number; tagged?: boolean }) {
  return (
    <span className="clock" data-clock="sim">
      {simtimeWords(ms)}{tagged && <>{' '}<span className="clock__tag">simtime</span></>}
    </span>
  );
}

/** A wall-clock span in its `.clock`, untagged: wall clock is the default (LEAD-DECISIONS entry 56). */
export function Wall({ ms }: { ms: number }) {
  return <span className="clock" data-clock="wall">{wallWords(ms)}</span>;
}

/** How often the progress bar may move, so NVDA's progress beeps stay rare through a sweep (spec 08 section 5). */
const PROGRESS_EVERY_MS = 60_000;

type Attempt = RunDetail['attempts'][number];

export function OpenRun({ run, reread, highlights }: { run: RunDetail; reread(): void; highlights?: number }) {
  const shell = useShell();
  const title = useRef<HTMLHeadingElement>(null);
  const [asking, setAsking] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [refused, setRefused] = useState<string | null>(null);
  const done = runEnded(run.state);
  const state = stateWords(run);
  const latest = run.attempts.at(-1);
  const live = useLiveProgress(run.id, run.attempt, run.state === 'running');
  const stub = stubSentence(run);
  // Every figure of a stub attempt is described by its sentence (ruling D110), which the ending's detail states.
  const stubDescribed = stub !== null && run.stop_detail ? 'run-stop-detail' : undefined;
  // A page opened before the run ended keeps its player through the ending, so the camera, the cursor and the frames
  // stay.
  const [openedEarly] = useState(!done);
  // The player's cursor, drawn on the metric panels' plots: a store the player sets, so a moving cursor renders nothing
  // of the page.
  const [cursor] = useState(playerCursor);
  const streamed = latest?.streaming_server !== undefined;
  // What a streaming server can serve of the attempt, to the picture and the metric panels alike, since both read the
  // same copy: while it runs, the stream it was given; once it has ended, a recording it streamed or the store holds.
  const served = done ? streamed || run.recording.stored : streamed;
  // What fills the window is named for the picture it shows: the run's live view, or its playback once it has ended.
  const fill = useFillWindow(done ? `Playback of ${run.label}` : `Live view of ${run.label}`);
  // Under the playback and the summary: the inputs, then the metric panels; the same two while the run still goes.
  const middle = (
    <>
      <section className="run-card run-card--inputs">
        <h4 className="run-card__title">Inputs <span className="muted run-card__note"> what this run was triggered with</span></h4>
        <Inputs run={run.id} attempt={run.attempt} manifestIn={latest?.compile !== undefined} />
      </section>
      {served && <MetricCharts run={run.id} attempt={run.attempt} cursor={cursor} live={!done} describedBy={stubDescribed} />}
    </>
  );

  // Focus on Cancel when the run ends goes to the run's heading as the button leaves, never to `body`. A blur with
  // nowhere to go is the button's removal, so only focus moving to another element clears the mark.
  const cancelFocused = useRef(false);
  useEffect(() => {
    const lost = document.activeElement === null || document.activeElement === document.body;
    if (done && cancelFocused.current && lost) title.current?.focus();
  }, [done]);

  const cancel = async () => {
    if (asking || stopping) return;
    setAsking(true);
    const answer = await postJson<CancelAnswer>(`/api/runs/${run.id}/cancel`, {}, pressKey());
    setAsking(false);
    if (!answer.ok) {
      const words = `Not cancelled: ${clause(answer.reason)}.`;
      setRefused(words);
      shell.announce(words);
      return;
    }
    setRefused(null);
    if (answer.value.outcome === 'stopping') setStopping(true);
    reread();
  };

  return (
    <section className="section run-page" data-ref={run.ref}>
      <header className="run-page__head">
        <div className="run-page__title">
          <h3 className="section__title" id="run-title" tabIndex={-1} ref={title}>{run.label} <code>{run.id}</code></h3>
          <p id="run-status" className="run-page__status">
            <span className="pill" data-tone={state.tone}>{state.glyph !== null && <span aria-hidden="true">{state.glyph}</span>}{state.words}</span>
            {', '}attempt {run.attempt}
            {run.findings.includes('plan-differs') && <> <span className="pill" data-tone="warn">differs from the check</span></>}
            {live?.state === 'live' && <> <span className="pill" data-tone="good"><span className="live-dot" aria-hidden="true" />live</span></>}
          </p>
          {run.stop_detail && <p id="run-stop-detail" className="muted">{run.stop_detail}</p>}
        </div>
        <div className="run-page__actions">
          {done && run.recording.stored && (
            <a className="btn btn--pill" href={`/api/runs/${run.id}/recording?attempt=${run.attempt}`} download id="run-recording"
              aria-describedby={stubDescribed}>
              {stub !== null ? 'Recording (.worldview), canned by the stub runner' : 'Download the recording'}
            </a>
          )}
          {!done && (
            <button
              className="btn btn--pill btn--danger" type="button" id="run-cancel" onClick={() => void cancel()}
              onFocus={() => (cancelFocused.current = true)}
              onBlur={(event) => (cancelFocused.current = event.relatedTarget === null)}
              aria-disabled={stopping || asking ? true : undefined} aria-describedby={refused !== null ? 'run-cancel-error' : undefined}
            >
              {stopping ? 'Stopping…' : 'Cancel the run'}
            </button>
          )}
        </div>
      </header>
      {refused !== null && <p className="field__error" id="run-cancel-error">{refused}</p>}
      {(!done || openedEarly) && (
        <section className="run-card">
          <h4 className="run-card__title">
            {done ? 'Playback' : live?.state === 'live' ? 'Live' : 'Progress'}
            <span className="muted run-card__note">
              {done ? ' in simtime, as it streamed' : live?.state === 'live' ? ' as the runner reports it' : live?.state === 'connecting' ? ' connecting to the live stream' : live?.state === 'lost' ? ` polled while it runs; ${live.why}` : ' polled while it runs'}
            </span>
          </h4>
          <div className="run-live" {...fill.region}>
            <div className="run-live__floor">
              <RunPlayback run={run} served={served} bare fill={fill} onCursor={cursor.set} describedBy={stubDescribed} />
            </div>
            <div className="run-live__numbers"><Progress run={run} live={live?.state === 'live' ? live : null} /></div>
          </div>
          {!done && <p className="field__hint">Cancelling never asks for approval: it only stops spending.</p>}
        </section>
      )}
      {!done && middle}
      {done && (
        <RunResults
          run={run} highlights={highlights} between={middle}
          beside={openedEarly ? undefined : served ? (
            <section className="run-card run-card--player" {...fill.region}>
              <h4 className="run-card__title">Playback <span className="muted run-card__note"> in simtime</span></h4>
              <RunPlayback run={run} served={served} bare fill={fill} onCursor={cursor.set} describedBy={stubDescribed} />
            </section>
          ) : (
            <section className="run-card run-card--player run-card--empty">
              <h4 className="run-card__title">Playback</h4>
              <p className="muted">This attempt stored no recording, so there is nothing to play.</p>
            </section>
          )}
        />
      )}
      <details className="run-card run-card--fold">
        <summary><h4 className="run-card__title">Provenance <span className="muted run-card__note"> what the run read, and the runner that ran it</span></h4></summary>
        <div className="run-card__body"><Provenance run={run} latest={latest} workloadAbove /></div>
      </details>
    </section>
  );
}

function Progress({ run, live }: { run: RunDetail; live: { simMs: number; values: RunDetail['progress']['values'] } | null }) {
  // The bar shows the newest progress once a minute of wall clock, however often the run reports it; the tiles read
  // the live stream when it is open, else the polled read.
  const simMs = live?.simMs ?? run.progress.sim_ms;
  const values = live?.values ?? run.progress.values;
  const newest = useRef(simMs);
  useEffect(() => {
    newest.current = simMs;
  });
  const [shown, setShown] = useState(simMs);
  useEffect(() => {
    const timer = window.setInterval(() => setShown(newest.current), PROGRESS_EVERY_MS);
    return () => window.clearInterval(timer);
  }, []);
  const at = simMs;
  return (
    <>
      <ul className="metric-tiles" aria-label="Progress so far, as the runner reports it">
        <li className="metric-tile">
          <span className="metric-tile__label">Simulated</span>
          <span className="metric-tile__value"><Sim ms={at} tagged={false} /></span>
          <span className="metric-tile__note">of <Sim ms={run.duration_sim_ms} /></span>
        </li>
        {values.map((value) => (
          <li className="metric-tile" key={value.metric}>
            <span className="metric-tile__label">{value.label} so far</span>
            <span className="metric-tile__value">{value.value ?? 'none yet'}</span>
            <span className="metric-tile__note">running count at <Sim ms={at} /></span>
          </li>
        ))}
        <li className="metric-tile">
          <span className="metric-tile__label">Elapsed</span>
          <span className="metric-tile__value">{run.elapsed_wall_ms === undefined ? 'not started' : <Wall ms={run.elapsed_wall_ms} />}</span>
          <span className="metric-tile__note">since its runner started</span>
        </li>
      </ul>
      <label className="field__label" htmlFor="run-progress">
        Simulated so far, of <Sim ms={run.duration_sim_ms} />
      </label>
      <progress id="run-progress" max={run.duration_sim_ms} value={shown} />
    </>
  );
}

/** The check's compiled flow in words: no compiler writes a flow plan yet, so the simulator runs its default flow. */
function checkWords(run: RunDetail): string {
  const { compile_sha256: digest, compiler } = run.expected;
  return digest === null ? "no flow plan: the simulator's default flow" : `digest ${digest.slice(0, 12)}, compiler ${compiler}`;
}

function compiledWords(run: RunDetail, latest: Attempt | undefined): string {
  const compile = latest?.compile;
  if (compile === undefined) return `the check's: ${checkWords(run)}; the runner has not compiled yet`;
  if (compile.digest === run.expected.compile_sha256 && compile.compiler === run.expected.compiler) {
    return `as the check compiled it: ${checkWords(run)}`;
  }
  return `digest ${compile.digest?.slice(0, 12) ?? 'none'}, compiler ${compile.compiler}, where the check's was ${checkWords(run)}`;
}

/** Every pin's provenance; `workloadAbove` where the inputs above name the generated workload itself. */
function Provenance({ run, latest, workloadAbove = false }: { run: RunDetail; latest: Attempt | undefined; workloadAbove?: boolean }) {
  const reads = Object.entries(run.reads).map(([scope, seq]) => `${scope} at changeset ${seq}`);
  return (
    <dl className="provenance">
      {latest?.job && (
        <>
          <dt>Job</dt>
          <dd><code>{latest.job}</code></dd>
        </>
      )}
      <dt>Read</dt>
      <dd>{reads.join('; ')}, as branch {run.cut.branch} stood when the runs were prepared</dd>
      <dt>Exact</dt>
      <dd>
        {latest?.exact === undefined ? 'not known yet: the runner has not handed in what it read'
          : latest.exact ? 'yes: every scope was read at its head, its labels and prologue with it'
            : `no: ${latest.exact_reason ?? 'it read below a moved head'}`}
      </dd>
      <dt>Scenario</dt>
      <dd>
        {run.scenario.name}, version {run.scenario.version}; <Sim ms={run.duration_sim_ms} />, seed {run.seed}
        {run.variant.map((varied) => `; ${varied.label} ${varied.value}`).join('')}
        {!workloadAbove && "; its orders and inventory generated from the version's workload spec"}
      </dd>
      <dt>Compiled flow</dt>
      <dd>{compiledWords(run, latest)}</dd>
      <dt>Runner</dt>
      <dd>
        image <code>{run.runner.tag || run.runner.image}</code>, {run.runner.chosen === 'default' ? 'the default' : 'chosen by you'}
        {latest?.runner_commit && <>; it reported commit <code>{latest.runner_commit.slice(0, 9)}</code>{latest.runner_dirty ? ', with uncommitted changes' : ', clean'}</>}
      </dd>
      {latest !== undefined && (latest.heartbeat_at !== undefined || latest.platform_status !== undefined) && (
        <>
          <dt>Runner status</dt>
          <dd>
            {latest.heartbeat_at !== undefined ? <>last heard {heardWords(latest.heartbeat_at)}</> : 'not heard from yet'}
            {latest.stalled && <>{' '}<span className="pill" data-tone="warn">stalled</span></>}
            {latest.platform_status !== undefined && <>; the platform says <code>{latest.platform_status}</code></>}
          </dd>
        </>
      )}
      {latest !== undefined && latest.findings.length > 0 && (
        <>
          <dt>Findings</dt>
          {latest.findings.map((finding) => (
            <dd key={finding.kind}>
              {finding.kind === 'plan-differs' && <><span className="pill" data-tone="warn">differs from the check</span>{' '}</>}
              {finding.sentence}
            </dd>
          ))}
        </>
      )}
      {latest?.compile !== undefined && (
        <>
          <dt>Manifest</dt>
          <dd><a href={`/api/runs/${run.id}/manifest?attempt=${latest.n}`}>what attempt {latest.n} read and compiled</a></dd>
        </>
      )}
    </dl>
  );
}

/** How long ago a runner's last heartbeat came, in wall-clock words: "3 s ago", "2 min ago". */
function heardWords(at: string): string {
  const seconds = Math.max(0, Math.round((Date.now() - Date.parse(at)) / 1000));
  if (seconds < 60) return `${seconds} s ago`;
  const minutes = Math.round(seconds / 60);
  return minutes < 60 ? `${minutes} min ago` : `${Math.round(minutes / 60)} h ago`;
}
