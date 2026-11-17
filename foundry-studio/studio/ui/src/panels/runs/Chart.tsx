// A results chart (spec 08 sections 3.10 and 3.12): series over a numeric axis, drawn by uPlot as crucible-ui's charts
// were (its stack's resize observer, its hooks drawing on the canvas), named by a sentence computed from its numbers,
// since a canvas says nothing to a screen reader. Colours are the tokens', read as each frame is drawn, so a change of
// theme redraws it in the new one; a series past the palette's eighth is dashed. The value axis spans the spec's range
// and is as wide as its widest label. A chart keeps one uPlot while its lines stay the same, so new values are set
// into it and a moved cursor redraws it, and only a new line or a new kind of chart builds another; the cursor is
// drawn only inside the x axis's range.
//
// The comparison's chart names each line at its end. An `interactive` chart (the run page's metric panels) has the
// hover layer instead: a crosshair that snaps to the nearest x, one tooltip listing every series there, the crosshair
// shared by every chart of one `syncKey` so the panels read at one instant, and `onHover` telling the panel which x is
// under the pointer. The tooltip enhances and never gates: every value it shows is in the panel's figures and legend
// too.

import { useEffect, useRef } from 'react';
import type uPlot from 'uplot';
import 'uplot/dist/uPlot.min.css';
import { endWords, PALETTE, paletteSlot, swatch, type ChartSpec } from './charts.ts';
import type { Cursor } from './playerCursor.ts';

const HEIGHT = 200;

const token = (name: string): string => getComputedStyle(document.documentElement).getPropertyValue(name).trim() || '#888';

const font = (): string => `11px ${getComputedStyle(document.body).fontFamily || 'sans-serif'}`;

/** A series' colour on the canvas: its palette slot's token, read as the frame is drawn; a hue by slot on a page with
 * no tokens (a unit test's). */
const seriesColour = (key: number): string => {
  const slot = paletteSlot(key);
  return getComputedStyle(document.documentElement).getPropertyValue(`--color-series-${slot}`).trim() || `hsl(${(slot * 137.508) % 360} 60% 45%)`;
};

type Plotter = typeof uPlot;

/** What makes a chart another uPlot rather than new values in the one it has: its kind, its titles and its lines. */
const shapeOf = (spec: ChartSpec): string =>
  JSON.stringify([spec.shape, spec.xTitle, spec.yTitle, spec.series.map((one) => [one.key, one.label])]);

const dataOf = (spec: ChartSpec): uPlot.AlignedData => [spec.x, ...spec.series.map((one) => one.values)] as uPlot.AlignedData;

/** The newest of what a chart is given, which the plot's hooks, axes and tooltip read as they draw. */
interface Drawn {
  spec: ChartSpec;
  cursor: Cursor | null;
  height: number;
  onHover?: (index: number | null) => void;
}

export function Chart({ name, describedBy, spec, cursor = null, interactive = false, height = HEIGHT, syncKey, onHover }: {
  name: string;
  describedBy?: string;
  spec: ChartSpec;
  /** An instant on the x axis drawn as a line, the player's cursor on a metric chart; moving it only redraws. */
  cursor?: Cursor | null;
  /** The hover layer: crosshair, tooltip, the crosshair shared with every chart of `syncKey`. */
  interactive?: boolean;
  height?: number;
  syncKey?: string;
  /** The index along `spec.x` under the pointer, null once it leaves. */
  onHover?: (index: number | null) => void;
}) {
  const holder = useRef<HTMLDivElement>(null);
  const plotted = useRef<uPlot | null>(null);
  const drawn = useRef<Drawn>({ spec, cursor, height, onHover });
  const shape = shapeOf(spec);
  useEffect(() => {
    const target = holder.current;
    if (target === null) return;
    let gone = false;
    let undo = () => {};
    // uPlot loads with the first chart: it reads the screen's pixel ratio as it loads, which a page without one (a
    // unit test's) lacks; there it draws nothing. The comparison's table states its numbers beside the chart; a
    // metric panel has only its legend's figures.
    void import('uplot')
      .then(({ default: Plot }) => {
        if (gone) return;
        const tip = interactive ? tooltip() : null;
        const options = optionsOf(Plot, drawn, Math.max(target.clientWidth, 1), interactive, syncKey, (plot) => {
          const index = plot.cursor.idx ?? null;
          if (tip !== null) placeTooltip(tip, plot, drawn.current.spec, index);
          drawn.current.onHover?.(index);
        });
        const plot = new Plot(options, dataOf(drawn.current.spec), target);
        plotted.current = plot;
        if (tip !== null) plot.over.appendChild(tip.root);
        const resized = new ResizeObserver(() => plot.setSize({ width: Math.max(target.clientWidth, 1), height: drawn.current.height }));
        resized.observe(target);
        const redraw = () => plot.redraw(false, true);
        const scheme = window.matchMedia('(prefers-color-scheme: dark)');
        scheme.addEventListener('change', redraw);
        const themed = new MutationObserver(redraw);
        themed.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
        undo = () => {
          resized.disconnect();
          themed.disconnect();
          scheme.removeEventListener('change', redraw);
          plotted.current = null;
          plot.destroy();
        };
      })
      .catch(() => undefined);
    return () => {
      gone = true;
      undo();
    };
    // A chart of the same shape takes its new values, cursor and height below, so only the shape builds another plot.
  }, [shape, interactive, syncKey]);
  useEffect(() => {
    const was = drawn.current;
    drawn.current = { spec, cursor, height, onHover };
    const plot = plotted.current;
    if (plot === null) return;
    if (was.height !== height) plot.setSize({ width: Math.max(holder.current?.clientWidth ?? 1, 1), height });
    if (was.spec !== spec) plot.setData(dataOf(spec));
    else if (was.cursor !== cursor) plot.redraw(false, false);
  }, [spec, cursor, height, onHover]);
  // A moved cursor redraws the plot it has, and renders nothing.
  useEffect(() => cursor?.subscribe(() => plotted.current?.redraw(false, false)), [cursor]);
  return <div className="chart__canvas" role="img" aria-label={name} aria-describedby={describedBy} ref={holder} style={{ minHeight: height }} />;
}

