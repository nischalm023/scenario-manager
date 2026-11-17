// Spec 05 sections 4.1 and 6, the Template list (`/templates`, mockup 02), titled Maps: the five map presets in the
// POC's order, each card with its title and line, its thumbnail and headline figures at the preset's defaults
// (evaluating until the API has computed it once), or why its defaults do not evaluate; behind its "i", the census
// sentence, each setting's range, its flows and what it was built from. A card is chosen by its radio or a click on it,
// but not on its "i", and the foot starts a draft of it with "Use this map". A filter narrows the list as the person
// types, its count spoken once typing pauses for 1 s of wall clock (spec 02's case A12). Every card carries `data-ref`.

import { useEffect, useRef, useState, type MouseEvent } from 'react';
import type { TemplateCard, TemplateList as Listed } from '../../protocol/template.ts';
import { useShell } from '../../shell/context.ts';
import { Info, TipFact, TipLine } from '../../shell/Info.tsx';
import { getJson } from './requests.ts';
import { startDraft } from './start.ts';
import { FloorSnapshot } from '../../viewer/FloorSnapshot.tsx';
import { censusSentence, grouped, headlineFigures, problemWords, rangeWords } from './words.ts';

type ListState = { state: 'loading' } | { state: 'ready'; items: TemplateCard[] } | { state: 'failed'; reason: string };

const COUNT_PAUSE_MS = 1000;
const THUMBNAIL_POLL_MS = 1500;

/** A preset's figures, those it has: what the card's line and the bar's strip both read. */
function highlights(card: TemplateCard): { label: string; value: number }[] {
  const census = card.thumbnail.state === 'ready' ? card.thumbnail.census : null;
  return census === null ? [] : headlineFigures(census);
}

/** What a card says only when asked: the census at the defaults, each setting's range, the flows, its source. */
function CardInfo({ card }: { card: TemplateCard }) {
  const thumbnail = card.thumbnail;
  return (
    <Info label={card.title}>
      {thumbnail.state === 'ready' && (
        <TipLine>{censusSentence(`${card.title} at its defaults`, thumbnail.census, card.knobs, thumbnail.problems)}</TipLine>
      )}
      {card.knobs.map((knob) => <TipFact key={knob.key} term={knob.label}>{rangeWords(knob)}</TipFact>)}
      <TipFact term="Flows">{card.flows.length > 0 ? card.flows.join('; ') : 'none'}</TipFact>
      {card.built_from !== null && <TipFact term="Built from">{card.built_from}</TipFact>}
    </Info>
  );
}

/** Whether a click on a card chooses it: not one on its "i", or in the words the "i" opened, which only say what the
 * preset holds, so looking into another preset never changes which one "Use this map" starts. */
const chooses = (event: MouseEvent<HTMLElement>): boolean => !(event.target instanceof Element && event.target.closest('.info') !== null);

const matches = (filter: string) => (card: TemplateCard): boolean => {
  const wanted = filter.trim().toLowerCase();
  return wanted === '' || [card.title, card.text, card.built_from ?? '', card.ref].some((text) => text.toLowerCase().includes(wanted));
};

