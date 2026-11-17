// Spec 02 section 3.1, Settings, and lean 8: the three choices this browser keeps (how much of a reply is read, the
// theme, what an agent `show` does), applied at once and kept in `localStorage` under `agent-studio.*`, read and
// written inside `try` so refused storage leaves the defaults.

import { useSyncExternalStore } from 'react';
import { readStored, writeStored } from './storage.ts';

/** Section 3.4's four values of "When the agent finishes a reply, your screen reader hears" (ruling D70). */
export type ReplyReading = 'd70' | 'full' | 'first' | 'off';
export type Theme = 'system' | 'light' | 'dark';
/** "When the agent opens a panel" (consolidated question 52, lead decided). */
export type AgentShow = 'switch' | 'link';

export interface Preferences {
  replyReading: ReplyReading;
  theme: Theme;
  agentShow: AgentShow;
}

const KEYS = {
  replyReading: 'agent-studio.reply-reading',
  theme: 'agent-studio.theme',
  agentShow: 'agent-studio.agent-show',
} as const;

const DEFAULTS: Preferences = { replyReading: 'd70', theme: 'system', agentShow: 'switch' };

const ALLOWED: { [K in keyof Preferences]: readonly Preferences[K][] } = {
  replyReading: ['d70', 'full', 'first', 'off'],
  theme: ['system', 'light', 'dark'],
  agentShow: ['switch', 'link'],
};

function readOne<K extends keyof Preferences>(name: K): Preferences[K] {
  const stored = readStored('local', KEYS[name]);
  return (ALLOWED[name] as readonly string[]).includes(stored ?? '') ? (stored as Preferences[K]) : DEFAULTS[name];
}

let current: Preferences = { ...DEFAULTS };
let loaded = false;
const listeners = new Set<() => void>();

export function preferences(): Preferences {
  if (!loaded) {
    loaded = true;
    current = { replyReading: readOne('replyReading'), theme: readOne('theme'), agentShow: readOne('agentShow') };
  }
  return current;
}

/** The tokens.css header's switch: `data-theme` on `html`, removed to follow the system. */
export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  if (theme === 'system') root.removeAttribute('data-theme');
  else root.setAttribute('data-theme', theme);
}

export function setPreference<K extends keyof Preferences>(name: K, value: Preferences[K]): void {
  current = { ...preferences(), [name]: value };
  writeStored('local', KEYS[name], value);
  if (name === 'theme') applyTheme(value as Theme);
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function usePreferences(): Preferences {
  return useSyncExternalStore(subscribe, preferences, preferences);
}

/** Forget what was read, for tests that change storage between renders. */
export function reloadPreferences(): void {
  loaded = false;
  listeners.forEach((listener) => listener());
}
