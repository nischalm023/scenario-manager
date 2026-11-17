// Spec 06 section 5's canvas, drawn for the eye while the explorer is its keyboard path: React Flow 12 laid out by
// dagre, the nodes always and each layer while its box is checked (ruling D103), sparse nodes whose figures open in a
// hover card (D104), schematic labels (D105). The container is one Tab stop, a group where arrow keys pan and + and -
// zoom; no node or edge takes focus, and `disableKeyboardA11y` keeps React Flow's own live region out of the page.
// A node is named by its content (name, kind words, short line, status), so its visible label stays in its name. An
// edge's hover card opens beside its label while the pointer is on the edge, its label or the card, and Escape closes
// it, marked handled so a filled window stays; the labels stand in React Flow's label layer, above every edge's reach,
// so a label never opens a neighbour's card.

import { createContext, useContext, useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import {
  applyNodeChanges, EdgeLabelRenderer, getSmoothStepPath, Handle, MarkerType, Position, ReactFlow, ReactFlowProvider, useReactFlow,
  type Edge, type EdgeProps, type Node, type NodeChange, type NodeProps,
} from '@xyflow/react';
import '@xyflow/react/dist/base.css';
import type { FlowEdge, FlowRead } from '../../protocol/flow.ts';
import { FILL_NAME, fillGlyph, fillTitle, type FillWindow } from '../../shell/fillWindow.ts';
import { CardText } from './Clocked.tsx';
import { layout, NODE_HEIGHT, NODE_WIDTH, PLACE_HEIGHT, placeId } from './layout.ts';
import {
  DIFF_GLYPH, HANDOFF, kindOf, markupKind, movesWords, segmentLabel, shortLine, statusWords, type Card, type Layer,
} from './words.ts';

type NodeData = { name: string; glyph: string; kind: string; line: string | null; mark: string | null; status: string; card: Card | undefined };
type EdgeData = {
  ref: string; layer: Layer; label: string | null; lane: number; diff: string | null; current: boolean; external: boolean;
  card: Card | undefined;
};

/** How long a card stays once the pointer leaves its edge, so the pointer can cross onto it (WCAG 1.4.13). */
const LINGER_MS = 250;

/** What a click on an edge's label selects, as a click on the edge does. */
const SelectEdge = createContext<(ref: string) => void>(() => undefined);

/** React Flow's types name no data attribute, which its wrapper elements take all the same. */
const attributes = (record: Record<string, string | undefined>) => record as unknown as Node['domAttributes'];

const SAY_LISTED = 'Listed in the explorer, where Enter selects it.';
const LABELS = {
  'node.a11yDescription.default': SAY_LISTED,
  'node.a11yDescription.keyboardDisabled': SAY_LISTED,
  'edge.a11yDescription.default': SAY_LISTED,
};

function FlowNodeView({ data }: NodeProps<Node<NodeData, 'flow'>>) {
  return (
    <>
      <Handle type="target" position={Position.Left} className="flow-handle" isConnectable={false} />
      <Handle type="target" position={Position.Top} id="top" className="flow-handle" isConnectable={false} />
      <Handle type="source" position={Position.Bottom} id="bottom" className="flow-handle" isConnectable={false} />
      <Handle type="target" position={Position.Bottom} id="bottom-in" className="flow-handle" isConnectable={false} />
      <Handle type="source" position={Position.Top} id="top-out" className="flow-handle" isConnectable={false} />
      <span className="flow-node__name">
        <span className="flow-node__glyph" aria-hidden="true">{data.glyph} </span>
        {data.name}<span className="visually-hidden">, {data.kind}</span>
        {data.mark !== null && <span className="flow-mark" aria-hidden="true"> {data.mark}</span>}
      </span>
      {data.line !== null && <span className="flow-node__resolves"><span className="visually-hidden">, </span>{data.line}</span>}
      {data.status !== '' && <span className="visually-hidden">{data.status}</span>}
      <span className="flow-hover"><CardText card={data.card} /></span>
      <Handle type="source" position={Position.Right} className="flow-handle" isConnectable={false} />
    </>
  );
}

/** A link as right-angled steps, the way a flowchart draws them; its lane shifts the run so parallel links between
 * two places stay apart, and the label sits at the middle of the path. */
function FlowEdgeView({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, data, markerEnd }: EdgeProps<Edge<EdgeData, 'flow'>>) {
  const [open, setOpen] = useState(false);
  const leaving = useRef<number | undefined>(undefined);
  const select = useContext(SelectEdge);
  const cardShown = open && data?.card !== undefined;
  useEffect(() => {
    if (!cardShown) return undefined;
    const escape = (event: { key: string; preventDefault(): void }) => {
      if (event.key !== 'Escape') return;
      event.preventDefault();
      setOpen(false);
    };
    document.addEventListener('keydown', escape);
    return () => document.removeEventListener('keydown', escape);
  }, [cardShown]);
  if (data === undefined) return null;
  const enter = () => { window.clearTimeout(leaving.current); setOpen(true); };
  const leave = () => { leaving.current = window.setTimeout(() => setOpen(false), LINGER_MS); };
  const vertical = sourcePosition === Position.Top || sourcePosition === Position.Bottom;
  const shift = data.lane * 22;
  const [sx, sy, tx, ty] = vertical ? [sourceX + shift, sourceY, targetX + shift, targetY] : [sourceX, sourceY + shift, targetX, targetY + shift];
  const [d, lx, ly] = getSmoothStepPath({
    sourceX: sx, sourceY: sy, targetX: tx, targetY: ty, sourcePosition, targetPosition, borderRadius: 10, offset: 28 + Math.abs(data.lane) * 14,
  });
  const diff = data.diff ?? undefined;
  return (
    <>
      <g data-ref={data.ref} data-layer={data.layer} aria-current={data.current ? 'true' : undefined} onPointerEnter={enter} onPointerLeave={leave}>
        <path className="flow-edge__reach" d={d} />
        <path
          className="flow-edge" data-kind={data.layer} data-layer={data.layer} data-diff={diff}
          data-executor={data.external ? 'external' : undefined} d={d} markerEnd={markerEnd}
        />
      </g>
      {data.label !== null && (
        <EdgeLabelRenderer>
          <svg className="flow-edge__label-layer" aria-hidden="true" style={{ transform: `translate(${lx}px, ${ly - 4}px)` }}>
            <text
              className="flow-edge__label" data-layer={data.layer} data-diff={diff} textAnchor="middle"
              onPointerEnter={enter} onPointerLeave={leave} onClick={() => select(data.ref)}
            >
              {data.label}{diff !== undefined ? `, ${diff}` : ''}
            </text>
          </svg>
        </EdgeLabelRenderer>
      )}
      {cardShown && (
        <EdgeLabelRenderer>
          <span
            className="flow-hover flow-hover--edge" role="tooltip" data-ref={data.ref} style={{ transform: `translate(${lx}px, ${ly + 8}px)` }}
            onPointerEnter={enter} onPointerLeave={leave}
          >
            <CardText card={data.card} />
          </span>
        </EdgeLabelRenderer>
      )}
    </>
  );
}

const NODE_TYPES = { flow: FlowNodeView };
const EDGE_TYPES = { flow: FlowEdgeView };

export interface CanvasProps {
  read: FlowRead;
  layers: Layer[];
  selected: string | null;
  problems: Map<string, number>;
  cards: Map<string, Card>;
  onSelect(ref: string): void;
  /** The page fills the window with the drawing and the lists beside it. */
  fill: FillWindow;
}

/**
 * Each link's lane among the links between the same two places, centred on zero, and whether a label fits. A lane
 * bows a link to one side of its own direction, so a link drawn the other way takes the opposite lane: opposite links
 * between two places then bow apart instead of onto one curve with their labels on one spot.
 */
function lanes(links: { id: string; ends: [string, string] }[]): Map<string, { lane: number; crowded: boolean }> {
  const groups = new Map<string, { id: string; against: boolean }[]>();
  for (const link of links) {
    const key = [...link.ends].sort().join('|');
    groups.set(key, [...(groups.get(key) ?? []), { id: link.id, against: link.ends[0] > link.ends[1] }]);
  }
  const placed = new Map<string, { lane: number; crowded: boolean }>();
  for (const group of groups.values()) {
    group.forEach(({ id, against }, at) => {
      const lane = at - (group.length - 1) / 2;
      placed.set(id, { lane: against ? -lane : lane, crowded: group.length > 2 });
    });
  }
  return placed;
}

function elements({ read, layers, selected, problems, cards }: Omit<CanvasProps, 'onSelect' | 'fill'>): { nodes: Node[]; edges: Edge[] } {
  const byRef = new Map(read.nodes.map((node) => [node.ref, node]));
  const edgeOf = new Map<string, FlowEdge>(read.edges.map((edge) => [edge.ref, edge]));
  const agent = layers.includes('agent');
  const nodes: Node[] = [];
  for (const placed of layout(read)) {
    const position = { x: placed.x, y: placed.y };
    if (placed.handoff !== undefined) {
      if (!agent) continue;
      const owner = byRef.get(placed.handoff.node);
      const name = `${owner?.name ?? placed.handoff.node}: ${placed.handoff.port} stands`;
      nodes.push({
        id: placed.id, type: 'flow', position, className: 'flow-node flow-node--place', ariaRole: 'button', width: NODE_WIDTH, height: PLACE_HEIGHT, selectable: false,
        data: { name, glyph: HANDOFF.glyph, kind: HANDOFF.words, line: null, mark: null, status: '', card: { text: `Where robots hand totes over at ${owner?.name ?? 'the node'}.`, simtime: null } },
        domAttributes: attributes({ 'data-ref': placed.handoff.node, 'data-port': placed.handoff.port, 'data-kind': 'handoff', 'data-layer': 'agent' }),
      });
      continue;
    }
    const node = byRef.get(placed.id);
    if (node === undefined) continue;
    const count = problems.get(node.ref) ?? 0;
    const line = shortLine(node, count);
    const diff = node.diff?.op ?? null;
    const kind = kindOf(node);
    nodes.push({
      id: node.ref, type: 'flow', position, className: 'flow-node', ariaRole: 'button', width: NODE_WIDTH, height: NODE_HEIGHT,
      data: {
        name: node.name, glyph: kind.glyph, kind: kind.words, line, mark: diff === null ? null : DIFF_GLYPH[diff] ?? null,
        status: statusWords({ changed: diff !== null, problems: count }), card: cards.get(node.ref),
      },
      domAttributes: attributes({
        'data-ref': node.ref, 'data-kind': markupKind(node), 'data-diff': diff ?? undefined,
        'aria-current': selected === node.ref ? 'true' : undefined,
      }),
    });
  }
  const links: { id: string; ends: [string, string] }[] = [];
  if (layers.includes('material')) for (const edge of read.edges) links.push({ id: edge.ref, ends: [edge.from, edge.to] });
  if (agent) for (const segment of read.segments) links.push({ id: segment.ref, ends: [placeId(segment.from), placeId(segment.to)] });
  const laid = lanes(links);
  const edges: Edge[] = [];
  const lane = (id: string) => laid.get(id) ?? { lane: 0, crowded: false };
  if (layers.includes('material')) {
    for (const edge of read.edges) {
      const label = movesWords(edge) + (edge.alternative_to ? ', an alternative' : '');
      edges.push({
        id: edge.ref, source: edge.from, target: edge.to, type: 'flow', ariaLabel: `Material flow: ${edge.label}${edge.diff ? `, ${edge.diff.op}` : ''}`,
        markerEnd: { type: MarkerType.ArrowClosed, width: 16, height: 16 },
        data: {
          ref: edge.ref, layer: 'material', label, lane: lane(edge.ref).lane, diff: edge.diff?.op ?? null, current: selected === edge.ref,
          external: false, card: cards.get(edge.ref),
        },
      });
    }
  }
  if (agent) {
    for (const segment of read.segments) {
      const at = lane(segment.ref);
      const words = segmentLabel(segment, segment.edge === undefined ? undefined : edgeOf.get(segment.edge));
      // A leg between a node and its own handoff place runs straight down or up between them.
      const [source, target] = [placeId(segment.from), placeId(segment.to)];
      const within = segment.from.node === segment.to.node && source !== target;
      const downward = within && source === segment.from.node;
      edges.push({
        id: segment.ref, source, target, type: 'flow',
        ...(within ? { sourceHandle: downward ? 'bottom' : 'top-out', targetHandle: downward ? 'top' : 'bottom-in' } : {}),
        ariaLabel: `Agent flow: ${segment.label}${segment.diff ? `, ${segment.diff.op}` : ''}`,
        data: {
          ref: segment.ref, layer: 'agent', label: at.crowded ? null : words, lane: at.lane, diff: segment.diff?.op ?? null,
          current: selected === segment.ref, external: segment.executor.kind === 'external', card: cards.get(segment.ref),
        },
      });
    }
  }
  return { nodes, edges };
}

function Canvas(props: CanvasProps) {
  const flow = useReactFlow();
  const { read, layers, selected, problems, cards } = props;
  const laid = useMemo(() => elements({ read, layers, selected, problems, cards }), [read, layers, selected, problems, cards]);
  // Where the person dragged a node to, by id: kept over re-reads and layer changes, dropped by Arrange. The nodes
  // React Flow draws are its own objects (they carry what it measured), rebuilt from the layout with those positions.
  const [moved, setMoved] = useState<Map<string, { x: number; y: number }>>(new Map());
  const measured = useRef(new Map<string, { width?: number; height?: number }>());
  const [sized, setSized] = useState(0);
  const nodes = useMemo(() => laid.nodes.map((node) => {
    const size = measured.current.get(node.id);
    const at = moved.get(node.id);
    return { ...node, ...(size !== undefined ? { measured: size } : {}), position: at ?? node.position };
    // `sized` counts the measurements, which live in the ref.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }), [laid, moved, sized]);
  const edges = laid.edges;
  // A dragged node takes its handoff places with it, so a robot leg to one stays a short drop.
  const onNodesChange = (changes: NodeChange[]) => {
    const next = applyNodeChanges(changes, nodes);
    let remeasured = false;
    for (const change of changes) {
      if (change.type !== 'dimensions') continue;
      const node = next.find((one) => one.id === change.id);
      if (node?.measured !== undefined) {
        measured.current.set(change.id, node.measured);
        remeasured = true;
      }
    }
    if (remeasured) setSized((n) => n + 1);
    setMoved((was) => {
      const kept = new Map(was);
      for (const change of changes) {
        if (change.type !== 'position' || change.position === undefined) continue;
        kept.set(change.id, change.position);
        const home = laid.nodes.find((node) => node.id === change.id);
        if (home === undefined) continue;
        const [dx, dy] = [change.position.x - home.position.x, change.position.y - home.position.y];
        for (const place of laid.nodes) {
          if (place.id.startsWith(`${change.id}#`)) kept.set(place.id, { x: place.position.x + dx, y: place.position.y + dy });
        }
      }
      return kept;
    });
  };
  const arrange = () => {
    setMoved(new Map());
    requestAnimationFrame(() => void flow.fitView({ padding: 0.12, maxZoom: 1.6, duration: 300 }));
  };
  // The drawing fills its box: fitted again whenever the elements or the box change, once the nodes are measured.
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const fit = () => void flow.fitView({ padding: 0.12, maxZoom: 1.6, duration: 0 });
    const soon = requestAnimationFrame(() => requestAnimationFrame(fit));
    const watcher = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(fit);
    if (watcher !== null && box.current !== null) watcher.observe(box.current);
    return () => {
      cancelAnimationFrame(soon);
      watcher?.disconnect();
    };
  }, [flow, laid, layers]);
  const shown = layers.length === 2 ? 'material and agent flow' : layers[0] === 'material' ? 'material flow only' : 'agent flow only';
  const keys = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = 48;
    const { x, y, zoom } = flow.getViewport();
    const moves: Record<string, [number, number]> = { ArrowLeft: [step, 0], ArrowRight: [-step, 0], ArrowUp: [0, step], ArrowDown: [0, -step] };
    const move = moves[event.key];
    if (move !== undefined) void flow.setViewport({ x: x + move[0], y: y + move[1], zoom });
    else if (event.key === '+' || event.key === '=') void flow.zoomIn();
    else if (event.key === '-') void flow.zoomOut();
    else return;
    event.preventDefault();
  };
  return (
    <div
      ref={box} className="flow-canvas flow-canvas--live" role="group" tabIndex={0} onKeyDown={keys}
      aria-label={`Flow graph drawing, ${shown}. Every node, edge and segment is also listed in the explorer.`}
    >
      <SelectEdge.Provider value={props.onSelect}>
        <ReactFlow
          // The wrapper's `role="application"` would put a screen reader in its forms mode inside our group.
          ref={(wrapper) => wrapper?.removeAttribute('role')}
          nodes={nodes} edges={edges} nodeTypes={NODE_TYPES} edgeTypes={EDGE_TYPES} fitView fitViewOptions={{ padding: 0.12, maxZoom: 1.6 }}
          nodesDraggable nodesConnectable={false} nodesFocusable={false} edgesFocusable={false} onNodesChange={onNodesChange}
          disableKeyboardA11y ariaLabelConfig={LABELS} proOptions={{ hideAttribution: true }}
          zoomOnScroll={false} preventScrolling={false} zoomOnDoubleClick={false} minZoom={0.3}
          onNodeClick={(_, node) => props.onSelect(String((node.domAttributes as Record<string, string> | undefined)?.['data-ref'] ?? node.id))}
          onEdgeClick={(_, edge) => props.onSelect(edge.id)}
        />
      </SelectEdge.Provider>
      <div className="flow-canvas__tools" role="toolbar" aria-label="Drawing">
        <button className="stage-tool" type="button" title="Arrange the drawing again" aria-label="Arrange the drawing again" onClick={arrange}>
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><rect x="3" y="4" width="6" height="6" rx="1" /><rect x="15" y="4" width="6" height="6" rx="1" /><rect x="9" y="14" width="6" height="6" rx="1" /><path d="M6 10v2h12v-2M12 12v2" /></svg>
        </button>
        <button className="stage-tool" type="button" title="Fit the drawing to the box" aria-label="Fit the drawing to the box" onClick={() => void flow.fitView({ padding: 0.12, maxZoom: 1.6, duration: 300 })}>
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" /></svg>
        </button>
        <button className="stage-tool" type="button" title={fillTitle(props.fill.filled)} aria-label={FILL_NAME} aria-pressed={props.fill.filled} onClick={props.fill.toggle}>
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={fillGlyph(props.fill.filled)} /></svg>
        </button>
        <span className="flow-canvas__hint">Drag a node to untangle its links</span>
      </div>
    </div>
  );
}

export function FlowCanvas(props: CanvasProps) {
  return (
    <ReactFlowProvider>
      <Canvas {...props} />
    </ReactFlowProvider>
  );
}
