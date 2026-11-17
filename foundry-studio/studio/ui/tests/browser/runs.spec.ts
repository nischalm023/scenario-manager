// Spec 08 section 8 in the browser, against dev.sh with the development seed written (brief I1.09's Done
// when): each shell's site made at its defaults runs Peak hour's check (every order at the start) from the Scenario
// panel's Run to `finished` on simrunner, with no `plan-differs` and the stub's sentence on its card (statement
// 8's first-cut half); six runs hold at most five seats, the sixth waiting first in the queue; a run's page opened
// before its first frame shows it live soon after it streams, and attaches again on the same player, its stage and
// focus kept, when its picture's attachment is lost; its numbers come live from the streaming server, its live view
// fills the window with them, and beside the open chat it draws its floor at a usable width; runs submitted either
// side of a flow changeset each record the changeset they read (spec 07 statement 9); axe finds no violation on
// `/runs` or on a running run's page, whose Cancel keeps focus reading "Stopping…" and hands it to the run's heading
// as the run ends; and a filter's radio keeps focus as the list changes. The stack runs with `STUDIO_RUNNER_HOLD_MS`
// set in the API's environment (15000 here) and named in `STUDIO_RUNNER_PASSED`, so the invoker hands it to every
// runner and a run holds its seat long enough to be seen running or waiting.

import AxeBuilder from '@axe-core/playwright';
import { readFileSync } from 'node:fs';
import { expect, test, type Page, type WebSocketRoute } from '@playwright/test';

const STUB = 'stub runner: canned results; no simulation ran';
const SHELLS = ['square', 'rectangle', 'l-shape', 'u-shape', 'i-shape'];

test.setTimeout(240_000);

function wpsUrl(): string {
  if (process.env.STUDIO_WPS_URL) return process.env.STUDIO_WPS_URL;
  const ports = readFileSync(new URL('../../../.dev-state/ports', import.meta.url), 'utf8');
  return `http://127.0.0.1:${/^WPS_PORT=(\d+)$/m.exec(ports)?.[1]}`;
}

async function json<T>(answer: { ok(): boolean; status(): number; text(): Promise<string> }, what: string): Promise<T> {
  const text = await answer.text();
  if (!answer.ok()) throw new Error(`${what} answered ${answer.status()}: ${text}`);
  return JSON.parse(text) as T;
}

/** A site made from `shell` at its defaults through Create site, under a customer of its own. */
async function madeSite(page: Page, shell: string): Promise<string> {
  const started = await json<{ draft: string }>(await page.request.post('/api/template-drafts', {
    data: { template: `template/site/${shell}` }, headers: { 'idempotency-key': crypto.randomUUID() },
  }), 'starting a draft');
  let evaluation = 0;
  await expect.poll(async () => {
    const draft = await json<{ latest: { n: number; state: string } | null }>(await page.request.get(`/api/template-drafts/${started.draft}`), 'the draft');
    evaluation = draft.latest?.n ?? 0;
    return draft.latest?.state;
  }, { timeout: 60_000 }).toBe('ready');
  const customer = `r-${Math.random().toString(36).slice(2, 10)}`;
  const accepted = await json<{ approval: { id: string } }>(await page.request.post(`/api/template-drafts/${started.draft}/site`, {
    data: { customer, slug: shell, name: `Runs ${shell} ${customer}`, evaluation }, headers: { 'idempotency-key': crypto.randomUUID() },
  }), 'Create site');
  await expect.poll(async () => {
    const read = await json<{ approval: { execution: string } }>(await page.request.get(`/api/approvals/${accepted.approval.id}`), 'the approval');
    return read.approval.execution;
  }, { timeout: 60_000 }).toBe('ok');
  return `customer/${customer}/site/${shell}`;
}

