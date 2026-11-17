// Spec 02 section 3.1, RunCard (spec 08 section 7, requests b to g): one card per submission. One run shows its own
// state; several show `queued`, `running` or `ended` with counts by state and a line per run. A run's ending detail
// stands beneath its state (the simulator's stop reason, or for an attempt whose manifest says `stub`, the stub's
// sentence, ruling D110, so no value of one appears without it); a `plan-differs` finding carries the warn pill.
// Progress is simtime and never spoken; "since it started" is wall clock.

import { useState } from 'react';
import type { ProjectedRun } from '../protocol/projection.ts';
import type { RunStatus } from '../protocol/run.ts';
import { useShell } from '../shell/context.ts';
import { ShellLink } from '../shell/ShellLink.tsx';
import { bodyOf, findingsOf, statusOf } from './bodies.ts';
import { useChatApi } from './chatContext.ts';
import type { RunCardModel } from './log.ts';
import { runEnded, runStateWords, simtimeWords } from './words.ts';

/** "14 min" of wall clock since `at`. */
function wallSince(at: string): string {
  const minutes = Math.max(0, Math.round((Date.now() - Date.parse(at)) / 60000));
  return minutes < 60 ? `${minutes} min` : `${Math.floor(minutes / 60)} h ${minutes % 60} min`;
}

function Findings({ run }: { run: ProjectedRun }) {
  return (
    <>
      {findingsOf(run).map((read, n) => {
        return (
          <p className="run-card__finding" key={n}>
            {read.finding.kind === 'plan-differs' && <><span className="pill" data-tone="warn">differs from the check</span>{' '}</>}
            {read.finding.sentence ?? read.finding.kind}
          </p>
        );
      })}
    </>
  );
}

function Progress({ run }: { run: ProjectedRun }) {
  const { model } = useChatApi();
  const status = statusOf(run);
  const progress = model.progress.get(run.run);
  if (status === null || status.state !== 'running') return null;
  const started = model.entries.find((entry) => {
    const status = bodyOf(entry, 'run.status');
    return status?.run === run.run && status.state === 'running';
  });
  const headline = progress?.values.find((value) => value.value !== null);
  return (
    <div className="run-card__progress">
      <p>
        Simulated {simtimeWords(progress?.sim_ms ?? status.sim_ms)}{' '}
        <span className="clock" data-clock="sim"><span className="clock__tag">simtime</span></span>
      </p>
      {(headline !== undefined || started !== undefined) && (
        <p>
          {headline !== undefined && `${headline.label} so far ${headline.value}. `}
          {started !== undefined && (
            <span className="clock" data-clock="wall">{wallSince(started.at)}</span>
          )}
          {started !== undefined && ' since it started'}
        </p>
      )}
    </div>
  );
}

export function RunCard({ card }: { card: RunCardModel }) {
  const api = useChatApi();
  const shell = useShell();
  const [error, setError] = useState<string | null>(null);
  const [stopping, setStopping] = useState(false);
  const statuses = card.runs.map(statusOf).filter((status): status is RunStatus => status !== null);
  const several = card.of > 1;
  const going = statuses.some((status) => !runEnded(status.state));
  const single = statuses[0];
  const key = several || single === undefined ? `submission:${card.submission}` : `run:${single.run}`;
  const titleId = `run-${several || single === undefined ? card.submission : single.run}-title`;
  const shown = !several && single !== undefined ? runStateWords(single) : null;
  const cardState = several ? (going ? (statuses.some((status) => status.state === 'running') ? 'running' : 'queued') : 'ended') : single?.state ?? 'queued';
  const counts = (['queued', 'running', 'finished', 'cancelled', 'failed', 'partial'] as const)
    .map((state) => [state, statuses.filter((status) => status.state === state).length] as const)
    .filter(([, count]) => count > 0).map(([state, count]) => `${count} ${state}`).join(', ');
  const errorId = `${titleId}-error`;

  const cancel = async () => {
    const outcome = await api.cancel(card);
    if (outcome.ok) {
      setStopping(true);
      shell.focusComposer();
    } else {
      setError(outcome.words);
      shell.announce(outcome.words);
    }
  };

  const openRoute = several || single === undefined ? `/runs?submission=${card.submission}` : `/runs/${single.run}`;
  return (
    <div className="chat-card run-card" data-state={cardState} data-ref={key}>
      <h4 className="chat-card__title" id={titleId} tabIndex={-1}>
        {several ? `Sweep of ${card.of} runs` : single !== undefined ? `Run ${single.run.slice(0, 8)}` : 'Run'}{' '}
        {shown !== null && (
          <span className="pill" data-tone={shown.tone}>
            {shown.glyph !== null && <span aria-hidden="true">{shown.glyph}</span>}
            {stopping && going ? 'stopping' : shown.words}
          </span>
        )}
        {several && <span className="pill" data-tone={going ? 'info' : undefined}>{stopping && going ? 'stopping' : cardState}</span>}
      </h4>
      {several && counts !== '' && <p>{counts}</p>}
      {several ? (
        <ul className="run-list">
          {card.runs.map((run) => {
            const status = statusOf(run);
            if (status === null) return null;
            return (
              <li className="run-line" key={run.run} data-ref={`run:${run.run}`}>
                <span>Run {status.position} of {status.of}</span>
                <span>{runStateWords(status).words}</span>
                {status.stop_detail && <p className="run-card__detail">{status.stop_detail}</p>}
                <Findings run={run} />
              </li>
            );
          })}
        </ul>
      ) : (
        card.runs.map((run) => (
          <div key={run.run}>
            <Progress run={run} />
            {statusOf(run)?.stop_detail && <p className="run-card__detail">{statusOf(run)?.stop_detail}</p>}
            <Findings run={run} />
          </div>
        ))
      )}
      <div className="chat-card__actions">
        <ShellLink className="btn btn--small" to={openRoute}>Open in Runs</ShellLink>
        {going && !api.readOnly && (
          <button className="btn btn--small btn--danger" type="button" aria-describedby={error !== null ? errorId : undefined} onClick={() => void cancel()}>
            {several ? 'Cancel the sweep' : 'Cancel the run'}
          </button>
        )}
      </div>
      {going && <p className="chat-card__footnote">Cancelling never asks for approval: it only stops spending.</p>}
      {error !== null && <p className="chat-card__error" id={errorId}>{error}</p>}
    </div>
  );
}
