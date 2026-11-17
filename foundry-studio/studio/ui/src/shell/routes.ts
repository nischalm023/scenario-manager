// Spec 03 section 3.2, the route table: one route per view, `parseRoute` from an address, `routeFor` back to the
// canonical address (the location always pinned, defaults left out, parameters in the table's order, values written
// by `URLSearchParams`), `canonical` for the rewrite on load, whether a change of route pushes or replaces the
// history entry, and which conversation a route's chat column shows (ruling D94). Two addresses are the same view
// exactly when their canonical forms are equal.

import type { FlowLayer, Panel } from '../protocol/panel.ts';
import {
  GrammarError, checkKinBeside, parseKinPins, parseLocation, parseRef, parseSitePath, parseUlid, printKinPins,
  printLocation, printRef, siteRootOf, templateRootOf, type KinPin, type Location,
} from './grammar.ts';

/** What a world route shows, by our namespace (SPEC.md section 7.2): WPS knows no kind of scope. */
export type WorldView =
  | 'map'            // a site's map scope, or its root scope at a changeset until the API resolves the cut
  | 'flow'
  | 'fleet'          // no panel in the PoC: its agent overrides show on the Scenario panel
  | 'site-template'  // a site's root with `panel=template`: made from, update (spec 05)
  | 'agents'         // a site's root with `panel=agents`: its fleets, read from the flow and what made the site
  | 'stations'       // a site's root with `panel=stations`: its station groups and their work, read from the flow
  | 'orders'         // a site's root with `panel=orders`: its inventory and order profile, set in its scenarios
  | 'template'       // a site template's root scope (spec 05)
  | 'history'
  | 'none';          // any other scope: no panel shows it in the PoC

export type Route =
  | { view: 'home'; foundryProject?: string; foundrySolution?: string }
  | { view: 'settings' }
  | { view: 'docs'; doc: string }  // a document by its repository path without `.md`; empty for the list
  | { view: 'templates'; highlights?: number }
  | { view: 'template-draft'; draft: string; n?: number; highlights?: number }
  | {
      view: 'world'; location: Location; show: WorldView; kin?: KinPin[]; layers?: FlowLayer;
      draft?: string; select?: string; highlights?: number;
    }
  | { view: 'scenarios'; site?: string; highlights?: number }
  | { view: 'scenario'; id: string; version?: number }
  | { view: 'runs'; site?: string; filter?: 'site' | 'mine'; submission?: string; highlights?: number }
  | { view: 'run'; id: string; attempt?: number; simMs?: number; select?: string; highlights?: number }
  | { view: 'compare'; ids: string[]; reference?: string }
  | { view: 'unreadable'; address: string; why: string };

/** The tab row's panels; `null` for a view no tab names (Home, History, a scope no panel shows). */
export function panelOf(route: Route): Panel | null {
  switch (route.view) {
    case 'templates': case 'template-draft': return 'template';
    case 'scenarios': case 'scenario': return 'scenario';
    case 'runs': case 'run': case 'compare': return 'runs';
    case 'world':
      if (route.show === 'map' || route.show === 'flow') return route.show;
      return route.show === 'site-template' || route.show === 'template' ? 'template' : null;
    default: return null;
  }
}

class Unreadable extends Error {}

function unreadable(why: string): never {
  throw new Unreadable(why);
}

function integer(text: string, least: number, name: string): number {
  if (!/^(0|[1-9][0-9]*)$/.test(text) || !Number.isSafeInteger(Number(text)) || Number(text) < least) {
    unreadable(`\`${name}\` is a whole number of ${least} or more, written without leading zeros`);
  }
  return Number(text);
}

function grammar<T>(read: () => T): T {
  try {
    return read();
  } catch (fault) {
    if (fault instanceof GrammarError) unreadable(fault.message);
    throw fault;
  }
}

