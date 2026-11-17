// Spec 02 section 3.1, PanelTabs: "the panel's last route in this browser tab", kept in `sessionStorage` per panel
// with the site it was shown for, so a tab's link returns to where the person left that panel on the same site.

import type { Panel } from '../protocol/panel.ts';
import { readStored, writeStored } from './storage.ts';

const LAST_ROUTES = 'agent-studio.last-routes';

export type LastRoutes = Partial<Record<Panel, { site: string | null; route: string }>>;

export function lastRoutes(): LastRoutes {
  try {
    return JSON.parse(readStored('session', LAST_ROUTES) ?? '{}') as LastRoutes;
  } catch {
    return {};
  }
}

/** Remember the route a panel last showed in this browser tab, for its tab's link. */
export function rememberRoute(panel: Panel, site: string | null, route: string): void {
  writeStored('session', LAST_ROUTES, JSON.stringify({ ...lastRoutes(), [panel]: { site, route } }));
}
