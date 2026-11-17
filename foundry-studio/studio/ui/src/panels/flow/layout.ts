// Spec 06 section 5's automatic layout: dagre ranks the flow's nodes left to right in the direction product flows,
// from both layers' links, so a layer toggle never moves a node; a handoff place is then set directly beneath the
// node it belongs to, so a robot's leg to or from it reads as a short drop rather than a line across the drawing.
// Positions are worked out on every read and never stored (SPEC.md 6.3).

import dagre from '@dagrejs/dagre';
import type { FlowPlace, FlowRead } from '../../protocol/flow.ts';

export const NODE_WIDTH = 168;
export const NODE_HEIGHT = 64;
export const PLACE_HEIGHT = 36;
/** The gap between a node and the handoff place beneath it. */
const PLACE_DROP = 28;

/** The ports a handoff place stands for: a storage node's relay stands, a belt machine's dropoff and collect. */
const HANDOFF_PORTS = new Set(['relay', 'dropoff', 'collect']);

/** The canvas element a segment's end stands at: its handoff place, or the node itself. */
export const placeId = (place: FlowPlace): string =>
  HANDOFF_PORTS.has(place.port) ? `${place.node}#${place.port}` : place.node;

export interface Placed {
  id: string;
  x: number;
  y: number;
  /** For a handoff place, its node and port. */
  handoff?: { node: string; port: string };
}

/** Every node and handoff place of a read, with its top-left corner. */
export function layout(read: FlowRead): Placed[] {
  const graph = new dagre.graphlib.Graph({ multigraph: true });
  graph.setGraph({ rankdir: 'LR', nodesep: 64, ranksep: 104, marginx: 16, marginy: 16, ranker: 'network-simplex' });
  graph.setDefaultEdgeLabel(() => ({}));
  const handoffs = new Map<string, { node: string; port: string }>();
  // A node is ranked with room for its handoff place beneath it, so the places never collide with the next row.
  const owners = new Set(read.segments.flatMap((segment) => [segment.from, segment.to]).filter((place) => HANDOFF_PORTS.has(place.port)).map((place) => place.node));
  for (const node of read.nodes) graph.setNode(node.ref, { width: NODE_WIDTH, height: NODE_HEIGHT + (owners.has(node.ref) ? PLACE_DROP + PLACE_HEIGHT : 0) });
  for (const segment of read.segments) {
    for (const place of [segment.from, segment.to]) {
      const id = placeId(place);
      if (id === place.node || handoffs.has(id)) continue;
      handoffs.set(id, { node: place.node, port: place.port });
    }
  }
  for (const edge of read.edges) graph.setEdge(edge.from, edge.to, {}, edge.ref);
  // A robot's leg ranks the nodes at its ends, as product's link does; a leg to or from a handoff place ranks its owner.
  for (const segment of read.segments) {
    const [from, to] = [segment.from.node, segment.to.node];
    if (from !== to) graph.setEdge(from, to, {}, segment.ref);
  }
  dagre.layout(graph);
  const placed: Placed[] = [];
  for (const id of graph.nodes().sort()) {
    const { x, y, height } = graph.node(id);
    // The node box sits at the top of the room dagre gave it; its handoff place takes the rest.
    const top = y - height / 2;
    placed.push({ id, x: x - NODE_WIDTH / 2, y: top });
    for (const [placeKey, handoff] of handoffs) {
      if (handoff.node === id) placed.push({ id: placeKey, x: x - NODE_WIDTH / 2, y: top + NODE_HEIGHT + PLACE_DROP, handoff });
    }
  }
  return placed;
}
