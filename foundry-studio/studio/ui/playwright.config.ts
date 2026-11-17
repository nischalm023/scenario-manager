// The browser tests (`bun run test:browser`): Playwright with @axe-core/playwright against a running dev.sh,
// headless Chromium from Playwright's own cache. @playwright/test stays pinned at 1.62.1, whose Chromium (1234) is the
// one cached on this machine, so nothing is ever downloaded. The stack's address comes from `STUDIO_URL`, else from
// dev.sh's `.dev-state/ports`; the API's test writer is reached through the same origin.

import { readFileSync } from 'node:fs';
import { defineConfig, devices } from '@playwright/test';

function studioUrl(): string {
  if (process.env.STUDIO_URL) return process.env.STUDIO_URL;
  try {
    const ports = readFileSync(new URL('../.dev-state/ports', import.meta.url), 'utf8');
    const port = /^UI_PORT=(\d+)$/m.exec(ports)?.[1];
    if (port) return `http://127.0.0.1:${port}`;
  } catch {
    // No dev.sh state: the run fails below with a sentence saying how to start the stack.
  }
  throw new Error('No Studio stack: start dev.sh, or set STUDIO_URL to a running one.');
}

export default defineConfig({
  testDir: './tests/browser',
  fullyParallel: false,
  workers: 1,
  timeout: 120_000,
  expect: { timeout: 10_000 },
  reporter: [['list']],
  use: {
    ...devices['Desktop Chrome'],
    baseURL: studioUrl(),
    headless: true,
    trace: 'off',
  },
});
