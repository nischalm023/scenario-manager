// Spec 02 section 3.3, step 13, and section 3.1's ChatColumn: the route's conversation (ruling D94). The tab posts
// `POST /api/conversations { site }` for a route of a site, or `{}` on a new-site route, and reads the conversation
// it answers; the address never names it. Its session is made once the browser tab's id has settled, and made again
// when that id changes (spec 03 section 3.3). On `conversation.changed` with `next` it follows to the site's next
// conversation, carrying the sentence that says why. Dismiss archives a site's conversation (or discards a new-site
// thread) and opens the next; "Earlier conversations" reads an archived one, read only, the address unchanged.

import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import type { Conversation, ConversationList, ConversationSummary, Dismissed, OpenedConversation } from '../protocol/conversation.ts';
import { getJson, postJson } from '../shell/api.ts';
import type { ChatTarget } from '../shell/routes.ts';
import type { SystemItem } from './log.ts';
import { ChatSession } from './session.ts';
import type { StreamListener } from './stream.ts';

export interface ChatHandle {
  /** The live conversation's session, once opened. */
  session: ChatSession | null;
  /** An archived conversation being read, else null. */
  viewing: ChatSession | null;
  /** Why the conversation could not be opened, in the chat's words. */
  failure: string | null;
  archived: ConversationSummary[];
  view(conversation: string | null): void;
  /** Dismiss (or Discard): the outcome's words, `ok` when it went. */
  dismiss(): Promise<{ ok: boolean; words: string }>;
  version: number;
}

/**
 * Whether the chat draws its message box (`#composer-input`): a conversation read, live, and not an archived one being
 * read; while it loads, or for a conversation that ended, the chat draws no box, and nothing may point at one.
 */
export function drawsComposer(chat: ChatHandle): boolean {
  const model = (chat.viewing ?? chat.session)?.stream.model ?? null;
  return model !== null && chat.viewing === null && model.conversation.state === 'live';
}

/** What the speech and the panel commands hear, for each session the hook makes; stable across renders. */
export type ListenerFor = (session: () => ChatSession | null) => StreamListener;

const targetKey = (target: ChatTarget): string | null =>
  target === null ? null : target.kind === 'site' ? `site:${target.site}` : 'thread';

export function useChat(target: ChatTarget, browserTab: string | null, listenerFor: ListenerFor): ChatHandle {
  const key = targetKey(target);
  const site = target?.kind === 'site' ? target.site : null;
  const [opened, setOpened] = useState<{ key: string; conversation: Conversation } | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [reopen, setReopen] = useState(0);
  const [viewingId, setViewingId] = useState<string | null>(null);
  const [archived, setArchived] = useState<ConversationSummary[]>([]);
  /** The sentence a followed conversation carries to the next one of the same target. */
  const [carried, setCarried] = useState<{ from: string; key: string | null; items: SystemItem[] } | null>(null);

  useEffect(() => {
    if (key === null) return;
    let current = true;
    void postJson<OpenedConversation>('/api/conversations', site !== null ? { site } : {}).then((answer) => {
      if (!current) return;
      if (answer.ok) {
        setFailure(null);
        setOpened({ key, conversation: answer.value.conversation });
      } else {
        setFailure(answer.status === 404 ? 'There is no such site.' : `The conversation could not be opened: ${answer.reason}`);
      }
    });
    return () => {
      current = false;
    };
  }, [key, site, reopen]);

  const conversationId = opened !== null && opened.key === key ? opened.conversation.id : null;

  const session = useMemo(() => {
    if (conversationId === null || browserTab === null) return null;
    const heard = listenerFor(() => made);
    const made: ChatSession = new ChatSession(conversationId, browserTab, {
      ...heard,
      follow: (changed, entry) => {
        heard.follow?.(changed, entry);
        if (made.local.actedChanges.has(entry.seq)) return;
        setCarried({
          from: conversationId, key,
          items: [{
            type: 'system', key: `carried-${entry.seq}`, kind: 'conversation.changed', speaker: 'Agent Studio',
            text: `${changed.by.name} dismissed this conversation; a new one continues this site.`,
          }],
        });
        setReopen((count) => count + 1);
      },
    });
    return made;
  }, [conversationId, browserTab, listenerFor, key]);

  useEffect(() => {
    if (session === null) return;
    session.start();
    return () => session.stop();
  }, [session]);

  useEffect(() => {
    if (session !== null && carried !== null && carried.key === key && carried.from !== session.conversationId) session.carry(carried.items);
  }, [session, carried, key]);

  useEffect(() => {
    if (site === null || conversationId === null) return;
    let current = true;
    void getJson<ConversationList>(`/api/conversations?site=${encodeURIComponent(site)}`).then((answer) => {
      if (current && answer.ok) setArchived(answer.value.items.filter((item) => item.state === 'archived'));
    });
    return () => {
      current = false;
    };
  }, [site, conversationId]);

  // An archived conversation is read only: it posts nothing, so it carries no browser tab.
  const viewing = useMemo(() => (viewingId === null ? null : new ChatSession(viewingId, '', {}, { readOnly: true })), [viewingId]);
  useEffect(() => {
    if (viewing === null) return;
    viewing.start();
    return () => viewing.stop();
  }, [viewing]);

  const shown = viewing ?? session;
  const subscribe = useCallback((notify: () => void) => shown?.stream.subscribe(notify) ?? (() => undefined), [shown]);
  const version = useSyncExternalStore(subscribe, () => shown?.stream.version ?? -1);

  const dismiss = useCallback(async (): Promise<{ ok: boolean; words: string }> => {
    if (session === null) return { ok: false, words: 'There is no conversation to dismiss.' };
    session.markDismissing(true);
    const answer = await postJson<Dismissed>(`/api/conversations/${session.conversationId}/dismiss`, {});
    if (!answer.ok) {
      session.markDismissing(false);
      const busy = answer.problem?.type === 'turn-running';
      return { ok: false, words: busy ? "Dismiss is available once the agent's turn ends." : `Not dismissed: ${answer.reason}` };
    }
    if ('live' in answer.value) {
      setOpened({ key: key ?? '', conversation: answer.value.live });
      return { ok: true, words: 'Dismissed; a new conversation continues this site.' };
    }
    setReopen((count) => count + 1);
    return { ok: true, words: 'Discarded; nothing was created.' };
  }, [session, key]);

  return { session, viewing, failure, archived, version, dismiss, view: setViewingId };
}
