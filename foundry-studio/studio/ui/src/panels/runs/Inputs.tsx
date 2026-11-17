// Spec 08 section 5's "What it read", its first half (brief I1.10, task 34): the run's inputs as values, every
// scenario field as the run read it with where it came from, then the generated workload (spec 08 section 3.12's
// provenance: the generator's version and seed, the digest of what it made and the request it was sent, as the
// runner's manifest names them). The values read as a grid of facts at the top of the page; where each came from, the
// table and the workload sit in a closed disclosure with "Copy the inputs as text", which puts the API's own lines
// on the clipboard so a copy and the page never disagree. The rest of the provenance follows as OpenRun's list; the
// flow's parameters, the class figures, the map's census and the resolved settings are not read yet (`complete:
// false`), which the table's caption says. A copy's outcome is the person's own act, so the shell's announcer speaks
// it; the panel holds no live region.

import { useEffect, useState } from 'react';
import type { InputRow, RunInputs } from '../../protocol/results.ts';
import type { WorkloadRead } from '../../protocol/runner.ts';
import { clause, getJson } from '../../shell/api.ts';
import { useShell } from '../../shell/context.ts';
import { TableScroll } from '../../shell/TableScroll.tsx';
import { shortId } from './words.ts';

type Read = { state: 'reading' } | { state: 'read'; value: RunInputs } | { state: 'failed'; reason: string };

/** An input's value, a time of simtime or wall clock in its `.clock`. */
function InputValue({ row }: { row: InputRow }) {
  if (row.clock === undefined) return <>{row.value}</>;
  return <span className="clock" data-clock={row.clock}>{row.value}</span>;
}

export function Inputs({ run, attempt, manifestIn }: { run: string; attempt: number; manifestIn: boolean }) {
  const shell = useShell();
  const [read, setRead] = useState<{ key: string; read: Read } | null>(null);
  const [copy, setCopy] = useState<string | null>(null);
  const key = `${run}/${attempt}/${manifestIn}`;
  useEffect(() => {
    let current = true;
    void getJson<RunInputs>(`/api/runs/${run}/inputs`).then((answer) => {
      if (!current) return;
      if (answer.ok && Array.isArray(answer.value?.rows)) setRead({ key, read: { state: 'read', value: answer.value } });
      else setRead({ key, read: { state: 'failed', reason: answer.ok ? 'the service answered something else than inputs' : clause(answer.reason) } });
    });
    return () => {
      current = false;
    };
  }, [run, key]);
  const shown: Read = read !== null && read.key === key ? read.read : { state: 'reading' };
  if (shown.state === 'reading') return <p className="panel__status" aria-busy="true">Reading the inputs</p>;
  if (shown.state === 'failed') return <p className="panel__status">The inputs could not be read: {shown.reason}.</p>;
  const inputs = shown.value;
  const rows = inputs.rows.filter((row) => row.section !== 'provenance');
  const varied = rows.filter((row) => row.varied).length;
  const copied = async () => {
    try {
      await navigator.clipboard.writeText(inputs.text);
      const words = `Copied the inputs of run ${shortId(run)} as text.`;
      setCopy(null);
      shell.announce(words);
    } catch {
      const words = 'The inputs could not be copied: this browser refused the clipboard.';
      setCopy(words);
      shell.announce(words);
    }
  };
  const sections = SECTIONS.filter((section) => rows.some((row) => row.section === section.key));
  return (
    <div className="run-inputs">
      <div className="run-inputs__facts">
        {sections.map((section) => (
          <div className="run-inputs__group" key={section.key}>
            {sections.length > 1 && <h5 className="run-inputs__section">{section.name}</h5>}
            <dl className="run-inputs__list" aria-label={section.name}>
            {rows.filter((row) => row.section === section.key).map((row) => (
              <div className="run-input" key={row.field} data-varied={row.varied ? 'true' : undefined}>
                <dt>{row.label}</dt>
                <dd>
                  <InputValue row={row} />
                  {row.varied && <> <span className="pill" data-tone="info">varied</span></>}
                </dd>
              </div>
            ))}
            </dl>
          </div>
        ))}
      </div>
      <details className="run-inputs__more">
        <summary>Where each value came from ({rows.length} values; {varied === 0 ? 'none varied' : `${varied} varied by the experiment`})</summary>
        <p>
          <button className="btn btn--small" type="button" id="run-inputs-copy" onClick={() => void copied()}
            aria-describedby={copy !== null ? 'run-inputs-copy-error' : undefined}>
            Copy the inputs as text
          </button>
        </p>
        {copy !== null && <p className="field__error" id="run-inputs-copy-error">{copy}</p>}
        <TableScroll name="Inputs">
          <table className="list-table">
            <caption>
              Every input of this run as a value, with where it came from.
              {!inputs.complete && ' The flow\'s parameters, the class figures, the map\'s census and the settings the runner resolved are not read yet.'}
            </caption>
            <thead>
              <tr><th scope="col">Input</th><th scope="col">Value</th><th scope="col">From</th></tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={row.field}>
                  <th scope="row">{row.label}</th>
                  <td><InputValue row={row} /></td>
                  <td>
                    {row.from}
                    {row.held !== undefined && <>; the scenario holds <InputValue row={{ ...row, value: row.held }} /></>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </TableScroll>
        <Workload workload={inputs.workload} row={inputs.rows.find((row) => row.section === 'provenance' && row.field === WORKLOAD)} />
      </details>
    </div>
  );
}

/** The input sections in the order they read, the scenario's own values first. */
const SECTIONS: { key: InputRow['section']; name: string }[] = [
  { key: 'scenario', name: 'Scenario' }, { key: 'flow', name: 'Flow' }, { key: 'fleet', name: 'Fleet' }, { key: 'map', name: 'Map' }, { key: 'settings', name: 'Settings' },
];

/** The provenance row of the generated workload. */
const WORKLOAD = 'workload';

/** The generated workload as the runner's manifest names it, or, before the manifest, its provenance row's words. */
function Workload({ workload, row }: { workload?: WorkloadRead; row?: InputRow }) {
  if (workload === undefined) return row === undefined ? null : <p>Generated workload: {row.value} ({row.from}).</p>;
  return (
    <dl className="provenance">
      <dt>Generated workload</dt>
      <dd>
        from generator {workload.generator}, seed {workload.seed}: {workload.bytes.toLocaleString('en-US')} bytes, sha256{' '}
        <code>{workload.sha256}</code>
      </dd>
      <dt>The request it was sent</dt>
      <dd><code>{JSON.stringify(workload.spec)}</code></dd>
    </dl>
  );
}
