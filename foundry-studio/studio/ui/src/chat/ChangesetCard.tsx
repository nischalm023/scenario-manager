// Spec 02 section 3.1, ChangesetCard (ruling D43, one reviewable draft per turn; D72, one scope or several): always the
// turn's last card. It says what the draft was built against and who checked it, lists each touched scope's changes
// ("Added", "Changed", "Removed", with hidden "was" and "becomes"), the problems with their severity in words, and by
// state: Apply unavailable while drafting, Apply and Discard when ready ("Applying" from the 202 until the approval's
// execution ends, as log.ts's `applyOf` derives it from the journal), "Applied as …", or spec 04's sentence for why it
// went stale.

import { Fragment, useState } from 'react';
import type { DraftOp } from '../protocol/draft.ts';
import type { ProjectedApproval, ProjectedDraft } from '../protocol/projection.ts';
import { getJson } from '../shell/api.ts';
import { useShell } from '../shell/context.ts';
import { parseLocation, printPin } from '../shell/grammar.ts';
import { ShellLink } from '../shell/ShellLink.tsx';
import { ExecutionLine } from './ApprovalCard.tsx';
import { appliedOf, staleOf, updatedOf } from './bodies.ts';
import { useChatApi } from './chatContext.ts';
import { changesetsWords, plural, staleWords } from './words.ts';

const SHOWN_OPS = 12;
const REBUILD = 'Rebuild the draft on the new head';

const PILL: Record<ProjectedDraft['state'], { tone: string | undefined; words: string }> = {
  drafting: { tone: 'info', words: 'being drafted' },
  ready: { tone: 'warn', words: 'not applied' },
  applying: { tone: 'info', words: 'being applied' },
  applied: { tone: 'good', words: 'applied' },
  discarded: { tone: undefined, words: 'discarded' },
  stale: { tone: 'bad', words: 'out of date' },
};

/** The words for each of spec 01 section 3.11's operations, which studio.css marks by `data-op`. */
const OP_WORDS: Record<string, string> = { declare: 'Added', patch: 'Changed', delete: 'Removed' };

function Op({ op }: { op: DraftOp }) {
  return (
    <li className="op" data-op={op.op} data-ref={op.key}>
      <span className="op__kind">{OP_WORDS[op.op]}</span>
      <span>
        <span className="op__target">{op.target}</span>
        {(op.was !== undefined || op.becomes !== undefined) && (
          <>
            <br />
            <span className="op__change">
              {op.field ? `${op.field}: ` : ''}
              {op.was !== undefined && <><span className="visually-hidden">was</span><del>{op.was}</del> </>}
              {op.was !== undefined && op.becomes !== undefined && <span aria-hidden="true">{'→'} </span>}
              {op.becomes !== undefined && <><span className="visually-hidden">becomes</span><ins>{op.becomes}</ins></>}
            </span>
          </>
        )}
      </span>
    </li>
  );
}

function AllOps({ draft, scope, total }: { draft: string; scope: string; total: number }) {
  const [ops, setOps] = useState<DraftOp[] | string | null>(null);
  const read = async () => {
    const answer = await getJson<{ ops?: (DraftOp & { scope?: string })[] }>(`/api/drafts/${draft}`);
    setOps(answer.ok ? (answer.value.ops ?? []).filter((op) => op.scope === undefined || op.scope === scope) : answer.reason);
  };
  return (
    <details onToggle={(event) => {
      if (event.currentTarget.open && ops === null) void read();
    }}>
      <summary>Show all {total} changes</summary>
      {typeof ops === 'string' && <p>The changes could not be read: {ops}</p>}
      {Array.isArray(ops) && <ol className="changeset-card__ops" aria-label={`All ${total} changes`}>{ops.map((op, n) => <Op key={n} op={op} />)}</ol>}
    </details>
  );
}

