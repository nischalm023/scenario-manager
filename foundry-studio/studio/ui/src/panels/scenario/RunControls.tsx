// Spec 07 section 5.1's Sweep group and Run (with spec 08 section 4.2, a person presses Run): one field a sweep may
// vary and its values, the count beside Run with the five-at-a-time note, and Run itself, focusable while it cannot
// run, `aria-disabled` and described by the element already saying its first reason (the status line, the readiness
// line or its first error, the values' hint), so the reason is said once. A press runs `press`: the experiment,
// told to the agent, then the plan, the submission and its execution, the button saying which step it waits on; the
// Run group says "Submitted" only once the execution is confirmed, which is told to the agent too (`runs.submitted`,
// spec 08 section 4.2), and otherwise why nothing was submitted.

import { useEffect, useRef, useState } from 'react';
import type { FieldEntry } from '../../protocol/scenario.ts';
import type { Readiness } from '../../protocol/workload.ts';
import { getJson, postJson } from '../../shell/api.ts';
import { routeFor } from '../../shell/routes.ts';
import { ShellLink } from '../../shell/ShellLink.tsx';
import { pressKey } from './api.ts';
import { press, type PressStep } from './press.ts';
import { runCountWords, runLabel, runsWords, sweepValues } from './words.ts';

export interface RunControlsProps {
  scenario: { id: string; name: string; version: number; revision: number; site: string };
  sweepable: FieldEntry[];
  repeats: number;
  readiness: Readiness | null;
  /** Nothing runs from this view, as the status line (`#sc-status`) says: an older version, a deleted scenario. */
  held: boolean;
  tell(kind: 'experiment.prepared' | 'runs.submitted', text: string, body: Record<string, unknown>): void;
  announce(sentence: string): void;
}

export function RunControls({ scenario, sweepable, repeats, readiness, held, tell, announce }: RunControlsProps) {
  const [vary, setVary] = useState('');
  const [values, setValues] = useState('');
  const [outcome, setOutcome] = useState<{ words: string; submitted: boolean } | null>(null);
  const [step, setStep] = useState<PressStep | null>(null);
  const shown = useRef(true);
  useEffect(() => {
    shown.current = true;
    return () => {
      shown.current = false;
    };
  }, []);
  const pressing = step !== null;
  const field = sweepable.find((entry) => entry.pointer === vary) ?? null;
  const parsed = field === null ? null : sweepValues(field, values);
  const count = field === null ? 1 : parsed?.ok ? parsed.values.length : 0;
  const firstError = readiness?.problems.findIndex((problem) => problem.severity === 'error') ?? -1;
  // The id of the element that says why Run cannot run; null when it can.
  const reason = held ? 'sc-status'
    : readiness === null ? 'sc-readiness'
      : firstError >= 0 ? `sc-problem-${firstError}`
        : parsed !== null && !parsed.ok ? 'sw-values-hint'
          : null;

  const run = async () => {
    if (reason !== null || pressing) return;
    setStep('preparing');
    setOutcome(null);
    const request = {
      scenario: scenario.id, version: scenario.version, revision: scenario.revision, repeats,
      ...(field !== null && parsed?.ok ? { varies: { target: `scenario:${scenario.id}`, pointer: field.pointer, values: parsed.values } } : {}),
    };
    const requests = {
      post: <T,>(path: string, body: unknown) => postJson<T>(path, body, pressKey()),
      get: getJson,
      wait: (ms: number) => new Promise<boolean>((resolve) => window.setTimeout(() => resolve(shown.current), ms)),
    };
    const ended = await press(requests, request, readiness?.cut?.seqs ?? null, {
      prepared: (experiment) => {
        const runs = experiment.run_order.length;
        const what = field === null ? `${runsWords(runs)} of ${scenario.name}` : `a sweep of ${runs} runs of ${scenario.name} over ${field.label.toLowerCase()}`;
        tell('experiment.prepared', `You prepared ${what}`, { experiment: experiment.id, runs_planned: runs });
      },
      step: setStep,
    });
    if (ended.kind === 'left') return;
    setStep(null);
    if (ended.kind === 'submitted') {
      const runs = ended.experiment.run_order.length;
      tell('runs.submitted', `You submitted ${runsWords(runs)} of ${scenario.name}`, { experiment: ended.experiment.id, runs });
    }
    const words = ended.kind === 'submitted' ? `Submitted ${runsWords(ended.experiment.run_order.length)}.` : ended.words;
    setOutcome({ words, submitted: ended.kind === 'submitted' });
    announce(words);
  };
  const runs = count * repeats;
  const note = runCountWords(count, repeats);
  const label = step === 'preparing' ? `Preparing ${runsWords(runs)}` : step === 'submitting' ? `Submitting ${runsWords(runs)}` : runLabel(runs);

  return (
    <>
      <fieldset className="fieldset">
        <legend>Sweep</legend>
        <div className="field">
          <fieldset className="option-list" aria-describedby="sw-vary-hint">
            <legend>Vary</legend>
            <div className="option">
              <input type="radio" id="sw-vary-none" name="vary" value="" checked={vary === ''} onChange={() => setVary('')} />{' '}
              <span><label className="option__label" htmlFor="sw-vary-none">Nothing: one run</label></span>
            </div>
            {sweepable.map((entry) => (
              <div className="option" key={entry.pointer}>
                <input
                  type="radio" id={`sw-vary-${entry.short_label.replaceAll(' ', '-')}`} name="vary" value={entry.pointer}
                  checked={vary === entry.pointer} onChange={() => setVary(entry.pointer)}
                />{' '}
                <span>
                  <label className="option__label" htmlFor={`sw-vary-${entry.short_label.replaceAll(' ', '-')}`}>{entry.label}</label>
                </span>
              </div>
            ))}
          </fieldset>
          <p className="field__hint" id="sw-vary-hint">One field at a time; the site is never changed.</p>
        </div>
        {field !== null && (
          <div className="field">
            <label className="field__label" htmlFor="sw-values">Values, separated by commas</label>
            <input
              className="input" id="sw-values" name="values" type="text" value={values} onChange={(event) => setValues(event.target.value)}
              aria-describedby="sw-values-hint" aria-invalid={values.trim() !== '' && parsed !== null && !parsed.ok ? true : undefined}
            />
            <p className="field__hint" id="sw-values-hint">
              {field.label}{field.unit ? `, in ${field.unit}` : ''}: 2 to 12 values.
              {values.trim() !== '' && parsed !== null && !parsed.ok ? ` ${parsed.why}` : ''}
            </p>
          </div>
        )}
      </fieldset>
      {note !== null && <p id="sw-count">{note}</p>}
      <p>
        <button
          className="btn btn--pill btn--accent" id="sw-run" type="button" onClick={() => void run()}
          aria-disabled={reason !== null || pressing ? true : undefined} aria-busy={pressing ? true : undefined}
          aria-describedby={[reason, note !== null ? 'sw-count' : null].filter(Boolean).join(' ') || undefined}
        >
          {label}
        </button>
      </p>
      {outcome !== null && (
        <p className="panel__status" id="sw-outcome">
          {outcome.words}
          {outcome.submitted && <> <ShellLink to={routeFor({ view: 'runs', site: scenario.site })}>Open the Runs panel</ShellLink></>}
        </p>
      )}
    </>
  );
}
