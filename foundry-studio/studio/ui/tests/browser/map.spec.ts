// Spec 03 section 8 in the browser, the Map panel (brief I1.12 section 3) against dev.sh: an L-Shape made from
// the development seed's shell (`examples/made_site.rs`, standing in for Create site until its route lands) opens
// drawn, with what it holds; Find keeps focus in its field and speaks the count; Enter on the stage selects what is
// under the centre cross and Escape clears it; a fixture `panel.command` from the test writer is outlined and listed;
// statement 9 (axe, the stage's stable name, the keys' buttons, one named region); and ruling D119: the panel opens
// orthographic, switches to orbit and back by keyboard, and keeps its camera across a reload.

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { listen, openConversation, spoken, write } from './support.ts';

const repo = new URL('../../../../', import.meta.url).pathname;
const SITE = 'customer/demo/site/map-test-l';
const NAME = 'Map test L-shape';
const LOCATION = `${SITE}/map@main`;
const MAP = `/w/${LOCATION}`;
const LABEL = `${NAME}, map at changeset 1`;
/** A second made site, whose flow the tests that follow a branch move, so the first stays at its first changesets for
 * the specs after this one that look for a made site at map 1, flow 1, fleet 1. */
const MOVED = 'customer/demo/site/map-test-moved';
const MOVED_NAME = 'Map test, moved';
const MOVED_MAP = `/w/${MOVED}/map@main`;

/** The world-proto-server the stack started, which the made site is written into. */
function wpsUrl(): string {
  if (process.env.STUDIO_WPS_URL) return process.env.STUDIO_WPS_URL;
  const ports = readFileSync(new URL('../../../.dev-state/ports', import.meta.url), 'utf8');
  const port = /^WPS_PORT=(\d+)$/m.exec(ports)?.[1];
  if (!port) throw new Error('No world-proto-server in studio/.dev-state/ports: start dev.sh, or set STUDIO_WPS_URL.');
  return `http://127.0.0.1:${port}`;
}

/** `examples/made_site.rs` against the stack's world-proto-server. */
const made = (...args: string[]) => execFileSync('cargo', ['run', '-q', '-p', 'studio-api', '--example', 'made_site', '--', ...args], {
  cwd: repo, env: { ...process.env, STUDIO_WPS_URL: wpsUrl() }, stdio: ['ignore', 'pipe', 'pipe'],
});

test.beforeAll(() => {
  made(SITE, NAME);
  made(MOVED, MOVED_NAME);
});

const mount = (page: Page) => page.locator('section.worldview-mount');
const stage = (page: Page) => page.locator('.worldview-mount__stage');
const centreLine = (page: Page) => page.locator('.worldview-mount p.muted').nth(1);

async function drawn(page: Page, path = MAP) {
  await page.goto(path);
  await expect(mount(page)).toHaveAttribute('data-state', /^(drawn|highlighted)$/, { timeout: 60_000 });
  await expect(centreLine(page)).not.toHaveText('Centre on: nothing yet.');
}

/** Pixels of the lines canvas holding ink: the axes always, and the outlines when anything is marked. */
const ink = (page: Page): Promise<number> => page.evaluate(() => {
  const lines = document.querySelector<HTMLCanvasElement>('.worldview-mount__lines');
  if (lines === null || lines.width === 0) return 0;
  const pixels = lines.getContext('2d')?.getImageData(0, 0, lines.width, lines.height).data ?? [];
  let inked = 0;
  for (let at = 3; at < pixels.length; at += 4) if (pixels[at] > 0) inked += 1;
  return inked;
});

/** The made site's entity of `component` called `label`, as Find answers it. */
async function found(page: Page, words: string, component: string): Promise<string> {
  const answer = await page.request.get(`/api/find/${LOCATION}?q=${encodeURIComponent(words)}`);
  const items = ((await answer.json()) as { items: { key: string; label: string; components: string[] }[] }).items;
  const item = items.find((one) => one.components.includes(component) && one.label === words);
  if (item === undefined) throw new Error(`no ${component} called ${words}`);
  return item.key;
}

test('a made L-Shape opens drawn and orthographic, named by what is drawn, with what the site holds', async ({ page }) => {
  await drawn(page);
  await expect(mount(page).locator('h3')).toHaveText(LABEL);
  await expect(stage(page)).toHaveAttribute('aria-label', LABEL);
  await expect(stage(page)).toHaveAttribute('aria-roledescription', 'floor map');
  expect(await page.locator('table.census tr').count()).toBeGreaterThan(5);
  await expect(page.locator('table.census caption')).toHaveText(/at map changeset 1, fleet 1, flow \d+, counted by kind/);
  await expect(mount(page)).toHaveAttribute('data-projection', 'orthographic');
  expect(await ink(page)).toBeGreaterThan(0);
});

