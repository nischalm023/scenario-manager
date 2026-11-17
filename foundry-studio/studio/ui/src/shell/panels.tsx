// Spec 02 section 6, the panel host contract: `PanelModule`, what each panel view gives the host (its title at once,
// the phrase the `h1` and `document.title` carry, the page context that rides with the next message, the site in
// context), and `ShellServices`, what the shell gives every panel. Until each panel's task lands, its module's content
// is "Not built yet" and no developer's note (D117, D82), so titles, tabs, page context and focus already work.

import { createContext, type ReactNode } from 'react';
import type { ChatSession } from '../chat/session.ts';
import type { ContextPanel, PageContext } from '../protocol/page.ts';
import type { ScopeSeq } from '../protocol/panel.ts';
import { printLocation, siteRootOf, type Location } from './grammar.ts';
import { routeFor, type Route } from './routes.ts';
import { Home } from './Home.tsx';
import { SettingsPanel } from './SettingsPanel.tsx';
import { DocsPanel } from '../docs/DocsPanel.tsx';
import { MapPanel } from '../panels/map/MapPanel.tsx';
import { ShellLink } from './ShellLink.tsx';
import { FlowPanel } from '../panels/flow/FlowPanel.tsx';
import { RunsPanel } from '../panels/runs/RunsPanel.tsx';
import { AgentsPanel } from '../panels/setup/AgentsPanel.tsx';
import { OrdersPanel } from '../panels/setup/OrdersPanel.tsx';
import { StationsPanel } from '../panels/setup/StationsPanel.tsx';
import { ScenarioList } from '../panels/scenario/ScenarioList.tsx';
import { ScenarioPanel } from '../panels/scenario/ScenarioPanel.tsx';
import { SiteTemplate } from '../panels/template/SiteTemplate.tsx';
import { TemplateDraftPanel } from '../panels/template/TemplateDraftPanel.tsx';
import { TemplateList } from '../panels/template/TemplateList.tsx';
import { TemplateView } from '../panels/template/TemplateView.tsx';

export interface PanelModule {
  panel: ContextPanel;
  /** The `h2` text, which answers at once, before the module loads. */
  title(route: string): string;
  /** What follows "Agent Studio, " in the `h1` and `document.title`. */
  pagePhrase(route: string): string;
  /** What rides with the next message (spec 03 section 3.4). */
  context(): PageContext;
  /** The site in context. */
  site(route: string): { location: string; name: string } | null;
  /** The panel host's state, and what it draws below the title once `ready` (not part of spec 02's interface). */
  state: 'loading' | 'ready' | 'error' | 'not-found';
  content: ReactNode;
}

export interface ShellServices {
  /** Section 3.4's rules: a panel speaks only the outcome of the user's own act in it, and failures. */
  announce(sentence: string): void;
  /**
   * A panel act that answers the agent (SPEC.md:125-126), its entry reading "Sent from the {panel} by {act}". It
   * resolves once accepted, focus then on the composer; a refusal is spoken by the shell and rejects with its words,
   * for the panel to show beside the control pressed (F4).
   */
  sendMessage(text: string, act: string): Promise<void>;
  browserTab(): string;
  /** Spec 03's check 3. */
  focusInPanel(): boolean;
  /** F6. */
  focusPanelTitle(): void;
  /** True when Settings says "Offer a link in the chat" (consolidated question 52, lead decided). */
  offersLinkForShow(): boolean;
  /**
   * Hear the conversation stream's items of `kinds` as they arrive: live-only items (`template.evaluation`), and
   * journal entries once the stream is live. The answer stops hearing them (brief I1.07b; spec 05 section 3.10).
   */
  subscribe(kinds: readonly string[], handler: (kind: string, body: unknown) => void): () => void;
  /** An object's panel names the object's site once it has read it (spec 03 section 3.2), and null as it goes. */
  nameSite(named: NamedSite | null): void;
}

/** A scenario's, run's or comparison's site, whose conversation the chat column shows, and the cut the panel shows. */
export interface NamedSite {
  site: string;
  seen: ScopeSeq[];
}

/** What a panel's content reads to reach the shell (spec 02 section 6). */
export const ServicesContext = createContext<ShellServices | null>(null);

/**
 * `ShellServices.sendMessage`: the panel act's message through the page's session, its entry naming the panel and
 * the act; once accepted, focus goes to the composer, as after answering a card (spec 03 section 3.3; F3); a refusal
 * is said (A12) and rejects with its words, for the panel to show beside the control pressed, where focus stays (F4).
 */
