// Spec 03 section 5's text twin beside the Map panel's stage: the agent's highlights, group by group with every mark
// in position order (the first 50, then "Show all"), what each group marks off the floor listed apart, and Clear
// highlights; the Selected pane with "Also on this cell" and Clear the selection; "What the site holds", one row per
// set of components; and Made from. Each side section carries a heading and no name, so only the mount is a region.

import { useState, type MouseEvent } from 'react';
import type { MapSummary, Mark, RefMarks } from '../../protocol/map.ts';
import { ShellLink } from '../../shell/ShellLink.tsx';
import { censusCaption, grouped, kindOf, mapFigures, markPlace } from './words.ts';

/** Marks listed per group before "Show all" (spec 03 section 3.5). */
const LISTED = 50;

export interface ShownGroup {
  note: string;
  refs: RefMarks[];
  marks: Mark[];
}

/** An entity chip that selects what it names, keeping focus on itself (spec 03 section 5). */
export function Chip({ mark, to, current, onSelect }: {
  mark: Pick<Mark, 'key' | 'label' | 'components'>; to: string; current: boolean; onSelect: (key: string) => void;
}) {
  const select = (event: MouseEvent<HTMLAnchorElement>) => {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    onSelect(mark.key);
  };
  return (
    <a className="entity-chip" href={to} data-ref={mark.key} aria-current={current ? 'true' : undefined} onClick={select}>
      {mark.label} <span className="entity-chip__kind">{kindOf(mark.components)}</span>
    </a>
  );
}

export function HighlightList({ groups, why, selection, selectRoute, onSelect, onClear }: {
  groups: ShownGroup[]; why: string | null; selection: string | null; selectRoute: (key: string) => string;
  onSelect: (key: string) => void; onClear: (() => void) | null;
}) {
  const [all, setAll] = useState<Set<number>>(new Set());
  return (
    <section className="section">
      <h3 className="section__title">Highlighted</h3>
      {why !== null && <p className="muted">{why}</p>}
      {why === null && groups.length === 0 && <p className="muted">Nothing is highlighted by the agent.</p>}
      {groups.map((group, n) => {
        const placed = group.marks.filter((mark) => mark.on_floor);
        const apart = group.marks.filter((mark) => !mark.on_floor);
        const shown = all.has(n) ? placed : placed.slice(0, LISTED);
        return (
          <div className="highlights__group" key={n}>
            <p className="highlight-note">
              <span className="pill" data-tone="agent">agent</span>{' '}
              {group.refs.map((ref) => <Chip key={ref.key} mark={ref} to={selectRoute(ref.key)} current={ref.key === selection} onSelect={onSelect} />)}
            </p>
            <p className="highlights__note">
              {group.note}. {grouped(group.marks.length)} {group.marks.length === 1 ? 'entity' : 'entities'}, {grouped(placed.length)} on the floor
              {placed.length > 0 ? ', in order along the floor:' : '.'}
            </p>
            {placed.length > 0 && (
              <ol className="highlights">
                {shown.map((mark) => (
                  <li key={mark.key}>
                    <Chip mark={mark} to={selectRoute(mark.key)} current={mark.key === selection} onSelect={onSelect} />{' '}
                    <span className="muted">{markPlace(mark)}</span>
                  </li>
                ))}
              </ol>
            )}
            {shown.length < placed.length && (
              <button className="btn btn--small highlights__more" type="button" onClick={() => setAll(new Set([...all, n]))}>
                Show all {grouped(placed.length)}
              </button>
            )}
            {apart.length > 0 && (
              <>
                <p className="highlights__note">Not drawn on this panel, standing off the floor's cells or nowhere:</p>
                <ul className="highlights">
                  {apart.map((mark) => (
                    <li key={mark.key}>
                      <Chip mark={mark} to={selectRoute(mark.key)} current={mark.key === selection} onSelect={onSelect} />{' '}
                      {markPlace(mark) !== null && <span className="muted">{markPlace(mark)}</span>}
                    </li>
                  ))}
                </ul>
              </>
            )}
          </div>
        );
      })}
      {onClear !== null && <button className="btn btn--small" type="button" onClick={onClear}>Clear highlights</button>}
    </section>
  );
}