test("Find keeps focus in its field and speaks the count; a result selects what it names", async ({ page }) => {
  await listen(page);
  await drawn(page);
  const field = page.locator('#map-find');
  await field.fill('pick station');
  await field.press('Enter');
  await expect(page.locator('ul.find-results li').first()).toBeVisible();
  await expect(field).toBeFocused();
  const count = await page.locator('ul.find-results li').count();
  await expect.poll(() => spoken(page)).toContain(`${count} results for 'pick station'`);
  await page.locator('ul.find-results a.entity-chip').first().click();
  await expect(page).toHaveURL(/select=/);
  await expect(page.locator('.detail-pane .entity-chip[aria-current="true"]')).toBeVisible();
  await field.fill('pick station 99 nowhere');
  await field.press('Enter');
  await expect.poll(() => spoken(page)).toContain("Nothing matches 'pick station 99 nowhere'");
});

test('statement 9: Enter selects what is under the centre cross and says so, Escape clears it, the name stays', async ({ page }) => {
  await listen(page);
  await drawn(page);
  const described = async () => page.evaluate(() => {
    const stage = document.querySelector('.worldview-mount__stage');
    return (stage?.getAttribute('aria-describedby') ?? '').split(' ').map((id) => document.getElementById(id)?.textContent ?? '').join(' | ');
  });
  await stage(page).focus();
  const before = { centre: await centreLine(page).textContent(), described: await described() };
  await page.keyboard.press('ArrowRight');
  await expect(centreLine(page)).not.toHaveText(before.centre ?? '');
  await page.keyboard.press('ArrowLeft');
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/select=/);
  await expect.poll(async () => (await spoken(page)).some((said) => said.startsWith('Selected '))).toBe(true);
  await expect(stage(page)).toBeFocused();
  await expect(page.locator('.detail-pane .entity-chip[aria-current="true"]')).toBeVisible();
  await expect(stage(page)).toHaveAttribute('aria-label', LABEL);
  expect(await described()).not.toBe(before.described);
  await page.keyboard.press('Escape');
  await expect(page).not.toHaveURL(/select=/);
  await expect(stage(page)).toBeFocused();
  await expect(stage(page)).toHaveAttribute('aria-label', LABEL);
});

