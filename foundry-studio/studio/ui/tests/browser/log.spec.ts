// Spec 02 in the browser, the log: statement 25 against the API's own projection (a window of the API's entries and
// projection draws a turn as the API's full snapshot does), and section 3.5's F8 ("Show earlier entries" keeps focus
// on its button while earlier entries remain and moves it to the log once the first entry is held, the entries read
// back above holding still what was on screen; and the log follows a streaming reply only while read at its end).

import { expect, test, type Page } from '@playwright/test';
import { browserTabOf, chatDrawn, fixture, freshSite, openConversation, replay, write } from './support.ts';
import { message, started, ulid } from './turns.ts';

/** `count` system notes, written ten at a time; their order in the journal does not matter here. */
async function notes(page: Page, conversation: string, count: number) {
  for (let from = 1; from <= count; from += 10) {
    const batch = Array.from({ length: Math.min(10, count - from + 1) }, (_, n) => from + n);
    await Promise.all(batch.map((n) => write(page, conversation, {
      entry: { seq: 0, kind: 'system.note', turn: null, caused_by: null, body: { text: `Note ${n} of this test.` } },
    })));
  }
}

async function open(page: Page, site: string) {
  await page.goto(`/w/${site}/flow@main`);
  await page.locator('#panel-title').waitFor();
  await chatDrawn(page);
}

/** Spec 02 section 3.5, F8: paging back moves focus to the log only when its button gives way. */
test('F8: "Show earlier entries" keeps focus while more remain, then gives way to the start and focuses the log', async ({ page }) => {
  test.setTimeout(120_000);
  const site = await freshSite();
  const conversation = (await openConversation(page, site)).id;
  await notes(page, conversation, 430);
  await open(page, site);
  const entries = page.locator('#chat-log > li.entry');
  await expect(entries).toHaveCount(200);
  const button = page.getByRole('button', { name: 'Show earlier entries' });
  await button.scrollIntoViewIfNeeded();
  const firstText = await entries.first().textContent();
  const before = await entries.first().boundingBox();
  await button.click();
  await expect(entries).toHaveCount(400);
  await expect(button).toBeFocused();
  // The entry that was first stands where it stood: what was read back went above it.
  const after = await entries.filter({ hasText: firstText ?? '' }).first().boundingBox();
  expect(Math.abs((after?.y ?? 0) - (before?.y ?? 0))).toBeLessThanOrEqual(1);
  await button.click();
  await expect(entries).toHaveCount(430);
  await expect(page.locator('#chat-log > li.chat__earlier')).toHaveText('Start of the conversation');
  await expect(page.locator('#chat-log')).toBeFocused();
});

/** Spec 02 section 3.5, F8: the log follows a streaming reply only while read at its end. */
test('F8: the log follows a streaming reply while read at its end, and holds still while read above it', async ({ page }) => {
  const site = await freshSite();
  const conversation = (await openConversation(page, site)).id;
  await notes(page, conversation, 40);
  await open(page, site);
  const log = page.locator('#chat-log');
  const belowView = () => log.evaluate((element) => element.scrollHeight - element.scrollTop - element.clientHeight);
  await expect.poll(belowView).toBeLessThanOrEqual(8);
  const tab = await browserTabOf(page);
  const turn = ulid();
  await write(page, conversation, message(turn, 'Tell me about station 3.'));
  await write(page, conversation, started(turn, 1, 41, tab), tab);
  const streaming = page.locator('li.entry--agent .entry__body[aria-busy="true"]');
  let text = '';
  const stream = async (from: number, to: number) => {
    for (let n = from; n < to; n += 1) {
      const delta = `Station 3 waits on totes, line ${n}. `.repeat(6);
      await write(page, conversation, { live: { kind: 'text.delta', body: { block: 'b1', offset: text.length, delta } } });
      text += delta;
    }
  };
  await stream(0, 6);
  await expect(streaming).toContainText('line 5');
  await expect.poll(belowView).toBeLessThanOrEqual(8);
  await log.evaluate((element) => {
    element.scrollTop = 0;
  });
  await page.waitForTimeout(200);
  await stream(6, 12);
  await expect(streaming).toContainText('line 11');
  expect(await log.evaluate((element) => element.scrollTop)).toBe(0);
});

/**
 * Spec 02 section 8, statement 25 (projection), against the API: the site fixture replayed through the test writer,
 * then drawn twice, from the API's snapshot of every entry and from the API's page of the entries after 24 with the
 * projection it sends for them; turn 2, begun before that window, draws its heading, rows and cards alike.
 */
test("statement 25 against the API: a window of the API's entries and projection draws turn 2 as its full snapshot does", async ({ page }) => {
  const site = await freshSite();
  const conversation = (await openConversation(page, site)).id;
  await replay(page, conversation, fixture('site-conversation'));
  const full = await (await page.request.get(`/api/conversations/${conversation}`)).json() as Record<string, unknown>;
  const window = await (await page.request.get(`/api/conversations/${conversation}/entries?after=24&limit=200`)).json() as { entries: { seq: number }[]; projection: unknown };
  expect(window.entries[0]?.seq).toBe(25);
  const turnTwo = async () => {
    const turn = page.locator('li.turn[aria-labelledby="turn-2"]');
    await turn.waitFor();
    return turn.evaluate((element) => ({
      header: element.querySelector('h3.turn__header')?.outerHTML,
      rows: element.querySelector('ul.tool-calls__list')?.outerHTML,
      cards: [...element.querySelectorAll('.chat-card')].map((card) => card.outerHTML),
    }));
  };
  await open(page, site);
  const whole = await turnTwo();
  expect(whole.cards).toHaveLength(4);
  await page.route(`**/api/conversations/${conversation}`, (route) => route.fulfill({
    json: { ...full, entries: window.entries, projection: window.projection },
  }));
  await open(page, site);
  await expect(page.getByRole('button', { name: 'Show earlier entries' })).toBeVisible();
  expect(await turnTwo()).toEqual(whole);
});
