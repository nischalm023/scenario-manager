// Where the wall clock goes between a knob change in the MapTemplate panel and its redrawn floor (the evaluation-latency
// chore): a measurement, not a test of the suite. On each shell template it starts a draft, changes the Aisles knob
// five times, letting the 800 ms pause end each change as a person's typing does, and reads the steps of each change
// from the browser's performance marks (`template:` in the panel, `floor:` in the mount) and the API's one timing
// line per evaluation from its log; `uptime`'s load averages are printed beside every round.
//
//   node tests/measure/eval-latency.ts [--rounds 2] [--templates Square,I-Shape] [--out results.json] [--shell]
//
// It writes into the stack it measures (a draft per template, the person's new-site thread emptied), so it measures
// only the stack dev.sh started from this worktree, and takes no address: its UI and API ports come from this
// worktree's studio/.dev-state, each believed only while the process listening on it is the one dev.sh recorded there,
// and its timing lines from studio/.dev-log/api.log. The browser is Playwright's full Chromium in its new headless
// mode, which draws through the GPU as a person's browser does; `--shell` takes the headless shell the suite runs
// instead, which rasterises WebGL in software on the page's own thread (five frames a second on this machine), so every
// step there waits on drawing. The page keeps its marks because the script sets MEASURING before it loads, and the
// script clears them once read.

import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { chromium, devices, type Browser } from '@playwright/test';
import { MEASURING } from '../../src/shell/marks.ts';
import { freshThread } from '../browser/support.ts';

const SHELLS = ['Square', 'Rectangle', 'L-Shape', 'U-Shape', 'I-Shape'];
const AISLES = [5, 6, 7, 6, 5];
/** How long a drawn floor is looked at before the next change, as a person would (wall clock). */
const LOOK_MS = 500;

/** The stack measured: its UI's address and the API's log. */
interface Stack { url: string; log: string }
interface Mark { name: string; at: number; detail: Record<string, unknown> | null }
type Row = Record<string, number | string>;

const load = (): string => execFileSync('uptime', { encoding: 'utf8' }).trim().replace(/^.*load averages?: /, '');

/** The processes listening on TCP `port` on this machine, by pid; none when nothing listens (lsof then exits 1). */
function listeners(port: string): string[] {
  try {
    return execFileSync('lsof', ['-nP', `-iTCP:${port}`, '-sTCP:LISTEN', '-t'], { encoding: 'utf8' }).split('\n').filter(Boolean);
  } catch {
    return [];
  }
}

/**
 * The stack dev.sh started from this worktree: the UI's port and the API's from studio/.dev-state, each held only while
 * the process listening on it is the one this worktree's dev.sh recorded in `ui.pid` or `api.pid`; else a refusal.
 */
function ownStack(): Stack {
  const studio = new URL('../../../', import.meta.url);
  const state = (name: string): string => {
    try {
      return readFileSync(new URL(`.dev-state/${name}`, studio), 'utf8').trim();
    } catch {
      return '';
    }
  };
  const ports = state('ports');
  const owned = (unit: string, variable: string): string => {
    const [port, pid] = [new RegExp(`^${variable}=(\\d+)$`, 'm').exec(ports)?.[1], state(`${unit}.pid`)];
    if (port !== undefined && pid !== '' && listeners(port).includes(pid)) return port;
    throw new Error(`eval-latency measures only the stack dev.sh started from ${fileURLToPath(new URL('..', studio))}, `
      + `and its ${unit} is not what listens on the port that worktree recorded: start it there with ./dev.sh start.`);
  };
  const ui = owned('ui', 'UI_PORT');
  owned('api', 'API_PORT');
  return { url: `http://127.0.0.1:${ui}`, log: fileURLToPath(new URL('.dev-log/api.log', studio)) };
}

/**
 * One evaluation's timing line: when it was logged (epoch ms of wall clock, the machine's one clock), the outcome its
 * row was committed with, and its total and phases in wall-clock milliseconds.
 */
interface Timed { end: number; outcome: string; phases: Record<string, number> }

