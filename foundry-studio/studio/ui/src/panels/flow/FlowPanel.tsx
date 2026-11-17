// Spec 06 section 5, the Flow panel (`/w/{site}/flow@{pin}`, BACKLOG task 21): the Layers boxes and "Check the
// flows", the draft banner, then the explorer (the graph's text twin, every entity a button by `data-ref`), the stage
// (canvas, legend, compiles-to footer) and the detail pane. A detail-pane edit goes into the person's own draft, whose
// banner holds Apply and Discard. The agent's highlights are drawn once the agent loop lands (task 3).

import { useCallback, useEffect, useMemo, useState, type KeyboardEvent } from 'react';
import type { ChangesetOp, DraftRead } from '../../protocol/changeset_draft.ts';
import type { FlowParameter, FlowParameters, FlowRead } from '../../protocol/flow.ts';
import { getJson } from '../../shell/api.ts';
import { useShell } from '../../shell/context.ts';
import { printLocation, printPin, siteRootOf } from '../../shell/grammar.ts';
import { routeFor, withParam, type Route } from '../../shell/routes.ts';
import { Info, TipFact, TipItem, TipLine, TipList } from '../../shell/Info.tsx';
import { useFillWindow } from '../../shell/fillWindow.ts';
import { CardText } from './Clocked.tsx';
import { DetailPane } from './DetailPane.tsx';
import { addEdit, discard, ownDraft, pressApply } from './drafts.ts';
import { FlowCanvas } from './FlowCanvas.tsx';
import {
  bannerWords, cards as cardsOf, compilesTo, DIFF_GLYPH, drawnLayers, staleWords, type Card,
} from './words.ts';

type WorldRoute = Extract<Route, { view: 'world' }>;

interface Loaded {
  /** The flow at the location, or with an open draft named, at the draft's stored cut with the draft drawn over it. */
  read: FlowRead;
  /** Every field's value at the same cut: the draft's base column and each editor's starting point. */
  parameters: FlowParameter[];
  /** The draft the route names, or else the person's open draft on the site. */
  draft: DraftRead | null;
}
type State = { state: 'loading' } | { state: 'failed'; reason: string } | ({ state: 'ready' } & Loaded);

async function load(location: string, site: string, draft: string | undefined): Promise<State> {
  const at = draft === undefined ? '' : `?draft=${draft}`;
  const [read, parameters, drafted] = await Promise.all([
    getJson<FlowRead>(`/api/flow/${location}${at}`),
    getJson<FlowParameters>(`/api/flow-parameters/${location}${at}`),
    draft === undefined ? ownDraft(site) : getJson<DraftRead>(`/api/drafts/${draft}`),
  ]);
  if (!read.ok) return { state: 'failed', reason: read.reason };
  return {
    state: 'ready', read: read.value, parameters: parameters.ok ? parameters.value.parameters : [],
    draft: drafted.ok ? drafted.value : null,
  };
}

interface Item { ref: string; name: string; diff: string | null; problems: number }

