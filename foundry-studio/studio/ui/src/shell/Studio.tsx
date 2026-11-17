// Spec 02 section 3.1, AppShell: the frame every route draws, `div.shell[data-chat]` with the app bar, the chat
// column when the route has a conversation (ruling D94; spec 03 section 3.2), and the workspace (context bar, tab row,
// panel host), after the skip links and before the announcer. It owns the address: the rewrite to the canonical form
// on load and after every navigation, pushes and replacements by spec 03's history rule, and `document.title`, which
// repeats the `h1`. Each conversation's stream feeds the announcer (spec 02 section 3.4) and the panel commands (spec 03
// section 3.3); the panel host hands its module the shell's services (spec 02 section 6).

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate, useNavigationType } from 'react-router';
import { announcer } from '../chat/announcer.ts';
import { ChatColumn } from '../chat/ChatColumn.tsx';
import { waitingCards } from '../chat/journal.ts';
import { speechListener } from '../chat/speech.ts';
import { combineListeners } from '../chat/stream.ts';
import { drawsComposer, useChat, type ListenerFor } from '../chat/useChat.ts';
import { waitingWords } from '../chat/words.ts';
import type { Me } from '../protocol/person.ts';
import type { Panel } from '../protocol/panel.ts';
import { AppBar } from './AppBar.tsx';
import { getJson } from './api.ts';
import { ChatDrawer } from './ChatDrawer.tsx';
import { browserTab, useBrowserTab } from './browserTab.ts';
import { applyHighlights, commandListener, highlightedKeys, noteUserNavigation } from './commands.ts';
import { ContextBar } from './ContextBar.tsx';
import { focusOnceDrawn, focusSoon, useFocusKeeper, useRegionKeys } from './focus.ts';
import { preferences } from './preferences.ts';
import { ShellContext, type NavigationCause, type Shell } from './context.ts';
import { PanelHost } from './PanelHost.tsx';
import { moduleFor, sendPanelMessage, ServicesContext, type NamedSite, type ShellServices } from './panels.tsx';
import { rememberRoute } from './lastRoutes.ts';
import { PanelTabs } from './PanelTabs.tsx';
import { readStored, writeStored } from './storage.ts';
import { canonical, chatTargetFor, panelOf, parseRoute, routeFor, viewIdentity } from './routes.ts';
import { SkipLinks } from './SkipLinks.tsx';

