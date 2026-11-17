// Spec 02 section 3.1, AppBar: the icon rail and the banner beside it. The rail holds the brand, the sections as
// icons and, at its foot, Foundry's projects in the same browser tab (never a frame), the link to Settings and the person
// signed in. Settings is a page of its own (`/settings`, `SettingsPanel`), not the popover it once was: it is a
// section link like the others, current while that page shows.

import type { ReactNode } from 'react';
import { useShell } from './context.ts';
import { ShellLink } from './ShellLink.tsx';

const FOUNDRY_PROJECTS = 'https://foundry.greyorange.com/projects';

const SECTIONS: { name: string; to: string | null; views: string[]; icon: ReactNode }[] = [
  { name: 'Dashboard', to: '/', views: ['home'], icon: <><rect x="3" y="3" width="7" height="9" rx="1" /><rect x="14" y="3" width="7" height="5" rx="1" /><rect x="14" y="12" width="7" height="9" rx="1" /><rect x="3" y="16" width="7" height="5" rx="1" /></> },
  { name: 'Runs', to: '/runs', views: ['runs', 'run', 'compare'], icon: <><circle cx="12" cy="12" r="9" /><path d="M10 8l6 4-6 4z" /></> },
  { name: 'Analytics', to: null, views: [], icon: <><path d="M4 20V10M10 20V4M16 20v-7M22 20H2" /></> },
  { name: 'Docs', to: '/docs', views: ['docs'], icon: <><path d="M2 4h6a4 4 0 0 1 4 4v13a3 3 0 0 0-3-3H2z" /><path d="M22 4h-6a4 4 0 0 0-4 4v13a3 3 0 0 1 3-3h7z" /></> },
  { name: 'Experiments builder', to: null, views: [], icon: <><path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6" /></> },
];

function initials(name: string): string {
  return name.split(/\s+/).filter(Boolean).slice(0, 2).map((word) => word[0].toUpperCase()).join('');
}

export function AppBar({ pagePhrase }: { pagePhrase: string }) {
  const { me, route } = useShell();
  return (
    <>
      <aside className="side-rail">
        <h1 className="visually-hidden">Agent Studio, {pagePhrase}</h1>
        <ShellLink className="side-rail__brand" to="/" title="Agent Studio" aria-label="Agent Studio, home">
          <img className="app-bar__logo" src="/logo.png" alt="" width="36" height="46" />
        </ShellLink>
        <nav className="side-nav" aria-label="Sections">
        {SECTIONS.map((section) => {
          const label = <><svg className="side-foot__icon" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{section.icon}</svg><span className="side-foot__label">{section.name}</span></>;
          if (section.to === null) {
            return <span key={section.name} className="side-foot__item side-nav__item" aria-disabled="true" title={`${section.name}, coming soon`} role="link">{label}</span>;
          }
          const current = section.views.includes(route.view);
          return (
            <ShellLink key={section.name} className="side-foot__item side-nav__item" to={section.to} title={section.name}
              aria-label={section.name} aria-current={current ? 'page' : undefined} data-active={current ? 'true' : undefined}>{label}</ShellLink>
          );
        })}
        </nav>
        <footer className="side-foot">
        <a className="side-foot__item" href={FOUNDRY_PROJECTS} data-link="foundry-projects" title="Back to Foundry" aria-label="Foundry projects">
          <span className="side-foot__f" aria-hidden="true">F</span>
          <span className="side-foot__label">Foundry</span>
        </a>
        <ShellLink className="side-foot__item app-bar__settings" to="/settings" title="Settings" aria-label="Settings"
          aria-current={route.view === 'settings' ? 'page' : undefined} data-active={route.view === 'settings' ? 'true' : undefined}>
          <svg className="side-foot__icon" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3h0a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8v0a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" /></svg>
          <span className="side-foot__label">Settings</span>
        </ShellLink>
        {me !== null && (
          <div className="side-foot__profile" title={`${me.name}, ${me.email}`}>
            <span className="avatar avatar--lg" aria-hidden="true">{initials(me.name)}</span>
          </div>
        )}
        </footer>
      </aside>
      <header className="app-bar" id="app-bar">
        <span className="app-bar__name">Agent Studio</span>
      </header>
    </>
  );
}
