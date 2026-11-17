// An "i" beside a label that opens the words a person needs only sometimes: a range, a default, what a field changes.
// The words stay in the document, hidden until asked, so a field's `aria-describedby` can still name them; the button
// opens them on hover or focus, and Escape closes them (WCAG 1.4.13), marked handled, so a filled view or a dialog
// around the "i" stays. A click pins them open, as a toggletip, until Escape, another click, or focus or a pointer
// press landing outside the "i": a tip left pinned as a view fills the window or a dialog opens would stay open out of
// sight and take the Escape that returns the view or closes the dialog. The "i" stands inside headings, labels and
// sentences, where only phrasing content is valid HTML, so its words are spans too, never paragraphs, lists or
// description lists: lines, lists of items and facts (`TipLine`, `TipList` of `TipItem`, `TipFact`), drawn as blocks by
// their classes, a list and a fact keeping their structure for assistive tech in the list, list item, term and
// definition roles.

import { useEffect, useId, useRef, useState, type ReactNode } from 'react';

export function Info({ id, label, children }: { id?: string; label: string; children: ReactNode }) {
  const own = useId();
  const tipId = id ?? `${own}-tip`;
  const info = useRef<HTMLSpanElement>(null);
  const [open, setOpen] = useState(false);
  const [pinned, setPinned] = useState(false);
  const shown = open || pinned;
  useEffect(() => {
    if (!shown) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.preventDefault();
      setOpen(false);
      setPinned(false);
    };
    document.addEventListener('keydown', escape);
    return () => document.removeEventListener('keydown', escape);
  }, [shown]);
  useEffect(() => {
    if (!pinned) return;
    // Captured, so a control that keeps a press or a focus from bubbling cannot keep a tip pinned.
    const elsewhere = (event: Event) => {
      if (!(event.target instanceof Node && info.current?.contains(event.target))) setPinned(false);
    };
    document.addEventListener('focusin', elsewhere, true);
    document.addEventListener('pointerdown', elsewhere, true);
    return () => {
      document.removeEventListener('focusin', elsewhere, true);
      document.removeEventListener('pointerdown', elsewhere, true);
    };
  }, [pinned]);
  return (
    <span className="info" ref={info} onMouseEnter={() => setOpen(true)} onMouseLeave={() => setOpen(false)}>
      <button
        className="info__button" type="button" aria-label={`About ${label}`} aria-expanded={shown} aria-controls={tipId}
        onClick={() => setPinned((was) => !was)} onFocus={() => setOpen(true)} onBlur={() => setOpen(false)}
      >
        <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
          <circle cx="12" cy="12" r="9" /><path d="M12 11v5M12 8h.01" />
        </svg>
      </button>
      <span className="info__tip" role="tooltip" id={tipId} hidden={!shown}>{children}</span>
    </span>
  );
}

/**
 * A fact of a tip: its term, then what it says, side by side. The definition takes no name from its term, which would
 * label it the way a field is labelled ("Aisles, stations" beside the Aisles field) and give two facts of one term one
 * name.
 */
export function TipFact({ term, children }: { term: ReactNode; children: ReactNode }) {
  return (
    <span className="tip-fact">
      <span className="tip-fact__term" role="term">{term}</span>{' '}
      <span className="tip-fact__words" role="definition">{children}</span>
    </span>
  );
}

/** A line of a tip: a sentence of its own. */
export function TipLine({ children }: { children: ReactNode }) {
  return <span className="tip-line">{children}</span>;
}

/** A tip's list, of `TipItem`s, drawn as bullets. */
export function TipList({ children }: { children: ReactNode }) {
  return <span className="tip-list" role="list">{children}</span>;
}

/** One item of a `TipList`. */
export function TipItem({ children }: { children: ReactNode }) {
  return <span className="tip-item" role="listitem">{children}</span>;
}
