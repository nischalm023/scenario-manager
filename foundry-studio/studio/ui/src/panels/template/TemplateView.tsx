// Spec 05 section 6, one template (`/w/template/site/{t}@main`): its card (title, line, built from), its knob
// declarations as a table, its default evaluation's drawing, census and known problems, each of its scopes'
// changesets, and "Use this template". Until world-proto-server has tags a template is read at the head of main.

import { useEffect, useState } from 'react';
import type { TemplateDetail } from '../../protocol/template.ts';
import { useShell } from '../../shell/context.ts';
import { TableScroll } from '../../shell/TableScroll.tsx';
import { getJson } from './requests.ts';
import { startDraft } from './start.ts';
import { defaultWords, figures, grouped, rangeWords } from './words.ts';
import { FloorSnapshot } from '../../viewer/FloorSnapshot.tsx';

type Read = { state: 'loading' } | { state: 'ready'; detail: TemplateDetail } | { state: 'failed'; reason: string };

const POLL_MS = 1500;

export function TemplateView({ template }: { template: string }) {
  const shell = useShell();
  const [read, setRead] = useState<Read>({ state: 'loading' });
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const load = async () => {
      const answer = await getJson<TemplateDetail>(`/api/templates/${template}@main`);
      if (!current) return;
      if (!answer.ok) {
        setRead({ state: 'failed', reason: answer.status === 404 ? `There is no site template ${template}.` : answer.reason });
        return;
      }
      setRead({ state: 'ready', detail: answer.value });
      if (answer.value.default.state === 'evaluating') timer = setTimeout(() => void load(), POLL_MS);
    };
    void load();
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [template]);

  if (read.state === 'loading') return <p className="panel__status" aria-busy="true">Reading the template</p>;
  if (read.state === 'failed') return <p className="panel__status">{read.reason}</p>;
  const { detail } = read;
  const fallback = detail.default;
  const use = async () => {
    setFailure(null);
    const answer = await startDraft(detail.ref, shell.browserTab);
    if (answer.ok) {
      shell.navigate(`/template-drafts/${answer.value.draft}`, 'user');
      return;
    }
    const sentence = `${detail.description.title} could not be started: ${answer.reason}`;
    setFailure(sentence);
    shell.announce(sentence);
  };
  return (
    <>
      <section className="section" data-ref={`scope:${detail.ref}`}>
        <h3 className="section__title" id="template-title">{detail.description.title}</h3>
        <p>{detail.description.text}</p>
        {detail.description.built_from !== null && <p className="muted">Built from {detail.description.built_from}.</p>}
        <p className="template-card__ref">
          {detail.ref}@main: {Object.entries(detail.seqs).map(([scope, seq]) => `${scope.slice(detail.ref.length + 1) || 'top scope'} at changeset ${seq}`).join(', ')}
        </p>
        <div className="template-card__actions">
          <button className="btn btn--primary" type="button" aria-describedby="template-title" onClick={() => void use()}>Use this template</button>
        </div>
        {failure !== null && <p className="field__error">{failure}</p>}
      </section>
      <section className="section">
        <h3 className="section__title">Its default evaluation</h3>
        {fallback.state === 'ready' && fallback.floor_key !== null && (
          <FloorSnapshot className="template-card__thumb" floorKey={fallback.floor_key} label={`${detail.description.title} at its defaults, drawn north-up`} />
        )}
        <p className="panel__status" aria-busy={fallback.state === 'evaluating'}>
          {fallback.state === 'evaluating' ? 'Evaluating the defaults…' : fallback.state === 'ready' ? `${detail.description.title} at its defaults.` : 'The defaults do not evaluate.'}
        </p>
        {fallback.census !== null && (
          <ul className="evaluation__summary">
            {figures(fallback.census, detail.knobs).map((tile) => (
              <li className="figure-tile" data-ref={`scope:${detail.ref}`} key={tile.key}>
                <span className="figure-tile__value">{grouped(tile.value)}</span><span className="figure-tile__label">{tile.label}</span>
              </li>
            ))}
          </ul>
        )}
        {fallback.problems.map((problem, n) => (
          <p className="problem" data-severity={problem.severity} data-ref={problem.ref ?? `scope:${detail.ref}`} key={n}>{problem.text}</p>
        ))}
      </section>
      <section className="section">
        <h3 className="section__title">Knobs</h3>
        <TableScroll name="The template's knobs">
          <table className="list-table">
            <caption>Each knob the template declares: its unit, its range or options, and its default.</caption>
            <thead><tr><th scope="col">Knob</th><th scope="col">Unit</th><th scope="col">Range or options</th><th scope="col">Default</th></tr></thead>
            <tbody>
              {detail.knobs.map((knob) => (
                <tr key={knob.key} data-ref={`scope:${detail.ref}`}>
                  <td>{knob.label}</td><td>{knob.unit}</td><td>{rangeWords(knob)}</td><td>{defaultWords(knob)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </TableScroll>
        <p className="muted">Flows: {detail.flows.length > 0 ? detail.flows.join('; ') : 'none'}.</p>
      </section>
    </>
  );
}
