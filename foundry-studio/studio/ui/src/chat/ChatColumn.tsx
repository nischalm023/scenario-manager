// Spec 02 section 3.1, ChatColumn: `aside.chat#chat` "Agent chat", the drawer's resizing edge the shell hands it, its
// header, the log and the composer, or the read-only line that replaces it. The header holds the title (the site's
// name, or "New site") and one action, Dismiss or Discard, beside the connection words while the stream is not live and
// the archived conversations (D117): the model's name is in Settings, and nothing points at a waiting card, since the
// card is the answer.

import { useMemo, useState, type ReactNode } from 'react';
import type { ConversationSummary } from '../protocol/conversation.ts';
import type { PageContext } from '../protocol/page.ts';
import { useShell } from '../shell/context.ts';
import { ChatContext, type ChatApi } from './chatContext.ts';
import { Composer } from './Composer.tsx';
import { openTurn } from './journal.ts';
import { drawLog } from './log.ts';
import { LogView } from './LogView.tsx';
import type { ChatSession } from './session.ts';
import type { ConnectionState } from './stream.ts';
import { drawsComposer, type ChatHandle } from './useChat.ts';

const CONNECTION_WORDS: Record<Exclude<ConnectionState, null>, string> = {
  connecting: 'Connecting to the agent',
  reconnecting: 'Reconnecting to the agent',
  offline: 'Offline: retrying every 30 s',
  'signed-out': 'Your sign-in has expired. Reload the page to sign in again.',
};

const day = new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short', year: 'numeric' });
const dateOf = (at: string | null): string => (at === null ? 'no entries' : day.format(new Date(at)));
const span = (summary: ConversationSummary): string => `${dateOf(summary.first_at)} to ${dateOf(summary.last_at)}`;

/**
 * `siteName` is the site's name as the shell knows it (the crumbs' and the context bar's): a conversation a site route
 * opened carries no `title` of its own (spec 01 section 3.17), only a new-site thread once Create site binds it.
 */
export function ChatColumn({ chat, context, siteName, edge = null }: {
  chat: ChatHandle; context: PageContext | null; siteName: string | null;
  /** The drawer's resizing edge (`ChatDrawer`), stood inside this landmark. */
  edge?: ReactNode;
}) {
  const shell = useShell();
  const shown: ChatSession | null = chat.viewing ?? chat.session;
  const model = shown?.stream.model ?? null;
  const reading = chat.viewing !== null;
  const [dismissError, setDismissError] = useState<string | null>(null);
  const items = useMemo(() => (model === null || shown === null ? [] : drawLog(model, shown.local)),
    // The stream's version moves whenever the model or this tab's own state changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [model, shown, chat.version]);

  const api = useMemo<ChatApi | null>(() => {
    if (shown === null || model === null) return null;
    return {
      me: shell.me, model, readOnly: reading || model.conversation.state !== 'live',
      answer: (question, answers) => shown.answer(question, answers),
      decide: (approval, decision, reason) => shown.decide(approval, decision, reason),
      attribute: (approval, wasThis) => shown.attribute(approval, wasThis),
      apply: (draft) => shown.apply(draft),
      discard: (draft) => shown.discard(draft),
      cancel: (card) => shown.cancel(card),
      send: async (text) => {
        const sent = await shown.send(text, context);
        if (!sent.ok) shell.announce(sent.words);
      },
      // F8: focus moves to the log only when the button gives way to "Start of the conversation"; before that it stays
      // on the button, which is still there.
      loadEarlier: async () => {
        if (await shown.loadEarlier()) document.getElementById('chat-log')?.focus();
      },
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shown, model, shell, reading, context, chat.version]);

  const conversation = model?.conversation ?? null;
  const thread = conversation !== null && conversation.site === null;
  const busy = model !== null && openTurn(model) !== null;
  const connection = shown?.stream.connection ?? null;
  const viewedSummary = reading ? chat.archived.find((summary) => summary.id === chat.viewing?.conversationId) : undefined;
  const title = reading
    ? `Earlier conversation, ${viewedSummary ? span(viewedSummary) : 'read only'}`
    : thread ? 'New site' : conversation?.title ?? siteName ?? conversation?.site ?? 'New site';
  const dismissWord = thread ? 'Discard' : 'Dismiss';
  const dismissWhy = thread
    ? "Discarding drops this new site's template draft and conversation; nothing has been created."
    : 'Dismissing archives this conversation and starts a new one for the site.';

  const dismiss = async () => {
    if (busy) {
      shell.announce("Dismiss is available once the agent's turn ends.");
      return;
    }
    const outcome = await chat.dismiss();
    setDismissError(outcome.ok ? null : outcome.words);
    if (outcome.ok) shell.focusComposer();
    shell.announce(outcome.words);
  };

  const stop = async (turn: string) => {
    if (shown === null) return;
    const outcome = await shown.stopTurn(turn);
    if (!outcome.ok) {
      shown.fail(outcome.words);
      shell.announce(outcome.words);
    }
  };

  return (
    <aside className="chat" id="chat" aria-label="Agent chat">
      {edge}
      <div className="chat__header">
        <h2 className="chat__title" id="chat-title">{title}</h2>
        <p className="chat__meta">
          {connection !== null && <span className="chat__connection" data-state={connection}>{CONNECTION_WORDS[connection]}</span>}{' '}
          {!reading && conversation !== null && conversation.state === 'live' && (
            <>
              <button className="btn btn--small btn--quiet chat__dismiss" type="button" aria-describedby="chat-dismiss-desc" data-focus-home="composer-input"
                aria-disabled={busy ? 'true' : undefined} onClick={() => void dismiss()}>
                {dismissWord}
              </button>
              <span className="visually-hidden" id="chat-dismiss-desc">
                {dismissWhy}{busy ? ` ${dismissWord} is available once the agent's turn ends.` : ''}
                {dismissError !== null ? ` ${dismissError}` : ''}
              </span>
            </>
          )}
        </p>
        {chat.archived.length > 0 && (
          <details className="chat__history">
            <summary>Earlier conversations ({chat.archived.length})</summary>
            <ul>
              {chat.archived.map((summary) => (
                <li key={summary.id}>
                  <button className="btn btn--small btn--quiet" type="button" onClick={() => chat.view(summary.id)}>{span(summary)}</button>
                </li>
              ))}
            </ul>
          </details>
        )}
      </div>
      {api !== null && model !== null && shown !== null ? (
        <ChatContext.Provider value={api}>
          <LogView items={items} thread={thread} />
          {!drawsComposer(chat) ? (
            <div className="composer" id="composer">
              <p className="composer__status">
                {reading || model.conversation.state === 'archived' ? 'This conversation is archived; you can read it.' : "This new site's conversation was discarded; you can read it."}
              </p>
              {reading && <p><button className="btn btn--small" type="button" onClick={() => chat.view(null)}>Back to this site's conversation</button></p>}
            </div>
          ) : (
            <Composer key={shown.conversationId} session={shown} model={model} context={context} onStop={(turn) => void stop(turn)} />
          )}
        </ChatContext.Provider>
      ) : (
        <ol className="chat__log" id="chat-log" aria-label="Conversation" tabIndex={0}>
          <li className="entry entry--system" data-kind="status">
            <h3 className="entry__speaker">Agent Studio</h3>
            <div className="entry__body"><p>{chat.failure ?? shown?.stream.failure ?? 'Reading the conversation'}</p></div>
          </li>
        </ol>
      )}
    </aside>
  );
}
