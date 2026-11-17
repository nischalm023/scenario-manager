// Spec 07 section 8 in the browser, the Scenario panel (brief I1.08's Done when): creating "Peak hour" at the stated
// defaults (every order at the start), editing a field, meeting a conflict, reading "Ready to run: … at map 1, flow 1,
// fleet 1", preparing and submitting a sweep of five over orders per hour, Run's press in event order against spec
// 08's run routes, the chat column bound to the scenario's site, walking the panel by keyboard in DOM order, Delete's
// confirmation landing on "Keep it", no image select in the DOM and no axe violation (statement 13's built half). The
// site is `STUDIO_TEST_SITE`, else an L-Shape at 6 aisles this file makes once, as Create site stages one, its `map`,
// `flow` and `fleet` each at changeset 1, so no other spec need have made a site first.

import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { listen, madeLShape, spoken } from './support.ts';

let made: Promise<string> | undefined;

/** `STUDIO_TEST_SITE`, else the L-Shape this file made, made at the first test that asks. */
function siteOf(): Promise<string> {
  if (process.env.STUDIO_TEST_SITE) return Promise.resolve(process.env.STUDIO_TEST_SITE);
  made ??= madeLShape();
  return made;
}

/** Peak hour made afresh on `site`, an earlier test's deleted first since a name is the site's once. */
async function peakHour(page: Page, site: string): Promise<string> {
  const listed = (await (await page.request.get(`/api/scenarios?site=${encodeURIComponent(site)}`)).json()) as {
    items: { id: string; name: string }[];
  };
  for (const item of listed.items.filter((item) => item.name.toLowerCase() === 'peak hour')) {
    await page.request.delete(`/api/scenarios/${item.id}`);
  }
  await page.goto(`/scenarios?site=${encodeURIComponent(site)}`);
  await page.getByRole('button', { name: 'New scenario' }).click();
  await expect(page.getByLabel('Name of the new scenario')).toBeFocused();
  await page.getByLabel('Name of the new scenario').fill('Peak hour');
  await page.getByRole('button', { name: 'Create scenario' }).click();
  await expect(page).toHaveURL(/\/scenarios\/[0-9A-Z]{26}$/);
  await expect(page.locator('#panel-title')).toBeFocused();
  await expect(page.locator('#sc-name')).toHaveText('Peak hour');
  return new URL(page.url()).pathname.split('/').pop() ?? '';
}

/** The panel events the site's live conversation holds, oldest first: what the agent reads with its next turn. */
async function panelEvents(page: Page, site: string): Promise<{ kind: string; text: string; body: unknown }[]> {
  const opened = (await (await page.request.post('/api/conversations', { data: { site } })).json()) as { conversation: { id: string } };
  const snapshot = (await (await page.request.get(`/api/conversations/${opened.conversation.id}`)).json()) as {
    entries: { kind: string; body: { kind: string; text: string; body: unknown } }[];
  };
  return snapshot.entries.filter((entry) => entry.kind === 'panel.event').map((entry) => entry.body);
}

async function ready(page: Page) {
  await expect(page.locator('#sc-readiness')).toContainText(/Ready to run: [\d,]+ orders at map 1, flow 1, fleet 1, as read at/, { timeout: 60_000 });
}

test('Peak hour at the stated defaults is ready at map 1, flow 1, fleet 1, and a field commits on Enter', async ({ page }) => {
  const site = await siteOf();
  const id = await peakHour(page, site);
  await ready(page);
  // D117: each thing said once, at the place of action.
  await expect(page.locator('#sc-readiness')).toHaveText(/^Ready to run: 1,200 orders at map 1, flow 1, fleet 1, as read at \d\d:\d\d\.$/);
  await expect(page.locator('.problem')).toHaveText([
    "Warning: This check runs the simulator's default flow, not the site's, and does not check that the racks start at the requested fill or that the stock fits them.",
  ]);
  await expect(page.locator('form.scenario-form').getByText('same values and seed')).toHaveCount(1);
  await expect(page.locator('#sw-vary-hint')).toHaveText('One field at a time; the site is never changed.');
  await expect(page.locator('#sw-count')).toHaveCount(0);
  await expect(page.getByRole('radio', { name: 'All at the start' })).toBeChecked();
  // Astra's cuts: only the release this build runs is offered, and a sweep never varies the release or its waves.
  for (const other of ['Even through each hour', 'Random arrivals at the hourly rate', 'Waves']) {
    await expect(page.getByRole('radio', { name: other })).toBeDisabled();
  }
  await expect(page.getByRole('group', { name: 'Release' })).toHaveAccessibleDescription('Only all at the start runs in this build.');
  const vary = page.getByRole('group', { name: 'Vary' });
  await expect(vary.getByRole('radio', { name: 'Release' })).toHaveCount(0);
  await expect(vary.getByRole('radio', { name: 'Wave interval' })).toHaveCount(0);
  await expect(vary.getByRole('radio', { name: 'Orders per hour' })).toHaveCount(1);
  await expect(page.locator('select#runner-image, select[name="runner_image"]')).toHaveCount(0);
  const duration = page.getByLabel('Duration');
  await expect(duration).toHaveValue('4');
  await expect(duration).toHaveAccessibleDescription(/hours of simtime/);
  await duration.fill('2');
  await duration.press('Enter');
  await expect(page.locator('p.panel__status')).toHaveText(/Version 1, open: saved\./);
  const read = (await (await page.request.get(`/api/scenarios/${id}`)).json()) as {
    version: { revision: number; content: { run_controls: { duration_sim_ms: number } } };
  };
  expect(read.version.revision).toBe(2);
  expect(read.version.content.run_controls.duration_sim_ms).toBe(7_200_000);
  await expect(duration).toBeFocused();
  await expect.poll(() => panelEvents(page, site)).toContainEqual({
    kind: 'scenario.changed', text: 'You changed Duration in Peak hour from 4 hours of simtime to 2 hours of simtime',
    body: { scenario: id, version: 1, revision: 2 },
  });
});

