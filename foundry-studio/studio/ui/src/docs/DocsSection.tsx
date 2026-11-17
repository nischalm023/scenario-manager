// The Docs section of Agent Studio: the repository's own documents as the build read them (`manifest.ts`), listed by
// folder at `/docs`, each drawn at its own route below it, and a plain sentence for a route that names none.

import { ShellLink } from '../shell/ShellLink.tsx';
import { DOCS_ROUTE, pageAt, routeOf } from './addresses.ts';
import { DocPage } from './DocPage.tsx';
import { titleOf } from './headings.ts';
import { PAGES } from './manifest.ts';
import './docs.css';

/** `doc`: the document's path in the repository without `.md`, as its route names it; empty for the list. */
export function DocsSection({ doc }: { doc: string }) {
  if (doc === '') return <DocsIndex />;
  const page = pageAt(doc);
  if (!PAGES.has(page)) {
    return (
      <div className="docs-index">
        <p>No document is served at <code>{page}</code>.</p>
        <p><ShellLink to={DOCS_ROUTE}>All documents</ShellLink></p>
      </div>
    );
  }
  return <DocPage page={page} />;
}

const folderOf = (page: string): string => page.slice(0, page.lastIndexOf('/'));

/** Each served document's title, read on the list's first drawing and kept: reading a title parses its whole document. */
let titles: ReadonlyMap<string, string> | undefined;

function DocsIndex() {
  const documents = (titles ??= new Map([...PAGES].map(([page, text]) => [page, titleOf(page, text)])));
  const folders = [...new Set([...PAGES.keys()].map(folderOf))].sort();
  return (
    <div className="docs-index">
      <p className="docs-index__lede">
        The repository's own documents, as they stood when this app was built. A link between them opens here; a link
        to code opens on GitHub.
      </p>
      {folders.map((folder) => (
        <section key={folder} className="docs-index__folder" aria-labelledby={`folder-${folder}`}>
          <h3 className="docs-index__path" id={`folder-${folder}`}><code>{folder}/</code></h3>
          <ul className="docs-index__list">
            {[...documents].filter(([page]) => folderOf(page) === folder).map(([page, title]) => (
              <li key={page}>
                <ShellLink to={routeOf(page)}>{title}</ShellLink>
                <span className="docs-index__file">{page.slice(folder.length + 1)}</span>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}
