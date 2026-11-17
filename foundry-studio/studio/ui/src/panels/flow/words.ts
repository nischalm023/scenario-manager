// Spec 06 section 5's words for the Flow panel: a node kind's glyph and words, who carries a segment, the short line
// and accessible names the canvas and the explorer share, the compiles-to footer, and the draft banner's sentences.
// Pure, so each is tested without a page. No sentence names one aisle or counts its robots (ruling D105).

import type { FlowEdge, FlowExecutor, FlowMadeFrom, FlowNode, FlowPlan, FlowSegment, FlowServedBy } from '../../protocol/flow.ts';
import type { DraftState } from '../../protocol/projection.ts';

export type Layer = 'material' | 'agent';

/** The layers drawn: both by default, else the one the route names (ruling D103). */
export const drawnLayers = (only: Layer | undefined): Layer[] => (only === undefined ? ['material', 'agent'] : [only]);

const KINDS: Record<string, { glyph: string; words: string }> = {
  stations: { glyph: '▣', words: 'stations' },
  storage: { glyph: '▤', words: 'storage' },
  source: { glyph: '⊕', words: 'where product enters' },
  sink: { glyph: '⊖', words: 'where product leaves' },
  conveyance: { glyph: '⟳', words: 'conveyance' },
};

export const HANDOFF = { glyph: '⇄', words: 'handoff place' };

export const kindOf = (node: FlowNode): { glyph: string; words: string } =>
  KINDS[node.kind] ?? { glyph: '•', words: node.kind };

/** The markup's node kind: a group of stations or storage, a place product enters or leaves, or a handoff place. */
export const markupKind = (node: FlowNode): 'group' | 'boundary' =>
  node.kind === 'source' || node.kind === 'sink' ? 'boundary' : 'group';

const counted = (n: number, one: string, many: string): string => `${n} ${n === 1 ? one : many}`;

/** A node's one short line: its sectors' member count, or its problem count in its place. */
export function shortLine(node: FlowNode, problems: number): string | null {
  if (problems > 0) return counted(problems, 'problem', 'problems');
  return markupKind(node) === 'boundary' ? null : counted(node.members, 'member', 'members');
}

/** What an edge moves, in its label's words: the label's part after "{from} to {to}: ". */
export const movesWords = (edge: FlowEdge): string => edge.label.split(': ').slice(1).join(': ') || edge.moves;

/** Who carries a segment, the way a label says it: "no robot", a cohort's name, or the kind of cohort assigned. */
export function carrier(executor: FlowExecutor): string {
  if (executor.kind === 'external') return 'no robot';
  const assigned = /^the (.+) assigned there$/.exec(executor.name);
  return assigned?.[1] ?? executor.name;
}

/** A segment's canvas label, "{who}: {what}", or "{who}, empty" for an empty move. */
export function segmentLabel(segment: FlowSegment, edge: FlowEdge | undefined): string {
  const who = carrier(segment.executor);
  if (segment.carries === 'nothing') return `${who}, empty`;
  return edge === undefined ? who : `${who}: ${movesWords(edge)}`;
}

/** The hidden words a status adds to an accessible name. */
export function statusWords(status: { changed: boolean; problems: number }): string {
  return (status.changed ? ', changed in the draft' : '') + (status.problems > 0 ? ', has a problem' : '');
}

export const DIFF_GLYPH: Record<string, string> = { changed: '✎', added: '+', removed: '−' };

/** "3f9c…e1": a digest shortened as the footer prints it. */
export function shortDigest(digest: string): string {
  const hex = digest.replace(/^sha256:/, '');
  return `${hex.slice(0, 4)}…${hex.slice(-2)}`;
}

/** The compiles-to footer's sentences for one read, and with a draft on view, against its cut's own plan. */
export function compilesTo(plan: FlowPlan | null, problems: number, base?: FlowPlan | null): string[] {
  if (plan === null) return [`Does not compile: ${counted(problems, 'problem', 'problems')}.`];
  const roots = `${plan.roots.length} root tasks`;
  if (base === undefined) {
    return [
      `Compiles to ${roots}: ${plan.roots.map((root) => root.type).join(', ')}.`,
      `Plan ${shortDigest(plan.digest)} without a scenario, compiler ${plan.compiler}.`,
    ];
  }
  const still = base !== null && base.roots.length === plan.roots.length ? `still ${roots}` : roots;
  const changes = base === null
    ? 'without it, the flow it was built on does not compile'
    : base.digest === plan.digest
      ? 'the plan without a scenario stays the same'
      : `the plan without a scenario changes from ${shortDigest(base.digest)} to ${shortDigest(plan.digest)}`;
  return [`With the draft: ${still}; ${changes}.`];
}

const changes = (n: number): string => counted(n, 'change', 'changes');

/** The draft banner's first sentence while the draft is open, and the one sentence once it is not. */
export function bannerWords(draft: { state: DraftState; ops: number; base: number | null; applied: number | null; staleWhy: string | null }): string {
  switch (draft.state) {
    case 'drafting': case 'ready': case 'applying': {
      const over = draft.base === null ? '' : ` over flow changeset ${draft.base}`;
      const state = { ready: 'ready to apply', drafting: 'not ready: its check found an error', applying: 'being applied' }[draft.state];
      return `Your draft${over}: ${changes(draft.ops)}, ${state}.`;
    }
    case 'applied': return draft.applied === null ? 'Your draft was applied.' : `Your draft was applied as flow changeset ${draft.applied}.`;
    case 'discarded': return 'Your draft was discarded.';
    case 'stale': return `Your draft went stale${draft.staleWhy === null ? '' : `: ${draft.staleWhy}`}; nothing was applied.`;
  }
}

