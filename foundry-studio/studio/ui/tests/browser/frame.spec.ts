// Spec 02 section 8 in the browser, the frame: statements 1 (one live region), 11 (axe, light and dark), 15 (no run
// budget), 16 (reflow at 320 by 640), 19 (regions by key), 20 (the ring wins over the agent's mark), Home's half of 30
// and the tab row's part of 33 (a disabled tab's reason in its tooltip), on every React route this task builds, with
// the chat holding the journal fixtures' cards.

import { existsSync, readdirSync, readFileSync } from 'node:fs';
import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { browserTabOf, chatDrawn, fixture, freshSite, freshThread, listen, madeLShape, openConversation, replay, write } from './support.ts';
import { message, show, started, toolStarted, ulid } from './turns.ts';

const RUN = '01KN0TJZANP5N0PGJD8F7A3FDB';

/** A site whose conversation holds the site fixture, and one whose turn waits on an approval. */
async function sites(page: Page) {
  const done = await freshSite();
  await replay(page, (await openConversation(page, done)).id, fixture('site-conversation'));
  const waiting = await freshSite();
  await replay(page, (await openConversation(page, waiting)).id, fixture('ended-turns'), { upTo: 13 });
  return { done, waiting };
}

function routes(done: string, waiting: string): string[] {
  return [
    '/', '/templates', `/w/${done}/map@main`, `/w/${done}/flow@main`, `/w/${done}@main?panel=history`, `/w/${done}@main?panel=template`,
    `/w/${waiting}/map@main`, `/w/${done}/fleet@main`, '/w/customer/demo@main', '/scenarios', `/scenarios?site=${encodeURIComponent(done)}`,
    '/runs', `/runs/${RUN}`, '/nowhere',
  ];
}

async function settle(page: Page, route: string) {
  await page.goto(route);
  await page.locator('#panel-title').waitFor();
  if (await page.locator('#chat').count()) await chatDrawn(page);
}

/** Spec 02 section 8, statement 1 (one live region). */
test('statement 1: #announcer is the only live region on every route', async ({ page }) => {
  const { done, waiting } = await sites(page);
  for (const route of routes(done, waiting)) {
    await settle(page, route);
    const live = await page.evaluate(() => [...document.querySelectorAll('[aria-live], [role="log"], [role="status"], [role="alert"]')].map((element) => element.id || element.tagName));
    expect(live, route).toEqual(['announcer']);
    const announcer = page.locator('body > :last-child');
    await expect(announcer).toHaveAttribute('id', 'announcer');
  }
});

/** Spec 02 section 8, statement 11 (axe-core), light and dark. */
test('statement 11: axe finds no serious or critical violation, light and dark', async ({ page }) => {
  const { done, waiting } = await sites(page);
  await replay(page, (await freshThread(page)).id, fixture('new-site-thread'), { upTo: 9 });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    for (const route of routes(done, waiting)) {
      await settle(page, route);
      const results = await new AxeBuilder({ page }).analyze();
      const grave = results.violations.filter((violation) => violation.impact === 'serious' || violation.impact === 'critical')
        .map((violation) => `${violation.id}: ${violation.nodes.map((node) => node.target.join(' ')).slice(0, 3).join(' | ')}`);
      expect(grave, `${scheme} ${route}`).toEqual([]);
    }
  }
});

/** Spec 02 section 8, statement 15 (no run budget). */
test('statement 15: the built bundle and the mockups hold no run-budget sentence', () => {
  const sentences = [/Runs in this conversation: \d+ of \d+/, /Runs left in this conversation/, /run budget/i];
  const assets = new URL('../../dist/assets/', import.meta.url);
  expect(existsSync(assets), 'build the bundle first (bun run build)').toBe(true);
  const mockups = `${process.env.HOME}/vt/greyorange/notes/agent-studio-2026-10-17/design/mockups/`;
  const files = [
    ...readdirSync(assets).map((name) => new URL(name, assets).pathname),
    ...(existsSync(mockups) ? readdirSync(mockups).filter((name) => name.endsWith('.html')).map((name) => mockups + name) : []),
  ];
  for (const file of files) {
    const text = readFileSync(file, 'utf8');
    for (const sentence of sentences) expect(sentence.test(text), `${file} holds ${sentence}`).toBe(false);
  }
});

