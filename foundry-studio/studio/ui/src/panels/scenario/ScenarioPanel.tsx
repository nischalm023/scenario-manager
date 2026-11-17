// Spec 07 section 5, the Scenario panel at `/scenarios/{ULID}?version=` (spec 03 section 3.2): after the title the
// status line, the readiness line and a toolbar (the Version select, Duplicate, Delete with its inline confirmation),
// then `form.scenario-form` with three fieldsets, Workload, Run and Sweep, then Run and the Flow link (ruling D95).
// It edits the latest version only, each field committed on its own (section 4.2) and told to the agent as a panel
// event; an older version or a deleted scenario reads as it was. Readiness is read again whenever a scope of the
// site moves (spec 01's scope events). The h2 is the panel host's, which answers before the scenario is read; once
// read, the panel names the scenario's site and the cut readiness read to the shell, which binds the chat column to
// the site's conversation and the page context to that cut (ruling D94; spec 03 sections 3.2 and 3.4).

import { useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import type { Problem } from '../../protocol/problem.ts';
import type { FieldCatalogue, FieldConflict, FieldEntry, ScenarioEdited, ScenarioRead } from '../../protocol/scenario.ts';
import type { JsonValue } from '../../protocol/serde_json/JsonValue.ts';
import type { Readiness } from '../../protocol/workload.ts';
import { clause, getJson, postJson, type Answer } from '../../shell/api.ts';
import { useShell } from '../../shell/context.ts';
import { ServicesContext } from '../../shell/panels.tsx';
import { ShellLink } from '../../shell/ShellLink.tsx';
import { deleteJson, patchJson, pressKey } from './api.ts';
import { NumberField, ReleaseField, type CommitOutcome } from './Field.tsx';
import { ReadinessLine, WallTime, type ReadinessState } from './ReadinessLine.tsx';
import { RunControls } from './RunControls.tsx';
import { conflictWords, fieldId, setByWords, sharedSetter, valueWords } from './words.ts';

type Loaded = { state: 'loading' } | { state: 'read'; read: ScenarioRead } | { state: 'failed'; reason: string };
type Status = { state: 'saved' | 'saving' | 'failed'; words: string | null };

const WORKLOAD = ['orders_per_hour', 'lines_per_order', 'sku_count', 'inventory_fill_pct', 'seed'].map((field) => `/workload/generator/${field}`);
const RELEASE = '/workload/generator/release';
const WAVES = '/workload/generator/wave_every_sim_ms';
const DURATION = '/run_controls/duration_sim_ms';
const SHOWN = [...WORKLOAD, RELEASE, WAVES, DURATION];

const query = (version?: number): string => (version === undefined ? '' : `?version=${version}`);
const fetchScenario = (id: string, version?: number) => getJson<ScenarioRead>(`/api/scenarios/${id}${query(version)}`);
const fetchReadiness = (id: string, version?: number) => getJson<Readiness>(`/api/scenarios/${id}/readiness${query(version)}`);

function without<T>(record: Record<string, T>, key: string): Record<string, T> {
  const next = { ...record };
  delete next[key];
  return next;
}

/** A field's input, or the checked radio of a closed field. */
function focusField(pointer: string) {
  const input = document.getElementById(fieldId(pointer)) ?? document.querySelector<HTMLElement>(`input[name="${pointer}"]:checked`);
  input?.focus();
}

function at(read: ScenarioRead, pointer: string): JsonValue | undefined {
  let value: unknown = read.version.content;
  for (const step of pointer.split('/').slice(1)) value = (value as Record<string, unknown> | undefined)?.[step];
  return value as JsonValue | undefined;
}

export function ScenarioPanel({ id, version }: { id: string; version?: number }) {
  const shell = useShell();
  const me = shell.me?.subject ?? null;
  const [catalogue, setCatalogue] = useState<FieldEntry[]>([]);
  const [loaded, setLoaded] = useState<Loaded>({ state: 'loading' });
  const [answered, setAnswered] = useState<{ edition: string | null; state: ReadinessState }>({ edition: null, state: { state: 'reading' } });
  const [status, setStatus] = useState<Status>({ state: 'saved', words: null });
  const [conflicts, setConflicts] = useState<Record<string, { words: string; mine: JsonValue }>>({});
  const [confirming, setConfirming] = useState(false);
  const [repeats, setRepeats] = useState(1);
  const [again, setAgain] = useState(0);
  const [generation, setGeneration] = useState<Record<string, number>>({});
  const deleteButton = useRef<HTMLButtonElement>(null);
  const keepIt = useRef<HTMLButtonElement>(null);
  const read = loaded.state === 'read' ? loaded.read : null;
  const site = read?.scenario.site_scope ?? null;
  // The readiness on screen answers one edition of the version (`{n}.{revision}`); one about another edition shows as
  // reading, so Run waits for the answer about the edition on screen.
  const edition = read === null ? null : `${read.version.n}.${read.version.revision}`;
  const readiness: ReadinessState = answered.edition === edition ? answered.state : { state: 'reading' };
  const say = useCallback((sentence: string) => shell.announce(sentence), [shell]);
  const nameSite = useContext(ServicesContext)?.nameSite;
  const cut = readiness.state === 'read' ? readiness.readiness.cut : null;
  const seen = useMemo(() => Object.entries(cut?.seqs ?? {}).map(([scope, seq]) => ({ scope, seq })), [cut]);
  useEffect(() => () => nameSite?.(null), [nameSite]);
  useEffect(() => {
    if (site !== null) nameSite?.({ site, seen });
  }, [nameSite, site, seen]);

  const accept = useCallback((answer: Answer<ScenarioRead>): ScenarioRead | null => {
    if (!answer.ok) {
      setLoaded({ state: 'failed', reason: answer.status === 404 ? 'no scenario has that id' : answer.reason });
      return null;
    }
    setLoaded({ state: 'read', read: answer.value });
    return answer.value;
  }, []);
  const reload = useCallback(() => fetchScenario(id, version).then(accept), [id, version, accept]);

  // Each readiness request is numbered, and the answer to one a newer request superseded is dropped.
  const asked = useRef(0);
  const askReadiness = useCallback((about: string) => {
    const n = (asked.current += 1);
    void fetchReadiness(id, version).then((answer) => {
      if (n !== asked.current) return;
      if (answer.ok) setAnswered({ edition: about, state: { state: 'read', readiness: answer.value } });
      else if (answer.status === 503 && answer.problem?.type === 'compute-busy') {
        setAnswered({ edition: about, state: { state: 'busy' } });
        window.setTimeout(() => setAgain((k) => k + 1), (answer.problem.retry_after_s ?? 2) * 1000);
      } else setAnswered({ edition: about, state: { state: 'failed', reason: clause(answer.reason) } });
    });
  }, [id, version]);
  /** A read the site's move or the person's Retry asks for: the answer on screen stops counting at once. */
  const readAgain = useCallback((about: string | null) => {
    if (about === null) return;
    setAnswered({ edition: about, state: { state: 'reading' } });
    askReadiness(about);
  }, [askReadiness]);

  useEffect(() => {
    void getJson<FieldCatalogue>('/api/scenario-fields').then((answer) => answer.ok && setCatalogue(answer.value.items));
    void fetchScenario(id, version).then(accept);
  }, [id, version, accept]);

  useEffect(() => {
    if (edition !== null) askReadiness(edition);
  }, [edition, again, askReadiness]);

  // Spec 01's scope events: a changeset on any scope of the site reads readiness again.
  const shownEdition = useRef(edition);
  useEffect(() => {
    shownEdition.current = edition;
  }, [edition]);
  useEffect(() => {
    if (site === null || typeof EventSource === 'undefined') return;
    const scopes = [site, `${site}/map`, `${site}/flow`, `${site}/fleet`].map((scope) => `scope=${encodeURIComponent(scope)}`).join('&');
    const events = new EventSource(`/api/scope-events?${scopes}`);
    events.addEventListener('scope.moved', () => readAgain(shownEdition.current));
    return () => events.close();
  }, [site, readAgain]);

  const tell = useCallback(async (kind: string, text: string, body: Record<string, unknown>) => {
    if (site === null) return;
    const opened = await postJson<{ conversation: { id: string } }>('/api/conversations', { site });
    if (!opened.ok) return;
    await postJson(`/api/conversations/${opened.value.conversation.id}/panel-events`, { kind, text, body }, pressKey());
  }, [site]);

  if (loaded.state === 'loading') return <div aria-busy="true"><p className="panel__status">Reading the scenario</p></div>;
  if (loaded.state === 'failed' || read === null) {
    return <p className="panel__status" data-state="failed">The scenario could not be read: {loaded.state === 'failed' ? loaded.reason : ''}.</p>;
  }

  const { scenario, versions } = read;
  const latest = read.version.n === scenario.latest_version;
  const deleted = scenario.deleted_at !== null;
  const readOnly = deleted || !latest;
  const entry = (pointer: string) => catalogue.find((field) => field.pointer === pointer);
  const shared = sharedSetter(SHOWN.map((pointer) => read.version.set_by[pointer]), me);

  // Every save of a field goes here, an edit's and "Use mine"'s alike, against the edition `base` the person saw: a
  // conflict keeps the person's value until a save of it lands or they keep the other one.
  const save = async (pointer: string, value: JsonValue, base: ScenarioRead): Promise<CommitOutcome> => {
    const field = entry(pointer);
    setStatus({ state: 'saving', words: null });
    const answer = await patchJson<ScenarioEdited>(`/api/scenarios/${id}`, {
      version: base.version.n, revision: base.version.revision, set: [{ pointer, value }],
    });
    if (answer.ok) {
      const before = at(base, pointer);
      const edited = answer.value;
      setConflicts((now) => without(now, pointer));
      const opened = edited.opened ? ` Version ${base.version.n} stays as its runs read it.` : '';
      setStatus({ state: 'saved', words: opened || null });
      if (edited.opened) say(`Version ${edited.version} opened with your change.${opened}`);
      if (field) {
        const text = `You changed ${field.label} in ${scenario.name} from ${valueWords(field, before)} to ${valueWords(field, value)}`;
        void tell('scenario.changed', text, { scenario: id, version: edited.version, revision: edited.revision });
      }
      if (version !== undefined && edited.opened) shell.navigate(`/scenarios/${id}`, 'within');
      else await reload();
      return { kind: 'saved' };
    }
    setStatus({ state: 'failed', words: null });
    const problem: Problem | null = answer.problem;
    if (problem?.type === 'scenario-changed' && field) {
      const found: FieldConflict | undefined = problem.conflicts?.find((one) => one.pointer === pointer) ?? problem.conflicts?.[0];
      const words = conflictWords(field, found?.value, found?.set_by, me);
      setConflicts((now) => ({ ...now, [pointer]: { words, mine: value } }));
      say(`${field.label} was not saved: ${words}`);
      return { kind: 'conflict', words };
    }
    const words = problem?.problems?.[0]?.text ?? `Not saved: ${clause(answer.reason)}.`;
    say(`${field?.label ?? 'The field'} was not saved: ${words}`);
    return { kind: 'refused', words };
  };

  const commit = (pointer: string, value: JsonValue): Promise<CommitOutcome> => save(pointer, value, read);
  // "Use mine" saves the person's value over the newest edition; "Keep this" takes that edition's value.
  const resolve = async (pointer: string, choice: 'mine' | 'theirs'): Promise<CommitOutcome> => {
    const mine = conflicts[pointer]?.mine;
    const fresh = await reload();
    let outcome: CommitOutcome = { kind: 'saved' };
    if (fresh !== null && choice === 'mine' && mine !== undefined) outcome = await save(pointer, mine, fresh);
    else if (fresh !== null) {
      setConflicts((now) => without(now, pointer));
      setGeneration((now) => ({ ...now, [pointer]: (now[pointer] ?? 0) + 1 }));
      setStatus({ state: 'saved', words: null });
    }
    window.setTimeout(() => focusField(pointer), 0);
    return outcome;
  };

  const duplicate = async () => {
    const answer = await postJson<{ scenario: string }>(`/api/scenarios/${id}/duplicate`, version === undefined ? {} : { version }, pressKey());
    if (answer.ok) shell.navigate(`/scenarios/${answer.value.scenario}`, 'user');
    else say(`Not duplicated: ${clause(answer.reason)}.`);
  };
  const remove = async () => {
    const answer = await deleteJson(`/api/scenarios/${id}`);
    setConfirming(false);
    if (answer.ok) await reload();
    else say(`Not deleted: ${clause(answer.reason)}.`);
    window.setTimeout(() => document.getElementById('panel-title')?.focus(), 0);
  };
  const ask = () => {
    setConfirming(true);
    window.setTimeout(() => keepIt.current?.focus(), 0);
  };
  const keep = () => {
    setConfirming(false);
    window.setTimeout(() => deleteButton.current?.focus(), 0);
  };

  const statusWords = (): string => {
    if (deleted) return '';
    const n = read.version.n;
    if (!latest) return `Version ${n}, ${read.version.state}: an older version, read as its runs read it.`;
    if (status.state === 'saving') return `Version ${n}, open: saving`;
    if (status.state === 'failed') return `Version ${n}, open: 1 field not saved`;
    if (read.version.state === 'sealed') return `Version ${n}, sealed by its runs: your next edit starts version ${n + 1}.`;
    const setter = shared ? ` Every field ${shared}.` : '';
    return `Version ${n}, open: saved.${status.words ?? ''}${setter}`;
  };
  const field = (pointer: string, children?: ReactNode) => {
    const found = entry(pointer);
    if (found === undefined) return null;
    const set = read.version.set_by[pointer];
    const props = {
      entry: found, scenario: id, value: at(read, pointer), readOnly, commit, resolve,
      setBy: shared ? null : setByWords(set, me) || null, setter: set ? set.by : null, conflict: conflicts[pointer] ?? null,
    };
    const key = `${pointer}#${generation[pointer] ?? 0}`;
    return pointer === RELEASE ? <ReleaseField key={key} {...props} /> : <NumberField key={key} {...props}>{children}</NumberField>;
  };
  const durationHours = Number(at(read, DURATION) ?? 0) / 3_600_000;

  return (
    <div className="home setup-page scenario-page">
      <header className="home__head page-head draft-head">
        <span className="page-head__icon" aria-hidden="true">
          <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" /></svg>
        </span>
        <div>
          <p className="home__title" id="sc-name">{scenario.name}</p>
          <p className="panel__status home__sub" id="sc-status" data-state={deleted ? 'saved' : status.state}>
            {deleted && scenario.deleted_at !== null
              ? <>Deleted by {scenario.deleted_by?.name ?? 'someone'} on <WallTime at={scenario.deleted_at} date />; it reads as it was.</>
              : statusWords()}
          </p>
        </div>
      </header>
      <div className="scenario-readiness"><ReadinessLine state={readiness} site={scenario.site_scope} retry={() => readAgain(edition)} /></div>
      <div className="panel__toolbar scenario-toolbar">
        <label className="visually-hidden" htmlFor="sc-version">Version</label>
        <select
          className="select" id="sc-version" name="version" value={read.version.n}
          onChange={(event) => {
            const n = Number(event.target.value);
            shell.navigate(n === scenario.latest_version ? `/scenarios/${id}` : `/scenarios/${id}?version=${n}`, 'user');
          }}
        >
          {versions.map((item) => <option key={item.version} value={item.version}>Version {item.version}, {item.state}</option>)}
        </select>
        {!latest && <button className="btn btn--small" type="button" onClick={() => shell.navigate(`/scenarios/${id}`, 'user')}>Show the latest</button>}
        <button className="btn btn--small" type="button" onClick={() => void duplicate()}>Duplicate</button>
        {!deleted && <button className="btn btn--small btn--danger" type="button" ref={deleteButton} onClick={ask}>Delete</button>}
      </div>
      {confirming && (
        <div className="section" id="sc-delete-confirm" role="group" aria-labelledby="sc-delete-question">
          <p id="sc-delete-question">Delete {scenario.name}? Its runs stay and keep their link.</p>
          <p className="chat-card__actions">
            <button className="btn btn--small" type="button" ref={keepIt} onClick={keep}>Keep it</button>{' '}
            <button className="btn btn--small btn--danger" type="button" onClick={() => void remove()}>Delete scenario</button>
          </p>
        </div>
      )}
      <form className="scenario-form" onSubmit={(event) => event.preventDefault()}>
        <fieldset className="fieldset">
          <legend>Workload</legend>
          <div className="form-row">{WORKLOAD.slice(0, 4).map((pointer) => field(pointer))}</div>
          {field(RELEASE)}
          {at(read, RELEASE) === 'waves' && field(WAVES)}
          <div className="form-row">{field(WORKLOAD[4])}</div>
        </fieldset>
        <fieldset className="fieldset">
          <legend>Run</legend>
          <div className="form-row">
            {field(DURATION, <>The run stops at <span className="clock" data-clock="sim">{durationHours} h<span className="clock__tag">simtime</span></span>; the generator makes orders for as long.</>)}
            <div className="field">
              <label className="field__label" htmlFor="rc-repeats">Repeats</label>
              <input
                className="input input--number" id="rc-repeats" name="repeats" type="number" min={1} max={10} step={1} value={repeats}
                aria-describedby="rc-repeats-hint" onChange={(event) => setRepeats(Math.min(10, Math.max(1, Math.round(Number(event.target.value) || 1))))}
              />
              <p className="field__hint" id="rc-repeats-hint">Each repeat runs with the next simulation seed and reads the same orders.</p>
            </div>
          </div>
        </fieldset>
        <RunControls
          scenario={{ id, name: scenario.name, version: read.version.n, revision: read.version.revision, site: scenario.site_scope }}
          sweepable={catalogue.filter((entryOf) => entryOf.sweepable)}
          repeats={repeats}
          readiness={readiness.state === 'read' ? readiness.readiness : null}
          held={readOnly}
          tell={(kind, text, body) => void tell(kind, text, body)}
          announce={say}
        />
        <p className="muted"><ShellLink to={`/w/${scenario.site_scope}/flow@main`}>Flow values this site runs with</ShellLink></p>
      </form>
    </div>
  );
}
