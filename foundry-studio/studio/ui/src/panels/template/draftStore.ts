// Spec 05 section 6, the context bar on `/template-drafts/{d}`: the draft the MapTemplate panel holds and the
// evaluation it draws, kept here so the context bar, which stands outside the panel, names them without a read of
// its own.

import { useSyncExternalStore } from 'react';
import type { TemplateDraft } from '../../protocol/template_draft.ts';

export interface Shown {
  draft: TemplateDraft;
  /** The evaluation the stage draws, null before the first is drawn. */
  n: number | null;
}

let shown: Shown | null = null;
const listeners = new Set<() => void>();

export function setShown(next: Shown | null): void {
  shown = next;
  listeners.forEach((listener) => listener());
}

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};

/** The draft `id` as the panel last held it, or null. */
export function useShown(id: string): Shown | null {
  const held = useSyncExternalStore(subscribe, () => shown);
  return held !== null && held.draft.draft === id ? held : null;
}
