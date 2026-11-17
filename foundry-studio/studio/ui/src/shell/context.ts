// Spec 02 sections 3.5 and 6: what every part of the shell reads from the frame around it: the route, how to
// navigate (a user's navigation moves focus to the panel title once, F6; the agent's never moves focus, F7), the
// announcer, who is signed in and this browser tab's id.

import { createContext, useContext } from 'react';
import type { Me } from '../protocol/person.ts';
import type { Route } from './routes.ts';

/** `user`: a link the person followed (pushes, F6); `agent`: a `show` (pushes, F7); `within`: a change within the view. */
export type NavigationCause = 'user' | 'agent' | 'within';

export interface Shell {
  route: Route;
  address: string;
  navigate(to: string, cause: NavigationCause): void;
  announce(sentence: string): void;
  me: Me | null;
  browserTab: string;
  focusComposer(): void;
  focusPanelTitle(): void;
}

export const ShellContext = createContext<Shell | null>(null);

export function useShell(): Shell {
  const shell = useContext(ShellContext);
  if (shell === null) throw new Error('a shell component rendered outside the shell');
  return shell;
}
