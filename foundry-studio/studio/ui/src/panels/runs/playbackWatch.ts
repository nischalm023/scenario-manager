// Within the run page's playback (`RunPlayback`), the watch is the page's hold on one attempt's picture: the watch
// ticket its player attaches with (`POST /api/runs/{run}/watch`, ruling D174, alive for a minute), got back after every
// loss by the recovery's rule (`playbackRecovery.ts`). This hook carries the rule out: it sends each ask the rule puts
// in flight and runs each pause the rule names, and hands the rule each answer, each event of the player's
// attachment, each Retry, and the run as the page renders it.

import { useCallback, useEffect, useReducer } from 'react';
import { runEnded } from '../../chat/words.ts';
import type { RunDetail, WatchTicket } from '../../protocol/runs.ts';
import { postJson } from '../../shell/api.ts';
import type { StreamEvent } from '../../viewer/WorldviewMount.tsx';
import { AGAIN_AFTER_MS, AGAIN_AT_MOST, hear, start, stopped, type RunSeen, type Ticket } from './playbackRecovery.ts';

export type PlaybackWatch = {
  ticket: Ticket;
  /** Why the picture is interrupted, if it is. */
  trouble: string | null;
  /** The page asks no more on its own, so no keyframe can come until someone presses Retry. */
  gaveUp: boolean;
  /** The page has had its tries: only Retry asks again. */
  hadItsTries: boolean;
  /** Hears every event of the player's attachment; `reach`, read only at a loss, is how far in simtime the picture
   * reached, the end of the window the viewer holds. */
  heard: (event: StreamEvent, reach: () => number | null) => void;
  /** Someone pressed Retry: a fresh ticket at once. */
  retry: () => void;
};

/** The watch of `run`'s latest attempt, for as long as it is the latest; `served`: whether a streaming server can
 * serve the attempt, as the page reckons it. */
export function usePlaybackWatch(run: RunDetail, served: boolean): PlaybackWatch {
  const seen: RunSeen = { served, progressed: run.progress.sim_ms > 0, ended: runEnded(run.state) };
  const [watch, dispatch] = useReducer(hear, seen, start);
  // The run is heard whenever this render has it otherwise than the watch: its server reckoned otherwise, or progress or
  // an ending the watch has yet to hear, which an older read rendered after a newer never takes back. Compared with the
  // watch's own state, so a render React makes again from an earlier state hears it again.
  if (watch.served !== seen.served || (seen.progressed && !watch.progressed) || (seen.ended && !watch.ended)) {
    dispatch({ kind: 'run', ...seen });
  }
  const { inFlight, pause } = watch;
  useEffect(() => {
    if (inFlight === null) return;
    // A request whose effect React has cleaned up answers no one, as React runs effects twice in development.
    let kept = true;
    void postJson<WatchTicket>(`/api/runs/${run.id}/watch`, {}).then((answer) => kept && dispatch(answer.ok
      ? { kind: 'got', ask: inFlight, ticket: answer.value }
      : { kind: 'refused', ask: inFlight, reason: answer.reason, notStreamed: answer.problem?.type === 'not-streamed' }));
    return () => {
      kept = false;
    };
  }, [run.id, inFlight]);
  useEffect(() => {
    if (pause === null) return;
    const timer = globalThis.setTimeout(() => dispatch({ kind: 'paused', pause }), AGAIN_AFTER_MS);
    return () => globalThis.clearTimeout(timer);
  }, [pause]);
  const heard = useCallback((event: StreamEvent, reach: () => number | null) => {
    switch (event.kind) {
      case 'attaching': dispatch({ kind: 'attaching' }); return;
      case 'attached': dispatch({ kind: 'accepted' }); return;
      case 'error': dispatch({ kind: 'lost', why: event.message, reach: reach() }); return;
      case 'closed': dispatch({ kind: 'lost', why: 'the streaming server closed the connection', reach: reach() }); return;
      default: return;
    }
  }, []);
  const retry = useCallback(() => dispatch({ kind: 'retry' }), []);
  return { ticket: watch.ticket, trouble: watch.trouble, gaveUp: stopped(watch), hadItsTries: watch.spent >= AGAIN_AT_MOST, heard, retry };
}
