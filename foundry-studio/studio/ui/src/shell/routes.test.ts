// Spec 03 section 8, statement 2 (routes): `canonical` is idempotent; a pinless world address becomes `@main`; a
// site's root scope becomes its `/map`; `parseRoute(routeFor(x))` is `x` for every row of the route table; parameters
// come out in the table's order. Also the history rule and the `sim_ms` throttle of statement 5, and the chat column
// a route shows (ruling D94).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { canonical, chatTargetFor, historyKind, parseRoute, routeFor, throttledSimMs, type Route } from './routes.ts';

const SITE = 'customer/demo/site/lshape-6';
const RUN = '01KN0TJZANP5N0PGJD8F7A3FDB';
const RUN2 = '01KN0TK2B7C9D3E5F8G1H4J6KM';
const DRAFT = '01KT5D3QXW8M2C7RTFZ9H4BN6P';
const ENTITY = 'b6a9a763-687c-4067-909b-460e986e2229';

/** One route per row of spec 03 section 3.2's table, with its parameters set. */
const ROWS: Route[] = [
  { view: 'home', foundryProject: '652f1c2ab3', foundrySolution: '652f1c2ab4' },
  { view: 'templates', highlights: 7 },
  { view: 'world', location: { scope: 'template/site/l-shape', pin: { kind: 'tag', name: 'v3' } }, show: 'template' },
  { view: 'world', location: { scope: 'template/site/l-shape', pin: { kind: 'branch', name: 'main' } }, show: 'history' },
  { view: 'template-draft', draft: DRAFT, n: 2, highlights: 9 },
  { view: 'world', location: { scope: SITE, pin: { kind: 'branch', name: 'main' } }, show: 'site-template' },
  {
    view: 'world', location: { scope: `${SITE}/map`, pin: { kind: 'changeset', seq: 1 } }, show: 'map',
    kin: [{ scope: 'flow', seq: 2 }, { scope: 'fleet', seq: 1 }], select: ENTITY, highlights: 412,
  },
  {
    view: 'world', location: { scope: `${SITE}/flow`, pin: { kind: 'branch', name: 'main' } }, show: 'flow',
    layers: 'material', draft: DRAFT, select: ENTITY, highlights: 431,
  },
  { view: 'world', location: { scope: SITE, pin: { kind: 'branch', name: 'main' } }, show: 'history' },
  { view: 'scenarios', site: SITE, highlights: 3 },
  { view: 'scenario', id: RUN, version: 2 },
  { view: 'runs', site: SITE, filter: 'mine', submission: RUN2, highlights: 5 },
  { view: 'run', id: RUN, attempt: 2, simMs: 90000, select: `run:${RUN}`, highlights: 6 },
  { view: 'compare', ids: [RUN, RUN2], reference: RUN2 },
  { view: 'docs', doc: '' },
  { view: 'docs', doc: 'sim/dessim/docs/a page' },
  { view: 'docs', doc: 'docs/phase#2 a?b%20c' },
];

describe('spec 03 statement 2: routes', () => {
  it.each(ROWS.map((route) => [routeFor(route), route] as const))('reads %s back as the route it was written from', (address, route) => {
    expect(parseRoute(address)).toEqual(route);
  });

  it.each(ROWS.map((route) => routeFor(route)))('keeps %s canonical, idempotently', (address) => {
    expect(canonical(address)).toBe(address);
    expect(canonical(canonical(address))).toBe(address);
  });

  /** Spec 03 section 8, statement 2 (routes): a pinless world address becomes @main. */
  it('writes a pinless world address at @main', () => {
    expect(canonical(`/w/${SITE}/flow`)).toBe(`/w/${SITE}/flow@main`);
  });

  /** Spec 03 section 8, statement 2 (routes): a site's root scope becomes its /map. */
  it("opens a site's root scope at a branch or a tag on its map", () => {
    expect(canonical(`/w/${SITE}@main`)).toBe(`/w/${SITE}/map@main`);
    expect(canonical(`/w/${SITE}`)).toBe(`/w/${SITE}/map@main`);
    expect(canonical(`/w/${SITE}@tag:v3`)).toBe(`/w/${SITE}/map@tag:v3`);
  });

  /** Spec 03 section 3.2: a site root at a changeset keeps its address for the API to resolve. */
  it("keeps a site's root at a changeset for the API to resolve, since it names the root's own sequence", () => {
    expect(parseRoute(`/w/${SITE}@2`)).toMatchObject({ view: 'world', show: 'map', location: { scope: SITE } });
  });

  /** Spec 03 section 3.2: a template's sub-scope at a branch opens its root. */
  it("rewrites a template's sub-scope at a branch to the template's root", () => {
    expect(canonical('/w/template/site/l-shape/map@main')).toBe('/w/template/site/l-shape@main');
  });

  /** Spec 03 section 8, statement 2 (routes): parameters come out in the table's order. */
  it("writes parameters in the table's order with URLSearchParams, whatever order they came in", () => {
    const written = canonical(`/w/${SITE}/map@1?highlights=412&select=${ENTITY}&kin=fleet@1,flow@2`);
    expect(written).toBe(`/w/${SITE}/map@1?kin=fleet%401%2Cflow%402&select=${ENTITY}&highlights=412`);
  });

  /** Spec 03 section 3.2: an address leaves its parameters' defaults out. */
  it('leaves defaults out: both layers, the filter that follows the site, the first reference', () => {
    expect(canonical(`/w/${SITE}/flow@main?layers=material,agent`)).toBe(`/w/${SITE}/flow@main`);
    expect(canonical(`/runs?filter=site&site=${SITE}`)).toBe(`/runs?site=${encodeURIComponent(SITE)}`);
    expect(canonical('/runs?filter=mine')).toBe('/runs');
    expect(canonical(`/runs/compare?ids=${RUN},${RUN2}&reference=${RUN}`)).toBe(`/runs/compare?ids=${RUN}%2C${RUN2}`);
  });

  /** Spec 03 section 3.2: a view keeps only the parameters it takes. */
  it('drops a parameter the view does not take', () => {
    expect(canonical('/templates?zoom=3')).toBe('/templates');
    expect(canonical(`/w/${SITE}/map@main?layers=agent`)).toBe(`/w/${SITE}/map@main`);
  });

  /** Spec 03 section 3.2 and section 3.8: an address off the grammar reads as unreadable. */
  it('reads an address off the grammar as unreadable, which the panel says in words', () => {
    expect(parseRoute(`/w/${SITE}/map@head`).view).toBe('unreadable');
    expect(parseRoute(`/w/${SITE}/map@main?kin=flow@2`).view).toBe('unreadable');
    expect(parseRoute('/nowhere').view).toBe('unreadable');
    expect(canonical('/nowhere?x=1')).toBe('/nowhere?x=1');
  });
});

