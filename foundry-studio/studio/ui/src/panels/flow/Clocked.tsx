// Spec 06 section 5's clock markup for the Flow panel: a value carries the clock it counts in as
// `.clock[data-clock]` with its words in a tag, the clock read from the protocol's one discriminator, so a simtime
// figure, its all-in companion or a time on a hover card never shows without "simtime" beside it; wall clock, the
// unmarked default, carries no tag (spec 02's clocks as Daniel ruled them on 2026-10-19).

import type { ReactNode } from 'react';
import type { Clock } from '../../protocol/clock.ts';
import type { Card } from './words.ts';

const TAG: Record<Clock, string | null> = { sim: 'simtime', scheduler: 'scheduler time', wall: null };

export function Clocked({ clock, children }: { clock: Clock; children: ReactNode }) {
  const tag = TAG[clock];
  return <span className="clock" data-clock={clock}>{children}{tag !== null && <span className="clock__tag">{tag}</span>}</span>;
}

/** A hover card's words, its simtime figure tagged. */
export function CardText({ card }: { card: Card | undefined }) {
  if (card === undefined) return null;
  return <>{card.text}{card.simtime !== null && <> <Clocked clock="sim">{card.simtime}</Clocked></>}</>;
}
