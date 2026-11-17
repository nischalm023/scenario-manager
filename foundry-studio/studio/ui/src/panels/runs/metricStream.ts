// One metric of an attempt as its own stream (the metric streams; `studio/protocol/src/metric_stream.rs`): a watch
// ticket from the API, the streaming server's metrics socket for the attempt with the metric named in the attach,
// and the buckets as they come, appended and never revised, since the server sends each once it is final. The rows
// are appended in place, so a long run never copies its history, and a reader takes only the rows new since it last
// read (`charts.ts`'s `StreamChart`). The hook holds the stream in a ref and shows it at most a few times a second of
// wall clock, so a run played fast does not redraw a chart on every slice. A socket that cannot open, or that the
// server closes with a reason, leaves what came standing and says why in words, then attaches again after a pause:
// without end until the server first answers `attached`, since a page opened as the run starts asks before the server
// holds a frame, then a few times; each attach sends the history again from simtime 0, which replaces what was held. A
// final refusal is tried again neither by the hook nor on request: a socket refusal the protocol lists as final for a
// viewer (`VIEWER_FINAL`), of the ask or of the recording, or the API's `not-streamed` answer to a ticket, final for
// the run's picture too, since no server can serve the attempt. A page attaches afresh at once by a new try, and so
// does a new attempt. The page computes nothing: a row's value is the server's.

import { useEffect, useRef, useState } from 'react';
import type { MetricDescriptor, MetricMessage, MetricPoint, MetricRequest, RunOutcome } from '../../protocol/metric_stream.ts';
import type { StreamRefusal } from '../../protocol/run_stream.ts';
import { VIEWER_FINAL } from '../../protocol/run_stream_rules.ts';
import type { WatchTicket } from '../../protocol/runs.ts';
import { clause, postJson } from '../../shell/api.ts';

/** The stream's format, `METRIC_STREAM_FORMAT` in protocol/src/metric_stream.rs, which the generated types do not carry. */
const METRIC_STREAM_FORMAT = 1;
/** How often at most the rows held are shown (wall clock). */
const SHOW_EVERY_MS = 400;
/** How long after a lost socket the hook attaches again (wall clock), and how many times once one was attached. */
const AGAIN_AFTER_MS = 2_000;
const AGAIN_AT_MOST = 5;

/** Why the server closed the socket, said for a reader: its close reason is a refusal's code. */
const CLOSED: Partial<Record<StreamRefusal, string>> = {
  'no-greeting': 'the request to attach came too late',
  unreadable: 'the streaming server could not read the request to attach',
  'unsupported-version': 'the streaming server speaks another version of the metric stream',
  'token-invalid': 'the streaming server refused the watch ticket',
  unavailable: 'something the streaming server needs did not answer: its database, its disk or the run store',
  'not-held': 'the streaming server holds no stream of this attempt',
  'no-recording': 'no streaming server can serve this attempt: none holds its stream, and no recording of it is stored',
  'unknown-metric': 'the streaming server does not compute this metric',
  'unsupported-trailing': 'the streaming server does not cut this metric into the windows asked for',
  'not-recorded': "the run's recording does not hold what this metric is computed from",
  'recording-unreadable': "the run's recording does not read, so this metric cannot be computed from it",
  silent: 'the stream fell silent',
};

/** Why a socket was lost, and what follows: another attach after a pause, one only on the page's request once the
 * hook has had its tries, or none, after a final refusal. */
export type Loss = { why: string; next: 'again' | 'on-request' | 'final' };

export type MetricStream = {
  descriptor: MetricDescriptor | null;
  /** Appended in place, never revised: the stream owns the array. */
  rows: MetricPoint[];
  live: boolean;
  /** How the run's recording ended, once the server said `ended`. */
  ended: RunOutcome | null;
  /** Why nothing more comes on this socket, when it was lost or refused, and what follows. */
  lost: Loss | null;
  /** The server said the page fell behind: it attaches afresh. */
  lagged: boolean;
};

/** The stream a page shows before its socket speaks; its rows are frozen, since no stream owns them. */
export const UNATTACHED_STREAM: MetricStream = { descriptor: null, rows: Object.freeze([]) as unknown as MetricPoint[], live: false, ended: null, lost: null, lagged: false };

/** A stream with rows of its own to append to. */
export const freshStream = (): MetricStream => ({ ...UNATTACHED_STREAM, rows: [] });

/** The stream after one of the server's messages: `attached` starts the history afresh, as the server sends it again
 * from simtime 0, and `rows` appends to the stream's own rows in place. */
