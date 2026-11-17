// The Flow panel's words (spec 06 section 5): the compiles-to footer with and without a draft on view, what serves
// each edge, the draft banner's sentences, who carries a segment, and why a draft went stale; none counts or names an
// aisle: an assigned executor is named only as a kind.

import { describe, expect, it } from 'vitest';
import type { FlowEdge, FlowNode, FlowPlan } from '../../protocol/flow.ts';
import { bannerWords, cards, carrier, compilesTo, edgeCompilesTo, madeFromWords, staleWords } from './words.ts';

const ROOTS = ['InitializeFleet', 'FillInventory', 'LoadOrders', 'FleetConservation', 'PutawaySupplyDemand', 'PutawayInduction', 'PickDemand'];
const plan = (digest: string): FlowPlan => ({
  v: 1, compiler: '0.1.0', digest, setup: null, settings: [], roots: ROOTS.map((type) => ({ type, after: [], config: null })),
});

describe('the compiles-to footer', () => {
  it('names the seven roots, the plan and the compiler', () => {
    expect(compilesTo(plan('3f9c00e1'), 0)).toEqual([
      `Compiles to 7 root tasks: ${ROOTS.join(', ')}.`,
      'Plan 3f9c…e1 without a scenario, compiler 0.1.0.',
    ]);
  });
  it('compares a draft against its base, and says when nothing compiles', () => {
    expect(compilesTo(plan('7a1d0042'), 0, plan('3f9c00e1'))).toEqual([
      'With the draft: still 7 root tasks; the plan without a scenario changes from 3f9c…e1 to 7a1d…42.',
    ]);
    expect(compilesTo(null, 2)).toEqual(['Does not compile: 2 problems.']);
    expect(compilesTo(plan('7a1d0042'), 0, null)).toEqual([
      'With the draft: 7 root tasks; without it, the flow it was built on does not compile.',
    ]);
  });
});

describe('the draft banner', () => {
  const draft = { ops: 1, base: 1, applied: null, staleWhy: null };
  it('words each state in one sentence', () => {
    expect(bannerWords({ ...draft, state: 'ready' })).toBe('Your draft over flow changeset 1: 1 change, ready to apply.');
    expect(bannerWords({ ...draft, state: 'applying' })).toBe('Your draft over flow changeset 1: 1 change, being applied.');
    expect(bannerWords({ ...draft, state: 'applied', applied: 2 })).toBe('Your draft was applied as flow changeset 2.');
    expect(bannerWords({ ...draft, state: 'discarded' })).toBe('Your draft was discarded.');
    const why = staleWords({ reason: 'head-moved', moved: [{ scope: 'customer/demo/site/s/map', base: 1, head: 2 }] });
    expect(bannerWords({ ...draft, state: 'stale', staleWhy: why }))
      .toBe('Your draft went stale: the map moved on (changeset 1 to 2); nothing was applied.');
  });
});

describe('the provenance and what serves each edge (spec 06 section 5)', () => {
  const pick: FlowEdge = { ref: 'e', label: 'Inventory storage to Pick stations: stock tote', from: 's', to: 'p', moves: 'inventory_tote:stock', served_by: { kind: 'root', task: 'PickDemand' }, made_from_template: true };
  const back: FlowEdge = { ref: 'r', label: 'Put stations to Inventory storage: stock tote', from: 'u', to: 's', moves: 'inventory_tote:stock', served_by: { kind: 'clearance' }, made_from_template: true };
  const out: FlowEdge = { ref: 'o', label: 'Pick stations to Outbound orders: order in its bin', from: 'p', to: 'x', moves: 'order:in_bin', served_by: { kind: 'completion' }, made_from_template: true };
  const stray: FlowEdge = { ref: 'n', label: 'Inventory storage to Pick stations: empty tote', from: 's', to: 'p', moves: 'inventory_tote:empty', served_by: { kind: 'nothing' }, made_from_template: false };

  it('names the template a site was made from at its changeset', () => {
    expect(madeFromWords({ template: 'template/site/l-shape', tag: null, seq: 3 })).toBe('template/site/l-shape at changeset 3');
    expect(madeFromWords(null)).toBe('its site template');
  });
  it('names the root task that carries an edge, or what serves it without one', () => {
    expect(edgeCompilesTo(pick, null)).toBe('the PickDemand root task');
    expect(edgeCompilesTo(back, null)).toBe('no root task: each finished presentation\'s clearance returns the tote to storage');
    expect(edgeCompilesTo(out, null)).toBe('no root task: it completes with no transport');
    expect(edgeCompilesTo(stray, null)).toBe('no root task: no registered root executes it, so the flow does not compile');
  });
  it('says on a return edge\'s hover card that each finished presentation\'s clearance returns the tote', () => {
    const node = (ref: string, name: string, kind: string): FlowNode => ({ ref, name, kind, sectors: [], members: 1, ports: {}, made_from_template: true });
    const nodes = [node('u', 'Put stations', 'stations'), node('s', 'Inventory storage', 'storage')];
    expect(cards({ nodes, edges: [back], segments: [], return_rules: [] }).get('r')?.text)
      .toBe('stock tote goes from Put stations to Inventory storage. Each finished presentation\'s clearance returns it to storage.');
  });
});

describe('who carries', () => {
  it('names an assigned cohort by its kind alone, and no robot for an external segment', () => {
    expect(carrier({ kind: 'assigned', name: 'the VTMs assigned there' })).toBe('VTMs');
    expect(carrier({ kind: 'external', name: 'outside the site' })).toBe('no robot');
    expect(carrier({ kind: 'cohort', name: 'HTM fleet', ref: 'x', count: 10 })).toBe('HTM fleet');
  });
});
