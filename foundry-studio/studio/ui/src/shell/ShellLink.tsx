// Spec 02 section 3.5, F6: a link the person follows inside Agent Studio is a navigation of theirs, so the shell
// pushes it and moves focus once to the new panel's title; a modified click still opens a new browser tab natively.

import type { AnchorHTMLAttributes, MouseEvent } from 'react';
import { useShell } from './context.ts';

export function ShellLink({ to, onClick, ...rest }: { to: string } & AnchorHTMLAttributes<HTMLAnchorElement>) {
  const shell = useShell();
  const follow = (event: MouseEvent<HTMLAnchorElement>) => {
    onClick?.(event);
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    shell.navigate(to, 'user');
  };
  return <a href={to} onClick={follow} {...rest} />;
}