function worldRoute(path: string, query: URLSearchParams): Route {
  let location = grammar(() => parseLocation(path));
  const panel = query.get('panel');
  const SETUP = ['agents', 'stations', 'orders'] as const;
  type Setup = (typeof SETUP)[number];
  const setup = SETUP.find((one) => one === panel) ?? null;
  if (panel !== null && panel !== 'template' && panel !== 'history' && setup === null) unreadable('`panel` is `template`, `history`, `agents`, `stations` or `orders`');
  const site = siteRootOf(location.scope);
  const template = templateRootOf(location.scope);
  let show: WorldView;
  if (panel === 'history') {
    show = 'history';
  } else if (site !== null) {
    const below = location.scope.slice(site.length + 1);
    if (panel === 'template') {
      if (below !== '') unreadable("`panel=template` names a site's root scope");
      show = 'site-template';
    } else if (setup !== null) {
      if (below !== '') unreadable(`\`panel=${setup}\` names a site's root scope`);
      show = setup as Setup;
    } else if (below === '') {
      // A branch or tag carries over to the map (rule 1); a changeset names the root's own sequence, so the map's
      // changeset waits for the API to resolve the root's cut (spec 04's `resolve_cut`).
      if (location.pin.kind !== 'changeset') location = { scope: `${site}/map`, pin: location.pin };
      show = 'map';
    } else {
      show = below === 'map' ? 'map' : below === 'flow' ? 'flow' : below === 'fleet' ? 'fleet' : 'none';
    }
  } else if (template !== null) {
    if (panel === 'template') unreadable("`panel=template` names a site's root scope");
    // A template's sub-scope opens its root (ruling D63); at a changeset the root's cut is the API's to resolve.
    if (location.scope !== template && location.pin.kind !== 'changeset') location = { scope: template, pin: location.pin };
    show = 'template';
  } else {
    if (panel === 'template') unreadable("`panel=template` names a site's root scope");
    show = 'none';
  }
  const route: Extract<Route, { view: 'world' }> = { view: 'world', location, show };
  const kin = query.get('kin');
  if (kin !== null) {
    const pins = grammar(() => parseKinPins(kin));
    const root = site ?? template;
    if (root === null) unreadable('`kin` holds kin scopes of a site or a template');
    grammar(() => checkKinBeside(pins, location, root));
    route.kin = pins;
  }
  const layers = query.get('layers');
  if (layers !== null && show === 'flow') {
    const named = layers.split(',');
    if (!named.every((layer) => layer === 'material' || layer === 'agent') || new Set(named).size !== named.length) {
      unreadable('`layers` is `material`, `agent` or both');
    }
    if (named.length === 1) route.layers = named[0] as FlowLayer;
  }
  const draft = query.get('draft');
  if (draft !== null && show === 'flow') route.draft = grammar(() => parseUlid(draft));
  const select = query.get('select');
  if (select !== null) route.select = selection(select);
  const highlights = query.get('highlights');
  if (highlights !== null) route.highlights = integer(highlights, 1, 'highlights');
  return route;
}

/** `select`: an entity reference or an object key, as its canonical text. */
function selection(text: string): string {
  const ref = grammar(() => parseRef(text));
  if (ref.kind === 'scope') unreadable('`select` names an entity or an object, never a scope');
  return printRef(ref);
}

