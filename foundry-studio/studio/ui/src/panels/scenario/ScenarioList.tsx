// Spec 07 section 5.1, the Scenario list at `/scenarios?site=` (spec 03 section 3.2): the site's scenarios as a
// table, each name linking to its panel, its latest version and that version's state, its runs and when it last
// changed (wall clock); empty, a sentence with New scenario and "or ask the agent". New scenario asks for a name
// inline and opens version 1 at section 3.2's stated defaults, focus on its title (brief I1.08, lead decided).

import { useEffect, useRef, useState, type FormEvent } from 'react';
import type { CreatedScenario, ScenarioList as List } from '../../protocol/scenario.ts';
import type { SiteList } from '../../protocol/site.ts';
import { clause, getJson, postJson } from '../../shell/api.ts';
import { useShell } from '../../shell/context.ts';
import { ShellLink } from '../../shell/ShellLink.tsx';
import { pressKey } from './api.ts';
import { WallTime } from './ReadinessLine.tsx';

type Listed = { state: 'reading' } | { state: 'read'; list: List } | { state: 'failed'; reason: string };

export function ScenarioList({ site }: { site?: string }) {
  const shell = useShell();
  const [listed, setListed] = useState<Listed>({ state: 'reading' });
  const [siteName, setSiteName] = useState<string | null>(null);
  const [naming, setNaming] = useState(false);
  const [name, setName] = useState('');
  const [refusal, setRefusal] = useState<{ words: string; existing: string | null } | null>(null);
  const nameInput = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (site === undefined) return;
    let current = true;
    void getJson<List>(`/api/scenarios?site=${encodeURIComponent(site)}`).then((answer) => {
      if (current) setListed(answer.ok ? { state: 'read', list: answer.value } : { state: 'failed', reason: clause(answer.reason) });
    });
    void getJson<SiteList>('/api/sites').then((answer) => {
      if (current && answer.ok) setSiteName(answer.value.items.find((item) => item.site === site)?.name ?? null);
    });
    return () => {
      current = false;
    };
  }, [site]);

  if (site === undefined) {
    return <p>The scenarios belong to a site: <ShellLink to="/">open a site</ShellLink> to see its scenarios.</p>;
  }
  const named = siteName ?? site;

  const start = () => {
    setNaming(true);
    window.setTimeout(() => nameInput.current?.focus(), 0);
  };
  const create = async (event: FormEvent) => {
    event.preventDefault();
    const answer = await postJson<CreatedScenario>('/api/scenarios', { name: name.trim(), site_scope: site }, pressKey());
    if (answer.ok) {
      shell.navigate(`/scenarios/${answer.value.scenario}`, 'user');
      return;
    }
    const existing = answer.problem?.problems?.[0]?.ref?.replace(/^scenario:/, '') ?? null;
    const words = answer.problem?.type === 'name-taken' ? `${name.trim()} already exists on this site.`
      : answer.problem?.problems?.[0]?.text ?? `Not made: ${clause(answer.reason)}.`;
    setRefusal({ words, existing });
    shell.announce(words);
  };

  const head = (
    <header className="home__head page-head draft-head">
      <span className="page-head__icon" aria-hidden="true">
        <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" /></svg>
      </span>
      <div>
        <p className="home__title">Scenarios</p>
        <p className="home__sub">What a run simulates on {named}: the orders it generates, and how long it runs.</p>
      </div>
      {!naming && <button className="btn btn--pill btn--accent draft-head__back" type="button" onClick={start}>New scenario</button>}
    </header>
  );
  const newScenario = naming && (
    <form className="setup-card scenario-new" onSubmit={(event) => void create(event)}>
      <div className="setup-card__head"><h3 className="setup-card__title">New scenario</h3><span className="pill">version 1 at the defaults</span></div>
      <div className="scenario-new__row">
        <div className="field">
          <label className="field__label" htmlFor="sc-new-name">Name of the new scenario</label>
          <input
            className="input" id="sc-new-name" name="name" type="text" maxLength={120} required value={name} ref={nameInput}
            aria-describedby={refusal ? 'sc-new-hint sc-new-error' : 'sc-new-hint'} aria-invalid={refusal ? true : undefined}
            onChange={(event) => { setName(event.target.value); setRefusal(null); }}
          />
        </div>
        <button className="btn btn--pill btn--accent" type="submit">Create scenario</button>
        <button className="btn btn--pill" type="button" onClick={() => { setNaming(false); setRefusal(null); }}>Cancel</button>
      </div>
      <p className="field__hint" id="sc-new-hint">It opens at the stated defaults: 300 orders an hour for 4 hours of simtime, every order at the start. Change any of it on the scenario.</p>
      {refusal !== null && (
        <p className="field__error" id="sc-new-error">
          {refusal.words}{refusal.existing !== null && <> <ShellLink to={`/scenarios/${refusal.existing}`}>Open it</ShellLink>.</>}
        </p>
      )}
    </form>
  );

  if (listed.state === 'reading') return <div aria-busy="true"><p className="panel__status">Reading the scenarios of {named}</p></div>;
  if (listed.state === 'failed') return <p className="panel__status" data-state="failed">The scenarios could not be read: {listed.reason}.</p>;
  const items = listed.list.items;
  const runs = items.reduce((sum, item) => sum + item.runs.total, 0);
  const running = items.reduce((sum, item) => sum + item.runs.running, 0);
  return (
    <div className="home setup-page">
      {head}
      <ul className="evaluation__summary">
        <li className="figure-tile"><span className="figure-tile__value">{items.length}</span><span className="figure-tile__label">Scenarios</span></li>
        <li className="figure-tile"><span className="figure-tile__value">{runs}</span><span className="figure-tile__label">Runs in all</span></li>
        <li className="figure-tile"><span className="figure-tile__value">{running}</span><span className="figure-tile__label">Running now</span></li>
      </ul>
      {newScenario}
      {items.length === 0 ? (
        !naming && (
          <section className="setup-card setup-card--empty">
            <h3 className="setup-card__title">No scenario on {named} yet</h3>
            <p className="muted">A scenario says what a run simulates. Make one with New scenario above, or ask the agent in the chat.</p>
          </section>
        )
      ) : (
        <ul className="setup-grid scenario-cards" aria-label={`The scenarios of ${named}`}>
          {items.map((item) => (
            <li className="setup-card scenario-card" key={item.id} data-ref={`scenario:${item.id}`}>
              <div className="setup-card__head">
                <h3 className="setup-card__title"><ShellLink className="scenario-card__name" to={`/scenarios/${item.id}`}>{item.name}</ShellLink></h3>
                <span className="pill" data-tone={item.state === 'sealed' ? 'good' : 'info'}>v{item.latest_version} · {item.state}</span>
              </div>
              <ul className="stat-row">
                <li><b>{item.runs.total}</b> run{item.runs.total === 1 ? '' : 's'}</li>
                {item.runs.running > 0 && <li><b>{item.runs.running}</b> running</li>}
              </ul>
              <p className="setup-card__foot muted">Changed <WallTime at={item.changed_at} date /></p>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