export function Studio() {
  const location = useLocation();
  const navigationType = useNavigationType();
  const routerNavigate = useNavigate();
  const address = `${location.pathname}${location.search}`;
  const route = useMemo(() => parseRoute(address), [address]);
  const [me, setMe] = useState<Me | null>(null);
  const [agentOpened, setAgentOpened] = useState<{ panel: Panel; view: string } | null>(null);
  /** The address a navigation of the person's is bound for, whose arrival moves focus to the panel title (F6). */
  const focusTitleAt = useRef<string | null>(null);
  const shown = useRef(address);
  const shownRoute = useRef(route);
  useLayoutEffect(() => {
    shown.current = address;
    shownRoute.current = route;
  }, [address, route]);

  // The rewrite keeps the address's fragment, the heading of a document it names.
  useEffect(() => {
    const target = routeFor(route);
    if (target !== address) void routerNavigate(`${target}${location.hash}`, { replace: true });
  }, [address, route, routerNavigate, location.hash]);

  useEffect(() => {
    void getJson<Me>('/api/me').then((answer) => {
      if (answer.ok) setMe(answer.value);
    });
  }, []);

  const focusPanelTitle = useCallback(() => document.getElementById('panel-title')?.focus(), []);
  // The chat is a drawer on the right of the work, folded to a tab by its own toggle (kept in this browser). A folded
  // chat draws nothing that can take focus, so whatever sends focus to its message box (the skip link, a panel's
  // message, Dismiss) opens it first.
  const [chatFolded, setChatFolded] = useState(() => readStored('local', 'studio.chat') === 'folded');
  const foldChat = useCallback((folded: boolean) => {
    writeStored('local', 'studio.chat', folded ? 'folded' : null);
    setChatFolded(folded);
  }, []);
  const chatFoldedRef = useRef(chatFolded);
  useLayoutEffect(() => {
    chatFoldedRef.current = chatFolded;
  }, [chatFolded]);
  const focusComposer = useCallback(() => {
    if (!chatFoldedRef.current) {
      focusSoon('composer-input');
      return;
    }
    foldChat(false);
    focusOnceDrawn('composer-input');
  }, [foldChat]);

  const navigate = useCallback((to: string, cause: NavigationCause) => {
    const hash = to.indexOf('#');
    const fragment = hash < 0 ? '' : to.slice(hash);
    const target = canonical(hash < 0 ? to : to.slice(0, hash));
    if (cause === 'within') {
      void routerNavigate(target, { replace: true });
      return;
    }
    if (cause === 'user') {
      noteUserNavigation();
      setAgentOpened(null);
    } else {
      const shownThere = parseRoute(target);
      const panel = panelOf(shownThere);
      setAgentOpened(panel === null ? null : { panel, view: viewIdentity(shownThere) });
    }
    // F6 moves focus once, on the person's own arrival; nothing is left pending for an agent's later `show` (F1, F7).
    focusTitleAt.current = cause === 'user' && target !== shown.current ? target : null;
    // The page already shown: a link naming a heading of it goes to that heading, as a fragment link does natively;
    // one naming none moves focus to the panel's title.
    if (target === shown.current && fragment === '') {
      if (cause === 'user') focusPanelTitle();
      return;
    }
    void routerNavigate(`${target}${fragment}`);
  }, [routerNavigate, focusPanelTitle]);

  useEffect(() => {
    const awaited = focusTitleAt.current;
    focusTitleAt.current = null;
    if (awaited === address) focusPanelTitle();
  }, [address, focusPanelTitle]);

  // Back and Forward are the person's own navigation: a later `show` of the turn becomes a link (check 3).
  useEffect(() => {
    if (navigationType === 'POP') noteUserNavigation();
  }, [address, navigationType]);

  const navigateRef = useRef(navigate);
  useLayoutEffect(() => {
    navigateRef.current = navigate;
  }, [navigate]);

  // The site an object's panel named for the view it shows (spec 03 section 3.2), ignored once another view shows.
  const [named, setNamed] = useState<{ view: string; named: NamedSite } | null>(null);
  const objectSite = named !== null && named.view === viewIdentity(route) ? named.named : null;
  const nameSite = useCallback((next: NamedSite | null) => setNamed((now) => {
    const view = viewIdentity(shownRoute.current);
    if (next === null) return now?.view === view ? null : now;
    return now?.view === view && JSON.stringify(now.named) === JSON.stringify(next) ? now : { view, named: next };
  }), []);
  // The agent chat shows only on a local dev build (`vite`, as dev.sh serves it): a built page (`vite build`, the
  // container) has no chat anywhere, as the Settings page has none, until the agent is ready for its users.
  const chatTarget = useMemo(() => (import.meta.env.DEV ? chatTargetFor(route, objectSite?.site ?? null) : null), [route, objectSite?.site]);
  // What panels follow on the stream through `ShellServices.subscribe`: each handler with the kinds it hears.
  const followers = useRef(new Set<{ kinds: readonly string[]; handler: (kind: string, body: unknown) => void }>());
  const tell = useCallback((kind: string, body: unknown) => {
    followers.current.forEach((one) => {
      if (one.kinds.includes(kind)) one.handler(kind, body);
    });
  }, []);
  // What each conversation's stream tells the rest of the tab: the announcer's rules and the panel commands.
  const listenerFor = useCallback<ListenerFor>((session) => combineListeners(
    speechListener({
      say: (sentence) => announcer().say(sentence),
      reading: () => preferences().replyReading,
      acted: (entry) => {
        const local = session()?.local;
        return local !== undefined && (local.actedChanges.has(entry.seq) || (entry.caused_by !== null && local.presses.has(entry.caused_by)));
      },
    }),
    commandListener({
      browserTab: () => browserTab().id(),
      route: () => shownRoute.current,
      navigate: (to, cause) => navigateRef.current(to, cause),
      say: (sentence) => announcer().say(sentence),
      offersLinkForShow: () => preferences().agentShow === 'link',
    }, session),
    {
      liveItem: tell,
      applied: (entry, _model, phase) => {
        if (phase === 'live') tell(entry.kind, entry.body);
      },
    },
  ), [tell]);
  const tab = useBrowserTab();
  const chat = useChat(chatTarget, tab, listenerFor);
  const bound = chat.session?.stream.model?.conversation ?? null;
  const boundSite = bound?.site ?? null;
  const boundTitle = bound?.title ?? null;
  // Settings names the model of the conversation the chat last showed (D117). The Settings page has no chat of its
  // own, so the label is held here for the page's life, in memory only: a reload names none, never an earlier
  // conversation's.
  const shownModel = chatTarget !== null ? (chat.viewing ?? chat.session)?.stream.model?.conversation.model_label ?? null : null;
  const [modelLabel, setModelLabel] = useState<string | null>(null);
  if (shownModel !== null && shownModel !== modelLabel) setModelLabel(shownModel);
  // What waits on the person in the conversation the chat shows: the folded chat's tab signs it, since the cards that
  // ask are then out of sight.
  const waiting = chat.viewing === null && chat.session?.stream.model ? waitingWords(waitingCards(chat.session.stream.model)) : null;
  const module = useMemo(
    () => moduleFor(route, (site) => (site === boundSite && boundTitle !== null ? boundTitle : site), objectSite, modelLabel),
    [route, boundSite, boundTitle, objectSite, modelLabel],
  );
  const pageContext = useMemo(() => module.context(), [module]);
  const site = module.site(address);
  const panel = panelOf(route);
  // The sidebar's own sections (Dashboard, Runs) carry their title in the page, so no site bar or panel tabs above them.
  const bare = route.view === 'home' || route.view === 'run' || route.view === 'compare' || route.view === 'templates' || route.view === 'template-draft'
    || (route.view === 'runs' && route.site === undefined);
  const phrase = module.pagePhrase(address);
  // "(opened by the agent)" stays through changes within the view the agent opened, and goes when the person
  // navigates: a link of theirs clears it, and Back or Forward (a POP) is theirs too.
  const agentPanel = agentOpened !== null && navigationType !== 'POP' && agentOpened.view === viewIdentity(route)
    ? agentOpened.panel : null;

  useEffect(() => {
    announcer().setTitle(`Agent Studio, ${phrase}`);
  }, [phrase]);

  useEffect(() => {
    if (panel !== null && route.view !== 'unreadable') rememberRoute(panel, site?.location ?? null, address);
  }, [panel, site?.location, address, route.view]);

  useFocusKeeper();
  useRegionKeys();

  // The highlights the address names, applied by `data-ref` to whatever the panel draws (spec 03 section 3.3).
  const highlights = 'highlights' in route ? route.highlights : undefined;
  const heldModel = chat.session?.stream.model ?? null;
  useEffect(() => {
    const root = document.getElementById('panel');
    if (root !== null) applyHighlights(root, highlightedKeys(heldModel, highlights));
  }, [heldModel, highlights, chat.version, address]);

  const services = useMemo<ShellServices>(() => ({
    announce: (sentence) => announcer().say(sentence),
    sendMessage: (text, act) => sendPanelMessage(chat.session, text, act, pageContext, {
      focusComposer, say: (sentence) => announcer().say(sentence),
    }),
    browserTab: () => browserTab().id() ?? '',
    focusInPanel: () => document.activeElement?.closest('main.panel') != null,
    focusPanelTitle,
    offersLinkForShow: () => preferences().agentShow === 'link',
    subscribe: (kinds, handler) => {
      const one = { kinds, handler };
      followers.current.add(one);
      return () => {
        followers.current.delete(one);
      };
    },
    nameSite,
  }), [chat.session, pageContext, focusPanelTitle, focusComposer, nameSite]);

  const shell = useMemo<Shell>(() => ({
    route, address, navigate, me, browserTab: tab ?? '', focusComposer, focusPanelTitle,
    announce: (sentence) => announcer().say(sentence),
  }), [route, address, navigate, me, tab, focusComposer, focusPanelTitle]);

  return (
    <ShellContext.Provider value={shell}>
      <SkipLinks messageBox={chatTarget !== null && drawsComposer(chat)} />
      <div className="shell" data-chat={chatTarget !== null ? 'present' : 'absent'}
        data-chat-folded={chatTarget !== null && chatFolded ? 'true' : undefined}>
        <AppBar pagePhrase={phrase} />
        {/* Setup belongs to a site: it shows once one is open, or while a new one is being made from a map preset. */}
        {(site !== null || route.view === 'templates' || route.view === 'template-draft') && (
          <div className="side-panel">
            <PanelTabs route={route} current={panel} site={site?.location ?? null} agentOpened={agentPanel} />
            {/* The site in context sits at the panel's foot: its name, where it is pinned, and its History; while a new
                site is being made, the map draft being configured, or that nothing is written until Create site. */}
            <ContextBar route={route} site={site} />
          </div>
        )}
        <div className="work">
          <ServicesContext.Provider value={services}>
            <PanelHost module={module} address={address} bare={bare} hideTitle={(route.view === 'world' && ['map', 'flow', 'agents', 'stations', 'orders'].includes(route.show)) || route.view === 'runs' || route.view === 'scenarios' || route.view === 'scenario'} />
          </ServicesContext.Provider>
        </div>
        {chatTarget !== null && (
          <ChatDrawer folded={chatFolded} waiting={waiting} toggle={() => foldChat(!chatFolded)}>
            {(edge) => <ChatColumn chat={chat} context={pageContext} siteName={site?.name ?? null} edge={edge} />}
          </ChatDrawer>
        )}
      </div>
    </ShellContext.Provider>
  );
}
