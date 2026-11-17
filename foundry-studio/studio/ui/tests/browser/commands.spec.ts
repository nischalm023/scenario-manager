// Spec 03 section 8 in the browser: statement 4 (which browser tab moves: only the initiating tab, focus unchanged by
// every command, no navigation replayed by a reload, a person's navigation or focus in the panel turning a later show
// into a link with the "suggests" sentence) and statement 5 (history: Back after a show restores the previous
// canonical address; a selection change adds no history entry). The `sim_ms` throttle is a unit test (routes.test.ts).
// And spec 02 section 3.5's F1, F5 and F7 where a command meets focus: a same-address click leaves nothing for a later
// show to move, and a show that removes the chat column holding focus lands it on the new panel's title. A duplicated
// browser tab is made as a browser makes one, its `sessionStorage` copied from the first tab's.

import { expect, test, type Page } from '@playwright/test';
import { browserTabOf, chatDrawn, freshSite, listen, openConversation, spoken, write } from './support.ts';
import { message, show, started, toolStarted, ulid } from './turns.ts';

async function settle(page: Page, route: string) {
  await page.goto(route);
  await page.locator('#panel-title').waitFor();
  await chatDrawn(page);
}

/** A turn of `page`'s browser tab, started live on a fresh site's flow route. */
async function turnOf(page: Page) {
  const site = await freshSite();
  const conversation = (await openConversation(page, site)).id;
  await settle(page, `/w/${site}/flow@main`);
  const tab = await browserTabOf(page);
  const turn = ulid();
  await write(page, conversation, message(turn, 'Show me the map.'));
  await write(page, conversation, started(turn, 1, 1, tab), tab);
  await write(page, conversation, toolStarted(turn, 'toolu_cmd', 'panel.show', 'Opening the Map panel'));
  return { site, conversation, turn };
}

const SAID = 'The agent opened the Map panel: the test site, map at its head.';

/** Spec 03 section 8, statement 4 (which browser tab moves). */
test('statement 4: only the initiating tab moves, focus stays, and a reload replays no navigation', async ({ browser }) => {
  const context = await browser.newContext();
  const [a, b] = [await context.newPage(), await context.newPage()];
  await listen(a);
  const { site, conversation, turn } = await turnOf(a);
  await settle(b, `/w/${site}/flow@main`);
  await a.locator('#composer-input').focus();
  await b.locator('#composer-input').focus();
  await write(a, conversation, show(turn, 'toolu_cmd', `/w/${site}/map@main`, 'map', SAID));
  await expect(a).toHaveURL(new RegExp(`/w/${site}/map@main$`));
  await expect(a.locator('#composer-input')).toBeFocused();
  await expect.poll(() => spoken(a)).toContain(SAID);
  await expect(b).toHaveURL(new RegExp(`/w/${site}/flow@main$`));
  await expect(b.locator('#composer-input')).toBeFocused();
  // A reload of A on the flow: the replay is catch-up, so the show navigates nowhere.
  await a.goto(`/w/${site}/flow@main`);
  await chatDrawn(a);
  await expect(a.locator('a.tool-row__link')).toHaveText('Open the Map panel');
  await a.waitForTimeout(1500);
  await expect(a).toHaveURL(new RegExp(`/w/${site}/flow@main$`));
  await context.close();
});

/**
 * Spec 03 section 8, statement 4 (which browser tab moves): a navigation of the person's or focus in the panel holds a
 * show back.
 */
test("statement 4: the person's own navigation since the turn began, or focus in the panel, turns a show into a link", async ({ page }) => {
  await listen(page);
  const first = await turnOf(page);
  // A Setup link other than Maps, which now opens the map the show names, so the address after the show tells.
  await page.locator('#panel-tabs a[data-panel="stations"]').click();
  await expect(page).toHaveURL(/panel=stations$/);
  await page.locator('#composer-input').focus();
  const navigatedSince = page.url();
  // The person navigated after the turn began (the Stations link), so the show is offered as a link.
  await write(page, first.conversation, show(first.turn, 'toolu_cmd', `/w/${first.site}/map@main`, 'map', SAID));
  await expect(page.locator('li.turn__suggestion')).toContainText('The agent suggests the Map panel:');
  await expect(page.locator('li.turn__suggestion a')).toHaveText('Open the Map panel');
  expect(page.url()).toBe(navigatedSince);
  await expect.poll(() => spoken(page)).toContain('The agent suggests the Map panel; the link is in the chat.');

  const second = await turnOf(page);
  await page.locator('#panel-title').focus();
  await write(page, second.conversation, show(second.turn, 'toolu_cmd', `/w/${second.site}/map@main`, 'map', SAID));
  await expect(page.locator('li.turn__suggestion')).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`/w/${second.site}/flow@main$`));
  await expect(page.locator('#panel-title')).toBeFocused();
});

