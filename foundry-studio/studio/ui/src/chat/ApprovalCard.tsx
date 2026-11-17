// Spec 02 section 3.1, ApprovalCard: pending, a region named "Approval needed: {summary}" whose input is read before
// any button (the facts, the notes, the moved-since note, the exact input behind a disclosure, the reason field, then
// approve and Deny); decided, a plain card with who decided, or "denied by your message" when a message denied it
// (lean 6), and, for an approved act whose execution failed or is not yet known, one line, or spec 04's
// `Unattributable` question with its buttons for the approver only. No budget, used or left, appears anywhere (D61).

import { Fragment, useState } from 'react';
import type { ProjectedApproval } from '../protocol/projection.ts';
import { useShell } from '../shell/context.ts';
import { ShellLink } from '../shell/ShellLink.tsx';
import type { ApprovalRequested } from '../protocol/approval.ts';
import { decidedOf, executedOf, notedOf, requestedOf } from './bodies.ts';
import { useChatApi } from './chatContext.ts';
import { approverOf, deniedByMessage } from './log.ts';
import { changesetsWords, inSentence } from './words.ts';

const DEFAULT_FOOTNOTE = 'Approving executes exactly this input. If the agent wants anything different, it has to ask again.';

/**
 * The approved act's one line (A14's words), or the attribution question, whose buttons only its approver sees, as
 * the journal names them (spec 01's `not-approver`); nothing while it runs or once it landed.
 */
export function ExecutionLine({ approval }: { approval: ProjectedApproval }) {
  const api = useChatApi();
  const shell = useShell();
  const [error, setError] = useState<string | null>(null);
  const executed = executedOf(approval);
  if (executed === null) return null;
  const approver = !api.readOnly && api.me !== null && approverOf(approval)?.subject === api.me.subject;
  if (executed.unattributable !== undefined) {
    const seqs = executed.unattributable.seqs;
    const [scope, seq] = Object.entries(seqs)[0] ?? [null, null];
    const site = scope?.split('/').slice(0, 4).join('/');
    const errorId = `approval-${approval.id}-attribution-error`;
    const answer = async (wasThis: boolean) => {
      const outcome = await api.attribute(approval.id, wasThis);
      if (outcome.ok) shell.announce(wasThis ? 'Recorded: this change landed.' : 'Recorded: this change did not land.');
      else {
        setError(outcome.words);
        shell.announce(outcome.words);
      }
    };
    return (
      <>
        <p>
          A change matching this one landed as {changesetsWords(seqs)}, but Agent Studio cannot prove it was this one. Check the
          history and say whether it was.{' '}
          {scope !== null && <ShellLink to={`/w/${scope}@${seq}?panel=history`}>The history of {site}</ShellLink>}
        </p>
        {approver && (
          <div className="chat-card__actions">
            <button className="btn btn--small" type="button" aria-describedby={error ? errorId : undefined} onClick={() => void answer(true)}>It was this change</button>
            <button className="btn btn--small" type="button" aria-describedby={error ? errorId : undefined} onClick={() => void answer(false)}>It was not</button>
          </div>
        )}
        {error !== null && <p className="chat-card__error" id={errorId}>{error}</p>}
      </>
    );
  }
  if (executed.execution === 'failed') return <p>The approved change failed: {executed.sentence ?? 'no reason was given'}</p>;
  if (executed.execution === 'unknown') return <p>{executed.sentence ?? 'Not yet known whether this change landed; checking again.'}</p>;
  return null;
}

function Facts({ requested }: { requested: ApprovalRequested }) {
  const facts = requested.facts;
  // Consecutive entries of one label share their `dt`.
  const groups: { label: string; values: string[] }[] = [];
  for (const { label, value } of facts.display) {
    const last = groups.at(-1);
    if (last !== undefined && last.label === label) last.values.push(value);
    else groups.push({ label, values: [value] });
  }
  return (
    <>
      {facts.lead && <p>{facts.lead}</p>}
      {groups.length > 0 && (
        <dl className="approval-card__input">
          {groups.map((group, n) => (
            <Fragment key={n}>
              <dt>{group.label}</dt>
              {group.values.map((value, m) => <dd key={m}>{value}</dd>)}
            </Fragment>
          ))}
        </dl>
      )}
    </>
  );
}

