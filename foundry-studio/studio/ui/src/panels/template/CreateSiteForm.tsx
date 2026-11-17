// Spec 05 sections 3.8, 4.4 and 6, Create site: the customer, the site's name and its reference slug, what the site
// would record, and the press, which is the person's approval (spec 01 section 3.10, rule 6). The button stays
// focusable while unavailable (`aria-disabled`, described by the reason beside it, and a press speaks the reason).
// The form follows the approval of the draft's write, its own press's or the one a draft read as `writing` names (a
// reload, another tab), reading it once a second (wall clock) until its execution ends, `unknown` included, which
// says so and keeps following; then it reads the draft for what it made and has the panel read it again.
// `site-exists` marks the Reference field invalid, links the existing site, moves focus there and is spoken once.

import { useEffect, useRef, useState } from 'react';
import type { ApprovalAnswer } from '../../protocol/approval.ts';
import type { Problem } from '../../protocol/problem.ts';
import type { Execution } from '../../protocol/projection.ts';
import type { WriteAccepted } from '../../protocol/made_from.ts';
import type { TemplateDraft } from '../../protocol/template_draft.ts';
import type { TemplateEvaluation } from '../../protocol/template_evaluation.ts';
import { Info, TipFact } from '../../shell/Info.tsx';
import { ShellLink } from '../../shell/ShellLink.tsx';
import { getJson, sendJson } from './requests.ts';
import { valueWords } from './words.ts';

const SLUG = /^[a-z0-9]([a-z0-9-]{0,62}[a-z0-9])?$/;
const FOLLOW_MS = 1000;
/** The executions that have not ended: the form keeps following them (spec 01 section 3.10). */
const UNENDED: Execution[] = ['not_started', 'running', 'unknown'];

const slugOf = (title: string): string => title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'site';