test('a conflict keeps the person’s text beside the other value, says it once, and Keep this takes the other', async ({ page }) => {
  await listen(page);
  const site = await siteOf();
  const id = await peakHour(page, site);
  await ready(page);
  const before = (await (await page.request.get(`/api/scenarios/${id}`)).json()) as { version: { revision: number } };
  const other = await page.request.patch(`/api/scenarios/${id}`, {
    data: { version: 1, revision: before.version.revision, set: [{ pointer: '/run_controls/duration_sim_ms', value: 10_800_000 }] },
  });
  expect(other.status()).toBe(200);
  const duration = page.getByLabel('Duration');
  await duration.fill('2');
  await duration.press('Enter');
  await expect(page.getByText('Changed by you to 3 hours of simtime; yours was not saved.', { exact: true })).toBeVisible();
  await expect(duration).toHaveValue('2');
  await expect(duration).toHaveAttribute('aria-invalid', 'true');
  await expect(duration).toBeFocused();
  await expect.poll(async () => (await spoken(page)).filter((sentence) => sentence.startsWith('Duration was not saved')).length).toBe(1);
  await page.getByRole('button', { name: 'Keep this' }).click();
  await expect(page.getByLabel('Duration')).toHaveValue('3');
  await expect(page.getByLabel('Duration')).toBeFocused();
  await expect(page.locator('.field__error')).toHaveCount(0);
});

test('a release conflict offers Use mine and Keep this, and a second conflict on Use mine keeps the person’s choice', async ({ page }) => {
  const site = await siteOf();
  const id = await peakHour(page, site);
  await ready(page);
  const release = '/workload/generator/release';
  const set = async (value: string) => {
    const now = (await (await page.request.get(`/api/scenarios/${id}`)).json()) as { version: { revision: number } };
    const answer = await page.request.patch(`/api/scenarios/${id}`, { data: { version: 1, revision: now.version.revision, set: [{ pointer: release, value }] } });
    expect(answer.status()).toBe(200);
  };
  await set('even');
  await page.reload();
  await expect(page.getByRole('radio', { name: 'Even through each hour' })).toBeChecked();
  await set('random');
  const atStart = page.getByRole('radio', { name: 'All at the start' });
  await atStart.click();
  const changed = (words: string) => page.getByText(`Changed by you to ${words}; yours was not saved.`, { exact: true });
  await expect(changed('random arrivals at the hourly rate')).toBeVisible();
  await expect(atStart).toBeChecked();
  // Another change lands between "Use mine"'s re-read and its save: the save meets a second conflict.
  let raced = false;
  await page.route((url) => url.pathname === `/api/scenarios/${id}`, async (route) => {
    if (route.request().method() !== 'PATCH' || raced) return route.fallback();
    raced = true;
    await set('even');
    await route.fallback();
  });
  await page.getByRole('button', { name: 'Use mine' }).click();
  await expect(changed('even through each hour')).toBeVisible();
  await expect(atStart).toBeChecked();
  await page.getByRole('button', { name: 'Use mine' }).click();
  await expect(page.getByRole('button', { name: 'Use mine' })).toHaveCount(0);
  await expect(atStart).toBeChecked();
  await expect(atStart).toBeFocused();
  const read = (await (await page.request.get(`/api/scenarios/${id}`)).json()) as { version: { content: { workload: { generator: { release: string } } } } };
  expect(read.version.content.workload.generator.release).toBe('all_at_start');
});

