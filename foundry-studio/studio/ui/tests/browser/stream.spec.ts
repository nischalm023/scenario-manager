// Spec 02 section 8 in the browser, the stream through a proxy that can cut or hold it: statement 6 (a stream cut for
// 30 s mid-turn, which the browser reconnects by itself, draws no entry twice, navigates on no replayed command, and
// speaks the catch-up set once, at stream.live; then a stream cut inside an event, and a `stream.reset` that comes in
// catch-up with its snapshot read cut, section 3.3's steps 4, 6 and 8) and statement 7 (pings stopped with the socket
// held open: the client reconnects within 50 s).

import { expect, test } from '@playwright/test';
import { browserTabOf, chatDrawn, freshSite, listen, openConversation, spoken, streamProxy, write } from './support.ts';
import { finished, message, note, show, started, text, toolFinished, toolStarted, ulid } from './turns.ts';

/** Spec 02 section 8, statement 6 (catch-up). */
test('statement 6: a 30 s cut mid-turn draws no entry twice, navigates nowhere, and speaks the catch-up set once', async ({ page, baseURL }) => {
  test.setTimeout(120_000);
  const proxy = await streamProxy(baseURL ?? '');
  try {
    await listen(page);
    const site = await freshSite();
    const conversation = (await openConversation(page, site)).id;
    await page.goto(`${proxy.url}/w/${site}/flow@main`);
    await chatDrawn(page);
    const tab = await browserTabOf(page);
    const turn = ulid();
    await write(page, conversation, message(turn, 'Why is station 3 slow?'));
    await write(page, conversation, started(turn, 1, 1, tab), tab);
    await expect(page.locator('li.turn')).toHaveCount(1);

    proxy.refuse(true);
    proxy.drop();
    await expect(page.locator('.chat__connection')).toHaveText('Reconnecting to the agent', { timeout: 10_000 });
    await write(page, conversation, toolStarted(turn, 'toolu_cut', 'flow.read', 'Reading the flow'));
    await write(page, conversation, toolFinished(turn, 'toolu_cut', 'failed', 'Reading the flow failed: the world graph server did not answer'));
    await write(page, conversation, toolStarted(turn, 'toolu_show', 'panel.show', 'Opening the Map panel'));
    await write(page, conversation, show(turn, 'toolu_show', `/w/${site}/map@main`, 'map', 'The agent opened the Map panel.'));
    await write(page, conversation, text(turn, 'b1', 'Station 3 waits on totes. Its fill pace sets the line.'));
    await write(page, conversation, finished(turn, 'done', 'b1'));
    await page.waitForTimeout(30_000);
    expect(await spoken(page)).toContain('Lost the connection to the agent. Reconnecting.');
    proxy.refuse(false);

    await expect(page.locator('.chat__connection')).toHaveCount(0, { timeout: 15_000 });
    await expect(page.locator('li.turn[data-state="done"]')).toHaveCount(1);
    await expect(page.locator('li.entry--user')).toHaveCount(1);
    await expect(page.locator('li.tool-row')).toHaveCount(2);
    await expect(page.locator('li.entry--agent[data-kind="text"]')).toHaveCount(1);
    await page.waitForTimeout(1000);
    expect(page.url()).toBe(`${proxy.url}/w/${site}/flow@main`);
    const heard = await spoken(page);
    // A5 is one sentence, never the row's words (D117).
    expect(heard.filter((sentence) => sentence === 'A tool call failed.')).toHaveLength(1);
    expect(heard.filter((sentence) => sentence === 'Station 3 waits on totes. Its fill pace sets the line.')).toHaveLength(1);
    expect(heard.filter((sentence) => sentence === 'The agent opened the Map panel.')).toHaveLength(0);
    expect(heard).toContain('Reconnected.');
  } finally {
    await proxy.close();
  }
});

/** Spec 02 section 8, statement 7 (stall). */
test('statement 7: with pings stopped and the socket held open, the client reconnects within 50 s', async ({ page, baseURL }) => {
  test.setTimeout(120_000);
  const proxy = await streamProxy(baseURL ?? '');
  try {
    const site = await freshSite();
    await openConversation(page, site);
    await page.goto(`${proxy.url}/w/${site}/flow@main`);
    await chatDrawn(page);
    await expect.poll(() => proxy.requests.length).toBe(1);
    // Held only once its answer flows: the connection words go at its first event, so the proxy holds this stream.
    await expect(page.locator('.chat__connection')).toHaveCount(0);
    const held = Date.now();
    proxy.hold(true);
    await expect.poll(() => proxy.requests.length, { timeout: 55_000, intervals: [1000] }).toBe(2);
    expect(proxy.requests[1] - held).toBeLessThanOrEqual(50_000);
    await expect(page.locator('.chat__connection')).toHaveCount(0, { timeout: 10_000 });
  } finally {
    await proxy.close();
  }
});

