// Spec 03 section 3.6's mount contract and ruling D119: WorldviewMount, the one component every floor view in Studio
// draws through (the Map panel, the MapTemplate panel's evaluation, the run page's playback): a floor document drawn by
// the worldview renderer, the wasm viewer of `crucible-viewer`, on a GPU canvas with its lines on a 2D canvas over it;
// or, for a run, that run's frames pushed into the same viewer from its frames socket, the floor arriving in the
// scenario frame (live frames, 26 October 2026);
// looked at orthographic, straight down, by default, or in orbit, by the projection control or O. It holds spec 03
// section 5's stage (a focusable `role="application"` named by what is drawn, described by its summary, centre and
// keys lines, every key with a button beside it, the centre cross while it has focus), `data-state` with section
// 3.6's six states, the camera kept per panel and place in `sessionStorage`, and what a canvas must follow: its size
// and pixel ratio, the theme and forced colours. Every camera move jumps (the viewer's own rule). A floor's way from its
// request to the first frame that presents it is marked `floor:asked`, `floor:read`, `floor:drawn` and `floor:painted`,
// in a measurement's session only.

import { useContext, useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';
import type { FrameAxes, FrameTag, Projection, Viewer } from 'crucible-viewer';
import { FILL_NAME, fillGlyph, fillTitle, type FillWindow } from '../shell/fillWindow.ts';
import { useFocusHome } from '../shell/focus.ts';
import { mark } from '../shell/marks.ts';
import { ServicesContext } from '../shell/panels.tsx';
import { openCanvasViewer } from './canvasViewer.ts';
import { readFloor } from './floorRead.ts';
import { onPaletteChange } from './paletteChanges.ts';
import { KEYS_LINE, keepCamera, keptCamera, moveCamera, positionWords, stageAction, type StageAction } from './stage.ts';

/** Where a mount's floor comes from: a site at a cut, a template draft's nth evaluation, or a run attempt's frames
 * socket (`frames`, the watch ticket's `frames_url`, opened with its `token`), whose scenario frame carries the
 * floor. */
export type FloorSource =
  | { kind: 'scope'; location: string; kin?: string }
  | { kind: 'evaluation'; draft: string; n: number }
  | { kind: 'run'; run: string; attempt: number; frames: string; token: string };

/** What a run source's attachment said, for the page to word: `attaching` as the mount clears the viewer for it, then
 * what its socket said (see `attachFrames` in `crucible-viewer`). */
export type StreamEvent =
  | { kind: 'attaching' }
  | { kind: 'attached'; from_ms: number; live_edge_ms: number }
  | { kind: 'frame'; tag: FrameTag }
  | { kind: 'lagged'; resume_from_ms: number }
  | { kind: 'ended'; outcome: string }
  | { kind: 'error'; message: string }
  | { kind: 'closed' };

/** One of the agent's highlight groups: its note and the keys of what it marks. */
export interface MountHighlight {
  note: string;
  by: 'agent';
  marks: string[];
}

/** What the stage says while it has no floor to draw: its source being read, nothing at this pin, or why not. */
export interface StageWords {
  state: 'loading' | 'empty' | 'failed';
  words: string;
  retry?: () => void;
}

export interface WorldviewMountProps {
  source: FloorSource;
  /** The floor document's key as the source answered it; null while it is read, or when there is none. */
  floorKey: string | null;
  waiting: StageWords | null;
  interactive: boolean;
  /** The panel keeping the camera, with the source's place: `map`, `template`. */
  panel: string;
  label: string;
  description: string;
  axes: FrameAxes | null;
  /** The one selection: its key, and the keys it marks on the floor, outlined as one union (spec 03 section 3.5). */
  selection: { key: string; marks: string[] } | null;
  highlights: MountHighlight[];
  highlightSeq: number | null;
  /** What stands under the pointer or the centre cross, the most specific first; empty off the floor. */
  onPick: (keys: string[], how: 'pointer' | 'centre') => void;
  onClearSelection: () => void;
  /** A key's label for the centre line, when the page can say one. */
  labelOf: (key: string) => Promise<string | null>;
  /** Told once a floor is drawn, by its key, so a page says it drawn only then (spec 05 section 6). */
  onDrawn?: (floorKey: string) => void;
  /** The highlights came with a `show`: frame them once the floor and their marks are both ready (spec 03 section 3.6). */
  frameOnOpen?: boolean;
  /** The projection the mount opens in when no camera is kept for its place; orthographic by default. */
  initialProjection?: Projection;
  /** Offered on the toolbar: the page opens this floor larger, for looking around in orbit. */
  onExpand?: () => void;
  /** Offered on the toolbar: the page fills the window with this floor and what it keeps beside it. */
  fill?: FillWindow;
  /** The page has a heading of its own for this floor: the mount's title names the section without ranking as one. */
  untitled?: boolean;
  /** What else describes the stage, by element id: on the run page, a stub attempt's sentence (ruling D110). */
  describedBy?: string;
  /** The mount's heading level: 3 by default; 4 under a page whose own title is the `h3`, as the run page's. */
  headingLevel?: 3 | 4;
  /** The viewer once it is open, and null as it goes, for a page that drives its cursor (the run page's playback). */
  onViewer?: (viewer: Viewer | null) => void;
  /** A run source's socket events, as they come. */
  onStream?: (event: StreamEvent) => void;
  /** A run source's Retry, when the page attaches afresh on a fresh ticket: the source's own lives a minute, so Retry
   * without it attaches again with a ticket the server may since refuse. */
  onRetry?: () => void;
}

type ViewerModule = typeof import('crucible-viewer');

/** How often, at most, the kept camera and the centre line follow a moved camera (wall clock). */
const FOLLOW_MS = 400;
/** A pointer moved less than this between press and release, in CSS pixels, is a click. */
const CLICK_PX = 4;
/** The arrows' buttons (spec 03 section 5: every key has one), each named by what it does and its key. */
const PAN_BUTTONS = [
  { glyph: 'M15 6l-6 6 6 6', name: 'Pan left (←)', right: -1, down: 0 },
  { glyph: 'M6 15l6-6 6 6', name: 'Pan up (↑)', right: 0, down: -1 },
  { glyph: 'M6 9l6 6 6-6', name: 'Pan down (↓)', right: 0, down: 1 },
  { glyph: 'M9 6l6 6-6 6', name: 'Pan right (→)', right: 1, down: 0 },
] as const;

/** A toolbar button: an icon, named for assistive tech and the tooltip by the same words the keys line uses. */
function Tool({ name, path, onClick, wide, pressed, title }: {
  name: string; path: string; onClick: () => void; wide?: boolean; pressed?: boolean; title?: string;
}) {
  return (
    <button className={`stage-tool${wide ? ' stage-tool--wide' : ''}`} type="button" aria-label={name} title={title ?? name} onClick={onClick} aria-pressed={pressed}>
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={path} /></svg>
    </button>
  );
}

const placeOf = (source: FloorSource): string => {
  switch (source.kind) {
    case 'scope': return `scope:${source.location}${source.kin ? `?kin=${source.kin}` : ''}`;
    case 'evaluation': return `evaluation:${source.draft}`;
    case 'run': return `run:${source.run}/${source.attempt}`;
  }
};

/** What the mount waits to draw: the floor by its key, or a run's frames by their socket's address. */
const wantedOf = (source: FloorSource, floorKey: string | null): string | null =>
  source.kind === 'run' ? `frames:${source.frames}` : floorKey;

/** Size the GPU canvas in device pixels from the stage's CSS size, and tell the viewer; nothing at all hides it. */
function fit(viewer: Viewer, stage: HTMLElement, gpu: HTMLCanvasElement): void {
  const ratio = window.devicePixelRatio || 1;
  const [width, height] = [Math.round(stage.clientWidth * ratio), Math.round(stage.clientHeight * ratio)];
  if (gpu.width !== width || gpu.height !== height) [gpu.width, gpu.height] = [width, height];
  viewer.resize(width, height, ratio);
}

export function WorldviewMount(props: WorldviewMountProps) {
  const { source, floorKey, waiting, interactive, label, description, axes, selection, highlights, highlightSeq, frameOnOpen, onExpand, fill, untitled } = props;
  const Heading = props.headingLevel === 4 ? 'h4' : 'h3';
  const services = useContext(ServicesContext);
  const id = useId();
  const stageRef = useRef<HTMLDivElement>(null);
  const gpuRef = useRef<HTMLCanvasElement>(null);
  const linesRef = useRef<HTMLCanvasElement>(null);
  const viewerRef = useRef<Viewer | null>(null);
  const moduleRef = useRef<ViewerModule | null>(null);
  // The floor handed to the viewer and not yet shown by a presented frame, by its key.
  const unpainted = useRef<string | null>(null);
  const [opened, setOpened] = useState<'opening' | 'open' | 'no-gpu'>('opening');
  const [drawn, setDrawn] = useState<{ key: string; failed: string | null } | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [projection, setProjection] = useState<Projection>(props.initialProjection ?? 'orthographic');
  const [centre, setCentre] = useState('Centre on: nothing yet.');
  // Spec 03 section 5: the stage's name never changes while it has focus; the heading and the description may.
  const [focusedName, setFocusedName] = useState<string | null>(null);
  const place = placeOf(source);
  const latest = useRef({ props, projection, place });
  useLayoutEffect(() => {
    latest.current = { props, projection, place };
  });

  // The viewer, opened once per mount; its frames run from requestAnimationFrame, which also keeps the camera.
  useEffect(() => {
    const [stage, gpu, lines] = [stageRef.current, gpuRef.current, linesRef.current];
    if (stage === null || gpu === null || lines === null) return;
    let alive = true;
    let frame = 0;
    let [kept, followed] = ['', 0];
    const follow = (viewer: Viewer, now: number) => {
      if (now - followed < FOLLOW_MS || !viewer.has_floor()) return;
      followed = now;
      const camera = Array.from(viewer.camera());
      const said = JSON.stringify(camera);
      if (said === kept) return;
      kept = said;
      const { props: shown, projection: looking, place: here } = latest.current;
      keepCamera(shown.panel, here, { camera, projection: looking });
      const keys = viewer.centre_entities();
      const at = Array.from(viewer.centre_mm());
      if (at.length < 2) {
        setCentre('Centre on: nothing, off the floor.');
        return;
      }
      const where = positionWords([Math.round(at[0]), Math.round(at[1])]);
      if (keys.length === 0) {
        setCentre(`Centre on: no cell, ${where}.`);
        return;
      }
      void shown.labelOf(keys[0]).then((name) => {
        if (alive && kept === said) setCentre(`Centre on: ${name ?? 'a floor cell'}, ${where}.`);
      });
    };
    const canvas = openCanvasViewer(gpu, lines);
    void (async () => {
      try {
        const opened = await canvas.opened;
        if (opened === null || !alive) return;
        const { module, viewer } = opened;
        [moduleRef.current, viewerRef.current] = [module, viewer];
        latest.current.props.onViewer?.(viewer);
        fit(viewer, stage, gpu);
        viewer.set_palette(module.readPalette(stage));
        viewer.set_projection('orthographic');
        let last = performance.now();
        const step = (now: number) => {
          if (!alive) return;
          const presented = viewer.frame(Math.max(0, now - last) / 1000);
          if (presented && unpainted.current !== null) {
            mark('floor:painted', { key: unpainted.current });
            unpainted.current = null;
          }
          last = now;
          follow(viewer, now);
          frame = requestAnimationFrame(step);
        };
        frame = requestAnimationFrame(step);
        setOpened('open');
      } catch {
        if (alive) setOpened('no-gpu');
      }
    })();
    return () => {
      alive = false;
      cancelAnimationFrame(frame);
      if (viewerRef.current !== null) latest.current.props.onViewer?.(null);
      canvas.free();
      viewerRef.current = null;
    };
  }, []);

  // The canvas follows the stage's size, the screen's pixel ratio, the theme and forced colours.
  useEffect(() => {
    const [viewer, module, stage, gpu] = [viewerRef.current, moduleRef.current, stageRef.current, gpuRef.current];
    if (opened !== 'open' || viewer === null || module === null || stage === null || gpu === null) return;
    const resized = new ResizeObserver(() => fit(viewer, stage, gpu));
    resized.observe(stage);
    let ratio = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
    const reratio = () => {
      fit(viewer, stage, gpu);
      ratio.removeEventListener('change', reratio);
      ratio = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      ratio.addEventListener('change', reratio);
    };
    ratio.addEventListener('change', reratio);
    const unthemed = onPaletteChange(() => viewer.set_palette(module.readPalette(stage)));
    const wheel = (event: WheelEvent) => {
      if (!latest.current.props.interactive) return;
      event.preventDefault();
      viewer.wheel(event.deltaX, event.deltaY, event.shiftKey);
    };
    gpu.addEventListener('wheel', wheel, { passive: false });
    return () => {
      resized.disconnect();
      ratio.removeEventListener('change', reratio);
      unthemed();
      gpu.removeEventListener('wheel', wheel);
    };
  }, [opened]);

  // The first floor of a place opens on the camera kept for this panel and place, if one is; else it stays fitted, in
  // the projection the page asked for.
  const restoreCamera = (viewer: Viewer) => {
    const { props: shown, place: here } = latest.current;
    const kept = keptCamera(shown.panel, here);
    if (kept === null) {
      if (shown.initialProjection !== undefined) viewer.set_projection(shown.initialProjection);
      return;
    }
    viewer.set_projection(kept.projection);
    viewer.set_camera(new Float32Array(kept.camera));
    setProjection(kept.projection);
  };

  // The floor: fetched by its key and drawn; the first one opens on the camera kept for this panel and place, else
  // fitted, and a later key (a new changeset of the same place) keeps the camera. A show's framing follows below.
  const frames = source.kind === 'run' ? source.frames : null;
  const token = source.kind === 'run' ? source.token : null;
  useEffect(() => {
    const viewer = viewerRef.current;
    if (opened !== 'open' || viewer === null || floorKey === null || frames !== null) return;
    let alive = true;
    void (async () => {
      mark('floor:asked', { key: floorKey });
      const read = await readFloor(floorKey);
      if (!alive) return;
      if (!read.ok) {
        setDrawn({ key: floorKey, failed: read.failed });
        return;
      }
      try {
        const first = !viewer.has_floor();
        mark('floor:read', { key: floorKey, bytes: read.bytes.length });
        viewer.set_floor(read.bytes);
        mark('floor:drawn', { key: floorKey });
        unpainted.current = floorKey;
        if (first) restoreCamera(viewer);
        setDrawn({ key: floorKey, failed: null });
        latest.current.props.onDrawn?.(floorKey);
      } catch (failure) {
        setDrawn({ key: floorKey, failed: `The floor could not be drawn: ${String(failure)}.` });
      }
    })();
    return () => {
      alive = false;
    };
  }, [opened, floorKey, attempt, frames]);

  // A run's frames: attached to by their socket, each pushed into the viewer; the scenario frame is the floor, drawn
  // the moment it is in. A frame the viewer refuses, the server's refusal or the socket closing early is the stage's
  // failure, retried by attaching again, by Retry or on a fresh ticket. Each attachment starts the viewer afresh, the
  // last one's frames and any damaged stream it stopped on forgotten, and says so at once: from then until its keyframe
  // the viewer holds no window. The socket closes with the mount, or when the address or ticket changes.
  useEffect(() => {
    const [viewer, module] = [viewerRef.current, moduleRef.current];
    if (opened !== 'open' || viewer === null || module === null || frames === null) return;
    const key = `frames:${frames}`;
    const said = (event: StreamEvent) => latest.current.props.onStream?.(event);
    const failed = (why: string) => setDrawn({ key, failed: `The run's frames could not be followed: ${why}.` });
    let floored = false;
    viewer.clear();
    said({ kind: 'attaching' });
    const attachment = module.attachFrames(frames, viewer, {
      token: token ?? undefined,
      onAttached: (attached) => said({ kind: 'attached', ...attached }),
      onFrame: (tag) => {
        if (tag === 'scenario' && !floored) {
          floored = true;
          unpainted.current = key;
          restoreCamera(viewer);
          setDrawn({ key, failed: null });
          latest.current.props.onDrawn?.(key);
        }
        said({ kind: 'frame', tag });
      },
      onLagged: (resume_from_ms) => said({ kind: 'lagged', resume_from_ms }),
      onEnded: (outcome) => said({ kind: 'ended', outcome }),
      onError: (message) => {
        failed(message);
        said({ kind: 'error', message });
      },
      onClose: () => {
        failed('the streaming server closed the connection');
        said({ kind: 'closed' });
      },
    });
    return () => attachment.close();
  }, [opened, frames, token, attempt]);

  const wanted = wantedOf(source, floorKey);
  const ready = drawn !== null && drawn.failed === null && drawn.key === wanted;
  // A show's highlights frame the stage once per command, over any camera kept; a later highlight never moves it.
  const framedFor = useRef<number | null>(null);
  useEffect(() => {
    const marks = highlights.flatMap((group) => group.marks);
    if (!ready || !frameOnOpen || highlightSeq === null || marks.length === 0 || framedFor.current === highlightSeq) return;
    framedFor.current = highlightSeq;
    viewerRef.current?.frame_entities(marks);
  }, [ready, frameOnOpen, highlights, highlightSeq]);
  useEffect(() => {
    if (ready) viewerRef.current?.set_highlights(highlights.map((group) => ({ marks: group.marks })));
  }, [ready, highlights]);
  useEffect(() => {
    if (ready) viewerRef.current?.set_selection(selection?.marks ?? []);
  }, [ready, selection]);
  useEffect(() => {
    if (ready) viewerRef.current?.set_axes(axes ?? undefined);
  }, [ready, axes]);

  const choose = (next: Projection, spoken: boolean) => {
    const viewer = viewerRef.current;
    if (viewer === null) return;
    viewer.set_projection(next);
    setProjection(next);
    if (spoken) services?.announce(next === 'orbit' ? 'Orbit: drag, or Shift with the arrows, turns and tilts.' : 'Orthographic, straight down.');
  };

  const act = (action: StageAction) => {
    const [viewer, stage] = [viewerRef.current, stageRef.current];
    if (viewer === null || stage === null || !ready || moveCamera(viewer, stage, action, projection)) return;
    switch (action.kind) {
      case 'frame': {
        const marks = highlights.flatMap((group) => group.marks);
        if (marks.length === 0 || viewer.frame_entities(marks) === 0) services?.announce('No highlights stand on the floor to frame.');
        return;
      }
      case 'select': props.onPick(viewer.centre_entities(), 'centre'); return;
      case 'clear': props.onClearSelection(); return;
      case 'projection': choose(projection === 'orbit' ? 'orthographic' : 'orbit', true); return;
    }
  };

  const pressed = useRef<[number, number] | null>(null);
  const down = (event: PointerEvent<HTMLCanvasElement>) => {
    if (!interactive || viewerRef.current === null) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    pressed.current = [event.clientX, event.clientY];
    viewerRef.current.pointer_down(event.nativeEvent.offsetX, event.nativeEvent.offsetY, event.button === 1);
  };
  const move = (event: PointerEvent<HTMLCanvasElement>) => {
    if (interactive) viewerRef.current?.pointer_move(event.nativeEvent.offsetX, event.nativeEvent.offsetY, event.shiftKey);
  };
  const up = (event: PointerEvent<HTMLCanvasElement>) => {
    const viewer = viewerRef.current;
    if (!interactive || viewer === null) return;
    viewer.pointer_up();
    const at = pressed.current;
    pressed.current = null;
    if (at !== null && Math.hypot(event.clientX - at[0], event.clientY - at[1]) < CLICK_PX && ready) {
      props.onPick(viewer.pick_entities(event.nativeEvent.offsetX, event.nativeEvent.offsetY), 'pointer');
    }
  };
  const key = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    const action = stageAction(event.key, event.shiftKey, projection);
    // Escape with nothing selected is left to what holds the stage: a filled window returns, a dialog closes.
    if (action === null || (action.kind === 'clear' && selection === null)) return;
    event.preventDefault();
    act(action);
  };

  const state = opened === 'no-gpu' ? 'no-gpu'
    : waiting !== null ? waiting.state
      : drawn !== null && drawn.key === wanted && drawn.failed !== null ? 'failed'
        : ready ? (highlights.length > 0 ? 'highlighted' : 'drawn') : 'loading';
  const status = state === 'no-gpu' ? 'This browser cannot draw the map; everything the map shows is in the lists beside it.'
    : waiting !== null ? waiting.words
      : state === 'failed' ? drawn?.failed ?? ''
        : state === 'loading' ? (source.kind === 'run' ? "Waiting for the run's first frame." : 'Drawing the floor.') : null;
  const retry = waiting?.retry ?? (state === 'failed' ? props.onRetry ?? (() => setAttempt((n) => n + 1)) : undefined);
  // Retry leaves once what it asked for draws; focus on it goes to the stage.
  const retryHome = useFocusHome(() => [stageRef.current]);
  const [titleId, summaryId, centreId, keysId] = [`${id}-title`, `${id}-summary`, `${id}-centre`, `${id}-keys`];
  const sourceText = source.kind === 'scope' ? `scope:${source.location}`
    : source.kind === 'evaluation' ? `evaluation:${source.draft}/${source.n}` : `run:${source.run}/${source.attempt}`;

  return (
    <section
      className="worldview-mount" aria-labelledby={titleId} data-source={sourceText}
      data-kin={source.kind === 'scope' ? source.kin : undefined} data-state={state} data-projection={projection}
      data-selection={selection?.key ?? ''} data-highlights={highlightSeq ?? ''}
    >
      {untitled ? <p className="visually-hidden" id={titleId}>{label}</p> : <Heading className="section__title" id={titleId}>{label}</Heading>}
      <div
        className="worldview-mount__stage" ref={stageRef}
        {...(interactive
          ? {
            role: 'application', tabIndex: 0, 'aria-roledescription': 'floor map', 'aria-label': focusedName ?? label,
            'aria-describedby': [props.describedBy, summaryId, centreId, keysId].filter((part) => part !== undefined).join(' '), onKeyDown: key,
            onFocus: () => {
              setFocusedName(label);
              viewerRef.current?.show_centre_cross(true);
            },
            onBlur: () => {
              setFocusedName(null);
              viewerRef.current?.show_centre_cross(false);
            },
          }
          : { role: 'img', 'aria-label': `${label}. ${description}`, 'aria-describedby': props.describedBy })}
      >
        <canvas
          className="worldview-mount__gpu" ref={gpuRef} aria-hidden="true"
          onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerLeave={() => viewerRef.current?.pointer_left()}
        />
        <canvas className="worldview-mount__lines" ref={linesRef} aria-hidden="true" />
      </div>
      {status !== null && (
        <p className="worldview-mount__status" aria-busy={state === 'loading'} {...retryHome}>
          {status}{' '}
          {retry !== undefined && state === 'failed' && <button className="btn btn--small" type="button" onClick={retry}>Retry</button>}
        </p>
      )}
      <p className="muted" id={summaryId}>{description}</p>
      <p className="muted" id={centreId}>{centre}</p>
      {interactive && (
        <div className="worldview-mount__controls">
          <div className="stage-tools" role="toolbar" aria-label="Map controls">
            <Tool name="Fit to view (0)" path="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" onClick={() => act({ kind: 'fit' })} />
            <Tool name="Zoom in (+)" path="M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM21 21l-5-5M11 8v6M8 11h6" onClick={() => act({ kind: 'zoom', steps: 1 })} />
            <Tool name="Zoom out (−)" path="M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM21 21l-5-5M8 11h6" onClick={() => act({ kind: 'zoom', steps: -1 })} />
            <Tool name="Frame the highlights (F)" path="M3 3h7v7H3zM14 3h7v7h-7zM3 14h7v7H3zM14 14h7v7h-7z" onClick={() => act({ kind: 'frame' })} />
            <span className="stage-tools__gap" aria-hidden="true" />
            {PAN_BUTTONS.map(({ glyph, name, right, down }) => <Tool key={name} name={name} path={glyph} onClick={() => act({ kind: 'pan', right, down })} />)}
            <span className="stage-tools__gap" aria-hidden="true" />
            <Tool name="Pick the centre (Enter)" path="M12 3v4M12 17v4M3 12h4M17 12h4M12 12h.01" onClick={() => act({ kind: 'select' })} />
            <fieldset className="worldview-mount__projection stage-tools__projection">
              <legend className="visually-hidden">Projection (O)</legend>
              {(['orthographic', 'orbit'] as const).map((choice) => (
                <label key={choice} className="stage-tool stage-tool--choice" title={choice === 'orthographic' ? 'Orthographic, straight down (O)' : 'Orbit: turn and tilt (O)'} data-on={projection === choice ? 'true' : undefined}>
                  <input type="radio" name={`${id}-projection`} value={choice} checked={projection === choice} onChange={() => choose(choice, false)} className="visually-hidden" />
                  <span aria-hidden="true">{choice === 'orthographic' ? '2D' : '3D'}</span>
                  <span className="visually-hidden">{choice === 'orthographic' ? 'Orthographic' : 'Orbit'}</span>
                </label>
              ))}
            </fieldset>
            {projection === 'orbit' && (
              <>
                <span className="stage-tools__gap" aria-hidden="true" />
                <Tool name="Turn left (Shift+←)" path="M3 12a9 9 0 1 0 3-6.7M3 4v5h5" onClick={() => act({ kind: 'turn', turn: -1, tilt: 0 })} />
                <Tool name="Turn right (Shift+→)" path="M21 12a9 9 0 1 1-3-6.7M21 4v5h-5" onClick={() => act({ kind: 'turn', turn: 1, tilt: 0 })} />
                <Tool name="Tilt up (Shift+↑)" path="M12 20V8M6 14l6-6 6 6" onClick={() => act({ kind: 'turn', turn: 0, tilt: 1 })} />
                <Tool name="Tilt down (Shift+↓)" path="M12 4v12M6 10l6 6 6-6" onClick={() => act({ kind: 'turn', turn: 0, tilt: -1 })} />
              </>
            )}
            {onExpand !== undefined && (
              <>
                <span className="stage-tools__gap" aria-hidden="true" />
                <Tool name="Expand the map" path="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7" onClick={onExpand} />
              </>
            )}
            {fill !== undefined && (
              <>
                <span className="stage-tools__gap" aria-hidden="true" />
                <Tool name={FILL_NAME} title={fillTitle(fill.filled)} path={fillGlyph(fill.filled)} pressed={fill.filled} onClick={fill.toggle} />
              </>
            )}
          </div>
        </div>
      )}
      {interactive && (
        <p className="muted visually-hidden" id={keysId}>{KEYS_LINE}</p>
      )}
    </section>
  );
}
