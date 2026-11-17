// Spec 05 section 6, the evaluation section of the MapTemplate panel (mockup 03): the stage, the shared WorldviewMount
// drawing the newest evaluation's floor through the worldview renderer (rulings D119 and D121: one renderer,
// orthographic by default, orbit beside it), focusable with its keys and buttons, and keeping the last drawn evaluation
// marked with its number when the newest has no floor; the census, its headline figures as tiles and every figure in a
// line under them at its place in the census, each tile, figure and the problems' count carrying the `data-ref` the
// agent highlights it by; what the knobs added or left out against the template's defaults; and the problems, each
// opening with its severity in words.
// The canvas is never the only carrier: every count and problem is text beside it. With no selection on an
// evaluation, Enter names what is under the centre cross (spec 02's A12).

import { Fragment, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import type { FrameAxes, Resolution } from '../../protocol/map.ts';
import type { KnobDeclaration } from '../../protocol/template.ts';
import type { TemplateEvaluation } from '../../protocol/template_evaluation.ts';
import { postJson } from '../../shell/api.ts';
import { ServicesContext } from '../../shell/panels.tsx';
import { axesSentence } from '../../viewer/stage.ts';
import { Info, TipFact } from '../../shell/Info.tsx';
import { WorldviewMount, type MountHighlight } from '../../viewer/WorldviewMount.tsx';
import { censusSentence, figures, grouped, headlineFigures } from './words.ts';

/** How many names of what the knobs added or left out are listed before "and n more". */
const LISTED = 12;
const NO_HIGHLIGHTS: MountHighlight[] = [];

/** The evaluation whose floor is drawn: its number, its floor document's key, and the axes drawn in its corner. */
export interface Drawn {
  n: number;
  floorKey: string;
  axes: FrameAxes | null;
}

function names(changes: { name: string | null; id: string | null }[]): { label: string; ref: string | null }[] {
  return changes.map((change) => ({ label: change.name ?? 'an entity with no label', ref: change.id }));
}

export function EvaluationView({ draft, title, evaluation, drawn, knobs, onDrawn }: {
  draft: string;
  title: string;
  evaluation: TemplateEvaluation | null;
  drawn: Drawn | null;
  knobs: KnobDeclaration[];
  /** Told when the mount has drawn an evaluation's floor, which is when the panel records and says it drawn. */
  onDrawn: (drawn: Drawn) => void;
}) {
  const services = useContext(ServicesContext);
  const labels = useRef(new Map<string, Promise<string | null>>());
  const labelOf = useCallback((key: string) => {
    let label = labels.current.get(key);
    if (label === undefined) {
      const asked = { location: `template-draft:${draft}`, panel: 'template', refs: [key] };
      label = postJson<Resolution>('/api/resolve', asked).then((answer) => (answer.ok ? answer.value.refs[0]?.label ?? null : null));
      labels.current.set(key, label);
    }
    return label;
  }, [draft]);
  const named = useCallback((keys: string[], how: 'pointer' | 'centre') => {
    if (how !== 'centre') return;
    if (keys.length === 0) {
      services?.announce('Nothing under the centre cross');
      return;
    }
    void labelOf(keys[0]).then((label) => services?.announce(`Under the centre cross: ${label ?? 'a floor cell'}`));
  }, [labelOf, services]);
  const nothing = useCallback(() => undefined, []);
  // The floor to draw: the newest evaluation's, else the one drawn last, kept and marked with its number.
  const newest = evaluation?.floor != null ? { n: evaluation.n, floorKey: evaluation.floor.key, axes: evaluation.axes } : null;
  const target = newest ?? drawn;
  const [targetN, targetKey, targetAxes] = [target?.n ?? null, target?.floorKey ?? null, target?.axes ?? null];
  const reported = useCallback((key: string) => {
    if (targetN !== null && key === targetKey) onDrawn({ n: targetN, floorKey: key, axes: targetAxes });
  }, [targetN, targetKey, targetAxes, onDrawn]);
  const axes = useMemo(() => (targetAxes === null ? null : { name: targetAxes.name, x: targetAxes.x, y: targetAxes.y }), [targetAxes]);
  // The expanded map: the same floor in a dialog that fills the window, opened in orbit for looking around.
  const [expanded, setExpanded] = useState(false);
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const element = dialog.current;
    if (element === null) return;
    if (expanded && !element.open) element.showModal();
    if (!expanded && element.open) element.close();
  }, [expanded]);
  const expand = useCallback(() => setExpanded(true), []);

  if (evaluation === null) return <p className="panel__status">Evaluating the template at its defaults</p>;
  const census = evaluation.census;
  const added = names(evaluation.changes_from_template?.added ?? []);
  const removed = names(evaluation.changes_from_template?.removed ?? []);
  const shown = targetN ?? evaluation.n;
  const label = `Evaluation ${shown} of ${title}`;
  const ref = (path: string) => `template-draft:${draft}#/evaluations/${evaluation.n}/${path}`;
  const counted = censusSentence(label, drawn !== null && drawn.n === evaluation.n ? census : null, knobs, evaluation.problems);
  const description = axes === null ? counted : `${counted} ${axesSentence(axes)}`;
  return (
    <>
      <WorldviewMount
        source={{ kind: 'evaluation', draft, n: shown }} floorKey={targetKey}
        waiting={targetKey !== null ? null : evaluation.state === 'failed'
          ? { state: 'failed', words: `Evaluation ${evaluation.n} made no floor; its problems follow.` }
          : { state: 'loading', words: 'Evaluating; the floor is drawn once it is compiled.' }}
        interactive panel="template" label={label} description={description} axes={axes} selection={null}
        highlights={NO_HIGHLIGHTS} highlightSeq={null} onPick={named} onClearSelection={nothing} labelOf={labelOf}
        onDrawn={reported} onExpand={expand}
      />
      <dialog className="map-dialog" ref={dialog} onClose={() => setExpanded(false)} aria-label={`${label}, expanded`}>
        {expanded && (
          <>
            <div className="map-dialog__head">
              <strong>{label}</strong>
              <span className="muted">Drag to turn, scroll to zoom. Switch 2D and 3D on the toolbar.</span>
              <button className="icon-btn icon-btn--ring map-dialog__close" type="button" aria-label="Close the expanded map" title="Close (Escape)" onClick={() => setExpanded(false)}>
                <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
              </button>
            </div>
            <WorldviewMount
              source={{ kind: 'evaluation', draft, n: shown }} floorKey={targetKey} waiting={null}
              interactive panel="template-expanded" label={`${label}, expanded`} description={description} axes={axes} selection={null}
              highlights={NO_HIGHLIGHTS} highlightSeq={null} onPick={named} onClearSelection={nothing} labelOf={labelOf}
              initialProjection="orbit"
            />
          </>
        )}
      </dialog>
      <p className="muted evaluation__state">
        Evaluation {evaluation.n} <span className="pill" data-tone="info">not written</span>
        {evaluation.problems.length === 0
          ? <span className="pill" data-tone="good" data-ref={ref('problems')}>no problems</span>
          : <span className="pill" data-tone={evaluation.problems.some((problem) => problem.severity === 'error') ? 'bad' : 'warn'} data-ref={ref('problems')}>{evaluation.problems.length === 1 ? '1 problem' : `${evaluation.problems.length} problems`}</span>}
        {drawn !== null && evaluation.floor === null && drawn.n !== evaluation.n && <> Drawn: evaluation {drawn.n}. Evaluation {evaluation.n} made no floor; its problems follow.</>}
      </p>
      <section className="section">
        <h3 className="section__title">
          What this map holds
          <Info label="these figures">
            <TipFact term="Aisles, stations">As your settings set them.</TipFact>
            <TipFact term="Storage slots, robots">Derived from the floor these settings make.</TipFact>
            <TipFact term="Written?">Not yet. Nothing is saved until you create the site.</TipFact>
          </Info>
        </h3>
        <ul className="evaluation__summary">
          {census !== null && headlineFigures(census).map((tile) => (
            <li className="figure-tile" data-ref={ref(`census/${tile.key}`)} key={tile.key}>
              <span className="figure-tile__value">{grouped(tile.value)}</span>
              <span className="figure-tile__label">{tile.label}</span>
            </li>
          ))}
        </ul>
        {census !== null && (
          <p className="muted evaluation__figures">
            Every figure: {figures(census, knobs).map((figure, n) => (
              <Fragment key={figure.key}>{n > 0 && ', '}<span data-ref={ref(`census/${figure.key}`)}>{grouped(figure.value)} {figure.label.toLowerCase()}</span></Fragment>
            ))}.
          </p>
        )}
        {added.length > 0 && (
          <>
            <h4 className="section__title" id="added-title">Added by your knobs</h4>
            <ul className="highlights" aria-labelledby="added-title">
              {added.slice(0, LISTED).map((item, n) => (
                <li data-ref={item.ref ?? undefined} key={n}><span className="pill" data-tone="good">added</span> {item.label}</li>
              ))}
              {added.length > LISTED && <li>and {added.length - LISTED} more</li>}
            </ul>
          </>
        )}
        {removed.length > 0 && (
          <>
            <h4 className="section__title" id="removed-title">Left out by your knobs</h4>
            <ul className="highlights" aria-labelledby="removed-title">
              {removed.slice(0, LISTED).map((item, n) => <li key={n}><span className="pill">left out</span> {item.label}</li>)}
              {removed.length > LISTED && <li>and {removed.length - LISTED} more</li>}
            </ul>
          </>
        )}
        {evaluation.problems.length > 0 && <h4 className="section__title">Problems</h4>}
        {evaluation.problems.map((problem, n) => (
          <p className="problem" data-severity={problem.severity} data-ref={problem.ref ?? ref(`problems/${n}`)} key={n}>
            {problem.text}
          </p>
        ))}
        {evaluation.notes.map((note, n) => <p className="muted" key={n}>{note.text}</p>)}
      </section>
    </>
  );
}
