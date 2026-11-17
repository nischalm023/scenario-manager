// A served document's headings (`headings.ts`): the ids GitHub gives them, pinned by github-slugger's own fixtures and
// by cases its character rule implies, and a document's title, its own or its file name's.

import type { Heading, Nodes, Root } from 'mdast';
import remarkGfm from 'remark-gfm';
import remarkParse from 'remark-parse';
import { unified } from 'unified';
import { describe, expect, it } from 'vitest';
import { headingIds, titled, titleOf } from './headings.ts';

/** The id each heading of a tree takes, the tree built by hand so a heading's text reaches the slugger untrimmed. */
function idsOf(texts: string[]): unknown[] {
  const tree: Root = { type: 'root', children: texts.map((value): Heading => ({ type: 'heading', depth: 2, children: [{ type: 'text', value }] })) };
  unified().use(headingIds).runSync(tree);
  return tree.children.map((node) => node.data?.hProperties?.id);
}

/** Each heading of a Markdown document, as `DocPage` reads it: its depth, its words and its id. */
function headingsOf(markdown: string, page = 'docs/notes.md'): { depth: number; words: string; id: unknown }[] {
  const processor = unified().use(remarkParse).use(remarkGfm).use(headingIds).use(titled, page);
  const tree = processor.runSync(processor.parse(markdown));
  const found: { depth: number; words: string; id: unknown }[] = [];
  const words = (node: Nodes): string => ('children' in node ? node.children.map(words).join('') : 'value' in node ? node.value : '');
  const walk = (node: Nodes): void => {
    if (node.type === 'heading') found.push({ depth: node.depth, words: words(node), id: node.data?.hProperties?.id });
    if ('children' in node) node.children.forEach(walk);
  };
  walk(tree);
  return found;
}

describe('a heading\'s id', () => {
  it('is github-slugger\'s on fixtures from its own tests (`test/fixtures.json`), in their order, by one slugger', () => {
    const fixtures: [string, string][] = [
      ['alpha', 'alpha'], ['alpha', 'alpha-1'], ['bravoCharlieDelta', 'bravocharliedelta'],
      ['__proto__', '__proto__'], ['__proto__', '__proto__-1'], ['hasOwnProperty', 'hasownproperty'],
      ['echo', 'echo'], ['echo', 'echo-1'], ['echo 1', 'echo-1-1'], ['echo-1', 'echo-1-2'], ['echo', 'echo-2'],
      ['foxtrot-1', 'foxtrot-1'], ['foxtrot', 'foxtrot'], ['foxtrot', 'foxtrot-2'],
      ['heading with a - dash', 'heading-with-a---dash'], ['heading with an _ underscore', 'heading-with-an-_-underscore'],
      ['heading with a period.txt', 'heading-with-a-periodtxt'],
      [' ', '-'], [' a', '-a'], ['a ', 'a-'], [' a ', '-a-'], ['-a-', '-a--1'],
      ['apostrophe’s should be trimmed', 'apostrophes-should-be-trimmed'],
      ['I ♥ unicode', 'i--unicode'], ['dash-dash', 'dash-dash'], ['en–dash', 'endash'], ['em–dash', 'emdash'],
      ['😄 unicode emoji', '-unicode-emoji'], ['😄-😄 unicode emoji', '--unicode-emoji'], ['😄_😄 unicode emoji', '_-unicode-emoji'],
      ['😄 - an emoji', '---an-emoji'], [':smile: - a gemoji', 'smile---a-gemoji'],
      ['Привет', 'привет'], ['Профили пользователей', 'профили-пользователей'], ['Привет non-latin 你好', 'привет-non-latin-你好'],
      [':ok: No underscore', 'ok-no-underscore'], [':ok_hand: Single', 'ok_hand-single'],
      [':ok_hand: :hatched_chick: Two in a row', 'ok_hand-hatched_chick-two-in-a-row'],
      ['a_ ‿ ⁀ ⁔ ︳ ︴ ﹍ ﹎ ﹏ ＿b', 'a_-‿-⁀-⁔-︳-︴-﹍-﹎-﹏-＿b'],
      ['a« ‘ ‛ “ ‟ ‹ ⸂ ⸄ ⸉ ⸌ ⸜ ⸠b', 'a-----------b'],
    ];
    expect(idsOf(fixtures.map(([input]) => input))).toEqual(fixtures.map(([, expected]) => expected));
  });

  it('drops a number other than a digit or a letter-number, and keeps an alphabetic symbol, as its character rule says', () => {
    expect(idsOf(['Area m²', '½ cup', '① first', 'x² + y²', 'Ⅻ Roman', 'Ⓐ circled', 'Café 2026']))
      .toEqual(['area-m', '-cup', '-first', 'x--y', 'ⅻ-roman', 'ⓐ-circled', 'café-2026']);
  });

  it('reads a heading as GitHub does: its text and inline code, without their marks', () => {
    expect(headingsOf([
      '# How a run\'s picture reaches the browser',
      '## 4. Slices',
      '## The `.worldview` recording format',
      '# Turn-Waypoint Encoding Contract — Phase 2 (HTM Trapezoidal Motion)',
      '### Café *and* [a link](x.md)',
    ].join('\n\n')).map(({ id }) => id)).toEqual([
      'how-a-runs-picture-reaches-the-browser',
      '4-slices',
      'the-worldview-recording-format',
      'turn-waypoint-encoding-contract--phase-2-htm-trapezoidal-motion',
      'café-and-a-link',
    ]);
  });

  it('starts afresh in each document', () => {
    expect(idsOf(['Notes', 'Notes'])).toEqual(['notes', 'notes-1']);
    expect(idsOf(['Notes'])).toEqual(['notes']);
  });
});