test("readiness after an edit holds Run until it answers, and an older edition's late answer never replaces a newer one", async ({ page }) => {
  const site = await siteOf();
  await peakHour(page, site);
  await ready(page);
  // The first readiness read after this point is answered by the API but handed to the page only when released.
  let release: () => void = () => {};
  const released = new Promise<void>((resolve) => {
    release = resolve;
  });
  let held = false;
  let delivered = false;
  await page.route((url) => url.pathname.endsWith('/readiness'), async (route) => {
    if (held) return route.fallback();
    held = true;
    const response = await route.fetch();
    await released;
    await route.fulfill({ response });
    delivered = true;
  });
  const duration = page.getByLabel('Duration');
  await duration.fill('2');
  await duration.press('Enter');
  await expect(page.locator('#sc-readiness')).not.toContainText('Ready to run');
  await expect(page.locator('#sw-run')).toHaveAttribute('aria-disabled', 'true');
  await expect(page.locator('#sw-run')).toHaveAccessibleDescription('Reading whether it is ready to run');
  await duration.fill('3');
  await duration.press('Enter');
  await expect(page.locator('#sc-readiness')).toContainText('Ready to run: 900 orders');
  await expect(page.locator('#sw-run')).not.toHaveAttribute('aria-disabled', 'true');
  release();
  await expect.poll(() => delivered).toBe(true);
  await page.waitForTimeout(500);
  await expect(page.locator('#sc-readiness')).toContainText('Ready to run: 900 orders');
});

test('a sweep of five over orders per hour is prepared and submitted, and the agent is told both', async ({ page }) => {
  const site = await siteOf();
  const id = await peakHour(page, site);
  await ready(page);
  await page.getByRole('radio', { name: 'Orders per hour' }).check();
  await page.getByLabel('Values, separated by commas').fill('200, 250, 300, 350, 400');
  await expect(page.locator('#sw-count')).toHaveText('5 values, 5 runs; at most five go at a time.');
  const run = page.getByRole('button', { name: 'Run 5 runs' });
  await expect(run).not.toHaveAttribute('aria-disabled', 'true');
  await run.click();
  await expect(page.locator('#sw-outcome')).toHaveText('Submitted 5 runs. Open the Runs panel', { timeout: 60_000 });
  await expect(run).toBeFocused();
  const read = (await (await page.request.get(`/api/scenarios/${id}`)).json()) as { versions: { experiments: string[] }[] };
  const [experiment] = read.versions[0].experiments;
  const prepared = (await (await page.request.get(`/api/experiments/${experiment}`)).json()) as {
    experiment: { kind: string; variants: { label: string }[]; run_order: unknown[] };
  };
  expect(prepared.experiment.kind).toBe('sweep');
  await expect.poll(async () => (await panelEvents(page, site)).filter((event) => event.kind === 'experiment.prepared').at(-1)).toEqual({
    kind: 'experiment.prepared', text: 'You prepared a sweep of 5 runs of Peak hour over orders per hour',
    body: { experiment, runs_planned: 5 },
  });
  expect(prepared.experiment.variants.map((variant) => variant.label)).toEqual(['rate 200', 'rate 250', 'rate 300', 'rate 350', 'rate 400']);
  await expect.poll(async () => (await panelEvents(page, site)).filter((event) => event.kind === 'runs.submitted').at(-1)).toEqual({
    kind: 'runs.submitted', text: 'You submitted 5 runs of Peak hour', body: { experiment, runs: 5 },
  });
  const runs = (await (await page.request.get(`/api/runs?scenario=${id}`)).json()) as { items: { submission: string; label: string }[] };
  expect(new Set(runs.items.map((run) => run.submission)).size).toBe(1);
  expect(runs.items.map((run) => run.label)).toEqual([200, 250, 300, 350, 400].map((rate) => `Peak hour, ${rate} orders per hour`));
});

test('Run goes plan, ready, submission and execution in order, and says Submitted only once it reads the execution ok', async ({ page }) => {
  const site = await siteOf();
  await peakHour(page, site);
  await ready(page);
  // Spec 08 section 3.7's run routes answer; the page only holds its first read of the approval until the test has
  // seen that nothing was said before it, then lets the API's answer through.
  const asked: string[] = [];
  page.on('request', (request) => {
    const path = new URL(request.url()).pathname;
    if (path.startsWith('/api/runs') || path.startsWith('/api/approvals/')) {
      asked.push(`${request.method()} ${path.replace(/[0-9A-HJKMNP-TV-Z]{26}/g, '{id}')}`);
    }
  });
  let release = () => {};
  const held = new Promise<void>((resolve) => (release = resolve));
  let approvalReads = 0;
  await page.route((url) => url.pathname.startsWith('/api/approvals/'), async (route) => {
    approvalReads += 1;
    if (approvalReads === 1) await held;
    await route.continue();
  });
  const run = page.locator('#sw-run');
  await run.click();
  await expect(run).toHaveText('Preparing 1 run');
  await expect(run).toHaveText('Submitting 1 run', { timeout: 60_000 });
  await expect.poll(() => approvalReads).toBe(1);
  await expect(page.locator('#sw-outcome')).toHaveCount(0);
  release();
  await expect(page.locator('#sw-outcome')).toHaveText('Submitted 1 run. Open the Runs panel');
  await expect(run).toHaveText('Run 1 run');
  await expect(run).toBeFocused();
  expect(asked.filter((one, n) => one !== asked[n - 1])).toEqual([
    'POST /api/runs/plan', 'GET /api/runs/plan/{id}', 'POST /api/runs', 'GET /api/approvals/{id}',
  ]);
});

