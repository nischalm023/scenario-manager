// The rules a reference in a served document follows (`addresses.ts`): a document's route, where a link goes and
// which file an image is, each read as GitHub reads the same Markdown.

import { describe, expect, it } from 'vitest';
import { parseRoute } from '../shell/routes.ts';
import { REPOSITORY, hrefOf, imageUrl, pageAt, resolveLink, routeOf, sourceOf } from './addresses.ts';

const FROM = 'docs/architecture/live-frames.md';
const SERVED = new Set([FROM, 'docs/designs/worldview-format.md', 'sim/dessim/docs/global-barrier.md']);
const served = (path: string) => SERVED.has(path);

describe('a document\'s route', () => {
  it('is its repository path without `.md` under /docs, and reads back to the path', () => {
    expect(routeOf(FROM)).toBe('/docs/docs/architecture/live-frames');
    expect(pageAt('docs/architecture/live-frames')).toBe(FROM);
  });

  it('carries a `#`, a `?` and a `%20` in a file\'s name from the link, through the shell\'s reading, to the document', () => {
    const page = 'docs/architecture/phase#2 a?b%20c.md';
    const target = resolveLink('phase%232%20a%3Fb%2520c.md#a-heading', FROM, (path) => path === page);
    expect(target).toEqual({ kind: 'page', page, fragment: 'a-heading' });
    const href = hrefOf(target);
    expect(href).toBe('/docs/docs/architecture/phase%232%20a%3Fb%2520c#a-heading');
    const route = parseRoute(href ?? '');
    expect(route).toEqual({ view: 'docs', doc: 'docs/architecture/phase#2 a?b%20c' });
    expect(route.view === 'docs' && pageAt(route.doc)).toBe(page);
  });

  it('names its file on GitHub with the same escapes', () => {
    expect(sourceOf('docs/architecture/phase#2.md')).toBe(`${REPOSITORY}docs/architecture/phase%232.md`);
  });
});

describe('a link', () => {
  it('to a served document, read against the linking document\'s folder, opens that document here at its heading', () => {
    const target = resolveLink('../designs/worldview-format.md#4-slices', FROM, served);
    expect(target).toEqual({ kind: 'page', page: 'docs/designs/worldview-format.md', fragment: '4-slices' });
    expect(hrefOf(target)).toBe('/docs/docs/designs/worldview-format#4-slices');
    expect(resolveLink('../../sim/dessim/docs/global-barrier.md', FROM, served))
      .toEqual({ kind: 'page', page: 'sim/dessim/docs/global-barrier.md', fragment: null });
  });

  it('beginning with one `/` is read from the repository\'s root, as GitHub reads it', () => {
    expect(resolveLink('/sim/dessim/docs/global-barrier.md#consumers', FROM, served))
      .toEqual({ kind: 'page', page: 'sim/dessim/docs/global-barrier.md', fragment: 'consumers' });
    expect(hrefOf(resolveLink('/studio/streaming/README.md', FROM, served))).toBe(`${REPOSITORY}studio/streaming/README.md`);
  });

  it('to a heading of the same document by its fragment alone stays on the page; by the file\'s name too, opens it there', () => {
    const target = resolveLink('#why-a-frame-never-changes-once-sent', FROM, served);
    expect(target).toEqual({ kind: 'heading', fragment: 'why-a-frame-never-changes-once-sent' });
    expect(hrefOf(target)).toBe('#why-a-frame-never-changes-once-sent');
    const named = resolveLink('live-frames.md#why-a-frame-never-changes-once-sent', FROM, served);
    expect(named).toEqual({ kind: 'page', page: FROM, fragment: 'why-a-frame-never-changes-once-sent' });
    expect(hrefOf(named)).toBe('/docs/docs/architecture/live-frames#why-a-frame-never-changes-once-sent');
  });

  it('splits its query and fragment from its path before decoding, so neither is read as part of a file\'s name', () => {
    expect(resolveLink('../designs/worldview-format.md?plain=1#4-slices', FROM, served))
      .toEqual({ kind: 'page', page: 'docs/designs/worldview-format.md', fragment: '4-slices' });
    const source = resolveLink('../../studio/streaming/src/attach.rs?plain=1#L96', FROM, served);
    expect(source).toEqual({ kind: 'repository', path: 'studio/streaming/src/attach.rs', query: '?plain=1', fragment: 'L96' });
    expect(hrefOf(source)).toBe(`${REPOSITORY}studio/streaming/src/attach.rs?plain=1#L96`);
  });

  it('to a repository file or folder the app does not serve opens it on GitHub', () => {
    expect(hrefOf(resolveLink('../src/model/parking.rs', 'sim/dessim/docs/temporal-state.md', served)))
      .toBe(`${REPOSITORY}sim/dessim/src/model/parking.rs`);
    expect(hrefOf(resolveLink('./../../sim/dessim/src/model/relay/', FROM, served))).toBe(`${REPOSITORY}sim/dessim/src/model/relay`);
  });

  it('decodes its escapes once, so a path with a space finds its document', () => {
    const spaced = new Set(['docs/architecture/two words.md']);
    expect(resolveLink('two%20words.md#a%20b', FROM, (path) => spaced.has(path)))
      .toEqual({ kind: 'page', page: 'docs/architecture/two words.md', fragment: 'a b' });
  });

  it('to the web stays as written, a protocol-relative `//` address among them', () => {
    for (const url of ['https://example.com/a?b#c', 'mailto:someone@example.com', '//example.com/a']) {
      expect(resolveLink(url, FROM, served), url).toEqual({ kind: 'web', url });
    }
  });

  it('that leaves the repository, opens a scheme the app does not, or does not decode, resolves nowhere', () => {
    for (const href of ['../../../x.md', '/../x.md', 'javascript:alert(1)', '%E0%A4%A.md', 'x.md#%E0%A4%A', '']) {
      const target = resolveLink(href, FROM, served);
      expect(target.kind, href).toBe('unresolved');
      expect(hrefOf(target), href).toBeNull();
    }
  });
});

describe('an image', () => {
  const figures = new Map([['docs/architecture/live-frames/life-of-a-frame.svg', '/assets/life-of-a-frame-1a2b.svg']]);

  it('is the file the build bundled, read against the document\'s folder or from the repository\'s root', () => {
    for (const src of ['live-frames/life-of-a-frame.svg', './live-frames/../live-frames/life-of-a-frame.svg', '/docs/architecture/live-frames/life-of-a-frame.svg']) {
      expect(imageUrl(src, FROM, figures), src).toBe('/assets/life-of-a-frame-1a2b.svg');
    }
  });

  it('is found by its path whatever its query, and keeps its fragment, the view of an SVG it names', () => {
    expect(imageUrl('live-frames/life-of-a-frame.svg?raw=true', FROM, figures)).toBe('/assets/life-of-a-frame-1a2b.svg');
    expect(imageUrl('live-frames/life-of-a-frame.svg#overview', FROM, figures)).toBe('/assets/life-of-a-frame-1a2b.svg#overview');
  });

  it('the build did not bundle, or from the web, has no address', () => {
    for (const src of ['live-frames/gone.svg', 'https://example.com/a.svg', '//example.com/a.svg', '']) {
      expect(imageUrl(src, FROM, figures), src).toBeNull();
    }
  });
});