/** Peak hour made on `site` through the Scenario panel, ready to run. */
async function peakHour(page: Page, site: string): Promise<string> {
  await page.goto(`/scenarios?site=${encodeURIComponent(site)}`);
  await page.getByRole('button', { name: 'New scenario' }).click();
  await page.getByLabel('Name of the new scenario').fill('Peak hour');
  await page.getByRole('button', { name: 'Create scenario' }).click();
  await expect(page).toHaveURL(/\/scenarios\/[0-9A-Z]{26}$/);
  await expect(page.locator('#sc-readiness')).toContainText('Ready to run', { timeout: 90_000 });
  return new URL(page.url()).pathname.split('/').pop() ?? '';
}

interface Plan { id: string; state: string; plan_sha256: string | null; plan: { runs: { run: string }[] } | null }

/** Peak hour's runs submitted through the API as a press does: an experiment of `values`, its plan, the press. */
async function submitted(page: Page, scenario: string, values: number[] = []): Promise<string[]> {
  const key = () => ({ 'idempotency-key': crypto.randomUUID() });
  const read = await json<{ version: { n: number; revision: number } }>(await page.request.get(`/api/scenarios/${scenario}`), 'the scenario');
  const varies = values.length === 0 ? {} : {
    varies: { target: `scenario:${scenario}`, pointer: '/workload/generator/orders_per_hour', values },
  };
  const experiment = await json<{ id: string }>(await page.request.post('/api/experiments', {
    data: { scenario, version: read.version.n, revision: read.version.revision, ...varies }, headers: key(),
  }), 'the experiment');
  const asked = await json<{ plan: string }>(await page.request.post('/api/runs/plan', { data: { experiment: experiment.id }, headers: key() }), 'the plan');
  let plan: Plan | null = null;
  await expect.poll(async () => {
    plan = await json<Plan>(await page.request.get(`/api/runs/plan/${asked.plan}`), 'the plan');
    return plan.state;
  }, { timeout: 120_000 }).toBe('ready');
  const ready = plan as unknown as Plan;
  await json(await page.request.post('/api/runs', { data: { plan: ready.id, plan_sha256: ready.plan_sha256 }, headers: key() }), 'the submission');
  return ready.plan?.runs.map((run) => run.run) ?? [];
}

const stateOf = async (page: Page, run: string): Promise<string> =>
  (await json<{ state: string }>(await page.request.get(`/api/runs/${run}`), 'the run')).state;

async function noViolation(page: Page, what: string) {
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    const results = await new AxeBuilder({ page }).include('#panel').analyze();
    const found = results.violations.map((violation) => `${violation.id}: ${violation.nodes.map((node) => node.target.join(' ')).slice(0, 3).join(' | ')}`);
    expect(found, `${what}, ${scheme}`).toEqual([]);
  }
}

for (const shell of SHELLS) {
  test(`${shell} at its defaults runs Peak hour's check from Run to finished, the stub's sentence on its card`, async ({ page }) => {
    const site = await madeSite(page, shell);
    await peakHour(page, site);
    const run = page.locator('#sw-run');
    await run.click();
    await expect(page.locator('#sw-outcome')).toHaveText('Submitted 1 run. Open the Runs panel', { timeout: 120_000 });
    const card = page.locator('aside#chat .run-card').last();
    await expect(card.locator('.chat-card__title .pill')).toHaveText(/finished/, { timeout: 120_000 });
    await expect(card.locator('.run-card__detail')).toHaveText(STUB);
    await expect(card.getByText('differs from the check')).toHaveCount(0);
    const runs = await json<{ items: { id: string; state: string; findings: string[]; stop_detail?: string }[] }>(
      await page.request.get(`/api/runs?site=${encodeURIComponent(site)}`), 'the runs',
    );
    expect(runs.items.map((item) => [item.state, item.findings, item.stop_detail])).toEqual([['finished', [], STUB]]);
    await card.getByRole('link', { name: 'Open in Runs' }).click();
    await expect(page).toHaveURL(new RegExp(`/runs/${runs.items[0].id}$`));
    await expect(page.locator('#run-title')).toContainText('Peak hour');
    await expect(page.locator('#run-stop-detail')).toHaveText(STUB);
    await expect(page.locator('table.runs-table tr:has(a[aria-current="page"]) code')).toHaveText(runs.items[0].id.slice(0, 8));
  });
}

