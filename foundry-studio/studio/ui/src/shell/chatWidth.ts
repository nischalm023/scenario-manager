// The agent chat drawer's width, which the person sets by moving its edge (`ChatDrawer`): kept in this browser, and
// shown between a least and a most width, never more than half the window, so the work beside it keeps its share. The
// width shown and its bounds follow the window as it resizes, so the edge never says a width the drawer is not; the
// width the person set is kept, and shown again once the window has room for it.

import { useCallback, useEffect, useState } from 'react';
import { readStored, writeStored } from './storage.ts';

const KEY = 'studio.chat-width';
/** CSS px: the width of tokens.css's `--chat-width` (22rem), the drawer's until the person moves its edge. */
const FIRST = 352;
const LEAST = 280;
const MOST = 800;

/** The drawer's width as shown, in CSS px; the least and the most it may be in this window; and a setter that keeps
 * the width in this browser once a drag or a key settles it. */
export interface ChatWidth {
  width: number;
  least: number;
  most: number;
  resize: (width: number, settled: boolean) => void;
}

export function useChatWidth(): ChatWidth {
  const [windowWidth, setWindowWidth] = useState(() => window.innerWidth);
  useEffect(() => {
    const follow = () => setWindowWidth(window.innerWidth);
    window.addEventListener('resize', follow);
    return () => window.removeEventListener('resize', follow);
  }, []);
  const most = Math.min(MOST, Math.max(LEAST, Math.floor(windowWidth / 2)));
  const bounded = useCallback((width: number) => Math.round(Math.min(Math.max(width, LEAST), most)), [most]);
  const [chosen, setChosen] = useState(() => {
    const kept = Number(readStored('local', KEY));
    return kept > 0 ? kept : FIRST;
  });
  const resize = useCallback((next: number, settled: boolean) => {
    const shown = bounded(next);
    setChosen(shown);
    if (settled) writeStored('local', KEY, String(shown));
  }, [bounded]);
  return { width: bounded(chosen), least: LEAST, most, resize };
}
