// Journal lines the browser tests write for a turn of their own, on their own site: a person's message and the
// turn it starts, tool rows, a panel command whose route names the test's site, text, and the turn's end. The bodies
// are spec 01 section 3.5's, as the journal fixtures write them.

import type { Line } from './support.ts';

const CROCKFORD = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';

export function ulid(): string {
  return `01M6${Array.from({ length: 22 }, () => CROCKFORD[Math.floor(Math.random() * 32)]).join('')}`;
}

const DEV = { subject: 'dev', name: 'Developer' };

const entry = (kind: string, turn: string | null, body: Record<string, unknown>, causedBy: string | null = null): Line =>
  ({ entry: { seq: 0, kind, turn, caused_by: causedBy, body } });

export function message(turn: string, text: string): Line {
  return entry('message.user', turn, {
    author: DEV, text, context: null, source: 'composer', browser_tab: null, client_message_id: crypto.randomUUID(),
  });
}

export function started(turn: string, n: number, messageSeq: number, tab: string | null): Line {
  return entry('turn.started', turn, { n, author: DEV, message_seq: messageSeq, attended: true, browser_tab: tab });
}

export const toolStarted = (turn: string, call: string, tool: string, summary: string): Line =>
  entry('tool.started', turn, { call, tool, class: 'read', summary }, call);

export const toolFinished = (turn: string, call: string, outcome: string, summary: string): Line =>
  entry('tool.finished', turn, { call, outcome, summary, refs: [] }, call);

export function show(turn: string, call: string, route: string, panel: 'map' | 'flow', announce: string): Line {
  return entry('panel.command', turn, {
    action: 'show', panel, location: route.replace(/^\/w\//, '').replace(/\?.*$/, ''), resolved: [], select: null, layers: null,
    draft: null, sim_ms: null, compare: null, metrics: null, highlight: null, route, as_it_was: null, announce,
  }, call);
}

export const text = (turn: string, block: string, words: string): Line =>
  entry('text.done', turn, { block, kind: 'text', text: words, refs: [] });

export const finished = (turn: string, outcome: string, replyBlock: string | null): Line =>
  entry('turn.finished', turn, { outcome, reply_block: replyBlock, usage: { model_calls: 1, input_tokens: 1, output_tokens: 1, cache_read_input_tokens: 0 } });

export const note = (turn: string | null, words: string): Line => entry('system.note', turn, { text: words });