test('six runs hold at most five seats, the sixth waiting first in the queue until a seat frees', async ({ page }) => {
  const site = await madeSite(page, 'rectangle');
  const scenario = await peakHour(page, site);
  let most = 0;
  let sixthWaited = false;
  let watching = true;
  const watch = (async () => {
    while (watching) {
      const queue = await json<{ going: unknown[]; waiting: { waiting?: { place_in_queue?: number } }[] }>(await page.request.get('/api/runs/queue'), 'the queue');
      most = Math.max(most, queue.going.length);
      if (queue.going.length === 5 && queue.waiting[0]?.waiting?.place_in_queue === 1) sixthWaited = true;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  })();
  const runs = await submitted(page, scenario, [100, 150, 200, 250, 300, 350]);
  expect(runs).toHaveLength(6);
  await page.goto(`/runs?site=${encodeURIComponent(site)}`);
  await expect(page.locator('#runs-queue')).toHaveText(/^Queue: \d runs? going, (none|\d) waiting; at most 5 go at a time in Agent Studio\.$/);
  for (const run of runs) await expect.poll(() => stateOf(page, run), { timeout: 120_000 }).toBe('finished');
  watching = false;
  await watch;
  expect(most, 'never more than five seats held').toBeLessThanOrEqual(5);
  expect(sixthWaited, 'the sixth waited first in the queue while five held seats (STUDIO_RUNNER_HOLD_MS set?)').toBe(true);
  await expect(page.locator('table.runs-table tbody tr')).toHaveCount(6, { timeout: 10_000 });
});

/** The run's progress in simtime, nothing before the press has made the run. */
async function progressOf(page: Page, run: string): Promise<number> {
  const answer = await page.request.get(`/api/runs/${run}`);
  return answer.status() === 404 ? 0 : (await json<{ progress: { sim_ms: number } }>(answer, 'the run')).progress.sim_ms;
}

/** The streaming server refuses an attach until it holds the run's first frame, and the page asks again 2 s on. */
test("a run's page opened before the run's first frame shows it live within 5 s of its first progress", async ({ page }) => {
  const site = await madeSite(page, 'rectangle');
  const scenario = await peakHour(page, site);
  const [run] = await submitted(page, scenario);
  await page.goto(`/runs/${run}`);
  expect(await progressOf(page, run), 'the page opened before the run reported any progress').toBe(0);
  await expect.poll(() => progressOf(page, run), { timeout: 120_000 }).toBeGreaterThan(0);
  await expect(page.locator('#run-playback-mode .pill')).toHaveText(/^Live, as of /, { timeout: 5_000 });
});

/**
 * Each ask once made a new player: a new viewer, socket and stage, and focus in the old one fell to the page. The
 * metric panels ask for watch tickets too, so the spec counts the picture's own frames attachments: it holds the first
 * open with nothing sent, closes it once the stage stands and the Fill toggle has focus, and passes every later one
 * through to the streaming server.
 */
test("a run's page whose picture's attachment is lost attaches again on one player, keeping focus in it", async ({ page }) => {
  const site = await madeSite(page, 'rectangle');
  const scenario = await peakHour(page, site);
  const [run] = await submitted(page, scenario);
  const attachments: WebSocketRoute[] = [];
  await page.routeWebSocket(/\/frames\//, (attachment) => {
    attachments.push(attachment);
    if (attachments.length > 1) attachment.connectToServer();
  });
  await page.goto(`/runs/${run}`);
  const fill = page.getByRole('button', { name: 'Fill the window' });
  await fill.focus({ timeout: 60_000 });
  await expect.poll(() => attachments.length, { timeout: 10_000 }).toBe(1);
  const stage = await page.locator('.worldview-mount__stage').elementHandle();
  await attachments[0].close();
  await expect(page.getByRole('button', { name: 'Retry' }), 'a closed socket offers Retry').toBeVisible();
  await expect.poll(() => attachments.length, { timeout: 10_000 }).toBeGreaterThan(1);
  await expect(page.locator('#run-playback-mode .pill')).toHaveText(/^Live, as of /, { timeout: 120_000 });
  expect(await stage?.evaluate((element) => element.isConnected), 'the same stage').toBe(true);
  await expect(fill).toBeFocused();
});

