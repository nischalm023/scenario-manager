// Drive `studio` end to end in a headless Chrome and photograph each step.
//
// This is the round's proof that the shell works: submit the cencora fixture,
// watch its progress count up, watch it finish, play, scrub inside the frames
// the viewer holds, reload at an instant, seek outside the window, turn each of
// the three overlays and each of the five views on, open a SECOND run on a
// different floor, open the DFW6 site and time its frames, cancel a run and
// open what it recorded, and submit a spec with its own files through the
// form — each one driven through the page's own controls, not a back door.
//
// Every step asserts what its own caption claims — the run, the attempt's
// state, where the cursor stands, the window held, how many times the window
// has been opened again, which backend the tab gave wgpu, that the status line
// carries no complaint, and that something was actually drawn — before its
// screenshot is taken. A blank canvas cannot pass, and neither can a page
// throwing on every animation frame: a console error or an uncaught exception
// fails the step it appeared in.
//
//   node ui_gate.mjs --ui http://127.0.0.1:<port>/ --shots <folder>
//                    [--minutes 10] [--wait 300000] [--repository <path>]
//                    [--no-webgpu] [--run <id>]
//
// `--no-webgpu` runs the WebGL2 half: it opens the run that is already there at
// an instant and photographs it, because the fallback is about drawing, not
// about the shell.

import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { asked, openChrome, pause } from './headless_chrome.mjs';

const ui = asked('ui');
const shots = asked('shots');
if (!ui || !shots) {
  console.error('ui_gate.mjs: --ui and --shots are both needed');
  process.exit(2);
}
const minutes = asked('minutes', '10');
const waitMs = Number(asked('wait', 300_000));
const webgpu = !process.argv.includes('--no-webgpu');
const backend = webgpu ? 'webgpu' : 'webgl';
// The checkout whose fixtures the "files from this machine" step picks up. It
// defaults to the checkout this script is in, which is the one being gated.
const repository = resolve(asked('repository', join(dirname(fileURLToPath(import.meta.url)), '..', '..')));

await mkdir(shots, { recursive: true });
const chrome = await openChrome({ width: 1500, height: 950, webgpu, quiet: true });
const taken = [];
/** What each timed floor cost to draw, for the results table. */
const timings = [];
/** How many console lines have already been accounted for. */
let seen = 0;

/** How many frames a timing step plays before it reads the page's numbers. */
const TIMED_FRAMES = 300;

/** What the status line says right now, as its data attributes. */
const status = () => chrome.evaluate("JSON.stringify({ ...document.getElementById('status')?.dataset })");

/**
 * Photograph one step, having first held it to what its caption claims.
 *
 * `expect` is read field by field, so a step states only what it is about: the
 * run being watched, the phase, the cursor (an instant, or a `[low, high]`
 * pair), the window as `from-to` or a prefix of it, how many re-attaches have
 * happened, the backend, and what was drawn — `movers` for a view with robots
 * and containers on it, `cells` for an overlay view, which draws none.
 */