/** A vertical axis as wide as its widest label, with room for its title. */
function axisWidth(plot: uPlot, values: string[] | null): number {
  const ink = plot.ctx;
  ink.save();
  ink.font = font();
  const widest = Math.max(0, ...(values ?? []).map((value) => ink.measureText(value).width));
  ink.restore();
  return Math.ceil(widest) + 28;
}

function optionsOf(Plot: Plotter, drawn: { current: Drawn }, width: number, interactive: boolean, syncKey: string | undefined, moved: (plot: uPlot) => void): uPlot.Options {
  const spec = () => drawn.current.spec;
  const first = spec();
  // An interactive chart thins its x ticks to the width it has, a label every 64 px at least, so a compact panel's
  // axis stays readable; the others' ticks are the spec's, every one.
  const xSplits = (plot: uPlot): number[] => {
    const ticks = spec().xTicks;
    if (!interactive) return ticks.map((one) => one.at);
    const px = plot.bbox.width / Math.max(devicePixelRatio, 1);
    const every = Math.max(1, Math.ceil((ticks.length * 64) / Math.max(px, 1)));
    return ticks.filter((_, index) => index % every === 0).map((one) => one.at);
  };
  const xLabels = (splits: number[]): string[] => {
    const labelOf = new Map(spec().xTicks.map((one) => [one.at, one.label]));
    return splits.map((split) => labelOf.get(split) ?? '');
  };
  const axis = { stroke: () => token('--color-text-tertiary'), grid: { stroke: () => token('--color-grid'), width: 1 }, ticks: { show: false }, font: font(), labelFont: font() };
  const cursor: uPlot.Cursor = interactive
    ? {
        show: true,
        x: true,
        y: false,
        drag: { x: false, y: false, setScale: false },
        focus: { prox: 24 },
        points: { show: true, size: 9, width: 2, stroke: () => token('--color-bg-primary'), fill: (_plot, series) => seriesColour(spec().series[series - 1]?.key ?? 1) },
        ...(syncKey !== undefined ? { sync: { key: syncKey, setSeries: false, scales: ['x', null] } } : {}),
      }
    : { show: false };
  return {
    width,
    height: drawn.current.height,
    legend: { show: false },
    cursor,
    scales: {
      x: { time: false, range: () => spec().xRange },
      y: { range: () => spec().yRange },
    },
    axes: [
      { ...axis, label: interactive ? undefined : first.xTitle, size: interactive ? 28 : undefined, splits: xSplits, values: (_plot, splits) => xLabels(splits) },
      { ...axis, label: interactive ? undefined : first.yTitle, size: (plot, values) => axisWidth(plot, values), values: (_plot, splits) => splits.map((split) => (spec().yShort ?? spec().yWords)(split)) },
    ],
    series: [
      {},
      ...first.series.map((one) => ({
        label: one.label,
        stroke: () => seriesColour(one.key),
        width: first.shape === 'dots' ? 0 : 2,
        spanGaps: false,
        ...(one.key > PALETTE ? { dash: [6, 4] } : {}),
        points: { show: first.shape !== 'steps' && first.series.length === 1 && !interactive, size: 7, stroke: () => token('--color-bg-primary'), fill: () => seriesColour(one.key) },
        ...(first.shape === 'steps' ? { paths: Plot.paths.stepped?.({ align: 1 }) } : {}),
      })),
    ],
    hooks: {
      draw: [(plot) => drawRanges(plot, spec()), (plot) => (interactive ? undefined : drawEnds(plot, spec())), (plot) => drawCursor(plot, drawn.current.cursor?.get() ?? null)],
      ...(interactive ? { setCursor: [moved] } : {}),
    },
  };
}