/** Spec 02 section 8, statement 16 (reflow). */
test('statement 16: at 320 by 640 CSS px no page scrolls sideways', async ({ page }) => {
  const { done, waiting } = await sites(page);
  await page.setViewportSize({ width: 320, height: 640 });
  for (const route of routes(done, waiting)) {
    await settle(page, route);
    const wide = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(wide, route).toBeLessThanOrEqual(0);
  }
});

/**
 * Ruling D192 (the UI fast-follow after PR 600): the app fills the window and only its panes scroll, phone to wide
 * screen.
 */
test('the document never scrolls: from 390 to 1920 CSS px wide, every route fills the window exactly', async ({ page }) => {
  const { done, waiting } = await sites(page);
  for (const [width, height] of [[390, 844], [768, 1024], [1280, 800], [1920, 1080]]) {
    await page.setViewportSize({ width, height });
    for (const route of routes(done, waiting)) {
      await settle(page, route);
      const beyond = await page.evaluate(() => ({
        down: document.documentElement.scrollHeight - document.documentElement.clientHeight,
        sideways: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      }));
      expect(beyond, `${route} at ${width} by ${height}`).toEqual({ down: 0, sideways: 0 });
    }
  }
});

/**
 * Ruling D192 (the UI fast-follow after PR 600): a map, a floor or a flow drawing fills the window and returns, by its
 * button and by Escape.
 */
test('the map and the flow drawing fill the window with their lists, the rest hidden, and return by Escape and by the button', async ({ page }) => {
  const site = await madeLShape();
  await openConversation(page, site);
  const fill = page.getByRole('button', { name: 'Fill the window' });
  const covers = (selector: string) => page.locator(selector).evaluate((element) => {
    const box = element.getBoundingClientRect();
    return [box.x, box.y, box.width, box.height].map(Math.round).join(' ');
  });
  const whole = await page.evaluate(() => `0 0 ${document.documentElement.clientWidth} ${document.documentElement.clientHeight}`);
  for (const [route, layout] of [[`/w/${site}/map@main`, '.map-layout'], [`/w/${site}/flow@main`, '.flow-layout']]) {
    await settle(page, route);
    await fill.click();
    await expect(fill).toHaveAttribute('aria-pressed', 'true');
    expect(await covers(layout), route).toBe(whole);
    // The rest of the shell is out of sight and out of the Tab order; the view's own lists and tools are not.
    await expect(page.locator('#chat')).toBeHidden();
    await expect(page.locator('#panel-tabs')).toBeHidden();
    await expect(page.locator(`${layout} :is(.side-stack, .flow-explorer)`).first()).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(fill).toHaveAttribute('aria-pressed', 'false');
    await expect(fill).toBeFocused();
    await expect(page.locator('#chat')).toBeVisible();
    await fill.click();
    await fill.click();
    await expect(page.locator('[data-filled]')).toHaveCount(0);
  }
});

/**
 * Ruling D192 (the UI fast-follow after PR 600): Escape from the flow's explorer leaves a filled window only once it
 * has nothing of the explorer's left to dismiss, its item's hover card first, then the selection.
 */
test("Escape on a filled flow's explorer item closes its card, clears the selection, then returns", async ({ page }) => {
  const site = await madeLShape();
  await openConversation(page, site);
  await settle(page, `/w/${site}/flow@main`);
  const fill = page.getByRole('button', { name: 'Fill the window' });
  await fill.click();
  const item = page.locator('.flow-explorer__item').first();
  const card = page.locator(`#flow-hover-${await item.getAttribute('data-ref')}`);
  // Reached by the keyboard, as a card opens on keyboard focus only.
  await item.focus();
  await page.keyboard.press('Shift+Tab');
  await page.keyboard.press('Tab');
  await expect(card).toBeVisible();
  await page.keyboard.press('Enter');
  await expect(item).toHaveAttribute('aria-current', 'true');
  await page.keyboard.press('Escape');
  await expect(card).toBeHidden();
  await expect(fill).toHaveAttribute('aria-pressed', 'true');
  await page.keyboard.press('Escape');
  await expect(item).not.toHaveAttribute('aria-current', 'true');
  await expect(fill).toHaveAttribute('aria-pressed', 'true');
  await page.keyboard.press('Escape');
  await expect(fill).toHaveAttribute('aria-pressed', 'false');
  await expect(page.locator('#chat')).toBeVisible();
});

/**
 * Ruling D192 (the UI fast-follow after PR 600): in a filled flow, Escape that closes an edge's hover card or an "i"
 * tip closes only that, and the next Escape returns; a filled window once returned on the same press.
 */
