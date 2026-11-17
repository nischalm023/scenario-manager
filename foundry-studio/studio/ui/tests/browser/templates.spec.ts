// Spec 05 section 9 in the browser, against dev.sh with the development seed written: the Template list holds
// the five shells in the POC's order with their drawings; "Use this map" opens the MapTemplate panel on a new
// draft; changes within 800 ms of each other make one PATCH, Enter sends at once, an evaluation that ends while a
// later change waits is never drawn, and Create site stays focusable and says why while a change evaluates; one
// template's view shows its knobs and default evaluation; an outline nobody set follows the aisles and says what it
// fits, and set small it holds with its fit error under it (D120); axe finds no violation, light or dark, and no panel
// scrolls sideways at 320 px; through the API alone, against the live server, each shell meets section 5.5's
// acceptance 1 and the L-Shape its acceptance 4; and Create site on the Rectangle and the L-Shape makes each site in
// one changeset, binds the thread and writes nothing more on a second press (section 4.4; spec 01 statement 16).

import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page, type Request } from '@playwright/test';
import type { SiteList } from '../../src/protocol/site.ts';
import type { DraftStarted, KnobsSet, TemplateDraft } from '../../src/protocol/template_draft.ts';
import type { TemplateEvaluation } from '../../src/protocol/template_evaluation.ts';
import { freshThread, listen, spoken } from './support.ts';

const SHELLS = ['Square', 'Rectangle', 'L-Shape', 'U-Shape', 'I-Shape'];