async function shoot(name, shows, expect = {}) {
  const said = JSON.parse((await status()) ?? '{}');
  const faults = [];

  for (const complaint of chrome.complaints(seen)) {
    faults.push(`the page said: ${complaint.level}: ${complaint.text}`);
  }
  seen = chrome.console.length;

  if (expect.run !== undefined && said.run !== expect.run) faults.push(`watching \`${said.run}\`, not ${expect.run}`);
  if (expect.phase !== undefined && !expect.phase.includes(said.phase)) {
    faults.push(`the attachment is \`${said.phase}\`, not ${expect.phase.join(' or ')}`);
  }
  if (expect.backend !== undefined && said.backend !== expect.backend) {
    faults.push(`wgpu got \`${said.backend}\`, not ${expect.backend}`);
  }
  if (expect.cursorMs !== undefined) {
    const [low, high] = Array.isArray(expect.cursorMs) ? expect.cursorMs : [expect.cursorMs, expect.cursorMs];
    const at = Number(said.cursorMs);
    if (!(at >= low && at <= high)) faults.push(`the cursor stands at ${at} ms, not within ${low}–${high}`);
  }
  if (expect.window !== undefined && !String(said.window).startsWith(expect.window)) {
    faults.push(`the window is \`${said.window}\`, and this step holds ${expect.window}…`);
  }
  if (expect.view !== undefined && said.view !== expect.view) {
    faults.push(`the floor is drawn in the \`${said.view}\` view, not ${expect.view}`);
  }
  if (expect.reattaches !== undefined && Number(said.reattaches) !== expect.reattaches) {
    faults.push(`the window has been opened again ${said.reattaches} times, not ${expect.reattaches}`);
  }
  if (expect.frames !== undefined && !(Number(said.frames) >= expect.frames)) {
    faults.push(`${said.frames} frames have arrived, and this step needs at least ${expect.frames}`);
  }
  const [robots, containers, cells] = String(said.drawn ?? '')
    .split(',')
    .map(Number);
  if (expect.drew === 'movers' && !(robots > 0 && containers > 0 && cells > 0)) {
    faults.push(`this step claims a drawn floor and the last frame submitted ${said.drawn}`);
  }
  if (expect.drew === 'cells' && !(cells > 0)) {
    faults.push(`this step claims a drawn floor and the last frame submitted ${said.drawn} cells`);
  }
  if (expect.cells !== undefined && cells !== expect.cells) {
    faults.push(`the floor drawn has ${cells} cells, not ${expect.cells}`);
  }
  if (said.trouble) faults.push(`the status line says: ${said.trouble}`);

  const file = join(shots, `${name}.png`);
  await chrome.screenshot(file);
  taken.push({ name, shows, said });
  console.log(`  ${name}: ${shows}`);
  console.log(
    `    ${said.phase ?? '-'} · ${said.backend ?? '-'} · ${said.frames ?? 0} frames · cursor ${said.cursorMs ?? '-'} ms · drew ${said.drawn ?? '-'} · window ${said.window || '-'}`
  );
  if (faults.length > 0) throw new Error(`${name} (${shows}):\n    - ${faults.join('\n    - ')}`);
  return said;
}

/**
 * Set a React-controlled input's value the way a person does. React listens
 * for the input event and reads the property, and assigning the property
 * directly does not raise the event — hence the prototype's own setter.
 */
const setControl = (selector, value) => `
  (() => {
    const element = document.querySelector(${JSON.stringify(selector)});
    if (!element) return false;
    const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
    setter.call(element, ${JSON.stringify(String(value))});
    element.dispatchEvent(new Event('input', { bubbles: true }));
    return true;
  })()`;

const click = (selector) => `
  (() => {
    const element = document.querySelector(${JSON.stringify(selector)});
    if (!element) return false;
    element.click();
    return true;
  })()`;

/**
 * Attached and holding frames, which is what every step below waits for. The
 * phase reads `ended` rather than `attached` once the recording's end frame has
 * arrived — which for a finished run is within a second of attaching — and both
 * mean the window is open.
 */
const watching =
  "['attached', 'ended'].includes(document.getElementById('status')?.dataset.phase) ? document.getElementById('status').dataset.run : null";

/** Drawn: the last frame submitted cells, so there is a floor on the canvas. */
const drawing = "Number((document.getElementById('status')?.dataset.drawn ?? '0,0,0').split(',')[2]) > 0";

/** The run the page says it is watching, off its own address. */
const named = "new URLSearchParams(location.hash.slice(1)).get('run')";

/** Where the page's own address says the cursor is. */
const addressAt = "Number(new URLSearchParams(location.hash.slice(1)).get('at') ?? -1)";

/** That run's row in the sidebar says it is in that state. */
const inState = (run, state) => `${inStateOf(run)} === ${JSON.stringify(state)}`;

/** The state that run's row in the sidebar shows. */
const inStateOf = (run) => `document.querySelector('#run-list [data-run="${run}"]')?.dataset.state`;

/** Where the playback cursor stands, in milliseconds. */
const cursorMs = "Number(document.getElementById('cursor-readout')?.dataset.cursorMs ?? -1)";

const mustClick = async (selector, what) => {
  if (!(await chrome.evaluate(click(selector)))) throw new Error(`there is no ${what} on the page (${selector})`);
};

/**
 * Play this floor for 300 frames and photograph what it cost to draw.
 *
 * The numbers are the page's own: the draw loop times the gap between frames
 * and the draw inside it, and keeps the last three hundred of each. Playing is
 * what makes the measurement worth taking — a paused viewer redraws one
 * unchanging instant — so this presses play, waits for three hundred frames to
 * go by since it did, reads the pair, and pauses again.
 */