/**
 * The streaming server closes a watch that states a format other than its own, as the page's once stated an older one.
 */
test("a running run's numbers come live from the streaming server", async ({ page }) => {
  const site = await madeSite(page, 'rectangle');
  const scenario = await peakHour(page, site);
  const [run] = await submitted(page, scenario);
  await page.goto(`/runs/${run}`);
  await expect(page.getByRole('heading', { level: 4, name: 'Live as the runner reports it' })).toBeVisible({ timeout: 60_000 });
});

/**
 * Ruling D192 (the UI fast-follow after PR 600): a floor fills the window with what its page keeps beside it, and
 * returns by Escape.
 */
test("a run's live view fills the window with its numbers beside it, a region of its own, the rest hidden, and returns by Escape", async ({ page }) => {
  const site = await madeSite(page, 'rectangle');
  const scenario = await peakHour(page, site);
  const [run] = await submitted(page, scenario);
  await page.goto(`/runs/${run}`);
  const fill = page.getByRole('button', { name: 'Fill the window' });
  await fill.click({ timeout: 60_000 });
  await expect(fill).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('region', { name: /^Live view of .+, filling the window$/ })).toBeVisible();
  const whole = await page.evaluate(() => `0 0 ${document.documentElement.clientWidth} ${document.documentElement.clientHeight}`);
  const covered = await page.locator('.run-live').evaluate((element) => {
    const box = element.getBoundingClientRect();
    return [box.x, box.y, box.width, box.height].map(Math.round).join(' ');
  });
  expect(covered).toBe(whole);
  await expect(page.locator('#chat')).toBeHidden();
  await expect(page.locator('.run-live__numbers')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(fill).toHaveAttribute('aria-pressed', 'false');
  await expect(page.locator('#chat')).toBeVisible();
});

/**
 * Ruling D192 (the UI fast-follow after PR 600): beside the open chat, a run's live view draws its floor at least 20rem
 * wide, or as wide as the view where it is narrower, the floor's toolbar clear of the numbers; at 1024 px its two
 * columns once left the floor about 0 px wide, as the site's map once did.
 */
