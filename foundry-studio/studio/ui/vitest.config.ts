// The unit tests (`bun run test`): Vitest with jsdom (brief I1.05, lead decided), over `src/**/*.test.ts(x)`. The
// browser tests are Playwright's (`bun run test:browser`, playwright.config.ts) and run against dev.sh. The
// viewer package resolves as vite.config.ts resolves it, so a page that loads it on demand imports cleanly here.

import { fileURLToPath } from 'node:url';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

const here = fileURLToPath(new URL('.', import.meta.url));

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { 'crucible-viewer': `${here}../../studio/viewer/js/src/index.ts` },
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.{ts,tsx}'],
  },
});