/** Axe in the light and the dark scheme; then light again. */
async function noViolation(page: Page, what: string) {
  for (const scheme of ['dark', 'light'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    const results = await new AxeBuilder({ page }).analyze();
    const found = results.violations.map((violation) => `${violation.id}: ${violation.nodes.map((node) => node.target.join(' ')).slice(0, 3).join(' | ')}`);
    expect(found, `${what}, ${scheme}`).toEqual([]);
  }
}

/** Spec 02 statement 16 on these panels: at 320 by 640 CSS px the page does not scroll sideways. */
async function reflows(page: Page, what: string) {
  await page.setViewportSize({ width: 320, height: 640 });
  const wider = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  const culprits = await page.evaluate(() => {
    const vw = document.documentElement.clientWidth;
    return [...document.querySelectorAll('body *')].filter((el) => el.getBoundingClientRect().right > vw + 1)
      .slice(0, 12).map((el) => `${el.tagName}.${typeof el.className === 'string' ? el.className : ''} ${Math.round(el.getBoundingClientRect().right)}`);
  });
  expect(wider, `${what} at 320 px: ${culprits.join('; ')}`).toBeLessThanOrEqual(0);
  await page.setViewportSize({ width: 1280, height: 720 });
}

/** The PATCHes the page sends to its draft, each with the wall-clock instant it left. */
function patches(page: Page): { at: number; body: unknown }[] {
  const sent: { at: number; body: unknown }[] = [];
  page.on('request', (request: Request) => {
    if (request.method() === 'PATCH' && /\/api\/template-drafts\//.test(request.url())) {
      sent.push({ at: Date.now(), body: request.postDataJSON() });
    }
  });
  return sent;
}

/** Open the list, wait for every drawing, and start a draft of `title`: the MapTemplate panel, evaluation 1 drawn. */
async function draftOf(page: Page, title: string): Promise<string> {
  await page.goto('/templates');
  await expect(page.locator('li.template-card')).toHaveText(SHELLS.map(() => /at its defaults: 4 aisles, .*; no problems\./), { timeout: 60_000 });
  const card = page.locator('li.template-card', { has: page.locator('h3', { hasText: new RegExp(`^${title}$`) }) });
  await card.getByRole('radio').check();
  await page.getByRole('button', { name: 'Use this map' }).click();
  await page.waitForURL(/\/template-drafts\/[0-9A-Z]{26}$/);
  await expect(page.locator('.worldview-mount h3')).toContainText('Evaluation 1', { timeout: 30_000 });
  return new URL(page.url()).pathname.split('/').pop() ?? '';
}

/** Every evaluation number the stage's heading has shown, recorded as the DOM changes. */
async function recordDrawn(page: Page) {
  await page.evaluate(() => {
    const drawn: number[] = [];
    (window as unknown as { __drawn: number[] }).__drawn = drawn;
    const note = () => {
      const heading = document.querySelector('.worldview-mount h3')?.textContent ?? '';
      const n = Number(/Evaluation (\d+)/.exec(heading)?.[1]);
      if (Number.isFinite(n) && drawn[drawn.length - 1] !== n) drawn.push(n);
    };
    note();
    new MutationObserver(note).observe(document.getElementById('panel') ?? document.body, { subtree: true, childList: true, characterData: true });
  });
}

const drawnSoFar = (page: Page): Promise<number[]> => page.evaluate(() => (window as unknown as { __drawn: number[] }).__drawn);

test.beforeEach(async ({ page }) => {
  await freshThread(page);
});

test('the list holds the five shells in the POC order, each drawn at its defaults, with no axe violation', async ({ page }) => {
  await page.goto('/templates');
  await expect(page.locator('li.template-card')).toHaveText(SHELLS.map(() => /at its defaults: 4 aisles, .*; no problems\./), { timeout: 60_000 });
  await expect(page.locator('li.template-card h3')).toHaveText(SHELLS);
  const refs = await page.locator('li.template-card').evaluateAll((cards) => cards.map((card) => card.getAttribute('data-ref')));
  expect(refs).toEqual(['square', 'rectangle', 'l-shape', 'u-shape', 'i-shape'].map((slug) => `scope:template/site/${slug}`));
  await page.getByLabel('Filter maps').fill('shape');
  await expect(page.locator('#template-filter-count')).toHaveText('3 of 5 maps shown');
  await page.getByLabel('Filter maps').fill('');
  await noViolation(page, '/templates');
  await reflows(page, '/templates');
});

test("ruling D121: each card shows the viewer's picture of its floor as a data URL, and no SVG is asked for", async ({ page }) => {
  const asked: string[] = [];
  page.on('request', (request) => asked.push(request.url()));
  const started = Date.now();
  await page.goto('/templates');
  const pictures = page.locator('li.template-card img.template-card__thumb');
  await expect(pictures.first()).toHaveAttribute('src', /^data:image\/png;base64,/, { timeout: 60_000 });
  const first = Date.now() - started;
  await expect(pictures).toHaveCount(SHELLS.length, { timeout: 60_000 });
  const all = Date.now() - started;
  const shown = await pictures.evaluateAll((images) => images.map((image) => ({ src: (image as HTMLImageElement).src.slice(0, 22), alt: (image as HTMLImageElement).alt })));
  expect(shown).toEqual(SHELLS.map((title) => ({ src: 'data:image/png;base64,', alt: `${title} at its defaults, drawn north-up` })));
  const colours = await pictures.first().evaluate(async (image) => {
    const copy = document.createElement('canvas');
    [copy.width, copy.height] = [(image as HTMLImageElement).naturalWidth, (image as HTMLImageElement).naturalHeight];
    const context = copy.getContext('2d');
    context?.drawImage(image as HTMLImageElement, 0, 0);
    const pixels = context?.getImageData(0, 0, copy.width, copy.height).data ?? [];
    const seen = new Set<string>();
    for (let at = 0; at < pixels.length; at += 4 * 89) seen.add(`${pixels[at]},${pixels[at + 1]},${pixels[at + 2]}`);
    return seen.size;
  });
  expect(colours, "the floor's own pixels, not a blank").toBeGreaterThan(3);
  const floors = new Set(asked.filter((url) => url.includes('/api/floors/template-thumbnails/')));
  expect(floors.size, 'each card drawn from its floor document, by the viewer').toBe(SHELLS.length);
  expect(asked.filter((url) => /\.svg(\?|$)/.test(url)), 'no SVG asked for').toEqual([]);
  // Where each card's picture stands: the card's "i" beside its title is an icon, not a drawing of the floor.
  expect(await page.locator('li.template-card .template-card__stage svg').count(), 'no SVG drawn').toBe(0);
  const costs = await page.evaluate(() => performance.getEntriesByName('floor-snapshot').map((entry) => Math.round(entry.duration)));
  console.log(`D121 snapshots: first thumbnail ${first} ms after navigation, all five ${all} ms; each picture ${costs.join(', ')} ms`);
});

/** The pictures on the Template list's cards once all five are drawn, in the list's order. */
async function cardPictures(page: Page): Promise<string[]> {
  const pictures = page.locator('li.template-card img.template-card__thumb');
  await expect(pictures).toHaveCount(SHELLS.length, { timeout: 60_000 });
  return pictures.evaluateAll((images) => images.map((image) => (image as HTMLImageElement).src));
}

test('two templates give two different pictures, and one template the same picture when drawn again', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'light' });
  await page.goto('/templates');
  const first = await cardPictures(page);
  expect(new Set(first).size, 'five shells, five different pictures').toBe(SHELLS.length);
  await page.reload();
  expect(await cardPictures(page), 'each card drawn again from its floor in a new page, the same picture').toEqual(first);
});