/** The tooltip's elements, made once per chart and filled as the cursor moves; names and values go in as text. */
interface Tooltip {
  root: HTMLDivElement;
  heading: HTMLDivElement;
  rows: HTMLDivElement;
}

function tooltip(): Tooltip {
  const root = document.createElement('div');
  root.className = 'chart-tip';
  root.setAttribute('aria-hidden', 'true');
  root.hidden = true;
  const heading = document.createElement('div');
  heading.className = 'chart-tip__when';
  const rows = document.createElement('div');
  rows.className = 'chart-tip__rows';
  root.append(heading, rows);
  return { root, heading, rows };
}

/** The tooltip at the cursor's x: the instant, then every series' value there, the value first and strong. */
function placeTooltip(tip: Tooltip, plot: uPlot, spec: ChartSpec, index: number | null) {
  if (index === null || spec.x[index] === undefined) {
    tip.root.hidden = true;
    return;
  }
  const x = spec.x[index];
  tip.heading.textContent = spec.xWords === undefined ? String(x) : `${spec.xWords(x)} of simtime`;
  tip.rows.replaceChildren();
  const said = spec.yShort ?? spec.yWords;
  const shown = spec.series
    .map((one) => ({ one, value: one.values[index] ?? null }))
    .filter((row): row is { one: ChartSpec['series'][number]; value: number } => row.value !== null)
    .sort((a, b) => b.value - a.value);
  for (const { one, value } of shown) {
    const row = document.createElement('div');
    row.className = 'chart-tip__row';
    const key = document.createElement('span');
    key.className = 'chart-tip__key';
    key.style.background = swatch(one.key);
    const figure = document.createElement('strong');
    figure.className = 'chart-tip__value';
    figure.textContent = said(value);
    const label = document.createElement('span');
    label.className = 'chart-tip__label';
    label.textContent = one.label;
    row.append(key, figure, label);
    tip.rows.appendChild(row);
  }
  if (shown.length === 0) {
    const none = document.createElement('div');
    none.className = 'chart-tip__row';
    none.textContent = 'no value';
    tip.rows.appendChild(none);
  }
  tip.root.hidden = false;
  const left = plot.cursor.left ?? 0;
  const width = plot.over.clientWidth;
  const flip = left > width * 0.6;
  tip.root.style.left = flip ? '' : `${left + 12}px`;
  tip.root.style.right = flip ? `${width - left + 12}px` : '';
  tip.root.style.top = '8px';
}

/** The cursor, when there is one inside the x axis's range, as a vertical line across the plot; one past the range,
 * the player's at a live edge the chart's newest bucket has not reached, is not drawn. */
function drawCursor(plot: uPlot, cursor: number | null) {
  const { min, max } = plot.scales.x;
  if (cursor === null || min === undefined || max === undefined || cursor < min || cursor > max) return;
  const x = plot.valToPos(cursor, 'x', true);
  if (!Number.isFinite(x)) return;
  const ink = plot.ctx;
  const { top, height } = plot.bbox;
  ink.save();
  ink.strokeStyle = token('--color-text-secondary');
  ink.lineWidth = 1;
  ink.setLineDash([4 * devicePixelRatio, 4 * devicePixelRatio]);
  ink.beginPath();
  ink.moveTo(x, top);
  ink.lineTo(x, top + height);
  ink.stroke();
  ink.restore();
}

/** Each series' ranges as bars in its colour, two series' bars side by side at one x. */
function drawRanges(plot: uPlot, spec: ChartSpec) {
  const ink = plot.ctx;
  ink.save();
  ink.lineWidth = 2;
  const apart = spec.series.length > 1 ? 3 * devicePixelRatio : 0;
  for (const one of spec.series) {
    ink.strokeStyle = seriesColour(one.key);
    for (const range of one.ranges ?? []) {
      const x = plot.valToPos(range.at, 'x', true) + (one.key === 1 ? -apart : apart);
      ink.beginPath();
      ink.moveTo(x, plot.valToPos(range.low, 'y', true));
      ink.lineTo(x, plot.valToPos(range.high, 'y', true));
      ink.stroke();
    }
  }
  ink.restore();
}

/** Every line labelled at its end with its metric and its last value, so no reading rests on its colour alone. */
function drawEnds(plot: uPlot, spec: ChartSpec) {
  const ink = plot.ctx;
  ink.save();
  ink.font = font();
  ink.textAlign = 'right';
  spec.series.forEach((one) => {
    const last = one.values.findLastIndex((value) => value !== null);
    if (last < 0) return;
    ink.fillStyle = token('--color-text-secondary');
    const x = plot.valToPos(spec.x[last], 'x', true);
    const y = plot.valToPos(one.values[last] as number, 'y', true);
    ink.fillText(endWords(one, one.values[last] as number, spec.yWords), x, y - 8);
  });
  ink.restore();
}