/** Spec 03 section 8, statement 12 (offer a link). */
test('spec 03 statement 12: with "Offer a link in the chat" set, a show leaves the address and the suggestion is spoken', async ({ page }) => {
  await listen(page);
  const { site, conversation, turn } = await turnOf(page);
  await page.getByRole('link', { name: 'Settings' }).click();
  await expect(page).toHaveURL(/\/settings$/);
  await page.getByRole('switch', { name: 'Switch to it' }).click();
  await expect(page.getByRole('switch', { name: 'Switch to it' })).toHaveAttribute('aria-checked', 'false');
  await page.goBack();
  // Settings is a page of its own since PR 600, with no chat: the show is written once the chat is drawn again, so it
  // arrives live rather than in the catch-up, which navigates and offers nothing.
  await chatDrawn(page);
  await write(page, conversation, show(turn, 'toolu_cmd', `/w/${site}/map@main`, 'map', SAID));
  await expect(page.locator('li.turn__suggestion')).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`/w/${site}/flow@main$`));
  await expect.poll(() => spoken(page)).toContain('The agent suggests the Map panel; the link is in the chat.');
  await page.getByRole('link', { name: 'Settings' }).click();
  await page.getByRole('switch', { name: 'Switch to it' }).click();
  await expect(page.getByRole('switch', { name: 'Switch to it' })).toHaveAttribute('aria-checked', 'true');
});

/** Spec 03 section 8, statement 5 (history). */
test('statement 5: Back after a show restores the previous address; a selection change adds no history entry', async ({ page }) => {
  const { site, conversation, turn } = await turnOf(page);
  const before = page.url();
  await write(page, conversation, show(turn, 'toolu_cmd', `/w/${site}/map@main`, 'map', SAID));
  await expect(page).toHaveURL(new RegExp(`/w/${site}/map@main$`));
  await expect(page.locator('#panel-tabs a[data-highlight="agent"]')).toContainText('Opened by the agent');
  const length = await page.evaluate(() => history.length);
  await write(page, conversation, { entry: { seq: 0, kind: 'panel.command', turn, caused_by: 'toolu_cmd', body: {
    action: 'select', panel: 'map', location: `${site}/map@main`, resolved: [], select: 'b6a9a763-687c-4067-909b-460e986e2229',
    layers: null, draft: null, sim_ms: null, compare: null, metrics: null, highlight: null,
    route: `/w/${site}/map@main?select=b6a9a763-687c-4067-909b-460e986e2229`, as_it_was: null, announce: 'The agent selected Pick station 3 on the Map panel',
  } } });
  await expect(page).toHaveURL(/select=b6a9a763-687c-4067-909b-460e986e2229$/);
  expect(await page.evaluate(() => history.length)).toBe(length);
  await page.goBack();
  await expect(page).toHaveURL(before);
  await expect(page.locator('#panel-tabs a[data-highlight="agent"]')).toHaveCount(0);
});

/** Spec 02 section 3.5, F1 and F7: an agent's navigation never moves focus. */
test('spec 02 F1 and F7: a click on the address already shown leaves nothing pending, so a later show moves no focus', async ({ page }) => {
  const site = await freshSite();
  const conversation = (await openConversation(page, site)).id;
  await settle(page, `/w/${site}/flow@main`);
  await page.locator('#panel-tabs a', { hasText: 'Process flow' }).click();
  await expect(page.locator('#panel-title')).toBeFocused();
  await page.locator('#composer-input').focus();
  const tab = await browserTabOf(page);
  const turn = ulid();
  await write(page, conversation, message(turn, 'Show me the map.'));
  await write(page, conversation, started(turn, 1, 1, tab), tab);
  await write(page, conversation, toolStarted(turn, 'toolu_same', 'panel.show', 'Opening the Map panel'));
  await write(page, conversation, show(turn, 'toolu_same', `/w/${site}/map@main`, 'map', SAID));
  await expect(page).toHaveURL(new RegExp(`/w/${site}/map@main$`));
  await page.waitForTimeout(300);
  await expect(page.locator('#composer-input')).toBeFocused();
});

