// A press of Run in the Scenario panel (spec 07 section 4.3; spec 08 section 4.2, a person presses Run), in event
// order: the experiment; the plan, asked with the cut the readiness line showed and read once a second (wall clock)
// until it is ready; the submission with the plan's hash, which the press approves; then that approval, read once a
// second until its execution ends. Only an execution that ended `ok` says the runs were submitted; any refusal says
// its own reason. The requests are handed in, so the unit tests drive every step.

import type { ApprovalAnswer } from '../../protocol/approval.ts';
import type { Experiment, ExperimentRequest } from '../../protocol/experiment.ts';
import type { PlanAccepted, PlanRead, PlanRequest, SubmitRequest } from '../../protocol/plan.ts';
import type { Execution } from '../../protocol/projection.ts';
import type { UnitCut } from '../../protocol/workload.ts';
import { clause, type Answer } from '../../shell/api.ts';
import { runsWords } from './words.ts';

export interface PressRequests {
  /** A POST with a fresh Idempotency-Key. */
  post<T>(path: string, body: unknown): Promise<Answer<T>>;
  get<T>(path: string): Promise<Answer<T>>;
  /** Wait `ms` of wall clock; false once the panel is gone, which ends the press where it stands. */
  wait(ms: number): Promise<boolean>;
}

export type PressStep = 'preparing' | 'submitting';

export type PressOutcome =
  | { kind: 'submitted'; experiment: Experiment }
  | { kind: 'not-submitted'; experiment: Experiment | null; words: string }
  | { kind: 'left' };

const EVERY_MS = 1000;
/** An approved act's execution that has not ended: `unknown` is checked again by the service (spec 02 section 3.8). */
const PENDING: (Execution | null)[] = [null, 'not_started', 'running', 'unknown'];
/** A refusal the service answers again if asked again; a network fault or a 5xx is read past. */
const final = (status: number | null): boolean => status !== null && status < 500;

export async function press(
  requests: PressRequests,
  request: ExperimentRequest,
  cut: UnitCut['seqs'] | null,
  on: { prepared(experiment: Experiment): void; step(step: PressStep): void },
): Promise<PressOutcome> {
  const prepared = await requests.post<Experiment>('/api/experiments', request);
  if (!prepared.ok) return { kind: 'not-submitted', experiment: null, words: `The runs were not prepared: ${clause(prepared.reason)}.` };
  const experiment = prepared.value;
  on.prepared(experiment);
  const refused = (why: string): PressOutcome => ({
    kind: 'not-submitted', experiment, words: `${runsWords(experiment.run_order.length)} prepared, not submitted: ${clause(why)}.`,
  });

  on.step('preparing');
  const planned = { experiment: experiment.id, ...(cut !== null ? { cut } : {}) } satisfies PlanRequest;
  const asked = await requests.post<PlanAccepted>('/api/runs/plan', planned);
  if (!asked.ok) return refused(asked.reason);
  const plan = asked.value.plan;
  let hash: string | null = null;
  while (hash === null) {
    const read = await requests.get<PlanRead>(`/api/runs/plan/${plan}`);
    if (!read.ok && final(read.status)) return refused(read.reason);
    if (read.ok && read.value.state === 'failed') return refused(read.value.problem?.detail ?? 'its check failed');
    if (read.ok && read.value.state === 'ready') {
      hash = read.value.plan_sha256 ?? null;
      if (hash === null) return refused('the ready plan named no hash');
    } else if (!(await requests.wait(EVERY_MS))) return { kind: 'left' };
  }

  on.step('submitting');
  const submitted = await requests.post<ApprovalAnswer>('/api/runs', { plan, plan_sha256: hash } satisfies SubmitRequest);
  if (!submitted.ok) return refused(submitted.reason);
  const approval = submitted.value.approval.id;
  for (;;) {
    const read = await requests.get<ApprovalAnswer>(`/api/approvals/${approval}`);
    if (!read.ok && final(read.status)) return refused(read.reason);
    if (read.ok) {
      const { execution, sentence } = read.value.approval;
      if (execution === 'ok') return { kind: 'submitted', experiment };
      if (!PENDING.includes(execution)) return refused(sentence ?? `its execution ended ${execution}`);
    }
    if (!(await requests.wait(EVERY_MS))) return { kind: 'left' };
  }
}
