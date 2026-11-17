// Spec 05 section 6 and ruling D119 in the browser, against dev.sh: the MapTemplate panel's evaluation is drawn by
// the shared WorldviewMount (brief I1.12, the seam), interactively: the stage is the focusable floor map named by the
// evaluation, opening orthographic; Enter names what is under the centre cross; O switches to orbit; a new evaluation
// is drawn under a new floor key with the camera kept; and the stage's name holds while it has focus (spec 03 section 5).

import { expect, test, type Page } from '@playwright/test';
import type { TemplateDraft } from '../../src/protocol/template_draft.ts';
import { freshThread, listen, spoken } from './support.ts';

/** A new L-Shape draft from the Template list, its first evaluation drawn; answers the draft's id. */
async function opened(page: Page): Promise<string> {
  await freshThread(page);
  await page.goto('/templates');
  const card = page.locator('li.template-card', { has: page.locator('h3', { hasText: /^L-Shape$/ }) });
  await card.getByRole('radio').check();
  await page.getByRole('button', { name: 'Use this map' }).click();
  await page.waitForURL(/\/template-drafts\/[0-9A-Z]{26}$/);
  await expect(page.locator('section.worldview-mount')).toHaveAttribute('data-state', 'drawn', { timeout: 60_000 });
  return new URL(page.url()).pathname.split('/').pop() ?? '';
}

test('the MapTemplate stage draws the evaluation through the viewer, interactively', async ({ page }) => {
  await listen(page);
  const draft = await opened(page);
  const mount = page.locator('section.worldview-mount');
  await expect(mount).toHaveAttribute('data-source', `evaluation:${draft}/1`);
  await expect(mount).toHaveAttribute('data-projection', 'orthographic');
  const stage = page.locator('.worldview-mount__stage');
  await expect(stage).toHaveAttribute('role', 'application');
  await expect(stage).toHaveAttribute('aria-label', 'Evaluation 1 of L-Shape');
  // Spec 05 section 6: the axes of the frame the wall knobs name, drawn in the corner and said in the text.
  await expect(page.locator('section.worldview-mount p.muted').first()).toContainText('Axes of the shell frame: x to the right, y up the drawing.');
  await expect.poll(() => page.evaluate(() => {
    const lines = document.querySelector<HTMLCanvasElement>('.worldview-mount__lines');
    const pixels = lines?.getContext('2d')?.getImageData(0, 0, lines.width, lines.height).data ?? [];
    let inked = 0;
    for (let at = 3; at < pixels.length; at += 4) if (pixels[at] > 0) inked += 1;
    return inked;
  }), 'the axes drawn on the lines over the floor').toBeGreaterThan(0);
  await stage.focus();
  await page.keyboard.press('Enter');
  await expect.poll(async () => (await spoken(page)).some((said) => said.startsWith('Under the centre cross: ') || said === 'Nothing under the centre cross')).toBe(true);
  await page.keyboard.press('o');
  await expect(mount).toHaveAttribute('data-projection', 'orbit');
  await page.keyboard.press('ArrowRight');
  await page.waitForTimeout(600);
  const camera = () => page.evaluate(() => Object.entries(sessionStorage).find(([key]) => key.startsWith('studio.camera.v1:template:'))?.[1] ?? '');
  const before = await camera();
  expect(before, 'the camera kept per panel and draft').not.toBe('');

  const floors: string[] = [];
  page.on('request', (request) => {
    if (request.url().includes('/api/floors/template-drafts/')) floors.push(request.url());
  });
  const aisles = page.getByLabel(/^Aisles/);
  await aisles.fill('3');
  await aisles.press('Enter');
  await expect(mount).toHaveAttribute('data-source', `evaluation:${draft}/2`, { timeout: 60_000 });
  await expect(mount).toHaveAttribute('data-state', 'drawn', { timeout: 60_000 });
  await expect.poll(() => floors.some((url) => url.includes(`/${draft}/2/floor.msgpack`))).toBe(true);
  await expect(mount).toHaveAttribute('data-projection', 'orbit');
  expect(JSON.parse(await camera()).camera, 'a new evaluation keeps the camera').toEqual(JSON.parse(before).camera);
});

