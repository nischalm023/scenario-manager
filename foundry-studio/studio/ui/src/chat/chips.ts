// Spec 02 section 3.1's EntityChip and section 3.4's reply rule: what a `[label](ref:{ref})` link in an agent's text
// stands for, read from its block's resolved refs, so the chip on screen and the reply the announcer speaks say the
// same label. A site's or a template's location is no entity (D60: references name entities, never scope paths), so
// its link leads to the location's panel route and is never a chip, written as a reference
// (`scope:template/site/square@main`) or as the bare path a tool gave (`template/site/square`).

import type { ResolvedRef } from '../protocol/turn.ts';
import { keyOf, parseLocation, parseRef, printLocation, siteRootOf, templateRootOf } from '../shell/grammar.ts';
import { canonical } from '../shell/routes.ts';

export const REF_PREFIX = 'ref:';
const SCOPE_PREFIX = 'scope:';

export interface Chip {
  key: string;
  label: string;
  kind: string | null;
  route: string | null;
}

/** What a `ref:` link draws: a link to a location's panel route, with the link's own words, or a chip. */
export type RefLink = { kind: 'location'; route: string; words: string } | { kind: 'chip'; chip: Chip };

/** The text a `ref:` link names, as written: percent-decoded, or as it stands when that fails. */
function targetOf(href: string): string {
  const target = href.slice(REF_PREFIX.length);
  try {
    return decodeURIComponent(target);
  } catch {
    return target;
  }
}

/** The key a `ref:` link names, its pin dropped (spec 03 rule 6), or null when it reads as no reference. */
function keyOfRefLink(href: string): string | null {
  try {
    return keyOf(parseRef(targetOf(href)));
  } catch {
    return null;
  }
}

/** The canonical route of the site's or template's location a `ref:` link names, or null when it names none. */
function locationRoute(href: string): string | null {
  const target = targetOf(href);
  const text = target.startsWith(SCOPE_PREFIX) ? target.slice(SCOPE_PREFIX.length) : target;
  try {
    const location = parseLocation(text);
    if (siteRootOf(location.scope) === null && templateRootOf(location.scope) === null) return null;
    return canonical(`/w/${printLocation(location)}`);
  } catch {
    return null;
  }
}

/** The chip a `ref:` link draws: the label the API resolved when it resolved the link, else the link's own words. */
function chipOf(href: string, words: string, refs: ResolvedRef[]): Chip {
  const key = keyOfRefLink(href);
  const resolved = refs.find((ref) => ref.key === key);
  return { key: key ?? href.slice(REF_PREFIX.length), label: resolved?.label ?? words, kind: resolved?.kind ?? null, route: resolved?.route ?? null };
}

/** What a `ref:` link with these words draws: a location's link, else its chip. */
export function refLinkOf(href: string, words: string, refs: ResolvedRef[]): RefLink {
  const route = locationRoute(href);
  return route !== null ? { kind: 'location', route, words } : { kind: 'chip', chip: chipOf(href, words, refs) };
}

/** A chip in a sentence, as its text shows: its label, with "(not found)" when it does not resolve. */
const chipWords = (chip: Chip): string => (chip.route === null ? `${chip.label} (not found)` : chip.label);

/** A `ref:` link in a sentence, as its text shows: a location's words, or its chip's. */
export const refLinkWords = (link: RefLink): string => (link.kind === 'location' ? link.words : chipWords(link.chip));
