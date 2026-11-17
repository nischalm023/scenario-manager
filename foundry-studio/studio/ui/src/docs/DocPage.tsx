// One served document drawn in the Docs section: its Markdown in GitHub's dialect, raw HTML dropped; its title first,
// the file's name when it has no `#` heading; each heading with GitHub's id, linking to itself; a link to another served
// document opening it here, a link to any other repository file opening it on GitHub and a web link opening a new
// browser tab; each image the file the build bundled; each figure, table and code block in a focusable region that
// scrolls sideways when the panel is narrower than it. The panel host's title is the page's `h2`, so a document's
// title is its `h3`. An address naming a heading brings that heading into view and moves focus to it.

import { createContext, useContext, useEffect, useRef, type ComponentProps } from 'react';
import Markdown, { type Components, type ExtraProps } from 'react-markdown';
import { useLocation } from 'react-router';
import remarkGfm from 'remark-gfm';
import { ShellLink } from '../shell/ShellLink.tsx';
import { tableName } from '../shell/tableName.ts';
import { TableScroll } from '../shell/TableScroll.tsx';
import { decoded, hrefOf, imageUrl, resolveLink, sourceOf } from './addresses.ts';
import { headingIds, titled } from './headings.ts';
import { FIGURES, PAGES } from './manifest.ts';

/** The repository path of the document being drawn, which its links and images are read against. */
const PageContext = createContext('');

const NEW_TAB = <span className="visually-hidden"> (opens a new browser tab)</span>;

type Element = NonNullable<ExtraProps['node']>;

/** Whether a heading already holds a link, which a link to the heading itself must not wrap. */
const holdsLink = (node: Element): boolean =>
  node.children.some((child) => child.type === 'element' && (child.tagName === 'a' || holdsLink(child)));

function Heading({ node, id, children }: ComponentProps<'h1'> & ExtraProps) {
  const depth = Number(node?.tagName.slice(1) ?? 1);
  const Level = `h${Math.min(depth + 2, 6)}` as 'h3' | 'h4' | 'h5' | 'h6';
  const linked = id !== undefined && node !== undefined && !holdsLink(node);
  // A heading an address names takes focus (`DocPage`), so the next Tab and a screen reader go on from it.
  return (
    <Level id={id} tabIndex={id === undefined ? undefined : -1} className={`doc__heading doc__heading--${depth}`}>
      {linked ? <a className="doc__self" href={`#${id}`}>{children}</a> : children}
    </Level>
  );
}

function Link({ href, children }: ComponentProps<'a'> & ExtraProps) {
  const page = useContext(PageContext);
  const target = resolveLink(href ?? '', page, (path) => PAGES.has(path));
  const to = hrefOf(target);
  if (target.kind === 'unresolved' || to === null) return <span className="doc__unresolved">{children}</span>;
  if (target.kind === 'page') return <ShellLink to={to}>{children}</ShellLink>;
  // A `mailto:` link hands the address to the mail program and opens no tab.
  if (target.kind === 'heading' || /^mailto:/i.test(to)) return <a href={to}>{children}</a>;
  return <a href={to} target="_blank" rel="noreferrer">{children}{NEW_TAB}</a>;
}

function Image({ src, alt, title }: ComponentProps<'img'> & ExtraProps) {
  const page = useContext(PageContext);
  const url = typeof src === 'string' ? imageUrl(src, page, FIGURES) : null;
  if (url === null) return <span className="doc__unresolved">{alt}</span>;
  return <img className="doc__image" src={url} alt={alt ?? ''} title={title} />;
}

/** A paragraph holding one image alone is a figure: drawn at its own size, scrolling sideways in a narrow panel. */
function Paragraph({ node, children }: ComponentProps<'p'> & ExtraProps) {
  const parts = node?.children.filter((child) => child.type !== 'text' || child.value.trim() !== '') ?? [];
  const only = parts.length === 1 ? parts[0] : undefined;
  if (only?.type === 'element' && only.tagName === 'img') {
    const title = only.properties.title;
    return (
      <div className="doc__figure" tabIndex={0} role="region" aria-label={typeof title === 'string' && title !== '' ? `${title}, figure` : 'A figure in this document'}>
        {children}
      </div>
    );
  }
  return <p>{children}</p>;
}

/**
 * Inline code may break after each `/` but one that begins a path, at the code's start or after a space, so a path
 * wraps between its folders and never inside a name, nor leaves its first `/` alone at a line's end.
 */
function Code({ className, children }: ComponentProps<'code'> & ExtraProps) {
  if (typeof children !== 'string' || children.includes('\n')) return <code className={className}>{children}</code>;
  return <code>{children.split(/(?<=\S\/)/).flatMap((part, index) => (index === 0 ? [part] : [<wbr key={index} />, part]))}</code>;
}

/** A code block scrolls sideways rather than wrap, in a focusable region named by its language. */
function Pre({ node, children }: ComponentProps<'pre'> & ExtraProps) {
  const code = node?.children.find((child) => child.type === 'element' && child.tagName === 'code');
  const classes = code?.type === 'element' ? code.properties.className : undefined;
  const language = Array.isArray(classes) ? classes.map(String).find((name) => name.startsWith('language-'))?.slice('language-'.length) : undefined;
  return <pre tabIndex={0} role="region" aria-label={language === undefined ? 'A code block' : `A ${language} code block`}>{children}</pre>;
}

function Table({ node, children }: ComponentProps<'table'> & ExtraProps) {
  return <TableScroll name={tableName(node, 'A table in this document')}><table>{children}</table></TableScroll>;
}

const COMPONENTS: Components = {
  h1: Heading, h2: Heading, h3: Heading, h4: Heading, h5: Heading, h6: Heading,
  a: Link, code: Code, img: Image, p: Paragraph, pre: Pre, table: Table,
};

/** `page`: a served document's path in the repository. */
export function DocPage({ page }: { page: string }) {
  const { hash, key } = useLocation();
  const article = useRef<HTMLElement>(null);
  // The document is drawn after the browser looked for the address's heading, and on arrival the shell moves focus to
  // the panel's title (F6), so the page brings the heading into view and focuses it itself, a frame after both. Each
  // following of a link to it does so again (`key`). The heading is looked for in this document only, since an id of
  // its own may also name an element of the shell.
  useEffect(() => {
    const id = decoded(hash.slice(1)) ?? hash.slice(1);
    if (id === '') return;
    const frame = requestAnimationFrame(() => {
      const heading = [...(article.current?.querySelectorAll<HTMLElement>('[id]') ?? [])].find((element) => element.id === id);
      heading?.scrollIntoView();
      heading?.focus({ preventScroll: true });
    });
    return () => cancelAnimationFrame(frame);
  }, [page, hash, key]);
  return (
    <article className="doc" ref={article}>
      <p className="doc__source">
        From <a href={sourceOf(page)} target="_blank" rel="noreferrer"><code>{page}</code>{NEW_TAB}</a>
      </p>
      <PageContext.Provider value={page}>
        <Markdown remarkPlugins={[remarkGfm, headingIds, [titled, page]]} skipHtml components={COMPONENTS}>{PAGES.get(page) ?? ''}</Markdown>
      </PageContext.Provider>
    </article>
  );
}