test("Escape in a filled flow closes an edge's card or an \"i\" tip first, then returns", async ({ page }) => {
  const site = await madeLShape();
  await openConversation(page, site);
  // The Material flow view, where no agent label lies over the material one hovered below.
  await settle(page, `/w/${site}/flow@main?layers=material`);
  const fill = page.getByRole('button', { name: 'Fill the window' });
  await fill.click();
  await page.locator('.flow-canvas text.flow-edge__label[data-layer="material"]', { hasText: 'stock tote' }).first().hover();
  const edgeCard = page.locator('.flow-canvas .flow-hover--edge');
  await expect(edgeCard).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(edgeCard).toBeHidden();
  await expect(fill).toHaveAttribute('aria-pressed', 'true');
  // A selected node's settings carry an "i", pinned open by a click.
  await page.locator('.flow-explorer__item', { hasText: 'Pick stations' }).first().click();
  const about = page.getByRole('button', { name: 'About these settings' });
  await about.click();
  const tip = page.locator(`[id="${await about.getAttribute('aria-controls')}"]`);
  await expect(tip).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(tip).toBeHidden();
  await expect(fill).toHaveAttribute('aria-pressed', 'true');
  await page.keyboard.press('Escape');
  await expect(fill).toHaveAttribute('aria-pressed', 'false');
});

/**
 * Ruling D192 (the UI fast-follow after PR 600): an "i" tip pinned in the page's header closes as the press on Fill the
 * window leaves it, so the first Escape returns; the tip once stayed pinned behind the filled map, out of sight, and
 * took that Escape.
 */
test('an "i" tip pinned beside the map closes as the map fills, so one Escape returns', async ({ page }) => {
  const site = await madeLShape();
  await openConversation(page, site);
  await settle(page, `/w/${site}/map@main`);
  // Hidden with the rest of the shell while the map fills, and still read.
  const about = page.getByRole('button', { name: 'About this map', includeHidden: true });
  await about.click();
  await expect(about).toHaveAttribute('aria-expanded', 'true');
  const fill = page.getByRole('button', { name: 'Fill the window' });
  await fill.click();
  await expect(fill).toHaveAttribute('aria-pressed', 'true');
  await expect(about).toHaveAttribute('aria-expanded', 'false');
  await page.keyboard.press('Escape');
  await expect(fill).toHaveAttribute('aria-pressed', 'false');
});

/**
 * Ruling D192 (the UI fast-follow after PR 600): a filled flow keeps its explorer beside the drawing and the page keeps
 * it under the drawing, so returning brings the focused item into view; it once stayed below the fold.
 */
test('returning from a filled flow by Escape brings the focused explorer item into view', async ({ page }) => {
  const site = await madeLShape();
  await openConversation(page, site);
  await settle(page, `/w/${site}/flow@main`);
  const fill = page.getByRole('button', { name: 'Fill the window' });
  await fill.click();
  const item = page.locator('.flow-explorer__item').last();
  // Reached by the keyboard, whose focus opens the item's card: the first Escape closes it, the second returns.
  await item.focus();
  await page.keyboard.press('Shift+Tab');
  await page.keyboard.press('Tab');
  await page.keyboard.press('Escape');
  await page.keyboard.press('Escape');
  await expect(fill).toHaveAttribute('aria-pressed', 'false');
  await expect(item).toBeFocused();
  await expect(item).toBeInViewport();
});

/**
 * Ruling D192 (the UI fast-follow after PR 600): at 390 CSS px a filled flow stacks as the page does, the drawing
 * first and as wide as the window allows; its wide layout once kept an 18rem explorer beside it and left it no width.
 */
test('at 390 px a filled flow drawing takes the width, its lists beneath it', async ({ page }) => {
  const site = await madeLShape();
  await openConversation(page, site);
  await page.setViewportSize({ width: 390, height: 844 });
  await settle(page, `/w/${site}/flow@main`);
  await page.getByRole('button', { name: 'Fill the window' }).click();
  const box = async (selector: string) => {
    const found = await page.locator(selector).boundingBox();
    if (found === null) throw new Error(`${selector} is not drawn`);
    return found;
  };
  const [layout, drawing, explorer] = [await box('.flow-layout'), await box('.flow-canvas'), await box('.flow-explorer')];
  // The filled view's padding is 16 px a side.
  expect(Math.round(drawing.width)).toBe(Math.round(layout.width) - 32);
  expect(drawing.height).toBeGreaterThan(layout.height / 2);
  expect(explorer.y).toBeGreaterThanOrEqual(drawing.y + drawing.height);
});

