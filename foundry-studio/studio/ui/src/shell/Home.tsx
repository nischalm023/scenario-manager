// Spec 02 section 3.1, Home (`/`), ruling D94: "It is New Site or select a site." New site goes to the Template list,
// whose chat opens the person's new-site thread (spec 02 section 3.3, step 13: the route opens its conversation, once,
// and says why when it cannot); "Open a site" lists spec 01's `GET /api/sites`, newest change first, each name linked to
// its Map at `@main`, which brings up the site's live conversation. Home lists no conversations and no runs. The
// sites show as tiles or as a list of collapsible rows (the person's choice, kept in this browser) under a
// salutation and four counts, and a search box narrows them by name or reference.

import { useEffect, useMemo, useState } from 'react';
import type { SiteItem, SiteList } from '../protocol/site.ts';
import { getJson } from './api.ts';
import { useShell } from './context.ts';
import { ShellLink } from './ShellLink.tsx';
import { readStored, writeStored } from './storage.ts';

type Sites = { state: 'reading' } | { state: 'read'; items: SiteItem[] } | { state: 'failed'; reason: string };

function conversationWords(row: SiteItem): string {
  if (row.conversation?.waiting) return 'waiting for you';
  return row.conversation?.working ? 'the agent is working' : '';
}

/** "2 days ago": a wall-clock instant in words, beside its clock tag. */
function ago(at: string): string {
  const minutes = Math.round((Date.now() - Date.parse(at)) / 60000);
  if (!Number.isFinite(minutes)) return at;
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.round(hours / 24);
  return `${days} day${days === 1 ? '' : 's'} ago`;
}

type View = 'tiles' | 'list';

/** "customer/demo/site/l-shape-2" names customer "demo" and site "l-shape-2". */
function placeOf(path: string): { customer: string; site: string } {
  const match = /^customer\/([^/]+)\/site\/(.+)$/.exec(path);
  return match !== null ? { customer: match[1], site: match[2] } : { customer: '', site: path };
}

function salutation(): string {
  const hour = new Date().getHours();
  return `Good ${hour < 12 ? 'Morning' : hour < 17 ? 'Afternoon' : 'Evening'}!`;
}

