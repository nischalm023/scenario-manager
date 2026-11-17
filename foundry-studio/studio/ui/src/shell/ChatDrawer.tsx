// The agent chat's drawer on the right of the work: its tab, which folds the chat away and opens it again, and its
// left edge, which resizes it. The chat stays mounted while folded, so its draft survives; folded, the tab says what
// waits on the person there (a question to answer, an approval to decide), since the card that asks is out of sight.
// The edge is a focusable `separator` (the window splitter pattern), so a screen reader names it and reads its value:
// the person drags it, or moves it with the arrow keys; moving it left widens the drawer, Home makes the drawer its
// narrowest and End its widest in this window. The drawer hands the edge to the chat it holds, to stand inside the
// chat's landmark: everything on a page belongs to one.

import { useRef, type CSSProperties, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react';
import { useChatWidth, type ChatWidth } from './chatWidth.ts';

const STEP = 16;

export function ChatDrawer({ folded, waiting, toggle, children }: {
  folded: boolean;
  /** What waits on the person in the chat, in words ("a question waits for you"), or null for nothing. */
  waiting: string | null;
  toggle: () => void;
  /** The chat, given the drawer's edge to stand in it; none while folded. */
  children: (edge: ReactNode) => ReactNode;
}) {
  const size = useChatWidth();
  const name = folded ? `Open the agent chat${waiting !== null ? `: ${waiting}` : ''}` : 'Fold the agent chat away';
  return (
    <div className="chat-drawer" style={{ '--chat-width': `${size.width}px` } as CSSProperties}>
      <button className="chat-drawer__tab" id="chat-toggle" type="button" onClick={toggle} aria-expanded={!folded} aria-controls="chat"
        title={name} aria-label={name} data-waiting={folded && waiting !== null ? 'true' : undefined}>
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
          <path d="M21 12a8 8 0 0 1-8 8H7l-4 3V12a8 8 0 0 1 8-8h2a8 8 0 0 1 8 8z" />
        </svg>
        <span className="chat-drawer__word">Agent chat</span>
        {folded && waiting !== null && <span className="chat-drawer__waiting" aria-hidden="true">!</span>}
      </button>
      {children(folded ? null : <Edge {...size} />)}
    </div>
  );
}

function Edge({ width, least, most, resize }: ChatWidth) {
  const drag = useRef<{ from: number; width: number } | null>(null);
  const dragged = (event: PointerEvent<HTMLDivElement>, settled: boolean) => {
    const start = drag.current;
    if (start === null) return;
    resize(start.width + start.from - event.clientX, settled);
    if (settled) drag.current = null;
  };
  const key = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? STEP * 4 : STEP;
    const next = { ArrowLeft: width + step, ArrowRight: width - step, Home: least, End: most }[event.key];
    if (next === undefined) return;
    event.preventDefault();
    resize(next, true);
  };
  return (
    <div className="chat-drawer__edge" role="separator" aria-orientation="vertical" aria-controls="chat" tabIndex={0}
      aria-label="Width of the agent chat" aria-valuemin={least} aria-valuemax={most} aria-valuenow={width}
      title="Drag, or use the arrow keys, to resize the agent chat"
      onPointerDown={(event) => {
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = { from: event.clientX, width };
      }}
      onPointerMove={(event) => dragged(event, false)} onPointerUp={(event) => dragged(event, true)}
      onPointerCancel={(event) => dragged(event, true)} onKeyDown={key} />
  );
}
