// The Docs section as the shell mounts it on a `/docs` route: loaded on its first visit, so the documents' text and
// figures stay out of the bundle every other page loads. A visit whose load fails, as when the network drops or a tab
// opened before a redeploy asks for a chunk the new build renamed, says so in the panel and leaves the rest of the app
// standing.

import { Component, lazy, Suspense, type ReactNode } from 'react';

const DocsSection = lazy(async () => ({ default: (await import('./DocsSection.tsx')).DocsSection }));

/**
 * The section's load, failed: the panel says why, and Try again reloads the page. A browser keeps a module's failed
 * load for the page's life and a redeploy renames the chunk, so only a fresh page can load the section.
 */
class LoadFailure extends Component<{ children: ReactNode }, { reason: string | null }> {
  state: { reason: string | null } = { reason: null };

  static getDerivedStateFromError(failure: unknown) {
    return { reason: failure instanceof Error ? failure.message : String(failure) };
  }

  render() {
    if (this.state.reason === null) return this.props.children;
    return (
      <div className="panel__error">
        <p>The documents could not load: {this.state.reason}</p>
        <button className="btn btn--small" type="button" onClick={() => window.history.go(0)}>Try again</button>
      </div>
    );
  }
}

/** `doc`: the document's path in the repository without `.md`, as its route names it; empty for the list. */
export function DocsPanel({ doc }: { doc: string }) {
  return (
    <LoadFailure>
      <Suspense fallback={<p className="panel__status">Loading the documents</p>}>
        <DocsSection doc={doc} />
      </Suspense>
    </LoadFailure>
  );
}
