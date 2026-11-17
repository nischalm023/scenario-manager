// Spec 02 section 3.4, the reply rule (ruling D70): a reply's spoken text is its rendered blocks, where each
// paragraph, list item, table row and heading ends a sentence, a code block reads "A code block of {k} lines.", a
// chip reads as the label its block's resolved refs give it and a location's link as its words, as on screen;
// `Intl.Segmenter` splits each block into sentences, so "e.g." and "1.5" stay inside one.

import type { Heading, List, ListItem, Paragraph, PhrasingContent, Root, RootContent, Table } from 'mdast';
import remarkGfm from 'remark-gfm';
import remarkParse from 'remark-parse';
import { unified } from 'unified';
import type { ResolvedRef } from '../protocol/turn.ts';
import type { ReplyReading } from '../shell/preferences.ts';
import { REF_PREFIX, refLinkOf, refLinkWords } from './chips.ts';

const segmenter = new Intl.Segmenter('en', { granularity: 'sentence' });
const markdown = unified().use(remarkParse).use(remarkGfm);

export function sentencesOf(text: string): string[] {
  return [...segmenter.segment(text)].map((part) => part.segment.trim()).filter((part) => part !== '');
}

export function firstSentence(text: string): string {
  return sentencesOf(text)[0] ?? text;
}

function phrasing(nodes: PhrasingContent[], refs: ResolvedRef[]): string {
  return nodes
    .map((node) => {
      switch (node.type) {
        case 'text': case 'inlineCode': return node.value;
        case 'break': return ' ';
        case 'image': case 'imageReference': case 'html': case 'footnoteReference': return '';
        case 'link':
          if (node.url.startsWith(REF_PREFIX)) return refLinkWords(refLinkOf(node.url, phrasing(node.children, refs), refs));
          return phrasing(node.children, refs);
        default: return 'children' in node ? phrasing(node.children as PhrasingContent[], refs) : '';
      }
    })
    .join('');
}

/** The units that each end a sentence, as the rendered block shows them. */
function units(nodes: (RootContent | ListItem)[], refs: ResolvedRef[]): string[] {
  return nodes.flatMap((node): string[] => {
    switch (node.type) {
      case 'paragraph': case 'heading': return [phrasing((node as Paragraph | Heading).children, refs)];
      case 'list': return (node as List).children.flatMap((item) => [units(item.children, refs).join(' ')]);
      case 'table':
        return (node as Table).children.map((row) => row.children.map((cell) => phrasing(cell.children, refs)).join(', '));
      case 'code': {
        const lines = node.value === '' ? 0 : node.value.split('\n').length;
        return [`A code block of ${lines} line${lines === 1 ? '' : 's'}.`];
      }
      case 'blockquote': return units(node.children, refs);
      default: return [];
    }
  });
}

/** The sentences a reader hears for a finished block of Markdown whose `ref:` links resolve as `refs` say. */
export function spokenSentences(text: string, refs: ResolvedRef[] = []): string[] {
  const tree = markdown.runSync(markdown.parse(text)) as Root;
  return units(tree.children, refs).flatMap((unit) => {
    const sentences = sentencesOf(unit);
    const last = sentences.length - 1;
    if (last >= 0 && !/[.!?:;…]$/.test(sentences[last])) sentences[last] = `${sentences[last]}.`;
    return sentences;
  });
}

/** What the setting lets through of a reply of n sentences (section 3.4's table). */
export function replyToSpeak(sentences: string[], reading: ReplyReading): string | null {
  const n = sentences.length;
  if (n === 0 || reading === 'off') return null;
  const more = (count: number) => `${count} more sentence${count === 1 ? '' : 's'} in the reply.`;
  if (reading === 'full' || (reading === 'd70' && n <= 3)) return sentences.join(' ');
  return n > 1 ? `${sentences[0]} ${more(n - 1)}` : sentences[0];
}
