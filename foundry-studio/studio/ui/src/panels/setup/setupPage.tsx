// What every Setup page is made of: the page's head (its icon, title, line and "i"), a placeholder for what
// Agent Studio holds no value for yet ("Coming soon"), and the page's words while its read is out or failed.

import type { ReactNode } from 'react';
import type { Route } from '../../shell/routes.ts';
import type { Read } from './read.ts';

export type WorldRoute = Extract<Route, { view: 'world' }>;

export function Head({ icon, title, words, info, right }: { icon: ReactNode; title: string; words: string; info?: ReactNode; right?: ReactNode }) {
  return (
    <header className="home__head page-head draft-head">
      <span className="page-head__icon" aria-hidden="true">
        <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">{icon}</svg>
      </span>
      <div>
        <p className="home__title">{title} {info}</p>
        <p className="home__sub">{words}</p>
      </div>
      {right !== undefined && <div className="draft-head__back">{right}</div>}
    </header>
  );
}

export function Soon({ label, note }: { label: string; note?: string }) {
  return (
    <div className="setup-fact" data-soon="true">
      <dt>{label}</dt>
      <dd><span className="setup-fact__soon">Coming soon</span>{note !== undefined && <span className="muted"> {note}</span>}</dd>
    </div>
  );
}

export function Status<T>({ read, what, children }: { read: Read<T>; what: string; children: (value: T) => ReactNode }) {
  if (read.state === 'loading') return <p className="panel__status" aria-busy="true">Reading {what}</p>;
  if (read.state === 'failed') return <p className="panel__status">{what[0].toUpperCase()}{what.slice(1)} could not be read: {read.reason}</p>;
  return <>{children(read.value)}</>;
}
