// How the shell is served, and how it finds the two things that live outside
// its folder: the viewer package and the repository's gate fixtures.
//
// `crucible-viewer` is aliased to the package's SOURCE rather than its
// `dist/`, so a change in the package reaches the page without a second build
// step; `build-wasm.sh` still has to have run, because the module the package
// loads is the one that build writes. `fs.allow` opens the repository to the
// dev server for the same reason — the wasm module and the four fixtures both
// sit outside this folder.
//
// The websocket is proxied rather than opened straight at the server so that
// the page has ONE origin: `dev.sh` puts the server's port in
// `CRUCIBLE_SERVER_PORT`, and a page served from here then needs no
// configuration of its own. `VITE_CRUCIBLE_WS` names the server outright for a
// page served some other way. `/api` reaches the Agent Studio API the same way,
// as the login gate will hand it on in the cluster: `dev.sh` puts the
// API's port in `STUDIO_API_PORT`.

import { fileURLToPath } from 'node:url';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

const here = fileURLToPath(new URL('.', import.meta.url));
const repository = fileURLToPath(new URL('../..', import.meta.url));
const serverPort = process.env.CRUCIBLE_SERVER_PORT ?? '20731';
const apiPort = process.env.STUDIO_API_PORT ?? '3100';
const uiPort = Number(process.env.CRUCIBLE_UI_PORT ?? 5173);

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { 'crucible-viewer': `${here}../../studio/viewer/js/src/index.ts` },
  },
  server: {
    port: uiPort,
    strictPort: true,
    fs: { allow: [repository] },
    proxy: {
      '/ws': { target: `ws://127.0.0.1:${serverPort}`, ws: true },
      '/api': { target: `http://127.0.0.1:${apiPort}` },
    },
  },
});
