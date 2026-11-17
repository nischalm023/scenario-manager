// Spec 08 section 5's Playback of a run (live frames, 26 October 2026): the run's picture, drawn by the worldview
// viewer through `WorldviewMount` fed from the streaming server's frames socket. The page first asks the API for a
// watch ticket (`POST /api/runs/{run}/watch`, ruling D174), which names the socket and the token the server admits
// the attach by, alive for a minute; a run with no live stream is one plain sentence. A badge says what the picture
// is: live, as of the simtime instant the locked frames reach, and how far that trails the simulator's own progress;
// or a replay, once every frame is in or the run has ended, and a warning when the server says its copy of the
// recording stops short of the run's end. The cursor moves over what has arrived, with play at a
// chosen speed and pause; while live it follows the edge until someone moves it. The page's watch (`playbackWatch.ts`)
// asks for a fresh ticket after every loss, which the same player attaches with, its viewer and whatever has focus in
// it kept; the stage offers Retry for a lost attachment, and the sentence that says why there is no playback offers it
// once the page stops asking.
// An attachment made again (on a fresh ticket, or after the server dropped a slow one) clears the viewer, which holds
// no window until that attachment's keyframe arrives; from the clear until then the controls keep the last window,
// play and the cursor inert, and drop it once none can come. Focus on a Retry or a playback control that leaves goes
// to Play or the stage, or before any stage to the sentence saying why there is no playback. The cursor stated is the
// instant the picture shows, within the window held, and the page is told it, to draw on the metric charts. Every
// time is simtime and says so; the viewer's window is read as each keyframe opens it, then every quarter second of
// wall clock, since the viewer has no way to say it grew.

import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import type { Viewer } from 'crucible-viewer';
import { runEnded, simtimeWords } from '../../chat/words.ts';
import type { RunOutcome } from '../../protocol/metric_stream.ts';
import type { RunDetail, WatchTicket } from '../../protocol/runs.ts';
import { clause } from '../../shell/api.ts';
import type { FillWindow } from '../../shell/fillWindow.ts';
import { useFocusHome } from '../../shell/focus.ts';
import { WorldviewMount, type StreamEvent } from '../../viewer/WorldviewMount.tsx';
import { usePlaybackWatch } from './playbackWatch.ts';
import { INCOMPLETE_WHY } from './words.ts';

/** How often the viewer's window and cursor are read (wall clock). */
const READ_EVERY_MS = 250;
/** Simulated seconds per real second the play button offers. */
const SPEEDS = [1, 10, 60] as const;
/** The window the controls offer, its cursor and whether it plays; `kept` while the viewer, attaching again, holds no
 * window and the controls keep its last one, until a keyframe opens one, or none can come and it is dropped;
 * `gaveUp`, the page's word that it asks no more, as the window last heard it. */
type Held = { span: [number, number] | null; cursor: number; playing: boolean; kept: boolean; gaveUp: boolean };
const keep = (was: Held): Held => (was.span === null || was.kept ? was : { ...was, kept: true });
const drop = (was: Held): Held => (was.kept ? { ...was, span: null, kept: false } : was);

/** A simtime instant in its `.clock`, tagged, as the run page writes every time. */
function SimAt({ ms }: { ms: number }) {
  return <span className="clock" data-clock="sim">{simtimeWords(ms)} <span className="clock__tag">simtime</span></span>;
}

/** The window `viewer` holds, from and to in simtime, or null before a keyframe opens one. */
function windowOf(viewer: Viewer): [number, number] | null {
  const window = Array.from(viewer.held_window());
  return window.length === 2 ? [window[0], window[1]] : null;
}

/** The stage of the player drawn within `root`, which focus goes to when a control beside it leaves. */
const stageIn = (root: HTMLElement | null): HTMLElement | null => root?.querySelector<HTMLElement>('.worldview-mount__stage') ?? null;

type RunPlaybackProps = {
  run: RunDetail;
  /** Whether a streaming server can serve the attempt, as the page reckons it: while none can, the API's
   * `not-streamed` is said, and the page asks again once one can. */
  served: boolean;
  /** No section or title of its own, for a page that puts the playback inside a card it has titled. */
  bare?: boolean;
  /** Offered on the stage's toolbar: the page fills the window with the picture and what it keeps beside it. */
  fill?: FillWindow;
  /** Told the cursor's simtime as it is read, so the page draws it on the metric charts. */
  onCursor?: (ms: number) => void;
  /** What else describes the picture, by element id: a stub attempt's sentence (ruling D110). */
  describedBy?: string;
};

/** A new run or attempt is a new picture: watched afresh, by a new player. */
export function RunPlayback(props: RunPlaybackProps) {
  return <Picture key={`${props.run.id}/${props.run.attempt}`} {...props} />;
}

