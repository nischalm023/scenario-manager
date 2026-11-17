// Spec 08 sections 5 and 8 in the browser against dev.sh (brief I1.10's Done when): a finished stub run's page shows
// its headline tiles, an hourly chart per headline metric each with its table of the same numbers, its recording's
// player, and the stub's sentence describing every number; its inputs copy as the API's own text; the expanded player
// takes the small one's place while it is open; the five runs
// of a sweep compare as differing only in orders per hour, by design; and on both pages axe finds no violation, nothing
// scrolls sideways at 320 CSS px and every time value sits in a `.clock[data-clock]` (statement 13's built half). The
// stack runs with `STUDIO_RUNNER_HOLD_MS` set, as runs.spec.ts says, which only slows these runs.

import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

const STUB = 'stub runner: canned results; no simulation ran';

test.setTimeout(300_000);

async function json<T>(answer: { ok(): boolean; status(): number; text(): Promise<string> }, what: string): Promise<T> {
  const text = await answer.text();
  if (!answer.ok()) throw new Error(`${what} answered ${answer.status()}: ${text}`);
  return JSON.parse(text) as T;
}

const key = () => ({ 'idempotency-key': crypto.randomUUID() });

/** A Rectangle made at its defaults through Create site, under a customer of its own. */
async function madeSite(page: Page): Promise<string> {
  const started = await json<{ draft: string }>(await page.request.post('/api/template-drafts', {
    data: { template: 'template/site/rectangle' }, headers: key(),
  }), 'starting a draft');
  let evaluation = 0;
  await expect.poll(async () => {
    const draft = await json<{ latest: { n: number; state: string } | null }>(await page.request.get(`/api/template-drafts/${started.draft}`), 'the draft');
    evaluation = draft.latest?.n ?? 0;
    return draft.latest?.state;
  }, { timeout: 60_000 }).toBe('ready');
  const customer = `r-${Math.random().toString(36).slice(2, 10)}`;
  const accepted = await json<{ approval: { id: string } }>(await page.request.post(`/api/template-drafts/${started.draft}/site`, {
    data: { customer, slug: 'rectangle', name: `Results ${customer}`, evaluation }, headers: key(),
  }), 'Create site');
  await expect.poll(async () => {
    const read = await json<{ approval: { execution: string } }>(await page.request.get(`/api/approvals/${accepted.approval.id}`), 'the approval');
    return read.approval.execution;
  }, { timeout: 60_000 }).toBe('ok');
  return `customer/${customer}/site/rectangle`;
}

/** Peak hour's runs on `site`, submitted as a press does and waited for until every one finished. */
async function finishedRuns(page: Page, site: string, values: number[] = []): Promise<string[]> {
  const made = await json<{ scenario: string }>(await page.request.post('/api/scenarios', { data: { name: 'Peak hour', site_scope: site }, headers: key() }), 'the scenario');
  const read = await json<{ version: { n: number; revision: number } }>(await page.request.get(`/api/scenarios/${made.scenario}`), 'the scenario');
  const varies = values.length === 0 ? {} : { varies: { target: `scenario:${made.scenario}`, pointer: '/workload/generator/orders_per_hour', values } };
  const experiment = await json<{ id: string }>(await page.request.post('/api/experiments', {
    data: { scenario: made.scenario, version: read.version.n, revision: read.version.revision, ...varies }, headers: key(),
  }), 'the experiment');
  const asked = await json<{ plan: string }>(await page.request.post('/api/runs/plan', { data: { experiment: experiment.id }, headers: key() }), 'the plan');
  let plan: { id: string; plan_sha256: string | null; plan: { runs: { run: string }[] } | null } | null = null;
  await expect.poll(async () => {
    plan = await json(await page.request.get(`/api/runs/plan/${asked.plan}`), 'the plan');
    return (plan as unknown as { state: string }).state;
  }, { timeout: 120_000 }).toBe('ready');
  const ready = plan as unknown as { id: string; plan_sha256: string; plan: { runs: { run: string }[] } };
  await json(await page.request.post('/api/runs', { data: { plan: ready.id, plan_sha256: ready.plan_sha256 }, headers: key() }), 'the submission');
  const runs = ready.plan.runs.map((run) => run.run);
  // The press is answered before its execution makes the runs, so a run reads 404 until it does.
  const stateOf = async (run: string) => {
    const answer = await page.request.get(`/api/runs/${run}`);
    return answer.status() === 404 ? 'not made yet' : (await json<{ state: string }>(answer, 'the run')).state;
  };
  for (const run of runs) await expect.poll(() => stateOf(run), { timeout: 180_000 }).toBe('finished');
  return runs;
}