async function timeFrames(name, what, expect) {
  const before = Number(JSON.parse((await status()) ?? '{}').timed ?? 0);
  await mustClick('#play', 'the play button');
  await chrome.until(
    `Number(document.getElementById('status')?.dataset.timed) >= ${before + TIMED_FRAMES}`,
    `${TIMED_FRAMES} frames to be drawn`,
    waitMs
  );
  const said = JSON.parse((await status()) ?? '{}');
  await mustClick('#play', 'the pause button');
  const [frame, worstFrame] = String(said.frameMs).split(',').map(Number);
  const [draw, worstDraw] = String(said.drawMs).split(',').map(Number);
  const slow = worstFrame > 33 ? ` — the worst frame took ${worstFrame} ms, past 33 ms` : '';
  const shows = `${what}: ${frame} ms median between frames (worst ${worstFrame}), ${draw} ms median in the draw (worst ${worstDraw})${slow}`;
  await shoot(name, shows, expect);
  return { frame, worstFrame, draw, worstDraw };
}

/**
 * Pick a spec and the files it names off this machine and submit them, the way
 * a person does with the file chooser: the page is handed the bytes as `File`s
 * on the form's own input, and the form reads them itself.
 */
async function submitFromDisk(name, paths) {
  const files = [];
  for (const path of paths) {
    files.push({ name: path.split('/').at(-1), bytes: (await readFile(join(repository, path))).toString('base64') });
  }
  await chrome.evaluate(setControl('#submit-minutes', '2'));
  await chrome.evaluate(setControl('#submit-threads', '1'));
  await chrome.evaluate(setControl('#submit-name', name));
  const put = await chrome.evaluate(`
    (() => {
      const input = document.getElementById('submit-files');
      if (!input) return false;
      const carrier = new DataTransfer();
      for (const one of ${JSON.stringify(files)}) {
        const bytes = Uint8Array.from(atob(one.bytes), (letter) => letter.charCodeAt(0));
        carrier.items.add(new File([bytes], one.name, { type: 'application/json' }));
      }
      input.files = carrier.files;
      input.dispatchEvent(new Event('change', { bubbles: true }));
      return input.files.length;
    })()`);
  if (put !== paths.length) throw new Error(`the form took ${put} of the ${paths.length} files picked`);
  const watchedBefore = await chrome.evaluate(named);
  await mustClick('#submit-picked', 'the button that submits the files picked');
  return chrome.until(
    `(() => { const said = ${named}; return said && said !== ${JSON.stringify(watchedBefore)} ? said : null; })()`,
    `${name} to be accepted`,
    waitMs
  );
}

/** Submit a bundled fixture and answer with the run the page starts watching. */
async function submit(fixture, forMinutes, name, notBeing = null) {
  await chrome.evaluate(setControl('#submit-minutes', forMinutes));
  await chrome.evaluate(setControl('#submit-threads', '1'));
  await chrome.evaluate(setControl('#submit-name', name));
  await mustClick(`[data-fixture="${fixture}"]`, `the ${fixture} fixture button`);
  // The page writes what it is watching into its own address, so the id of the
  // run just submitted is there to read — and every wait names it, rather than
  // trusting whichever run happens to be at the top of the list.
  return chrome.until(
    notBeing === null
      ? named
      : `(() => { const said = ${named}; return said && said !== ${JSON.stringify(notBeing)} ? said : null; })()`,
    `${name} to be accepted`,
    waitMs
  );
}