function Picture({ run, served, bare = false, fill, onCursor, describedBy }: RunPlaybackProps) {
  const { ticket, trouble, gaveUp, hadItsTries, heard, retry } = usePlaybackWatch(run, served);
  // One element holds the playback through every answer, so the sentence's Retry, which leaves once a ticket comes,
  // hands focus to the stage of the player in its place; when the run's ending gives the page its asks back before
  // any ticket, Retry leaves with no stage drawn, and the sentence, saying the page asks again, takes focus until then.
  const root = useRef<HTMLElement | null>(null);
  const at = useCallback((element: HTMLElement | null) => {
    root.current = element;
  }, []);
  const sentence = useRef<HTMLParagraphElement>(null);
  const sentenceHome = useFocusHome(() => [stageIn(root.current), sentence.current]);
  const Wrap = bare ? 'div' : 'section';
  return (
    <Wrap className={bare ? 'run-playback' : 'section run-playback'} ref={at}>
      {ticket.kind === 'got' ? (
        <Player
          run={run} ticket={ticket.ticket} bare={bare} root={root} fill={fill} onCursor={onCursor} trouble={trouble}
          gaveUp={gaveUp} heard={heard} onRetry={retry} describedBy={describedBy}
        />
      ) : (
        <>
          {!bare && <h4 className="section__title">Playback</h4>}
          <p className="muted" id="run-playback-none" tabIndex={-1} ref={sentence} {...sentenceHome}>
            {ticket.kind === 'asking' ? "Asking for the run's stream."
              : ticket.kind === 'none' ? 'No streaming server is up for this run, so there is no playback to show.'
                : <>No playback: {clause(ticket.reason)}.{' '}
                  {hadItsTries ? <button className="btn btn--small" type="button" onClick={retry}>Retry</button> : 'Asking again.'}</>}
          </p>
        </>
      )}
    </Wrap>
  );
}

