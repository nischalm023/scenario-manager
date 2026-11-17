// The Stations page of Setup: a site's station groups and their work, read from its flow graph, edited through the
// person's own flow draft (spec 06 section 3.4), the same draft the Flow panel's detail pane writes. The page reads
// that draft with the stations, shows each field as Apply would write it, the site's value under it where the two
// differ, and applies or discards the draft from the bar at its foot. Queues and shifts, which Agent Studio holds no
// value for yet, stand as labelled placeholders.

import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import type { ChangesetOp, DraftRead } from '../../protocol/changeset_draft.ts';
import type { FlowNode, FlowParameter, FlowParameters, FlowRead } from '../../protocol/flow.ts';
import { getJson } from '../../shell/api.ts';
import { useShell } from '../../shell/context.ts';
import { printPin } from '../../shell/grammar.ts';
import { Info, TipFact, TipLine } from '../../shell/Info.tsx';
import { routeFor } from '../../shell/routes.ts';
import { ShellLink } from '../../shell/ShellLink.tsx';
import { addEdit, discard, ownDraft, pressApply } from '../flow/drafts.ts';
import type { Read } from './read.ts';
import { Head, Soon, Status, type WorldRoute } from './setupPage.tsx';
import { grouped } from './words.ts';

const OPERATION: Record<string, string> = { pick: 'Picking', put: 'Putaway', decant: 'Decant' };
/** How a station's field reads to a person: by its pointer's last part. */
const FIELD_WORDS: Record<string, { label: string; help: string }> = {
  pick_sim_ms: { label: 'Operator time per pick', help: 'How long the operator takes on one pick, before the tote swap.' },
  tote_swap_sim_ms: { label: 'Tote swap', help: 'The pause while one tote leaves the station and the next arrives.' },
  presentations_per_hour: { label: 'Fill pace', help: 'How many totes an hour the station can be presented with.' },
  admission_factor: { label: 'Admission factor', help: 'How many totes may be on their way to the station per presentation point.' },
  sequencing: { label: 'Sequencing', help: 'The order totes are presented in.' },
};
const words = (parameter: FlowParameter) => FIELD_WORDS[parameter.pointer.split('/').pop() ?? ''] ?? { label: parameter.short_label, help: parameter.label };
const inSeconds = (parameter: FlowParameter) => parameter.pointer.endsWith('_sim_ms');

/**
 * A station's one field: its value and, at a branch head, the input that changes it in the person's draft; where the
 * draft changes it (`was` set), what the site holds without the draft.
 */
function StationField({ parameter, value, was, editable, onCommit }: { parameter: FlowParameter; value: unknown; was?: unknown; editable: boolean; onCommit(value: unknown): void }) {
  const shown = inSeconds(parameter) && typeof value === 'number' ? value / 1000 : value;
  const unit = inSeconds(parameter) ? 's' : parameter.unit ?? '';
  const { label, help } = words(parameter);
  const id = `st-${parameter.ref.slice(0, 8)}-${parameter.pointer.split('/').pop()}`;
  const hint = was === undefined ? undefined : `${id}-draft`;
  const inWords = (raw: unknown) => {
    const option = parameter.options?.find((choice) => choice.value === raw);
    return `${option?.label ?? String(inSeconds(parameter) && typeof raw === 'number' ? raw / 1000 : raw)}${unit && ` ${unit}`}`;
  };
  let field: ReactNode;
  if (!editable) {
    field = <span className="station-field__value">{inWords(value)}</span>;
  } else if (parameter.options !== undefined) {
    field = (
      <select id={id} className="select" value={String(value)} aria-describedby={hint} onChange={(event) => onCommit(event.target.value)}>
        {parameter.options.map((choice) => <option key={choice.value} value={choice.value}>{choice.label}</option>)}
      </select>
    );
  } else {
    const commit = (text: string) => {
      const number = Number(text);
      if (text.trim() === '' || !Number.isFinite(number) || number === shown) return;
      onCommit(inSeconds(parameter) ? Math.round(number * 1000) : number);
    };
    field = (
      <span className="input-unit station-field__input">
        <input id={id} className="input input--number" type="number" step="any" min={0} defaultValue={String(shown)} key={String(shown)} aria-describedby={hint}
          onBlur={(event) => commit(event.target.value)} onKeyDown={(event) => { if (event.key === 'Enter') commit(event.currentTarget.value); }} />
        {unit && <span className="input-unit__unit">{unit}</span>}
      </span>
    );
  }
  return (
    <div className="setup-fact station-field" data-pointer={parameter.pointer}>
      <dt><label htmlFor={id}>{label}</label> <Info label={label}><TipLine>{help}</TipLine></Info></dt>
      <dd>{field}{hint !== undefined && <p className="field__hint" id={hint}>Your draft changes this from {inWords(was)}.</p>}</dd>
    </div>
  );
}