/** The API's timing line for each evaluation of `draft`, by its number. */
function serverPhases(log: string, draft: string): Map<number, Timed> {
  const found = new Map<number, Timed>();
  const timed = /^(\S+) .*template evaluation timed in wall-clock milliseconds draft=(\S+) n=(\d+) outcome=(\S+) wall_clock_ms=(\d+) phases_wall_clock_ms=(.*)$/;
  for (const line of readFileSync(log, 'utf8').split('\n')) {
    const read = timed.exec(line);
    if (read === null || read[2] !== draft) continue;
    const phases: Record<string, number> = { total: Number(read[5]) };
    for (const pair of read[6].trim().split(' ')) {
      const [name, ms] = pair.split('=');
      phases[name] = (phases[name] ?? 0) + Number(ms);
    }
    found.set(Number(read[3]), { end: Date.parse(read[1]), outcome: read[4], phases });
  }
  return found;
}

/** One session on `stack`: a fresh draft of `title`, the knob changed five times, each change's steps in ms. */
async function session(browser: Browser, stack: Stack, title: string): Promise<Row[]> {
  const context = await browser.newContext({ ...devices['Desktop Chrome'], baseURL: stack.url });
  await context.addInitScript((flag) => {
    (globalThis as unknown as Record<string, unknown>)[flag] = true;
  }, MEASURING);
  const page = await context.newPage();
  try {
    await freshThread(page);
    await page.goto('/templates');
    const card = page.locator('li.template-card', { has: page.locator('h3', { hasText: new RegExp(`^${title}$`) }) });
    await card.getByRole('button', { name: 'Use this template' }).click({ timeout: 60_000 });
    await page.waitForURL(/\/template-drafts\/[0-9A-Z]{26}$/);
    const draft = new URL(page.url()).pathname.split('/').pop() ?? '';
    const painted = (n: number) => page.waitForFunction(
      (key) => performance.getEntriesByName('floor:painted').some((mark) => (mark as PerformanceMark).detail?.key === key),
      `template-drafts/${draft}/${n}/floor.msgpack`, { timeout: 120_000, polling: 50 });
    await painted(1);
    const aisles = page.getByLabel('Aisles', { exact: true });
    for (const [index, value] of AISLES.entries()) {
      await page.waitForTimeout(LOOK_MS);
      await aisles.fill(String(value));
      await page.waitForFunction((count) => performance.getEntriesByName('template:knob-accepted').length >= count, index + 1, { timeout: 30_000, polling: 20 });
      const n = await page.evaluate((at) => ((performance.getEntriesByName('template:knob-accepted')[at] as PerformanceMark).detail as { n: number }).n, index);
      await painted(n);
    }
    // Each mark's instant in epoch ms, the clock the API's log lines are stamped with; the page keeps none once read.
    const marks: Mark[] = await page.evaluate(() => {
      const kept = performance.getEntriesByType('mark').map((mark) => ({
        name: mark.name, at: performance.timeOrigin + mark.startTime, detail: (mark as PerformanceMark).detail as Record<string, unknown> | null,
      }));
      performance.clearMarks();
      return kept;
    });
    const server = serverPhases(stack.log, draft);
    return marks.filter((mark) => mark.name === 'template:knob-accepted').map((accepted) => {
      const n = accepted.detail?.n as number;
      const after = (name: string, from: number, fits: (mark: Mark) => boolean = () => true) =>
        marks.find((mark) => mark.name === name && mark.at >= from && fits(mark))?.at ?? Number.NaN;
      const key = `template-drafts/${draft}/${n}/floor.msgpack`;
      const sent = [...marks].reverse().find((mark) => mark.name === 'template:knob-sent' && mark.at <= accepted.at)?.at ?? Number.NaN;
      const changed = [...marks].reverse().find((mark) => mark.name === 'template:knob-changed' && mark.at <= sent)?.at ?? Number.NaN;
      const seenMark = marks.find((mark) => mark.name === 'template:draft-read' && mark.at >= accepted.at && mark.detail?.n === n && mark.detail?.state !== 'in-progress');
      const seen = seenMark?.at ?? Number.NaN;
      const evaluationRead = after('template:evaluation-read', seen, (mark) => mark.detail?.n === n);
      const byKey = (mark: Mark) => mark.detail?.key === key;
      const [asked, read, drawn, shown] = ['floor:asked', 'floor:read', 'floor:drawn', 'floor:painted'].map((name) => after(name, evaluationRead, byKey));
      const timed = server.get(n);
      const [start, end] = timed === undefined ? [Number.NaN, Number.NaN] : [timed.end - timed.phases.total, timed.end];
      // In event order, each step from the end of the one before, so that they add up to `after the pause`; the
      // evaluation's own phases, from the API's line, follow it.
      const inside = Object.entries(timed?.phases ?? {}).filter(([name]) => name !== 'total').map(([name, ms]) => [`server ${name}`, ms]);
      return {
        template: title, draft, n, via: String(seenMark?.detail?.via ?? '?'), outcome: timed?.outcome ?? '?',
        pause: sent - changed, 'to evaluation': start - sent, evaluation: end - start, ...Object.fromEntries(inside),
        told: seen - end, 'evaluation read': evaluationRead - seen, 'floor asked': asked - evaluationRead,
        'floor fetch': read - asked, set_floor: drawn - read, 'first frame': shown - drawn,
        'after the pause': shown - sent, 'change to floor': shown - changed,
      };
    });
  } finally {
    await context.close();
  }
}