function Player({ run, ticket, bare, root, fill, onCursor, trouble, gaveUp, heard, onRetry, describedBy }: {
  run: RunDetail; ticket: WatchTicket; bare: boolean;
  /** What holds the playback, around the stage. */
  root: RefObject<HTMLElement | null>;
  fill?: FillWindow; onCursor?: (ms: number) => void;
  /** Why the picture is interrupted, if it is. */
  trouble: string | null;
  /** The page asks no more on its own, so no keyframe can come until someone presses Retry. */
  gaveUp: boolean;
  /** Hears every event of the attachment, for the page's recovery, with how far the picture reached at a loss. */
  heard: (event: StreamEvent, reach: () => number | null) => void;
  onRetry: () => void; describedBy?: string;
}) {
  const [viewer, setViewer] = useState<Viewer | null>(null);
  // How far the picture reaches in simtime: the server's word at the attach, then the viewer's window as it grows.
  const [edge, setEdge] = useState<number | null>(null);
  // The frames' last word, `ended`'s outcome, once every frame is in.
  const [framesEnded, setFramesEnded] = useState<string | null>(null);
  const [held, setHeld] = useState<Held>({ span: null, cursor: 0, playing: false, kept: false, gaveUp });
  const [speed, setSpeed] = useState<number>(SPEEDS[0]);
  const [follow, setFollow] = useState(true);
  const live = !runEnded(run.state) && framesEnded === null;
  const following = useRef({ follow, live });
  useEffect(() => {
    following.current = { follow, live };
  });

  // Whether the controls keep the last window follows the stream, which says the moment the viewer's window empties or
  // a keyframe opens it, never a timed read, so Play and the cursor are live again at the fresh keyframe, not up to a
  // quarter second after it. The timed read follows the window as it grows.
  const read = useCallback(() => {
    if (viewer === null) return;
    const span = windowOf(viewer);
    if (span === null) return;
    if (following.current.follow && following.current.live && !viewer.playing()) viewer.set_cursor(span[1]);
    // The instant the picture shows: the renderer clamps the viewer's cursor to the window, and the viewer keeps the
    // cursor as asked, for a later window that reaches it.
    const cursor = Math.min(Math.max(viewer.cursor_ms(), span[0]), span[1]);
    setHeld((was) => ({ ...was, span, cursor, playing: viewer.playing(), kept: false }));
    onCursor?.(cursor);
    setEdge((was) => Math.max(was ?? 0, span[1]));
  }, [viewer, onCursor]);
  useEffect(() => {
    if (viewer === null) return;
    const timer = globalThis.setInterval(read, READ_EVERY_MS);
    return () => globalThis.clearInterval(timer);
  }, [viewer, read]);
  // A kept window goes once no keyframe can come: the page gave up, or the fresh attachment ended without one. The
  // page's word is heard in `held` itself: React may render again from an earlier `held` (an update it put off behind
  // a socket's `close`) without the drop made in this render, and must then hear the word, and drop, again.
  if (gaveUp !== held.gaveUp) setHeld((was) => ({ ...(gaveUp ? drop(was) : was), gaveUp }));

  const onStream = (event: StreamEvent) => {
    heard(event, () => (viewer === null ? null : windowOf(viewer)?.[1] ?? null));
    switch (event.kind) {
      case 'attaching': setHeld(keep); return;
      case 'attached': setEdge((was) => Math.max(was ?? 0, event.live_edge_ms)); return;
      case 'frame':
        if (event.tag === 'keyframe') read();
        return;
      case 'lagged': setHeld(keep); return;
      case 'ended':
        setFramesEnded(event.outcome);
        setHeld(drop);
        return;
      default: return;
    }
  };

  const toggle = () => {
    if (viewer === null || held.kept) return;
    if (held.playing) {
      viewer.pause();
    } else {
      setFollow(false);
      viewer.play(speed);
    }
  };
  const chooseSpeed = (next: number) => {
    setSpeed(next);
    if (viewer !== null && held.playing) viewer.play(next);
  };
  const scrub = (ms: number) => {
    if (viewer === null || held.kept) return;
    viewer.pause();
    setFollow(false);
    viewer.set_cursor(ms);
    onCursor?.(ms);
    setHeld((was) => ({ ...was, cursor: ms, playing: false }));
  };

  const behind = edge !== null ? Math.max(0, run.progress.sim_ms - edge) : 0;
  const badge = trouble !== null ? { tone: 'warn', words: `Interrupted: ${trouble}` }
    : edge === null ? { tone: 'neutral', words: 'Attaching to the stream' }
      : live ? { tone: 'live', words: `Live, as of ${simtimeWords(edge)} simtime${behind > 0 ? `, trailing the simulator by ${simtimeWords(behind)}` : ''}` }
        : framesEnded === ('recording-incomplete' satisfies RunOutcome) ? { tone: 'warn', words: `Replay of an incomplete recording: ${INCOMPLETE_WHY}` }
          : { tone: 'neutral', words: 'Replay' };
  // Follow the live edge leaves as the run ends, and every control as a kept window goes; focus on one goes to Play,
  // else to the stage.
  const play = useRef<HTMLButtonElement>(null);
  const controlsHome = useFocusHome(() => [play.current, stageIn(root.current)]);
  return (
    <>
      <WorldviewMount
        source={{ kind: 'run', run: run.id, attempt: ticket.attempt, frames: ticket.frames_url, token: ticket.token }}
        floorKey={null} waiting={null} interactive headingLevel={4} panel="runs" label="Playback"
        description={`Robots and totes of ${run.label}, attempt ${ticket.attempt}, as the run states them; ${live ? 'live' : 'a replay'}.`}
        axes={null} selection={null} highlights={[]} highlightSeq={null}
        onPick={() => undefined} onClearSelection={() => undefined} labelOf={async () => null}
        onViewer={setViewer} onStream={onStream} onRetry={onRetry} untitled={bare} fill={fill} describedBy={describedBy}
      />
      <p className="run-playback__line" id="run-playback-mode">
        <span className="pill" data-tone={badge.tone}>{badge.words}</span>
        {held.span !== null && <>Cursor at <SimAt ms={held.cursor} />, of <SimAt ms={held.span[0]} /> to <SimAt ms={held.span[1]} />.</>}
      </p>
      {held.span !== null && (
        <div className="run-playback__controls" {...controlsHome}>
          <button
            className="btn btn--small" type="button" ref={play} onClick={toggle} aria-pressed={held.playing}
            aria-disabled={held.kept || undefined}
          >
            {held.playing ? 'Pause' : 'Play'}
          </button>
          <label>
            Speed{' '}
            <select value={speed} onChange={(event) => chooseSpeed(Number(event.target.value))}>
              {SPEEDS.map((choice) => <option key={choice} value={choice}>{choice}× simtime</option>)}
            </select>
          </label>
          <label className="run-playback__cursor">
            Cursor, simtime{' '}
            <input
              type="range" min={held.span[0]} max={held.span[1]} step={1000} value={held.cursor}
              aria-valuetext={`${simtimeWords(held.cursor)} simtime`} onChange={(event) => scrub(Number(event.target.value))}
              aria-disabled={held.kept || undefined}
            />
          </label>
          {live && (
            <label>
              <input type="checkbox" checked={follow} onChange={(event) => setFollow(event.target.checked)} /> Follow the live edge
            </label>
          )}
        </div>
      )}
    </>
  );
}
