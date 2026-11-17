// The agent loop on a live model in the browser (brief I1.03, "Done when"; spec 01 sections 3.4, 3.9 and 3.10): a
// new-site turn on Gemini whose journal's first tool is its question, which resumes on the answer to Create site's card,
// is approved, binds the thread and ends `done` with its reply announced as spec 02's reply rule reads it, axe finding
// nothing while either card waits; and a turn whose reply is streaming that Stop ends `stopped` within 1 s of wall
// clock, both ends measured on this test's clock; and the templates question the lead asked on 2026-10-20, whose chat
// shows no thought summary and no missing chip. Without the key file, or with the stack's agent on another provider,
// the specs skip and say why: the live turn is then unverified. The key file is only checked for, never read.

import AxeBuilder from '@axe-core/playwright';
import { existsSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { replyToSpeak, spokenSentences } from '../../src/chat/sentences.ts';
import type { Snapshot } from '../../src/protocol/snapshot.ts';
import type { ResolvedRef } from '../../src/protocol/turn.ts';
import { freshSite, freshThread, listen, openConversation, spoken } from './support.ts';

const KEY_FILE = process.env.STUDIO_GEMINI_API_KEY_FILE ?? join(homedir(), '.config/agent-studio/secrets/google-api-keys.txt');

test.skip(!existsSync(KEY_FILE), `No Gemini key file at ${KEY_FILE}, so no live turn runs: the live Gemini turn is unverified.`);

test.beforeEach(async ({ request }) => {
  const me = (await (await request.get('/api/me')).json()) as { agent?: { provider: string } };
  test.skip(me.agent?.provider !== 'gemini', `The stack's agent runs on ${me.agent?.provider ?? 'no provider'}, not Gemini.`);
});

const snapshot = async (page: Page, conversation: string): Promise<Snapshot> =>
  (await (await page.request.get(`/api/conversations/${conversation}`)).json()) as Snapshot;

/** Axe in the dark and the light scheme, the page left light. */
async function noViolation(page: Page, what: string) {
  for (const scheme of ['dark', 'light'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    const results = await new AxeBuilder({ page }).analyze();
    const found = results.violations.map((violation) => `${violation.id}: ${violation.nodes.map((node) => node.target.join(' ')).slice(0, 3).join(' | ')}`);
    expect(found, `${what}, ${scheme}`).toEqual([]);
  }
}

async function send(page: Page, words: string) {
  await page.locator('#composer-input').fill(words);
  await page.locator('#composer').getByRole('button', { name: 'Send', exact: true }).click();
}

/** Brief I1.03, "Done when", the live turn on Gemini; spec 01 sections 3.4 and 3.10, rule 2 (the card's approval). */
test("a new-site turn asked to put a question first is answered, resumes to Create site's card, is approved, binds the thread and ends done with its reply spoken", async ({ page }) => {
  test.setTimeout(300_000);
  await listen(page);
  const thread = (await freshThread(page)).id;
  const slug = `agent-${Date.now().toString(36)}`;
  await page.goto('/templates');
  await page.locator('#composer').waitFor();
  await send(page, [
    'Before anything else, ask me one question with the ask_user tool: which template the new site starts from, with',
    'exactly two options, "Square" and "L-Shape". Once I answer, start a draft from the template I chose',
    '(template/site/square@main or template/site/l-shape@main) with template_start_draft, change no knob, and call',
    `site_create_from_template for its evaluation with customer "demo", slug "${slug}" and name "Agent ${slug}".`,
    'Once the site is made, tell me so in one sentence.',
  ].join(' '));

  const question = page.locator('form.question-card');
  await expect(question).toBeVisible({ timeout: 120_000 });
  await noViolation(page, 'a question card pending');
  const items = question.locator('fieldset.question-card__item');
  for (let n = 0; n < (await items.count()); n += 1) {
    const square = items.nth(n).locator('input[value*="square" i]');
    await ((await square.count()) > 0 ? square.first() : items.nth(n).locator('input').first()).check();
  }
  await question.getByRole('button', { name: 'Send', exact: true }).click();

  const card = page.locator('.approval-card[data-state="pending"]');
  await expect(card).toBeVisible({ timeout: 180_000 });
  await noViolation(page, 'an approval card pending');
  await card.getByRole('button', { name: 'Create site', exact: true }).click();

  await expect.poll(async () => (await snapshot(page, thread)).active_turn, { timeout: 120_000 }).toBeNull();
  const ended = await snapshot(page, thread);
  expect(ended.conversation.site).toBe(`customer/demo/site/${slug}`);
  const finished = ended.entries.filter((entry) => entry.kind === 'turn.finished').at(-1);
  const body = finished?.body as { outcome: string; reply_block: string | null } | undefined;
  expect(body?.outcome).toBe('done');
  const first = ended.entries.find((entry) => entry.kind === 'tool.started' && entry.turn === finished?.turn);
  expect((first?.body as { tool?: string } | undefined)?.tool, "the turn's first tool").toBe('ask_user');
  const reply = ended.entries.map((entry) => entry.body as { block?: string; text?: string; refs?: ResolvedRef[] })
    .find((done, n) => ended.entries[n].kind === 'text.done' && done.block === body?.reply_block);
  const expected = replyToSpeak(spokenSentences(reply?.text ?? '', reply?.refs ?? []), 'd70') ?? '';
  expect(expected, 'the reply as the announcer reads it').not.toBe('');
  await expect.poll(() => spoken(page)).toContain(expected);
});

/**
 * Brief chore-chat-fixes, "Done when": the question whose reply showed Gemini's thought summaries as note entries and
 * every template it named as a missing chip (D117; D60). The journal holds the reply's words alone, the chat draws
 * exactly those, no note and no missing chip, a template the reply links leads to its panel route, and no knob is
 * named by its key (`cut_width`) rather than its label.
 */
test('the templates question draws no thought summary and no missing chip', async ({ page }) => {
  test.setTimeout(240_000);
  const thread = (await freshThread(page)).id;
  await page.goto('/templates');
  await page.locator('#composer').waitFor();
  await send(page, 'Which templates can I start a site from, and how do they differ?');
  const finishedYet = async () => (await snapshot(page, thread)).entries.some((entry) => entry.kind === 'turn.finished');
  await expect.poll(finishedYet, { timeout: 200_000 }).toBe(true);
  const ended = await snapshot(page, thread);
  const finished = ended.entries.filter((entry) => entry.kind === 'turn.finished').at(-1);
  expect((finished?.body as { outcome?: string } | undefined)?.outcome).toBe('done');
  const blocks = ended.entries.filter((entry) => entry.kind === 'text.done').map((entry) => entry.body as { kind: string; text: string });
  expect(blocks.map((block) => block.kind), 'every journaled block is the agent\'s words').toEqual(blocks.map(() => 'text'));
  const log = page.locator('#chat-log');
  await expect(log.locator('li.entry--agent')).toHaveCount(blocks.length);
  await expect(log.locator('li.entry--note, li[data-kind="note"]')).toHaveCount(0);
  await expect(log.locator('.entity-chip[data-state="missing"]')).toHaveCount(0);
  for (const link of await log.locator('li.entry--agent a[href^="/w/template/"]').all()) {
    expect(await link.getAttribute('href')).toMatch(/^\/w\/template\/site\/[a-z0-9-]+@main$/);
  }
  const listed = (await (await page.request.get('/api/templates')).json()) as { items: { knobs: { key: string }[] }[] };
  const keys = new Set(listed.items.flatMap((card) => card.knobs.map((knob) => knob.key)).filter((key) => key.includes('_')));
  const said = blocks.map((block) => block.text).join('\n');
  expect([...keys].filter((key) => said.includes(key)), 'knobs named by their keys').toEqual([]);
});

/** Brief I1.03, "Done when": Stop ends a turn whose reply is streaming `stopped` within 1 s of wall clock, from the
 * press to the service reading the turn as ended, both on this test's clock (spec 01 section 3.14). */
test('a turn stopped while its reply streams ends stopped within 1 s of wall clock', async ({ page }) => {
  test.setTimeout(120_000);
  await listen(page);
  const site = await freshSite();
  const conversation = (await openConversation(page, site)).id;
  await page.goto(`/w/${site}/map@main`);
  await page.locator('#composer').waitFor();
  await send(page, 'Without calling any tool, write an essay of at least two thousand words on how warehouses lay out their aisles.');
  await page.locator('li.entry--agent[data-kind="text"] > .entry__body[aria-busy="true"]').first().waitFor({ timeout: 90_000 });
  const stop = page.locator('#composer').getByRole('button', { name: 'Stop', exact: true });
  const pressed = Date.now();
  await stop.click();
  await expect.poll(async () => (await snapshot(page, conversation)).active_turn, { timeout: 5_000, intervals: [25] }).toBeNull();
  const elapsed = Date.now() - pressed;
  const finished = (await snapshot(page, conversation)).entries.filter((entry) => entry.kind === 'turn.finished').at(-1);
  expect((finished?.body as { outcome?: string } | undefined)?.outcome).toBe('stopped');
  expect(elapsed, 'from the press to the turn read as ended').toBeLessThanOrEqual(1_000);
  await expect.poll(() => spoken(page)).toContain('The agent stopped.');
});