test('a change of theme draws every card again in the palette it reads, after the pictures queued before it', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'light' });
  await page.goto('/templates');
  const light = await cardPictures(page);
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect.poll(async () => (await cardPictures(page)).filter((src, n) => src !== light[n]).length, 'each card drawn again, dark').toBe(SHELLS.length);
  const dark = await cardPictures(page);
  await page.emulateMedia({ colorScheme: 'light' });
  await expect.poll(() => cardPictures(page), 'back to light: the pictures kept for that palette').toEqual(light);
  expect(dark).not.toEqual(light);
});

test('changes within the pause make one PATCH, Enter sends at once, and the panel has no axe violation', async ({ page }) => {
  await listen(page);
  const sent = patches(page);
  await draftOf(page, 'Rectangle');
  await noViolation(page, 'the MapTemplate panel');
  await reflows(page, 'the MapTemplate panel');

  await page.getByLabel('Width', { exact: true }).fill('41650');
  await page.waitForTimeout(300);
  await page.getByLabel('Depth', { exact: true }).fill('28050');
  await expect(page.locator('#knob-width-by')).toHaveText('Evaluating…');
  await expect(page.locator('.worldview-mount h3')).toContainText('Evaluation 2', { timeout: 30_000 });
  expect(sent.map((one) => one.body)).toEqual([{ revision: 1, knobs: { width: 41650, depth: 28050 } }]);
  await expect(page.locator('#knob-width-by')).toHaveText('Set by you');
  await expect.poll(() => spoken(page)).toContainEqual(expect.stringMatching(/^Evaluation 2 ready: 4 aisles, .*no problems\.$/));

  const typed = Date.now();
  await page.getByLabel('HTMs', { exact: true }).fill('25');
  await page.getByLabel('HTMs', { exact: true }).press('Enter');
  await expect.poll(() => sent.length).toBe(2);
  expect(sent[1].at - typed, 'Enter sent before the pause ended').toBeLessThan(800);
  await expect(page.locator('.worldview-mount h3')).toContainText('Evaluation 3', { timeout: 30_000 });
});

test('an evaluation that ends while a later change waits is never drawn', async ({ page }) => {
  const sent = patches(page);
  await draftOf(page, 'L-Shape');
  await recordDrawn(page);
  await page.getByLabel('HTMs', { exact: true }).fill('30');
  await page.getByLabel('HTMs', { exact: true }).press('Enter');
  await expect.poll(() => sent.length).toBe(1);
  await page.getByLabel('Station pitch', { exact: true }).fill('5100');
  await expect.poll(() => sent.length, { timeout: 10_000 }).toBe(2);
  await expect(page.locator('.worldview-mount h3')).toContainText('Evaluation 3', { timeout: 30_000 });
  const drawn = await drawnSoFar(page);
  expect(drawn, `drawn in order: ${drawn.join(', ')}`).not.toContain(2);
  expect(drawn[drawn.length - 1]).toBe(3);
});

test('Create site stays focusable while a change evaluates, and pressing it says why', async ({ page }) => {
  await listen(page);
  await draftOf(page, 'U-Shape');
  const create = page.getByRole('button', { name: 'Create site' });
  await expect(create).not.toHaveAttribute('aria-disabled', 'true');
  await page.getByLabel('HTMs', { exact: true }).fill('26');
  await expect(create).toHaveAttribute('aria-disabled', 'true');
  await expect(page.locator('#create-site-reason')).toHaveText('Create site is unavailable while a change evaluates.');
  await create.focus();
  await expect(create).toBeFocused();
  // A keyboard press, since Playwright's click waits for an `aria-disabled` control to be enabled.
  await page.keyboard.press('Enter');
  await expect.poll(() => spoken(page)).toContain('Create site is unavailable while a change evaluates.');
  await expect(page.locator('.worldview-mount h3')).toContainText('Evaluation 2', { timeout: 30_000 });
  await expect(create).not.toHaveAttribute('aria-disabled', 'true');
});