describe('spec 03 section 3.2, history: which changes push an entry', () => {
  const map = parseRoute(`/w/${SITE}/map@main`);
  /** Spec 03 section 3.2, the history rule: what pushes an entry. */
  it('pushes for another panel, location, panel parameter, kin, version or ids', () => {
    expect(historyKind(map, parseRoute(`/w/${SITE}/flow@main`))).toBe('push');
    expect(historyKind(map, parseRoute(`/w/${SITE}/map@1`))).toBe('push');
    expect(historyKind(map, parseRoute(`/w/${SITE}@main?panel=history`))).toBe('push');
    expect(historyKind(parseRoute(`/w/${SITE}/map@1`), parseRoute(`/w/${SITE}/map@1?kin=flow@2`))).toBe('push');
    expect(historyKind(parseRoute(`/scenarios/${RUN}`), parseRoute(`/scenarios/${RUN}?version=2`))).toBe('push');
  });

  /** Spec 03 section 3.2, the history rule: what replaces the entry. */
  it('replaces for select, highlights, layers, draft, attempt and sim_ms', () => {
    expect(historyKind(map, parseRoute(`/w/${SITE}/map@main?select=${ENTITY}`))).toBe('replace');
    expect(historyKind(map, parseRoute(`/w/${SITE}/map@main?highlights=4`))).toBe('replace');
    expect(historyKind(parseRoute(`/w/${SITE}/flow@main`), parseRoute(`/w/${SITE}/flow@main?layers=agent&draft=${DRAFT}`))).toBe('replace');
    expect(historyKind(parseRoute(`/runs/${RUN}`), parseRoute(`/runs/${RUN}?attempt=2&sim_ms=5`))).toBe('replace');
  });
});

describe('spec 03 statement 5, in part: sim_ms is written at most once a second, and once more when it settles', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  /** Spec 03 section 8, statement 5 (history), in part: sim_ms is written at most once a second. */
  it('writes the first value, holds the rest of the second, then writes where it settled', () => {
    const written: number[] = [];
    const write = throttledSimMs((simMs) => written.push(simMs));
    for (let simMs = 0; simMs <= 2400; simMs += 100) {
      write(simMs);
      vi.advanceTimersByTime(100);
    }
    vi.advanceTimersByTime(2000);
    expect(written.length).toBeLessThanOrEqual(4);
    expect(written[0]).toBe(0);
    expect(written.at(-1)).toBe(2400);
  });
});

describe("spec 03 section 3.2, the chat column of a route (ruling D94)", () => {
  /** Spec 03 section 3.2, the chat column of a route (ruling D94). */
  it('shows a site route its live conversation, the template routes the new-site thread, Home none', () => {
    expect(chatTargetFor(parseRoute(`/w/${SITE}/flow@main`), null)).toEqual({ kind: 'site', site: SITE });
    expect(chatTargetFor(parseRoute('/templates'), null)).toEqual({ kind: 'thread' });
    expect(chatTargetFor(parseRoute('/'), null)).toBeNull();
    expect(chatTargetFor(parseRoute('/runs'), null)).toBeNull();
    expect(chatTargetFor(parseRoute(`/runs?site=${SITE}`), null)).toEqual({ kind: 'site', site: SITE });
  });
});