interface Stations { flow: FlowRead; fields: FlowParameter[]; draft: DraftRead | null }

/**
 * The stations at the route's read, their fields and, at a branch head, the person's open draft on the site, read
 * together so the page never shows one without the others: any of the three failing fails the page, with its reason.
 * The draft is read as its operations, not drawn by the API over the flow, because a draft value the simulator's
 * reader refuses would draw no stations at all.
 */
async function readStations(site: string, pin: string, editable: boolean): Promise<Read<Stations>> {
  const [flow, parameters, held] = await Promise.all([
    getJson<FlowRead>(`/api/flow/${site}/flow@${pin}`),
    getJson<FlowParameters>(`/api/flow-parameters/${site}/flow@${pin}`),
    editable ? ownDraft(site) : null,
  ]);
  if (!flow.ok) return { state: 'failed', reason: flow.reason };
  if (!parameters.ok) return { state: 'failed', reason: parameters.reason };
  if (held !== null && !held.ok) return { state: 'failed', reason: held.reason };
  const draft = held !== null && held.ok ? held.value : null;
  return { state: 'ready', value: { flow: flow.value, fields: parameters.value.parameters, draft } };
}

/** The value the draft gives a field: its last operation patching the field or one above it; undefined for none. */
function draftedValue(draft: DraftRead | null, parameter: FlowParameter): unknown {
  let value: unknown;
  for (const op of draft?.ops ?? []) {
    if (op.target !== parameter.ref || !`${parameter.pointer}/`.startsWith(`${op.pointer}/`)) continue;
    value = parameter.pointer.slice(op.pointer.length).split('/').slice(1)
      .reduce<unknown>((at, step) => (at as Record<string, unknown> | undefined)?.[step], op.value);
  }
  return value;
}