test('the Square\'s side follows its aisles and says so; set small it holds, its fit error under it (D120)', async ({ page }) => {
  await draftOf(page, 'Square');
  const side = page.getByLabel('Side', { exact: true });
  await expect(side).toHaveValue('40800');
  await expect(page.locator('#knob-side-by')).toHaveText('Fits 4 aisles and the stations on its walls: 40,800 mm');
  await page.getByLabel('Aisles', { exact: true }).fill('6');
  await page.getByLabel('Aisles', { exact: true }).press('Enter');
  await expect(page.locator('p.muted', { hasText: /^Evaluation 2 not written/ })).toBeVisible({ timeout: 30_000 });
  await expect(side).toHaveValue('56100');
  await expect(page.locator('#knob-side-by')).toHaveText('Fits 6 aisles and the stations on its walls: 56,100 mm');
  await expect(page.locator('#knob-form .problem')).toHaveCount(0);

  await side.fill('40800');
  await side.press('Enter');
  await expect(page.locator('p.muted', { hasText: /^Evaluation 3 not written/ })).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('#knob-side-by')).toHaveText('Set by you');
  await expect(page.locator('#knob-side-problem-0')).toHaveText('Error: The aisle row needs 15,300 mm more floor than the shell\'s width gives.');

  await side.fill('');
  await side.press('Enter');
  await expect(page.locator('p.muted', { hasText: /^Evaluation 4 not written/ })).toBeVisible({ timeout: 30_000 });
  await expect(side).toHaveValue('56100');
  await expect(page.locator('#knob-side-by')).toHaveText('Fits 6 aisles and the stations on its walls: 56,100 mm');
});

test('one template shows its card, its default evaluation and its knobs, with no axe violation', async ({ page }) => {
  await page.goto('/w/template/site/l-shape@main');
  await expect(page.locator('#panel .figure-tile').first()).toBeVisible({ timeout: 60_000 });
  await expect(page.locator('#panel table tbody tr')).toHaveCount(12);
  await expect(page.getByRole('button', { name: 'Use this template' })).toBeVisible();
  await noViolation(page, 'one template');
  await reflows(page, 'one template');
});

test('each shell evaluates at its defaults through the API: no error, its document loaded and its floor kept', async ({ request }) => {
  // Spec 05 section 5.5, acceptance 1, the flow half excepted (task 17 is unbuilt): an evaluation is `ready` only once
  // foundry's reader loaded its document and its floor compiled.
  const slugs = ['square', 'rectangle', 'l-shape', 'u-shape', 'i-shape'];
  for (const [i, slug] of slugs.entries()) {
    const started = await request.post('/api/template-drafts', {
      headers: { 'Idempotency-Key': `acceptance-1-${slug}-${Date.now()}` },
      data: { template: `template/site/${slug}@main` },
    });
    expect(started.status(), slug).toBe(201);
    const { draft } = (await started.json()) as DraftStarted;
    const latest = async () => ((await (await request.get(`/api/template-drafts/${draft}`)).json()) as TemplateDraft).latest;
    await expect.poll(async () => (await latest())?.state, { timeout: 60_000, message: slug }).not.toBe('in-progress');
    const evaluation = (await (await request.get(`/api/template-drafts/${draft}/evaluations/1`)).json()) as TemplateEvaluation;
    const errors = evaluation.problems.filter((problem) => problem.severity === 'error').map((problem) => problem.text);
    expect([evaluation.state, errors], slug).toEqual(['ready', []]);
    expect(evaluation.prologue?.name).toBe(`${SHELLS[i]}, evaluation 1`);
    expect(evaluation.prologue?.primary_frame, `${slug}'s primary frame`).toBeTruthy();
    expect(evaluation.floor?.bytes ?? 0, `${slug}'s floor`).toBeGreaterThan(0);
    expect(evaluation.problems.filter((problem) => problem.code === 'flow-invalid'), `${slug}'s flow compiles`).toEqual([]);
  }
});

