# studio

Agent Studio's browser shell (design/specs/02-shell-and-chat.md and 03-panel-protocol-and-map.md in the Agent
Studio notes): the app bar, the chat column beside the panel a route shows, the tab row, the context bar, and the one
live region a screen reader hears. A site's conversation is bound to the site (ruling D94): a route of a site shows
its live conversation, the template routes the person's new-site thread, and Home none.

- `src/shell/` — the frame and the address: spec 03's route table and grammar (`routes.ts`, `grammar.ts`), the
  app bar and Settings, Home, the context bar, the tab row, the panel host and each panel's placeholder module,
  which names the task that builds it.
- `src/chat/` — the stream client, the log drawn as a pure function of the journal, the cards, the composer, the
  announcer and its speech rules, and the panel commands a turn's journal carries.
- `src/protocol/` — generated from `studio/protocol` by its `ts_bindings` test; nothing here hand-copies a
  shape the service sends.
- `src/styles/` — the design's `tokens.css` and `studio.css`, copied unchanged.

The Crucible page (the run list, the submit form, playback and the uPlot charts) left this folder when the shell
landed; `git show 6abdd377d:studio/src/components/Charts.tsx` reads it back for the Runs panel's tasks. The
`crucible-viewer` alias and uPlot stay for the tasks that mount the viewer and draw the charts again.

## Running it

`dev.sh` brings up Agent Studio's local stack: Postgres, world-proto-server built from `../world-proto` and
seeded, the API (`studio/api`) and this dev server, which hands `/api` to the API. `dev.sh --help` lists
`--status`, `--logs` and `--stop`.

## Gates

```
bun run typecheck      # tsc -b
bun run lint           # eslint
bun run build          # tsc -b && vite build
bun run test           # Vitest with jsdom: the unit statements of specs 02 and 03
bun run test:browser   # Playwright and axe against a running dev.sh
```

`test:browser` reads the stack's address from `studio/.dev-state/ports` (or `STUDIO_URL`), replays the journal fixtures of
`studio/protocol/tests/journal/` through the API's local test writer, and runs Chromium headless from
Playwright's cache; it never downloads a browser.