try {
  // ── the WebGL half: the run is already recorded, so open it and draw ──────
  if (!webgpu) {
    const at = asked('at', '300000');
    const run = asked('run');
    if (!run) throw new Error('--no-webgpu needs --run <id>, the run WebGPU already recorded');
    await chrome.navigate(`${ui}#run=${run}&at=${at}`);
    await chrome.until(watching, 'the attachment', waitMs);
    await chrome.until(drawing, 'the first frame to draw', waitMs);
    await pause(1500);
    await shoot(`18-webgl-at-${at}`, `the same run at ${Number(at) / 1000} s on the WebGL2 fallback`, {
      run,
      phase: ['attached', 'ended'],
      backend: 'webgl',
      cursorMs: [Number(at) - 100, Number(at) + 100],
      window: String(at),
      drew: 'movers',
      frames: 1,
    });
  } else {
    // ── open ─────────────────────────────────────────────────────────────────
    await chrome.navigate(ui);
    await chrome.until("document.getElementById('status')?.dataset.backend !== 'opening'", 'the viewer to open');
    await shoot('01-opened', 'the shell with no run watched: the submit form, an empty run list, the status line', {
      phase: ['idle'],
      backend: 'webgpu',
    });

    // ── submit cencora, and wait for it ──────────────────────────────────────
    //
    // The run is queued and then simulated, and what says so is the sidebar's
    // own clock moving: the server copies each run's counters into the registry
    // twice a second and `list` carries them, so a row whose simulated time
    // rises is a run being executed rather than a row that was drawn once.
    const run = await submit('cencora_v3', minutes, 'cencora-ui-gate');
    const simMs = `Number(document.querySelector('#run-list [data-run="${run}"]')?.dataset.simMs ?? -1)`;
    await chrome.until(`['queued', 'running'].includes(${inStateOf(run)}) ? 1 : 0`, 'the run to be queued', waitMs);
    await chrome.until(`${simMs} > 0`, 'the run to start simulating', waitMs);
    const counted = Number(await chrome.evaluate(simMs));
    await chrome.until(`${simMs} > ${counted}`, 'its simulated clock to move on', waitMs);
    await shoot('02-submitted', `${run} running in the sidebar, its simulated clock counting up past ${counted} ms`, {
      backend: 'webgpu',
    });

    await chrome.until(inState(run, 'finished'), 'the run to finish', waitMs);
    await shoot('03-finished', 'the run finished; the shell attaches on its own now there is a recording', {
      backend: 'webgpu',
    });

    // ── watched at the start ─────────────────────────────────────────────────
    await chrome.until(watching, 'the attachment', waitMs);
    await chrome.until("Number(document.getElementById('status')?.dataset.frames) > 60", 'the frames', waitMs);
    await chrome.until(drawing, 'the first frame to draw', waitMs);
    // Two of the run's own metrics, chosen through the filter box. The picks
    // and orders a chart opens on are flat at zero on a ten-minute cencora, so
    // they come off first — which is the picker being driven both ways.
    await chrome.evaluate(
      "document.querySelectorAll('#metric-list [data-metric] input:checked').forEach((box) => box.click())"
    );
    await chrome.evaluate(setControl('#metric-filter', 'travel_distance_mm'));
    await chrome.evaluate(
      "Array.from(document.querySelectorAll('#metric-list [data-metric] input')).slice(0, 2).forEach((box) => box.click())"
    );
    await chrome.evaluate(setControl('#metric-filter', ''));
    await pause(2000);
    await shoot(
      '04-attached-at-zero',
      'attached at 0: the cencora floor drawn, the held window the whole run, two of its own metrics charted',
      { run, phase: ['attached', 'ended'], backend: 'webgpu', cursorMs: [0, 0], window: '0-', reattaches: 0, drew: 'movers', frames: 60 }
    );

    // ── play ─────────────────────────────────────────────────────────────────
    await mustClick('#play', 'the play button');
    await pause(2500);
    await shoot('05-playing', 'playing forward: the cursor has moved off zero on its own', {
      run,
      phase: ['attached', 'ended'],
      cursorMs: [1, Number.MAX_SAFE_INTEGER],
      reattaches: 0,
      drew: 'movers',
    });
    await mustClick('#play', 'the pause button');
    // The address follows the cursor while it plays, so a reload lands where
    // the run got to rather than back at the instant it was opened at.
    const followed = Number(await chrome.evaluate(addressAt));
    if (!(followed > 0)) throw new Error(`the address says at=${followed} after playing forward`);

    // ── scrub, inside the window ─────────────────────────────────────────────
    await chrome.evaluate(setControl('#cursor', 450_000));
    await chrome.evaluate(
      "document.getElementById('cursor').dispatchEvent(new PointerEvent('pointerup', { bubbles: true }))"
    );
    await chrome.until(`${cursorMs} >= 449900 && ${cursorMs} <= 450100`, 'the cursor to reach 450 s', waitMs);
    await pause(1200);
    await shoot('06-scrubbed', 'the cursor dragged to 450 s inside the held window: no re-attach', {
      run,
      phase: ['attached', 'ended'],
      cursorMs: [449_900, 450_100],
      window: '0-',
      reattaches: 0,
      drew: 'movers',
    });

    // ── reload, and re-attach where the address says ─────────────────────────
    await chrome.reload(`${ui}#run=${run}&at=300000`);
    await chrome.until(watching, 'the re-attachment', waitMs);
    await chrome.until("Number(document.getElementById('status')?.dataset.frames) > 20", 'the frames', waitMs);
    await chrome.until(drawing, 'the first frame to draw', waitMs);
    await pause(1500);
    await shoot(
      '07-reloaded-at-300s',
      'the page reloaded and re-attached at 300 s: the window now opens there, not at 0',
      {
        run,
        phase: ['attached', 'ended'],
        backend: 'webgpu',
        cursorMs: [299_900, 300_100],
        window: '300000',
        reattaches: 0,
        drew: 'movers',
        frames: 20,
      }
    );

    // ── seek, outside the window ─────────────────────────────────────────────
    //
    // The window that comes back opens at the KEYFRAME at or before 60 s,
    // which on this recording is the one at 0 — so what says the seek happened
    // is that the window moved and the page opened it again, not that it
    // begins at the instant asked for.
    await chrome.evaluate(setControl('#cursor', 60_000));
    await chrome.evaluate(
      "document.getElementById('cursor').dispatchEvent(new PointerEvent('pointerup', { bubbles: true }))"
    );
    await chrome.until(
      `Number(document.getElementById('status')?.dataset.reattaches) > 0 &&
       Number(document.getElementById('status')?.dataset.window?.split('-')[0]) <= 60000`,
      'the window to open again at or before 60 s',
      waitMs
    );
    await chrome.until(drawing, 'the first frame of the new window to draw', waitMs);
    await pause(1500);
    await shoot(
      '08-seek-to-60s',
      'the cursor dragged to 60 s, outside the window: the shell attached again, on the keyframe at or before it',
      {
        run,
        phase: ['attached', 'ended'],
        backend: 'webgpu',
        cursorMs: [59_900, 60_100],
        window: '0-',
        reattaches: 1,
        drew: 'movers',
        frames: 1,
      }
    );

    // ── the three overlays, each of which needs another section ──────────────
    //
    // An overlay is about the cells, so it submits no movers at all; and each
    // click opens the window again, because the tints ride a section the last
    // attachment did not ask for.
    const overlays = [
      ['visits', 'the visit counts, off the heat section'],
      ['wait', 'how long robots waited, measured from the motion section'],
      ['speed', 'how fast they moved, measured from the same section'],
    ];
    let reattaches = 1;
    for (const [overlay, what] of overlays) {
      await mustClick(`[data-overlay-choice="${overlay}"]`, `the ${overlay} overlay button`);
      reattaches += 1;
      await chrome.until(watching, `the ${overlay} attachment`, waitMs);
      await chrome.until(drawing, 'the overlay to draw', waitMs);
      await pause(2000);
      await shoot(`09-overlay-${overlay}`, `the ${overlay} overlay: ${what}`, {
        run,
        phase: ['attached', 'ended'],
        backend: 'webgpu',
        reattaches,
        drew: 'cells',
        frames: 1,
      });
    }
    await mustClick('[data-overlay-choice="off"]', 'the no-overlay button');
    reattaches += 1;
    await chrome.until(drawing, 'the floor to draw again', waitMs);

    // ── the five views ───────────────────────────────────────────────────────
    //
    // A view is the viewer's own state and no new sections, so nothing is
    // asked for again: what each step holds is that the page is in the view it
    // clicked and drawing what that view is about. Only the state view draws
    // the movers — a reachability or a measurement view is about the cells,
    // and robots standing over it would be read as part of the answer.
    const views = [
      ['state', 'what each cell and robot is doing right now', 'movers'],
      ['unladen-htm', 'where a D30 carrying nothing may go', 'cells'],
      ['laden-htm', 'where a D30 under a tote may go', 'cells'],
      ['vtm', 'where a DA50 may go', 'cells'],
      ['simulation', 'the cells painted with what the run measured', 'cells'],
    ];
    for (const [mode, what, drew] of views) {
      await mustClick(`[data-view="${mode}"]`, `the ${mode} view button`);
      await chrome.until(`document.getElementById('status')?.dataset.view === ${JSON.stringify(mode)}`, mode, waitMs);
      await chrome.until(drawing, `the ${mode} view to draw`, waitMs);
      await pause(1200);
      await shoot(`10-view-${mode}`, `the ${mode} view: ${what}`, {
        run,
        phase: ['attached', 'ended'],
        backend: 'webgpu',
        view: mode,
        reattaches,
        drew,
      });
    }
    await mustClick('[data-view="state"]', 'the state view button');
    await chrome.until("document.getElementById('status')?.dataset.view === 'state'", 'the state view', waitMs);

    // ── how fast this machine draws cencora ──────────────────────────────────
    const cencoraTimes = await timeFrames('11-cencora-frame-time', 'cencora playing', {
      run,
      phase: ['attached', 'ended'],
      backend: 'webgpu',
      view: 'state',
      reattaches,
      drew: 'movers',
    });

    // ── a second run on a DIFFERENT floor ────────────────────────────────────
    //
    // The floor is GPU state: the pipelines hold its cells and the cell index
    // its squares. A second scenario block rebuilds both, and what says so is
    // that the cell count is this floor's and not the last one's — cencora has
    // 9,001 cells and minimal_easyloop_flow far fewer.
    const cencoraCells = Number(String(JSON.parse((await status()) ?? '{}').drawn).split(',').at(-1));
    const second = await submit('minimal_easyloop_flow', '10', 'easyloop-ui-gate', run);
    await chrome.until(inState(second, 'finished'), 'the second run to finish', waitMs);
    await chrome.until(`${watching} === ${JSON.stringify(second)}`, 'the second attachment', waitMs);
    await chrome.until(drawing, "the second run's floor to draw", waitMs);
    await pause(2000);
    const other = await shoot(
      '12-second-run-different-floor',
      `a second run on another floor (${second}): the floor, the run state and the charts are all this run's`,
      { run: second, phase: ['attached', 'ended'], backend: 'webgpu', cursorMs: [0, 0], window: '0-', drew: 'movers' }
    );
    const drewCells = Number(String(other.drawn).split(',').at(-1));
    if (drewCells === cencoraCells) {
      throw new Error(`the second run drew ${drewCells} cells, the same floor the first one drew`);
    }

    // ── the DFW6 site, the largest floor there is, opened and played ─────────
    const dfw6 = await submit('walmart_dfw6_smoke', '10', 'dfw6-ui-gate', second);
    await chrome.until(inState(dfw6, 'finished'), 'the DFW6 run to finish', waitMs);
    await chrome.until(`${watching} === ${JSON.stringify(dfw6)}`, 'the DFW6 attachment', waitMs);
    await chrome.until(drawing, "DFW6's floor to draw", waitMs);
    await pause(2000);
    await shoot('13-dfw6-opened', `the DFW6 site (${dfw6}) opened at 0`, {
      run: dfw6,
      phase: ['attached', 'ended'],
      backend: 'webgpu',
      cursorMs: [0, 0],
      drew: 'movers',
    });
    const dfw6Times = await timeFrames('14-dfw6-frame-time', 'DFW6 playing', {
      run: dfw6,
      phase: ['attached', 'ended'],
      backend: 'webgpu',
      drew: 'movers',
    });

    // ── a run cancelled part-way, and what it recorded ───────────────────────
    //
    // A cancelled run is recorded from the model as it stands when the stop
    // lands, so what it holds is a shorter run rather than a torn file — and
    // the page opens it the way it opens any other.
    const stopped = await submit('cencora_v3', '120', 'cencora-ui-gate-cancelled', dfw6);
    await chrome.until(`document.querySelector('[data-cancel="${stopped}"]') !== null`, 'it to be cancellable', waitMs);
    await chrome.until(
      `Number(document.querySelector('#run-list [data-run="${stopped}"]')?.dataset.simMs) > 30000`,
      'the run to simulate its first half-minute',
      waitMs
    );
    await mustClick(`[data-cancel="${stopped}"]`, 'the cancel button');
    await chrome.until(inState(stopped, 'cancelled'), 'the run to be cancelled', waitMs);
    await pause(500);
    await shoot('15-cancelled', `a run (${stopped}) submitted at 120 minutes and cancelled part-way`, {
      backend: 'webgpu',
    });

    await mustClick(`#run-list [data-run="${stopped}"] button`, "the cancelled run's name");
    await chrome.until(`${watching} === ${JSON.stringify(stopped)}`, "the cancelled run's attachment", waitMs);
    await chrome.until(drawing, 'what the cancelled run recorded to draw', waitMs);
    await pause(1500);
    const partial = await shoot('16-cancelled-recording', 'what the cancelled run recorded, opened and drawn', {
      run: stopped,
      phase: ['attached', 'ended'],
      backend: 'webgpu',
      cursorMs: [0, 0],
      drew: 'movers',
      frames: 1,
    });
    const recorded = Number(String(partial.window).split('-').at(-1));
    if (!(recorded > 0 && recorded < 120 * 60_000)) {
      throw new Error(`the cancelled run's recording reaches ${recorded} ms, which is not part of a 120-minute run`);
    }

    // ── a spec and its own files, chosen the way a person chooses them ───────
    const own = await submitFromDisk('minimal_putaway', [
      'sim/dessim/tests/fixtures/minimal_putaway_spec.json',
      'sim/dessim/tests/fixtures/minimal_putaway.json',
    ]);
    await chrome.until(inState(own, 'finished'), 'the run from picked files to finish', waitMs);
    await chrome.until(`${watching} === ${JSON.stringify(own)}`, 'its attachment', waitMs);
    await chrome.until(drawing, 'its floor to draw', waitMs);
    await pause(1500);
    await shoot('17-spec-from-disk', `a spec and its world document picked from this machine (${own}), run and drawn`, {
      run: own,
      phase: ['attached', 'ended'],
      backend: 'webgpu',
      drew: 'movers',
    });

    timings.push(['cencora', cencoraTimes], ['walmart_dfw6_smoke', dfw6Times]);
    console.log(`\nthe run this drove: ${run}`);
  }

  // One last look for anything the page complained about after the last shot.
  const late = chrome.complaints(seen);
  if (late.length > 0) {
    throw new Error(`after the last step the page said:\n    - ${late.map((one) => `${one.level}: ${one.text}`).join('\n    - ')}`);
  }

  await writeFile(
    join(shots, `RESULTS-${backend}.md`),
    [
      `# studio, driven headless (${backend})`,
      '',
      `UI at ${ui}; Chrome headless at 1500×950; every step driven through the page's own controls, and`,
      'every step asserted on the status line before its picture was taken.',
      '',
      '| shot | what it shows | status line |',
      '|---|---|---|',
      ...taken.map(
        ({ name, shows, said }) =>
          `| \`${name}.png\` | ${shows} | ${said.phase ?? '-'} · ${said.backend ?? '-'} · ${said.frames ?? 0} frames · cursor ${said.cursorMs ?? '-'} ms · drew ${said.drawn ?? '-'} · window ${said.window || '-'} |`
      ),
      ...(timings.length === 0
        ? []
        : [
            '',
            `## What each floor cost to draw, over ${TIMED_FRAMES} frames of playback`,
            '',
            'Measured by the page itself: the gap from one drawn frame to the next, which is what a person sees,',
            "and the part of that gap spent inside the viewer's own draw call.",
            '',
            'Read the DRAW column here, not the gap. A headless Chrome times its animation frames off a clock it',
            'drives itself, so the gap stays at the 16.7 ms of a 60 Hz display however long the draw takes; on a',
            'real display a draw above 16.7 ms is what sets the frame rate.',
            '',
            '| floor | median gap | worst gap | median draw | worst draw |',
            '|---|---|---|---|---|',
            ...timings.map(
              ([floor, one]) =>
                `| ${floor} | ${one.frame} ms | ${one.worstFrame} ms | ${one.draw} ms | ${one.worstDraw} ms |`
            ),
          ]),
      '',
    ].join('\n')
  );
  console.log(`\nwrote ${taken.length} screenshots and RESULTS-${backend}.md into ${shots}`);
  await chrome.close();
  process.exit(0);
} catch (failure) {
  console.error(`\nui_gate.mjs failed: ${failure.message}`);
  console.error(
    chrome.console
      .slice(-20)
      .map((line) => `  ${line.level}: ${line.text}`)
      .join('\n')
  );
  await chrome.screenshot(join(shots, `FAILED-${backend}.png`)).catch(() => undefined);
  await chrome.close();
  process.exit(1);
}
