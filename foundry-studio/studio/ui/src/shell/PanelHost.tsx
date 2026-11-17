// Spec 02 section 3.1, PanelHost: `main.panel#panel`, named by its title. Every state starts with the
// `h2.panel__title`, whose text the module answers at once, so a person's navigation moves focus there once (F6) even
// while the panel loads; below it the loading words, the error with "Try again", spec 03's not-found sentence, or the
// module's content.

import type { ContextPanel } from '../protocol/page.ts';
import type { PanelModule } from './panels.tsx';

const CLASS_OF: Record<ContextPanel, string> = {
  home: 'home', templates: 'template', template: 'template', map: 'map', flow: 'flow', history: 'history',
  scenario: 'scenario', runs: 'runs',
};

export function PanelHost({ module, address, retry, error, bare = false, hideTitle = false }: {
  module: PanelModule; address: string; retry?: () => void; error?: string; bare?: boolean;
  /** The module draws its own title on the page: the host's `h2` stays for assistive tech only. */
  hideTitle?: boolean;
}) {
  const state = error !== undefined ? 'error' : module.state;
  const title = module.title(address);
  return (
    <main className={`panel panel--${CLASS_OF[module.panel]}${bare ? ' panel--bare' : ''}${module.panel === 'templates' || module.panel === 'template' ? ' panel--pinned' : ''}`} id="panel" tabIndex={-1} aria-labelledby="panel-title">
      <div className={bare || hideTitle ? 'panel__header visually-hidden' : 'panel__header'}>
        <h2 className="panel__title" id="panel-title" tabIndex={-1}>{title}</h2>
      </div>
      {state === 'loading' && (
        <div aria-busy="true"><p className="panel__status">Loading the {title} panel</p></div>
      )}
      {state === 'error' && (
        <div className="panel__error">
          <p>The {title} panel could not load: {error}</p>
          <button className="btn btn--small" type="button" onClick={retry}>Try again</button>
        </div>
      )}
      {(state === 'ready' || state === 'not-found') && module.content}
    </main>
  );
}
