// Within the run page's playback, the recovery is the rule by which its watch (`playbackWatch.ts`) gets an attempt's
// picture back, as one pure reducer, so that a test can walk every sequence of what the watch hears. An ask is one
// `POST /api/runs/{run}/watch`, named by its number; the state names the ask in flight, and an answer to any other ask
// is dropped. At most one ask is in flight: a loss, the pause or the run's ending heard while one is changes the state,
// and the answer decides what follows; only Retry asks over it, and the answer to the ask it replaces is then dropped.
// A loss, an attachment lost or an ask refused while no attachment is being made or playing, interrupts the picture
// until an attachment is accepted, and is answered by one ask 2 s on (wall clock); once the run reports progress or
// has ended, five such asks, then only Retry. The five are given back when a lost attachment's picture had reached
// further in simtime than any lost before, and once as the run ends with a picture to play, when a page that stopped
// asks at once. An attachment playing goes on until it is lost. The run, as the page renders it, is state too, so an
// answer means what the state says as it arrives; once the run has reported progress or ended, the state keeps it, as
// an older read of the run can render after a newer.

import type { WatchTicket } from '../../protocol/runs.ts';
import { clause } from '../../shell/api.ts';

export const AGAIN_AFTER_MS = 2_000;
export const AGAIN_AT_MOST = 5;

/** The page's answer for the picture: asking for its first ticket, the ticket the player attaches with, the API's
 * word that no server can serve the attempt, or why an ask failed before any ticket came. */
export type Ticket =
  | { kind: 'asking' }
  | { kind: 'got'; ticket: WatchTicket }
  | { kind: 'none' }
  | { kind: 'refused'; reason: string };

/** The attachment made with the ticket held: awaited (the player has yet to begin it, so what an earlier attachment
 * says is not its), attaching (not yet accepted), playing (accepted) or lost. */
export type Attachment = 'awaited' | 'attaching' | 'playing' | 'lost';

/** The run as the page renders it: whether a streaming server can serve the attempt (`served`, as the page reckons
 * it), whether the run reports progress (its simtime past zero), and whether it has ended. */
export type RunSeen = { served: boolean; progressed: boolean; ended: boolean };

export type Recovery = RunSeen & {
  ticket: Ticket;
  /** The attachment made with the ticket, while one is held. */
  attachment: Attachment | null;
  /** How many asks were made, each named by its number from 1; `inFlight` names the one in flight, the latest. */
  asked: number;
  inFlight: number | null;
  /** Why the picture is interrupted: from a loss until an attachment is accepted. */
  trouble: string | null;
  /** The pause before the page's next ask of its own, named by the asks made as it began, while it runs. */
  pause: number | null;
  /** The page's own asks since the five were last given back; `furthest`, the furthest simtime a lost attachment's
   * picture reached; `endingHeard`, whether the run's ending with a picture to play has given the five back. */
  spent: number;
  furthest: number | null;
  endingHeard: boolean;
};

/** What the watch hears: an ask answered or refused, by its number; the attachment begun, accepted, or lost with how
 * far in simtime its picture reached; the pause over, by its name; Retry; and the run as a render has it. */
export type Heard =
  | { kind: 'got'; ask: number; ticket: WatchTicket }
  | { kind: 'refused'; ask: number; reason: string; notStreamed: boolean }
  | { kind: 'attaching' }
  | { kind: 'accepted' }
  | { kind: 'lost'; why: string; reach: number | null }
  | { kind: 'paused'; pause: number }
  | { kind: 'retry' }
  | ({ kind: 'run' } & RunSeen);

/** A watch's start: its first ask in flight. */
export const start = (run: RunSeen): Recovery => ({
  ...run, ticket: { kind: 'asking' }, attachment: null, asked: 1, inFlight: 1, trouble: null, pause: null, spent: 0,
  furthest: null, endingHeard: run.ended && run.served,
});

/** An attachment is being made or plays: it, not an ask, decides what follows. */
export const attachmentGoesOn = ({ attachment }: Recovery): boolean => attachment !== null && attachment !== 'lost';

/** The page stopped: interrupted, with no ask in flight, no pause and no attachment going on; only Retry asks. */
export const stopped = (was: Recovery): boolean =>
  was.trouble !== null && was.inFlight === null && was.pause === null && !attachmentGoesOn(was);

const ask = (was: Recovery): Recovery => ({ ...was, asked: was.asked + 1, inFlight: was.asked + 1, pause: null });

/** After a loss: with nothing in flight or going on, the pause, or nothing until Retry once five are spent. */
const following = (was: Recovery): Recovery =>
  was.inFlight !== null || attachmentGoesOn(was) || was.spent >= AGAIN_AT_MOST ? was : { ...was, pause: was.asked };

function answered(was: Recovery, heard: Extract<Heard, { kind: 'got' | 'refused' }>): Recovery {
  const held = was.ticket.kind === 'got';
  if (heard.kind === 'got') {
    return was.attachment === 'playing' ? was : { ...was, ticket: { kind: 'got', ticket: heard.ticket }, attachment: 'awaited' };
  }
  if (heard.notStreamed && !was.served && !held) return { ...was, ticket: { kind: 'none' }, trouble: null };
  if (attachmentGoesOn(was)) return was;
  return following({ ...was, ticket: held ? was.ticket : { kind: 'refused', reason: heard.reason }, trouble: clause(heard.reason) });
}

export function hear(was: Recovery, heard: Heard): Recovery {
  switch (heard.kind) {
    case 'got':
    case 'refused':
      return heard.ask === was.inFlight ? answered({ ...was, inFlight: null }, heard) : was;
    case 'attaching': return was.attachment === 'awaited' ? { ...was, attachment: 'attaching' } : was;
    case 'accepted': return was.attachment === 'attaching' ? { ...was, attachment: 'playing', trouble: null } : was;
    case 'lost': {
      if (was.attachment !== 'attaching' && was.attachment !== 'playing') return was;
      const further = heard.reach !== null && (was.furthest === null || heard.reach > was.furthest);
      const lost = { ...was, attachment: 'lost' as const, trouble: heard.why };
      return following(further ? { ...lost, spent: 0, furthest: heard.reach } : lost);
    }
    case 'paused':
      return heard.pause === was.pause ? { ...ask(was), spent: was.spent + (was.progressed || was.ended ? 1 : 0) } : was;
    case 'retry': return ask(was);
    case 'run': {
      const seen = { ...was, served: heard.served, progressed: was.progressed || heard.progressed, ended: was.ended || heard.ended };
      const servable = heard.served && !was.served && was.ticket.kind === 'none';
      const asking = servable ? { ...ask(seen), ticket: { kind: 'asking' as const } } : seen;
      if (!seen.ended || !heard.served || was.endingHeard) return asking;
      const ending = { ...asking, spent: 0, endingHeard: true };
      return stopped(ending) ? ask(ending) : ending;
    }
  }
}
