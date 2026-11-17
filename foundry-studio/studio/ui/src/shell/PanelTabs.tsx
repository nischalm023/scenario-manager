// Spec 02 section 3.1, PanelTabs (lean 1): the sidebar's "Setup" section, a list of links, not a tablist, since every
// panel is a route; the shown panel's link carries `aria-current="page"`, and the tab an agent `show` opened in this
// browser tab carries `data-highlight="agent"` and "Opened by the agent" until the person navigates. Maps stands for
// two panels, a site's map and the template page saying what made the site (linked from the map), so either marks
// it. A disabled tab is a link marked
// `aria-disabled`, still in the Tab order, whose reason is its tooltip alone (D117): shown on hover and on keyboard
// focus, hidden by Escape until the pointer or focus comes back (WCAG 1.4.13). Each link goes to the panel's last
// route in this browser tab, else spec 03's default by context.

import { useEffect, useState, type ReactNode } from 'react';
import type { Panel } from '../protocol/panel.ts';
import { printPin } from './grammar.ts';
import { lastRoutes } from './lastRoutes.ts';
import { routeFor, type Route } from './routes.ts';
import { ShellLink } from './ShellLink.tsx';

type Entry = Panel | 'scripting' | 'agents' | 'stations' | 'orders';
const ORDER: { panel: Entry; label: string; hint: string; icon: ReactNode }[] = [
  { panel: 'template', label: 'Maps', hint: 'Layout, aisles, racks and stations', icon: <><path d="M12 2l9 5-9 5-9-5z" /><path d="M3 12l9 5 9-5" /><path d="M3 17l9 5 9-5" /></> },
  { panel: 'orders', label: 'Inventory & orders', hint: 'Item master and order stats', icon: <><path d="M21 8l-9-5-9 5 9 5 9-5z" /><path d="M3 8v8l9 5 9-5V8" /><path d="M12 13v8" /></> },
  { panel: 'agents', label: 'Agents', hint: 'Fleet types and kinematics', icon: <><path d="M13 2L4 14h7l-1 8 9-12h-7z" /></> },
  { panel: 'flow', label: 'Process flow', hint: 'Routing and task sequencing', icon: <><circle cx="12" cy="12" r="3" /><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1" /></> },
  { panel: 'stations', label: 'Stations', hint: 'Station and staff shift config', icon: <><circle cx="12" cy="8" r="4" /><path d="M4 21a8 8 0 0 1 16 0" /></> },
  { panel: 'scenario', label: 'Scenarios', hint: 'Workload and run controls', icon: <><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" /></> },
  { panel: 'runs', label: 'Sim runs', hint: 'Results and comparisons', icon: <><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></> },
  { panel: 'scripting', label: 'Scripting', hint: 'After the PoC', icon: <><path d="M8 6l-5 6 5 6M16 6l5 6-5 6M14 4l-4 16" /></> },
];

/** The entry a panel is shown under: Maps holds a site's map as well as the template page of what made it. */
const entryOf = (panel: Panel): Entry => (panel === 'map' ? 'template' : panel);

const Icon = ({ children }: { children: ReactNode }) => (
  <svg className="panel-tabs__icon" viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{children}</svg>
);

/** Spec 03's default route of a panel for the site in context and the route shown. */
function defaultHref(panel: Exclude<Entry, 'scripting'>, route: Route, site: string | null): string | null {
  const worldPin = route.view === 'world' && route.location.pin.kind !== 'changeset' ? printPin(route.location.pin) : 'main';
  switch (panel) {
    case 'agents': case 'stations': case 'orders': return site === null ? null : `/w/${site}@${worldPin}?panel=${panel}`;
    case 'map': case 'flow': return site === null ? null : `/w/${site}/${panel}@${worldPin}`;
    // Maps: choosing and configuring a map before the site exists; the site's floor, with what made it, once it does.
    case 'template':
      if (route.view === 'template-draft') return routeFor({ view: 'template-draft', draft: route.draft });
      return site === null ? '/templates' : `/w/${site}/map@${worldPin}`;
    case 'scenario':
      if (route.view === 'scenario') return routeFor({ view: 'scenario', id: route.id });
      return routeFor({ view: 'scenarios', ...(site !== null ? { site } : {}) });
    // A site's Sim runs are its own runs; with no site, every run of the person's.
    case 'runs': return site === null ? '/runs' : routeFor({ view: 'runs', site });
  }
}

function DisabledTab({ panel, label, hint, icon, why }: { panel: string; label: string; hint: string; icon: ReactNode; why: string }) {
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const shown = (hovered || focused) && !dismissed;
  useEffect(() => {
    if (!shown) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setDismissed(true);
    };
    document.addEventListener('keydown', escape);
    return () => document.removeEventListener('keydown', escape);
  }, [shown]);
  const id = `panel-tab-${panel}-why`;
  return (
    <li onMouseEnter={() => { setHovered(true); setDismissed(false); }} onMouseLeave={() => setHovered(false)}>
      <a className="panel-tabs__tab" role="link" aria-disabled="true" tabIndex={0} data-panel={panel} aria-describedby={id}
        onFocus={() => { setFocused(true); setDismissed(false); }} onBlur={() => setFocused(false)}>
        <Icon>{icon}</Icon>
        <span className="panel-tabs__text"><span className="panel-tabs__label">{label}</span><span className="panel-tabs__hint">{hint}</span></span>
      </a>
      <span className="panel-tabs__why" role="tooltip" id={id} hidden={!shown}>{why}</span>
    </li>
  );
}

export function PanelTabs({ route, current, site, agentOpened }: {
  route: Route; current: Panel | null; site: string | null; agentOpened: Panel | null;
}) {
  const remembered = lastRoutes();
  return (
    <nav className="panel-tabs" id="panel-tabs" aria-label="Panels">
      <p className="panel-tabs__heading">Setup</p>
      <ul className="panel-tabs__list">
        {ORDER.map(({ panel, label, hint, icon }) => {
          if (panel === 'scripting') return <DisabledTab key={panel} panel={panel} label={label} hint={hint} icon={icon} why="After the PoC" />;
          const setupPage = panel === 'agents' || panel === 'stations' || panel === 'orders';
          const last = setupPage ? undefined : remembered[panel === 'template' && site !== null ? 'map' : panel];
          const usable = last !== undefined && last.site === site && (site !== null || panel !== 'flow');
          const href = usable ? last.route : defaultHref(panel, route, site);
          if (href === null) return <DisabledTab key={panel} panel={panel} label={label} hint={hint} icon={icon} why="Open a site first" />;
          const byAgent = agentOpened !== null && entryOf(agentOpened) === panel;
          return (
            <li key={panel}>
              <ShellLink className="panel-tabs__tab" to={href} data-panel={panel} data-route={href}
                aria-current={(current !== null && entryOf(current) === panel) || (route.view === 'world' && route.show === panel) ? 'page' : undefined}
                data-highlight={byAgent ? 'agent' : undefined}>
                <Icon>{icon}</Icon>
                <span className="panel-tabs__text"><span className="panel-tabs__label">{label}</span><span className="panel-tabs__hint">{byAgent ? 'Opened by the agent' : hint}</span></span>
              </ShellLink>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}
