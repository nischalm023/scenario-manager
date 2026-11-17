// Spec 03 section 5's Find on the Map panel: a search field whose submit keeps focus in the field, lists what matched
// after it as `ul.find-results` of entity chips in mark order, and has the announcer speak the count as the outcome
// of the person's own act ("3 results for 'pick 3'", "Nothing matches 'pick 33'"); choosing a result selects it.

import { useState, type FormEvent } from 'react';
import type { Mark } from '../../protocol/map.ts';
import { Chip } from './MapLists.tsx';
import { findOnMap } from './reads.ts';
import { grouped, markPlace } from './words.ts';

export function FindForm({ location, kin, selection, selectRoute, onSelect, announce }: {
  location: string; kin: string | undefined; selection: string | null; selectRoute: (key: string) => string;
  onSelect: (key: string) => void; announce: (sentence: string) => void;
}) {
  const [words, setWords] = useState('');
  const [found, setFound] = useState<{ words: string; items: Mark[]; more: boolean } | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const asked = words.trim();
    if (asked === '') return;
    const answer = await findOnMap(location, kin, asked);
    if (!answer.ok) {
      setFailure(`Find failed: ${answer.reason}`);
      announce(`Find failed: ${answer.reason}`);
      return;
    }
    setFailure(null);
    setFound({ words: asked, items: answer.value.items, more: answer.value.more });
    const count = answer.value.items.length;
    announce(count === 0 ? `Nothing matches '${asked}'` : `${grouped(count)}${answer.value.more ? ' or more' : ''} results for '${asked}'`);
  };
  return (
    <>
      <form className="panel__toolbar" role="search" aria-label="Find on the map" onSubmit={(event) => void submit(event)}>
        <label className="visually-hidden" htmlFor="map-find">Find an entity by name, kind or external id</label>
        <input
          className="input" id="map-find" type="search" placeholder="Find a station, aisle or robot" value={words}
          onChange={(event) => setWords(event.target.value)}
        />
        <button className="btn btn--small" type="submit">Find</button>
      </form>
      {failure !== null && <p className="field__error">{failure}</p>}
      {found !== null && found.items.length > 0 && (
        <ul className="find-results" aria-label={`Results for '${found.words}'`}>
          {found.items.map((mark) => (
            <li key={mark.key}>
              <Chip mark={mark} to={selectRoute(mark.key)} current={mark.key === selection} onSelect={onSelect} />{' '}
              {markPlace(mark) !== null && <span className="muted">{markPlace(mark)}</span>}
            </li>
          ))}
        </ul>
      )}
    </>
  );
}