export function FlowPanel({ route }: { route: WorldRoute }) {
  const shell = useShell();
  const location = printLocation(route.location);
  const site = siteRootOf(route.location.scope) ?? route.location.scope;
  const [version, setVersion] = useState(0);
  const [state, setState] = useState<State>({ state: 'loading' });
  const [closedCard, setClosedCard] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  useEffect(() => {
    let current = true;
    void load(location, site, route.draft).then((next) => { if (current) setState(next); });
    return () => { current = false; };
  }, [location, site, route.draft, version]);

  const layers = useMemo(() => drawnLayers(route.layers), [route.layers]);
  const ready = state.state === 'ready' ? state : null;
  const problems = useMemo(() => {
    const counted = new Map<string, number>();
    for (const problem of ready?.read.problems ?? []) if (problem.ref !== undefined) counted.set(problem.ref, (counted.get(problem.ref) ?? 0) + 1);
    return counted;
  }, [ready?.read]);
  const cards = useMemo(() => (ready === null ? new Map<string, Card>() : cardsOf(ready.read)), [ready]);
  const names = useMemo(() => new Map<string, string>(ready === null ? [] : [
    ...ready.read.nodes.map((n): [string, string] => [n.ref, n.name]), ...ready.read.edges.map((e): [string, string] => [e.ref, e.label]),
    ...ready.read.segments.map((s): [string, string] => [s.ref, s.label]), ...ready.read.return_rules.map((r): [string, string] => [r.ref, r.label]),
  ]), [ready]);
  const selected = route.select ?? null;
  const go = useCallback((next: WorldRoute) => shell.navigate(routeFor(next), 'within'), [shell]);
  const fill = useFillWindow('Process flow');
  const select = useCallback((ref: string | null, name?: string) => {
    go(withParam(route, 'select', ref ?? undefined) as WorldRoute);
    if (ref !== null && name !== undefined) shell.announce(`Selected ${name}; its details follow the canvas.`);
  }, [go, route, shell]);

  if (state.state === 'loading') return <p className="panel__status" aria-busy="true">Reading the flow graph</p>;
  if (state.state === 'failed') {
    return (
      <div className="panel__error">
        <p>The flow graph could not be read: {state.reason}</p>
        <button className="btn btn--small" type="button" onClick={() => setVersion((v) => v + 1)}>Retry</button>
      </div>
    );
  }
  const { read, parameters, draft } = state;
  const editable = route.location.pin.kind === 'branch';
  const onView = read.draft !== undefined && ['drafting', 'ready', 'applying'].includes(read.draft.state);
  const flowScope = `${site}/flow`;

  const check = async () => {
    setVersion((v) => v + 1);
    const fresh = await getJson<FlowRead>(`/api/flow/${location}`);
    if (!fresh.ok) shell.announce(`The flows could not be checked: ${fresh.reason}`);
    else shell.announce(fresh.value.problems.length === 0 ? 'The flows compile with no problem.' : `The flows have ${fresh.value.problems.length} problems.`);
  };
  const edit = async (op: ChangesetOp, summary: string) => {
    setBusy('Adding the change to your draft');
    const answer = await addEdit(site, `${site}@${printPin(route.location.pin)}`, read.seqs, op, summary);
    setBusy(null);
    if (!answer.ok && answer.problem?.type === 'head-moved') {
      setVersion((v) => v + 1);
      return shell.announce('The site moved on after this view read it, so nothing changed; it has been read again: make the change once more.');
    }
    if (!answer.ok) return shell.announce(`The change was not added to your draft: ${answer.reason}`);
    if (route.draft === answer.value.draft) setVersion((v) => v + 1);
    else go({ ...route, draft: answer.value.draft });
  };
  const apply = async (id: string) => {
    setBusy('Applying your draft');
    const answer = await pressApply(id);
    setBusy(null);
    if (!answer.ok) shell.announce(`Not applied: ${answer.problem?.type === 'validation-failed' ? 'the flow would not compile' : answer.reason}.`);
    else if (answer.value.execution === 'ok') shell.announce('Your draft was applied.');
    else if (answer.value.execution === 'failed') shell.announce(`Not applied: ${answer.value.sentence ?? 'the apply failed'}`);
    else shell.announce('Whether your draft was applied is not yet known; it is checked again until it is.');
    if (route.draft === id) setVersion((v) => v + 1);
    else go({ ...route, draft: id });
  };
  const drop = async (id: string) => {
    const answer = await discard(id);
    if (!answer.ok) shell.announce(`The draft was not discarded: ${answer.reason}`);
    setVersion((v) => v + 1);
  };

  // Escape on an item closes its hover card, then clears the selection; with neither to dismiss, it is left to what
  // holds the explorer, so a filled window returns.
  const escape = (event: KeyboardEvent<HTMLButtonElement>, ref: string) => {
    if (event.key !== 'Escape') return;
    if (closedCard !== ref && event.currentTarget.matches(':focus-visible')) setClosedCard(ref);
    else if (selected !== null) select(null);
    else return;
    event.preventDefault();
  };
  /** A row's two lines: "Pick stations to Inventory storage: tote after the pick" reads as the route, then what moves;
   * "HTM fleet: Pick stations to relay" as who carries, then where. The words stay whole for assistive tech. */
  const lines = (name: string): [string, string | null] => {
    const at = name.indexOf(': ');
    return at === -1 ? [name, null] : [name.slice(0, at), name.slice(at + 2)];
  };
  const entry = (item: Item) => {
    const [head, tail] = lines(item.name);
    return (
      <li className="flow-explorer__entry" key={item.ref}>
        <button
          className="flow-explorer__item" type="button" data-ref={item.ref} data-diff={item.diff ?? undefined}
          aria-current={selected === item.ref ? 'true' : undefined} aria-describedby={`flow-hover-${item.ref}`}
          onClick={() => select(item.ref, item.name)} onKeyDown={(event) => escape(event, item.ref)}
          onFocus={() => setClosedCard(null)} onBlur={() => setClosedCard(null)}
        >
          <span className="flow-explorer__text">
            <span className="flow-explorer__head">{head.replace(/ to /g, ' \u2192 ')}</span>
            {tail !== null && <span className="flow-explorer__tail">{tail}</span>}
            {item.diff !== null && <><span className="flow-mark" aria-hidden="true">{DIFF_GLYPH[item.diff]}</span><span className="visually-hidden">, {item.diff} in the draft</span></>}
            {item.problems > 0 && <span className="visually-hidden">, has a problem</span>}
          </span>
        </button>
        <span className="flow-hover" role="tooltip" id={`flow-hover-${item.ref}`} data-closed={closedCard === item.ref ? 'true' : undefined}><CardText card={cards.get(item.ref)} /></span>
      </li>
    );
  };
  const item = (ref: string, name: string, diff: { op: string } | undefined): Item => ({ ref, name, diff: diff?.op ?? null, problems: problems.get(ref) ?? 0 });
  const segmentsOf = (edge: string | undefined) => read.segments.filter((segment) => segment.edge === edge);
  const empties = segmentsOf(undefined);

  const draftWords = draft === null ? null : bannerWords({
    state: read.draft?.state ?? draft.state, ops: draft.ops.length, base: draft.cut[flowScope] ?? null,
    applied: draft.applied_seqs?.[flowScope] ?? null, staleWhy: staleWords(draft.stale_reason),
  });
  const draftOpen = draft !== null && (draft.state === 'drafting' || draft.state === 'ready');
  const footer = compilesTo(read.plan, read.problems.length, onView ? read.draft?.base?.plan ?? null : undefined);
  const pinned = editable ? null : `This view is pinned to ${route.location.pin.kind === 'changeset' ? `changeset ${route.location.pin.seq}` : `tag ${route.location.pin.name}`}. Open the branch head to edit.`;

  return (
    <>
      <header className="home__head page-head draft-head">
        <span className="page-head__icon" aria-hidden="true">
          <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="12" cy="12" r="3" /><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1" />
          </svg>
        </span>
        <div>
          <p className="home__title">Process flow <Info label="the drawing">
            <TipFact term="Solid line">Material flow: where product goes, labelled with what moves.</TipFact>
            <TipFact term="Dashed line">Agent flow: which robots carry it. Dotted where no robot does.</TipFact>
            <TipFact term="Shapes">▣ stations, ▤ storage, ⊕ product enters, ⊖ product leaves, ⇄ handoff place.</TipFact>
            <TipFact term="Figures">Hover a node, or focus its name in the list, to see its numbers.</TipFact>
          </Info></p>
          <p className="home__sub">Where product goes and which robots carry it.</p>
        </div>
      </header>
      <div className="panel__toolbar flow-toolbar">
        <div className="flow-views" role="tablist" aria-label="View">
          {([['material', 'Material flow'], ['agent', 'Agent flow'], ['both', 'Overlay']] as const).map(([view, words]) => {
            const on = view === 'both' ? layers.length === 2 : layers.length === 1 && layers[0] === view;
            return (
              <button
                key={view} type="button" role="tab" className="flow-view" aria-selected={on} data-view={view}
                onClick={() => go({ ...route, layers: view === 'both' ? undefined : view })}
              >
                {view !== 'both' && <span className="flow-layer__swatch" data-kind={view} aria-hidden="true" />}{words}
              </button>
            );
          })}
        </div>
        {read.plan === null
          ? <a className="pill" data-tone="bad" href="#flow-problems">Does not compile</a>
          : <span className="pill compile-pill" data-tone="good">Compiles to {read.plan.roots.length} tasks
            <Info label="the compiled tasks">
              <TipLine>The flow compiles to these root tasks of the simulation:</TipLine>
              <TipList>{read.plan.roots.map((root) => <TipItem key={root.type}><code>{root.type}</code></TipItem>)}</TipList>
            </Info></span>}
        {read.problems.length > 0 && <a className="pill" data-tone="warn" href="#flow-problems">{read.problems.length === 1 ? '1 problem' : `${read.problems.length} problems`}</a>}
        <button className="btn btn--pill btn--accent flow-toolbar__check" type="button" onClick={() => void check()}>Check the flows</button>
      </div>
      {draft !== null && draftWords !== null && (
        <div className="draft-banner">
          <p className="draft-banner__text">{draftWords}{busy !== null && ` ${busy}.`}</p>
          {draftOpen && (
            <button className="btn btn--small" type="button" aria-pressed={onView} onClick={() => go({ ...route, draft: onView ? undefined : draft.draft })}>Show the draft</button>
          )}
          {draftOpen && editable && <button className="btn btn--small btn--primary" type="button" disabled={busy !== null} onClick={() => void apply(draft.draft)}>Apply</button>}
          {draftOpen && <button className="btn btn--small" type="button" disabled={busy !== null} onClick={() => void drop(draft.draft)}>Discard</button>}
        </div>
      )}
      {read.nodes.length === 0 ? (
        <p>This site has no flow graph yet.</p>
      ) : (
        <div className="flow-layout" {...fill.region}>
          <section className="flow-explorer" aria-label="Flow explorer">
            <details className="flow-explorer__group" open>
              <summary><h3 className="label">Nodes <span className="flow-explorer__count">{read.nodes.length}</span></h3></summary>
              <ul>{read.nodes.map((node) => entry(item(node.ref, node.name, node.diff)))}</ul>
            </details>
            <details className="flow-explorer__group" open>
              <summary><h3 className="label">Material flow <span className="flow-explorer__count">{read.edges.length}</span></h3></summary>
              <ul>{read.edges.map((edge) => entry(item(edge.ref, edge.label, edge.diff)))}</ul>
            </details>
            <details className="flow-explorer__group" open>
              <summary><h3 className="label">Agent-flow segments <span className="flow-explorer__count">{read.segments.length}</span></h3></summary>
              <ul>
                {read.edges.filter((edge) => segmentsOf(edge.ref).length > 0).map((edge) => (
                  <li key={edge.ref}><span className="flow-explorer__sub">{edge.label}</span>
                    <ul>{segmentsOf(edge.ref).map((segment) => entry(item(segment.ref, segment.label, segment.diff)))}</ul>
                  </li>
                ))}
                {empties.length > 0 && (
                  <li><span className="flow-explorer__sub">Empty moves</span><ul>{empties.map((segment) => entry(item(segment.ref, segment.label, segment.diff)))}</ul></li>
                )}
              </ul>
            </details>
            <details className="flow-explorer__group" open>
              <summary><h3 className="label">Return rules <span className="flow-explorer__count">{read.return_rules.length}</span></h3></summary>
              <ul>{read.return_rules.map((rule) => entry(item(rule.ref, rule.label, rule.diff)))}</ul>
            </details>
            <details className="flow-explorer__group">
              <summary><h3 className="label">Compiles to <span className="flow-explorer__count">{read.plan?.roots.length ?? 0}</span></h3></summary>
              <ol className="flow-roots">{(read.plan?.roots ?? []).map((root) => <li key={root.type}><code>{root.type}</code></li>)}</ol>
            </details>
            <details className="flow-explorer__group" id="flow-problems" open={read.problems.length > 0}>
              <summary><h3 className="label">Problems <span className="flow-explorer__count">{read.problems.length}</span></h3></summary>
              <ul>
                {read.problems.map((problem, at) => (
                  <li key={at} className="problem" data-severity={problem.severity}>
                    <span>{problem.text}</span>
                    {problem.ref !== undefined && (
                      <button type="button" className="btn btn--quiet btn--small" onClick={() => select(problem.ref ?? null, names.get(problem.ref ?? ''))}>Select {names.get(problem.ref) ?? 'it'}</button>
                    )}
                  </li>
                ))}
                {read.problems.length === 0 && <li className="muted">None. The flows compile.</li>}
              </ul>
            </details>
          </section>
          <div className="flow-stage">
            <FlowCanvas read={read} layers={layers} selected={selected} problems={problems} cards={cards} onSelect={(ref) => select(ref, names.get(ref) ?? ref)} fill={fill} />
            <footer className="flow-legend flow-compiles visually-hidden">
              {footer.map((sentence) => <span key={sentence}>{sentence}</span>)}
              {read.plan === null && <a href="#flow-problems">The problems</a>}
            </footer>
          </div>
          <DetailPane
            read={read} selected={selected} parameters={parameters} readOnly={pinned}
            draftOver={onView ? `flow changeset ${draft?.cut[flowScope] ?? ''}` : null}
            mapRoute={(ref) => routeFor({ view: 'world', location: { scope: `${site}/map`, pin: route.location.pin }, show: 'map', select: ref })}
            onEdit={(op, summary) => void edit(op, summary)} onSelect={(ref, name) => select(ref, name)}
          />
        </div>
      )}
    </>
  );
}
