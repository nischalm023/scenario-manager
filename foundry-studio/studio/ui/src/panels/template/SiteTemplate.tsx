// Spec 05 section 6, a site's template (`/w/{site}@main?panel=template`): the made-from records, newest first, each
// with the template at the changesets it was read at, the knob values with their labels and units, the act, its
// author and its wall-clock time; a site no template made says so. "Update from template" waits for task 37 (lead
// decided).

import { Fragment, useEffect, useState } from 'react';
import type { MadeFromList } from '../../protocol/made_from.ts';
import { getJson } from './requests.ts';
import { grouped } from './words.ts';

type Read = { state: 'loading' } | { state: 'ready'; list: MadeFromList } | { state: 'failed'; reason: string };

const unitWords = (unit: string): string => (unit === 'mm' ? ' mm' : unit === 'deg' ? ' degrees' : '');

export function SiteTemplate({ site }: { site: string }) {
  const [read, setRead] = useState<Read>({ state: 'loading' });
  useEffect(() => {
    let current = true;
    void getJson<MadeFromList>(`/api/made-from/${site}`).then((answer) => {
      if (current) setRead(answer.ok ? { state: 'ready', list: answer.value } : { state: 'failed', reason: answer.reason });
    });
    return () => {
      current = false;
    };
  }, [site]);
  if (read.state === 'loading') return <p className="panel__status" aria-busy="true">Reading what this site was made from</p>;
  if (read.state === 'failed') return <p className="panel__status">What this site was made from cannot be read: {read.reason}</p>;
  if (read.list.items.length === 0) return <p>This site was not made from a template.</p>;
  return (
    <ol className="highlights" aria-label="Made from, newest first">
      {read.list.items.map((made) => (
        <li key={made.id} data-ref={`template-draft:${made.draft}`}>
          <section className="section">
            <h3 className="section__title">
              {made.act === 'create' ? 'Made' : 'Updated'} from <code>{made.template}</code> by {made.author.name}{' '}
              <span className="clock" data-clock="wall">{new Date(made.at).toLocaleString()}</span>
            </h3>
            <p className="muted">
              The template at {Object.entries(made.template_seqs).map(([scope, seq]) => `${scope.slice(made.template.length + 1) || 'top scope'} changeset ${seq}`).join(', ')}
              {made.tag !== null && `, tag ${made.tag}`}; evaluation {made.evaluation} of its draft.
            </p>
            <dl className="provenance" aria-label="Knob values">
              {made.knobs.map((knob) => (
                <Fragment key={knob.key}><dt>{knob.label}</dt><dd>{grouped(knob.value)}{unitWords(knob.unit)}</dd></Fragment>
              ))}
            </dl>
          </section>
        </li>
      ))}
    </ol>
  );
}
