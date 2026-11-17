// The Docs section over the repository's real documents (`manifest.ts`): what the manifest serves and leaves out, each
// served document drawn with its title, every link and image in them reaching what it names
// (`testing/docsIntegrity.tsx`: a heading drawn, a file git tracks and a place on GitHub's page of it, a bundled figure
// drawn), and each bundled figure shown by some document. A document, heading or figure renamed or moved without its
// links fails here.

import { cleanup, render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ShellContext } from '../shell/context.ts';
import { integrityMisses } from '../testing/docsIntegrity.tsx';
import { shellOf } from '../testing/render.tsx';
import { DOCS_ROUTE, routeOf } from './addresses.ts';
import { DocsSection } from './DocsSection.tsx';
import { titleOf } from './headings.ts';
import { PAGES } from './manifest.ts';

// Node's modules through `importActual`, since the app's type program declares none.
const { readFileSync } = await vi.importActual<{ readFileSync(path: string, encoding: 'utf8'): string }>('node:fs');
const { execFileSync } = await vi.importActual<{
  execFileSync(file: string, args: string[], options: { cwd: string; encoding: 'utf8'; maxBuffer: number }): string;
}>('node:child_process');
/** The repository's root: what precedes this folder. */
const ROOT = decodeURIComponent(new URL(import.meta.url).pathname).split('studio/ui/src/docs/')[0];

afterEach(cleanup);

describe('the manifest', () => {
  it('serves docs/architecture/ without the retired Java DES\'s notes, and sim/dessim/docs/ with its subfolders', () => {
    const pages = [...PAGES.keys()];
    expect(pages).toContain('docs/architecture/live-frames.md');
    expect(pages).toContain('docs/designs/worldview-format.md');
    expect(pages).toContain('sim/dessim/docs/domain/htm-dispatch.md');
    expect(pages.filter((page) => /^docs\/architecture\/(c4-|fleet-simulator-|microservices-|polyglot-|service-communication-)/.test(page))).toEqual([]);
  });
});

describe('every served document', () => {
  it.each([...PAGES.keys()])('%s draws with its title as its first heading', (page) => {
    const { container } = render(
      <MemoryRouter initialEntries={[routeOf(page)]}>
        <ShellContext.Provider value={shellOf()}>
          <DocsSection doc={page.replace(/\.md$/, '')} />
        </ShellContext.Provider>
      </MemoryRouter>,
    );
    const first = container.querySelector('article.doc h3, article.doc h4, article.doc h5, article.doc h6');
    expect([first?.tagName, first?.textContent]).toEqual(['H3', titleOf(page, PAGES.get(page) ?? '')]);
  });

  it('reach every heading, file and figure they name, and together show every figure the build bundles', () => {
    const tracked = new Set(execFileSync('git', ['ls-files', '-z'], { cwd: ROOT, encoding: 'utf8', maxBuffer: 64 << 20 }).split('\0').filter((path) => path !== ''));
    expect(integrityMisses({ tracked, read: (path) => readFileSync(`${ROOT}${path}`, 'utf8') })).toEqual([]);
  });
});

describe('the list of documents', () => {
  it('names each served document by its title, grouped under its folder, linking to its route', () => {
    render(
      <MemoryRouter initialEntries={[DOCS_ROUTE]}>
        <ShellContext.Provider value={shellOf()}>
          <DocsSection doc="" />
        </ShellContext.Provider>
      </MemoryRouter>,
    );
    const architecture = screen.getByRole('region', { name: 'docs/architecture/' });
    const link = [...architecture.querySelectorAll('a')].find((anchor) => anchor.textContent === 'How a run\'s picture reaches the browser');
    expect(link?.getAttribute('href')).toBe('/docs/docs/architecture/live-frames');
    expect(screen.getByRole('heading', { level: 3, name: 'sim/dessim/docs/domain/' })).not.toBeNull();
  });

  it('says so for a route that names no served document', () => {
    render(
      <MemoryRouter initialEntries={['/docs/docs/architecture/fleet-simulator-engine']}>
        <ShellContext.Provider value={shellOf()}>
          <DocsSection doc="docs/architecture/fleet-simulator-engine" />
        </ShellContext.Provider>
      </MemoryRouter>,
    );
    expect(screen.getByText('docs/architecture/fleet-simulator-engine.md').closest('p')?.textContent)
      .toBe('No document is served at docs/architecture/fleet-simulator-engine.md.');
    expect(screen.getByRole('link', { name: 'All documents' }).getAttribute('href')).toBe('/docs');
  });
});
