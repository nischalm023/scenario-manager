// Spec 06 section 9's statements 11 and 12 on the built Flow panel, and the made L-Shape's graph (brief I1.11's Done
// when), against the running stack: each test makes an L-Shape at 6 aisles in the stack's world-proto-server from
// fixtures/l-shape-6.json as Create site stages one, every made id traded for a fresh one, and opens its Flow panel.

import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { listen, madeLShape, spoken } from './support.ts';

const ADMISSION = '/flow.node/intake/0/admission_factor';

async function opened(page: Page): Promise<string> {
  const site = await madeLShape();
  await page.goto(`/w/${site}/flow@main`);
  await expect(page.locator('footer.flow-compiles')).toContainText('Compiles to 7 root tasks');
  return site;
}

const group = (page: Page, heading: string) => page.locator('.flow-explorer__group').filter({ has: page.locator(`h3:text-is("${heading}")`) });
const items = (page: Page) => page.locator('.flow-explorer button.flow-explorer__item[data-ref]');

test('a made L-Shape\'s Flow panel draws 5 nodes, 8 edges and 13 segments, compiling to 7 root tasks', async ({ page }) => {
  await opened(page);
  for (const [heading, count] of [['Nodes', 5], ['Material flow', 8], ['Agent-flow segments', 13], ['Return rules', 1]] as const) {
    await expect(group(page, heading).locator('[data-ref]'), heading).toHaveCount(count);
  }
  await expect(page.locator('.flow-canvas .react-flow__node.flow-node:not([data-kind="handoff"])')).toHaveCount(5);
  await expect(page.locator('.flow-canvas path.flow-edge[data-kind="material"]')).toHaveCount(8);
  await expect(page.locator('.flow-canvas path.flow-edge[data-kind="agent"]')).toHaveCount(13);
});

