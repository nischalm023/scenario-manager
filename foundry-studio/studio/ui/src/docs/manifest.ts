// The Docs section's manifest: which of the repository's own Markdown documents the app serves, and the figures they
// show, read when the app is built. The documents stay where the repository keeps them, so a pull request that changes
// a mechanism updates its document and GitHub renders it too; the app serves the same files behind its own access
// control. A folder named here is served whole and a file one by one; nothing outside these patterns is read.
//
// docs/architecture/ also holds the retired Java DES's design notes (the fleet simulator on Desmo-J, its services and
// their Cloud Run deployment), which describe no code in this repository; they are left out by name until they are
// deleted or rewritten.

/** What every key the globs below answer starts with: the way from this folder up to the repository's root. */
const UP = '../../../../';

const texts = import.meta.glob<string>(
  [
    '../../../../docs/architecture/**/*.md',
    '!../../../../docs/architecture/c4-*.md',
    '!../../../../docs/architecture/fleet-simulator-*.md',
    '!../../../../docs/architecture/microservices-decomposition-design.md',
    '!../../../../docs/architecture/polyglot-monorepo-structure-design.md',
    '!../../../../docs/architecture/service-communication-patterns.md',
    '../../../../docs/compositions.md',
    '../../../../docs/designs/worldview-format.md',
    '../../../../docs/designs/worldview-render-dump.md',
    '../../../../sim/dessim/docs/**/*.md',
  ],
  { query: '?raw', import: 'default', eager: true },
);

const urls = import.meta.glob<string>(
  ['../../../../docs/architecture/**/*.{svg,png}', '../../../../sim/dessim/docs/**/*.{svg,png}'],
  { query: '?url', import: 'default', eager: true },
);

function byPath<T>(found: Record<string, T>): Map<string, T> {
  const entries = Object.entries(found).map(([key, value]): [string, T] => [key.slice(UP.length), value]);
  return new Map(entries.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
}

/** Each served document's Markdown, by its path in the repository, in path order. */
export const PAGES: ReadonlyMap<string, string> = byPath(texts);

/** Each figure the served documents may show, by its path in the repository: the URL the built app serves it at. */
export const FIGURES: ReadonlyMap<string, string> = byPath(urls);