export async function sendPanelMessage(
  session: ChatSession | null, text: string, act: string, context: PageContext,
  shell: { focusComposer(): void; say(sentence: string): void },
): Promise<void> {
  const outcome = session === null
    ? { ok: false as const, words: 'There is no conversation on this page to send it to.' }
    : await session.send(text, context, { source: 'panel', act });
  if (outcome.ok) {
    shell.focusComposer();
    return;
  }
  shell.say(outcome.words);
  throw new Error(outcome.words);
}

const PANEL_WORD: Record<'map' | 'flow' | 'template' | 'scenario' | 'runs', string> = {
  map: 'Map', flow: 'Flow', template: 'Template', scenario: 'Scenario', runs: 'Runs',
};

/** "map at changeset 1", "following branch main", "at tag v3": a location's pin in words. */
export function pinWords(location: Location): string {
  const scope = location.scope.split('/').pop() ?? location.scope;
  switch (location.pin.kind) {
    case 'changeset': return `${scope} at changeset ${location.pin.seq}`;
    case 'tag': return `at tag ${location.pin.name}`;
    case 'branch': return `following branch ${location.pin.name}`;
  }
}

/** Spec 02 section 3.1, PanelHost: a panel not built yet. */
const NOT_BUILT: ReactNode = <p className="panel__status">Not built yet</p>;

interface Shape {
  panel: ContextPanel;
  title: string;
  phrase: string;
  contextTitle: string;
  site: string | null;
  state?: PanelModule['state'];
  content: ReactNode;
}

function shapeOf(route: Route, modelLabel: string | null): Shape {
  switch (route.view) {
    case 'settings':
      return {
        // Settings is nobody's panel: it reports Home as its context, the page context a route without a chat carries.
        panel: 'home', title: 'Settings', phrase: 'Settings', contextTitle: 'Settings', site: null,
        content: <SettingsPanel modelLabel={modelLabel} />,
      };
    case 'docs':
      return { panel: 'home', title: 'Docs', phrase: route.doc === '' ? 'Docs' : `Docs: ${route.doc}`, contextTitle: 'Docs', site: null, content: <DocsPanel doc={route.doc} /> };
    case 'home':
      return { panel: 'home', title: 'Sites', phrase: 'Home: make a new site or open one', contextTitle: 'Home', site: null, content: <Home /> };
    case 'templates':
      return {
        panel: 'templates', title: 'Maps', phrase: 'a new site: choosing a map', contextTitle: 'Map presets',
        site: null, content: <TemplateList />,
      };
    case 'template-draft':
      return {
        panel: 'template', title: 'Configure map', phrase: 'a new site: configuring its map',
        contextTitle: `Map draft ${route.draft}`, site: null, content: <TemplateDraftPanel key={route.draft} id={route.draft} />,
      };
    case 'scenarios':
      return {
        panel: 'scenario', title: 'Scenario', phrase: route.site ? `the scenarios of ${route.site}` : 'the scenarios',
        contextTitle: 'Scenario list', site: route.site ?? null, content: <ScenarioList site={route.site} />,
      };
    case 'scenario':
      return {
        panel: 'scenario', title: 'Scenario', phrase: `scenario ${route.id}`, contextTitle: `Scenario ${route.id}`,
        site: null, content: <ScenarioPanel key={`${route.id} ${route.version ?? ''}`} id={route.id} version={route.version} />,
      };
    case 'runs':
      return {
        panel: 'runs', title: 'Runs', phrase: route.site ? `the runs of ${route.site}` : 'your runs', contextTitle: 'Runs list',
        site: route.site ?? null,
        // One key for the list and a run's page, so a filter's radio keeps focus as the route moves between them.
        content: <RunsPanel key="runs" site={route.site} filter={route.filter} submission={route.submission} highlights={route.highlights} />,
      };
    case 'run':
      return {
        panel: 'runs', title: 'Runs', phrase: `run ${route.id}`, contextTitle: `Run ${route.id}`, site: null,
        content: <RunsPanel key="runs" open={route.id} highlights={route.highlights} />,
      };
    case 'compare':
      return {
        panel: 'runs', title: 'Runs', phrase: `a comparison of ${route.ids.length} runs`, contextTitle: `Comparison of ${route.ids.length} runs`,
        site: null, content: <RunsPanel key="runs" compare={{ ids: route.ids, reference: route.reference }} />,
      };
    case 'unreadable':
      return {
        panel: 'home', title: 'Nothing here', phrase: 'an address that names nothing', contextTitle: 'Nothing here', site: null,
        state: 'not-found',
        content: (
          <>
            <p>This address names nothing Agent Studio can read: {route.why}</p>
            <p><ShellLink to="/">Go to the sites</ShellLink></p>
          </>
        ),
      };
    case 'world': return worldShape(route);
  }
}