async function noViolation(page: Page, what: string) {
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    const results = await new AxeBuilder({ page }).include('#panel').analyze();
    const found = results.violations.map((violation) => `${violation.id}: ${violation.nodes.map((node) => node.target.join(' ')).slice(0, 3).join(' | ')}`);
    expect(found, `${what}, ${scheme}`).toEqual([]);
  }
}

/** Every text in the panel that reads as a time and sits outside a `.clock[data-clock]`. */
async function looseTimes(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const panel = document.getElementById('panel');
    const loose: string[] = [];
    if (panel === null) return ['no panel'];
    const walker = document.createTreeWalker(panel, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
      const text = node.textContent ?? '';
      if (/\d+ (s|min|h)\b|\d\d:\d\d/.test(text) && node.parentElement?.closest('.clock[data-clock]') === null) loose.push(text);
    }
    return loose;
  });
}

/** At 320 CSS px wide, nothing scrolls sideways. */
async function noSidewaysScroll(page: Page, what: string) {
  await page.setViewportSize({ width: 320, height: 640 });
  await page.waitForTimeout(300);
  const wider = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(wider, `${what} at 320 CSS px`).toBeLessThanOrEqual(0);
  await page.setViewportSize({ width: 1280, height: 800 });
}

test("a finished stub run's page shows its tiles, a chart and its table per headline metric, the placeholder, and the stub's sentence on every number", async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  const site = await madeSite(page);
  const [run] = await finishedRuns(page, site);
  await page.goto(`/runs/${run}`);
  await expect(page.locator('#run-title')).toContainText('Peak hour');
  const metrics = await json<{ headlines: string[] }>(await page.request.get(`/api/runs/${run}/metrics`), 'the metrics');
  const tiles = page.locator('.metric-tile');
  await expect(tiles).toHaveCount(metrics.headlines.length);
  for (const tile of await tiles.all()) await expect(tile).toHaveAttribute('aria-describedby', 'run-stub-summary');
  await expect(page.locator('#run-stub-summary')).toHaveText(STUB);
  const charts = page.locator('#panel [role="img"]');
  await expect(charts).toHaveCount(metrics.headlines.length);
  for (const chart of await charts.all()) {
    await expect(chart).toHaveAttribute('aria-describedby', /run-stub-hours/);
    expect((await chart.getAttribute('aria-label'))?.length ?? 0).toBeGreaterThan(20);
    await expect(chart.locator('canvas').first()).toBeAttached();
  }
  await expect(page.locator('#run-stub-hours')).toHaveText(STUB);
  await expect(page.getByText('Table of the same numbers')).toHaveCount(metrics.headlines.length);
  for (const twin of await page.locator('figure.chart table.census').all()) await expect(twin).toHaveAttribute('aria-describedby', 'run-stub-hours');
  await expect(page.locator('.run-player')).toHaveAttribute('data-state', 'ready', { timeout: 60_000 });
  const recording = page.getByRole('link', { name: 'Recording (.worldview), canned by the stub runner' });
  await expect(recording).toHaveAttribute('aria-describedby', 'run-stop-detail');
  const download = await page.request.get((await recording.getAttribute('href')) ?? '');
  expect(download.headers()['content-disposition']).toMatch(/^attachment; filename="Peak hour - .* - canned by the stub runner\.worldview"/);

  // The inputs at the top of the page, copied as the API's own text from their fold.
  await page.getByText(/^Where each value came from \(\d+ values; none varied\)$/).click();
  await page.getByRole('button', { name: 'Copy the inputs as text' }).click();
  const text = await (await page.request.get(`/api/runs/${run}/inputs?format=text`)).text();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(text);

  await page.getByText(/^All metrics \(\d+\)$/).click();
  for (const summary of await page.getByText('Table of the same numbers').all()) await summary.click();
  expect(await looseTimes(page)).toEqual([]);
  await noViolation(page, "a finished stub run's page");
  await noSidewaysScroll(page, "a finished stub run's page");
});

