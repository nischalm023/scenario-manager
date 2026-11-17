// Spec 05 section 3.8's following, for a draft read as `writing` (a reload, or another browser tab): the form follows
// the approval the draft's write names, once a second of wall clock, through `unknown`, which it says and does not end
// on, to the execution's end; then it reads the draft for what it made and has the panel read the draft again.

import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Execution } from '../../protocol/projection.ts';
import type { TemplateDraft } from '../../protocol/template_draft.ts';
import { CreateSiteForm } from './CreateSiteForm.tsx';

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const DRAFT = '01M5B8ZT3QHRDJVGFSAB828HYY';
const APPROVAL = '01M5B8ZTRXZ5CHQECVHH9XF2DQ';
const SITE = 'customer/demo/site/rect-1';

function draftIn(state: TemplateDraft['state'], made: boolean): TemplateDraft {
  const seqs = { [SITE]: 1 };
  return {
    draft: DRAFT, state, mode: 'create', revision: 1,
    template: { ref: 'template/site/rectangle', tag: null, seqs: { 'template/site/rectangle': 1 } },
    title: 'Rectangle', conversation: null, knobs: {}, set_by: {}, declarations: [], latest: { n: 1, state: 'ready' },
    write: { approval: APPROVAL, evaluation: 1, result: made ? { site: SITE, seqs, made_from: APPROVAL } : null, problem: null },
  };
}

const answer = (body: unknown) => new Response(JSON.stringify(body), { status: 200 });

describe('spec 05 section 3.8: Create site follows its write until the execution ends', () => {
  it('follows a writing draft\'s approval through unknown to ok, then reads the draft and has the panel read it', async () => {
    vi.useFakeTimers();
    const executions: Execution[] = ['running', 'unknown', 'unknown', 'ok'];
    const read: string[] = [];
    vi.stubGlobal('fetch', vi.fn(async (path: string) => {
      read.push(path);
      if (path === `/api/approvals/${APPROVAL}`) {
        const execution = executions.shift() ?? 'ok';
        return answer({ approval: { id: APPROVAL, execution, sentence: execution === 'unknown' ? 'not yet known' : null } });
      }
      return answer(draftIn('created', true));
    }));
    const created = vi.fn();
    const settled = vi.fn();
    render(
      <CreateSiteForm
        draft={draftIn('writing', false)} evaluation={null} unavailable={null} browserTab="tab"
        announce={() => undefined} onCreated={created} onSettled={settled}
      />,
    );
    expect(screen.getByText('Creating the site…')).not.toBeNull();
    for (let second = 0; second < 3; second += 1) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1000);
      });
    }
    expect(screen.getByText('Not yet known whether the site was made; checking again.')).not.toBeNull();
    expect(read.filter((path) => path.startsWith('/api/template-drafts/'))).toEqual([]);
    expect(created).not.toHaveBeenCalled();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(created).toHaveBeenCalledWith(SITE);
    expect(settled).toHaveBeenCalledTimes(1);
    expect(read.filter((path) => path === `/api/template-drafts/${DRAFT}`)).toHaveLength(1);
  });
});