/**
 * SPEC.md section 3.5 (the accessibility contract): a filled view hides the page's landmarks with the rest of the
 * shell, so it is a region of its own, named for what fills the window; it once stood in no landmark at all.
 */
test('a filled map or flow drawing is a region named for what fills the window, and no region once it returns', async ({ page }) => {
  const site = await madeLShape();
  await openConversation(page, site);
  const fill = page.getByRole('button', { name: 'Fill the window' });
  for (const [route, name] of [[`/w/${site}/map@main`, 'Map'], [`/w/${site}/flow@main`, 'Process flow']]) {
    await settle(page, route);
    const region = page.getByRole('region', { name: `${name}, filling the window` });
    await expect(region).toHaveCount(0);
    await fill.click();
    await expect(region.getByRole('button', { name: 'Fill the window' })).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(region).toHaveCount(0);
  }
});

/** SPEC.md section 3.5 (the accessibility contract): the narrow layout once removed the banner it lists. */
test('at 390 px the banner, which only names the app, is out of sight and still a landmark', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await settle(page, '/');
  const banner = page.getByRole('banner');
  await expect(banner).toHaveText('Agent Studio');
  expect(await banner.evaluate((element) => element.getBoundingClientRect().height)).toBeLessThanOrEqual(1);
});

/**
 * Ruling D192 (the UI fast-follow after PR 600): beside the open chat, the site's map and Configure map draw their floor
 * at least 20rem wide, or as wide as their layout where it is narrower, the floor's toolbar clear of the lists; their
 * two columns once left the floor a few pixels at 1024 px, its toolbar standing over the lists.
 */