export function ApprovalCard({ approval }: { approval: ProjectedApproval }) {
  const api = useChatApi();
  const shell = useShell();
  const [reason, setReason] = useState('');
  const [error, setError] = useState<string | null>(null);
  const requested = requestedOf(approval);
  if (requested === null) return null;
  const base = `approval-${approval.id}`;
  const titleId = `${base}-title`;
  const summary = requested.summary;
  const decided = decidedOf(approval);
  const byYou = decided !== null && decided.author.subject === api.me?.subject;
  const noted = notedOf(approval);

  if (approval.state === 'pending' && !api.readOnly) {
    const errorId = `${base}-error`;
    const act = async (decision: 'approve' | 'deny') => {
      const outcome = await api.decide(approval.id, decision, decision === 'deny' && reason.trim() !== '' ? reason.trim() : undefined);
      if (outcome.ok) {
        shell.focusComposer();
        shell.announce(decision === 'approve' ? `Approved: ${inSentence(summary)}.` : 'Denied.');
      } else {
        setError(outcome.words);
        shell.announce(outcome.words);
      }
    };
    const described = error !== null ? errorId : undefined;
    return (
      <section className="chat-card approval-card" id={base} data-state="pending" aria-labelledby={titleId}>
        <h4 className="chat-card__title" id={titleId} tabIndex={-1}>
          Approval needed: {inSentence(summary)} <span className="pill" data-tone="warn">waiting for you</span>
        </h4>
        <Facts requested={requested} />
        {requested.facts.notes.map((note, n) => <p className="approval-card__note" key={n}>{note}</p>)}
        {noted !== null && <p className="approval-card__note" data-kind="moved">{noted.note}</p>}
        <details className="approval-card__exact">
          <summary>The exact input</summary>
          <pre>{JSON.stringify(requested.input, null, 2)}</pre>
        </details>
        <div className="field">
          <label htmlFor={`${base}-reason`}>Reason, if you deny (the agent reads it)</label>
          <input className="input" id={`${base}-reason`} type="text" value={reason} onChange={(event) => setReason(event.currentTarget.value)} />
        </div>
        <div className="chat-card__actions">
          <button className="btn btn--primary" type="button" aria-describedby={described} onClick={() => void act('approve')}>
            {requested.facts.action_label}
          </button>
          <button className="btn" type="button" aria-describedby={described} onClick={() => void act('deny')}>Deny</button>
        </div>
        <p className="chat-card__footnote">{requested.facts.footnote ?? DEFAULT_FOOTNOTE}</p>
        {error !== null && <p className="chat-card__error" id={errorId}>{error}</p>}
      </section>
    );
  }

  const turn = approval.turn ? api.model.turns.get(approval.turn) : undefined;
  const who = byYou ? 'you' : decided?.author.name ?? 'someone';
  // A message's denial is that message, which stands in the log: the card names it and does not repeat it.
  const byMessage = deniedByMessage(api.model, approval);
  return (
    <div className="chat-card approval-card" id={base} data-state={approval.state}>
      <h4 className="chat-card__title" id={titleId} tabIndex={-1}>
        {approval.state === 'approved' && <>Approved: {inSentence(summary)} <span className="pill" data-tone="good"><span aria-hidden="true">{'✓'}</span>approved by {who}</span></>}
        {approval.state === 'denied' && (
          <>Denied: {inSentence(summary)} <span className="pill" data-tone="bad">{byMessage ? `denied by ${byYou ? 'your' : `${who}'s`} message` : `denied by ${who}`}</span></>
        )}
        {approval.state === 'expired' && <>Not decided: {inSentence(summary)}</>}
        {approval.state === 'pending' && <>Approval needed: {inSentence(summary)} <span className="pill" data-tone="warn">waiting</span></>}
      </h4>
      {approval.state === 'denied' && !byMessage && decided?.reason && <p>{decided.reason}</p>}
      {approval.state === 'expired' && <p>{turn?.end_reason === 'wait-expired' ? 'The wait ran out.' : 'The turn was stopped.'}</p>}
      <Facts requested={requested} />
      {approval.state === 'approved' && <ExecutionLine approval={approval} />}
    </div>
  );
}