export function SelectedPane({ selected, also, dropped, selectRoute, onSelect, onClear }: {
  selected: RefMarks | null; also: RefMarks[]; dropped: string | null; selectRoute: (key: string) => string;
  onSelect: (key: string) => void; onClear: () => void;
}) {
  const place = selected?.marks.find((mark) => mark.key === selected.key) ?? selected?.marks[0] ?? null;
  // What stands nowhere itself, a sector, is outlined by its members on the floor (spec 03 section 3.5).
  const standsItself = selected?.marks.some((mark) => mark.key === selected.key && mark.on_floor) ?? false;
  const outlinedBy = standsItself ? 0 : selected?.marks.filter((mark) => mark.on_floor).length ?? 0;
  return (
    <section className="detail-pane">
      <h3 className="detail-pane__title">Selected</h3>
      {dropped !== null && <p className="muted">{dropped}</p>}
      {selected === null ? (
        <p className="muted">Nothing selected. Click the map, press Enter on the map, choose an entity above, or use Find.</p>
      ) : (
        <>
          <p><Chip mark={selected} to={selectRoute(selected.key)} current onSelect={onSelect} /></p>
          <dl className="provenance">
            <dt>Components</dt><dd>{selected.components.join(', ')}</dd>
            <dt>Key</dt><dd><code>{selected.key}</code></dd>
            <dt>Scope</dt><dd>{selected.scope ?? 'none'}</dd>
            <dt>Position</dt><dd>{place !== null ? markPlace(place) ?? 'stands nowhere' : 'stands nowhere'}</dd>
          </dl>
          {outlinedBy > 0 && <p className="muted">Outlined on the map by its {grouped(outlinedBy)} marks; it stands on no cell itself.</p>}
          {also.length > 0 && (
            <>
              <p className="muted">Also on this cell:</p>
              <ul className="highlights">
                {also.map((ref) => <li key={ref.key}><Chip mark={ref} to={selectRoute(ref.key)} current={false} onSelect={onSelect} /></li>)}
              </ul>
            </>
          )}
          <button className="btn btn--small" type="button" onClick={onClear}>Clear the selection (Escape)</button>
        </>
      )}
    </section>
  );
}

export function Figures({ summary }: { summary: MapSummary }) {
  const figures = mapFigures(summary);
  if (figures.length === 0) return null;
  return (
    <section className="section">
      <h3 className="section__title">What this map holds</h3>
      <ul className="evaluation__summary">
        {figures.map((one) => (
          <li className="figure-tile" key={one.label}>
            <span className="figure-tile__value">{grouped(one.value)}</span>
            <span className="figure-tile__label">{one.label}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

export function Census({ summary }: { summary: MapSummary }) {
  return (
    <details className="section census-fold">
      <summary className="section__title">Every entity by kind <span className="muted">({summary.counts.length} kinds)</span></summary>
      <table className="census">
        <caption className="visually-hidden">{censusCaption(summary.resolved)}</caption>
        <tbody>
          {summary.counts.map((row) => (
            <tr key={row.label}><th scope="row">{row.label}</th><td className="num">{grouped(row.count)}</td></tr>
          ))}
        </tbody>
      </table>
    </details>
  );
}

/**
 * The newest make or update of the site: its preset, read at which changesets, and the settings it was given; and the
 * link to the site's template page (`history`), which lists every make and update.
 */
export function MadeFromSection({ summary, history }: { summary: MapSummary; history: string | null }) {
  const made = summary.made_from;
  if (made === null) return <p className="muted">This site was not made from a map preset.</p>;
  const preset = (made.template.split('/').pop() ?? made.template).replace(/-/g, ' ');
  const unit = (knob: { unit: string }) => (knob.unit === 'mm' ? ' mm' : knob.unit === 'deg' ? '°' : '');
  const set = made.knobs.filter((knob) => knob.value !== 0);
  const read = Object.entries(made.template_seqs)
    .map(([scope, seq]) => `${scope.slice(made.template.length + 1) || 'top scope'} at changeset ${seq}`).join(', ');
  return (
    <section className="section preset-card">
      <div className="preset-card__head">
        <span className="site-badge" aria-hidden="true">{preset.charAt(0).toUpperCase()}</span>
        <div>
          <h3 className="section__title">Made from the {preset} preset</h3>
          <p className="muted">
            {made.act === 'create' ? 'Created' : 'Updated'} by {made.author.name},{' '}
            <span className="clock" data-clock="wall">{new Date(made.at).toLocaleString()}</span>
          </p>
        </div>
      </div>
      <p className="muted preset-card__read"><code>{made.template}</code>, {read}</p>
      <ul className="stat-row preset-card__knobs" aria-label="Settings">
        {set.map((knob) => <li key={knob.key}><b>{grouped(knob.value)}{unit(knob)}</b> {knob.label.toLowerCase()}</li>)}
      </ul>
      {history !== null && <p><ShellLink to={history} data-route={history}>Every make and update of this site</ShellLink></p>}
    </section>
  );
}