/** Spec 02 section 3.5, F5 and F7: focus never falls to body when a navigation replaces what held it. */
test('spec 02 F5 and F7: a show of a route without a chat column, the composer focused, lands focus on the new panel title', async ({ page }) => {
  const { conversation, turn } = await turnOf(page);
  await page.locator('#composer-input').focus();
  await write(page, conversation, { entry: { seq: 0, kind: 'panel.command', turn, caused_by: 'toolu_cmd', body: {
    action: 'show', panel: 'runs', location: null, resolved: [], select: null, layers: null, draft: null, sim_ms: null,
    compare: null, metrics: null, highlight: null, route: '/runs', as_it_was: null, announce: 'The agent opened the Runs panel.',
  } } });
  await expect(page).toHaveURL(/\/runs$/);
  await expect(page.locator('#chat')).toHaveCount(0);
  await expect(page.locator('#panel-title')).toBeFocused();
});

/** A second browser tab that copied `from`'s `sessionStorage`, as a duplicated tab does, opened on `route`. */
async function duplicateOf(from: Page, route: string, before?: () => Promise<void>) {
  const copied = await browserTabOf(from);
  const tab = await from.context().newPage();
  await tab.addInitScript((id) => sessionStorage.setItem('agent-studio.browser-tab', id), copied);
  await before?.();
  await settle(tab, route);
  return { tab, copied };
}

/**
 * Spec 03 section 8, statement 4 (which browser tab moves), for a duplicated tab: it settles an id of its own before
 * its conversation opens, and the first tab's turn moves the first tab alone.
 */
test('statement 4, a duplicated tab: its own id before its chat opens, and none of the first tab\'s turns move it', async ({ browser }) => {
  const context = await browser.newContext();
  const a = await context.newPage();
  const { site, conversation, turn } = await turnOf(a);
  const { tab: b, copied } = await duplicateOf(a, `/w/${site}/flow@main`);
  expect(await browserTabOf(b)).not.toBe(copied);
  expect(await browserTabOf(a)).toBe(copied);
  await write(a, conversation, show(turn, 'toolu_cmd', `/w/${site}/map@main`, 'map', SAID));
  await expect(a).toHaveURL(new RegExp(`/w/${site}/map@main$`));
  await b.waitForTimeout(1000);
  await expect(b).toHaveURL(new RegExp(`/w/${site}/flow@main$`));
  await context.close();
});

/**
 * Spec 03 section 8, statement 4, for a duplicate whose probe the first tab answers late: the duplicate settles on the
 * copied id, then takes a new one when the answer comes; its next message carries the new id, and a turn of the first
 * tab moves the first tab alone.
 */
test('statement 4, a duplicate answered late: its id changes after it settled, and its posts and commands follow', async ({ browser }) => {
  const context = await browser.newContext();
  const a = await context.newPage();
  const site = await freshSite();
  const conversation = (await openConversation(a, site)).id;
  await settle(a, `/w/${site}/flow@main`);
  let busy: Promise<void> = Promise.resolve();
  // The first tab is busy for 4 s, so the duplicate's probe goes unanswered past its wait.
  const { tab: b, copied } = await duplicateOf(a, `/w/${site}/flow@main`, async () => {
    busy = a.evaluate(() => {
      const until = Date.now() + 4000;
      while (Date.now() < until) { /* the first tab's main thread is busy */ }
    });
  });
  expect(await browserTabOf(b)).toBe(copied);
  await busy;
  await expect.poll(() => browserTabOf(b), { timeout: 10_000 }).not.toBe(copied);
  const fresh = await browserTabOf(b);
  await chatDrawn(b);
  // The message is answered here, as scenario.spec's is, so no turn starts on the model: the turn below is the only one.
  await b.route((url) => /\/api\/conversations\/[^/]+\/messages$/.test(url.pathname), (route) =>
    route.fulfill({ status: 503, json: { type: 'agent-busy', title: 'Busy', status: 503, detail: 'Held by the test.' } }));
  const sent = b.waitForRequest((request) => request.method() === 'POST' && request.url().endsWith('/messages'));
  await b.locator('#composer-input').fill('Which aisle is the slowest?');
  await b.keyboard.press('Enter');
  expect(((await sent).postDataJSON() as { browser_tab: string }).browser_tab).toBe(fresh);
  const turn = ulid();
  await write(a, conversation, message(turn, 'Show me the map.'));
  await write(a, conversation, started(turn, 1, 2, copied), copied);
  await write(a, conversation, toolStarted(turn, 'toolu_late', 'panel.show', 'Opening the Map panel'));
  await write(a, conversation, show(turn, 'toolu_late', `/w/${site}/map@main`, 'map', SAID));
  await expect(a).toHaveURL(new RegExp(`/w/${site}/map@main$`));
  await b.waitForTimeout(1000);
  await expect(b).toHaveURL(new RegExp(`/w/${site}/flow@main$`));
  await context.close();
});