export function StationsPanel({ site, route }: { site: string; route: WorldRoute }) {
  const shell = useShell();
  const pin = printPin(route.location.pin);
  const at = `${site}/flow@${pin}`;
  const [held, setHeld] = useState<{ at: string; read: Read<Stations> } | null>(null);
  /** Edits saved to the draft since the stations were last read, shown until the next read carries them. */
  const [pending, setPending] = useState<Map<string, unknown>>(new Map());
  const [busy, setBusy] = useState<string | null>(null);
  const editable = route.location.pin.kind === 'branch';
  const asked = useRef(0);
  /** Read the stations again, the last read staying on view until the answer lands; only the newest read lands. */
  const reload = useCallback(() => {
    asked.current += 1;
    const mine = asked.current;
    return readStations(site, pin, editable).then((read) => {
      if (mine !== asked.current) return;
      setHeld({ at, read });
      setPending(new Map());
    });
  }, [site, pin, editable, at]);
  useEffect(() => void reload(), [reload]);
  const stations: Read<Stations> = held?.at === at ? held.read : { state: 'loading' };
  const draft = stations.state === 'ready' ? stations.value.draft : null;

  const edit = async (parameter: FlowParameter, value: unknown, station: string) => {
    if (stations.state !== 'ready') return;
    setBusy('Saving to your draft');
    const op: ChangesetOp = { op: 'patch', target: parameter.ref, pointer: parameter.pointer, value: value as ChangesetOp['value'] };
    const answer = await addEdit(site, `${site}@${pin}`, stations.value.flow.seqs, op, `${station}: ${words(parameter).label} to ${String(value)}`);
    if (!answer.ok) {
      setBusy(null);
      shell.announce(`The change was not saved: ${answer.reason}`);
      return;
    }
    setPending((was) => new Map(was).set(`${parameter.ref}${parameter.pointer}`, value));
    await reload();
    setBusy(null);
  };
  const apply = async () => {
    if (draft === null) return;
    setBusy('Applying your changes');
    const answer = await pressApply(draft.draft);
    if (!answer.ok) shell.announce(`Not applied: ${answer.reason}`);
    else if (answer.value.execution === 'ok') shell.announce('Your station changes were applied.');
    else shell.announce(`Not applied: ${answer.value.sentence ?? 'the apply did not end well'}`);
    // An Apply refused or failed leaves the draft open (a site that moved on stales it): the next read shows it again.
    await reload();
    setBusy(null);
  };
  const drop = async () => {
    if (draft === null) return;
    await discard(draft.draft);
    await reload();
  };
  const open = draft !== null && (draft.state === 'drafting' || draft.state === 'ready');

  return (
    <div className="home setup-page">
      <Head icon={<><circle cx="12" cy="8" r="4" /><path d="M4 21a8 8 0 0 1 16 0" /></>} title="Stations" words="Where people work the totes: how many stations, how fast, and how they are fed."
        info={<Info label="this page"><TipFact term="Source">Each station group and its work come from the site's process flow.</TipFact><TipFact term="Editing">A change goes into your draft first; Apply makes it the site's, as a new flow changeset.</TipFact><TipFact term="Not yet">Queues, breaks and shifts are not held by Agent Studio.</TipFact></Info>}
        right={!editable ? <span className="pill">Read only: pinned view</span> : undefined} />
      <Status read={stations} what="the stations">
        {({ flow: read, fields }) => {
          const groups = read.nodes.filter((node) => node.kind === 'stations');
          if (groups.length === 0) return <p className="home__empty">This site has no station groups in its flow.</p>;
          const fieldsOf = (node: FlowNode) => fields.filter((one) => one.ref === node.ref && one.sweepable && one.pointer.startsWith('/flow.node/'));
          /** A field's value as Apply would write it (an edit saved since the last read, else the draft's, else the
           * site's), and the site's own wherever the two differ. */
          const shownOf = (parameter: FlowParameter) => {
            const value = pending.get(`${parameter.ref}${parameter.pointer}`) ?? draftedValue(draft, parameter) ?? parameter.value;
            return { value, was: value !== parameter.value ? parameter.value : undefined };
          };
          return (
            <>
              <ul className="evaluation__summary">
                {groups.map((node) => (
                  <li className="figure-tile" key={node.ref}><span className="figure-tile__value">{grouped(node.members)}</span><span className="figure-tile__label">{node.name}</span></li>
                ))}
              </ul>
              <div className="setup-grid">
                {groups.map((node) => {
                  const process = (node.process ?? {}) as Record<string, unknown>;
                  const op = String(process.operation ?? '');
                  const allInDrafted = fieldsOf(node).some((parameter) => parameter.pointer === node.all_in?.field && shownOf(parameter).was !== undefined);
                  return (
                    <section className="setup-card station-card" key={node.ref} data-ref={node.ref}>
                      <div className="setup-card__head">
                        <h3 className="setup-card__title">{node.name}</h3>
                        <span className="pill" data-tone="info">{grouped(node.members)} stations · {OPERATION[op] ?? op}</span>
                      </div>
                      {node.all_in && <p className="station-card__allin">{allInDrafted && 'Without your draft: '}<span className="clock" data-clock="sim">{node.all_in.text}</span></p>}
                      <dl className="setup-facts">
                        {fieldsOf(node).map((parameter) => (
                          <StationField key={parameter.pointer} parameter={parameter} {...shownOf(parameter)} editable={editable && busy === null}
                            onCommit={(value) => void edit(parameter, value, node.name)} />
                        ))}
                        <Soon label="Queue limit" note="per station" />
                        <Soon label="Breaks and shifts" />
                      </dl>
                      <p className="setup-card__foot">
                        <ShellLink to={routeFor({ ...route, show: 'flow', location: { scope: `${site}/flow`, pin: route.location.pin }, select: node.ref })}>See it in the process flow</ShellLink>
                      </p>
                    </section>
                  );
                })}
              </div>
            </>
          );
        }}
      </Status>
      {(open || busy !== null) && (
        <footer className="pinned-foot draft-foot" aria-live="off">
          <div className="apply-bar__what">
            <strong>{busy ?? `${draft?.ops.length ?? 0} change${(draft?.ops.length ?? 0) === 1 ? '' : 's'} in your draft`}</strong>
            <span className="muted">{busy !== null ? 'One moment.' : 'Nothing changes on the site until you apply.'}</span>
          </div>
          <div className="apply-bar__actions">
            <button className="btn btn--pill" type="button" disabled={busy !== null} onClick={() => void drop()}>Discard</button>
            <button className="btn btn--pill btn--accent" type="button" disabled={busy !== null || !open} onClick={() => void apply()}>Apply changes</button>
          </div>
        </footer>
      )}
    </div>
  );
}
