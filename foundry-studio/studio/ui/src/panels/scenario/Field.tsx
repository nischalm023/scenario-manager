// Spec 07 section 5.2, one field of the scenario form: a native input with its label, its unit and clock in its
// description, who last set it in its hint, and its `data-ref` (`scenario:{ULID}#{pointer}`, spec 03's key grammar).
// It commits on Enter or on leaving it, the release on a choice; Escape restores the saved value; a refusal marks it
// invalid, described by its error line; a conflict keeps the person's value beside the other one with "Use mine" and
// "Keep this", the release's as its number fields' are, and focus stays in the field.

import { useState, type KeyboardEvent, type ReactNode } from 'react';
import type { FieldEntry } from '../../protocol/scenario.ts';
import type { JsonValue } from '../../protocol/serde_json/JsonValue.ts';
import { capitalised, fieldId, shownValue, storedValue, valueWords } from './words.ts';

/** What a commit did: saved, refused with what to do, or met another change of the field. */
export type CommitOutcome = { kind: 'saved' } | { kind: 'refused'; words: string } | { kind: 'conflict'; words: string };

export interface FieldProps {
  entry: FieldEntry;
  scenario: string;
  value: JsonValue | undefined;
  /** "set by you", or null when the status line says it for every field. */
  setBy: string | null;
  setter: 'agent' | 'user' | null;
  readOnly: boolean;
  /** A conflict the panel holds for this field, with the person's own value. */
  conflict: { words: string; mine: JsonValue } | null;
  commit(pointer: string, value: JsonValue): Promise<CommitOutcome>;
  resolve(pointer: string, choice: 'mine' | 'theirs'): Promise<CommitOutcome>;
  /** Extra description, such as the run's end in simtime. */
  children?: ReactNode;
}

export function NumberField({ entry, scenario, value, setBy, setter, readOnly, conflict, commit, resolve, children }: FieldProps) {
  const saved = shownValue(entry, value);
  const [text, setText] = useState(shownValue(entry, conflict?.mine ?? value));
  const [known, setKnown] = useState(saved);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const id = fieldId(entry.pointer);
  // A saved value that changed under the field (a re-read, an edit elsewhere) replaces text the person left alone.
  if (saved !== known) {
    setKnown(saved);
    if (text === known && conflict === null) setText(saved);
  }

  const send = async () => {
    if (readOnly || busy || (text === saved && conflict === null)) return;
    const read = storedValue(entry, text);
    if (!read.ok) {
      setError(read.why);
      return;
    }
    setBusy(true);
    const outcome = await commit(entry.pointer, read.value);
    setBusy(false);
    setError(outcome.kind === 'refused' ? outcome.words : null);
  };
  const settle = async (choice: 'mine' | 'theirs') => {
    setBusy(true);
    const outcome = await resolve(entry.pointer, choice);
    setBusy(false);
    setError(outcome.kind === 'refused' ? outcome.words : null);
  };
  const keys = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      void send();
    } else if (event.key === 'Escape' && text !== saved) {
      event.preventDefault();
      setText(saved);
      setError(null);
    }
  };
  const described = [entry.unit ? `${id}-unit` : '', `${id}-hint`, error ? `${id}-error` : '', conflict ? `${id}-conflict` : '']
    .filter(Boolean).join(' ');
  return (
    <div className="field" data-ref={`scenario:${scenario}#${entry.pointer}`} {...(setter ? { 'data-set-by': setter } : {})}>
      <label className="field__label" htmlFor={id}>{entry.label}</label>
      <div className="input-unit">
        <input
          className="input input--number" id={id} name={entry.pointer} type="number" step="any" value={text}
          readOnly={readOnly} aria-describedby={described} aria-invalid={error !== null || conflict !== null ? true : undefined}
          onChange={(event) => setText(event.target.value)} onKeyDown={keys} onBlur={() => void send()}
        />
        {entry.unit && <span className="input-unit__unit" id={`${id}-unit`}>{entry.unit}</span>}
      </div>
      <p className="field__hint" id={`${id}-hint`}>
        {children ?? entry.hint}
        {setBy && <> <span className="field__set-by">{capitalised(setBy)}.</span></>}
      </p>
      {error && <p className="field__error" id={`${id}-error`}>{error}</p>}
      {conflict && <ConflictLine id={id} words={conflict.words} settle={(choice) => void settle(choice)} />}
    </div>
  );
}

/** A conflict's sentence, which says the other value, and its two ways out. */
function ConflictLine({ id, words, settle }: { id: string; words: string; settle(choice: 'mine' | 'theirs'): void }) {
  return (
    <>
      <p className="field__error" id={`${id}-conflict`}>{words}</p>
      <p className="chat-card__actions">
        <button className="btn btn--small" type="button" onClick={() => settle('mine')}>Use mine</button>{' '}
        <button className="btn btn--small" type="button" onClick={() => settle('theirs')}>Keep this</button>
      </p>
    </>
  );
}

/** The release pattern: native radios in a nested fieldset, committed on a choice, those this build does not run
 * disabled and said so once; the person's choice stays checked while it saves and while a conflict holds it. */
export function ReleaseField({ entry, scenario, value, setBy, setter, readOnly, conflict, commit, resolve }: FieldProps) {
  const id = fieldId(entry.pointer);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState<string | null>(null);
  const shown = saving ?? conflict?.mine ?? value;
  const after = (outcome: CommitOutcome) => setError(outcome.kind === 'refused' ? outcome.words : null);
  const choose = async (choice: string) => {
    setSaving(choice);
    const outcome = await commit(entry.pointer, choice);
    setSaving(null);
    after(outcome);
  };
  const settle = async (choice: 'mine' | 'theirs') => after(await resolve(entry.pointer, choice));
  const runs = entry.options.filter((option) => option.accepted).map((option) => valueWords(entry, option.value));
  const limited = runs.length < entry.options.length;
  const described = [limited ? `${id}-runs` : '', error ? `${id}-error` : '', conflict ? `${id}-conflict` : '']
    .filter(Boolean).join(' ');
  return (
    <div className="field" data-ref={`scenario:${scenario}#${entry.pointer}`} {...(setter ? { 'data-set-by': setter } : {})}>
      <fieldset className="option-list" aria-describedby={described || undefined}>
        <legend>{entry.label}</legend>
        {entry.options.map((option) => {
          const [label, description] = option.words.split(': ');
          const optionId = `${id}-${option.value}`;
          return (
            <div className="option" key={option.value}>
              <input
                type="radio" id={optionId} name={entry.pointer} value={option.value} checked={shown === option.value}
                disabled={readOnly || !option.accepted} aria-describedby={`${optionId}-desc`} onChange={() => void choose(option.value)}
              />{' '}
              <span>
                <label className="option__label" htmlFor={optionId}>{label}</label>
                <span className="option__description" id={`${optionId}-desc`}>{description ?? ''}</span>
              </span>
            </div>
          );
        })}
        {limited && <p className="field__hint" id={`${id}-runs`}>Only {runs.join(' and ')} {runs.length === 1 ? 'runs' : 'run'} in this build.</p>}
        {setBy && <p className="field__hint"><span className="field__set-by">{capitalised(setBy)}.</span></p>}
        {error && <p className="field__error" id={`${id}-error`}>{error}</p>}
        {conflict && <ConflictLine id={id} words={conflict.words} settle={(choice) => void settle(choice)} />}
      </fieldset>
    </div>
  );
}

