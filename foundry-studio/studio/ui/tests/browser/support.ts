// Support for the browser tests of spec 02 section 8 and spec 03 section 8: each test opens a site of its own (a fresh
// conversation, numbered from 1), made in the stack's world-proto-server since the API opens conversations only for
// sites it lists, replays the journal fixtures of studio/protocol/tests/journal through the API's local test
// writer (brief I1.04, the path the agent loop takes), records what the announcer is handed before its nodes go, and,
// for spec 02 section 3.3's stream client, stands a proxy between the browser and the stack that can cut or hold the
// conversation's stream, cut it inside an event, turn its `stream.live` into a `stream.reset`, or cut a snapshot read.

import { readFileSync } from 'node:fs';
import { createServer, request, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import { connect } from 'node:net';
import { StringDecoder } from 'node:string_decoder';
import type { Page } from '@playwright/test';
import { ulid } from './turns.ts';

export interface Entry {
  seq: number;
  kind: string;
  turn: string | null;
  caused_by: string | null;
  body: Record<string, unknown>;
}

export type Line = { entry: Entry } | { live: { kind: string; body: unknown } };

export function fixture(name: 'site-conversation' | 'new-site-thread' | 'ended-turns'): Line[] {
  const text = readFileSync(new URL(`../../../../studio/protocol/tests/journal/${name}.jsonl`, import.meta.url), 'utf8');
  return text.split('\n').filter((line) => line.trim() !== '').map((line) => {
    const read = JSON.parse(line) as Entry | { live: { kind: string; body: unknown } };
    return 'live' in read ? { live: read.live } : { entry: read };
  });
}

/**
 * The fixtures' turn ids, which every replay shares. The API keys a turn by its id alone (migration 0018), so `write`
 * gives each conversation a turn of its own in place of each fixture turn, the same one every time it is named there.
 */
const FIXTURE_TURNS = new Set((['site-conversation', 'new-site-thread', 'ended-turns'] as const)
  .flatMap((name) => fixture(name))
  .flatMap((line) => ('entry' in line && line.entry.turn !== null ? [line.entry.turn] : [])));
const ownTurns = new Map<string, Map<string, string>>();

function ownTurn(conversation: string, turn: string): string {
  if (!FIXTURE_TURNS.has(turn)) return turn;
  const mine = ownTurns.get(conversation) ?? new Map<string, string>();
  ownTurns.set(conversation, mine);
  const own = mine.get(turn) ?? ulid();
  mine.set(turn, own);
  return own;
}

/** world-proto-server in the stack under test: `STUDIO_WPS_URL`, else dev.sh's `.dev-state/ports`. */
function wpsUrl(): string {
  if (process.env.STUDIO_WPS_URL) return process.env.STUDIO_WPS_URL;
  const ports = readFileSync(new URL('../../../.dev-state/ports', import.meta.url), 'utf8');
  const port = /^WPS_PORT=(\d+)$/m.exec(ports)?.[1];
  if (!port) throw new Error('No world-proto-server in the stack: start dev.sh without --no-wps, or set STUDIO_WPS_URL.');
  return `http://127.0.0.1:${port}`;
}

async function wpsPost(path: string, body: unknown, tolerated: number[] = []): Promise<void> {
  const answer = await fetch(`${wpsUrl()}${path}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  if (!answer.ok && !tolerated.includes(answer.status)) throw new Error(`world-proto-server answered ${answer.status} to ${path}: ${await answer.text()}`);
}

/**
 * A site no other test uses, so its conversation starts empty and numbers from 1. The API opens a conversation only for
 * a site world-proto-server lists (brief I1.06b), so the site is made there: under a customer of its own, so the list
 * each opening reads holds this site alone, the root a boundary whose one changeset states its prologue.
 */
export async function freshSite(): Promise<string> {
  const customer = `customer/t-${Math.random().toString(36).slice(2, 10)}`;
  const site = `${customer}/site/s`;
  await wpsPost('/scopes', { path: 'customer', boundary: false }, [409]);
  for (const path of [customer, `${customer}/site`]) await wpsPost('/scopes', { path, boundary: false });
  await wpsPost('/scopes', { path: site, boundary: true });
  await wpsPost(`/scopes/${site}/-/changesets`, {
    scope: site, base: 0, author: 'dev', message: 'A site of its own for one browser test', ops: [],
    prologue: { doc: crypto.randomUUID(), kind: 'world', name: `Test site ${customer.slice('customer/'.length)}` },
  });
  return site;
}

interface Stage { scope: string; ops: { id: string }[] }
const L_SHAPE = readFileSync(new URL('./fixtures/l-shape-6.json', import.meta.url), 'utf8');

/**
 * A made L-Shape at 6 aisles under a customer of its own, as Create site stages one from fixtures/l-shape-6.json, every
 * made id traded for a fresh one: its scopes made, then map, fleet, flow and the root each written at changeset 1.
 */
export async function madeLShape(): Promise<string> {
  const written = JSON.parse(L_SHAPE) as { prologue: { doc: string }; stages: Stage[] };
  let text = L_SHAPE;
  for (const id of [written.prologue.doc, ...written.stages.flatMap((stage) => stage.ops.map((op) => op.id))]) {
    text = text.replaceAll(id, crypto.randomUUID());
  }
  const fresh = JSON.parse(text) as { prologue: unknown; stages: Stage[] };
  const customer = `customer/f-${Math.random().toString(36).slice(2, 10)}`;
  const site = `${customer}/site/s`;
  await wpsPost('/scopes', { path: 'customer', boundary: false }, [409]);
  for (const path of [customer, `${customer}/site`]) await wpsPost('/scopes', { path, boundary: false });
  await wpsPost('/scopes', { path: site, boundary: true });
  for (const sub of ['map', 'fleet', 'flow']) await wpsPost('/scopes', { path: `${site}/${sub}`, boundary: false });
  for (const stage of fresh.stages) {
    const scope = stage.scope === '.' ? site : `${site}/${stage.scope}`;
    await wpsPost(`/scopes/${scope}/-/changesets`, {
      scope, base: 0, author: 'dev', message: 'A made L-Shape for one browser test', ops: stage.ops,
      ...(stage.scope === '.' ? { prologue: fresh.prologue } : {}),
    });
  }
  return site;
}

export async function openConversation(page: Page, site: string | null): Promise<{ id: string }> {
  const answer = await page.request.post('/api/conversations', { data: site === null ? {} : { site } });
  if (!answer.ok()) throw new Error(`opening the conversation answered ${answer.status()}: ${await answer.text()}`);
  return ((await answer.json()) as { conversation: { id: string } }).conversation;
}

/**
 * The person's new-site thread, emptied first, since a person has one: a thread an earlier test left with a turn open
 * gets that turn ended through the test writer, then is discarded, and the next one starts empty.
 */
export async function freshThread(page: Page): Promise<{ id: string }> {
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const thread = await openConversation(page, null);
    const snapshot = (await (await page.request.get(`/api/conversations/${thread.id}`)).json()) as {
      entries: unknown[]; active_turn: { id: string } | null;
    };
    if (snapshot.entries.length === 0) return thread;
    if (snapshot.active_turn !== null) {
      const usage = { model_calls: 0, input_tokens: 0, output_tokens: 0, cache_read_input_tokens: 0 };
      await write(page, thread.id, { entry: { seq: 0, kind: 'turn.finished', turn: snapshot.active_turn.id, caused_by: null, body: { outcome: 'stopped', usage } } });
    }
    await page.request.post(`/api/conversations/${thread.id}/dismiss`, { data: {} });
  }
  throw new Error('the new-site thread would not empty');
}

/** Hand one fixture line to the test writer; `tab` stands in for the browser tab a `turn.started` records. */
export async function write(page: Page, conversation: string, line: Line, tab?: string): Promise<number | null> {
  let sent: unknown;
  if ('live' in line) {
    sent = { live: line.live };
  } else {
    const entry = line.entry;
    const body = entry.kind === 'turn.started' && tab !== undefined ? { ...entry.body, browser_tab: tab } : entry.body;
    const turn = entry.turn ? ownTurn(conversation, entry.turn) : null;
    sent = { kind: entry.kind, body, ...(turn ? { turn } : {}), ...(entry.caused_by ? { caused_by: entry.caused_by } : {}) };
  }
  const answer = await page.request.post(`/api/dev/conversations/${conversation}/entries`, { data: sent });
  if (answer.status() >= 300) throw new Error(`the test writer refused it (${answer.status()}): ${await answer.text()}`);
  if (answer.status() === 202) return null;
  return ((await answer.json()) as { entry?: { seq: number } }).entry?.seq ?? null;
}

/** Replay `lines` in order, those whose fixture `seq` is within `[from, upTo]` (live items ride with their place). */
export async function replay(page: Page, conversation: string, lines: Line[], options: { from?: number; upTo?: number; tab?: string } = {}) {
  let last = 0;
  for (const line of lines) {
    const seq = 'entry' in line ? line.entry.seq : last + 0.5;
    if ('entry' in line) last = line.entry.seq;
    if (seq < (options.from ?? 0) || seq > (options.upTo ?? Infinity)) continue;
    await write(page, conversation, line, options.tab);
  }
}

/** Record every sentence handed to `#announcer`, since each node leaves 7 s after it is added. */
export async function listen(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const spoken: string[] = [];
    (window as unknown as { __spoken: string[] }).__spoken = spoken;
    new MutationObserver((changes) => {
      for (const change of changes) {
        if ((change.target as Element).id !== 'announcer') continue;
        change.addedNodes.forEach((node) => spoken.push(node.textContent ?? ''));
      }
    }).observe(document, { childList: true, subtree: true });
  });
}

export const spoken = (page: Page): Promise<string[]> => page.evaluate(() => (window as unknown as { __spoken: string[] }).__spoken);

export const browserTabOf = (page: Page): Promise<string> =>
  page.evaluate(() => sessionStorage.getItem('agent-studio.browser-tab') ?? '');

/** Wait until the chat has read the conversation and drawn its log and composer. */
export async function chatDrawn(page: Page): Promise<void> {
  await page.locator('#composer').waitFor();
  await page.locator('#chat-log > li:not([data-kind="status"])').first().waitFor();
}

export interface StreamProxy {
  url: string;
  /** Destroy every stream connection now, as a network failure. */
  drop(): void;
  /** While on, a new stream request fails at the socket, so the browser keeps retrying by itself. */
  refuse(on: boolean): void;
  /** Hold the streams open now: they stay connected but nothing more reaches the browser; later ones flow. */
  hold(on: boolean): void;
  /** Cut the stream inside the next event of `kind`: half of it reaches the browser, then the connection drops. */
  cutInside(kind: string): void;
  /** On the next stream, its `stream.live` reaches the browser as a `stream.reset`, so the reset comes in catch-up. */
  resetInsteadOfLive(): void;
  /** Cut the next snapshot read (`GET /api/conversations/{c}`) at the socket, as a network failure. */
  cutNextSnapshot(): void;
  /** The cuts and rewrites asked for that have not happened yet, so a test can tell that each did. */
  armed(): string[];
  /** How many stream requests arrived, and when each did (ms since the epoch). */
  requests: number[];
  close(): Promise<void>;
}

const STREAM = /^\/api\/conversations\/[^/]+\/stream/;
const SNAPSHOT = /^\/api\/conversations\/[^/?]+$/;

/** An event block naming `kind` (`event: {kind}` on a line of its own). */
const isEvent = (block: string, kind: string): boolean => block.split('\n').includes(`event: ${kind}`);

/** A proxy in front of `target` (the Vite dev server) that can cut or hold the conversation streams. */
export async function streamProxy(target: string): Promise<StreamProxy> {
  const to = new URL(target);
  const streams = new Set<ServerResponse>();
  const held = new Set<ServerResponse>();
  let refusing = false;
  let cutKind: string | null = null;
  let resetLive = false;
  let cutSnapshot = false;
  const requests: number[] = [];
  const server: Server = createServer((incoming: IncomingMessage, outgoing: ServerResponse) => {
    const isStream = STREAM.test(incoming.url ?? '');
    if (incoming.method === 'GET' && SNAPSHOT.test(incoming.url ?? '') && cutSnapshot) {
      cutSnapshot = false;
      incoming.socket.destroy();
      return;
    }
    if (isStream) {
      requests.push(Date.now());
      if (refusing) {
        incoming.socket.destroy();
        return;
      }
    }
    const rewriteLive = isStream && resetLive;
    if (rewriteLive) resetLive = false;
    const forward = request({
      host: to.hostname, port: to.port, method: incoming.method, path: incoming.url,
      headers: { ...incoming.headers, host: to.host },
    }, (answer) => {
      outgoing.writeHead(answer.statusCode ?? 502, answer.headers);
      if (!isStream) {
        answer.pipe(outgoing);
        return;
      }
      streams.add(outgoing);
      // Whole events only, so one can be cut inside or rewritten; the browser dispatches nothing before an event ends.
      const decoder = new StringDecoder('utf8');
      let pending = '';
      let cut = false;
      answer.on('data', (chunk: Buffer) => {
        if (held.has(outgoing) || cut) return;
        pending += decoder.write(chunk);
        const end = pending.lastIndexOf('\n\n');
        if (end < 0) return;
        const blocks = pending.slice(0, end).split('\n\n');
        pending = pending.slice(end + 2);
        for (const block of blocks) {
          if (cutKind !== null && isEvent(block, cutKind)) {
            cutKind = null;
            cut = true;
            outgoing.write(block.slice(0, Math.floor(block.length / 2)));
            outgoing.socket?.destroy();
            return;
          }
          const sent = rewriteLive && isEvent(block, 'stream.live') ? 'event: stream.reset\ndata: {"reason":"slow-client"}' : block;
          outgoing.write(`${sent}\n\n`);
        }
      });
      answer.on('end', () => outgoing.end());
      outgoing.on('close', () => {
        streams.delete(outgoing);
        held.delete(outgoing);
        answer.destroy();
      });
    });
    forward.on('error', () => outgoing.destroy());
    incoming.pipe(forward);
  });
  const upgraded = new Set<{ destroy(): void }>();
  server.on('upgrade', (incoming: IncomingMessage, socket, head) => {
    upgraded.add(socket);
    const upstream = connect(Number(to.port), to.hostname, () => {
      const lines = [`${incoming.method} ${incoming.url} HTTP/1.1`, ...Object.entries(incoming.headers).map(([name, value]) => `${name}: ${String(value)}`)];
      upstream.write(`${lines.join('\r\n')}\r\n\r\n`);
      upstream.write(head);
      upstream.pipe(socket);
      socket.pipe(upstream);
    });
    upgraded.add(upstream);
    upstream.on('error', () => socket.destroy());
    socket.on('error', () => upstream.destroy());
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  const port = typeof address === 'object' && address !== null ? address.port : 0;
  return {
    url: `http://127.0.0.1:${port}`,
    drop: () => streams.forEach((stream) => stream.socket?.destroy()),
    refuse: (on) => {
      refusing = on;
    },
    hold: (on) => {
      if (on) streams.forEach((stream) => held.add(stream));
      else held.clear();
    },
    cutInside: (kind) => {
      cutKind = kind;
    },
    resetInsteadOfLive: () => {
      resetLive = true;
    },
    cutNextSnapshot: () => {
      cutSnapshot = true;
    },
    armed: () => [
      ...(cutKind !== null ? [`cut inside ${cutKind}`] : []), ...(resetLive ? ['reset instead of live'] : []),
      ...(cutSnapshot ? ['cut snapshot'] : []),
    ],
    requests,
    close: () => new Promise<void>((resolve) => {
      streams.forEach((stream) => stream.socket?.destroy());
      upgraded.forEach((socket) => socket.destroy());
      server.closeAllConnections();
      server.close(() => resolve());
    }),
  };
}