test("beside the open chat, a run's live view draws its floor at a usable width, its toolbar clear of the numbers", async ({ page }) => {
  const site = await madeSite(page, 'rectangle');
  const scenario = await peakHour(page, site);
  const [run] = await submitted(page, scenario);
  await page.goto(`/runs/${run}`);
  // The floor is measured once its player has attached: before the player has its ticket, a sentence stands in the
  // floor's place.
  await expect(page.locator('#run-playback-mode .pill')).toHaveText(/^(Live, as of|Replay)/, { timeout: 120_000 });
  type Box = { x: number; y: number; width: number; height: number };
  const box = async (selector: string): Promise<Box> => {
    const found = await page.locator(selector).first().boundingBox();
    if (found === null) throw new Error(`${selector} is not drawn`);
    return found;
  };
  const apart = (a: Box, b: Box) => a.x + a.width <= b.x || b.x + b.width <= a.x || a.y + a.height <= b.y || b.y + b.height <= a.y;
  for (const width of [1024, 1280, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(page.locator('#chat')).toBeVisible();
    const [whole, floor, tools, numbers] = [
      await box('.run-live'), await box('.run-live .worldview-mount__stage'), await box('.run-live .stage-tools'), await box('.run-live__numbers'),
    ];
    expect(floor.width, `at ${width}`).toBeGreaterThanOrEqual(Math.min(320, Math.floor(whole.width)));
    expect(apart(tools, numbers), `at ${width}: the toolbar stands clear of the numbers`).toBe(true);
  }
});

test('runs submitted either side of a flow changeset each record the changeset they read', async ({ page }) => {
  const site = await madeSite(page, 'rectangle');
  const scenario = await peakHour(page, site);
  const flow = `${site}/flow`;
  const [first] = await submitted(page, scenario);
  // A flow edit lands: the pick node's pick time, set through world-proto-server as an applied draft would.
  const exported = await (await fetch(`${wpsUrl()}/scopes/${flow}/-/export`)).json() as {
    entities: { id: string; components: Record<string, { process?: { operation?: string }; work?: Record<string, unknown> }> }[];
  };
  const pick = exported.entities.find((entity) => entity.components['flow.node']?.process?.operation === 'pick');
  if (pick === undefined) throw new Error(`${flow} has no pick node`);
  const node = { ...pick.components['flow.node'], work: { ...pick.components['flow.node'].work, pick_sim_ms: 7500 } };
  const listing = await (await fetch(`${wpsUrl()}/scopes?under=${encodeURIComponent(site)}`)).json() as { scopes: { scope: string; head: number }[] };
  const head = listing.scopes.find((scope) => scope.scope === flow)?.head ?? 0;
  const posted = await fetch(`${wpsUrl()}/scopes/${flow}/-/changesets`, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ scope: flow, base: head, author: 'dev', message: 'A slower pick between two submissions', ops: [{ op: 'patch', id: pick.id, components: { 'flow.node': node } }] }),
  });
  expect(posted.ok, await posted.text()).toBe(true);
  const [second] = await submitted(page, scenario);
  for (const run of [first, second]) await expect.poll(() => stateOf(page, run), { timeout: 120_000 }).toBe('finished');
  const reads = async (run: string) => (await json<{ reads: Record<string, number> }>(await page.request.get(`/api/runs/${run}`), 'the run')).reads[flow];
  expect([await reads(first), await reads(second)]).toEqual([head, head + 1]);
  await page.goto(`/runs/${second}`);
  await expect(page.locator('.provenance')).toContainText(`${flow} at changeset ${head + 1}`);
  const read = await json<{ version: { n: number; revision: number } }>(await page.request.get(`/api/scenarios/${scenario}`), 'the scenario');
  expect([read.version.n, read.version.revision]).toEqual([1, 1]);
});

test("axe finds no violation on /runs or on a running run's page, whose Cancel keeps focus until the run ends", async ({ page }) => {
  const site = await madeSite(page, 'rectangle');
  const scenario = await peakHour(page, site);
  const [run] = await submitted(page, scenario);
  await page.goto(`/runs/${run}`);
  await expect(page.locator('#run-title')).toBeVisible();
  await expect(page.locator('#run-status .pill').first()).toHaveText(/running/, { timeout: 60_000 });
  await expect(page.locator('progress#run-progress')).toHaveAccessibleName('Simulated so far, of 4 h simtime');
  await expect(page.locator('table.runs-table tr:has(a[aria-current="page"]) code')).toHaveText(run.slice(0, 8));
  await noViolation(page, 'a running run');
  // Spec 08 section 5: Cancel never asks, keeps focus reading "Stopping…", and hands focus to the run's h3 as it leaves.
  const panel = page.locator('#panel');
  await panel.getByRole('button', { name: 'Cancel the run' }).focus();
  await page.keyboard.press('Enter');
  await expect(panel.getByRole('button', { name: 'Stopping…' })).toBeFocused();
  await expect.poll(() => stateOf(page, run), { timeout: 60_000 }).toBe('cancelled');
  await expect(page.locator('#run-title')).toBeFocused({ timeout: 10_000 });
  await expect(page.locator('#run-status')).toContainText('cancelled at');
  await page.goto(`/runs?site=${encodeURIComponent(site)}`);
  await expect(page.locator('table.runs-table')).toBeVisible();
  await noViolation(page, '/runs');
  // A filter's radio keeps focus as the list it chooses replaces the other.
  await panel.getByRole('radio', { name: 'This site' }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(page).toHaveURL(/[?&]filter=mine/);
  await expect(panel.getByRole('radio', { name: 'All mine' })).toBeFocused();
  await expect(page.locator('table.runs-table caption')).toContainText('Your runs');
});