export function CreateSiteForm({ draft, evaluation, unavailable, browserTab, announce, onCreated, onSettled }: {
  draft: TemplateDraft;
  evaluation: TemplateEvaluation | null;
  unavailable: string | null;
  browserTab: string;
  announce(sentence: string): void;
  onCreated(site: string): void;
  /** The write ended: the panel reads the draft again. */
  onSettled(): void;
}) {
  const [customer, setCustomer] = useState('demo');
  const [name, setName] = useState(draft.title);
  const [slug, setSlug] = useState(slugOf(draft.title));
  const [pressed, setPressed] = useState<string | null>(null);
  const [ended, setEnded] = useState<string | null>(null);
  const [unknown, setUnknown] = useState(false);
  const [refusal, setRefusal] = useState<Problem | null>(null);
  const [taken, setTaken] = useState<string | null>(null);
  const slugField = useRef<HTMLInputElement>(null);

  const writing = draft.state === 'writing' ? draft.write?.approval ?? null : null;
  const approval = pressed ?? (writing !== ended ? writing : null);
  const wrongField = !SLUG.test(customer) ? 'The customer is lowercase letters, digits and hyphens.'
    : !SLUG.test(slug) ? 'The reference is lowercase letters, digits and hyphens.'
      : name.trim() === '' ? 'The site needs a name.' : null;
  const following = unknown ? 'Not yet known whether the site was made; checking again.' : 'Creating the site…';
  const reason = approval !== null ? following : unavailable ?? wrongField;

  const refused = (problem: Problem | null, fallback: string) => {
    if (problem?.type === 'site-exists') {
      setTaken(`customer/${customer}/site/${slug}`);
      slugField.current?.focus();
      announce('A site with this reference exists.');
      return;
    }
    setRefusal(problem ?? { type: 'internal', title: 'Not created', status: 500, detail: fallback });
  };

  useEffect(() => {
    if (approval === null) return;
    let current = true;
    const timer = setInterval(() => {
      void getJson<ApprovalAnswer>(`/api/approvals/${approval}`).then(async (read) => {
        if (!current || !read.ok) return;
        const execution = read.value.approval.execution;
        setUnknown(execution === 'unknown');
        if (execution === null || UNENDED.includes(execution)) return;
        clearInterval(timer);
        const after = await getJson<TemplateDraft>(`/api/template-drafts/${draft.draft}`);
        if (!current) return;
        setPressed(null);
        setEnded(approval);
        setUnknown(false);
        onSettled();
        const result = after.ok ? after.value.write?.result ?? null : null;
        if (result !== null) onCreated(result.site);
        else refused(after.ok ? after.value.write?.problem ?? null : null, read.value.approval.sentence ?? 'The site was not created.');
      });
    }, FOLLOW_MS);
    return () => {
      current = false;
      clearInterval(timer);
    };
    // `refused` reads the fields as they were pressed; following one approval needs nothing else.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [approval, draft.draft, onCreated, onSettled]);

  const press = async () => {
    if (reason !== null) {
      announce(reason);
      return;
    }
    if (evaluation === null) return;
    setRefusal(null);
    setTaken(null);
    const body = { customer, slug, name: name.trim(), evaluation: evaluation.n, browser_tab: browserTab };
    const answer = await sendJson<WriteAccepted>('POST', `/api/template-drafts/${draft.draft}/site`, body);
    if (answer.ok) setPressed(answer.value.approval.id);
    else refused(answer.problem, answer.reason);
  };

  const changed = draft.declarations.filter((knob) => knob.key in draft.knobs);
  const seqs = Object.entries(draft.template.seqs).map(([scope, seq]) => `${scope.slice(draft.template.ref.length + 1) || 'its top scope'} at changeset ${seq}`);
  return (
    <form className="create-site create-site--foot" aria-labelledby="create-title" onSubmit={(event) => { event.preventDefault(); void press(); }}>
      <h3 className="section__title" id="create-title">
        Create the site
        <Info label="what is recorded">
          <TipFact term="Preset"><code>{draft.template.ref}</code>, {seqs.join(', ')}</TipFact>
          <TipFact term="Settings">{changed.length === 0 ? "All at the preset's defaults." : changed.map((knob) => `${knob.label} ${valueWords(knob, draft.knobs[knob.key])}`).join(', ')}</TipFact>
          <TipFact term="Lands on">One changeset on <code>customer/{customer}/site/{slug}</code>, creating the site with its map, flow and fleet.</TipFact>
          {draft.conversation !== null && <TipFact term="This conversation">Becomes the site&rsquo;s, and returns whenever the site is opened.</TipFact>}
        </Info>
      </h3>
      <div className="create-site__row">
        <div className="field">
          <div className="knob__head">
            <label htmlFor="site-customer">Customer</label>
            <Info id="site-customer-hint" label="the customer">
            <TipFact term="Who">An existing customer, or a new one.</TipFact>
            <TipFact term="Format">Lowercase letters, digits and hyphens.</TipFact>
          </Info>
          </div>
          <input className="input mono" id="site-customer" type="text" value={customer} aria-describedby="site-customer-hint" onChange={(event) => setCustomer(event.target.value)} />
        </div>
        <div className="field">
          <label htmlFor="site-name">Site name</label>
          <input className="input" id="site-name" type="text" value={name} onChange={(event) => setName(event.target.value)} />
        </div>
        <div className="field">
          <div className="knob__head">
            <label htmlFor="site-slug">Reference</label>
            <Info id="site-slug-hint" label="the reference">
              <TipFact term="Becomes"><code>customer/{customer}/site/{slug}</code></TipFact>
              <TipFact term="Format">Lowercase letters, digits and hyphens.</TipFact>
            </Info>
          </div>
          <input
            ref={slugField} className="input mono" id="site-slug" type="text" value={slug}
            aria-invalid={taken !== null ? true : undefined}
            aria-describedby={taken !== null ? 'site-slug-hint site-slug-error' : 'site-slug-hint'}
            onChange={(event) => { setSlug(event.target.value); setTaken(null); }}
          />
          {taken !== null && (
            <p className="field__error" id="site-slug-error">
              A site with this reference exists: <ShellLink to={`/w/${taken}/map@main`}>{taken}</ShellLink>
            </p>
          )}
        </div>
      </div>
      <div className="chat-card__actions">
        <button className="btn btn--pill btn--accent" type="submit" aria-disabled={reason !== null ? true : undefined} aria-describedby={reason !== null ? 'create-site-reason' : undefined}>
          Create site
        </button>
        {reason !== null && <span className="chat-card__footnote" id="create-site-reason">{reason}</span>}
      </div>
      {refusal !== null && <p className="problem" data-severity="error">Error: {refusal.detail}</p>}
    </form>
  );
}
