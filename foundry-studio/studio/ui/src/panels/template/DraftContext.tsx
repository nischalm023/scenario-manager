// Spec 05 section 6, the context bar on `/template-drafts/{d}` (spec 02, request 16): the template's title and
// "(template)", its reference, a pill per template scope ("map at changeset 5"; "tag v3" once tags land), and the
// evaluation the panel draws; it offers no actions. It reads the draft the MapTemplate panel holds.

import { useShown } from './draftStore.ts';

export function DraftContext({ draft }: { draft: string }) {
  const shown = useShown(draft);
  if (shown === null) {
    return (
      <div className="context-bar__site">
        <span className="context-bar__name">New site</span>
        <span className="muted">Reading the template draft. Nothing is written until you create the site.</span>
      </div>
    );
  }
  const { ref, seqs, tag } = shown.draft.template;
  return (
    <>
      <div className="context-bar__site">
        <span className="context-bar__name">{shown.draft.title} (template)</span>
        <code className="context-bar__ref">{ref}</code>
      </div>
      <div className="context-bar__pin">
        {tag !== null
          ? <span className="pill" data-tone="info">tag {tag}</span>
          : Object.entries(seqs).map(([scope, seq]) => (
            <span className="pill" key={scope}>{scope === ref ? 'template' : scope.slice(ref.length + 1)} at changeset {seq}</span>
          ))}
        {shown.n !== null && <span className="pill" data-tone="info">evaluation {shown.n}</span>}
      </div>
    </>
  );
}
