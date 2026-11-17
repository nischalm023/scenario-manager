// Spec 02 section 3.4, the announcer (lean 3): the page's one live region, `div#announcer[role=log]`, present and
// empty from the first paint as the last element of `body` (index.html). Each announcement appends one `p`, removed
// 7 s after it was added, as React Aria's LiveAnnouncer does. While the browser tab is hidden nothing is appended: the
// announcements are kept in order, their count stands in front of `document.title`, and on return one sentence sums
// them up. Which entry speaks what is speech.ts's; this module only says it.

import { firstSentence } from './sentences.ts';

const LIFETIME_MS = 7000;
const MOST_SUMMED = 5;

export class Announcer {
  private held: string[] = [];
  private titleBase: string;
  private readonly doc: Document;
  private readonly lifetimeMs: number;

  constructor(doc: Document = document, lifetimeMs = LIFETIME_MS) {
    this.doc = doc;
    this.lifetimeMs = lifetimeMs;
    this.titleBase = doc.title;
    doc.addEventListener('visibilitychange', () => this.visibilityChanged());
  }

  say(sentence: string): void {
    if (sentence.trim() === '') return;
    if (this.doc.visibilityState === 'hidden') {
      this.held.push(sentence);
      this.paintTitle();
      return;
    }
    this.append(sentence);
  }

  /** The `h1` text, which `document.title` repeats after any count of announcements kept in the background. */
  setTitle(base: string): void {
    this.titleBase = base;
    this.paintTitle();
  }

  private append(sentence: string): void {
    const root = this.doc.getElementById('announcer');
    if (root === null) return;
    const line = this.doc.createElement('p');
    line.textContent = sentence;
    root.appendChild(line);
    setTimeout(() => line.remove(), this.lifetimeMs);
  }

  private paintTitle(): void {
    this.doc.title = this.held.length > 0 ? `(${this.held.length}) ${this.titleBase}` : this.titleBase;
  }

  private visibilityChanged(): void {
    if (this.doc.visibilityState === 'hidden' || this.held.length === 0) return;
    const kept = this.held;
    this.held = [];
    this.paintTitle();
    const firsts = kept.slice(0, MOST_SUMMED).map((sentence) => {
      const first = firstSentence(sentence).replace(/\.$/, '');
      return /^[A-Z][a-z]/.test(first) ? first[0].toLowerCase() + first.slice(1) : first;
    });
    const more = kept.length > MOST_SUMMED ? `; and ${kept.length - MOST_SUMMED} more` : '';
    this.append(`While this tab was in the background: ${firsts.join('; ')}${more}.`);
  }
}

let shared: Announcer | null = null;

/** The page's announcer. */
export function announcer(): Announcer {
  shared ??= new Announcer();
  return shared;
}