function readRoute(path: string, query: URLSearchParams): Route {
  const highlights = (): { highlights?: number } => {
    const text = query.get('highlights');
    return text === null ? {} : { highlights: integer(text, 1, 'highlights') };
  };
  if (path === '/') {
    const project = query.get('foundry_project');
    const solution = query.get('foundry_solution');
    return { view: 'home', ...(project ? { foundryProject: project } : {}), ...(solution ? { foundrySolution: solution } : {}) };
  }
  if (path === '/settings') return { view: 'settings' };
  if (path === '/docs' || path.startsWith('/docs/')) {
    try {
      return { view: 'docs', doc: path.slice('/docs/'.length).split('/').map(decodeURIComponent).join('/') };
    } catch {
      return unreadable('the document is not percent-encoded text');
    }
  }
  if (path === '/templates') return { view: 'templates', ...highlights() };
  if (path.startsWith('/w/')) {
    let location: string;
    try {
      location = decodeURIComponent(path.slice(3));
    } catch {
      return unreadable('the location is not percent-encoded text');
    }
    return worldRoute(location, query);
  }
  if (path === '/scenarios') {
    const site = query.get('site');
    return { view: 'scenarios', ...(site !== null ? { site: grammar(() => parseSitePath(site)) } : {}), ...highlights() };
  }
  if (path === '/runs') {
    const site = query.get('site');
    const filter = query.get('filter');
    if (filter !== null && filter !== 'site' && filter !== 'mine') unreadable('`filter` is `site` or `mine`');
    const submission = query.get('submission');
    const route: Extract<Route, { view: 'runs' }> = { view: 'runs', ...highlights() };
    if (site !== null) route.site = grammar(() => parseSitePath(site));
    const byDefault = route.site !== undefined ? 'site' : 'mine';
    if (filter !== null && filter !== byDefault) route.filter = filter;
    if (submission !== null) route.submission = grammar(() => parseUlid(submission));
    return route;
  }
  if (path === '/runs/compare') {
    const ids = (query.get('ids') ?? '').split(',').filter((id) => id !== '').map((id) => grammar(() => parseUlid(id)));
    if (ids.length < 2 || ids.length > 12) unreadable('`ids` names 2 to 12 runs');
    const reference = query.get('reference');
    if (reference !== null && !ids.includes(reference)) unreadable('`reference` is one of the runs `ids` names');
    return { view: 'compare', ids, ...(reference !== null && reference !== ids[0] ? { reference } : {}) };
  }
  const object = /^\/(template-drafts|scenarios|runs)\/([^/]+)$/.exec(path);
  if (object !== null) {
    const id = grammar(() => parseUlid(object[2]));
    if (object[1] === 'template-drafts') {
      const n = query.get('n');
      return { view: 'template-draft', draft: id, ...(n !== null ? { n: integer(n, 1, 'n') } : {}), ...highlights() };
    }
    if (object[1] === 'scenarios') {
      const version = query.get('version');
      return { view: 'scenario', id, ...(version !== null ? { version: integer(version, 1, 'version') } : {}) };
    }
    const route: Extract<Route, { view: 'run' }> = { view: 'run', id, ...highlights() };
    const attempt = query.get('attempt');
    if (attempt !== null) route.attempt = integer(attempt, 1, 'attempt');
    const simMs = query.get('sim_ms');
    if (simMs !== null) route.simMs = integer(simMs, 0, 'sim_ms');
    const select = query.get('select');
    if (select !== null) route.select = selection(select);
    return route;
  }
  return unreadable('no route of Agent Studio has this path');
}

/** The route an address names; an address no route reads is `unreadable`, which the panel says in words. */
export function parseRoute(address: string): Route {
  const url = new URL(address, 'http://studio.invalid');
  try {
    return readRoute(url.pathname, url.searchParams);
  } catch (fault) {
    if (fault instanceof Unreadable) return { view: 'unreadable', address, why: fault.message };
    throw fault;
  }
}

/** Spec 03 section 3.2's parameter order; a view writes only its own. */
const ORDER = [
  'foundry_project', 'foundry_solution', 'panel', 'kin', 'layers', 'draft', 'select', 'highlights', 'n', 'version',
  'site', 'filter', 'submission', 'attempt', 'sim_ms', 'ids', 'reference',
] as const;

type Params = Partial<Record<(typeof ORDER)[number], string | number | undefined>>;

function withQuery(path: string, params: Params): string {
  const query = new URLSearchParams();
  for (const name of ORDER) {
    const value = params[name];
    if (value !== undefined) query.set(name, String(value));
  }
  const text = query.toString();
  return text === '' ? path : `${path}?${text}`;
}

/** The canonical address of a route. */
export function routeFor(route: Route): string {
  switch (route.view) {
    case 'home': return withQuery('/', { foundry_project: route.foundryProject, foundry_solution: route.foundrySolution });
    case 'settings': return '/settings';
    // Each of the path's names escaped whole, so a `#`, `?` or `%` in a file's name stays part of the name.
    case 'docs': return route.doc === '' ? '/docs' : `/docs/${route.doc.split('/').map(encodeURIComponent).join('/')}`;
    case 'templates': return withQuery('/templates', { highlights: route.highlights });
    case 'template-draft': return withQuery(`/template-drafts/${route.draft}`, { n: route.n, highlights: route.highlights });
    case 'world': {
      const panel = route.show === 'history' ? 'history' : route.show === 'site-template' ? 'template'
        : route.show === 'agents' || route.show === 'stations' || route.show === 'orders' ? route.show : undefined;
      return withQuery(`/w/${printLocation(route.location)}`, {
        panel, kin: route.kin ? printKinPins(route.kin) : undefined, layers: route.layers, draft: route.draft,
        select: route.select, highlights: route.highlights,
      });
    }
    case 'scenarios': return withQuery('/scenarios', { site: route.site, highlights: route.highlights });
    case 'scenario': return withQuery(`/scenarios/${route.id}`, { version: route.version });
    case 'runs':
      return withQuery('/runs', { site: route.site, filter: route.filter, submission: route.submission, highlights: route.highlights });
    case 'run':
      return withQuery(`/runs/${route.id}`, { select: route.select, highlights: route.highlights, attempt: route.attempt, sim_ms: route.simMs });
    case 'compare': return withQuery('/runs/compare', { ids: route.ids.join(','), reference: route.reference });
    case 'unreadable': return route.address;
  }
}

