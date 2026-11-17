// One served document as `DocPage` draws it, on small documents: a document without a `#` heading opens with its file
// name as its title; a code block and a figure scroll in focusable named regions; a link to another document goes
// through the shell, and only a link that opens a new browser tab says so; inline code breaks between a path's
// folders.

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ShellContext } from '../shell/context.ts';
import { shellOf } from '../testing/render.tsx';
import { DocPage } from './DocPage.tsx';

vi.mock('./manifest.ts', () => ({
  PAGES: new Map([
    ['docs/untitled.md', 'Text first.\n\n## A section\n\n### A part'],
    ['docs/parts.md', [
      '# Parts',
      '```rust\nfn main() {}\n```',
      '```\nplain\n```',
      '![A drawing](drawing.svg "The drawing")',
      '![An untitled drawing](drawing.svg)',
    ].join('\n\n')],
    ['docs/links.md', [
      '# Links',
      '[Parts](parts.md), [mail](mailto:someone@example.com), [the web](https://example.com/), [code](../src/main.rs)',
    ].join('\n\n')],
    ['docs/paths.md', '# Paths\n\n`/api`, `GET /api/runs`, `studio/ui/src`'],
  ]),
  FIGURES: new Map([['docs/drawing.svg', '/assets/drawing-1a2b.svg']]),
}));

afterEach(cleanup);

function drawn(page: string, navigate = vi.fn()) {
  render(
    <MemoryRouter>
      <ShellContext.Provider value={{ ...shellOf(), navigate }}>
        <DocPage page={page} />
      </ShellContext.Provider>
    </MemoryRouter>,
  );
  return navigate;
}

const levels = () => [...document.querySelectorAll('article.doc :is(h1, h2, h3, h4, h5, h6)')].map((heading) => `${heading.tagName} ${heading.textContent}`);

describe('a served document', () => {
  it('without a `#` heading opens with its file name as its title, the panel\'s h2 above it and its own headings below', () => {
    drawn('docs/untitled.md');
    expect(levels()).toEqual(['H3 untitled', 'H4 A section', 'H5 A part']);
  });

  it('with one keeps its own as its title', () => {
    drawn('docs/parts.md');
    expect(levels()).toEqual(['H3 Parts']);
  });

  it('draws each code block in a focusable region named by its language, and each figure in one named by its title', () => {
    drawn('docs/parts.md');
    for (const name of ['A rust code block', 'A code block', 'The drawing, figure', 'A figure in this document']) {
      expect(screen.getByRole('region', { name }).tabIndex, name).toBe(0);
    }
  });

  it('opens another served document through the shell, and says a new browser tab opens only where one does', () => {
    const navigate = drawn('docs/links.md');
    fireEvent.click(screen.getByRole('link', { name: 'Parts' }));
    expect(navigate).toHaveBeenCalledWith('/docs/docs/parts', 'user');
    const mail = screen.getByRole('link', { name: 'mail' });
    expect([mail.getAttribute('target'), mail.getAttribute('href')]).toEqual([null, 'mailto:someone@example.com']);
    expect(mail.textContent).toBe('mail');
    for (const name of [/^the web/, /^code/]) {
      const link = screen.getByRole('link', { name });
      expect([link.getAttribute('target'), link.textContent], String(name)).toEqual(['_blank', expect.stringMatching(/ \(opens a new browser tab\)$/)]);
    }
  });

  it('lets inline code break after a slash between folders, never after the slash that begins a path', () => {
    drawn('docs/paths.md');
    const codes = [...document.querySelectorAll('article.doc > p:not(.doc__source) code')];
    expect(codes.map((code) => code.innerHTML.replaceAll('<wbr>', '|'))).toEqual(['/api', 'GET /api/|runs', 'studio/|ui/|src']);
  });
});