/** Why a stale draft went stale, from its `stale_reason`: "the map moved on (changeset 1 to 2)". */
export function staleWords(reason: unknown): string | null {
  if (reason === null || typeof reason !== 'object') return null;
  const why = reason as { reason?: string; text?: string; moved?: { scope: string; base: number; head: number }[] };
  if (why.reason === 'head-moved' && why.moved !== undefined && why.moved.length > 0) {
    return why.moved.map((m) => `the ${m.scope.split('/').pop()} moved on (changeset ${m.base} to ${m.head})`).join(' and ');
  }
  return why.text ?? null;
}

/** Where a template-made entity's values come from: the site's newest made-from record's template at its tag or changeset. */
export function madeFromWords(made: FlowMadeFrom | null): string {
  if (made === null) return 'its site template';
  const tag = made.tag === null ? '' : `@tag:${made.tag}`;
  return `${made.template}${tag}${made.seq === null ? '' : ` at changeset ${made.seq}`}`;
}

/** What serves an edge that no root task carries, in words. */
const NO_ROOT: Record<Exclude<FlowServedBy['kind'], 'root'>, string> = {
  clearance: 'no root task: each finished presentation\'s clearance returns the tote to storage',
  completion: 'no root task: it completes with no transport',
  nothing: 'no root task: no registered root executes it, so the flow does not compile',
};

/** What a material edge compiles to: the root task carrying it and each setting the plan takes from it, or what serves it instead. */
export function edgeCompilesTo(edge: FlowEdge, plan: FlowPlan | null): string {
  const served = edge.served_by;
  if (served.kind !== 'root') return NO_ROOT[served.kind];
  const settings = (plan?.settings ?? []).filter((entry) => entry.from === edge.ref).map((entry) => entry.setting);
  return `the ${served.task} root task${settings.length === 0 ? '' : `, and the setting${settings.length === 1 ? '' : 's'} ${settings.join(', ')}`}`;
}

/** A field's value in words: numbers grouped, booleans as yes or no, anything else as its text. */
export function valueWords(value: unknown): string {
  if (typeof value === 'number') return value.toLocaleString('en-US', { maximumFractionDigits: 3 });
  if (typeof value === 'boolean') return value ? 'yes' : 'no';
  if (typeof value === 'string') return value.replaceAll('_', ' ');
  return value === null || value === undefined ? 'not stated' : JSON.stringify(value);
}

const operationOf = (node: FlowNode): string | null => {
  const process = node.process as { operation?: string } | null | undefined;
  return process?.operation ?? null;
};

/** What the node is, in words: "stations, put". */
export const nodeWhat = (node: FlowNode): string => [kindOf(node).words, operationOf(node)].filter(Boolean).join(', ');

const CUSTODY: Record<string, string> = {
  hands_over: 'hands it over', keeps_through_work: 'keeps it through the work', sets_down: 'sets it down',
  arrives: 'it arrives', leaves: 'it leaves',
};

export const custodyWords = (atEnd: string): string => CUSTODY[atEnd] ?? valueWords(atEnd);

/** A hover card: its sentences, and the simtime figure it ends with, which the card tags with its clock. */
export interface Card { text: string; simtime: string | null }

const card = (text: string): Card => ({ text, simtime: null });

/** The hover cards, one sentence or two per entity, keyed by reference. */
export function cards(read: { nodes: FlowNode[]; edges: FlowEdge[]; segments: FlowSegment[]; return_rules: { ref: string }[] }): Map<string, Card> {
  const name = new Map(read.nodes.map((node) => [node.ref, node.name]));
  const edgeOf = new Map(read.edges.map((edge) => [edge.ref, edge]));
  const out = new Map<string, Card>();
  for (const node of read.nodes) {
    const sectors = node.sectors.map((sector) => `${sector.name ?? 'a sector'}, ${counted(sector.members, 'member', 'members')}`);
    const text = [`${nodeWhat(node)}.`, sectors.length > 0 ? `Sectors: ${sectors.join('; ')}.` : ''].filter(Boolean).join(' ');
    out.set(node.ref, { text, simtime: node.all_in ? `${node.all_in.text}.` : null });
  }
  for (const edge of read.edges) {
    const steps = read.segments.filter((s) => s.edge === edge.ref).sort((a, b) => (a.step ?? 0) - (b.step ?? 0));
    const who = [...new Set(steps.map((s) => carrier(s.executor)))];
    const carried = who.length === 0 ? '' : ` Carried by ${who.join(', then ')}.`;
    const served = edge.served_by;
    const serving = served.kind === 'root' ? ` Its root task is ${served.task}.`
      : served.kind === 'clearance' ? ' Each finished presentation\'s clearance returns it to storage.' : '';
    out.set(edge.ref, card(`${movesWords(edge)} goes from ${name.get(edge.from) ?? 'a node'} to ${name.get(edge.to) ?? 'a node'}.${carried}${serving}`));
  }
  for (const segment of read.segments) {
    const edge = segment.edge === undefined ? undefined : edgeOf.get(segment.edge);
    const where = (place: { node: string; port: string }) => `${name.get(place.node) ?? 'a node'} (${valueWords(place.port)})`;
    const what = segment.carries === 'nothing' ? 'nothing' : edge === undefined ? 'its load' : movesWords(edge);
    out.set(segment.ref, card(`${carrier(segment.executor)} carries ${what} from ${where(segment.from)} to ${where(segment.to)}; at the end, ${custodyWords(segment.at_end)}.`));
  }
  const unread = card('Kept so that documents naming it still load; no run reads it. Each finished presentation\'s clearance returns its own tote to storage.');
  for (const rule of read.return_rules) out.set(rule.ref, unread);
  return out;
}