/** The canonical form of an address; an unreadable one stays as written, so the panel can name it. */
export const canonical = (address: string): string => routeFor(parseRoute(address));

/** A view's identity: what changes it pushes a history entry; everything else is a change within the view. */
export function viewIdentity(route: Route): string {
  switch (route.view) {
    case 'world':
      return `world ${printLocation(route.location)} ${route.show} ${route.kin ? printKinPins(route.kin) : ''}`;
    case 'template-draft': return `template-draft ${route.draft}`;
    case 'scenario': return `scenario ${route.id} ${route.version ?? ''}`;
    case 'run': return `run ${route.id}`;
    case 'compare': return `compare ${route.ids.join(',')}`;
    case 'docs': return `docs ${route.doc}`;
    case 'unreadable': return `unreadable ${route.address}`;
    default: return route.view;
  }
}

/**
 * Spec 03 section 3.2, "History": another panel or location, `panel`, `kin`, `version` or `ids` pushes an entry;
 * `select`, `highlights`, `layers`, `draft`, `n`, `site`, `filter`, `submission`, `attempt` and `sim_ms` replace it.
 */
export function historyKind(from: Route, to: Route): 'push' | 'replace' {
  return viewIdentity(from) === viewIdentity(to) ? 'replace' : 'push';
}

const TAKES: Record<'select' | 'highlights', readonly Route['view'][]> = {
  select: ['world', 'run'],
  highlights: ['templates', 'template-draft', 'world', 'scenarios', 'runs', 'run'],
};

/** The route with `select` or `highlights` changed, as a panel command or a change within the view writes it. */
export function withParam(route: Route, name: 'select' | 'highlights', value: string | number | undefined): Route {
  if (!TAKES[name].includes(route.view)) return route;
  const next: Record<string, unknown> = { ...route };
  if (value === undefined) delete next[name];
  else next[name] = value;
  return next as Route;
}

/**
 * `sim_ms` while playback moves on its own or a slider drags: written at most once a second (wall clock) and once
 * more when it settles, since Safari refuses past about a hundred `replaceState` calls in thirty seconds.
 */
export function throttledSimMs(write: (simMs: number) => void, everyMs = 1000): (simMs: number) => void {
  let lastWrite = -Infinity;
  let pending: number | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const flush = () => {
    timer = null;
    if (pending === null) return;
    const value = pending;
    pending = null;
    lastWrite = Date.now();
    write(value);
  };
  return (simMs: number) => {
    pending = simMs;
    const wait = lastWrite + everyMs - Date.now();
    if (wait <= 0 && timer === null) flush();
    else if (timer === null) timer = setTimeout(flush, Math.max(wait, 0));
  };
}

/** Which conversation a route's chat column shows (spec 03 section 3.2, ruling D94); no route names one. */
export type ChatTarget = { kind: 'site'; site: string } | { kind: 'thread' } | null;

export function chatTargetFor(route: Route, objectSite: string | null): ChatTarget {
  switch (route.view) {
    case 'templates': case 'template-draft': return { kind: 'thread' };
    case 'world': {
      const site = siteRootOf(route.location.scope);
      if (site !== null) return { kind: 'site', site };
      return templateRootOf(route.location.scope) !== null ? { kind: 'thread' } : null;
    }
    case 'scenarios': case 'runs': return route.site !== undefined ? { kind: 'site', site: route.site } : null;
    case 'scenario': case 'run': case 'compare': return objectSite !== null ? { kind: 'site', site: objectSite } : null;
    default: return null;
  }
}
