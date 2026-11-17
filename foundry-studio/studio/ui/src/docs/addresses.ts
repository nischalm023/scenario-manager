// Where a reference in a served document leads, so that one Markdown file reads the same on GitHub and in the app: the
// route each document takes, what a link reaches (another served document, a heading of this one, a file the app does
// not serve, which opens on GitHub, or the web) and the built URL of an image. A reference splits into its path, query
// and fragment before any escape is decoded, and its path is read as GitHub reads a relative link: from the linking
// document's folder, or from the repository's root after a single `/`; `//` begins a web address.

import { routeFor } from '../shell/routes.ts';

/** Where a link to a repository file the app does not serve opens: the file on GitHub, on main. */
export const REPOSITORY = 'https://github.com/greyorange/foundry-studio/blob/main/';

/** The Docs section's own route; each document's route is under it. */
export const DOCS_ROUTE = routeFor({ view: 'docs', doc: '' });

/** A served document's route: `/docs/` and its path in the repository without `.md`, as the shell writes it. */
export const routeOf = (page: string): string => routeFor({ view: 'docs', doc: page.replace(/\.md$/, '') });

/** The repository path of the document a route names by its path without `.md`. */
export const pageAt = (doc: string): string => `${doc}.md`;

/** A repository path as a URL path: each name escaped whole, so a `#`, `?` or `%` in it stays part of the name. */
const escapedPath = (path: string): string => path.split('/').map(encodeURIComponent).join('/');

export type Target =
  | { kind: 'page'; page: string; fragment: string | null }
  | { kind: 'heading'; fragment: string }
  | { kind: 'repository'; path: string; query: string; fragment: string | null }
  | { kind: 'web'; url: string }
  | { kind: 'unresolved'; why: string };

const SCHEME = /^[a-z][a-z0-9+.-]*:/i;

/** A reference's three parts as written, escapes kept: its path, its query with the `?`, and its fragment. */
function partsOf(reference: string): { path: string; query: string; fragment: string | null } {
  const hash = reference.indexOf('#');
  const located = hash < 0 ? reference : reference.slice(0, hash);
  const question = located.indexOf('?');
  return {
    path: question < 0 ? located : located.slice(0, question),
    query: question < 0 ? '' : located.slice(question),
    fragment: hash < 0 ? null : reference.slice(hash + 1),
  };
}

/** `text` with its percent-escapes decoded; null when they do not decode. */
export function decoded(text: string): string | null {
  try {
    return decodeURIComponent(text);
  } catch {
    return null;
  }
}

/** The repository path a decoded `path` names from the document `from`; null when it climbs above the repository's root. */
function repositoryPath(path: string, from: string): string | null {
  const parts = path.startsWith('/') ? [] : from.split('/').slice(0, -1);
  for (const part of path.split('/')) {
    if (part === '' || part === '.') continue;
    if (part !== '..') parts.push(part);
    else if (parts.pop() === undefined) return null;
  }
  return parts.join('/');
}

/** What a link `href` in the document `from` reaches, given which repository paths are served documents. */
export function resolveLink(href: string, from: string, served: (path: string) => boolean): Target {
  if (href.startsWith('//')) return { kind: 'web', url: href };
  if (SCHEME.test(href)) {
    return /^(https?|mailto):/i.test(href) ? { kind: 'web', url: href } : { kind: 'unresolved', why: 'a link the app does not open' };
  }
  const parts = partsOf(href);
  const relative = decoded(parts.path);
  const fragment = parts.fragment === null ? null : decoded(parts.fragment);
  if (relative === null || (parts.fragment !== null && fragment === null)) return { kind: 'unresolved', why: 'a link whose escapes do not decode' };
  if (relative === '') return fragment === null ? { kind: 'unresolved', why: 'an empty link' } : { kind: 'heading', fragment };
  const path = repositoryPath(relative, from);
  if (path === null) return { kind: 'unresolved', why: 'a path above the repository' };
  // A served document opens here, the one already open among them, whatever its query asks of GitHub's view of the file.
  return served(path) ? { kind: 'page', page: path, fragment } : { kind: 'repository', path, query: parts.query, fragment };
}

/** The address a resolved link opens, with its fragment; null for a link that resolves nowhere. */
export function hrefOf(target: Target): string | null {
  const withFragment = (base: string, fragment: string | null) => (fragment === null ? base : `${base}#${encodeURIComponent(fragment)}`);
  switch (target.kind) {
    case 'page': return withFragment(routeOf(target.page), target.fragment);
    case 'heading': return withFragment('', target.fragment);
    case 'repository': return withFragment(`${REPOSITORY}${escapedPath(target.path)}${target.query}`, target.fragment);
    case 'web': return target.url;
    case 'unresolved': return null;
  }
}

/** A served document's own file on GitHub. */
export const sourceOf = (page: string): string => `${REPOSITORY}${escapedPath(page)}`;

/**
 * The built URL of the image `src` in the document `from`, an SVG's fragment (a view it names) kept; null when the app
 * does not bundle that file. The section shows only figures its build bundles and never an image from the web, so
 * opening a document fetches nothing from another host.
 */
export function imageUrl(src: string, from: string, figures: ReadonlyMap<string, string>): string | null {
  if (src.startsWith('//') || SCHEME.test(src)) return null;
  const parts = partsOf(src);
  const relative = decoded(parts.path);
  const path = relative === null || relative === '' ? null : repositoryPath(relative, from);
  const url = path === null ? undefined : figures.get(path);
  if (url === undefined) return null;
  return parts.fragment === null ? url : `${url}#${parts.fragment}`;
}
