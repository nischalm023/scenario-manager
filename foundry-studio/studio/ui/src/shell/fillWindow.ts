// A view filling the window: a map, a floor or a flow drawing, with whatever its page keeps beside it (an explorer,
// playback controls), grows over the whole window and returns by its button or by Escape. The page spreads `region`
// on the element that fills; studio.css then hides the rest of the shell, its landmarks too, so the view's own
// controls are the ones a keyboard reaches, and the view stands as a region of its own, named for what fills. Escape a
// control has already used (the stage's, clearing a selection; an "i" tip's or a hover card's, closing it) does not
// also return: the fill listens on the window, which hears a key after the document where those tips and cards listen.
// Back on its page, the focused control is scrolled into view: the page lays out what the filled view kept beside the
// drawing (the flow's explorer) under it, often below the fold.

import { useCallback, useEffect, useRef, useState } from 'react';

export interface FillWindow {
  filled: boolean;
  toggle: () => void;
  /** The attributes of the element that fills: none while it stands in its page. */
  region: { 'data-filled'?: 'true'; role?: 'region'; 'aria-label'?: string };
}

/** `name`: what fills the window, as its region is named while it does ("Process flow, filling the window"). */
export function useFillWindow(name: string): FillWindow {
  const [filled, setFilled] = useState(false);
  useEffect(() => {
    if (!filled) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !event.defaultPrevented) setFilled(false);
    };
    window.addEventListener('keydown', escape);
    return () => window.removeEventListener('keydown', escape);
  }, [filled]);
  const wasFilled = useRef(false);
  useEffect(() => {
    if (wasFilled.current && !filled) document.activeElement?.scrollIntoView({ block: 'nearest' });
    wasFilled.current = filled;
  }, [filled]);
  const toggle = useCallback(() => setFilled((was) => !was), []);
  const region: FillWindow['region'] = filled ? { 'data-filled': 'true', role: 'region', 'aria-label': `${name}, filling the window` } : {};
  return { filled, toggle, region };
}

/** The toggle's words: one name, its state in `aria-pressed`, and the tooltip saying what a press does now. */
export const FILL_NAME = 'Fill the window';
export const fillTitle = (filled: boolean): string => (filled ? 'Return to the page (Escape)' : 'Fill the window (Escape returns)');
/** The toggle's icon: arrows out to fill, arrows in to return. */
export const fillGlyph = (filled: boolean): string =>
  (filled ? 'M4 14h6v6M20 10h-6V4M14 10l7-7M3 21l7-7' : 'M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7');
