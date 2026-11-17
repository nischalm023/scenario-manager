// Spec 02 section 8 in the browser, the chat: statements 10 (the approval card's Tab order, with no jump link since
// D117), 23 (the question card), Dismiss's half of 30, 16 for a reply's table, and statements 8, 9 and 26 as far as the
// API's test writer reaches: a card it writes has no row, so an answer or decision on it answers not-found (brief
// I1.03 section 2), and the attribution route is not mounted yet (404); the chat says either.

import { expect, test, type Page } from '@playwright/test';
import { browserTabOf, chatDrawn, fixture, freshSite, freshThread, listen, openConversation, replay, spoken, write } from './support.ts';
import { finished, message, show, started, text, toolFinished, toolStarted, ulid } from './turns.ts';

async function settle(page: Page, route: string) {
  await page.goto(route);
  await page.locator('#panel-title').waitFor();
  await chatDrawn(page);
}

const focusedId = (page: Page) => page.evaluate(() => document.activeElement?.id ?? '');

/** Spec 02 section 8, statement 10 (approval order). */
test("statement 10: no jump link; Tab from the approval card's heading reaches the exact input, the reason, then approve", async ({ page }) => {
  const site = await freshSite();
  await replay(page, (await openConversation(page, site)).id, fixture('ended-turns'), { upTo: 13 });
  await settle(page, `/w/${site}/map@main`);
  await expect(page.locator('#chat .chat__header a')).toHaveCount(0);
  await page.locator('#approval-01M588MFY0CS240YCSV4E4BFBK-title').focus();
  await page.keyboard.press('Tab');
  await expect(page.locator('.approval-card__exact > summary')).toBeFocused();
  await page.keyboard.press('Tab');
  expect(await focusedId(page)).toBe('approval-01M588MFY0CS240YCSV4E4BFBK-reason');
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'Apply', exact: true })).toBeFocused();
});

/** Spec 02 section 8, statement 23 (question card). */
test("statement 23: no option starts checked; sending with an item unanswered focuses its first option and A12 counts it", async ({ page }) => {
  await listen(page);
  const thread = await freshThread(page);
  await replay(page, thread.id, fixture('new-site-thread'), { upTo: 9 });
  await settle(page, '/templates');
  const card = page.locator('form.question-card');
  await expect(card.locator('input:checked')).toHaveCount(0);
  await card.getByLabel('L (recommended)').check();
  await card.getByRole('button', { name: 'Send' }).focus();
  await page.keyboard.press('Enter');
  expect(await focusedId(page)).toBe('question-01M587G5TGB5PY5PCZS6P8RWW0-1-0');
  await expect(card.locator('p.question-card__error')).toHaveText(['Choose an answer.']);
  await expect.poll(() => spoken(page)).toContain('One question still needs an answer.');
  // Both answered: the test writer's question has no row, so the answers route answers not-found, and the card says
  // why it was not sent and keeps focus (F4).
  await card.getByLabel('6', { exact: true }).check();
  await card.getByRole('button', { name: 'Send' }).click();
  await expect(card.locator('p.chat-card__error')).toHaveText('Not sent: No question has that id.');
  await expect(card.getByRole('button', { name: 'Send' })).toBeFocused();
});

/** Spec 02 section 8, statement 30 (Home and Dismiss), its Dismiss half. */
test("statement 30, Dismiss's half: unavailable while a turn works; after it focus is on the composer, the log is the new conversation, another tab hears A15 once", async ({ browser }) => {
  const context = await browser.newContext();
  const [a, b] = [await context.newPage(), await context.newPage()];
  await listen(a);
  await listen(b);
  const site = await freshSite();
  const conversation = (await openConversation(a, site)).id;
  const turn = ulid();
  await write(a, conversation, message(turn, 'Which aisle is the slowest?'));
  await write(a, conversation, started(turn, 1, 1, null));
  await settle(a, `/w/${site}/map@main`);
  await settle(b, `/w/${site}/flow@main`);
  const dismiss = a.locator('button.chat__dismiss');
  await expect(dismiss).toHaveAttribute('aria-disabled', 'true');
  await expect(a.locator('#chat-dismiss-desc')).toContainText("Dismiss is available once the agent's turn ends.");
  await write(a, conversation, finished(turn, 'done', null));
  await expect(dismiss).not.toHaveAttribute('aria-disabled', 'true');
  await dismiss.click();
  await expect(a.locator('#chat-log li[data-kind="welcome"]')).toBeVisible();
  await expect(a.locator('#composer-input')).toBeFocused();
  await expect.poll(() => spoken(a)).toContain('Dismissed; a new conversation continues this site.');
  await expect(b.locator('#chat-log li[data-kind="welcome"]')).toBeVisible();
  await expect(b.locator('#chat-log')).toContainText('Developer dismissed this conversation; a new one continues this site.');
  const heard = (await spoken(b)).filter((sentence) => sentence.includes('dismissed this conversation'));
  expect(heard).toEqual(['Developer dismissed this conversation; a new one continues this site.']);
  expect((await spoken(a)).filter((sentence) => sentence.includes('dismissed this conversation'))).toEqual([]);
  await context.close();
});

