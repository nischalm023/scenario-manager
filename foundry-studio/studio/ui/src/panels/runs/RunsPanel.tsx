// Spec 08 section 5, the Runs panel's list (mockup 08): `/runs`, a site's runs, all of the person's or one
// submission's, with the queue in one sentence (or why it could not be read, the last queue read said as out of
// date) and `table.runs-table` in a TableScroll, a checkbox per run and
// "Compare selected"; `/runs/{run}`, that run on a page of its own (`OpenRun`) under a switcher that moves between
// the site's runs without the list, and a link back to it; the row's link marked
// `aria-current="page"`, the one attribute marking it; and `/runs/compare`, the list with the compared runs ticked and
// the comparison below it (`Comparison`, brief I1.10), the list and the chat column following the run the comparison
// measures against. The list shows no metric of a run: ruling D192 dropped its headline column, which showed an
// arbitrary first metric; a run's figures are on its page. The list comes a page at a time (spec 08 section 3.7),
// "Show older runs" following the last page's `next`. The panel reads the endpoints the agent's run tools read, again
// every 2 s of wall clock, and says nothing itself: state changes reach the announcer through the chat, and the panel
// holds no live region. The shell keeps one instance across the views, so a filter's radio keeps focus as it moves, and
// the ticked runs follow the address's comparison whenever it changes, Back included.

import { useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import type { RunDetail, RunList, RunQueue, RunSummary } from '../../protocol/runs.ts';
import { clause, getJson, type Answer } from '../../shell/api.ts';
import { useShell } from '../../shell/context.ts';
import { ServicesContext } from '../../shell/panels.tsx';
import { routeFor } from '../../shell/routes.ts';
import { ShellLink } from '../../shell/ShellLink.tsx';
import { readStored, writeStored } from '../../shell/storage.ts';
import { TableScroll } from '../../shell/TableScroll.tsx';
import { WallTime } from '../scenario/ReadinessLine.tsx';
import { Comparison } from './Comparison.tsx';
import { OpenRun, Wall } from './OpenRun.tsx';
import { queueWords, shortId, simulatedWords, stateWords } from './words.ts';

const EVERY_MS = 2000;
const COMPARE_LEAST = 2;
const COMPARE_MOST = 12;

type Read<T> = { state: 'reading' } | { state: 'read'; value: T } | { state: 'failed'; reason: string };

const readOf = <T,>(answer: Answer<T>): Read<T> =>
  answer.ok ? { state: 'read', value: answer.value } : { state: 'failed', reason: clause(answer.reason) };

/** The queue as read: nothing yet, the newest answer and when it came (wall clock), or why the newest read failed with
 * the last queue read before it, if any was, which is then out of date. */
type QueueRead =
  | { state: 'reading' }
  | { state: 'read'; value: RunQueue; at: string }
  | { state: 'failed'; reason: string; last: { value: RunQueue; at: string } | null };

function queueAfter(was: QueueRead, answer: Answer<RunQueue>, at: string): QueueRead {
  if (answer.ok) return { state: 'read', value: answer.value, at };
  const last = was.state === 'read' ? { value: was.value, at: was.at } : was.state === 'failed' ? was.last : null;
  return { state: 'failed', reason: clause(answer.reason), last };
}

export interface RunsPanelProps {
  site?: string;
  filter?: 'site' | 'mine';
  submission?: string;
  /** `/runs/{run}`: the run open below the list. */
  open?: string;
  /** The agent's `show` entry the address names, whose metrics the open run headlines when the project pins none. */
  highlights?: number;
  /** `/runs/compare`: the runs compared below the list, and the one their changes are measured against. */
  compare?: { ids: string[]; reference?: string };
}

/** Which runs the list shows: one submission's, a site's, or every run the person submitted. */
type Shown = { kind: 'submission'; submission: string } | { kind: 'site'; site: string } | { kind: 'mine' };

function shownBy({ site, filter, submission }: RunsPanelProps, openSite: string | null): Shown {
  if (submission !== undefined) return { kind: 'submission', submission };
  const known = site ?? openSite;
  return filter === 'mine' || known === null ? { kind: 'mine' } : { kind: 'site', site: known };
}

/** The list's first `pages` pages as one list, each page continuing after the last one's `next`, which it keeps. */
async function readPages(query: string, pages: number): Promise<Answer<RunList>> {
  const items: RunSummary[] = [];
  let answer = await getJson<RunList>(`/api/runs?${query}`);
  for (let page = 1; answer.ok; page += 1) {
    items.push(...answer.value.items);
    const next = answer.value.next;
    if (next === undefined || page >= pages) return { ...answer, value: { items, ...(next !== undefined ? { next } : {}) } };
    answer = await getJson<RunList>(`/api/runs?${query}&after=${encodeURIComponent(next)}`);
  }
  return answer;
}

function queryOf(shown: Shown): string {
  switch (shown.kind) {
    case 'submission': return `submission=${encodeURIComponent(shown.submission)}`;
    case 'site': return `site=${encodeURIComponent(shown.site)}`;
    case 'mine': return 'mine=true';
  }
}

const ICON = { width: 18, height: 18, viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', strokeWidth: 2, strokeLinecap: 'round', strokeLinejoin: 'round', 'aria-hidden': true } as const;
type View = 'tiles' | 'list';

type FilterKey = 'customer' | 'site' | 'scenario' | 'status';
type Filters = Record<FilterKey, string>;
const NO_FILTERS: Filters = { customer: '', site: '', scenario: '', status: '' };

/** "customer/demo/site/l-shape-2" names customer "demo" and site "l-shape-2". */
function placeOf(site: string): { customer: string; site: string } {
  const match = /^customer\/([^/]+)\/site\/(.+)$/.exec(site);
  return match !== null ? { customer: match[1], site: match[2] } : { customer: '', site: site };
}

const statusOf = (run: RunSummary): string => run.state;
const valueOf = (run: RunSummary, key: FilterKey): string =>
  key === 'customer' ? placeOf(run.site).customer : key === 'site' ? placeOf(run.site).site : key === 'scenario' ? run.scenario.name : statusOf(run);
const passes = (run: RunSummary, filters: Filters): boolean =>
  (Object.keys(filters) as FilterKey[]).every((key) => filters[key] === '' || valueOf(run, key) === filters[key]);
const optionsOf = (items: RunSummary[]): Record<FilterKey, string[]> => {
  const of = (key: FilterKey) => [...new Set(items.map((run) => valueOf(run, key)).filter((value) => value !== ''))].sort();
  return { customer: of('customer'), site: of('site'), scenario: of('scenario'), status: of('status') };
};

const SUBTITLE: Record<Shown['kind'], string> = {
  mine: 'Every simulation you have submitted, newest first. Open a run for its results, or tick two or more to compare them side by side.',
  site: 'The simulations run on this site, newest first. Open a run for its results, or tick two or more to compare them side by side.',
  submission: 'The runs of one submission, newest first. Open a run for its results, or tick two or more to compare them side by side.',
};

const WHOSE: Record<Shown['kind'], string> = { submission: 'Runs of one submission', site: 'Runs of this site', mine: 'Your runs' };

export function RunsPanel(props: RunsPanelProps) {
  const shell = useShell();
  const nameSite = useContext(ServicesContext)?.nameSite;
  const [detail, setDetail] = useState<{ id: string; read: Read<RunDetail> } | null>(null);
  const [list, setList] = useState<{ query: string; read: Read<RunList> } | null>(null);
  const [queue, setQueue] = useState<QueueRead>({ state: 'reading' });
  const [chosen, setChosen] = useState<string[]>(props.compare?.ids ?? []);
  const [asked, setAsked] = useState(0);
  const [older, setOlder] = useState<{ query: string; pages: number } | null>(null);
  const [reference, setReference] = useState<{ compared: string; run: string } | null>(null);
  const [search, setSearch] = useState('');
  const [filters, setFilters] = useState<Filters>(NO_FILTERS);
  const [view, setView] = useState<View>(() => (readStored('local', 'studio.runs-view') === 'tiles' ? 'tiles' : 'list'));
  const chooseView = (next: View) => {
    setView(next);
    writeStored('local', 'studio.runs-view', next);
  };

  // The ticked runs are the address's comparison's whenever the address changes it (Back included), since the shell
  // keeps this instance across the views; between such changes they are the person's.
  const compared = props.compare === undefined ? '' : `${props.compare.ids.join(',')}|${props.compare.reference ?? ''}`;
  const routeIds = props.compare?.ids.join(',') ?? '';
  const [tickedFor, setTickedFor] = useState(routeIds);
  if (tickedFor !== routeIds) {
    setTickedFor(routeIds);
    setChosen(props.compare?.ids ?? []);
  }
  const onReference = useCallback((run: string) => setReference({ compared, run }), [compared]);

  // The run read beside the list: the open one, or the run a comparison measures against, once the comparison names
  // it (the API may replace the asked one), whose site the list and the chat column show.
  const measured = reference !== null && reference.compared === compared ? reference.run : undefined;
  const anchor = props.open ?? (props.compare !== undefined ? measured : undefined);
  const opened: Read<RunDetail> | null = anchor === undefined ? null
    : detail !== null && detail.id === anchor ? detail.read : { state: 'reading' };
  const anchored = opened?.state === 'read' ? opened.value : null;
  const openRun = props.open !== undefined ? anchored : null;
  // The anchor names the site whose runs the list shows, so the list waits for it rather than show another first.
  const named = props.site !== undefined || props.submission !== undefined;
  const siteUnknown = !named && ((props.compare !== undefined && anchor === undefined) || (anchor !== undefined && opened?.state === 'reading'));
  const shown = shownBy(props, anchored?.site ?? null);
  const query = siteUnknown ? null : queryOf(shown);
  const pages = older !== null && older.query === query ? older.pages : 1;

  useEffect(() => {
    let current = true;
    const tick = async () => {
      const [listed, queued, run] = await Promise.all([
        query === null ? null : readPages(query, pages),
        getJson<RunQueue>('/api/runs/queue'),
        anchor === undefined ? null : getJson<RunDetail>(`/api/runs/${anchor}`),
      ]);
      if (!current) return;
      if (listed !== null && query !== null) setList({ query, read: readOf(listed) });
      setQueue((was) => queueAfter(was, queued, new Date().toISOString()));
      if (run !== null && anchor !== undefined) setDetail({ id: anchor, read: readOf(run) });
    };
    void tick();
    const timer = window.setInterval(() => void tick(), EVERY_MS);
    return () => {
      current = false;
      window.clearInterval(timer);
    };
  }, [query, pages, anchor, asked]);

  // An open run names its site and what it read, so the chat column is the site's conversation (spec 02 section 6).
  useEffect(() => () => nameSite?.(null), [nameSite]);
  useEffect(() => {
    if (anchored === null || props.site !== undefined) return;
    nameSite?.({ site: anchored.site, seen: Object.entries(anchored.reads).map(([scope, seq]) => ({ scope, seq })) });
  }, [nameSite, anchored, props.site]);

  const site = props.site ?? anchored?.site ?? null;
  const filterTo = (filter: 'site' | 'mine') => {
    if (site !== null) shell.navigate(routeFor({ view: 'runs', site, filter }), 'within');
  };
  const choose = (id: string, on: boolean) => setChosen((now) => (on ? [...now, id] : now.filter((one) => one !== id)));
  const comparable = chosen.length >= COMPARE_LEAST && chosen.length <= COMPARE_MOST;
  const compare = () => {
    if (comparable) shell.navigate(routeFor({ view: 'compare', ids: chosen }), 'user');
  };
  const everything = useMemo<Read<RunList>>(() => (list !== null && list.query === query ? list.read : { state: 'reading' }), [list, query]);
  const needle = search.trim().toLowerCase();
  // A site's list shows no customer or site column, so a filter on either, set on the list of all the person's runs,
  // hides nothing here.
  const applied = useMemo(() => (shown.kind === 'site' ? { ...filters, customer: '', site: '' } : filters), [filters, shown.kind]);
  const listed = useMemo<Read<RunList>>(() => {
    if (everything.state !== 'read') return everything;
    const items = everything.value.items.filter((run) => passes(run, applied)
      && (needle === '' || `${run.label} ${run.id} ${run.scenario.name} ${stateWords(run).words} ${run.site}`.toLowerCase().includes(needle)));
    return { state: 'read', value: { ...everything.value, items } };
  }, [everything, needle, applied]);
  const options = useMemo(() => optionsOf(everything.state === 'read' ? everything.value.items : []), [everything]);
  const filtering = needle !== '' || Object.values(applied).some((value) => value !== '');
  const all = everything.state === 'read' ? everything.value.items : [];
  const counts = {
    running: all.filter((run) => run.state === 'running').length,
    waiting: all.filter((run) => run.state === 'queued').length,
    finished: all.filter((run) => run.state === 'finished').length,
  };

  if (props.open !== undefined) {
    return (
      <RunView
        open={props.open} runs={all} anchored={anchored} site={site} counted={everything.state === 'read'} opened={opened} openRun={openRun}
        highlights={props.highlights} reread={() => setAsked((n) => n + 1)} navigate={(to) => shell.navigate(to, 'user')}
      />
    );
  }

  return (
    <div className="home runs-home">
      <header className="home__head">
        <div>
          <p className="home__title" aria-hidden="true">Runs</p>
          <p className="home__sub">{SUBTITLE[shown.kind]}</p>
        </div>
      </header>
      {everything.state === 'read' && (
        <div className="home__metrics">
          <div className="metric"><span className="metric__label">Runs listed</span><span className="metric__value">{all.length}</span><span className="metric__note">{everything.value.next !== undefined ? 'Older runs are not loaded yet' : 'Every run in this list'}</span></div>
          <div className="metric"><span className="metric__label">Running</span><span className="metric__value">{counts.running}</span><span className="metric__note">Simulating right now</span></div>
          <div className="metric"><span className="metric__label">Waiting</span><span className="metric__value">{counts.waiting}</span><span className="metric__note">Queued for a seat</span></div>
          <div className="metric"><span className="metric__label">Finished</span><span className="metric__value">{counts.finished}</span><span className="metric__note">Ran to the end</span></div>
        </div>
      )}
      <div className="home__bar">
        <label className="home__search">
          <svg {...ICON}><circle cx="11" cy="11" r="7" /><path d="M20 20l-3.5-3.5" /></svg>
          <input type="search" placeholder="Search runs" aria-label="Search runs" value={search} onChange={(event) => setSearch(event.target.value)} />
        </label>
        <div className="home__views" role="group" aria-label="View">
          <button type="button" className="icon-btn" aria-pressed={view === 'tiles'} title="Tiles" aria-label="Tiles" onClick={() => chooseView('tiles')}>
            <svg {...ICON}><rect x="3" y="3" width="7" height="7" rx="1" /><rect x="14" y="3" width="7" height="7" rx="1" /><rect x="3" y="14" width="7" height="7" rx="1" /><rect x="14" y="14" width="7" height="7" rx="1" /></svg>
          </button>
          <button type="button" className="icon-btn" aria-pressed={view === 'list'} title="List" aria-label="List" onClick={() => chooseView('list')}>
            <svg {...ICON}><path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01" /></svg>
          </button>
        </div>
      </div>
      {shown.kind !== 'submission' && site !== null && (
        <fieldset className="segmented" aria-label="Show runs from">
          <label>
            <input type="radio" name="runs-filter" value="site" checked={shown.kind === 'site'} onChange={() => filterTo('site')} /> This site
          </label>
          <label>
            <input type="radio" name="runs-filter" value="mine" checked={shown.kind === 'mine'} onChange={() => filterTo('mine')} /> All mine
          </label>
        </fieldset>
      )}
      <QueueLine queue={queue} />
      {chosen.length > 0 && (
        <div className="compare-bar">
          <button className="btn btn--pill btn--accent" type="button" id="runs-compare" onClick={compare}
            aria-disabled={comparable ? undefined : true} aria-describedby="runs-compare-hint">
            Compare selected
          </button>
          <span className="field__hint" id="runs-compare-hint">
            {chosen.length < COMPARE_LEAST ? `${chosen.length} chosen; choose 2 to 12 runs to compare.` : `${chosen.length} chosen; 2 to 12 compare.`}
          </span>
          <button className="btn btn--pill" type="button" onClick={() => setChosen([])}>Clear</button>
        </div>
      )}
      <Table listed={listed} whose={WHOSE[shown.kind]} open={props.open} chosen={chosen} choose={choose} view={view} searching={filtering} filters={filters} options={options} setFilter={(key, value) => setFilters((now) => ({ ...now, [key]: value }))} oneSite={shown.kind === 'site'} />
      {listed.state === 'read' && query !== null && (listed.value.next !== undefined || pages > 1) && (
        <p>
          {/* Once every run shows the button stays, saying so, so focus never falls to the page's body. */}
          <button
            className="btn" type="button" id="runs-older" aria-disabled={listed.value.next === undefined ? true : undefined}
            onClick={() => listed.value.next !== undefined && setOlder({ query, pages: pages + 1 })}
          >
            {listed.value.next !== undefined ? 'Show older runs' : 'All runs shown'}
          </button>
        </p>
      )}
      {props.compare !== undefined && <Comparison ids={props.compare.ids} reference={props.compare.reference} onReference={onReference} />}
    </div>
  );
}

/** A run on a page of its own: a switcher over it, top left, that moves between the site's runs (the list's, newest
 * first, the open run among them even when the list has yet to load), a link back to the list, then the run. */
function RunView({ open, runs, anchored, site, counted, opened, openRun, highlights, reread, navigate }: {
  open: string; runs: RunSummary[]; anchored: RunDetail | null; site: string | null; counted: boolean;
  opened: Read<RunDetail> | null; openRun: RunDetail | null; highlights?: number; reread(): void; navigate(to: string): void;
}) {
  const listed = runs.some((run) => run.id === open) || anchored === null ? runs : [anchored, ...runs];
  return (
    <div className="home runs-home run-view">
      <nav className="run-switch" aria-label="Runs">
        <ShellLink className="btn btn--pill run-switch__back" to={site === null ? '/runs' : routeFor({ view: 'runs', site, filter: 'site' })}>
          <span aria-hidden="true">←</span> All runs
        </ShellLink>
        <label className="run-switch__pick">
          <span className="visually-hidden">Open another run</span>
          <select value={open} onChange={(event) => navigate(routeFor({ view: 'run', id: event.target.value }))} aria-label="Open another run">
            {listed.map((run) => <option key={run.id} value={run.id}>{run.label} · {stateWords(run).words} · {shortId(run.id)}</option>)}
            {listed.length === 0 && <option value={open}>{shortId(open)}</option>}
          </select>
        </label>
        {counted && <span className="muted run-switch__count">{runs.length} runs on this site, newest first</span>}
      </nav>
      {opened?.state === 'reading' && <p className="panel__status" aria-busy="true">Reading the run</p>}
      {opened?.state === 'failed' && <p className="panel__status">The run could not be read: {opened.reason}.</p>}
      {openRun !== null && <OpenRun key={openRun.id} run={openRun} reread={reread} highlights={highlights} />}
    </div>
  );
}

function QueueLine({ queue }: { queue: QueueRead }) {
  switch (queue.state) {
    case 'reading': return <p className="muted" id="runs-queue" aria-busy="true">Reading the queue</p>;
    case 'read': return <p className="muted" id="runs-queue">{queueWords(queue.value)}</p>;
    case 'failed':
      if (queue.last === null) return <p className="muted" id="runs-queue">The queue could not be read: {queue.reason}.</p>;
      return (
        <p className="muted" id="runs-queue">
          <span className="pill" data-tone="warn">out of date</span> {queueWords(queue.last.value)} As read
          at <WallTime at={queue.last.at} />; reading it again failed: {queue.reason}.
        </p>
      );
  }
}

function Table({ listed, whose, open, chosen, choose, view, searching, filters, options, setFilter, oneSite }: {
  listed: Read<RunList>; whose: string; open?: string; chosen: string[]; choose(id: string, on: boolean): void; view: View; searching: boolean;
  filters: Filters; options: Record<FilterKey, string[]>; setFilter(key: FilterKey, value: string): void;
  /** The list holds one site's runs, so it names no customer or site: each row's would be the page's own. */
  oneSite: boolean;
}) {
  if (listed.state === 'reading') return <p className="panel__status" aria-busy="true">Reading the runs</p>;
  if (listed.state === 'failed') return <p className="panel__status">The runs could not be read: {listed.reason}.</p>;
  const items = listed.value.items;
  if (items.length === 0 && !searching) return <p className="empty-state">No run yet. A scenario's Run starts one.</p>;
  if (view === 'tiles') {
    return items.length === 0 ? <p className="home__empty">No run matches your search or filters.</p> : (
      <ul className="home__sites home__sites--tiles" aria-label={whose}>
        {items.map((run) => <Tile key={run.id} run={run} open={run.id === open} chosen={chosen.includes(run.id)} choose={choose} />)}
      </ul>
    );
  }
  return (
    <TableScroll name={whose}>
      <table className="list-table runs-table">
        <caption className="visually-hidden">{whose}, the newest submission first.</caption>
        <thead>
          <tr>
            <th scope="col"><span className="visually-hidden">Compare</span></th><th scope="col">Run</th>
            {!oneSite && <th scope="col"><HeadFilter label="Customer" name="customer" value={filters.customer} options={options.customer} onChange={(v) => setFilter('customer', v)} /></th>}
            {!oneSite && <th scope="col"><HeadFilter label="Site" name="site" value={filters.site} options={options.site} onChange={(v) => setFilter('site', v)} /></th>}
            <th scope="col"><HeadFilter label="Scenario" name="scenario" value={filters.scenario} options={options.scenario} onChange={(v) => setFilter('scenario', v)} /></th>
            <th scope="col"><HeadFilter label="Status" name="status" value={filters.status} options={options.status} onChange={(v) => setFilter('status', v)} /></th>
            <th scope="col" className="num">Simulated time</th>
            <th scope="col">Started</th><th scope="col" className="num">Duration</th>
          </tr>
        </thead>
        <tbody>
          {items.length === 0 && <tr><td colSpan={oneSite ? 7 : 9} className="muted">No run matches your search or filters.</td></tr>}
          {items.map((run) => <Row key={run.id} run={run} open={run.id === open} chosen={chosen.includes(run.id)} choose={choose} oneSite={oneSite} />)}
        </tbody>
      </table>
    </TableScroll>
  );
}

/** A funnel beside a column's name; it opens a small list of that column's values, one of which narrows the table. */
function HeadFilter({ label, name, value, options, onChange }: {
  label: string; name: string; value: string; options: string[]; onChange(value: string): void;
}) {
  const [at, setAt] = useState<{ top: number; left: number } | null>(null);
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const open = at !== null;
  useEffect(() => {
    if (!open) return;
    const close = () => setAt(null);
    const away = (event: MouseEvent) => {
      if (!menu.current?.contains(event.target as Node) && !button.current?.contains(event.target as Node)) close();
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        close();
        button.current?.focus();
      }
    };
    document.addEventListener('mousedown', away);
    document.addEventListener('keydown', escape);
    window.addEventListener('resize', close);
    document.addEventListener('scroll', close, true);
    return () => {
      document.removeEventListener('mousedown', away);
      document.removeEventListener('keydown', escape);
      window.removeEventListener('resize', close);
      document.removeEventListener('scroll', close, true);
    };
  }, [open]);
  const toggle = () => {
    const box = button.current?.getBoundingClientRect();
    setAt(open || box === undefined ? null : { top: box.bottom + 6, left: Math.max(8, Math.min(box.left, window.innerWidth - 232)) });
  };
  const pick = (next: string) => {
    onChange(next);
    setAt(null);
    button.current?.focus();
  };
  return (
    <span className="head-filter">
      {label}
      <button ref={button} type="button" className="head-filter__button" data-active={value !== '' ? 'true' : undefined} aria-haspopup="true"
        aria-expanded={open} title={value !== '' ? `Filter ${label}: ${value}` : `Filter ${label}`} aria-label={`Filter by ${label.toLowerCase()}`} onClick={toggle}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
          <path d="M3 5h18l-7 8v6l-4 2v-8z" />
        </svg>
      </button>
      {at !== null && (
        <div ref={menu} className="head-filter__menu" style={{ top: at.top, left: at.left }} role="group" aria-label={`${label} filter`}>
          {['', ...options].map((option) => (
            <label key={option} className="head-filter__option">
              <input type="radio" name={`filter-${name}`} checked={value === option} onChange={() => pick(option)} />
              {option === '' ? `All ${label.toLowerCase()}s` : name === 'status' ? capital(option) : option}
            </label>
          ))}
          {options.length === 0 && <span className="muted">Nothing to filter yet.</span>}
        </div>
      )}
    </span>
  );
}

const capital = (words: string): string => words.charAt(0).toUpperCase() + words.slice(1);

function Status({ run }: { run: RunSummary }) {
  const state = stateWords(run);
  return (
    <span className="pill pill--status" data-tone={state.tone}>
      {state.glyph !== null && <span aria-hidden="true">{state.glyph}</span>}<span className="pill__words">{state.words}</span>
    </span>
  );
}

function Tile({ run, open, chosen, choose }: {
  run: RunSummary; open: boolean; chosen: boolean; choose(id: string, on: boolean): void;
}) {
  const place = placeOf(run.site);
  return (
    <li className="site-entry site-entry--tiles" data-ref={run.ref}>
      <div className="run-tile__top">
        <Status run={run} />
        <input type="checkbox" className="run-tile__check" checked={chosen} onChange={(event) => choose(run.id, event.target.checked)}
          aria-label={`Compare run ${shortId(run.id)}, ${run.label}`} />
      </div>
      <div className="site-entry__id">
        <span className="site-badge" aria-hidden="true">{run.label.charAt(0).toUpperCase()}</span>
        <div className="site-entry__text">
          <ShellLink className="site-entry__name" to={routeFor({ view: 'run', id: run.id })} aria-current={open ? 'page' : undefined}>{run.label}</ShellLink>
          <code className="site-entry__ref">{shortId(run.id)}</code>
        </div>
      </div>
      <dl className="site-entry__place">
        <div><dt>Customer</dt><dd>{place.customer}</dd></div>
        <div><dt>Site</dt><dd>{place.site}</dd></div>
      </dl>
      <p className="site-entry__facts">
        <span className="clock" data-clock="sim">{simulatedWords(run)} <span className="clock__tag">simtime</span></span>
      </p>
      <div className="site-entry__meta">
        {run.started_at ? <WallTime at={run.started_at} /> : <span className="muted">not yet</span>}
        {run.elapsed_wall_ms !== undefined && <Wall ms={run.elapsed_wall_ms} />}
      </div>
    </li>
  );
}

function Row({ run, open, chosen, choose, oneSite }: {
  run: RunSummary; open: boolean; chosen: boolean; choose(id: string, on: boolean): void; oneSite: boolean;
}) {
  const place = placeOf(run.site);
  return (
    <tr data-ref={run.ref}>
      <td>
        <input
          type="checkbox" checked={chosen} onChange={(event) => choose(run.id, event.target.checked)}
          aria-label={`Compare run ${shortId(run.id)}, ${run.label}`}
        />
      </td>
      <td>
        <ShellLink className="runs-table__name" to={routeFor({ view: 'run', id: run.id })} aria-current={open ? 'page' : undefined}>{run.label}</ShellLink>
        <br /><code className="muted">{shortId(run.id)}</code>
      </td>
      {!oneSite && <><td>{place.customer}</td><td>{place.site}</td></>}
      <td>{run.scenario.name}</td>
      <td>
        <Status run={run} />
        {run.findings.includes('plan-differs') && <> <span className="pill" data-tone="warn">differs from the check</span></>}
        {/* A failed or partial run's pill already says why it ended; a cancelled run's says only when. */}
        {run.stop_reason && run.state === 'cancelled' && <><br /><span className="muted">{capital(run.stop_reason)}</span></>}
        {run.stop_detail && <><br /><span className="muted">{run.stop_detail}</span></>}
      </td>
      <td className="num">
        <span className="clock" data-clock="sim">{simulatedWords(run)} <span className="clock__tag">simtime</span></span>
      </td>
      <td>{run.started_at ? <WallTime at={run.started_at} /> : <span className="muted">not yet</span>}</td>
      <td className="num">
        {run.elapsed_wall_ms !== undefined ? <Wall ms={run.elapsed_wall_ms} /> : <span className="muted">not yet</span>}
      </td>
    </tr>
  );
}
