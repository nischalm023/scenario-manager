// Drive a headless Chrome of this machine's own, over its debugging protocol.
//
// `chrome --headless=new --screenshot` cannot photograph either of the pages
// this repository wants photographed: it captures when the page has LOADED,
// and everything those pages do — open a websocket, take sixty-odd frames,
// draw them — happens after that. `--virtual-time-budget` does not help,
// because virtual time races past a socket that is still delivering. So this
// opens the page, watches its console, waits on a condition the caller states,
// and only then takes the picture.
//
// Node's built-in WebSocket is the whole dependency: nothing is installed, and
// neither Playwright nor puppeteer is on this machine.
//
// Never the browser somebody is using. Every window opened here is a headless
// one with a temporary profile of its own.

import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

export const CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

/**
 * Start a headless Chrome and attach to its first page.
 *
 * `webgpu: false` takes `navigator.gpu` away from every page before anything
 * in it runs, which is the state a browser without WebGPU is in — Chrome's own
 * WebGPU-disabling flags leave the object in place with no adapter behind it,
 * and wgpu then binds the canvas to a context it cannot draw through. That is
 * how the WebGL2 fallback gets photographed on a machine whose browser would
 * otherwise take the other path.
 */
export async function openChrome({
  width = 1240,
  height = 900,
  webgpu = true,
  chrome = CHROME,
  quiet = false,
  deadline = Date.now() + 60_000,
} = {}) {
  const profile = await mkdtemp(join(tmpdir(), 'crucible-headless-'));
  const forget = async () => {
    await rm(profile, { recursive: true, force: true }).catch(() => undefined);
  };
  const browser = spawn(
    chrome,
    [
      '--headless=new',
      '--remote-debugging-port=0',
      `--user-data-dir=${profile}`,
      `--window-size=${width},${height}`,
      // This Chrome offers WebGPU headless already; withholding it is what
      // shows the WebGL2 fallback drawing. The page reports which it got.
      '--enable-unsafe-webgpu',
      '--no-first-run',
      '--disable-extensions',
      'about:blank',
    ],
    { stdio: ['ignore', 'ignore', 'pipe'] }
  );
  browser.stderr.on('data', () => {});
  // A machine without Chrome where this expects it says so, rather than ending
  // the process with an unhandled ENOENT naming nothing — the courtesy `dev.sh`
  // pays for bun, npm and the container engine.
  let refused;
  browser.on('error', (failure) => {
    refused = failure.code === 'ENOENT' ? new Error(`no browser at ${chrome}`) : failure;
  });

  // The debugging port Chrome chose, off the file it writes into the profile.
  let port;
  while (Date.now() < deadline && port === undefined && refused === undefined) {
    try {
      const [line] = (await readFile(join(profile, 'DevToolsActivePort'), 'utf8')).split('\n');
      if (line) port = Number(line);
    } catch {
      await pause(100);
    }
  }
  if (refused !== undefined) {
    await forget();
    throw refused;
  }
  if (port === undefined) {
    browser.kill();
    await forget();
    throw new Error(`Chrome at ${chrome} never opened a debugging port`);
  }

  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const page = targets.find((target) => target.type === 'page');
  const socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((open, fail) => {
    socket.addEventListener('open', open, { once: true });
    socket.addEventListener('error', fail, { once: true });
  });

  let sent = 0;
  const waiting = new Map();
  const console_ = [];
  socket.addEventListener('message', (message) => {
    const said = JSON.parse(message.data);
    if (said.id !== undefined && waiting.has(said.id)) {
      const { settle, fail } = waiting.get(said.id);
      waiting.delete(said.id);
      said.error ? fail(new Error(said.error.message)) : settle(said.result);
    }
    if (said.method === 'Runtime.consoleAPICalled') {
      // The severity is kept, not flattened into the text: a gate that fails on
      // what a page complained about has to be able to tell `console.error`
      // from the page saying how many frames it took.
      const line = said.params.args.map((arg) => arg.value ?? arg.description).join(' ');
      console_.push({ level: said.params.type, text: line });
      if (!quiet) console.log(`  page ${said.params.type}:`, line);
    }
    if (said.method === 'Runtime.exceptionThrown') {
      const thrown = said.params.exceptionDetails;
      const line = thrown.exception?.description ?? thrown.text;
      console_.push({ level: 'exception', text: line });
      if (!quiet) console.log('  page threw:', line);
    }
  });

  const call = (method, params = {}) =>
    new Promise((settle, fail) => {
      const id = ++sent;
      waiting.set(id, { settle, fail });
      socket.send(JSON.stringify({ id, method, params }));
    });

  await call('Runtime.enable');
  await call('Page.enable');
  if (!webgpu) {
    await call('Page.addScriptToEvaluateOnNewDocument', {
      source: "Object.defineProperty(navigator, 'gpu', { value: undefined });",
    });
  }

  /** Evaluate an expression in the page and bring its value back. */
  const evaluate = async (expression) => {
    const { result, exceptionDetails } = await call('Runtime.evaluate', {
      expression,
      returnByValue: true,
      awaitPromise: true,
    });
    if (exceptionDetails) {
      throw new Error(exceptionDetails.exception?.description ?? exceptionDetails.text);
    }
    return result.value;
  };

  /**
   * The next load event, or a refusal after `waitMs`. Started BEFORE whatever
   * causes the load, because a fast page fires it before the call returns.
   */
  const nextLoad = (waitMs) =>
    new Promise((settle, fail) => {
      const listener = (message) => {
        if (JSON.parse(message.data).method === 'Page.loadEventFired') {
          clearTimeout(giveUp);
          socket.removeEventListener('message', listener);
          settle();
        }
      };
      const giveUp = setTimeout(() => {
        socket.removeEventListener('message', listener);
        fail(new Error(`the page never finished loading within ${waitMs} ms`));
      }, waitMs);
      socket.addEventListener('message', listener);
    });

  return {
    call,
    evaluate,
    /** Every console line and uncaught exception, each with its severity. */
    console: console_,
    /**
     * What the page has complained about since `from` — console errors and
     * anything thrown. A caller failing on these takes the length of this
     * array as its mark and passes it back next time, so one complaint fails
     * one step rather than every step after it.
     */
    complaints(from = 0) {
      return console_.slice(from).filter((line) => line.level === 'error' || line.level === 'exception');
    },
    /**
     * Go to a page and wait for its load event.
     *
     * Only for an address that is a different DOCUMENT. Going from a page to
     * the same page with another fragment is a same-document navigation: the
     * browser fires no load event and nothing is reloaded, so this would wait
     * for something that is never going to happen. That is `reload`'s job.
     */
    async navigate(url, waitMs = 60_000) {
      const loaded = nextLoad(waitMs);
      await call('Page.navigate', { url });
      await loaded;
    },
    /**
     * Put the page at `url` and load it again from scratch — a person pressing
     * reload, which is what a shell that rebuilds its state from its own
     * address has to survive.
     */
    async reload(url, waitMs = 60_000) {
      await evaluate(`location.href = ${JSON.stringify(url)}`);
      const loaded = nextLoad(waitMs);
      await call('Page.reload');
      await loaded;
    },
    /**
     * Wait until `expression` is truthy in the page, and answer with its value.
     * `what` is what the wait was for, so a timeout says which one ran out.
     */
    async until(expression, what, waitMs = 60_000) {
      const stop = Date.now() + waitMs;
      while (Date.now() < stop) {
        const now = await evaluate(expression);
        if (now) return now;
        await pause(150);
      }
      throw new Error(`waited ${waitMs} ms for ${what} and it never came true`);
    },
    async screenshot(path) {
      const { data } = await call('Page.captureScreenshot', { format: 'png' });
      await writeFile(path, Buffer.from(data, 'base64'));
      return path;
    },
    /** Stop the browser and remove the profile directory it was given. */
    close() {
      browser.kill();
      return forget();
    },
  };
}

export const pause = (ms) => new Promise((wake) => setTimeout(wake, ms));

/** `--name value` off the command line, or a fallback. */
export function asked(name, fallback) {
  const at = process.argv.indexOf(`--${name}`);
  return at === -1 ? fallback : process.argv[at + 1];
}