/** Spec 02 section 8, statement 8 (two tabs), as far as the test writer reaches. */
test('statement 8, as far as the test writer reaches: a show moves tab A only; B shows the links and works; an answer turns both cards', async ({ browser }) => {
  const context = await browser.newContext();
  const [a, b] = [await context.newPage(), await context.newPage()];
  const site = await freshSite();
  const conversation = (await openConversation(a, site)).id;
  await settle(a, `/w/${site}/flow@main`);
  await settle(b, `/w/${site}/flow@main`);
  const tabA = await browserTabOf(a);
  expect(tabA).not.toBe(await browserTabOf(b));
  const turn = ulid();
  await write(a, conversation, message(turn, 'Show me the map.'));
  await write(a, conversation, started(turn, 1, 1, tabA), tabA);
  await write(a, conversation, toolStarted(turn, 'toolu_two', 'panel.show', 'Opening the Map panel'));
  await write(a, conversation, show(turn, 'toolu_two', `/w/${site}/map@main`, 'map', 'The agent opened the Map panel.'));
  await write(a, conversation, toolFinished(turn, 'toolu_two', 'ok', 'Opened the Map panel'));
  await expect(a).toHaveURL(new RegExp(`/w/${site}/map@main$`));
  await expect(b).toHaveURL(new RegExp(`/w/${site}/flow@main$`));
  await expect(b.locator('a.tool-row__link')).toHaveText('Open the Map panel');
  // B's composer is `working`: Stop shows, Send sends nothing, and Enter says why (Playwright clicks no aria-disabled).
  await expect(b.getByRole('button', { name: 'Stop' })).toBeVisible();
  await expect(b.locator('#composer button[type="submit"]')).toHaveAttribute('aria-disabled', 'true');
  await expect(b.locator('#composer-status')).toHaveText('');
  await b.locator('#composer-input').press('Enter');
  await expect(b.locator('#composer-status')).toHaveText('The agent is working. Stop it or wait; your message stays in the box.');
  // The question and its answer through the test writer: the answer turns the card answered in both tabs.
  const thread = fixture('new-site-thread');
  const asked = thread.find((line) => 'entry' in line && line.entry.kind === 'question.asked');
  const answered = thread.find((line) => 'entry' in line && line.entry.kind === 'question.answered');
  if (!asked || !answered || !('entry' in asked) || !('entry' in answered)) throw new Error('the fixture holds a question and its answer');
  await write(a, conversation, { entry: { ...asked.entry, turn } });
  await expect(b.locator('form.question-card')).toBeVisible();
  await write(a, conversation, { entry: { ...answered.entry, turn } });
  await expect(a.locator('.question-card[data-state="answered"]')).toBeVisible({ timeout: 2000 });
  await expect(b.locator('.question-card[data-state="answered"]')).toBeVisible({ timeout: 2000 });
  await context.close();
});

/** Spec 02 section 8, statement 9 (focus), as far as the test writer reaches. */
test('statement 9, as far as the test writer reaches: focus after a show, a tab link and the end of a turn', async ({ page }) => {
  const site = await freshSite();
  const conversation = (await openConversation(page, site)).id;
  await settle(page, `/w/${site}/flow@main`);
  const tab = await browserTabOf(page);
  const turn = ulid();
  await write(page, conversation, message(turn, 'Show me the map.'));
  await write(page, conversation, started(turn, 1, 1, tab), tab);
  // An agent show leaves focus where it was: in the composer.
  await page.locator('#composer-input').focus();
  await write(page, conversation, toolStarted(turn, 'toolu_focus', 'panel.show', 'Opening the Map panel'));
  await write(page, conversation, show(turn, 'toolu_focus', `/w/${site}/map@main`, 'map', 'The agent opened the Map panel.'));
  await expect(page).toHaveURL(new RegExp(`/w/${site}/map@main$`));
  await expect(page.locator('#composer-input')).toBeFocused();
  // A tab link moves focus once, to the panel's title.
  await page.locator('#panel-tabs a', { hasText: 'Process flow' }).click();
  await expect(page.locator('#panel-title')).toBeFocused();
  // With focus on Stop when the turn ends, focus lands on the composer input, never body.
  await page.getByRole('button', { name: 'Stop' }).focus();
  await write(page, conversation, finished(turn, 'done', null));
  await expect(page.getByRole('button', { name: 'Stop' })).toHaveCount(0);
  await expect(page.locator('#composer-input')).toBeFocused();
});

