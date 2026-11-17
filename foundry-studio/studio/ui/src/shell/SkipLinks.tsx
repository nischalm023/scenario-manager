// Spec 02 section 3.1, SkipLinks: the first Tab stops, "Skip to the panel" and, while the chat draws its message box,
// "Skip to the message box", which a loading or ended conversation does not draw, and which opens a folded chat
// first. The React port takes the click and moves focus itself, so the address gains no fragment and history no entry.

import type { MouseEvent } from 'react';
import { useShell } from './context.ts';

function skipToPanel(event: MouseEvent<HTMLAnchorElement>) {
  const target = document.getElementById('panel');
  if (target === null) return;
  event.preventDefault();
  target.focus();
}

export function SkipLinks({ messageBox }: { messageBox: boolean }) {
  const { focusComposer } = useShell();
  return (
    <div className="skip-links">
      <a href="#panel" onClick={skipToPanel}>Skip to the panel</a>{' '}
      {messageBox && (
        <a href="#composer-input" onClick={(event) => {
          event.preventDefault();
          focusComposer();
        }}>Skip to the message box</a>
      )}
    </div>
  );
}
