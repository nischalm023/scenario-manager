// Photograph the viewer's own test page once it has actually drawn.
//
// `studio/viewer/test/index.html` attaches to a run, pushes every frame
// into the viewer and draws; it puts its report on the window once a frame has
// submitted something, and says how far it has got in the line under the
// canvas. This waits for the first and prints the second.
//
//   node shoot.mjs --url <page url> --out <file.png>
//                  [--width 1240] [--height 900] [--wait 60000]
//                  [--chrome <binary>] [--no-webgpu]
//                  [--expect-cursor <ms>] [--expect-window <from>-<to>]
//                  [--expect-movers]
//
// The `--expect-` checks are what make this a gate rather than a photograph:
// the page reports where its cursor stands, how much of the run it holds and
// how many rows the last frame submitted, and a run that drew the wrong
// instant or nothing at all fails here instead of producing a picture nobody
// looks at closely.
//
// `--no-webgpu` photographs the WebGL2 fallback; see `headless_chrome.mjs`.

import { asked, openChrome, pause } from './headless_chrome.mjs';

const url = asked('url');
const out = asked('out');
if (!url || !out) {
  console.error('shoot.mjs: --url and --out are both needed');
  process.exit(2);
}
const waitMs = Number(asked('wait', 60_000));

const chrome = await openChrome({
  width: Number(asked('width', 1240)),
  height: Number(asked('height', 900)),
  webgpu: !process.argv.includes('--no-webgpu'),
  chrome: asked('chrome'),
  deadline: Date.now() + waitMs,
});

await chrome.navigate(url);

let said = '';
let report = null;
const stop = Date.now() + waitMs;
while (Date.now() < stop) {
  await pause(250);
  const now = await chrome.evaluate("document.getElementById('said')?.textContent ?? ''");
  if (now && now !== said) {
    said = now;
    console.log('  said:', said);
  }
  report = await chrome.evaluate('window.viewerReport ?? null');
  if (report || said.startsWith('FAILED')) break;
}

await chrome.screenshot(out);
chrome.close();

const faults = [];
if (said.startsWith('FAILED')) faults.push(said);
if (!report) faults.push('the page never drew anything');
if (report) {
  const [robots, containers, cells] = report.drawn;
  if (cells === 0) faults.push('no cells were submitted, so the canvas holds no floor');
  if (process.argv.includes('--expect-movers') && (robots === 0 || containers === 0)) {
    faults.push(`expected robots and containers, drew ${robots} and ${containers}`);
  }
  const cursor = asked('expect-cursor');
  if (cursor !== undefined && Number(cursor) !== report.cursor) {
    faults.push(`expected the cursor at ${cursor} ms, it stands at ${report.cursor}`);
  }
  const window = asked('expect-window');
  if (window !== undefined && window !== report.window.join('-')) {
    faults.push(`expected the window ${window} ms, it holds ${report.window.join('-')}`);
  }
}

console.log(`${said}\nwrote ${out}`);
for (const fault of faults) console.error(`shoot.mjs: ${fault}`);
process.exit(faults.length === 0 ? 0 : 1);