/** Spec 02 section 8, statement 26 (button acts), its attribution half. */
test('statement 26, as far as the test writer reaches: an unattributable execution asks everyone, with buttons for the approver', async ({ page }) => {
  const site = await freshSite();
  const conversation = (await openConversation(page, site)).id;
  const ended = fixture('ended-turns');
  await replay(page, conversation, ended, { upTo: 12 });
  const approval = '01M588MFY0CS240YCSV4E4BFBK';
  const turn = '01M588KYBGZ6T51P6SJ2H2AA1B';
  await write(page, conversation, { entry: { seq: 0, kind: 'approval.decided', turn, caused_by: approval, body: { approval, decision: 'approve', author: { subject: 'dev', name: 'Developer' } } } });
  await write(page, conversation, { entry: { seq: 0, kind: 'approval.execution', turn, caused_by: approval, body: {
    approval, execution: 'unknown', sentence: 'Not yet known whether this change landed; checking again.',
    unattributable: { seqs: { [`${site}/flow`]: 10 } }, approver: { subject: 'dev', name: 'Developer' },
    draft: '01M588MEYR5EFE4P5Y8Q1PCWYC',
  } } });
  await settle(page, `/w/${site}/map@main`);
  const card = page.locator('.approval-card[data-state="approved"]');
  await expect(card).toContainText('A change matching this one landed as flow changeset 10, but Agent Studio cannot prove it was this one.');
  await expect(card.getByRole('button', { name: 'It was this change' })).toBeVisible();
  await expect(card.getByRole('button', { name: 'It was not' })).toBeVisible();
  await card.getByRole('button', { name: 'It was this change' }).click();
  await expect(card.locator('p.chat-card__error')).toContainText('Not sent: Agent Studio serves no route POST /api/approvals/');
});