test('statement 11: one live region, the keyboard walk, hover cards on items and edges, layers=material, no aisle named, no axe violation', async ({ page }) => {
  await listen(page);
  const site = await opened(page);
  const live = await page.evaluate(() => [...document.querySelectorAll('[aria-live], [role="log"], [role="status"], [role="alert"]')].map((e) => e.id || e.tagName));
  expect(live).toEqual(['announcer']);
  const read = await (await page.request.get(`/api/flow/${site}/flow`)).json() as Record<'nodes' | 'edges' | 'segments' | 'return_rules', { ref: string }[]>;
  const answered = [...read.nodes, ...read.edges, ...read.segments, ...read.return_rules].map((entity) => entity.ref).sort();
  expect((await items(page).evaluateAll((all) => all.map((e) => e.getAttribute('data-ref')))).sort()).toEqual(answered);

  // The walk: every explorer item in order, each group's summary before its items (the explorer's groups fold since
  // PR 600), then the canvas group as one stop, never a node or an edge; the drawing's tools, which PR 600 put inside
  // the group, follow it.
  const groups = await page.locator('.flow-explorer summary').count();
  await page.getByRole('button', { name: 'Check the flows' }).focus();
  const stops: string[] = [];
  for (let at = 0; at < answered.length + groups + 2; at += 1) {
    await page.keyboard.press('Tab');
    stops.push(await page.evaluate(() => {
      const focused = document.activeElement as HTMLElement;
      if (focused.closest('.flow-canvas') === null) return focused.getAttribute('data-ref') ?? focused.tagName;
      return focused.matches('.flow-canvas') ? 'canvas' : focused.closest('.flow-canvas__tools') !== null ? 'tool' : 'inside';
    }));
  }
  expect(stops.filter((stop) => stop === 'SUMMARY')).toHaveLength(groups);
  const walked = stops.filter((stop) => stop !== 'SUMMARY');
  expect(walked.slice(0, answered.length).sort()).toEqual(answered);
  expect(walked[answered.length]).toBe('canvas');
  expect(walked[answered.length + 1]).not.toBe('inside');

  // Focus opens the card, the pointer can enter it, Escape closes it, a second Escape clears the selection.
  const pick = page.locator('.flow-explorer__item', { hasText: 'Pick stations' }).first();
  const ref = await pick.getAttribute('data-ref');
  await page.getByRole('button', { name: 'Check the flows' }).focus();
  while (await page.evaluate(() => document.activeElement?.textContent) !== await pick.textContent()) await page.keyboard.press('Tab');
  const card = page.locator(`#flow-hover-${ref}`);
  await expect(card).toBeVisible();
  await page.keyboard.press('Enter');
  await expect(pick).toHaveAttribute('aria-current', 'true');
  await expect(page.locator(`.flow-canvas .react-flow__node[data-ref="${ref}"]`)).toHaveAttribute('aria-current', 'true');
  await expect.poll(() => spoken(page)).toContain('Selected Pick stations; its details follow the canvas.');
  await card.hover();
  await expect(card).toBeVisible();
  await pick.focus();
  await page.keyboard.press('Escape');
  await expect(card).toBeHidden();
  await expect(pick).toHaveAttribute('aria-current', 'true');
  await page.keyboard.press('Escape');
  await expect(pick).not.toHaveAttribute('aria-current', 'true');

  // An edge on the canvas opens its hover card on pointer hover, naming what serves it: the PickDemand root task, or
  // for a return each finished presentation's clearance; the pointer can rest on the card, and Escape closes it
  // (spec 06 section 5; WCAG 1.4.13). In the Material flow view: in PR 600's overlay an agent label lies over this
  // material one.
  await page.getByRole('tab', { name: 'Material flow' }).click();
  await page.locator('.flow-canvas text.flow-edge__label[data-layer="material"]', { hasText: 'stock tote' }).first().hover();
  const edgeCard = page.locator('.flow-canvas .flow-hover--edge');
  await expect(edgeCard).toBeVisible();
  await expect(edgeCard).toContainText(/Its root task is PickDemand|Each finished presentation's clearance returns it to storage/);
  await edgeCard.hover();
  await expect(edgeCard).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(edgeCard).toBeHidden();
  await page.getByRole('tab', { name: 'Overlay' }).click();

  // No canvas text, hover card or detail pane names one aisle or counts its VTMs (D105).
  const segment = page.locator('.flow-explorer__item', { hasText: 'VTMs' }).first();
  if (await segment.count() > 0) await segment.click();
  const words = await page.evaluate(() => [...document.querySelectorAll('.flow-canvas, .flow-hover, .detail-pane')].map((e) => e.textContent ?? '').join(' '));
  expect(words).not.toMatch(/aisle\s*\d|\d+\s*VTMs?\b/i);

  const results = await new AxeBuilder({ page }).include('#panel').analyze();
  expect(results.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(' ')).slice(0, 3).join(' | ')}`)).toEqual([]);

  // The "Material flow" tab hides every segment, handoff place and agent label, writes layers=material, and leaves the explorer.
  await page.getByRole('tab', { name: 'Material flow' }).click();
  await expect(page).toHaveURL(/layers=material/);
  await expect(page.getByRole('tab', { name: 'Material flow' })).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('.flow-canvas [data-layer="agent"]')).toHaveCount(0);
  await expect(page.locator('.flow-canvas path.flow-edge[data-kind="material"]')).toHaveCount(8);
  await expect(items(page)).toHaveCount(answered.length);
  await page.getByRole('tab', { name: 'Overlay' }).click();
  await expect(page).not.toHaveURL(/layers=/);
});

test('statement 12: a detail-pane edit marks exactly its entity changed, then Apply moves the flow\'s head', async ({ page }) => {
  await listen(page);
  const site = await opened(page);
  const head = async () => ((await (await page.request.get(`/api/flow/${site}/flow`)).json()) as { seqs: Record<string, number> }).seqs[`${site}/flow`];
  const before = await head();
  const pick = page.locator('.flow-explorer__item', { hasText: 'Pick stations' }).first();
  const ref = await pick.getAttribute('data-ref');
  await pick.click();
  const row = page.locator(`.detail-pane tr[data-pointer="${ADMISSION}"]`);
  await row.locator('input').fill('5');
  await row.locator('input').press('Enter');
  await expect(page).toHaveURL(/draft=/);
  await expect(page.locator('.draft-banner')).toContainText('1 change, ready to apply');
  const marked = page.locator('.flow-canvas [data-diff="changed"]');
  await expect(marked.first()).toBeVisible();
  expect(new Set(await marked.evaluateAll((all) => all.map((e) => e.closest('[data-ref]')?.getAttribute('data-ref'))))).toEqual(new Set([ref]));
  await expect(page.locator('.flow-explorer__item[data-diff="changed"]')).toHaveCount(1);
  await expect(page.locator('.detail-pane tr[data-diff="changed"]')).toHaveCount(1);
  await expect(row.locator('td').nth(0)).toHaveText('4');
  await expect(row.locator('td').nth(1)).toHaveText('5');
  await expect(page.locator('footer.flow-compiles')).toContainText('With the draft: still 7 root tasks');
  await page.getByRole('button', { name: 'Apply' }).click();
  await expect(page.locator('.draft-banner')).toContainText(`Your draft was applied as flow changeset ${before + 1}.`, { timeout: 60_000 });
  expect(await head()).toBe(before + 1);
});