const ICON = { width: 18, height: 18, viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', strokeWidth: 2, strokeLinecap: 'round', strokeLinejoin: 'round', 'aria-hidden': true } as const;
const WEEK_MS = 7 * 24 * 3600 * 1000;

function Badge({ site }: { site: SiteItem }) {
  return <span className="site-badge" aria-hidden="true">{(site.name ?? placeOf(site.site).site).charAt(0).toUpperCase()}</span>;
}

function mapOf(row: SiteItem) {
  return `/w/${row.site}/map@main`;
}

function Chips({ row }: { row: SiteItem }) {
  const words = conversationWords(row);
  const branches = row.branches?.length ?? 0;
  return (
    <>
      {words !== '' && <span className="site-chip">{words}</span>}
      <span className="site-chip site-chip--quiet">{branches} branch{branches === 1 ? '' : 'es'}</span>
      {row.changed_at !== '' && <span className="clock" data-clock="wall">{ago(row.changed_at)}</span>}
    </>
  );
}

function Detail({ row }: { row: SiteItem }) {
  const main = row.branches?.find((branch) => branch.name === 'main');
  const tags = row.tags ?? [];
  return (
    <div className="site-detail">
      <dl className="site-detail__facts">
        <div><dt>Customer</dt><dd>{placeOf(row.site).customer}</dd></div>
        <div><dt>Site</dt><dd>{placeOf(row.site).site}</dd></div>
        <div><dt>Reference</dt><dd><code>{row.site}</code></dd></div>
        <div><dt>Branches</dt><dd>{row.branches?.map((branch) => branch.name).join(', ') || 'none'}</dd></div>
        <div><dt>Map changeset on main</dt><dd>{main?.heads[`${row.site}/map`] ?? 'none'}</dd></div>
        <div><dt>Tags</dt><dd>{tags.length > 0 ? tags.join(', ') : 'none'}</dd></div>
        <div><dt>Last changed</dt><dd>{row.changed_at !== '' ? new Date(row.changed_at).toLocaleString() : 'never'}</dd></div>
        <div><dt>Conversation</dt><dd>{conversationWords(row) || (row.conversation === null ? 'not opened' : 'idle')}</dd></div>
      </dl>
      <div className="site-detail__actions">
        <ShellLink className="btn btn--pill btn--accent" to={mapOf(row)}>Open site</ShellLink>
        <ShellLink className="btn btn--pill" to={`/w/${row.site}@main?panel=history`}>History</ShellLink>
      </div>
    </div>
  );
}

function Tile({ row }: { row: SiteItem }) {
  const place = placeOf(row.site);
  const main = row.branches?.find((branch) => branch.name === 'main');
  const changeset = main?.heads[`${row.site}/map`];
  return (
    <li className="site-entry site-entry--tiles">
      <Badge site={row} />
      <div className="site-entry__text">
        <ShellLink className="site-entry__name" to={mapOf(row)}>{row.name ?? row.site}</ShellLink>
      </div>
      <dl className="site-entry__place">
        <div><dt>Customer</dt><dd>{place.customer}</dd></div>
        <div><dt>Site</dt><dd>{place.site}</dd></div>
      </dl>
      <p className="site-entry__facts">
        {changeset !== undefined && <span>Map changeset {changeset}</span>}
        {(row.tags?.length ?? 0) > 0 && <span>{row.tags.length} tag{row.tags.length === 1 ? '' : 's'}</span>}
      </p>
      <div className="site-entry__meta"><Chips row={row} /></div>
    </li>
  );
}

function Row({ row }: { row: SiteItem }) {
  const [open, setOpen] = useState(false);
  const detail = `detail-${row.site.replace(/[^\w-]/g, '-')}`;
  const name = row.name ?? row.site;
  const place = placeOf(row.site);
  return (
    <li className="site-entry site-entry--list" data-open={open ? 'true' : undefined}>
      <div className="site-entry__line">
        <button type="button" className="icon-btn icon-btn--ring site-entry__toggle" aria-expanded={open} aria-controls={detail}
          title={open ? `Hide ${name}` : `Show ${name}`} aria-label={open ? `Hide details of ${name}` : `Show details of ${name}`} onClick={() => setOpen(!open)}>
          <svg {...ICON}><path d={open ? 'M6 9l6 6 6-6' : 'M9 6l6 6-6 6'} /></svg>
        </button>
        <Badge site={row} />
        <div className="site-entry__text">
          <span className="site-entry__name">{name}</span>
          <span className="site-entry__ref">Customer {place.customer} · Site {place.site}</span>
        </div>
        <div className="site-entry__meta"><Chips row={row} /></div>
        <ShellLink className="icon-btn icon-btn--ring" to={mapOf(row)} title={`Open ${name}`} aria-label={`Open ${name}`}>
          <svg {...ICON}><path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5" /></svg>
        </ShellLink>
      </div>
      <div id={detail} hidden={!open}>{open && <Detail row={row} />}</div>
    </li>
  );
}

function Metric({ label, value, note }: { label: string; value: number | string; note: string }) {
  return (
    <div className="metric">
      <span className="metric__label">{label}</span>
      <span className="metric__value">{value}</span>
      <span className="metric__note">{note}</span>
    </div>
  );
}

export function Home() {
  const shell = useShell();
  const [sites, setSites] = useState<Sites>({ state: 'reading' });
  const [query, setQuery] = useState('');
  const [now] = useState(() => Date.now());
  const [view, setView] = useState<View>(() => (readStored('local', 'studio.home-view') === 'list' ? 'list' : 'tiles'));

  useEffect(() => {
    let current = true;
    void getJson<SiteList>('/api/sites').then((answer) => {
      if (!current) return;
      if (answer.ok) setSites({ state: 'read', items: answer.value.items });
      else setSites({ state: 'failed', reason: answer.reason });
    });
    return () => {
      current = false;
    };
  }, []);

  const choose = (next: View) => {
    setView(next);
    writeStored('local', 'studio.home-view', next);
  };

  const items = useMemo(() => (sites.state === 'read' ? sites.items : []), [sites]);
  const shown = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (needle === '') return items;
    return items.filter((row) => `${row.name ?? ''} ${row.site}`.toLowerCase().includes(needle));
  }, [items, query]);
  const total = items.length;
  const recent = items.filter((row) => row.changed_at !== '' && now - Date.parse(row.changed_at) < WEEK_MS).length;
  const working = items.filter((row) => row.conversation?.working === true).length;
  const waiting = items.filter((row) => row.conversation?.waiting).length;

  return (
    <div className="home">
      <header className="home__head">
        <div>
          <h3 className="home__title">{salutation()}</h3>
          <p className="home__sub">Open a site to see its map and talk to the agent, or start a new one from a template.</p>
        </div>
      </header>
      {sites.state === 'read' && (
        <div className="home__metrics">
          <Metric label="Total sites" value={total} note={total === 0 ? 'Create your first site' : 'In your workspace'} />
          <Metric label="Changed this week" value={recent} note="Sites with a new change in 7 days" />
          <Metric label="Agent working" value={working} note="Conversations in progress" />
          <Metric label="Waiting for you" value={waiting} note="Need your answer" />
        </div>
      )}
      <div className="home__section">
        <h3 className="home__heading">Recent sites</h3>
        <button className="btn btn--pill btn--accent" type="button" onClick={() => shell.navigate('/templates', 'user')}>
          <svg {...ICON}><path d="M12 5v14M5 12h14" /></svg>New site
        </button>
      </div>
      <div className="home__bar">
        <label className="home__search">
          <svg {...ICON}><circle cx="11" cy="11" r="7" /><path d="M20 20l-3.5-3.5" /></svg>
          <input type="search" placeholder="Search sites" aria-label="Search sites" value={query} onChange={(event) => setQuery(event.target.value)} />
        </label>
        <div className="home__views" role="group" aria-label="View">
          <button type="button" className="icon-btn" aria-pressed={view === 'tiles'} title="Tiles" aria-label="Tiles" onClick={() => choose('tiles')}>
            <svg {...ICON}><rect x="3" y="3" width="7" height="7" rx="1" /><rect x="14" y="3" width="7" height="7" rx="1" /><rect x="3" y="14" width="7" height="7" rx="1" /><rect x="14" y="14" width="7" height="7" rx="1" /></svg>
          </button>
          <button type="button" className="icon-btn" aria-pressed={view === 'list'} title="List" aria-label="List" onClick={() => choose('list')}>
            <svg {...ICON}><path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01" /></svg>
          </button>
        </div>
      </div>
      {sites.state === 'reading' && <p className="panel__status">Reading your sites</p>}
      {sites.state === 'failed' && <p className="panel__status">Your sites could not be read: {sites.reason}</p>}
      {sites.state === 'read' && total === 0 && <p className="home__empty">No sites yet. Press New site to start from a template.</p>}
      {sites.state === 'read' && total > 0 && shown.length === 0 && <p className="home__empty" role="status">No site matches "{query}".</p>}
      {shown.length > 0 && (
        <ul className={`home__sites home__sites--${view}`} aria-label="Your sites">
          {shown.map((row) => (view === 'tiles' ? <Tile key={row.site} row={row} /> : <Row key={row.site} row={row} />))}
        </ul>
      )}
    </div>
  );
}