export function TemplateList() {
  const shell = useShell();
  const [list, setList] = useState<ListState>({ state: 'loading' });
  const [reload, setReload] = useState(0);
  const [filter, setFilter] = useState('');
  const typed = useRef(false);
  const [starting, setStarting] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const read = async () => {
      const answer = await getJson<Listed>('/api/templates');
      if (!current) return;
      if (!answer.ok) {
        setList({ state: 'failed', reason: answer.reason });
        return;
      }
      setList({ state: 'ready', items: answer.value.items });
      if (answer.value.items.some((card) => card.thumbnail.state === 'evaluating')) timer = setTimeout(() => void read(), THUMBNAIL_POLL_MS);
    };
    void read();
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [reload]);

  const items = list.state === 'ready' ? list.items : [];
  const shown = items.filter(matches(filter));
  const count = `${shown.length} of ${items.length} maps shown`;

  useEffect(() => {
    if (!typed.current) return;
    const timer = setTimeout(() => shell.announce(count), COUNT_PAUSE_MS);
    return () => clearTimeout(timer);
  }, [count, filter, shell]);

  const use = async (card: TemplateCard) => {
    if (starting !== null) return;
    setStarting(card.ref);
    setFailure(null);
    const answer = await startDraft(card.ref, shell.browserTab);
    setStarting(null);
    if (answer.ok) {
      shell.navigate(`/template-drafts/${answer.value.draft}`, 'user');
      return;
    }
    const sentence = `${card.title} could not be started: ${answer.reason}`;
    setFailure(sentence);
    shell.announce(sentence);
  };

  const chosen = items.find((card) => card.ref === selected) ?? null;
  const chosenStats = chosen !== null ? highlights(chosen) : [];

  return (
    <>
    <div className="home template-pick">
      <header className="home__head page-head">
        <span className="page-head__icon" aria-hidden="true">
          <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M12 2l9 5-9 5-9-5z" /><path d="M3 12l9 5 9-5" /><path d="M3 17l9 5 9-5" />
          </svg>
        </span>
        <div>
          <p className="home__title">Maps</p>
          <p className="home__sub">Layout shape &amp; size, aisles, bays, zones, and pick locations.</p>
        </div>
      </header>
      <p className="home__heading">Choose your map source</p>
      <ul className="source-list" aria-label="Map source">
        <li className="source-card" data-selected="true">
          <span className="source-card__dot" aria-hidden="true" />
          <strong>Presets</strong>
          <span>Browse and apply from the map preset library.</span>
        </li>
        {[['Upload file', 'Import a JSON map document.'], ['Import from Solutions', 'Bring in a map artifact from this solution.'], ['Quick Maps', 'Open the map creator.']].map(([name, text]) => (
          <li className="source-card" aria-disabled="true" key={name} title={`${name}, coming soon`}>
            <span className="source-card__dot" aria-hidden="true" />
            <strong>{name}</strong>
            <span>{text}</span>
          </li>
        ))}
      </ul>
      <div className="home__section">
        <p className="home__heading">Map preset</p>
        <label className="visually-hidden" htmlFor="template-filter">Filter maps</label>
        <input
          className="input template-pick__filter" id="template-filter" type="search" placeholder="Filter maps" value={filter}
          aria-describedby="template-filter-count"
          onChange={(event) => {
            typed.current = true;
            setFilter(event.target.value);
          }}
        />
      </div>
      {list.state === 'loading' && <p className="panel__status" aria-busy="true">Reading the map presets</p>}
      {list.state === 'failed' && (
        <div className="panel__error">
          <p>The map presets could not be read: {list.reason}</p>
          <button className="btn btn--small" type="button" onClick={() => setReload((n) => n + 1)}>Retry</button>
        </div>
      )}
      {list.state === 'ready' && items.length === 0 && <p>No map presets yet.</p>}
      {failure !== null && <p className="field__error">{failure}</p>}
      {list.state === 'ready' && items.length > 0 && (
        <ul className="template-list" aria-label="Map presets">
          {shown.map((card, index) => {
            const title = `template-card-${index}-title`;
            const thumbnail = card.thumbnail;
            const on = card.ref === selected;
            const some = highlights(card).slice(0, 3);
            return (
              <li className="template-card" data-ref={`scope:${card.ref}`} data-selected={on ? 'true' : undefined} key={card.ref}
                onClick={(event) => chooses(event) && setSelected(card.ref)}>
                <div className="template-card__stage" aria-busy={thumbnail.state === 'evaluating'}>
                  {thumbnail.state === 'ready' && thumbnail.floor_key !== null && (
                    <FloorSnapshot className="template-card__thumb" floorKey={thumbnail.floor_key} label={`${card.title} at its defaults, drawn north-up`} />
                  )}
                  {thumbnail.state === 'evaluating' && <span className="muted">Drawing…</span>}
                  {thumbnail.state === 'failed' && <span className="muted">Does not draw</span>}
                </div>
                <div className="template-card__body">
                  <div className="template-card__head">
                    <label className="template-card__pick">
                      <input type="radio" name="template" checked={on} onChange={() => setSelected(card.ref)} aria-describedby={title} />
                      <h3 className="template-card__title" id={title}>{card.title}</h3>
                    </label>
                    <CardInfo card={card} />
                  </div>
                  <p className="template-card__text">{card.text}</p>
                  {thumbnail.state === 'failed' && (
                    <p className="template-card__failure">Its defaults do not evaluate: {thumbnail.problems[0]?.text ?? 'no reason was given'}</p>
                  )}
                  {some.length > 0 && (
                    <ul className="stat-row" aria-label={`${card.title} at its defaults`}>
                      {some.map((one) => (
                        <li key={one.label}><b>{grouped(one.value)}</b> {one.label.toLowerCase()}</li>
                      ))}
                    </ul>
                  )}
                  {thumbnail.problems.length > 0 && <span className="pill" data-tone="warn">{problemWords(thumbnail.problems)}</span>}
                </div>
              </li>
            );
          })}
        </ul>
      )}
      <p className="field__hint" id="template-filter-count">{count}</p>
    </div>
    <footer className="pinned-foot" aria-label="Chosen map">
        <div className="apply-bar__what">
          <strong>{chosen !== null ? chosen.title : 'No map chosen'}</strong>
          <span className="muted">{chosen !== null ? chosen.text : 'Choose a preset above to see what it holds.'}</span>
          {chosen !== null && chosenStats.length > 0 && (
            <ul className="stat-row apply-bar__stats" aria-label={`${chosen.title} at its defaults`}>
              {chosenStats.map((one) => <li key={one.label}><b>{grouped(one.value)}</b> {one.label.toLowerCase()}</li>)}
            </ul>
          )}
        </div>
        <div className="apply-bar__actions">
          <button className="btn btn--pill" type="button" disabled={chosen === null || starting !== null} onClick={() => setSelected(null)}>Cancel</button>
          <button className="btn btn--pill btn--accent" type="button" aria-disabled={chosen === null ? true : undefined}
            onClick={() => chosen !== null && void use(chosen)}>
            {starting !== null ? 'Starting a draft…' : 'Use this map'}
          </button>
        </div>
    </footer>
    </>
  );
}
