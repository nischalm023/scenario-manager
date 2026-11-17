// Spec 02 sections 3.5 and 3.6: focus never falls to `body` when what held it goes away for a reason the person did
// not cause in this tab (F5, F7): a control that leaves moves focus to its card's heading, else its turn's heading,
// else, for the composer's own controls, the composer input, else the log, and when the whole chat column went with
// it (an agent's `show` of a route without one), the new panel's title; a control a component's own state takes away,
// which no render of the shell follows, moves focus to the home that component names, if focus was still on it. And
// the region keys: F6 and Ctrl+Grave accent (Shift for backwards) visit the composer input, the newest turn's heading
// (the log itself while it holds no turn), the current tab link (the first enabled one on Home, a disabled tab being a
// link too) and `main#panel`. A folded chat draws neither of its two, so its tab, which opens it, stands in their
// place.

import { useEffect, useLayoutEffect, useRef, type FocusEvent as ReactFocusEvent } from 'react';

/** Where focus goes if `element` leaves: the ids of the places it belongs to, nearest first. */
function homesOf(element: Element): string[] {
  const homes: string[] = [];
  const named = element.closest('[data-focus-home]')?.getAttribute('data-focus-home');
  if (named) homes.push(named);
  const card = element.closest('.chat-card')?.querySelector('h4.chat-card__title[id]')?.id;
  if (card) homes.push(card);
  const turn = element.closest('li.turn')?.querySelector(':scope > h3.turn__header[id]')?.id;
  if (turn) homes.push(turn);
  if (element.closest('#composer')) homes.push('composer-input');
  if (element.closest('#chat-log')) homes.push('chat-log');
  // PanelHost draws the title in every state, so it survives whatever else a navigation replaced (F7).
  homes.push('panel-title');
  return homes;
}

/** A focus request waiting for its element, which lapses after a few seconds of wall clock. */
let pending: { id: string; until: number } | null = null;
const PENDING_MS = 5000;

/**
 * Move focus to the element with `id` now, or as soon as it is drawn: after Dismiss the composer input belongs to the
 * next conversation, which is drawn once its snapshot is read (F3).
 */
export function focusSoon(id: string): void {
  const element = document.getElementById(id);
  if (element !== null) {
    pending = null;
    element.focus();
  } else {
    pending = { id, until: Date.now() + PENDING_MS };
  }
}

/**
 * Move focus to the element with `id` once a render has drawn it: the composer input of a folded chat is in the
 * document but hidden until the render that opens the chat.
 */
export function focusOnceDrawn(id: string): void {
  pending = { id, until: Date.now() + PENDING_MS };
}

/** F5 and F7: after every render, a focus that fell to `body` returns to the nearest home of what held it. */
export function useFocusKeeper(): void {
  const last = useRef<{ element: Element; homes: string[] } | null>(null);
  useEffect(() => {
    const remember = (event: FocusEvent) => {
      if (event.target instanceof Element) last.current = { element: event.target, homes: homesOf(event.target) };
    };
    document.addEventListener('focusin', remember);
    return () => document.removeEventListener('focusin', remember);
  }, []);
  useLayoutEffect(() => {
    if (pending !== null && Date.now() > pending.until) pending = null;
    if (pending !== null) {
      const waited = document.getElementById(pending.id);
      if (waited !== null) {
        pending = null;
        waited.focus();
        return;
      }
    }
    const held = last.current;
    const fell = document.activeElement === null || document.activeElement === document.body;
    if (held === null || !fell || held.element.isConnected) return;
    const home = held.homes.map((id) => document.getElementById(id)).find((element) => element !== null);
    home?.focus();
  });
}

/**
 * F5 for controls a component's own state takes away, which the keeper above does not see, since it checks after the
 * shell's renders only: the handlers go on what holds the controls, and after each render of the component, focus
 * that fell to `body` as the control holding it left goes to the first of `homes` still drawn. A control holds focus
 * from its focus until focus leaves it while it stands: a person who moved focus on, or pressed the pointer on the
 * page's blank space, let it go, and its later leaving moves nothing. A blur that leaves focus on the control, as the
 * window losing focus does, lets nothing go; nor does a browser's blur of a control React removes, which comes during
 * React's commit, when React hands no event to a handler.
 */
export function useFocusHome(homes: () => (HTMLElement | null)[]): {
  onFocus(event: ReactFocusEvent): void;
  onBlur(event: ReactFocusEvent): void;
} {
  const holder = useRef<Element | null>(null);
  useLayoutEffect(() => {
    const left = holder.current;
    if (left === null || left.isConnected) return;
    holder.current = null;
    if (document.activeElement !== null && document.activeElement !== document.body) return;
    homes().find((home) => home?.isConnected)?.focus();
  });
  return {
    onFocus: (event) => {
      holder.current = event.target;
    },
    onBlur: (event) => {
      if (holder.current === event.target && document.activeElement !== event.target) holder.current = null;
    },
  };
}

function newestTurnHeading(): HTMLElement | null {
  const headings = document.querySelectorAll<HTMLElement>('#chat-log h3.turn__header[tabindex="-1"]');
  return headings[headings.length - 1] ?? null;
}

/** The four regions in order; the chat's two are absent on a route without a chat column, and its tab stands for both
 * while it is folded. */
export function regions(): HTMLElement[] {
  const folded = document.querySelector<HTMLElement>('#chat-toggle[aria-expanded="false"]');
  const composer = folded ?? document.getElementById('composer-input');
  const log = folded === null ? document.getElementById('chat-log') : null;
  const tab = document.querySelector<HTMLElement>('#panel-tabs a[aria-current="page"]') ??
    document.querySelector<HTMLElement>('#panel-tabs a.panel-tabs__tab:not([aria-disabled="true"])');
  const panel = document.getElementById('panel');
  return [composer, log === null ? null : newestTurnHeading() ?? log, tab, panel].filter((region): region is HTMLElement => region !== null);
}

function regionOf(element: Element | null, list: HTMLElement[]): number {
  if (element === null) return -1;
  if (element.closest('#chat-toggle')) return list.findIndex((region) => region.id === 'chat-toggle');
  if (element.closest('#composer')) return list.findIndex((region) => region.id === 'composer-input');
  if (element.closest('#chat-log')) return list.findIndex((region) => region.closest('#chat-log') !== null);
  if (element.closest('#panel-tabs')) return list.findIndex((region) => region.closest('#panel-tabs') !== null);
  if (element.closest('#panel')) return list.findIndex((region) => region.id === 'panel');
  return -1;
}

/** Section 3.6: F6, Shift+F6, Ctrl+Grave accent and Ctrl+Shift+Grave accent move between the regions. */
export function useRegionKeys(): void {
  useEffect(() => {
    const move = (event: KeyboardEvent) => {
      const byKey = event.key === 'F6' && !event.ctrlKey && !event.altKey && !event.metaKey;
      const byGrave = event.code === 'Backquote' && event.ctrlKey && !event.altKey && !event.metaKey;
      if (!byKey && !byGrave) return;
      const list = regions();
      if (list.length === 0) return;
      event.preventDefault();
      const at = regionOf(document.activeElement, list);
      const next = event.shiftKey ? (at <= 0 ? list.length - 1 : at - 1) : (at < 0 ? 0 : (at + 1) % list.length);
      list[next].focus();
    };
    document.addEventListener('keydown', move);
    return () => document.removeEventListener('keydown', move);
  }, []);
}