test("the chat column shows the scenario's site's conversation, and a message carries the cut readiness read", async ({ page }) => {
  const site = await siteOf();
  await peakHour(page, site);
  await ready(page);
  await expect(page.locator('aside#chat')).toBeVisible();
  await expect(page.locator('#context-bar .context-bar__ref')).toHaveText(site);
  const opened = (await (await page.request.post('/api/conversations', { data: { site } })).json()) as { conversation: { id: string } };
  const readiness = (await (await page.request.get(`${new URL(page.url()).pathname.replace('/scenarios/', '/api/scenarios/')}/readiness`)).json()) as {
    cut: { seqs: Record<string, number> };
  };
  let sent: { path: string; context: { panel: string; seen: { scope: string; seq: number }[] } } | null = null;
  await page.route((url) => /\/api\/conversations\/[^/]+\/messages$/.test(url.pathname), async (route) => {
    const body = route.request().postDataJSON() as { context: NonNullable<typeof sent>['context'] };
    sent = { path: new URL(route.request().url()).pathname, context: body.context };
    await route.fulfill({ status: 503, json: { type: 'agent-busy', title: 'Busy', status: 503, detail: 'Held by the test.' } });
  });
  await page.locator('#composer-input').fill('How many orders does this scenario make?');
  await page.locator('#composer-input').press('Enter');
  await expect.poll(() => sent).not.toBeNull();
  expect(sent!.path).toBe(`/api/conversations/${opened.conversation.id}/messages`);
  expect(sent!.context.panel).toBe('scenario');
  expect(sent!.context.seen).toEqual(Object.entries(readiness.cut.seqs).map(([scope, seq]) => ({ scope, seq })));
});

test('the panel walks by keyboard in DOM order, Delete asks with focus on Keep it, and axe finds no violation', async ({ page }) => {
  const site = await siteOf();
  await peakHour(page, site);
  await ready(page);
  const order = await page.evaluate(() => {
    const main = document.getElementById('panel');
    const focusable = main?.querySelectorAll<HTMLElement>('a[href], button, input, select, textarea, [tabindex]:not([tabindex="-1"])') ?? [];
    return [...focusable]
      .filter((node) => !(node instanceof HTMLInputElement && node.type === 'radio' && !node.checked))
      .filter((node) => !node.hasAttribute('disabled') && node.offsetParent !== null)
      .map((node) => node.id || node.textContent?.trim() || node.getAttribute('name') || node.tagName);
  });
  await page.locator('#panel-title').focus();
  const walked: string[] = [];
  for (let step = 0; step < order.length; step += 1) {
    await page.keyboard.press('Tab');
    walked.push(await page.evaluate(() => {
      const node = document.activeElement as HTMLElement | null;
      return node?.id || node?.textContent?.trim() || node?.getAttribute('name') || node?.tagName || '';
    }));
  }
  expect(walked).toEqual(order);
  expect(order.slice(0, 3)).toEqual(['sc-version', 'Duplicate', 'Delete']);

  await page.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Keep it' })).toBeFocused();
  await expect(page.locator('#sc-delete-question')).toHaveText('Delete Peak hour? Its runs stay and keep their link.');
  await page.getByRole('button', { name: 'Keep it' }).click();
  await expect(page.getByRole('button', { name: 'Delete', exact: true })).toBeFocused();

  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    const results = await new AxeBuilder({ page }).include('#panel').analyze();
    expect(results.violations.map((violation) => `${violation.id}: ${violation.nodes.map((node) => node.target.join(' ')).join(' | ')}`), scheme).toEqual([]);
  }
  await page.goto(`/scenarios?site=${encodeURIComponent(site)}`);
  // The site's scenarios as cards, as PR 600 lists them, no longer a table.
  await expect(page.locator('ul.scenario-cards')).toBeVisible();
  const list = await new AxeBuilder({ page }).include('#panel').analyze();
  expect(list.violations.map((violation) => violation.id)).toEqual([]);
});