test("beside the open chat, the site's map and Configure map draw their floor at a usable width, its toolbar clear of their lists", async ({ page }) => {
  const site = await madeLShape();
  await openConversation(page, site);
  const started = await page.request.post('/api/template-drafts', {
    data: { template: 'template/site/rectangle' }, headers: { 'idempotency-key': crypto.randomUUID() },
  });
  const { draft } = (await started.json()) as { draft: string };
  type Box = { x: number; y: number; width: number; height: number };
  const box = async (selector: string): Promise<Box> => {
    const found = await page.locator(selector).first().boundingBox();
    if (found === null) throw new Error(`${selector} is not drawn`);
    return found;
  };
  const apart = (a: Box, b: Box) => a.x + a.width <= b.x || b.x + b.width <= a.x || a.y + a.height <= b.y || b.y + b.height <= a.y;
  for (const width of [1024, 1280, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    for (const [route, layout, lists] of [[`/w/${site}/map@main`, '.map-layout', '.side-stack'], [`/template-drafts/${draft}`, '.template-layout', '.knob-form']]) {
      await settle(page, route);
      await expect(page.locator('#chat')).toBeVisible();
      await page.locator('.worldview-mount__stage').first().waitFor({ state: 'attached' });
      const [whole, floor, tools, beside] = [await box(layout), await box('.worldview-mount__stage'), await box('.worldview-mount .stage-tools'), await box(lists)];
      expect(floor.width, `${route} at ${width}`).toBeGreaterThanOrEqual(Math.min(320, Math.floor(whole.width)));
      expect(apart(tools, beside), `${route} at ${width}: the toolbar stands clear of the lists`).toBe(true);
    }
  }
});

/** Spec 02 section 8, statement 19 (regions by key). */
test('statement 19: F6 and Ctrl+Grave accent visit the composer, the newest turn, the current tab and the panel', async ({ page }) => {
  const { done } = await sites(page);
  await settle(page, `/w/${done}/flow@main`);
  const focused = () => page.evaluate(() => {
    const element = document.activeElement as HTMLElement | null;
    return element?.id || element?.textContent?.trim().slice(0, 30) || element?.tagName || '';
  });
  const order = ['composer-input', 'turn-2', 'Process flowRouting and task s', 'panel'];
  for (const [key, back] of [['F6', 'Shift+F6'], ['Control+Backquote', 'Control+Shift+Backquote']]) {
    await page.locator('body').click({ position: { x: 1, y: 1 } });
    const forward: string[] = [];
    for (let n = 0; n < 5; n += 1) {
      await page.keyboard.press(key);
      forward.push(await focused());
    }
    expect(forward, key).toEqual([...order, order[0]]);
    const backward: string[] = [];
    for (let n = 0; n < 4; n += 1) {
      await page.keyboard.press(back);
      backward.push(await focused());
    }
    expect(backward, back).toEqual(['panel', 'Process flowRouting and task s', 'turn-2', 'composer-input']);
  }
  // The dashboard has no site, so no Setup section: the panel is the one region.
  await settle(page, '/');
  await page.keyboard.press('F6');
  expect(await focused()).toBe('panel');
  await page.keyboard.press('F6');
  expect(await focused()).toBe('panel');
});

/** Spec 02 section 8, statement 20 (the ring wins). */
test("statement 20: Tab to the agent's marked tab shows the 2 px solid ring, light, dark and under forced colours", async ({ page }) => {
  const site = await freshSite();
  const conversation = (await openConversation(page, site)).id;
  await settle(page, `/w/${site}/flow@main`);
  const tab = await browserTabOf(page);
  const turn = ulid();
  await write(page, conversation, message(turn, 'Show me the map.'));
  await write(page, conversation, started(turn, 1, 1, tab), tab);
  await write(page, conversation, toolStarted(turn, 'toolu_ring', 'panel.show', 'Opening the Map panel'));
  await write(page, conversation, show(turn, 'toolu_ring', `/w/${site}/map@main`, 'map', 'The agent opened the Map panel.'));
  await expect(page).toHaveURL(new RegExp(`/w/${site}/map@main$`));
  const marked = page.locator('#panel-tabs a[data-highlight="agent"]');
  await expect(marked).toHaveText('MapsOpened by the agent');
  for (const look of [{ colorScheme: 'light' }, { colorScheme: 'dark' }, { forcedColors: 'active' }] as const) {
    await page.emulateMedia(look);
    // Maps heads the Setup links, so the keyboard reaches it back from the link after it.
    await page.locator('#panel-tabs a[data-panel="orders"]').focus();
    await page.keyboard.press('Shift+Tab');
    await expect(marked).toBeFocused();
    const ring = await marked.evaluate((element) => {
      const style = getComputedStyle(element);
      return `${style.outlineStyle} ${style.outlineWidth}`;
    });
    expect(ring, JSON.stringify(look)).toBe('solid 2px');
  }
});

/** Spec 02 section 8, statement 33 (the simplified chat), the tab row's part, with keys as a reader uses them. */
test("statement 33's tab row: a disabled tab is in the Tab order, its tooltip shows on focus and goes on Escape", async ({ page }) => {
  // The Setup links show beside a new site's map presets too, where none of a site's own pages opens yet.
  await settle(page, '/templates');
  const flow = page.locator('#panel-tabs a[data-panel="flow"]');
  await expect(flow).toHaveAttribute('aria-disabled', 'true');
  await expect(flow).toHaveText('Process flowRouting and task sequencing');
  await expect(page.locator('#panel-tabs')).not.toContainText('(open a site first)');
  const tooltip = page.locator('#panel-tab-flow-why');
  await expect(tooltip).toBeHidden();
  await page.locator('#panel-tabs a[data-panel="agents"]').focus();
  await page.keyboard.press('Tab');
  await expect(flow).toBeFocused();
  await expect(tooltip).toBeVisible();
  await expect(tooltip).toHaveText('Open a site first');
  await expect(flow).toHaveAccessibleDescription('Open a site first');
  await page.keyboard.press('Escape');
  await expect(tooltip).toBeHidden();
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/\/templates$/);
});

/** Spec 02 section 8, statement 30 (Home and Dismiss), its Home half. */
test("statement 30, Home's half: New site and \"Open a site\", and no conversation list", async ({ page }) => {
  await listen(page);
  await settle(page, '/');
  await expect(page.locator('.shell')).toHaveAttribute('data-chat', 'absent');
  await expect(page.locator('#chat')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'New site' })).toBeVisible();
  // Home names the sites to open "Recent sites" since its redesign.
  await expect(page.getByRole('heading', { level: 3, name: 'Recent sites' })).toBeVisible();
  await expect(page.locator('#panel')).not.toContainText('conversation list');
  await expect(page.locator('#panel-tabs [aria-current="page"]')).toHaveCount(0);
  await page.getByRole('button', { name: 'New site' }).click();
  await expect(page).toHaveURL(/\/templates$/);
  await expect(page.locator('#chat-title')).toHaveText('New site');
  await expect(page.locator('#panel-title')).toBeFocused();
});