test("the stage's name holds while it has focus, and the heading follows a new evaluation", async ({ page }) => {
  const draft = await opened(page);
  const stage = page.locator('.worldview-mount__stage');
  await expect(stage).toHaveAttribute('aria-label', 'Evaluation 1 of L-Shape');
  await stage.focus();
  const { revision } = (await (await page.request.get(`/api/template-drafts/${draft}`)).json()) as TemplateDraft;
  const set = await page.request.patch(`/api/template-drafts/${draft}`, {
    headers: { 'Idempotency-Key': `name-held-${draft}` },
    data: { revision, knobs: { aisles: 3 } },
  });
  expect(set.status(), await set.text()).toBe(200);
  await expect(page.locator('section.worldview-mount h3')).toHaveText('Evaluation 2 of L-Shape', { timeout: 60_000 });
  await expect(stage).toBeFocused();
  await expect(stage, 'the name a screen reader was given stays while it reads the stage').toHaveAttribute('aria-label', 'Evaluation 1 of L-Shape');
  await page.getByLabel(/^Aisles/).focus();
  await expect(stage).toHaveAttribute('aria-label', 'Evaluation 2 of L-Shape');
});

test('an evaluation the person started is spoken once its floor is drawn, never before', async ({ page }) => {
  await listen(page);
  const draft = await opened(page);
  let release = () => {};
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  let asked = () => {};
  const floorAsked = new Promise<void>((resolve) => {
    asked = resolve;
  });
  await page.route(`**/api/floors/template-drafts/${draft}/2/floor.msgpack`, async (route) => {
    asked();
    await held;
    await route.continue();
  });
  const aisles = page.getByLabel(/^Aisles/);
  await aisles.fill('3');
  await aisles.press('Enter');
  await floorAsked;
  await page.waitForTimeout(500);
  const early = (await spoken(page)).filter((said) => said.startsWith('Evaluation 2'));
  expect(early, 'nothing said of evaluation 2 while its floor is still on its way').toEqual([]);
  release();
  await expect(page.locator('section.worldview-mount')).toHaveAttribute('data-source', `evaluation:${draft}/2`);
  await expect(page.locator('section.worldview-mount')).toHaveAttribute('data-state', 'drawn');
  await expect.poll(async () => (await spoken(page)).filter((said) => said.startsWith('Evaluation 2')).length).toBe(1);
});

/**
 * Ruling D192 (the UI fast-follow after PR 600): the agent highlights a figure by its place in the evaluation's census,
 * and a headline tile by the reference it carries, which an earlier highlight may name.
 */
test('every figure of an evaluation is on the page, each under the reference the agent highlights it by', async ({ page }) => {
  const draft = await opened(page);
  const at = `template-draft:${draft}#/evaluations/1`;
  const figures = page.locator('.evaluation__figures [data-ref]');
  await expect(figures.first()).toBeVisible();
  const refs = await figures.evaluateAll((spans) => spans.map((span) => span.getAttribute('data-ref')));
  expect(refs).toEqual(expect.arrayContaining(['by_knob/aisles', 'derived/robot', 'derived/slot'].map((place) => `${at}/census/${place}`)));
  const tiles = await page.locator('.evaluation__summary .figure-tile').evaluateAll((all) => all.map((tile) => tile.getAttribute('data-ref')));
  expect(tiles).toEqual(expect.arrayContaining(['aisles', 'storage-slots', 'robots'].map((tile) => `${at}/census/${tile}`)));
  expect(tiles.filter((ref) => ref === null), 'a tile with no reference').toEqual([]);
  await expect(page.locator(`[data-ref="${at}/problems"]`)).toHaveText('no problems');
});
