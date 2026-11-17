// Spec 02 section 3.1, Composer (D117: one context chip, "Message", Send, and Stop only while the agent works). The
// page context that rides with the next message is one chip, present only while the page has one the person has not
// removed, whose words describe the input; its × leaves the context out of the next message and the chip returns after
// that send. The status line holds only a refused or failed send. The mode follows the conversation's open turn:
// `sending` and `working` send nothing (in `working`, Send or Enter shows and speaks section 3.8's `turn-running`
// words); while a turn waits, a message answers its question in words or denies its approval (lean 6), which the card
// says after. Enter sends unless an input method is composing; the key rule itself is in Settings.

import { useState, type FormEvent, type KeyboardEvent } from 'react';
import type { PageContext } from '../protocol/page.ts';
import { useShell } from '../shell/context.ts';
import { composerMode } from './composerMode.ts';
import type { LogModel } from './journal.ts';
import type { ChatSession } from './session.ts';
import { pageContextWords, TURN_RUNNING } from './words.ts';

export function Composer({ session, model, context, onStop }: {
  session: ChatSession; model: LogModel; context: PageContext | null; onStop: (turn: string) => void;
}) {
  const shell = useShell();
  const [text, setText] = useState(() => session.unsent());
  const [leaveOut, setLeaveOut] = useState(false);
  /** The working turn a Send was pressed in: its refusal stands until that turn stops working. */
  const [refusedIn, setRefusedIn] = useState<string | null>(null);
  const sending = session.local.optimistic.some((held) => held.state === 'sending');
  const mode = composerMode(model, sending);
  const chip = leaveOut || context === null ? null : pageContextWords(context);
  const refused = mode.mode === 'working' && refusedIn === mode.turn;
  const status = session.status;
  const sends = mode.mode !== 'sending' && mode.mode !== 'working';

  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    if (mode.mode === 'working') {
      setRefusedIn(mode.turn);
      shell.announce(TURN_RUNNING);
      return;
    }
    const message = text.trim();
    if (!sends || message === '') return;
    setText('');
    setLeaveOut(false);
    const sent = await session.send(message, leaveOut ? null : context);
    if (!sent.ok) {
      setText(session.unsent());
      shell.announce(sent.words);
    }
  };

  const keys = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key !== 'Enter' || event.shiftKey) return;
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    event.preventDefault();
    void submit();
  };

  return (
    <form className="composer" id="composer" aria-label="Message the agent" onSubmit={(event) => void submit(event)}>
      {chip !== null && (
        <p className="composer__context" id="composer-context">
          {/* studio.css has no `.pill--wrap` rule yet, and a page title can outrun 320 CSS px (statement 16). */}
          <span className="pill pill--wrap composer__chip" style={{ whiteSpace: 'normal' }}>
            <span id="composer-context-words">{chip}</span>
            <button className="composer__chip-remove" type="button" aria-label="Leave the page context out" onClick={() => {
              setLeaveOut(true);
              document.getElementById('composer-input')?.focus();
            }}>×</button>
          </span>
        </p>
      )}
      <label className="visually-hidden" htmlFor="composer-input">Message the agent</label>
      <textarea className="composer__input" id="composer-input" name="message" placeholder="Message"
        aria-describedby={chip !== null ? 'composer-context-words composer-status' : 'composer-status'} value={text}
        onChange={(event) => {
          setText(event.currentTarget.value);
          session.keepUnsent(event.currentTarget.value);
        }}
        onKeyDown={keys} />
      <p className="composer__status" id="composer-status" data-tone={refused ? 'bad' : status?.tone ?? undefined}>
        {refused ? TURN_RUNNING : status?.words ?? ''}
      </p>
      {status?.retry && !refused && (
        <p><button className="btn btn--small" type="button" onClick={() => void session.retry()}>Try again</button></p>
      )}
      <div className="composer__row">
        {mode.mode === 'working' && (
          <button className="btn" type="button" data-focus-home="composer-input" onClick={() => onStop(mode.turn)}>Stop</button>
        )}
        <button className="btn btn--primary" type="submit" aria-disabled={sends ? undefined : 'true'}>Send</button>
      </div>
    </form>
  );
}
