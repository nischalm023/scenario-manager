// Spec 07 section 5.1, the readiness line: one plain sentence, "Ready to run: 900 orders at map 1, flow 2, fleet 1,
// as read at {wall-clock time}" or "Not ready: 2 problems", its state said once, in the pill; then each problem,
// starting with its severity in words, by an id Run's description can name. A world-proto-server that does not
// answer is said so, with the time of the attempt and Retry; never a live region.

import type { Readiness } from '../../protocol/workload.ts';
import { readinessWords } from './words.ts';

export type ReadinessState =
  | { state: 'reading' }
  | { state: 'read'; readiness: Readiness }
  | { state: 'busy' }
  | { state: 'failed'; reason: string };

/** "16:58", or with `date` "19 Oct 2026, 16:58": a wall-clock instant, which carries no tag (spec 02's clocks). */
export function WallTime({ at, date = false }: { at: string; date?: boolean }) {
  const instant = new Date(at);
  const time = instant.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
  const day = date ? `${instant.toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' })}, ` : '';
  return <span className="clock" data-clock="wall">{day}{time}</span>;
}

export function ReadinessLine({ state, site, retry }: { state: ReadinessState; site: string; retry(): void }) {
  if (state.state === 'reading') return <p id="sc-readiness" className="muted">Reading whether it is ready to run</p>;
  if (state.state === 'busy') {
    return <p id="sc-readiness">Agent Studio is busy computing; this view tries again in a moment.</p>;
  }
  if (state.state === 'failed') {
    return (
      <p id="sc-readiness">
        Whether it is ready to run could not be read: {state.reason}{' '}
        <button className="btn btn--small" type="button" onClick={retry}>Retry</button>
      </p>
    );
  }
  const { readiness } = state;
  const unanswered = readiness.problems.find((problem) => problem.code === 'wps-unavailable');
  if (unanswered !== undefined) {
    return (
      <p id="sc-readiness">
        <span className="pill" data-tone="bad">Not ready</span>: the world server did not answer, as tried at{' '}
        <WallTime at={readiness.checked_at} />.{' '}
        <button className="btn btn--small" type="button" onClick={retry}>Retry</button>
      </p>
    );
  }
  const { state: word, detail } = readinessWords(readiness, site);
  return (
    <>
      <p id="sc-readiness">
        <span className="pill" data-tone={readiness.ready ? 'good' : 'bad'}>{word}</span>: {detail}
        {readiness.ready && <>, as read at <WallTime at={readiness.checked_at} /></>}.
      </p>
      {readiness.problems.length > 0 && (
        <ul className="problem-list" aria-label="What readiness found">
          {readiness.problems.map((problem, n) => (
            <li className="problem" id={`sc-problem-${n}`} data-severity={problem.severity} key={`${problem.code}-${n}`}>
              <span className="problem__kind">{problem.severity === 'error' ? 'Error:' : 'Warning:'}</span> {problem.text}
            </li>
          ))}
        </ul>
      )}
    </>
  );
}