/**
 * Spec 02 section 8, statement 6 (catch-up), and section 3.3, step 6: a stream cut inside an event dispatches none of
 * it; the browser's own reconnect replays it whole, so it is drawn once and its A9 is spoken once, at stream.live.
 */
test('statement 6, cut inside an event: the replay draws it once, whole, and speaks it once', async ({ page, baseURL }) => {
  const proxy = await streamProxy(baseURL ?? '');
  try {
    await listen(page);
    const site = await freshSite();
    const conversation = (await openConversation(page, site)).id;
    await page.goto(`${proxy.url}/w/${site}/flow@main`);
    await chatDrawn(page);
    await expect(page.locator('.chat__connection')).toHaveCount(0);
    proxy.cutInside('system.note');
    await write(page, conversation, note(null, 'A note cut in two on its way.'));
    const notes = page.locator('#chat-log > li[data-kind="system.note"]');
    await expect(notes).toHaveCount(1, { timeout: 15_000 });
    await expect.poll(() => proxy.requests.length).toBe(2);
    await write(page, conversation, note(null, 'A note after the cut.'));
    await expect(notes).toHaveCount(2);
    await expect(notes.first()).toContainText('A note cut in two on its way.');
    expect(proxy.armed()).toEqual([]);
    await expect.poll(async () => (await spoken(page)).filter((sentence) => sentence === 'A note after the cut.')).toHaveLength(1);
    expect((await spoken(page)).filter((sentence) => sentence === 'A note cut in two on its way.')).toHaveLength(1);
  } finally {
    await proxy.close();
  }
});

/**
 * Spec 02 section 8, statement 6 (catch-up), and section 3.3, steps 4 and 8: a `stream.reset` that comes before the
 * replay's `stream.live`, whose snapshot read is then cut, is retried with backoff; nothing is drawn twice, and what the
 * catch-up collected (a failed call's A5, the failed turn's A9) is spoken once, at the next connection's stream.live.
 */
test('statement 6, a reset in catch-up with its snapshot read cut: retried, drawn once, the catch-up set spoken once', async ({ page, baseURL }) => {
  test.setTimeout(120_000);
  const proxy = await streamProxy(baseURL ?? '');
  try {
    await listen(page);
    const site = await freshSite();
    const conversation = (await openConversation(page, site)).id;
    await page.goto(`${proxy.url}/w/${site}/flow@main`);
    await chatDrawn(page);
    const turn = ulid();
    await write(page, conversation, message(turn, 'Read the flow.'));
    await write(page, conversation, started(turn, 1, 1, null));
    await expect(page.locator('li.turn')).toHaveCount(1);
    proxy.refuse(true);
    proxy.drop();
    await write(page, conversation, toolStarted(turn, 'toolu_reset', 'flow.read', 'Reading the flow'));
    await write(page, conversation, toolFinished(turn, 'toolu_reset', 'failed', 'Reading the flow failed: the world graph server did not answer'));
    await write(page, conversation, note(turn, 'The model stayed unavailable, so the turn failed.'));
    await write(page, conversation, finished(turn, 'failed', null));
    proxy.resetInsteadOfLive();
    proxy.cutNextSnapshot();
    proxy.refuse(false);
    await expect(page.locator('li.turn[data-state="failed"]')).toHaveCount(1, { timeout: 30_000 });
    await expect(page.locator('.chat__connection')).toHaveCount(0, { timeout: 30_000 });
    await expect(page.locator('li.tool-row')).toHaveCount(1);
    await expect(page.locator('li.entry--user')).toHaveCount(1);
    await expect.poll(async () => (await spoken(page)).filter((sentence) => sentence === 'The model stayed unavailable, so the turn failed.')).toHaveLength(1);
    expect((await spoken(page)).filter((sentence) => sentence === 'A tool call failed.')).toHaveLength(1);
    expect(proxy.armed()).toEqual([]);
  } finally {
    await proxy.close();
  }
});
