// Spec 06 section 5's detail pane, by selection: what the entity is and how it links, its figures with the draft's
// column beside the base's while a draft is on view, and an editor for each figure a sweep may vary (spec 06 section
// 3.4's `sweepable`), whose edit goes into the person's own draft. A pinned view edits nothing.

import { Fragment, type ReactNode } from 'react';
import type { ChangesetOp } from '../../protocol/changeset_draft.ts';
import type { FlowFieldDiff, FlowParameter, FlowRead } from '../../protocol/flow.ts';
import type { ProblemItem } from '../../protocol/problem.ts';
import { ShellLink } from '../../shell/ShellLink.tsx';
import { Clocked } from './Clocked.tsx';
import { Info, TipLine } from '../../shell/Info.tsx';
import { carrier, custodyWords, edgeCompilesTo, kindOf, madeFromWords, movesWords, nodeWhat, valueWords } from './words.ts';

export interface DetailProps {
  read: FlowRead;
  selected: string | null;
  parameters: FlowParameter[];
  /** Why nothing here is editable (a pinned view), or null at a branch head. */
  readOnly: string | null;
  /** The base changeset's column heading while a draft is on view. */
  draftOver: string | null;
  mapRoute(ref: string): string;
  onEdit(op: ChangesetOp, summary: string): void;
  onSelect(ref: string, name: string): void;
}

function Shown({ value, parameter }: { value: unknown; parameter: FlowParameter }) {
  const option = parameter.options?.find((choice) => choice.value === value);
  const text = option !== undefined ? `${option.label} (${option.value})` : `${valueWords(value)}${parameter.unit ? ` ${parameter.unit}` : ''}`;
  return parameter.clock === null ? <>{text}</> : <Clocked clock={parameter.clock}>{text}</Clocked>;
}

function Editor({ parameter, now, onCommit }: { parameter: FlowParameter; now: unknown; onCommit(value: unknown): void }) {
  const label = `${parameter.label}, in the draft`;
  if (parameter.options !== undefined) {
    return (
      <select aria-label={label} value={String(now)} onChange={(event) => onCommit(event.target.value)}>
        {parameter.options.map((choice) => <option key={choice.value} value={choice.value}>{choice.label}</option>)}
      </select>
    );
  }
  if (typeof now === 'boolean') return <input type="checkbox" aria-label={label} checked={now} onChange={(event) => onCommit(event.target.checked)} />;
  const commit = (text: string) => {
    const value = Number(text);
    if (text.trim() !== '' && Number.isFinite(value) && value !== now) onCommit(value);
  };
  return (
    <input
      type="number" step="any" aria-label={label} defaultValue={String(now)} key={String(now)} className="input input--number"
      onBlur={(event) => commit(event.target.value)}
      onKeyDown={(event) => { if (event.key === 'Enter') commit(event.currentTarget.value); }}
    />
  );
}

function Props({ rows }: { rows: [string, ReactNode][] }) {
  return <dl className="detail-pane__props">{rows.map(([term, value]) => <Fragment key={term}><dt>{term}</dt><dd>{value}</dd></Fragment>)}</dl>;
}