export function ChangesetCard({ draft, execution, applying }: {
  draft: ProjectedDraft; execution: ProjectedApproval | null; applying: boolean;
}) {
  const api = useChatApi();
  const shell = useShell();
  const [error, setError] = useState<string | null>(null);
  const updated = updatedOf(draft);
  const stale = staleOf(draft);
  const base = `draft-${draft.id}`;
  const titleId = `${base}-title`;
  const pill = PILL[draft.state];
  let site = updated?.initiated_on ?? '';
  let branch = 'main';
  try {
    const location = parseLocation(site);
    site = location.scope;
    if (location.pin.kind === 'branch') branch = printPin(location.pin);
  } catch {
    // An unreadable `initiated_on` is shown as written.
  }
  const flowPart = updated?.parts.find((part) => /\/(flow|map)$/.test(part.scope));
  const showRoute = flowPart ? `/w/${flowPart.scope}@${branch}${flowPart.scope.endsWith('/flow') ? `?draft=${draft.id}` : ''}` : null;
  const showWord = flowPart?.scope.endsWith('/flow') ? 'Flow' : 'Map';
  const errorId = `${base}-error`;
  const described = error !== null ? errorId : undefined;

  const act = async (what: 'apply' | 'discard') => {
    const outcome = what === 'apply' ? await api.apply(draft.id) : await api.discard(draft.id);
    if (outcome.ok) {
      shell.focusComposer();
      if (what === 'discard') shell.announce('Discarded.');
      else if (outcome.said !== undefined) shell.announce(outcome.said);
    } else {
      setError(outcome.words);
      shell.announce(outcome.words);
    }
  };

  return (
    <div className="chat-card changeset-card" data-state={draft.state}>
      <h4 className="chat-card__title" id={titleId} tabIndex={-1}>
        Draft changeset <span className="pill" data-tone={pill.tone}>{pill.words}</span>
      </h4>
      {updated !== null && (
        <>
          <p className="changeset-card__base">
            Built against {changesetsWords(updated.bases)} of <code>{site}</code>, branch {branch}.
          </p>
          <p className="changeset-card__check">
            {updated.checked_by === 'world-proto-server' ? 'Checked by world-proto-server.' : 'Checked by Agent Studio; world-proto-server checks again when it is applied.'}
          </p>
          {updated.parts.map((part) => {
            const scopeWord = part.scope.split('/').pop() ?? part.scope;
            const label = `${scopeWord[0]?.toUpperCase() ?? ''}${scopeWord.slice(1)}`;
            return (
              <Fragment key={part.scope}>
                <p className="changeset-card__scope">{label}: {plural(part.ops_total, 'change')}</p>
                <ol className="changeset-card__ops" aria-label={`${plural(part.ops_total, 'change')} to the ${scopeWord}`}>
                  {part.ops.slice(0, SHOWN_OPS).map((op, n) => <Op key={n} op={op} />)}
                </ol>
                {part.ops_total > SHOWN_OPS && <AllOps draft={draft.id} scope={part.scope} total={part.ops_total} />}
              </Fragment>
            );
          })}
          {updated.problems.length > 0 && (
            <ul aria-label="Problems">
              {updated.problems.map((problem, n) => (
                <li className="problem" data-severity={problem.severity} key={n}>
                  <span className="problem__kind">{problem.severity === 'error' ? 'Error:' : 'Warning:'}</span> {problem.text}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
      {draft.state === 'applied' && <p>Applied as {changesetsWords(appliedOf(draft)?.seqs ?? {})}.</p>}
      {draft.state === 'stale' && <p>{staleWords(stale)}</p>}
      {stale?.reason === 'generators-moved' && !api.readOnly && (
        <div className="chat-card__actions">
          <button className="btn" type="button" onClick={() => void api.send(REBUILD)}>{REBUILD}</button>
        </div>
      )}
      {execution !== null && <ExecutionLine approval={execution} />}
      {(draft.state === 'drafting' || draft.state === 'ready') && !api.readOnly && (
        <>
          <div className="chat-card__actions">
            {draft.state === 'drafting' || applying ? (
              <button className="btn btn--primary" type="button" aria-disabled="true" aria-describedby={`${base}-why ${described ?? ''}`.trim()}>
                {applying ? 'Applying' : 'Apply to the site'}
              </button>
            ) : (
              <button className="btn btn--primary" type="button" aria-describedby={described} onClick={() => void act('apply')}>Apply to the site</button>
            )}
            <button className="btn" type="button" aria-describedby={described} onClick={() => void act('discard')}>Discard</button>
            {showRoute !== null && <ShellLink className="btn btn--quiet" to={showRoute}>Show on the {showWord} panel</ShellLink>}
          </div>
          <p className="chat-card__footnote" id={`${base}-why`}>
            {draft.state === 'drafting' ? 'You can apply once the turn finishes.' : 'Applying writes one changeset to the shared site.'}
          </p>
        </>
      )}
      {error !== null && <p className="chat-card__error" id={errorId}>{error}</p>}
    </div>
  );
}
