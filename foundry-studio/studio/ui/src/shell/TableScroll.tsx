// Spec 02 section 3.1, TableScroll: every table or canvas that may scroll sideways sits in a focusable named region,
// so at 320 CSS px a keyboard user can scroll it and a listener hears what it holds ("Your sites, table").

import type { ReactNode } from 'react';

export function TableScroll({ name, children }: { name: string; children: ReactNode }) {
  return (
    <div className="table-scroll" tabIndex={0} role="region" aria-label={`${name}, table`}>
      {children}
    </div>
  );
}