export function takeMessage(stream: MetricStream, message: MetricMessage): MetricStream {
  switch (message.type) {
    case 'attached':
      return { ...freshStream(), descriptor: message.descriptor, live: message.live };
    case 'rows':
      if (message.rows.length === 0) return stream;
      for (const row of message.rows) stream.rows.push(row);
      return { ...stream };
    case 'ended':
      return { ...stream, ended: message.outcome, live: false };
    case 'lagged':
      return { ...stream, lagged: true, live: false };
  }
}

/** The words a close frame's reason says, or the reason itself. */
export function closedWords(reason: string): string {
  return CLOSED[reason as StreamRefusal] ?? (reason === '' ? 'the streaming server closed the connection' : reason);
}

/** The socket for `metric` of `attempt`, the run's latest, through a fresh watch ticket; a new attempt, or a new
 * `tries`, attaches afresh at once. `runGoes` says whether the run still goes, as the page last read it. */
export function useMetricStream(run: string, attempt: number, metric: string, tries = 0, runGoes = false): MetricStream {
  // The stream is held with the ask it answers, so a new run, attempt, metric or try reads as unattached until its
  // socket speaks.
  const key = `${run}/${attempt}/${metric}/${tries}`;
  const [shown, setShown] = useState<{ key: string; stream: MetricStream } | null>(null);
  const held = useRef<MetricStream>(UNATTACHED_STREAM);
  // Read when a ticket is refused, not when the socket opens: the run can end while the hook asks again.
  const goes = useRef(runGoes);
  useEffect(() => {
    goes.current = runGoes;
  });
  useEffect(() => {
    held.current = freshStream();
    if (typeof WebSocket === 'undefined') return;
    let socket: WebSocket | null = null;
    let alive = true;
    let timer: number | null = null;
    let retry: number | null = null;
    let attachedOnce = false;
    let againAfterAttached = 0;
    const show = () => {
      timer = null;
      if (alive) setShown({ key, stream: held.current });
    };
    const soon = () => {
      if (timer === null) timer = globalThis.setTimeout(show, SHOW_EVERY_MS);
    };
    const take = (next: MetricStream) => {
      held.current = next;
      soon();
    };
    // What came stays standing with why it stopped; another attach follows unless the refusal is final or the page
    // has had its tries.
    const lose = (why: string, final = false) => {
      const next = final ? 'final' : !attachedOnce || againAfterAttached < AGAIN_AT_MOST ? 'again' : 'on-request';
      take({ ...held.current, live: false, lost: { why, next } });
      if (next !== 'again') return;
      if (attachedOnce) againAfterAttached += 1;
      retry = globalThis.setTimeout(attach, AGAIN_AFTER_MS);
    };
    const attach = () => {
      retry = null;
      void postJson<WatchTicket>(`/api/runs/${run}/watch`, {}).then((ticket) => {
        if (!alive) return;
        // `not-streamed` is final for a run that has ended. While the run goes it also answers a streaming server
        // whose heartbeat is stale, as one restarting is, so the hook asks again.
        if (!ticket.ok) {
          lose(clause(ticket.reason), ticket.problem?.type === 'not-streamed' && !goes.current);
          return;
        }
        open(ticket.value);
      });
    };
    const open = (ticket: WatchTicket) => {
      try {
        socket = new WebSocket(ticket.metrics_url);
      } catch (failure) {
        lose(failure instanceof Error ? failure.message : 'the socket did not open');
        return;
      }
      const opened = socket;
      opened.onopen = () => {
        const request: MetricRequest = { type: 'attach', v: METRIC_STREAM_FORMAT, metric, token: ticket.token };
        opened.send(JSON.stringify(request));
      };
      opened.onmessage = (event) => {
        let said: MetricMessage;
        try {
          said = JSON.parse(String(event.data)) as MetricMessage;
        } catch {
          return;
        }
        if (said.type === 'attached') attachedOnce = true;
        take(takeMessage(held.current, said));
      };
      // A socket that fails is closed too, so its close alone says it was lost, once.
      opened.onclose = (event) => {
        if (!alive || held.current.ended !== null) return;
        if (held.current.lagged) lose('the page fell behind the stream');
        else lose(closedWords(event.reason), VIEWER_FINAL.has(event.reason as StreamRefusal));
      };
    };
    attach();
    return () => {
      alive = false;
      if (timer !== null) globalThis.clearTimeout(timer);
      if (retry !== null) globalThis.clearTimeout(retry);
      socket?.close();
    };
  }, [key, run, metric]);
  return shown !== null && shown.key === key ? shown.stream : UNATTACHED_STREAM;
}
