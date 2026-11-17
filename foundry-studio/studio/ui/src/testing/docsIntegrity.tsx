// Test support for the Docs section's integrity tests, over the repository's own documents (`docs/docs.test.tsx`) and
// over small broken ones (`docs/integrity.test.tsx`): every way the served documents (`docs/manifest.ts`) fail to reach
// what they name, in words. The checks read what `DocsSection` draws, never the Markdown a second time. An address in
// the app must name a served document and a heading drawn on it. An address on GitHub, read as the browser resolves it,
// must name a path git tracks, exactly as git spells it, and a place GitHub's page of that file has: one of its lines,
// or a Markdown file's heading.

import { render } from '@testing-library/react';
import type { Nodes } from 'mdast';
import { MemoryRouter } from 'react-router';
import remarkGfm from 'remark-gfm';
import remarkParse from 'remark-parse';
import { unified } from 'unified';
import { decoded, pageAt, REPOSITORY, routeOf } from '../docs/addresses.ts';
import { DocsSection } from '../docs/DocsSection.tsx';
import { headingIds } from '../docs/headings.ts';
import { FIGURES, PAGES } from '../docs/manifest.ts';
import { ShellContext } from '../shell/context.ts';
import { parseRoute } from '../shell/routes.ts';
import { shellOf } from './render.tsx';

/** What the checks read of the repository besides the served documents. */
export interface Repository {
  /** Every path git tracks, as git spells it. */
  tracked: ReadonlySet<string>;
  /** The text of a tracked file. */
  read(path: string): string;
}

/** A line of a file as GitHub's page of it names one, `L12`, or the lines from one to another, `L12-L20`. */
const LINES = /^L(\d+)(?:-L(\d+))?$/;

const slugged = unified().use(remarkParse).use(remarkGfm).use(headingIds);

/** What `DocsSection` draws of a served document: heading ids, link and image addresses, words drawn unresolved. */
function drawnOf(page: string) {
  const { container, unmount } = render(
    <MemoryRouter initialEntries={[routeOf(page)]}>
      <ShellContext.Provider value={shellOf()}>
        <DocsSection doc={page.replace(/\.md$/, '')} />
      </ShellContext.Provider>
    </MemoryRouter>,
  );
  const all = (selector: string) => [...(container.querySelector('article.doc')?.querySelectorAll(selector) ?? [])];
  const drawn = {
    ids: new Set(all('[id]').filter((element) => /^H[1-6]$/.test(element.tagName)).map((element) => element.id)),
    links: all('a[href]').map((anchor) => anchor.getAttribute('href') ?? ''),
    images: all('img').map((image) => image.getAttribute('src') ?? ''),
    unresolved: all('.doc__unresolved').map((element) => element.textContent ?? ''),
  };
  unmount();
  return drawn;
}

/** `address` split at its first `#`: what precedes it, and its fragment decoded as `DocPage` decodes it, or null. */
function fragmentOf(address: string): [string, string | null] {
  const hash = address.indexOf('#');
  return hash < 0 ? [address, null] : [address.slice(0, hash), decoded(address.slice(hash + 1)) ?? address.slice(hash + 1)];
}

/** Why the app's address `href`, drawn on `page`, reaches no heading drawn in the app; null when it reaches one. */
function appMiss(drawn: ReadonlyMap<string, { ids: Set<string> }>, page: string, href: string): string | null {
  const [located, fragment] = fragmentOf(href);
  const route = located === '' ? null : parseRoute(located);
  const target = route === null ? page : route.view === 'docs' ? pageAt(route.doc) : '';
  if (!drawn.has(target)) return 'names no served document';
  if (fragment === null || drawn.get(target)?.ids.has(fragment)) return null;
  return target === page ? 'names no heading drawn on it' : `names no heading drawn on ${target}`;
}

/** The ids GitHub gives the headings of a Markdown file the app does not serve, which no page of the section draws. */
function idsOf(text: string): Set<string> {
  const ids = new Set<string>();
  const visit = (node: Nodes): void => {
    const id = node.type === 'heading' ? node.data?.hProperties?.id : undefined;
    if (typeof id === 'string') ids.add(id);
    if ('children' in node) node.children.forEach(visit);
  };
  visit(slugged.runSync(slugged.parse(text)));
  return ids;
}

/** How many lines GitHub's page of a file numbers: a last line's newline begins no line of its own. */
const lineCount = (text: string): number => text.split('\n').length - (text === '' || text.endsWith('\n') ? 1 : 0);

/**
 * Why the GitHub address `href` reaches nothing; null when it names a tracked path and a place on GitHub's page of it.
 * The address is read as the browser resolves it, `..` and `.` taken out, and a served document's headings are those
 * `drawn` holds of it.
 */
function repositoryMiss(
  repository: Repository,
  folders: ReadonlySet<string>,
  drawn: ReadonlyMap<string, { ids: Set<string> }>,
  href: string,
): string | null {
  const resolved = new URL(href).href;
  const [located, fragment] = fragmentOf(resolved.slice(REPOSITORY.length));
  const path = resolved.startsWith(REPOSITORY) ? decoded(located.split('?')[0]) : null;
  if (path === null || (!repository.tracked.has(path) && !folders.has(path))) return 'names no file or folder git tracks';
  if (fragment === null) return null;
  const file = repository.tracked.has(path) ? repository.read(path) : null;
  const lines = LINES.exec(fragment);
  if (lines !== null) {
    const [first, last] = [Number(lines[1]), Number(lines[2] ?? lines[1])];
    return file !== null && first >= 1 && first <= last && last <= lineCount(file) ? null : `names lines ${path} does not have`;
  }
  if (file !== null && path.endsWith('.md')) return (drawn.get(path)?.ids ?? idsOf(file)).has(fragment) ? null : `names no heading of ${path}`;
  return `names a fragment other than a line of ${path}`;
}

/** Every way the served documents fail to reach what they name; empty when every link and image resolves. */
export function integrityMisses(repository: Repository): string[] {
  const folders = new Set<string>();
  for (const path of repository.tracked) {
    const names = path.split('/');
    for (let end = 1; end < names.length; end += 1) folders.add(names.slice(0, end).join('/'));
  }
  const drawn = new Map([...PAGES.keys()].map((page) => [page, drawnOf(page)]));
  const misses: string[] = [];
  for (const [page, { links, unresolved }] of drawn) {
    for (const words of unresolved) misses.push(`${page}: "${words}" reaches nothing`);
    for (const href of links) {
      // A web address, one beginning `//` with its host among them, is the web's to answer for.
      const inApp = href.startsWith('#') || (href.startsWith('/') && !href.startsWith('//'));
      const why = href.startsWith(REPOSITORY) ? repositoryMiss(repository, folders, drawn, href) : inApp ? appMiss(drawn, page, href) : null;
      if (why !== null) misses.push(`${page}: ${href} ${why}`);
    }
  }
  const shown = new Set([...drawn.values()].flatMap(({ images }) => images.map((src) => src.split('#')[0])));
  for (const [path, url] of FIGURES) {
    if (!shown.has(url)) misses.push(`${path}: the figure is shown by no document`);
  }
  return misses;
}