export function DetailPane(props: DetailProps) {
  const { read, selected } = props;
  const names = new Map<string, string>([
    ...read.nodes.map((n): [string, string] => [n.ref, n.name]), ...read.edges.map((e): [string, string] => [e.ref, e.label]),
    ...read.segments.map((s): [string, string] => [s.ref, s.label]), ...read.return_rules.map((r): [string, string] => [r.ref, r.label]),
  ]);
  const link = (ref: string) => <button type="button" className="btn btn--quiet btn--small" onClick={() => props.onSelect(ref, names.get(ref) ?? ref)}>{names.get(ref) ?? ref}</button>;
  const node = read.nodes.find((n) => n.ref === selected);
  const edge = read.edges.find((e) => e.ref === selected);
  const segment = read.segments.find((s) => s.ref === selected);
  const rule = read.return_rules.find((r) => r.ref === selected);
  const entity = node ?? edge ?? segment ?? rule;
  if (selected === null || entity === undefined) {
    return (
      <section className="detail-pane detail-pane--empty">
        <h3 className="detail-pane__title" id="detail-title">Details</h3>
        <p className="muted">Pick anything in the drawing or the lists, and what it is and what you can change about it shows here.</p>
      </section>
    );
  }
  const title = node?.name ?? (edge ?? segment ?? rule)?.label ?? '';
  let kind = 'Return rule';
  const glyph = node !== undefined ? kindOf(node).glyph : edge !== undefined ? '→' : segment !== undefined ? '⇢' : '↩';
  const tone = node !== undefined ? 'node' : edge !== undefined ? 'material' : segment !== undefined ? 'agent' : 'rule';
  const lead = node !== undefined ? nodeWhat(node)
    : edge !== undefined ? `Product moves here: ${movesWords(edge)}.`
      : segment !== undefined ? `A robot leg: ${segment.executor.name} carries it.`
        : 'A return rule, kept so that documents naming it still load; no run reads it.';
  let rows: [string, ReactNode][] = [];
  if (node !== undefined) {
    kind = node.kind === 'stations' ? 'Station-group node' : node.kind === 'storage' ? 'Storage node' : 'Node';
    rows = [['Sectors', node.sectors.map((s) => `${s.name ?? 'a sector'}, ${s.members} members`).join('; ') || 'none']];
    if (node.members > 0) rows.push(['On the map', <ShellLink to={props.mapRoute(node.ref)}>Show the {node.members} members on the map</ShellLink>]);
    if (node.all_in) rows.push(['All-in', <Clocked clock="sim">{node.all_in.text}</Clocked>]);
  } else if (edge !== undefined) {
    kind = 'Material edge';
    const steps = read.segments.filter((s) => s.edge === edge.ref).sort((a, b) => (a.step ?? 0) - (b.step ?? 0));
    rows = [['From', link(edge.from)], ['To', link(edge.to)], ['Moves', movesWords(edge)]];
    if (edge.when !== undefined) rows.push(['Only when', valueWords(edge.when)]);
    if (edge.alternative_to !== undefined) rows.push(['An alternative to', link(edge.alternative_to)]);
    if (edge.returns_by !== undefined) rows.push(['Returns by', link(edge.returns_by)]);
    rows.push(['Segments', steps.length === 0 ? 'none' : <ol>{steps.map((s) => <li key={s.ref}>{link(s.ref)}</li>)}</ol>]);
    rows.push(['Compiles to', edgeCompilesTo(edge, read.plan)]);
  } else if (segment !== undefined) {
    kind = 'Agent-flow segment';
    const executor = segment.executor.count === undefined ? segment.executor.name : `${segment.executor.name}, ${segment.executor.count} robots`;
    rows = [['Carries', segment.edge === undefined ? 'nothing (an empty move)' : link(segment.edge)], ['Step', String(segment.step ?? 1)],
      ['Who', segment.executor.kind === 'assigned' ? `${executor}: at each rack, the cohort hard-assigned to its aisle, which the compiler resolves; the cohorts are in the fleet` : executor],
      ['From', `${names.get(segment.from.node) ?? ''} (${valueWords(segment.from.port)})`], ['To', `${names.get(segment.to.node) ?? ''} (${valueWords(segment.to.port)})`],
      ['At the end', `${carrier(segment.executor)} ${custodyWords(segment.at_end)}`]];
    if (segment.retained_for !== undefined) rows.push(['Retained for', <>{segment.retained_for.map((ref) => <Fragment key={ref}>{link(ref)} </Fragment>)}</>]);
  } else if (rule !== undefined) {
    rows = [['Backlog', `${rule.min_backlog}, which no run reads`], ['To', valueWords(rule.destination)],
      ['Named by', <>{read.edges.filter((e) => e.returns_by === rule.ref).map((e) => <Fragment key={e.ref}>{link(e.ref)} </Fragment>)}</>]];
  }
  const all = props.parameters.filter((p) => p.ref === selected);
  const fields = all.filter((p) => p.sweepable);
  const rest = all.filter((p) => !p.sweepable);
  const diffs = new Map<string, FlowFieldDiff>((entity.diff?.fields ?? []).map((field) => [field.pointer, field]));
  const problems: ProblemItem[] = read.problems.filter((problem) => problem.ref === selected);
  const caption = `Values of ${title}${props.draftOver === null ? '' : `, at ${props.draftOver} and in the draft`}`;
  return (
    <section className="detail-pane" data-ref={selected} data-tone={tone}>
      <header className="detail-pane__head">
        <span className="detail-pane__glyph" aria-hidden="true">{glyph}</span>
        <div>
          <p className="label">{kind}</p>
          <h3 className="detail-pane__title" id="detail-title">{title}</h3>
          <p className="detail-pane__lead">{lead}</p>
        </div>
      </header>
      <Props rows={rows} />
      {fields.length > 0 && (
        <div className="detail-pane__settings">
          <h4 className="detail-pane__h4">
            {props.readOnly === null ? 'Adjust' : 'Settings'}
            <Info label="these settings">
              <TipLine>{props.readOnly === null
                ? 'Each value here can be varied. A change goes into your draft first; Apply above makes it the site\u2019s.'
                : props.readOnly}</TipLine>
            </Info>
          </h4>
          <div className="table-scroll" tabIndex={0} role="region" aria-label={caption}>
            <table className="param-table param-table--inspector">
              <caption className="visually-hidden">{caption}</caption>
              <thead><tr>
                <th scope="col">Setting</th><th scope="col">{props.draftOver ?? 'Now'}</th>
                {props.draftOver !== null && <th scope="col">Draft</th>}
                {props.readOnly === null && <th scope="col"><span className="visually-hidden">New value</span></th>}
              </tr></thead>
              <tbody>
                {fields.map((parameter) => {
                  const diff = diffs.get(parameter.pointer);
                  const base = diff?.was ?? parameter.value;
                  return (
                    <tr key={parameter.pointer} data-pointer={parameter.pointer} data-diff={diff ? 'changed' : undefined}>
                      <th scope="row">{parameter.short_label}</th>
                      <td><Shown value={base} parameter={parameter} /></td>
                      {props.draftOver !== null && <td className={diff ? 'draft-value' : 'inherited'}>{diff ? <Shown value={diff.becomes} parameter={parameter} /> : 'unchanged'}</td>}
                      {props.readOnly === null && (
                        <td><Editor parameter={parameter} now={diff?.becomes ?? parameter.value} onCommit={(value) => props.onEdit(
                          { op: 'patch', target: parameter.ref, pointer: parameter.pointer, value: value as ChangesetOp['value'] },
                          `${title}: ${parameter.label} to ${valueWords(value)}`)} /></td>
                      )}
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>
      )}
      {problems.map((problem, at) => (
        <p key={at} className="problem" data-severity={problem.severity}>{problem.text}</p>
      ))}
      {(rest.length > 0 || entity.made_from_template) && (
        <details className="detail-pane__more">
          <summary>All values{rest.length > 0 ? ` (${rest.length})` : ''}</summary>
          {rest.length > 0 && (
            <table className="param-table param-table--inspector param-table--plain">
              <caption className="visually-hidden">Every other value of {title}</caption>
              <tbody>
                {rest.map((parameter) => (
                  <tr key={parameter.pointer} data-pointer={parameter.pointer}>
                    <th scope="row">{parameter.short_label}</th>
                    <td><Shown value={parameter.value} parameter={parameter} /></td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {entity.made_from_template && <p className="muted">{madeFromWords(read.made_from)}. Updating the site from its preset puts back its values here.</p>}
        </details>
      )}
    </section>
  );
}
