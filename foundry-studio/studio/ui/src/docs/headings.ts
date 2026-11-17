// A served document's headings as GitHub reads them: the id each heading takes, so `other.md#a-heading` names the same
// heading here and on GitHub, and the document's title, its first top-level `#` heading, else its file name, which a
// document without one is given as a heading of its own so every document opens with its title.

import type { Heading, Nodes, Root } from 'mdast';
import remarkGfm from 'remark-gfm';
import remarkParse from 'remark-parse';
import { unified, type Plugin } from 'unified';

/**
 * github-slugger's rule (its `script/generate-regex.js`): a heading's text in lower case, every character dropped that
 * is a symbol, a control, format, private-use or unassigned character, a separator other than the space, punctuation
 * other than the connector's (`_`) and the hyphen's, or a number other than a digit or a letter-number, unless it is
 * alphabetic (`Ⓐ`); then each space a `-`. github-slugger reads Unicode 13's tables and this the browser's, so a
 * character assigned since then may differ.
 */
const DROPPED = /[^\p{L}\p{M}\p{Nd}\p{Nl}\p{Pc}\p{Cs}\p{Alphabetic} -]/gu;

/** github-slugger's slugger: each heading's slug, a repeat within one document taking `-1`, `-2`, …. */
function slugger(): (text: string) => string {
  const occurrences = new Map<string, number>();
  return (text) => {
    const original = text.toLowerCase().replace(DROPPED, '').replace(/ /g, '-');
    let slug = original;
    while (occurrences.has(slug)) {
      const next = (occurrences.get(original) ?? 0) + 1;
      occurrences.set(original, next);
      slug = `${original}-${next}`;
    }
    occurrences.set(slug, 0);
    return slug;
  };
}

/** A heading's words as GitHub reads them for its id: its text and inline code, without their marks or raw HTML. */
const wordsOf = (node: Nodes): string =>
  node.type === 'html' ? '' : 'children' in node ? node.children.map(wordsOf).join('') : 'value' in node ? node.value : '';

/** A remark plugin: each heading of a document takes GitHub's id, as `data.hProperties.id`. */
export const headingIds: Plugin<[], Root> = () => (tree) => {
  const slug = slugger();
  const visit = (node: Nodes): void => {
    if (node.type === 'heading') node.data = { ...node.data, hProperties: { ...node.data?.hProperties, id: slug(wordsOf(node)) } };
    if ('children' in node) node.children.forEach(visit);
  };
  visit(tree);
};

const isTitle = (node: Nodes): node is Heading => node.type === 'heading' && node.depth === 1;
const fileNameOf = (page: string): string => page.slice(page.lastIndexOf('/') + 1).replace(/\.md$/, '');

const parser = unified().use(remarkParse).use(remarkGfm);

/**
 * A document's title: its first top-level `#` heading in plain words, else its file name without `.md`. The whole
 * document is read: a heading's `[words][name]` reads as its words alone only when `name` is defined, anywhere below.
 */
export function titleOf(page: string, text: string): string {
  const title = parser.parse(text).children.find(isTitle);
  return title === undefined ? fileNameOf(page) : wordsOf(title);
}

/**
 * A remark plugin, after `headingIds`: a document with no top-level `#` heading opens with its file name as one, which
 * takes no id, so it shifts no id GitHub gives the document's own headings.
 */
export const titled: Plugin<[string], Root> = (page) => (tree) => {
  if (tree.children.some(isTitle)) return;
  tree.children.unshift({ type: 'heading', depth: 1, children: [{ type: 'text', value: fileNameOf(page) }] });
};