function worldShape(route: Extract<Route, { view: 'world' }>): Shape {
  const site = siteRootOf(route.location.scope);
  const where = `${route.location.scope}, ${pinWords(route.location)}`;
  const at = (panel: string) => `${site ?? route.location.scope}: the ${panel}, ${pinWords(route.location)}`;
  switch (route.show) {
    case 'map':
      return { panel: 'map', title: 'Map', phrase: at('Map panel'), contextTitle: `Map: ${where}`, site, content: <MapPanel key={printLocation(route.location)} route={route} /> };
    case 'flow':
      return { panel: 'flow', title: 'Flow', phrase: at('Flow panel'), contextTitle: `Flow: ${where}`, site, content: <FlowPanel route={route} /> };
    case 'site-template':
      return {
        panel: 'template', title: 'Template', phrase: at('Template panel'), contextTitle: `Template: ${where}`, site,
        content: site === null ? null : <SiteTemplate site={site} />,
      };
    case 'template':
      return {
        panel: 'template', title: 'Template', phrase: `the template ${route.location.scope}`, contextTitle: `Template: ${printLocation(route.location)}`,
        site: null, content: <TemplateView template={route.location.scope} />,
      };
    case 'history':
      return {
        panel: 'history', title: 'History', phrase: `the history of ${route.location.scope}`, contextTitle: `History: ${where}`, site,
        content: NOT_BUILT,
      };
    // The Setup pages that read what a site already holds: the chat's context names the panel their data comes from.
    case 'agents':
      return { panel: 'flow', title: 'Agents', phrase: at('Agents page'), contextTitle: `Agents: ${where}`, site, content: site === null ? null : <AgentsPanel site={site} route={route} /> };
    case 'stations':
      return { panel: 'flow', title: 'Stations', phrase: at('Stations page'), contextTitle: `Stations: ${where}`, site, content: site === null ? null : <StationsPanel site={site} route={route} /> };
    case 'orders':
      return { panel: 'scenario', title: 'Inventory & orders', phrase: at('Inventory and orders page'), contextTitle: `Inventory & orders: ${where}`, site, content: site === null ? null : <OrdersPanel site={site} /> };
    case 'fleet':
      return {
        panel: 'scenario', title: 'Fleet', phrase: `${site}: the fleet`, contextTitle: `Fleet: ${where}`, site,
        content: (
          <p>
            No panel shows the fleet scope in the proof of concept; its agent overrides show on the{' '}
            <ShellLink to={routeFor({ view: 'scenarios', ...(site ? { site } : {}) })}>Scenario panel</ShellLink>.
          </p>
        ),
      };
    case 'none':
      return {
        panel: 'history', title: 'No panel', phrase: `${route.location.scope}: no panel shows this scope`, contextTitle: `No panel: ${where}`, site,
        content: (
          <p>
            No panel shows this scope in the proof of concept.{' '}
            <ShellLink to={routeFor({ ...route, show: 'history', kin: undefined, select: undefined, highlights: undefined })}>Its history</ShellLink>
          </p>
        ),
      };
  }
}

/** The module of a route's view: "Not built yet" until the panel's task replaces it. */
export function moduleFor(
  route: Route, siteName: (site: string) => string, named: NamedSite | null = null, modelLabel: string | null = null,
): PanelModule {
  const shape = shapeOf(route, modelLabel);
  const site = shape.site ?? named?.site ?? null;
  const address = routeFor(route);
  const selection = 'select' in route && route.select !== undefined ? { key: route.select, label: route.select } : null;
  const highlights = 'highlights' in route && route.highlights !== undefined ? route.highlights : null;
  return {
    panel: shape.panel,
    title: () => shape.title,
    pagePhrase: () => shape.phrase,
    context: () => ({ route: address, panel: shape.panel, title: shape.contextTitle, seen: named?.seen ?? [], selection, highlights }),
    site: () => (site === null ? null : { location: site, name: siteName(site) }),
    state: shape.state ?? 'ready',
    content: shape.content,
  };
}

export const panelWord = (panel: keyof typeof PANEL_WORD): string => PANEL_WORD[panel];

/** "map at changeset 1" for a pin given as a changeset of a scope path, as the chat's links word it. */
export function changesetWords(scope: string, seq: number): string {
  return `${scope.split('/').pop() ?? scope} changeset ${seq}`;
}