const median = (values: number[]): number => {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
};

const LABELS = new Set(['template', 'draft', 'n', 'via', 'outcome', 'round', 'load']);

/** Phase by template: median and maximum ms, as markdown. */
function table(rows: Row[], templates: string[]): string {
  const phases = [...new Set(rows.flatMap((row) => Object.keys(row)))].filter((key) => !LABELS.has(key));
  const head = `| phase | ${templates.map((template) => `${template} med | max`).join(' | ')} |`;
  const rule = `|---|${templates.map(() => '---:|---:').join('|')}|`;
  const lines = phases.map((phase) => `| ${phase} | ${templates.map((template) => {
    const values = rows.filter((row) => row.template === template)
      .map((row) => row[phase]).filter((value): value is number => typeof value === 'number' && Number.isFinite(value));
    return values.length === 0 ? '- | -' : `${Math.round(median(values))} | ${Math.round(Math.max(...values))}`;
  }).join(' | ')} |`);
  return [head, rule, ...lines].join('\n');
}

async function main() {
  const args = process.argv.slice(2);
  const option = (name: string) => {
    const at = args.indexOf(name);
    return at >= 0 ? args[at + 1] : undefined;
  };
  const valued = ['--rounds', '--out', '--templates'];
  const stray = args.filter((arg, at) => arg !== '--shell' && !valued.includes(arg) && !valued.includes(args[at - 1] ?? ''));
  if (stray.length > 0) throw new Error(`eval-latency takes no stack or address (${stray.join(' ')}): it measures this worktree's own.`);
  const rounds = Number(option('--rounds') ?? 1);
  const out = option('--out');
  const only = option('--templates')?.split(',') ?? SHELLS;
  const stack = ownStack();
  const browser = await chromium.launch({ headless: true, channel: args.includes('--shell') ? undefined : 'chromium' });
  const rows: Row[] = [];
  console.log(`${args.includes('--shell') ? 'headless shell' : 'Chromium, new headless'}; load at start: ${load()}`);
  try {
    for (const title of only) {
      for (let round = 0; round < rounds; round += 1) {
        const before = load();
        const found = await session(browser, stack, title);
        rows.push(...found.map((row) => ({ ...row, round, load: before })));
        const headline = found.map((row) => Math.round(row['after the pause'] as number)).join(', ');
        console.log(`${title} round ${round + 1} (load ${before}): after the pause ${headline} ms`);
      }
    }
  } finally {
    await browser.close();
  }
  console.log(`load at end: ${load()}`);
  if (out !== undefined) writeFileSync(out, JSON.stringify(rows, null, 1));
  console.log(table(rows, only));
}

await main();
