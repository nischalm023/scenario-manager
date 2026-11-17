// Spec 03 section 3.6, the Map panel (`/w/{site}/map@{pin}`, mockup 04; BACKLOG task 14): the site drawn as a run of
// that cut would load it, through the shared WorldviewMount (orthographic by default, orbit beside it; ruling D119),
// beside its text twin: the agent's highlights from the address's journal entry, the selection, what the site holds,
// what it was made from, and Find. Panel commands reach it by the address (`highlights`, `select`) and by `data-ref`
// (spec 03 section 3.3). On a branch it follows every scope of its cut; a new floor key redraws with the camera kept
// and re-resolves the highlights and the selection. Display only (ruling D53).

import { useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import type { MapSummary, RefMarks } from '../../protocol/map.ts';
import type { Problem } from '../../protocol/problem.ts';
import { useShell } from '../../shell/context.ts';
import { useFillWindow } from '../../shell/fillWindow.ts';
import { printKinPins, printLocation, printPin, siteRootOf } from '../../shell/grammar.ts';
import { ServicesContext } from '../../shell/panels.tsx';
import { routeFor, withParam, type Route } from '../../shell/routes.ts';
import { ShellLink } from '../../shell/ShellLink.tsx';
import { WorldviewMount, type MountHighlight, type StageWords } from '../../viewer/WorldviewMount.tsx';
import { FindForm } from './FindForm.tsx';
import { useFollowing } from './following.ts';
import { Census, Figures, HighlightList, MadeFromSection, SelectedPane, type ShownGroup } from './MapLists.tsx';
import { Info, TipFact } from '../../shell/Info.tsx';
import { combinedMarks } from './marks.ts';
import { readHighlightEntry, readSummary, resolveOnMap } from './reads.ts';
import { mapSeq, markPlace, stageLabel, summaryLine } from './words.ts';

/** How the newest read of the summary went; the summary itself is kept apart, so a failed refresh loses nothing. */
type Request =
  | { state: 'reading'; since: number }
  | { state: 'read' }
  | { state: 'failed'; reason: string; problem: Problem | null };

/** Whole seconds past which the stage counts the wait aloud on its line (wall clock). */
const COUNT_AFTER_S = 2;
/** Spec 03 section 3.8: a failed read is tried once more, this long after it (wall clock). */
const RETRY_MS = 10_000;

export function MapPanel({ route }: { route: Extract<Route, { view: 'world' }> }) {
  const shell = useShell();
  const services = useContext(ServicesContext);
  const location = printLocation(route.location);
  const kin = route.kin ? printKinPins(route.kin) : undefined;
  const site = siteRootOf(route.location.scope);
  // The site's template page, at the branch the map follows: it reads the site's root, which a map's changeset is not.
  const madeFromRoute = site === null ? null
    : `/w/${site}@${route.location.pin.kind === 'changeset' ? 'main' : printPin(route.location.pin)}?panel=template`;
  const [summary, setSummary] = useState<MapSummary | null>(null);
  const [request, setRequest] = useState<Request>(() => ({ state: 'reading', since: Date.now() }));
  const [rereads, setRereads] = useState(0);
  const [waited, setWaited] = useState(0);
  const floorKey = summary?.floor?.key ?? null;

  useEffect(() => {
    let current = true;
    void readSummary(location, kin).then((answer) => {
      if (!current) return;
      if (answer.ok) {
        setSummary(answer.value);
        setRequest({ state: 'read' });
      } else {
        setRequest({ state: 'failed', reason: answer.reason, problem: answer.problem });
      }
    });
    return () => {
      current = false;
    };
  }, [location, kin, rereads]);

  useEffect(() => {
    if (request.state !== 'reading') return;
    const timer = setInterval(() => setWaited(Math.floor((Date.now() - request.since) / 1000)), 1000);
    return () => clearInterval(timer);
  }, [request]);

  const reread = useCallback(() => setRereads((n) => n + 1), []);
  const fill = useFillWindow('Map');
  // Once per failure, until a read succeeds again: the automatic retry that fails waits for a move or a press.
  const retried = useRef(false);
  useEffect(() => {
    if (request.state === 'read') retried.current = false;
    if (request.state !== 'failed' || retried.current) return;
    const timer = setTimeout(() => {
      retried.current = true;
      reread();
    }, RETRY_MS);
    return () => clearTimeout(timer);
  }, [request, reread]);
  const following = route.location.pin.kind === 'branch' && summary !== null ? summary.resolved.map((row) => row.scope) : [];
  useFollowing(following, reread);

  // The agent's highlights the address names, resolved at this cut, each group's marks in mark order.
  const [groups, setGroups] = useState<{ seq: number; key: string | null; groups: ShownGroup[]; why: string | null; shown: boolean } | null>(null);
  useEffect(() => {
    const seq = route.highlights;
    if (seq === undefined || site === null || floorKey === null) return;
    let current = true;
    void (async () => {
      const entry = await readHighlightEntry(site, seq);
      if (!entry.ok) {
        if (current) setGroups({ seq, key: floorKey, groups: [], why: entry.why, shown: false });
        return;
      }
      const asked = entry.command.highlight ?? [];
      const resolved = await resolveOnMap(location, kin, asked.flatMap((group) => group.refs));
      if (!current) return;
      if (!resolved.ok) {
        const at = summary !== null ? `map changeset ${mapSeq(summary) ?? 0}` : 'this map';
        setGroups({ seq, key: floorKey, groups: [], why: `These highlights no longer resolve at ${at}: ${resolved.reason}`, shown: false });
        return;
      }
      const byRef = new Map(resolved.value.map((ref) => [ref.ref, ref]));
      const shown = asked.map((group) => {
        const refs = group.refs.map((ref) => byRef.get(ref)).filter((ref): ref is RefMarks => ref !== undefined);
        return { note: group.note, refs, marks: combinedMarks(refs) };
      });
      setGroups({ seq, key: floorKey, groups: shown, why: null, shown: entry.command.action === 'show' });
    })();
    return () => {
      current = false;
    };
  }, [route.highlights, site, location, kin, floorKey, summary]);
  const shownGroups = groups !== null && groups.seq === route.highlights ? groups : null;

  // The one selection, resolved at this cut with what else stood on the cell it was picked from.
  const lastPick = useRef<string[]>([]);
  const spokenOnSelect = useRef<string | null>(null);
  const [selected, setSelected] = useState<{ key: string; ref: RefMarks | null; also: RefMarks[]; dropped: string | null } | null>(null);
  useEffect(() => {
    const key = route.select;
    if (key === undefined || floorKey === null) return;
    let current = true;
    const also = lastPick.current[0] === key ? lastPick.current.slice(1) : [];
    void resolveOnMap(location, kin, [key, ...also]).then((answer) => {
      if (!current) return;
      if (!answer.ok) {
        setSelected({ key, ref: null, also: [], dropped: `The selection is dropped: ${answer.reason}` });
        return;
      }
      const [ref, ...rest] = answer.value;
      setSelected({ key, ref, also: rest, dropped: null });
      if (spokenOnSelect.current === key) {
        spokenOnSelect.current = null;
        const place = markPlace(ref.marks.find((mark) => mark.key === key) ?? ref.marks[0] ?? { centre_mm: null });
        services?.announce(`Selected ${ref.label}${place !== null ? `, ${place}` : ''}`);
      }
    });
    return () => {
      current = false;
    };
  }, [route.select, location, kin, floorKey, services]);
  const shownSelection = route.select !== undefined && selected?.key === route.select ? selected : null;

  const selectRoute = useCallback((key: string) => routeFor(withParam(route, 'select', key)), [route]);
  const select = useCallback((key: string) => shell.navigate(selectRoute(key), 'within'), [shell, selectRoute]);
  const clearSelection = useCallback(() => shell.navigate(routeFor(withParam(route, 'select', undefined)), 'within'), [shell, route]);
  const clearHighlights = () => {
    shell.navigate(routeFor(withParam(route, 'highlights', undefined)), 'within');
    services?.announce('Highlights cleared');
  };
  const pick = (keys: string[], how: 'pointer' | 'centre') => {
    lastPick.current = keys;
    if (keys.length === 0) {
      if (how === 'centre') services?.announce('Nothing under the centre cross');
      return;
    }
    if (how === 'centre') spokenOnSelect.current = keys[0];
    select(keys[0]);
  };

  const labels = useRef(new Map<string, Promise<string | null>>());
  const labelOf = useCallback((key: string) => {
    const asked = `${location}|${kin ?? ''}|${key}`;
    let label = labels.current.get(asked);
    if (label === undefined) {
      label = resolveOnMap(location, kin, [key]).then((answer) => (answer.ok ? answer.value[0]?.label ?? null : null));
      labels.current.set(asked, label);
    }
    return label;
  }, [location, kin]);

  const highlights = useMemo<MountHighlight[]>(
    () => (shownGroups?.groups ?? []).map((group) => ({ note: group.note, by: 'agent', marks: group.marks.filter((mark) => mark.on_floor).map((mark) => mark.key) })),
    [shownGroups],
  );
  const axes = useMemo(() => (summary === null ? null : { name: summary.axes.name, x: summary.axes.x, y: summary.axes.y }), [summary]);
  const selectedRef = shownSelection?.ref ?? null;
  const stageSelection = useMemo(
    () => (selectedRef === null ? null : { key: selectedRef.key, marks: selectedRef.marks.filter((mark) => mark.on_floor).map((mark) => mark.key) }),
    [selectedRef],
  );
  const highlighted = highlights.reduce((sum, group) => sum + group.marks.length, 0);
  const name = summary?.site.label ?? site ?? route.location.scope;

  const failure = request.state === 'failed' ? request.problem?.detail ?? `The map could not be read: ${request.reason}` : null;
  const waiting: StageWords | null = summary === null
    ? failure !== null
      ? { state: 'failed', words: failure, retry: reread }
      : { state: 'loading', words: `Compiling the floor of ${name}${waited > COUNT_AFTER_S ? `, ${waited} s so far` : ''}.` }
    : summary.empty
      ? { state: 'empty', words: `The map of ${summary.site.label} has nothing on it at changeset ${mapSeq(summary) ?? 0}.` }
      : summary.floor === null
        ? { state: 'failed', words: summary.floor_problem?.detail ?? 'This map cannot be drawn.', retry: reread }
        : null;

  return (
    <>
      <header className="home__head page-head draft-head">
        <span className="page-head__icon" aria-hidden="true">
          <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M12 2l9 5-9 5-9-5z" /><path d="M3 12l9 5 9-5" /><path d="M3 17l9 5 9-5" />
          </svg>
        </span>
        <div>
          <p className="home__title">Maps <Info label="this map">
            <TipFact term="Display only">Drawing and moving things arrives after the proof of concept.</TipFact>
            <TipFact term="To change it">Update the site from its preset with other settings.</TipFact>
            <TipFact term="Keys">Arrows pan, + and − zoom, 0 fits, Enter picks the centre, O switches 2D and 3D.</TipFact>
          </Info></p>
          <p className="home__sub">Layout shape &amp; size, aisles, bays, zones, and pick locations.</p>
        </div>
      </header>
      <FindForm location={location} kin={kin} selection={route.select ?? null} selectRoute={selectRoute} onSelect={select} announce={(sentence) => services?.announce(sentence)} />
      {summary !== null && summary.empty && site !== null && (
        <p><ShellLink to={`/w/${site}/map@main`}>The newest changeset of the map</ShellLink></p>
      )}
      {summary !== null && !summary.exact && <p className="muted">{summary.exact_reason}</p>}
      {summary !== null && failure !== null && (
        <p className="muted">
          {failure} Shown as last read. <button className="btn btn--small" type="button" onClick={reread}>Retry</button>
        </p>
      )}
      <div className="map-layout" {...fill.region}>
        <WorldviewMount fill={fill}
          source={{ kind: 'scope', location: summary?.location ?? location, ...(kin ? { kin } : {}) }}
          floorKey={waiting === null ? floorKey : null} waiting={waiting} interactive panel="map"
          label={summary === null ? `${name}, ${location.split('@')[1] === 'main' ? 'map following main' : location}` : stageLabel(summary)}
          description={summary === null ? 'The floor is being read.' : summaryLine(summary, highlighted, selectedRef?.label ?? null)}
          axes={axes} selection={stageSelection} highlights={highlights} highlightSeq={route.highlights ?? null}
          onPick={pick} onClearSelection={clearSelection} labelOf={labelOf} frameOnOpen={shownGroups?.shown ?? false}
        />
        <div className="side-stack">
          {summary !== null && <Figures summary={summary} />}
          {summary !== null && <MadeFromSection summary={summary} history={madeFromRoute} />}
          {(route.highlights !== undefined || (shownGroups?.groups.length ?? 0) > 0) && (
            <HighlightList
              groups={shownGroups?.groups ?? []} why={shownGroups?.why ?? null} selection={route.select ?? null}
              selectRoute={selectRoute} onSelect={select} onClear={route.highlights !== undefined ? clearHighlights : null}
            />
          )}
          <SelectedPane
            selected={selectedRef} also={shownSelection?.also ?? []} dropped={shownSelection?.dropped ?? null}
            selectRoute={selectRoute} onSelect={select} onClear={clearSelection}
          />
          {summary !== null && <Census summary={summary} />}
        </div>
      </div>
    </>
  );
}