describe('a document\'s title', () => {
  it('is its first top-level heading in plain words, never a `#` line in a code block, else its file name', () => {
    expect(titleOf('docs/designs/worldview-format.md', 'intro\n\n# The `.worldview` recording **format**\n\n# Later')).toBe('The .worldview recording format');
    expect(titleOf('docs/a.md', '# A [linked](b.md) title #')).toBe('A linked title');
    expect(titleOf('docs/a.md', 'Set in a line\n=============')).toBe('Set in a line');
    expect(titleOf('docs/a.md', '```sh\n# a comment\n```\n\n# The title')).toBe('The title');
    expect(titleOf('sim/dessim/docs/algorithm-summary.md', 'Directory of algorithms\n\n## Order Choreography')).toBe('algorithm-summary');
  });

  it('is the same however far into a long document its heading, its underline, a code block around a `#` line or its link\'s definition reaches', () => {
    expect(titleOf('docs/a.md', `${'a'.repeat(4090)}\n\nSet late\n========`)).toBe('Set late');
    expect(titleOf('docs/a.md', `\`\`\`\n# a comment\n${'x\n'.repeat(3000)}\`\`\`\n\n# The title`)).toBe('The title');
    expect(titleOf('docs/a.md', `${'Text.\n\n'.repeat(2000)}# Far down`)).toBe('Far down');
    expect(titleOf('docs/a.md', `# [Architecture][guide]\n\n${'Text.\n\n'.repeat(2000)}[guide]: guide.md`)).toBe('Architecture');
  });

  it('opens a document that has none, as its file name with no id, shifting no id of its own headings', () => {
    expect(headingsOf('Text\n\n## Notes\n\n## Notes', 'docs/notes.md')).toEqual([
      { depth: 1, words: 'notes', id: undefined },
      { depth: 2, words: 'Notes', id: 'notes' },
      { depth: 2, words: 'Notes', id: 'notes-1' },
    ]);
    expect(headingsOf('# Its own\n\n## Notes', 'docs/notes.md').map(({ words }) => words)).toEqual(['Its own', 'Notes']);
  });
});
