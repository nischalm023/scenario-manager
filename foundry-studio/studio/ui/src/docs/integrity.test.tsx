// The integrity check of `testing/docsIntegrity.tsx` on small documents broken on purpose: each failure the Docs
// section's link test promises to catch is caught, read from what the section draws: a reference takes its first
// definition, as the section draws it, and a figure shown by a reference-style image, a view of it among them, counts
// as shown. A GitHub address is read as the browser resolves it, and its fragment must be one of the file's lines, or a
// heading of a Markdown file: for a served document, a heading its page draws.

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup } from '@testing-library/react';
import { integrityMisses } from '../testing/docsIntegrity.tsx';
import { REPOSITORY as GITHUB } from './addresses.ts';

vi.mock('./manifest.ts', () => ({
  PAGES: new Map([
    ['docs/a.md', [
      '# A',
      '## Here',
      '[here](#here), [itself by name](a.md#here), [b](b.md#there), [notes](../notes/c.md#c-heading), [the web](//example.com/c)',
      '[a line](../src/main.rs#L3), [lines](../src/main.rs#L2-L3), [a line of notes](../notes/c.md#L1)',
      '[a dead file](gone.md), [a wrongly cased file](../src/Main.rs), [a dead heading here](#nowhere)',
      '[a dead heading of a served document](b.md#nowhere), [a dead heading of another file](../notes/c.md#nowhere)',
      '[no line 0](../src/main.rs#L0), [past the last line](../src/main.rs#L4), [lines backwards](../src/main.rs#L3-L2)',
      '[a name in source](../src/main.rs#main), [an id an HTML page holds](../notes/page.html#an-id)',
      '[a link the app does not open](javascript:void(0)), [the first of two definitions][twice]',
      '[a heading b draws, on GitHub](https://github.com/greyorange/foundry-studio/blob/main/docs/b.md#there)',
      '[a line on GitHub through ..](https://github.com/greyorange/foundry-studio/blob/main/docs/../src/main.rs#L1)',
      '[a file on another branch through ..](https://github.com/greyorange/foundry-studio/blob/main/../dev1/src/main.rs)',
      '![a missing figure](figures/missing.svg)',
      '![a figure by reference][shown], ![a view by reference][view], ![a missing figure by reference][gone]',
      '[twice]: first.md',
      '[twice]: b.md',
      '[shown]: figures/shown.svg',
      '[view]: figures/viewed.svg#overview',
      '[gone]: figures/gone.svg',
    ].join('\n\n')],
    ['docs/b.md', '# B\n\n## There'],
  ]),
  FIGURES: new Map([
    ['docs/figures/shown.svg', '/assets/shown-1a2b.svg'],
    ['docs/figures/viewed.svg', '/assets/viewed-5e6f.svg'],
    ['docs/figures/orphan.svg', '/assets/orphan-3c4d.svg'],
  ]),
}));

const FILES = new Map([
  ['docs/a.md', ''], ['docs/b.md', ''], ['docs/figures/shown.svg', ''], ['docs/figures/viewed.svg', ''], ['docs/figures/orphan.svg', ''],
  ['notes/c.md', '# C heading'], ['notes/page.html', '<p id="an-id">An id</p>'], ['src/main.rs', 'fn main() {\n    run();\n}\n'],
]);

afterEach(cleanup);

describe('the Docs section\'s integrity check', () => {
  it('names each dead link, dead heading or line, wrongly cased path, missing image, inline or by reference, and orphan figure, and nothing else', () => {
    expect(integrityMisses({ tracked: new Set(FILES.keys()), read: (path) => FILES.get(path) ?? '' }).sort()).toEqual([
      'docs/a.md: "a link the app does not open" reaches nothing',
      'docs/a.md: "a missing figure by reference" reaches nothing',
      'docs/a.md: "a missing figure" reaches nothing',
      'docs/a.md: #nowhere names no heading drawn on it',
      'docs/a.md: /docs/docs/b#nowhere names no heading drawn on docs/b.md',
      `docs/a.md: ${GITHUB}../dev1/src/main.rs names no file or folder git tracks`,
      `docs/a.md: ${GITHUB}docs/first.md names no file or folder git tracks`,
      `docs/a.md: ${GITHUB}docs/gone.md names no file or folder git tracks`,
      `docs/a.md: ${GITHUB}notes/c.md#nowhere names no heading of notes/c.md`,
      `docs/a.md: ${GITHUB}notes/page.html#an-id names a fragment other than a line of notes/page.html`,
      `docs/a.md: ${GITHUB}src/Main.rs names no file or folder git tracks`,
      `docs/a.md: ${GITHUB}src/main.rs#L0 names lines src/main.rs does not have`,
      `docs/a.md: ${GITHUB}src/main.rs#L3-L2 names lines src/main.rs does not have`,
      `docs/a.md: ${GITHUB}src/main.rs#L4 names lines src/main.rs does not have`,
      `docs/a.md: ${GITHUB}src/main.rs#main names a fragment other than a line of src/main.rs`,
      'docs/figures/orphan.svg: the figure is shown by no document',
    ]);
  });
});
