// A running attempt's numbers as they happen (ruling D174): a watch ticket from the API, the streaming server's
// websocket for the attempt, and the newest `progress` it says, which the page shows beside the polled read (its
// picture comes on the frames socket); a socket that cannot open or is dropped leaves the polled read standing, and
// says why in words.

import { useEffect, useState } from 'react';
import type { ProgressValue } from '../../protocol/run.ts';
import type { StreamRefusal, ViewerMessage } from '../../protocol/run_stream.ts';
import type { WatchTicket } from '../../protocol/runs.ts';
import { postJson } from '../../shell/api.ts';

/** The stream's format, which the watch states as `v`: `STREAM_FORMAT` in protocol/src/run_stream.rs, which the
 * generated types do not carry. The server closes a socket that speaks another. */
const STREAM_FORMAT = 2;

/** Why the server closed the socket, said for a reader: its close reason is a refusal's code. */
const CLOSED: Partial<Record<StreamRefusal, string>> = {
  'no-greeting': 'the request to watch came too late',
  unreadable: 'the streaming server could not read the request to watch',
  'unsupported-version': 'the streaming server speaks another version of the stream',
  'token-invalid': 'the streaming server refused the watch ticket',
  unavailable: 'the streaming server could not check the watch ticket',
  'not-held': 'the streaming server holds no stream of this attempt',
  silent: 'the stream fell silent',
};

export type Live =
  | { state: 'connecting' }
  | { state: 'live'; simMs: number; values: ProgressValue[]; at: number }
  | { state: 'ended'; simMs: number; values: ProgressValue[] }
  | { state: 'lost'; why: string };

export function useLiveProgress(run: string, attempt: number, wanted: boolean): Live | null {
  const key = `${run}#${attempt}`;
  const [held, setHeld] = useState<{ key: string; live: Live } | null>(null);
  const setLive = (next: Live | ((was: Live | null) => Live)) =>
    setHeld((was) => ({ key, live: typeof next === 'function' ? next(was?.key === key ? was.live : null) : next }));
  useEffect(() => {
    if (!wanted || typeof WebSocket === 'undefined') return;
    let socket: WebSocket | null = null;
    let alive = true;
    let simMs = 0;
    let values: ProgressValue[] = [];
    void postJson<WatchTicket>(`/api/runs/${run}/watch`, {}).then((ticket) => {
      if (!alive) return;
      if (!ticket.ok) {
        setLive({ state: 'lost', why: ticket.reason });
        return;
      }
      try {
        socket = new WebSocket(ticket.value.url);
      } catch (failure) {
        setLive({ state: 'lost', why: failure instanceof Error ? failure.message : 'the socket did not open' });
        return;
      }
      socket.onopen = () => socket?.send(JSON.stringify({ type: 'watch', v: STREAM_FORMAT, token: ticket.value.token }));
      socket.onmessage = (event) => {
        let said: ViewerMessage;
        try {
          said = JSON.parse(String(event.data)) as ViewerMessage;
        } catch {
          return;
        }
        if (said.type === 'progress') {
          simMs = said.sim_ms;
          values = said.values;
          setLive({ state: 'live', simMs, values, at: Date.now() });
        } else if (said.type === 'ended') {
          setLive({ state: 'ended', simMs: said.ending.sim_ms ?? simMs, values });
        } else if (said.type === 'runner-lost') {
          setLive({ state: 'lost', why: 'the runner was lost' });
        } else if (said.type === 'lagged') {
          setLive({ state: 'lost', why: 'this view fell behind the stream' });
        }
      };
      socket.onerror = () => { if (alive) setLive((was) => (was?.state === 'live' ? was : { state: 'lost', why: 'the stream could not be reached' })); };
      socket.onclose = (event) => {
        if (!alive) return;
        const why = CLOSED[event.reason as StreamRefusal] ?? 'the stream closed';
        setLive((was) => (was?.state === 'ended' || was?.state === 'lost' ? was : { state: 'lost', why }));
      };
    });
    return () => {
      alive = false;
      socket?.close();
    };
    // `setLive` closes over `key`, which `run` and `attempt` make.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run, attempt, wanted]);
  if (!wanted) return null;
  return held !== null && held.key === key ? held.live : { state: 'connecting' };
}
