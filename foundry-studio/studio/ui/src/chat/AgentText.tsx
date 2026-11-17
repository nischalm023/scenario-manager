// Spec 02 section 3.1, AgentText and EntityChip. While a block streams it is plain text marked `aria-busy`; at
// `text.done` it renders as sanitized Markdown (react-markdown with remark-gfm; brief I1.05, lead decided): no raw
// HTML, no images, a heading as `p.entry__subhead` so the outline stays chat h2, turn h3, card h4; a table inside a
// TableScroll named by its header cells, so it scrolls sideways at 320 CSS px and says what it holds (section 5);
// `[label](ref:…)` as an EntityChip from the block's resolved refs, or, naming a site's or a template's location, as a
// link to its panel route (`chips.ts`); an http(s) link opening a new browser tab and saying so, any other link as text.

import type { ReactNode } from 'react';
import Markdown, { defaultUrlTransform, type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { ResolvedRef } from '../protocol/turn.ts';
import { ShellLink } from '../shell/ShellLink.tsx';
import { tableName } from '../shell/tableName.ts';
import { TableScroll } from '../shell/TableScroll.tsx';
import { REF_PREFIX, refLinkOf } from './chips.ts';

/** The default transform empties a `ref:` link; ours keeps it for the chip and passes every other URL to it. */
function keepRefLinks(url: string): string {
  return url.startsWith(REF_PREFIX) ? url : defaultUrlTransform(url);
}

export function EntityChip({ label, refKey, kind, route }: { label: string; refKey: string; kind: string | null; route: string | null }) {
  if (route === null) {
    return <span className="entity-chip" data-state="missing" data-ref={refKey}>{label} (not found)</span>;
  }
  return (
    <ShellLink className="entity-chip" data-ref={refKey} to={route} aria-label={kind ? `${label}, ${kind}` : label}>
      {label}
      {kind && <span className="entity-chip__kind" aria-hidden="true">{kind}</span>}
    </ShellLink>
  );
}

function components(refs: ResolvedRef[]): Components {
  const heading = ({ children }: { children?: ReactNode }) => <p className="entry__subhead">{children}</p>;
  return {
    h1: heading, h2: heading, h3: heading, h4: heading, h5: heading, h6: heading,
    img: () => null,
    table: ({ node, children }) => <TableScroll name={tableName(node, 'A table in the reply')}><table>{children}</table></TableScroll>,
    a: ({ href, children }) => {
      if (href?.startsWith(REF_PREFIX)) {
        const link = refLinkOf(href, typeof children === 'string' ? children : href.slice(REF_PREFIX.length), refs);
        if (link.kind === 'location') return <ShellLink to={link.route}>{children}</ShellLink>;
        const { chip } = link;
        return <EntityChip label={chip.label} refKey={chip.key} kind={chip.kind} route={chip.route} />;
      }
      if (href !== undefined && /^https?:\/\//.test(href)) {
        return (
          <a href={href} target="_blank" rel="noreferrer">
            {children}<span className="visually-hidden"> (opens a new browser tab)</span>
          </a>
        );
      }
      return <>{children}</>;
    },
  };
}

export function AgentText({ text, streaming, refs }: { text: string; streaming: boolean; refs: ResolvedRef[] }) {
  return (
    <li className="entry entry--agent" data-kind="text">
      <div className="entry__body" aria-busy={streaming ? 'true' : undefined}>
        {streaming ? (
          <p>{text}</p>
        ) : (
          <Markdown remarkPlugins={[remarkGfm]} skipHtml urlTransform={keepRefLinks} components={components(refs)}>
            {text}
          </Markdown>
        )}
      </div>
    </li>
  );
}
