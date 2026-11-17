// Spec 01 section 3.11's person's draft as the Flow panel drives it (spec 06 section 3.4): found with `mine=1`, made
// by the first detail-pane edit at the cut the panel's read resolved (its `seqs`, so a site moved since is refused
// rather than adopted), amended at the revision last read (a stale revision read again and sent once more), applied
// by a press whose approval is read once a second of wall clock until its execution ends, or discarded. A write in
// doubt (`unknown`) has not ended: the API asks world-proto-server again until it knows.

import type { ApprovalAnswer, ApprovalRecord } from '../../protocol/approval.ts';
import type { ChangesetOp, DraftChecked, DraftList, DraftRead } from '../../protocol/changeset_draft.ts';
import type { ScopePath, Seq } from '../../protocol/grammar.ts';
import { getJson, type Answer } from '../../shell/api.ts';
import { sendJson } from '../template/requests.ts';

/** The person's open draft on `site`, if they hold one. */
export async function ownDraft(site: string): Promise<Answer<DraftRead | null>> {
  const answer = await getJson<DraftList>(`/api/drafts?site=${encodeURIComponent(site)}&mine=1`);
  return answer.ok ? { ok: true, status: answer.status, value: answer.value.items[0] ?? null } : answer;
}

/**
 * Add `op` to the person's draft on `site`, making the draft at `read` (the site at the panel's pin) and `seqs` (the
 * changesets the displayed read resolved) when none is open.
 */
export async function addEdit(
  site: string, read: string, seqs: { [key in ScopePath]: Seq }, op: ChangesetOp, summary: string,
): Promise<Answer<DraftChecked>> {
  const held = await ownDraft(site);
  if (!held.ok) return held;
  if (held.value === null) return sendJson<DraftChecked>('POST', '/api/drafts', { site: read, seqs, summary, ops: [op] });
  const amend = (revision: number) => sendJson<DraftChecked>('PATCH', `/api/drafts/${held.value?.draft}`, { revision, ops: [op] });
  const first = await amend(held.value.revision);
  if (first.ok || first.problem?.type !== 'draft-changed') return first;
  const again = await getJson<DraftRead>(`/api/drafts/${held.value.draft}`);
  return again.ok ? amend(again.value.revision) : again;
}

const ENDED = new Set(['ok', 'failed', 'interrupted']);

/**
 * Press Apply, then read the approval once a second (wall clock) until its execution ends, for up to two minutes; the
 * approval as last read, its execution maybe still running or unknown, when it has not ended by then.
 */
export async function pressApply(draft: string): Promise<Answer<ApprovalRecord>> {
  const pressed = await sendJson<ApprovalAnswer>('POST', `/api/drafts/${draft}/apply`);
  if (!pressed.ok) return pressed;
  let approval = pressed.value.approval;
  for (let tries = 0; tries < 120 && !ENDED.has(approval.execution ?? ''); tries += 1) {
    await new Promise((resolve) => setTimeout(resolve, 1000));
    const read = await getJson<ApprovalAnswer>(`/api/approvals/${approval.id}`);
    if (!read.ok) return read;
    approval = read.value.approval;
  }
  return { ok: true, status: pressed.status, value: approval };
}

export const discard = (draft: string): Promise<Answer<DraftRead>> => sendJson<DraftRead>('POST', `/api/drafts/${draft}/discard`);