test("statement 9: axe finds nothing, each key has its button, and inside the panel only the mount is a named region", async ({ page }) => {
  await drawn(page);
  for (const scheme of ['dark', 'light'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    const results = await new AxeBuilder({ page }).analyze();
    const violations = results.violations.map((violation) => `${violation.id}: ${violation.nodes.map((node) => node.target.join(' ')).slice(0, 3).join(' | ')}`);
    expect(violations, scheme).toEqual([]);
  }
  const keyed = ['Fit to view (0)', 'Zoom in (+)', 'Zoom out (−)', 'Frame the highlights (F)', 'Pan left (←)', 'Pan right (→)', 'Pan up (↑)', 'Pan down (↓)', 'Pick the centre (Enter)'];
  for (const name of keyed) {
    await expect(page.getByRole('button', { name, exact: true })).toBeVisible();
  }
  const centred = await centreLine(page).textContent();
  await page.getByRole('button', { name: 'Pan right (→)', exact: true }).click();
  await expect(centreLine(page), 'a pan button pans as its key does').not.toHaveText(centred ?? '');
  await page.getByRole('button', { name: 'Pan left (←)', exact: true }).click();
  await page.getByRole('button', { name: 'Pick the centre (Enter)', exact: true }).click();
  await expect(page, 'the centre pick selects what Enter would').toHaveURL(/select=/);
  await expect(page.getByRole('group', { name: 'Projection (O)' })).toBeVisible();
  const regions = await page.evaluate(() => [...document.querySelectorAll('#panel *')].filter((element) => {
    const role = element.getAttribute('role') ?? (element.tagName === 'SECTION' ? 'region' : null);
    const named = element.hasAttribute('aria-label') || element.hasAttribute('aria-labelledby');
    return role === 'region' && named;
  }).map((element) => element.className));
  expect(regions).toEqual(['worldview-mount']);
  await page.setViewportSize({ width: 320, height: 640 });
  const wider = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(wider, 'no sideways scroll at 320 px').toBeLessThanOrEqual(0);
});

test('a panel.command from the test writer is outlined on the floor and listed, every mark in order', async ({ page }) => {
  await drawn(page);
  const plain = await ink(page);
  const sector = await found(page, 'Pick stations', 'sector');
  const conversation = await openConversation(page, SITE);
  const seq = await write(page, conversation.id, { entry: { seq: 0, kind: 'panel.command', turn: null, caused_by: null, body: {
    action: 'highlight', panel: 'map', location: LOCATION, resolved: [], select: null, layers: null, draft: null, sim_ms: null,
    compare: null, metrics: null, highlight: [{ refs: [sector], note: 'where stock totes arrive for picking', marks: 4, on_floor: 4 }],
    route: `${MAP}?highlights=1`, as_it_was: null, announce: 'The agent highlighted the pick stations on the Map panel',
  } } });
  await drawn(page, `${MAP}?highlights=${seq}`);
  await expect(mount(page)).toHaveAttribute('data-state', 'highlighted');
  await expect(mount(page)).toHaveAttribute('data-highlights', String(seq));
  await expect(page.locator('.highlights__note').first()).toContainText('where stock totes arrive for picking');
  const listed = page.locator('.highlights__group ol.highlights li');
  await expect(listed).toHaveCount(4);
  const xs = await listed.locator('.muted').allTextContents();
  const places = xs.map((text) => Number(/x (-?[\d.]+) m/.exec(text)?.[1]));
  expect([...places].sort((a, b) => a - b), 'in position order along x').toEqual(places);
  await expect.poll(() => ink(page)).toBeGreaterThan(plain);
  await page.getByRole('button', { name: 'Clear highlights' }).click();
  await expect(page).not.toHaveURL(/highlights=/);
  await expect(mount(page)).toHaveAttribute('data-state', 'drawn');
});

test('a selected sector is outlined on the floor by its members, as the Selected pane says', async ({ page }) => {
  await drawn(page);
  const plain = await ink(page);
  const sector = await found(page, 'Pick stations', 'sector');
  await drawn(page, `${MAP}?select=${sector}`);
  await expect(page.locator('.detail-pane .entity-chip[aria-current="true"]')).toBeVisible();
  await expect(mount(page)).toHaveAttribute('data-selection', sector);
  await expect.poll(() => ink(page), 'its members outlined, the sector standing on no cell itself').toBeGreaterThan(plain);
});

/** The camera the Map panel keeps for the made site's map in `sessionStorage`, as written. */
const keptCamera = (page: Page): Promise<string> =>
  page.evaluate(() => Object.entries(sessionStorage).find(([key]) => key.startsWith('studio.camera.v1:map:'))?.[1] ?? '');

test('a show that carries highlights opens framed on their marks; a later highlight opens on the camera kept', async ({ page }) => {
  await page.goto('/');
  await page.evaluate(() => sessionStorage.clear());
  await drawn(page);
  await stage(page).focus();
  for (const key of ['+', '+', '+', '+', 'ArrowLeft', 'ArrowLeft', 'ArrowUp']) await page.keyboard.press(key);
  await expect.poll(() => keptCamera(page)).not.toBe('');
  await page.waitForTimeout(600);
  const away = JSON.parse(await keptCamera(page)).camera;
  const sector = await found(page, 'Pick stations', 'sector');
  const conversation = await openConversation(page, SITE);
  const command = (action: 'show' | 'highlight') => ({ entry: { seq: 0, kind: 'panel.command', turn: null, caused_by: null, body: {
    action, panel: 'map', location: LOCATION, resolved: [], select: null, layers: null, draft: null, sim_ms: null,
    compare: null, metrics: null, highlight: [{ refs: [sector], note: 'where stock totes arrive for picking', marks: 4, on_floor: 4 }],
    route: `${MAP}?highlights=1`, as_it_was: null, announce: `The agent ${action === 'show' ? 'showed' : 'highlighted'} the pick stations`,
  } } });
  const shown = await write(page, conversation.id, command('show'));
  await drawn(page, `${MAP}?highlights=${shown}`);
  await expect(mount(page)).toHaveAttribute('data-state', 'highlighted');
  await expect.poll(async () => JSON.parse((await keptCamera(page)) || '{}').camera, 'the show frames its marks over the camera kept').not.toEqual(away);
  await page.waitForTimeout(600);
  const framed = JSON.parse(await keptCamera(page)).camera;
  const later = await write(page, conversation.id, command('highlight'));
  await drawn(page, `${MAP}?highlights=${later}`);
  await expect(mount(page)).toHaveAttribute('data-state', 'highlighted');
  await page.waitForTimeout(600);
  expect(JSON.parse(await keptCamera(page)).camera, 'a highlight opens on the camera kept').toEqual(framed);
});

test('ruling D119: the Map opens orthographic, O and the radios switch to orbit and back, and a reload keeps the camera', async ({ page }) => {
  await listen(page);
  await page.goto('/');
  await page.evaluate(() => sessionStorage.clear());
  await drawn(page);
  await expect(mount(page)).toHaveAttribute('data-projection', 'orthographic');
  await expect(page.getByRole('radio', { name: 'Orthographic' })).toBeChecked();
  await stage(page).focus();
  await page.keyboard.press('o');
  await expect(mount(page)).toHaveAttribute('data-projection', 'orbit');
  await expect(page.getByRole('radio', { name: 'Orbit' })).toBeChecked();
  await expect.poll(() => spoken(page)).toContain('Orbit: drag, or Shift with the arrows, turns and tilts.');
  await expect(page.getByRole('button', { name: 'Turn left (Shift+←)' })).toBeVisible();
  await page.keyboard.press('o');
  await expect(mount(page)).toHaveAttribute('data-projection', 'orthographic');
  await page.getByRole('radio', { name: 'Orthographic' }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(mount(page)).toHaveAttribute('data-projection', 'orbit');

  await stage(page).focus();
  await page.keyboard.press('Shift+ArrowLeft');
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('+');
  await expect.poll(() => page.evaluate(() => Object.keys(sessionStorage).filter((key) => key.startsWith('studio.camera.v1:map:')).length)).toBe(1);
  await page.waitForTimeout(600);
  const before = await centreLine(page).textContent();
  const camera = await page.evaluate(() => Object.entries(sessionStorage).find(([key]) => key.startsWith('studio.camera.v1:map:'))?.[1]);
  await page.reload();
  await expect(mount(page)).toHaveAttribute('data-state', /^(drawn|highlighted)$/, { timeout: 60_000 });
  await expect(mount(page)).toHaveAttribute('data-projection', 'orbit');
  await expect(centreLine(page)).toHaveText(before ?? '');
  const after = await page.evaluate(() => Object.entries(sessionStorage).find(([key]) => key.startsWith('studio.camera.v1:map:'))?.[1]);
  expect(JSON.parse(after ?? '{}').camera, 'the camera kept across the reload').toEqual(JSON.parse(camera ?? '{}').camera);
});


test('statement 10: a flow changeset alone draws a new floor and re-reads the caption within 30 s, the camera kept', async ({ page }) => {
  await page.goto('/');
  await page.evaluate(() => sessionStorage.clear());
  const floors: string[] = [];
  page.on('request', (request) => {
    if (request.url().includes('/api/floors/')) floors.push(request.url());
  });
  await drawn(page, MOVED_MAP);
  const caption = page.locator('table.census caption');
  const flow = Number(/flow (\d+)/.exec((await caption.textContent()) ?? '')?.[1]);
  await stage(page).focus();
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('+');
  const keptCamera = () => page.evaluate(() => Object.entries(sessionStorage).find(([key]) => key.startsWith('studio.camera.v1:map:'))?.[1] ?? '');
  await expect.poll(keptCamera).not.toBe('');
  await page.waitForTimeout(600);
  const camera = JSON.parse(await keptCamera()).camera;
  made('--touch-flow', MOVED);
  await expect(caption).toHaveText(new RegExp(`flow ${flow + 1}`), { timeout: 35_000 });
  await expect(mount(page)).toHaveAttribute('data-state', 'drawn');
  await expect(stage(page)).toHaveAttribute('aria-label', `${MOVED_NAME}, map at changeset 1`);
  await expect.poll(() => new Set(floors).size, 'a second floor, under a new key').toBe(2);
  await page.waitForTimeout(600);
  expect(JSON.parse(await keptCamera()).camera, 'the camera stays').toEqual(camera);
});

test('a refresh the server refuses keeps the map as it was read and still followed, and is tried again after 10 s', async ({ page }) => {
  await page.goto('/');
  await page.evaluate(() => sessionStorage.clear());
  await drawn(page, MOVED_MAP);
  const caption = page.locator('table.census caption');
  const flow = Number(/flow (\d+)/.exec((await caption.textContent()) ?? '')?.[1]);
  let refused = 0;
  await page.route('**/api/map/**', async (route) => {
    if (refused > 0) return route.continue();
    refused += 1;
    return route.fulfill({
      status: 503, contentType: 'application/problem+json',
      body: JSON.stringify({ type: 'wps-unavailable', title: 'World graph server unavailable', status: 503, detail: 'The world graph server did not answer.' }),
    });
  });
  made('--touch-flow', MOVED);
  await expect.poll(() => refused, { timeout: 35_000 }).toBe(1);
  await expect(page.getByText('The world graph server did not answer.')).toBeVisible();
  await expect(caption, 'the counts as they were read').toHaveText(new RegExp(`flow ${flow}, counted`));
  await expect(mount(page)).toHaveAttribute('data-state', 'drawn');
  await expect(caption, 'read again once, 10 s later').toHaveText(new RegExp(`flow ${flow + 1}, counted`), { timeout: 20_000 });
  await expect(page.getByText('The world graph server did not answer.')).toHaveCount(0);
});
