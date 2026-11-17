// Spec 05 sections 4.3 and 6, the MapTemplate panel (`/template-drafts/{d}`, mockup 03): the knobs a person and the
// agent set, the newest evaluation drawn, and Create site. A valid change waits in one 800 ms pause of wall clock for
// the whole form, which every later change restarts; Enter ends it at once; then one PATCH names every knob changed
// since the last (ruling D88). The panel follows `template.evaluation` on the conversation's stream, reading the draft
// once a second meanwhile, and draws an evaluation only when it is the newest the draft has numbered and no change of
// the person's waits, so a superseded or stale one is never drawn (section 4.3, step 8). The evaluation the person
// started is spoken once drawn; focus never moves on an evaluation. A knob nobody set shows the value the drawn
// evaluation gave it, its default expression's included, and an emptied field returns its knob to its default (D120).
// Each step from a change to its floor is a `template:` performance mark, kept only in a measurement's session.

import { useCallback, useContext, useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { KnobDeclaration } from '../../protocol/template.ts';
import type { KnobsSet, TemplateDraft } from '../../protocol/template_draft.ts';
import type { TemplateEvaluation } from '../../protocol/template_evaluation.ts';
import type { Answer } from '../../shell/api.ts';
import { useShell } from '../../shell/context.ts';
import { mark } from '../../shell/marks.ts';
import { ServicesContext } from '../../shell/panels.tsx';
import { ShellLink } from '../../shell/ShellLink.tsx';
import { CreateSiteForm } from './CreateSiteForm.tsx';
import { setShown } from './draftStore.ts';
import { EvaluationView, type Drawn } from './EvaluationView.tsx';
import { KnobForm } from './KnobForm.tsx';
import { getJson, sendJson } from './requests.ts';
import { grouped, invalidWords } from './words.ts';

/** Spec 05 section 4.3, step 2: the pause after a person's last change (lead decided, within Daniel's ~700-900 ms). */
const PAUSE_MS = 800;
const FOLLOW_MS = 1000;

const without = <T,>(set: Set<T>, item: T): Set<T> => new Set([...set].filter((held) => held !== item));

/** The fields' text with `values` written in, except for knobs whose change waits. */
function filled(held: Record<string, string>, values: Record<string, number>, waiting: Set<string>): Record<string, string> {
  const next = { ...held };
  for (const [key, value] of Object.entries(values)) {
    if (!waiting.has(key)) next[key] = String(value);
  }
  return next;
}

/** The integer defaults the form shows before the first evaluation is drawn; an expression's waits for it. */
const plainDefaults = (knobs: KnobDeclaration[]): Record<string, number> =>
  Object.fromEntries(knobs.flatMap((knob) => (typeof knob.default === 'number' ? [[knob.key, knob.default]] : [])));

/** "Evaluation 3 ready: 6 aisles, 144 relays, no problems." or "Evaluation 2 has 1 problem: Error: …". */
function outcomeSentence(evaluation: TemplateEvaluation): string {
  const { n, problems, census } = evaluation;
  if (problems.length > 0) {
    return `Evaluation ${n} has ${problems.length === 1 ? '1 problem' : `${problems.length} problems`}: ${problems[0].text}`;
  }
  const counts = [
    census?.by_knob.aisles !== undefined ? `${census.by_knob.aisles} aisles` : null,
    census?.derived.relay !== undefined ? `${grouped(census.derived.relay)} relays` : null,
    census?.derived.robot !== undefined ? `${grouped(census.derived.robot)} robots` : null,
  ].filter((part): part is string => part !== null);
  return `Evaluation ${n} ready: ${[...counts, 'no problems'].join(', ')}.`;
}

export function TemplateDraftPanel({ id }: { id: string }) {
  const shell = useShell();
  const services = useContext(ServicesContext);
  const [draft, setDraft] = useState<TemplateDraft | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [evaluation, setEvaluation] = useState<TemplateEvaluation | null>(null);
  const [drawn, setDrawn] = useState<Drawn | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [pending, setPending] = useState<Set<string>>(new Set());
  const [patching, setPatching] = useState(false);
  const [started, setStarted] = useState<{ n: number; knobs: string[] } | null>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [formError, setFormError] = useState<string | null>(null);
  const pause = useRef<ReturnType<typeof setTimeout> | null>(null);
  const awaited = useRef<number | null>(null);
  const now = useRef({ draft, pending, values, patching, evaluation });
  useLayoutEffect(() => {
    now.current = { draft, pending, values, patching, evaluation };
  });

  const apply = useCallback((read: Answer<TemplateDraft>) => {
    if (!read.ok) {
      setFailure(read.status === 404 ? 'There is no such template draft.' : read.reason);
      return;
    }
    setFailure(null);
    setDraft(read.value);
    setValues((held) => {
      const shown = Object.keys(held).length > 0 ? held : filled(held, plainDefaults(read.value.declarations), new Set());
      return filled(shown, read.value.knobs, now.current.pending);
    });
  }, []);
  const refresh = useCallback((via: 'stream' | 'poll' | 'send' | 'settled') => getJson<TemplateDraft>(`/api/template-drafts/${id}`).then((read) => {
    if (read.ok) mark('template:draft-read', { draft: id, n: read.value.latest?.n ?? null, state: read.value.latest?.state ?? null, via });
    apply(read);
  }), [id, apply]);

  useEffect(() => {
    let current = true;
    void getJson<TemplateDraft>(`/api/template-drafts/${id}`).then((read) => {
      if (current) apply(read);
    });
    return () => {
      current = false;
    };
  }, [id, apply]);

  // The stream tells the panel each evaluation's end; the draft is read once a second as well while one runs.
  useEffect(() => services?.subscribe(['template.evaluation'], (_, body) => {
    if ((body as { draft?: string } | null)?.draft === id) void refresh('stream');
  }), [services, id, refresh]);
  const running = draft?.latest?.state === 'in-progress' || patching;
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(() => void refresh('poll'), FOLLOW_MS);
    return () => clearInterval(timer);
  }, [running, refresh]);

  // The evaluation the person started is spoken once drawn (section 6): one with a floor when the mount has drawn it.
  const speak = useCallback((said: TemplateEvaluation) => {
    awaited.current = null;
    services?.announce(outcomeSentence(said));
  }, [services]);
  const onDrawn = useCallback((shown: Drawn) => {
    setDrawn(shown);
    const said = now.current.evaluation;
    if (awaited.current !== null && awaited.current <= shown.n && said?.n === shown.n) speak(said);
  }, [speak]);

  // Section 4.3, step 8: draw the newest evaluation only, and only while no change of the person's waits.
  const newest = draft?.latest ?? null;
  useEffect(() => {
    if (newest === null || newest.state === 'in-progress' || newest.state === 'superseded') return;
    if (pending.size > 0 || patching || evaluation?.n === newest.n) return;
    let current = true;
    void getJson<TemplateEvaluation>(`/api/template-drafts/${id}/evaluations/${newest.n}`).then((read) => {
      const held = now.current;
      if (!current || !read.ok || held.draft?.latest?.n !== read.value.n || held.pending.size > 0 || held.patching) return;
      mark('template:evaluation-read', { draft: id, n: read.value.n });
      setEvaluation(read.value);
      setValues((shown) => filled(shown, read.value.knobs, now.current.pending));
      if (awaited.current !== null && awaited.current <= read.value.n) {
        setStarted(null);
        if (read.value.floor === null) speak(read.value);
      }
    });
    return () => {
      current = false;
    };
  }, [newest, pending, patching, evaluation, id, speak]);

  useEffect(() => {
    if (draft !== null) setShown({ draft, n: drawn?.n ?? null });
  }, [draft, drawn]);
  useEffect(() => () => setShown(null), []);

  const send = useCallback(async () => {
    if (pause.current !== null) clearTimeout(pause.current);
    pause.current = null;
    const held = now.current;
    if (held.draft === null || held.pending.size === 0) return;
    const keys = [...held.pending];
    const knobs = Object.fromEntries(keys.map((key) => {
      const text = held.values[key]?.trim() ?? '';
      return [key, text === '' ? null : Number(text)];
    }));
    setPending(new Set());
    setPatching(true);
    setFormError(null);
    mark('template:knob-sent', { draft: id, knobs: keys });
    const answer = await sendJson<KnobsSet>('PATCH', `/api/template-drafts/${id}`, { revision: held.draft.revision, knobs });
    setPatching(false);
    if (answer.ok) {
      mark('template:knob-accepted', { draft: id, n: answer.value.evaluation.n });
      awaited.current = answer.value.evaluation.n;
      setStarted({ n: answer.value.evaluation.n, knobs: keys });
    } else if (answer.problem?.problems?.some((item) => item.knob)) {
      const refused = Object.fromEntries(answer.problem.problems.filter((item) => item.knob).map((item) => [item.knob as string, item.text]));
      setErrors((was) => ({ ...was, ...refused }));
      setValues((was) => {
        const next = { ...was };
        for (const key of Object.keys(refused)) delete next[key];
        return next;
      });
    } else {
      const sentence = answer.problem?.detail ?? answer.reason;
      setFormError(sentence);
      services?.announce(sentence);
    }
    await refresh('send');
  }, [id, refresh, services]);

  const change = (key: string, text: string) => {
    setValues((was) => ({ ...was, [key]: text }));
    const knob = draft?.declarations.find((one) => one.key === key);
    if (knob === undefined) return;
    // An emptied field asks for the knob's default, which a later evaluation fills in.
    const invalid = text.trim() === '' ? null : invalidWords(knob, Number(text));
    setErrors((was) => {
      const next = { ...was };
      if (invalid === null) delete next[key];
      else next[key] = invalid;
      return next;
    });
    if (invalid === null) mark('template:knob-changed', { draft: id, knob: key });
    const waiting = invalid === null ? new Set([...pending, key]) : without(pending, key);
    setPending(waiting);
    if (pause.current !== null) clearTimeout(pause.current);
    pause.current = waiting.size > 0 ? setTimeout(() => void send(), PAUSE_MS) : null;
  };

  const escape = (key: string) => {
    if (!pending.has(key) && errors[key] === undefined) return;
    const shownValue = evaluation?.knobs[key] ?? draft?.knobs[key];
    setValues((was) => ({ ...was, [key]: shownValue === undefined ? '' : String(shownValue) }));
    setErrors((was) => {
      const next = { ...was };
      delete next[key];
      return next;
    });
    const waiting = without(pending, key);
    setPending(waiting);
    if (waiting.size === 0 && pause.current !== null) {
      clearTimeout(pause.current);
      pause.current = null;
    }
  };

  useEffect(() => () => {
    if (pause.current !== null) clearTimeout(pause.current);
  }, []);

  const created = useCallback((site: string) => shell.navigate(`/w/${site}/map@main`, 'user'), [shell]);
  const settled = useCallback(() => void refresh('settled'), [refresh]);

  if (failure !== null && draft === null) return <p className="panel__status">This template draft cannot be read: {failure}</p>;
  if (draft === null) return <p className="panel__status" aria-busy="true">Reading the template draft</p>;

  const evaluating = new Set([...pending, ...(started?.knobs ?? [])]);
  const busy = pending.size > 0 || patching || newest?.state === 'in-progress' || (newest !== null && evaluation?.n !== newest.n);
  const firstError = evaluation?.problems.find((problem) => problem.severity === 'error');
  const unavailable = draft.state === 'writing' ? 'Creating the site…'
    : draft.state === 'created' ? 'This draft made its site.'
      : draft.state !== 'open' ? `This draft is ${draft.state}.`
        : busy ? 'Create site is unavailable while a change evaluates.'
          : Object.keys(errors).length > 0 ? 'A knob holds a value outside its declaration.'
            : evaluation === null ? 'The template is still evaluating.'
              : firstError !== undefined ? `Evaluation ${evaluation.n} has a problem: ${firstError.text}`
                : null;

  return (
    <>
      <div className="home draft-body">
      <header className="home__head page-head draft-head">
        <span className="page-head__icon" aria-hidden="true">
          <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M12 2l9 5-9 5-9-5z" /><path d="M3 12l9 5 9-5" /><path d="M3 17l9 5 9-5" />
          </svg>
        </span>
        <div>
          <p className="home__title">{draft.title} map</p>
          <p className="home__sub">Adjust the layout, check the floor it makes, then create the site.</p>
        </div>
        <ShellLink className="btn btn--pill draft-head__back" to="/templates">All map presets</ShellLink>
      </header>
      {formError !== null && <p className="field__error">{formError}</p>}
      <div className="template-layout">
        <KnobForm
          draft={id} declarations={draft.declarations} values={values} setBy={draft.set_by} me={shell.me?.subject ?? null}
          evaluated={evaluation?.knobs ?? {}} sources={evaluation?.sources ?? {}}
          evaluating={evaluating} errors={errors} problems={evaluation?.problems ?? []}
          onChange={change} onEnter={() => void send()} onEscape={escape}
        />
        <div className="evaluation" aria-busy={busy}>
          <EvaluationView draft={id} title={draft.title} evaluation={evaluation} drawn={drawn} knobs={draft.declarations} onDrawn={onDrawn} />
        </div>
      </div>
      </div>
      <footer className="pinned-foot">
        <CreateSiteForm
          draft={draft} evaluation={evaluation} unavailable={unavailable} browserTab={shell.browserTab}
          announce={(sentence) => services?.announce(sentence)} onCreated={created} onSettled={settled}
        />
      </footer>
    </>
  );
}