test('the L-Shape evaluates through the API at each knob\'s bounds and each orientation, failing only checks', async ({ request }) => {
  // Spec 05 section 5.5, acceptance 4 (lead decided: the L-Shape here, all five in the engine's own tests): each knob at
  // its minimum and its maximum, the others at their defaults, and each option, evaluates; it fails only checks and
  // outlines held at their range (D120), and a site it cannot load only beside one of those errors; without an error it
  // is ready with its floor.
  test.setTimeout(300_000);
  const started = await request.post('/api/template-drafts', {
    headers: { 'Idempotency-Key': `acceptance-4-${Date.now()}` },
    data: { template: 'template/site/l-shape@main' },
  });
  expect(started.status()).toBe(201);
  const { draft } = (await started.json()) as DraftStarted;
  const read = async () => (await (await request.get(`/api/template-drafts/${draft}`)).json()) as TemplateDraft;
  await expect.poll(async () => (await read()).latest?.state, { timeout: 60_000 }).not.toBe('in-progress');
  const { declarations } = await read();
  const values: [string, number][] = declarations.flatMap((knob): [string, number][] => {
    if (knob.options.length > 0) return knob.options.map((option): [string, number] => [knob.key, option.value]);
    const { min, max, step } = knob;
    if (min === null || max === null) throw new Error(`${knob.key} declares neither options nor a range`);
    return [[knob.key, min], [knob.key, max - ((max - min) % (step ?? 1))]];
  });
  expect(values.length, 'two values or more for each of the L-Shape\'s 12 knobs').toBeGreaterThanOrEqual(2 * 12);
  let [revision, moved]: [number, string | null] = [1, null];
  for (const [key, value] of values) {
    const knobs: Record<string, number | null> = { [key]: value };
    if (moved !== null && moved !== key) knobs[moved] = null;
    moved = key;
    const answer = await request.patch(`/api/template-drafts/${draft}`, {
      headers: { 'Idempotency-Key': `acceptance-4-${draft}-${revision}` },
      data: { revision, knobs },
    });
    expect(answer.status(), `${key} at ${value}: ${await answer.text()}`).toBe(200);
    const set = (await answer.json()) as KnobsSet;
    revision = set.revision;
    await expect.poll(async () => (await read()).latest?.state, { timeout: 60_000 }).not.toBe('in-progress');
    const evaluation = (await (await request.get(`/api/template-drafts/${draft}/evaluations/${set.evaluation.n}`)).json()) as TemplateEvaluation;
    const fits = ['check-failed', 'derived-out-of-range'];
    const checked = evaluation.problems.some((problem) => fits.includes(problem.code) && problem.severity === 'error');
    const others = evaluation.problems.filter((problem) => !fits.includes(problem.code) && !(problem.code === 'floor-compile-failed' && checked));
    expect(others.map((problem) => problem.text), `${key} at ${value}`).toEqual([]);
    if (!checked) expect([evaluation.state, (evaluation.floor?.bytes ?? 0) > 0], `${key} at ${value}`).toEqual(['ready', true]);
  }
});

test('Create site makes the Rectangle and the L-Shape each in one changeset, binds the thread and writes nothing more', async ({ page, request }) => {
  // Spec 05 section 4.4 through the panel's press, which is the person's approval (spec 01 section 3.10, rule 6).
  test.setTimeout(240_000);
  for (const title of ['Rectangle', 'L-Shape']) {
    const id = await draftOf(page, title);
    const draft = (await (await request.get(`/api/template-drafts/${id}`)).json()) as TemplateDraft;
    const slug = `${title.toLowerCase().replace(/[^a-z]+/g, '-')}-${Date.now()}`;
    const root = `customer/demo/site/${slug}`;
    // Exactly: the field's "i" is named "About the reference".
    await page.getByLabel('Reference', { exact: true }).fill(slug);
    await page.getByRole('button', { name: 'Create site' }).click();
    await page.waitForURL(new RegExp(`/w/${root}/map@main$`), { timeout: 60_000 });
    // The map's Made from card links to the site's template page, which lists what made the site.
    await page.getByRole('link', { name: 'Every make and update of this site' }).click();
    await expect(page).toHaveURL(new RegExp(`/w/${root}@main\\?panel=template$`));
    await expect(page.locator('#panel ol[aria-label="Made from, newest first"] > li')).toHaveCount(1);

    const made = (await (await request.get(`/api/template-drafts/${id}`)).json()) as TemplateDraft;
    const seqs = { [root]: 1, [`${root}/fleet`]: 1, [`${root}/flow`]: 1, [`${root}/map`]: 1 };
    expect([made.state, made.write?.result?.seqs], title).toEqual(['created', seqs]);
    const listed = async () => ((await (await request.get('/api/sites')).json()) as SiteList).items.filter((item) => item.site === root);
    expect((await listed()).map((item) => [item.name, item.branches[0]?.heads]), title).toEqual([[title, seqs]]);
    const bound = (await (await request.get(`/api/conversations/${draft.conversation}`)).json()) as { conversation: { site: string | null } };
    expect(bound.conversation.site, 'the thread is bound').toBe(root);
    const again = await request.post(`/api/template-drafts/${id}/site`, {
      headers: { 'Idempotency-Key': `again-${slug}` },
      data: { customer: 'demo', slug, name: title, evaluation: made.write?.evaluation },
    });
    expect(again.status(), 'a second press').toBe(409);
    expect((await listed()).map((item) => item.branches[0]?.heads), 'nothing more written').toEqual([seqs]);
  }
});
