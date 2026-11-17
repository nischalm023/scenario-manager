// Spec 02 section 3.1, ContextBar: the region "Site in context", one sentence at most (D117). With no site, "No site
// open" and New site (ruling D94); on the template routes, "New site" and "Nothing is written until you create the
// site." until spec 05's words for a template being evaluated land with its panel; with a site, its name and reference,
// the branch or tag pill, the shown scope's changeset when the address freezes one ("map at changeset 1", never a bare
// "changeset 1"), "following the branch head" or "frozen" with "Go to the head", and History. The changeset a branch
// stands at, and "Update from template", wait for the reads of task 6b and spec 05.

import { DraftContext } from '../panels/template/DraftContext.tsx';
import { printPin } from './grammar.ts';
import { routeFor, type Route } from './routes.ts';
import { ShellLink } from './ShellLink.tsx';

export function ContextBar({ route, site }: { route: Route; site: { location: string; name: string } | null }) {
  if (route.view === 'template-draft') {
    return (
      <div className="context-bar" id="context-bar" role="region" aria-label="Site in context">
        <DraftContext draft={route.draft} />
      </div>
    );
  }
  if (site === null) {
    const choosing = route.view === 'templates' || (route.view === 'world' && route.show === 'template');
    return (
      <div className="context-bar" id="context-bar" role="region" aria-label="Site in context">
        <div className="context-bar__site">
          <span className="context-bar__name">{choosing ? 'New site' : 'No site open'}</span>
          {choosing && <span className="muted">Nothing is written until you create the site.</span>}
        </div>
        {!choosing && (
          <div className="context-bar__actions">
            <ShellLink className="btn btn--small" to="/templates" data-route="/templates">New site</ShellLink>
          </div>
        )}
      </div>
    );
  }
  const world = route.view === 'world' ? route : null;
  const location = world?.location ?? null;
  const pin = location?.pin ?? null;
  const shownScope = location ? location.scope.split('/').pop() : null;
  const historyRoute = `/w/${site.location}@${pin !== null && pin.kind !== 'changeset' ? printPin(pin) : 'main'}?panel=history`;
  const head = world && location ? routeFor({ ...world, location: { scope: location.scope, pin: { kind: 'branch', name: 'main' } }, kin: undefined }) : null;
  return (
    <div className="context-bar" id="context-bar" role="region" aria-label="Site in context">
      <div className="context-bar__site">
        <span className="context-bar__name">{site.name}</span>
        <code className="context-bar__ref">{site.location}</code>
      </div>
      {pin !== null && location !== null && (
        <div className="context-bar__pin">
          {pin.kind === 'branch' && <span className="pill" data-tone="info">branch {pin.name}</span>}
          {pin.kind === 'tag' && <span className="pill" data-tone="info">tag {pin.name}</span>}
          {pin.kind === 'changeset' && <span className="pill">{shownScope} at changeset {pin.seq}</span>}
          {pin.kind === 'branch' && <span className="muted">following the branch head</span>}
          {pin.kind !== 'branch' && head !== null && (
            <>
              <span className="muted">frozen</span>
              <ShellLink className="btn btn--small" to={head}>Go to the head</ShellLink>
            </>
          )}
        </div>
      )}
      <div className="context-bar__actions">
        <ShellLink className="btn btn--small" to={historyRoute} data-route={historyRoute}>History</ShellLink>
      </div>
    </div>
  );
}
