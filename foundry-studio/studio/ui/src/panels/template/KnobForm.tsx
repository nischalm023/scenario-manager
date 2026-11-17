// Spec 05 section 6, the knob form: one `fieldset` per group with its `legend`, each knob a `label`, a number input
// with the declaration's `min`, `max` and `step` (a `select` for options), its unit, its hint (range, default and what
// it changes) behind an "i" beside the label, its "set by" line and its problems, every one named by the field's
// `aria-describedby`. A value outside
// the declaration sets `aria-invalid` and shows its error; a knob at the template's default carries no "set by" line,
// and one its default expression gave says what it fits instead (D120). An emptied field returns its knob to its
// default. There is no evaluate button (ruling D88): the panel sends a pause after the last change, or at once on Enter.

import type { KeyboardEvent } from 'react';
import type { EvaluationProblem, KnobDeclaration } from '../../protocol/template.ts';
import type { KnobSetBy } from '../../protocol/template_draft.ts';
import type { KnobSource } from '../../protocol/template_evaluation.ts';
import { Info, TipFact, TipItem, TipList } from '../../shell/Info.tsx';
import { derivedWords, hintParts, setByWords } from './words.ts';

export interface KnobFormProps {
  draft: string;
  declarations: KnobDeclaration[];
  values: Record<string, string>;
  setBy: Record<string, KnobSetBy>;
  /** The drawn evaluation's value of every knob, and where each came from. */
  evaluated: Record<string, number>;
  sources: Record<string, KnobSource>;
  me: string | null;
  /** Knobs whose change waits in the pause or evaluates: their "set by" line reads "Evaluating…". */
  evaluating: Set<string>;
  errors: Record<string, string>;
  problems: EvaluationProblem[];
  onChange(key: string, text: string): void;
  onEnter(): void;
  onEscape(key: string): void;
}

/** Groups in the order the declarations come (the API sorts them), each with its knobs. */
function groups(declarations: KnobDeclaration[]): [string, KnobDeclaration[]][] {
  const out: [string, KnobDeclaration[]][] = [];
  for (const knob of declarations) {
    const name = knob.group ?? 'Knobs';
    const held = out.find(([group]) => group === name);
    if (held) held[1].push(knob);
    else out.push([name, [knob]]);
  }
  return out;
}

export function KnobForm(props: KnobFormProps) {
  const { draft, declarations, values, setBy, evaluated, sources, me, evaluating, errors, problems } = props;
  const keyed = (event: KeyboardEvent<HTMLInputElement | HTMLSelectElement>, key: string) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      props.onEnter();
    } else if (event.key === 'Escape') {
      props.onEscape(key);
    }
  };
  return (
    <form className="knob-form" id="knob-form" aria-labelledby="knobs-title" onSubmit={(event) => event.preventDefault()}>
      <h3 className="section__title knob-form__title" id="knobs-title">
        Layout settings
        <Info label="how changes apply">
          <TipList>
            <TipItem><b>Type a value</b> and the floor redraws a moment after you stop. <b>Enter</b> redraws at once.</TipItem>
            <TipItem><b>Escape</b> puts back the value on the map.</TipItem>
            <TipItem><b>Clear a field</b> to return it to the preset&rsquo;s default.</TipItem>
            <TipItem>You and the agent edit the same values.</TipItem>
          </TipList>
        </Info>
      </h3>
      {groups(declarations).map(([group, knobs]) => (
        <fieldset className="fieldset" key={group}>
          <legend>{group}</legend>
          {knobs.map((knob) => {
            const id = `knob-${knob.key}`;
            const pending = evaluating.has(knob.key);
            const error = errors[knob.key];
            const own = problems.filter((problem) => problem.knob === knob.key);
            const derived = sources[knob.key] === 'derived' && own.length === 0 && knob.key in evaluated;
            const line = pending ? 'Evaluating…' : setByWords(setBy[knob.key], me) ?? (derived ? derivedWords(knob, evaluated, declarations) : null);
            const described = [
              `${id}-hint`, line !== null ? `${id}-by` : null, error ? `${id}-error` : null,
              ...own.map((_, n) => `${id}-problem-${n}`),
            ].filter((part): part is string => part !== null).join(' ');
            const field = {
              id, name: knob.key, value: values[knob.key] ?? '', 'aria-describedby': described,
              'aria-invalid': error ? true : undefined,
            };
            return (
              <div className="knob" data-set-by={setBy[knob.key]?.by ?? 'template'} data-ref={`template-draft:${draft}#/knobs/${knob.key}`} key={knob.key}>
                <div className="knob__head">
                  <label htmlFor={id}>{knob.label}</label>
                  <Info id={`${id}-hint`} label={knob.label}>
                    {hintParts(knob).changes !== '' && <TipFact term="What it sets">{hintParts(knob).changes}</TipFact>}
                    <TipFact term="Range">{hintParts(knob).range}</TipFact>
                    <TipFact term="Preset default">{hintParts(knob).fallback}</TipFact>
                  </Info>
                </div>
                {knob.options.length > 0 ? (
                  <select
                    className="select" {...field}
                    onChange={(event) => props.onChange(knob.key, event.target.value)}
                    onKeyDown={(event) => keyed(event, knob.key)}
                  >
                    {knob.options.map((option) => <option key={option.value} value={String(option.value)}>{option.label}</option>)}
                  </select>
                ) : (
                  <div className="input-unit">
                    <input
                      className="input input--number" type="number" min={knob.min ?? undefined} max={knob.max ?? undefined}
                      step={knob.step ?? undefined} {...field}
                      onChange={(event) => props.onChange(knob.key, event.target.value)}
                      onKeyDown={(event) => keyed(event, knob.key)}
                    />
                    {knob.unit !== 'count' && <span className="input-unit__unit">{knob.unit === 'mm' ? 'mm' : 'degrees'}</span>}
                  </div>
                )}
                {line !== null && <p className="knob__setby" id={`${id}-by`}>{line}</p>}
                {error && <p className="field__error" id={`${id}-error`}>{error}</p>}
                {own.map((problem, n) => (
                  <p className="problem" data-severity={problem.severity} data-ref={problem.ref ?? undefined} id={`${id}-problem-${n}`} key={n}>
                    {problem.text}
                  </p>
                ))}
              </div>
            );
          })}
        </fieldset>
      ))}
    </form>
  );
}