/** Spec 02 section 8, statement 16 (reflow), for a reply's table in its TableScroll. */
test("statement 16 for a reply's table: at 320 CSS px a wide table stands in its own named, focusable region, and the page does not scroll sideways", async ({ page }) => {
  const site = await freshSite();
  const conversation = (await openConversation(page, site)).id;
  const turn = ulid();
  await write(page, conversation, message(turn, 'Compare the stations.'));
  await write(page, conversation, started(turn, 1, 1, null));
  const table = [
    '| Station | Fill pace | Admission cap | Mean wait | Longest wait | Picks per hour | Totes per hour |',
    '| --- | --- | --- | --- | --- | --- | --- |',
    '| Pick station 3 | 15 s | 6 | 21 s | 74 s | 412 | 388 |',
  ].join('\n');
  await write(page, conversation, text(turn, 'b1', `Here they are side by side.\n\n${table}`));
  await write(page, conversation, finished(turn, 'done', 'b1'));
  await page.setViewportSize({ width: 320, height: 640 });
  await settle(page, `/w/${site}/map@main`);
  const region = page.getByRole('region', { name: 'Station, Fill pace, Admission cap, Mean wait, Longest wait, Picks per hour, Totes per hour, table' });
  await expect(region).toBeVisible();
  await expect(region).toHaveAttribute('tabindex', '0');
  // The region scrolls sideways what cannot wrap; studio.css lets these cells wrap, so here it holds the table whole.
  expect(await region.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(0);
  expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(0);
});

/** What in the chat drawer runs past its right edge: nothing may, but what scrolls sideways in a region of its own. */
const pastTheDrawer = (page: Page) => page.evaluate(() => {
  const drawer = document.querySelector('.chat-drawer');
  if (drawer === null) return ['no drawer'];
  const edge = drawer.getBoundingClientRect().right;
  return [...drawer.querySelectorAll('*')]
    .filter((element) => element.closest('.table-scroll') === null && element.getBoundingClientRect().right > edge + 0.5)
    .map((element) => `${element.tagName.toLowerCase()}.${element.className} ${(element.textContent ?? '').slice(0, 40)}`);
});

/**
 * Ruling D192 (the UI fast-follow after PR 600): the chat drawer is resizable, keeps its width in this browser, never
 * takes more than half the window, its edge saying the width it shows as the window resizes, and its content wraps.
 */
test('the chat drawer resizes by key and by drag, follows the window, keeps its width across a reload, and nothing in it runs past its edge', async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1000 });
  const site = await freshSite();
  const conversation = (await openConversation(page, site)).id;
  await replay(page, conversation, fixture('site-conversation'));
  // A failed run whose reason is one long line, and a reply quoting a long reference, as the review site's chat held.
  const run = { run: ulid(), submission: ulid(), position: 1, of: 1, attempt: 1, sim_ms: 0 };
  const reason = 'solver failed: scheduler failed: task graph invariant violation [SchedulerInvariant::ReservationOverlap] robot_7 overlaps robot_3 at cell:12,4';
  await write(page, conversation, { entry: { seq: 0, kind: 'run.status', turn: null, caused_by: null, body: { ...run, state: 'queued', from: null } } });
  await write(page, conversation, { entry: { seq: 0, kind: 'run.status', turn: null, caused_by: null, body: { ...run, state: 'failed', from: 'queued', ended_by: 'simulator', stop_reason: reason, stop_detail: 'dessim::scheduler::invariants::check_reservations(window=00:41:12.300..00:41:13.900)' } } });
  const turn = ulid();
  const asked = await write(page, conversation, message(turn, 'Where is cell 1,10?'));
  await write(page, conversation, started(turn, 3, asked ?? 0, null));
  await write(page, conversation, text(turn, 'b9', `It is \`61f2767c-d529-472d-8f68-1cb8a510dc17/instance:0/local:floor/cell:1,10\`, at /w/${site}/map@main?select=61f2767c-d529-472d-8f68-1cb8a510dc17%2Finstance%3A0%2Flocal%3Afloor%2Fcell%3A1%2C10.`));
  await write(page, conversation, finished(turn, 'done', 'b9'));
  await settle(page, `/w/${site}/map@main`);
  await expect(page.locator('#chat-log')).toContainText('ReservationOverlap');

  const edge = page.getByRole('separator', { name: 'Width of the agent chat' });
  const width = () => page.locator('.chat-drawer').evaluate((element) => Math.round(element.getBoundingClientRect().width));
  await edge.focus();
  await page.keyboard.press('Home');
  await expect(edge).toHaveAttribute('aria-valuenow', '280');
  expect(await width()).toBe(280);
  expect(await pastTheDrawer(page)).toEqual([]);
  await page.keyboard.press('End');
  await expect(edge).toHaveAttribute('aria-valuenow', '800');
  expect(await pastTheDrawer(page)).toEqual([]);
  // A narrower window shows the drawer at half its width, and the edge says so; a wider one shows the width set again.
  await page.setViewportSize({ width: 1280, height: 1000 });
  await expect(edge).toHaveAttribute('aria-valuenow', '640');
  await expect(edge).toHaveAttribute('aria-valuemax', '640');
  expect(await width()).toBe(640);
  await page.setViewportSize({ width: 1920, height: 1000 });
  await expect(edge).toHaveAttribute('aria-valuenow', '800');
  expect(await width()).toBe(800);
  await page.keyboard.press('ArrowRight');
  await expect(edge).toHaveAttribute('aria-valuenow', '784');
  // Dragging the edge 200 px to the right narrows the drawer by as much.
  const box = await edge.boundingBox();
  if (box === null) throw new Error('the edge is not drawn');
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 200, box.y + box.height / 2, { steps: 5 });
  await page.mouse.up();
  expect(await width()).toBe(584);
  await page.reload();
  await chatDrawn(page);
  expect(await width()).toBe(584);
  expect(await pastTheDrawer(page)).toEqual([]);
});

/**
 * Ruling D192 (the UI fast-follow after PR 600): a folded chat hides its cards and draws nothing that can take focus.
 */
test('a folded chat names the approval that waits on its tab, and F6 and the skip link reach into it', async ({ page }) => {
  const site = await freshSite();
  await replay(page, (await openConversation(page, site)).id, fixture('ended-turns'), { upTo: 13 });
  await settle(page, `/w/${site}/map@main`);
  await page.getByRole('button', { name: 'Fold the agent chat away' }).click();
  const tab = page.getByRole('button', { name: 'Open the agent chat: an approval waits for you' });
  await expect(tab).toBeVisible();
  await expect(page.locator('#composer-input')).toBeHidden();
  // F6 from the panel visits the tab, in the place of the message box and the log a folded chat does not draw.
  await page.locator('#panel').focus();
  await page.keyboard.press('F6');
  await expect(tab).toBeFocused();
  // "Skip to the message box" opens the chat, then puts focus in its message box.
  await page.getByRole('link', { name: 'Skip to the message box' }).focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('#composer-input')).toBeFocused();
  await expect(page.locator('.shell')).not.toHaveAttribute('data-chat-folded', 'true');
  await expect(page.getByRole('button', { name: 'Fold the agent chat away' })).toBeVisible();
});