/**
 * The expanded player once opened a second viewer beside the small one, decoding the recording a second time; and its
 * Expand button goes with the small player, so closing the dialog once left focus on the page's body.
 */
test("a finished run's expanded player takes the small one's place while it is open, and gives it back with focus", async ({ page }) => {
  const site = await madeSite(page);
  const [run] = await finishedRuns(page, site);
  await page.goto(`/runs/${run}`);
  const players = page.locator('.run-player');
  const heading = page.getByRole('heading', { level: 4, name: 'Playback in simtime' });
  await expect(players).toHaveAttribute('data-state', 'ready', { timeout: 60_000 });
  // Closed by Escape, then by its Close button: either way focus comes back to the playback's heading.
  for (const close of [() => page.keyboard.press('Escape'), () => page.getByRole('button', { name: 'Close the expanded playback' }).click()]) {
    await page.getByRole('button', { name: 'Expand the playback' }).click();
    await expect(page.locator('dialog .run-player')).toHaveAttribute('data-state', 'ready', { timeout: 60_000 });
    await expect(players).toHaveCount(1);
    await expect(page.getByText('Playing in the expanded view.')).toBeVisible();
    await close();
    await expect(page.locator('dialog[open]')).toHaveCount(0);
    await expect(heading).toBeFocused();
    await expect(players).toHaveCount(1);
    await expect(players).toHaveAttribute('data-state', 'ready', { timeout: 60_000 });
  }
});

test('the five runs of a sweep compare as differing only in orders per hour, by design', async ({ page }) => {
  const site = await madeSite(page);
  const runs = await finishedRuns(page, site, [200, 250, 300, 350, 400]);
  await page.goto(`/runs/compare?ids=${runs.join(',')}`);
  await expect(page.getByRole('heading', { level: 3, name: 'Comparison' })).toBeVisible({ timeout: 30_000 });
  await expect(page.getByRole('heading', { level: 4, name: 'What differs', exact: true })).toBeVisible();
  const differs = page.locator('dl.provenance').filter({ hasText: 'Differs, varied by the experiment' });
  await expect(differs.locator('dt:has-text("Differs, varied by the experiment") + dd')).toContainText('Orders per hour: 200 orders an hour on run 1');
  await expect(differs.locator('dt:has-text("Differs outside the experiment") + dd')).toHaveText('nothing');
  await expect(page.locator('#compare-stub')).toHaveText(STUB);
  await expect(page.locator('#panel [role="img"]')).toHaveCount(1);
  await expect(page.locator('#panel [role="img"]')).toHaveAttribute('aria-label', /against orders per hour/);
  const rows = page.getByRole('region', { name: 'Comparison, table' }).locator('tbody tr');
  await expect(rows).toHaveCount(5);
  await expect(rows.first()).toContainText('Run 1, Peak hour, 200 orders per hour (reference)');
  await expect(page.getByText('None: every run finished.')).toBeVisible();
  const reply = await json<{ differs: { section: string; label: string; by_experiment: boolean; warns: boolean }[] }>(
    await page.request.get(`/api/runs/compare?ids=${runs.join(',')}`), 'the comparison',
  );
  expect(reply.differs.filter((field) => field.section !== 'provenance').map((field) => field.label)).toEqual(['Orders per hour']);
  expect(reply.differs.every((field) => field.by_experiment && !field.warns)).toBe(true);
  for (const rate of [200, 250, 300, 350, 400]) {
    await expect(page.getByRole('checkbox', { name: new RegExp(`^Compare run \\w{8}, Peak hour, ${rate} orders per hour$`) })).toBeChecked();
  }
  expect(await looseTimes(page)).toEqual([]);
  await noViolation(page, 'the comparison of a sweep');
  await noSidewaysScroll(page, 'the comparison of a sweep');
});
