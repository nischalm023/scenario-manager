// A press of Run (spec 07 section 4.3; spec 08 section 4.2) against scripted answers: it goes plan, ready,
// submission, execution in order, reading the plan and the approval once a second, says the runs were submitted only
// once the execution ends ok, and otherwise says why not in the refusal's own words.

import { describe, expect, it } from 'vitest';
import type { ApprovalAnswer } from '../../protocol/approval.ts';
import type { Experiment } from '../../protocol/experiment.ts';
import type { Answer } from '../../shell/api.ts';
import { press, type PressRequests, type PressStep } from './press.ts';

const experiment: Experiment = {
  id: '01KQ7Z0D1C9W8V0R5M3S6T2Y4X', scenario: '01KK8AJ7RYA6GNNQ3QTRQCW7Z8', version: 1, revision: 1, sweep: null,
  variants: [], seeds: [0], run_overrides: {}, kind: 'sweep', submission: null,
  run_order: [1, 2, 3, 4, 5].map((variant) => ({ variant, seed: 0 })),
};
const request = { scenario: experiment.scenario, version: 1, revision: 1 };
/** The press's `202`: the approval's record as every button act answers it, approved, its execution not started. */
const pressed: ApprovalAnswer = {
  approval: {
    id: 'APPROVAL', conversation: null, tool: 'run.submit', class: 'submit', via: 'button', state: 'approved',
    execution: 'not_started', sentence: null, outcome: null, requested_at: '2026-10-20T12:00:00.000Z',
    decided_by: { subject: 'dev', name: 'Developer' }, decided_at: '2026-10-20T12:00:00.000Z',
  },
};
const ok = <T,>(value: T, status = 200): Answer<T> => ({ ok: true, status, value });
const refused = (status: number, reason: string): Answer<never> => ({ ok: false, status, problem: null, reason });

/** Answers by path, each path's list taken in order and its last answer repeated; every request and wait recorded. */
function scripted(answers: Record<string, Answer<unknown>[]>) {
  const asked: string[] = [];
  const answer = (method: string, path: string) => {
    asked.push(`${method} ${path}`);
    const queue = answers[`${method} ${path}`];
    if (queue === undefined) throw new Error(`nothing scripted for ${method} ${path}`);
    return Promise.resolve((queue.length > 1 ? queue.shift() : queue[0]) as Answer<never>);
  };
  const requests: PressRequests = {
    post: (path) => answer('POST', path),
    get: (path) => answer('GET', path),
    wait: (ms) => {
      asked.push(`wait ${ms}`);
      return Promise.resolve(true);
    },
  };
  return { asked, requests };
}

describe('a press of Run', () => {
  it("says a refused plan's own reason and asks nothing more", async () => {
    const { asked, requests } = scripted({
      'POST /api/experiments': [ok(experiment, 201)],
      'POST /api/runs/plan': [refused(409, 'These runs were submitted already; prepare a new experiment to run them again.')],
    });
    const outcome = await press(requests, request, null, { prepared: () => {}, step: () => {} });
    expect(outcome).toEqual({
      kind: 'not-submitted', experiment,
      words: '5 runs prepared, not submitted: These runs were submitted already; prepare a new experiment to run them again.',
    });
    expect(asked).toEqual(['POST /api/experiments', 'POST /api/runs/plan']);
  });

  it('goes plan, ready, submission and execution in order, and is submitted only once the execution ends ok', async () => {
    const { asked, requests } = scripted({
      'POST /api/experiments': [ok(experiment, 201)],
      'POST /api/runs/plan': [ok({ plan: 'PLAN' }, 202)],
      'GET /api/runs/plan/PLAN': [ok({ state: 'preparing' }), ok({ state: 'ready', plan_sha256: 'abc' })],
      'POST /api/runs': [ok(pressed, 202)],
      'GET /api/approvals/APPROVAL': [
        ok({ approval: { execution: 'not_started', sentence: null } }),
        ok({ approval: { execution: 'running', sentence: null } }),
        ok({ approval: { execution: 'ok', sentence: null } }),
      ],
    });
    const steps: PressStep[] = [];
    const told: string[] = [];
    const outcome = await press(requests, request, { 'customer/demo/site/lshape-6/flow': 2 }, {
      prepared: (made) => told.push(made.id), step: (step) => steps.push(step),
    });
    expect(outcome).toEqual({ kind: 'submitted', experiment });
    expect(asked).toEqual([
      'POST /api/experiments', 'POST /api/runs/plan', 'GET /api/runs/plan/PLAN', 'wait 1000', 'GET /api/runs/plan/PLAN',
      'POST /api/runs', 'GET /api/approvals/APPROVAL', 'wait 1000', 'GET /api/approvals/APPROVAL', 'wait 1000',
      'GET /api/approvals/APPROVAL',
    ]);
    expect(steps).toEqual(['preparing', 'submitting']);
    expect(told).toEqual([experiment.id]);
  });

  it("says a failed plan's problem and a failed execution's sentence, and submits nothing after a failed plan", async () => {
    const failedPlan = scripted({
      'POST /api/experiments': [ok(experiment, 201)],
      'POST /api/runs/plan': [ok({ plan: 'PLAN' }, 202)],
      'GET /api/runs/plan/PLAN': [ok({ state: 'failed', problem: { type: 'plan-stale', title: '', status: 409, detail: 'The site moved since the panel read it: flow 3, not 2.' } })],
    });
    const stale = await press(failedPlan.requests, request, null, { prepared: () => {}, step: () => {} });
    expect(stale).toMatchObject({ kind: 'not-submitted', words: '5 runs prepared, not submitted: The site moved since the panel read it: flow 3, not 2.' });
    expect(failedPlan.asked).not.toContain('POST /api/runs');

    const failedRun = scripted({
      'POST /api/experiments': [ok(experiment, 201)],
      'POST /api/runs/plan': [ok({ plan: 'PLAN' }, 202)],
      'GET /api/runs/plan/PLAN': [ok({ state: 'ready', plan_sha256: 'abc' })],
      'POST /api/runs': [ok(pressed, 202)],
      'GET /api/approvals/APPROVAL': [ok({ approval: { execution: 'failed', sentence: 'These runs were already submitted.' } })],
    });
    const failed = await press(failedRun.requests, request, null, { prepared: () => {}, step: () => {} });
    expect(failed).toMatchObject({ kind: 'not-submitted', words: '5 runs prepared, not submitted: These runs were already submitted.' });
  });
});
