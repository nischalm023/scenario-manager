// The Inventory & orders page of Setup: a site's inventory and order profile, which lives with its scenarios, each
// setting its own: one card a scenario with its profile, then what a profile holds.

import type { FieldCatalogue, ScenarioRead, ScenarioSummary } from '../../protocol/scenario.ts';
import { Info, TipLine } from '../../shell/Info.tsx';
import { routeFor } from '../../shell/routes.ts';
import { ShellLink } from '../../shell/ShellLink.tsx';
import { useRead } from './read.ts';
import { Head, Status } from './setupPage.tsx';
import { grouped } from './words.ts';

const PROFILE_FIELDS: [string, string, (spec: Record<string, unknown>) => string][] = [
  ['orders_per_hour', 'Orders an hour', (spec) => grouped(Number(spec.orders_per_hour))],
  ['lines_per_order', 'Lines an order', (spec) => String(spec.lines_per_order)],
  ['sku_count', 'SKUs', (spec) => grouped(Number(spec.sku_count))],
  ['inventory_fill_pct', 'Inventory fill', (spec) => `${spec.inventory_fill_pct}%`],
  ['units_per_stored_tote', 'Units a tote', (spec) => String(spec.units_per_stored_tote)],
  ['release', 'Release', (spec) => String(spec.release).replace(/_/g, ' ')],
];

/** One scenario's profile, read from its latest version. */
function ScenarioProfile({ scenario }: { scenario: ScenarioSummary }) {
  const read = useRead<ScenarioRead>(`/api/scenarios/${scenario.id}`);
  const spec = read.state === 'ready' ? (read.value.version.content.workload.generator as unknown as Record<string, unknown>) : null;
  return (
    <section className="setup-card">
      <div className="setup-card__head">
        <h3 className="setup-card__title"><ShellLink to={routeFor({ view: 'scenario', id: scenario.id })}>{scenario.name}</ShellLink></h3>
        <span className="pill" data-tone="info">version {scenario.latest_version}</span>
      </div>
      {spec === null ? <p className="muted" aria-busy={read.state === 'loading'}>{read.state === 'failed' ? `Not read: ${read.reason}` : 'Reading…'}</p> : (
        <ul className="stat-row">{PROFILE_FIELDS.map(([key, label, show]) => <li key={key}><b>{show(spec)}</b> {label.toLowerCase()}</li>)}</ul>
      )}
    </section>
  );
}

/** The inventory and order profile: one card per scenario, then what a profile holds. */
function OrdersSection({ site, scenarios }: { site: string; scenarios: ScenarioSummary[] }) {
  const catalogue = useRead<FieldCatalogue>('/api/scenario-fields');
  return (
    <section className="setup-page orders-section" aria-labelledby="orders-title">
      <div className="home__section">
        <h3 className="home__heading" id="orders-title">Inventory &amp; order profile <Info label="the profile"><TipLine>Orders and starting inventory are generated per scenario from these values. Open a scenario to change them.</TipLine></Info></h3>
      </div>
      {scenarios.length === 0 ? (
        <p className="muted">Each scenario of {site.split('/').pop()} sets its own orders an hour, lines an order, SKUs and inventory fill. Create one above and its profile shows here.</p>
      ) : (
        <div className="setup-grid">{scenarios.map((scenario) => <ScenarioProfile key={scenario.id} scenario={scenario} />)}</div>
      )}
      <details className="kin-group">
        <summary>What a profile holds</summary>
        <Status read={catalogue} what="the profile fields">
          {(fields) => (
            <dl className="setup-facts setup-facts--columns">
              {fields.items.filter((field) => field.pointer.startsWith('/workload/')).map((field) => (
                <div className="setup-fact" key={field.pointer}><dt>{field.label}</dt><dd><span className="muted">{field.hint}</span></dd></div>
              ))}
            </dl>
          )}
        </Status>
      </details>
    </section>
  );
}

/** The `?panel=orders` address still answers: the same section, with the page's head. */
export function OrdersPanel({ site }: { site: string }) {
  const scenarios = useRead<{ items: ScenarioSummary[] }>(`/api/scenarios?site=${encodeURIComponent(site)}`);
  return (
    <div className="home setup-page">
      <Head icon={<><path d="M21 8l-9-5-9 5 9 5 9-5z" /><path d="M3 8v8l9 5 9-5V8" /><path d="M12 13v8" /></>} title="Inventory & orders" words="Set per scenario; this page reads them." />
      <Status read={scenarios} what="the scenarios">{(list) => <OrdersSection site={site} scenarios={list.items} />}</Status>
    </div>
  );
}
