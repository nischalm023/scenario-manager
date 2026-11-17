// Spec 02 section 3.1, QuestionCard: its questions and Send (D117). While open, a form landmark named by its first
// question, one fieldset per item whose legend is the question in the agent's words, which carries its topic (no
// title, no topic pill); radios or checkboxes each named by its label alone and described by its one-line
// description, none checked at first, so "(recommended)" never answers for the person; no "Other" and no text field,
// whatever `other` says, since words go in the composer. Send with items unanswered marks each and puts focus on the
// first one's first option (F4); a sent card moves focus to the composer (F3). Answered or expired, it is a plain card
// of the questions and their answers, then by whom, or why not.

import { Fragment, useState, type FormEvent } from 'react';
import type { ProjectedQuestion } from '../protocol/projection.ts';
import type { ItemAnswer, QuestionItem } from '../protocol/question.ts';
import { useShell } from '../shell/context.ts';
import { answeredOf, askedOf } from './bodies.ts';
import { useChatApi } from './chatContext.ts';

function expiredWhy(endReason: string | null | undefined): string {
  return endReason === 'wait-expired' ? 'the wait ran out' : 'the turn was stopped';
}

/** A12 for a Send with items unanswered: how many still need one. */
const stillNeeded = (count: number): string =>
  (count === 1 ? 'One question still needs an answer.' : `${count} questions still need an answer.`);

/** Each question with its answer, or the questions alone when none was given on the card. */
function Asked({ items, answers }: { items: QuestionItem[]; answers: ItemAnswer[] }) {
  if (answers.length === 0) return <>{items.map((item, n) => <p className="question-card__text" key={n}>{item.text}</p>)}</>;
  return (
    <dl className="question-card__answers">
      {answers.map((answer) => (
        <Fragment key={answer.item}>
          <dt>{items[answer.item]?.text ?? `Question ${answer.item + 1}`}</dt>
          <dd>{[...answer.selected, ...(answer.other ? [answer.other] : [])].join(', ')}</dd>
        </Fragment>
      ))}
    </dl>
  );
}

export function QuestionCard({ question }: { question: ProjectedQuestion }) {
  const api = useChatApi();
  const shell = useShell();
  const asked = askedOf(question);
  const [chosen, setChosen] = useState<string[][]>(() => (asked?.items ?? []).map(() => []));
  const [unanswered, setUnanswered] = useState<number[]>([]);
  const [error, setError] = useState<string | null>(null);
  /** The answers this tab sent, shown until the journal's `question.answered` arrives. */
  const [sentHere, setSentHere] = useState<ItemAnswer[] | null>(null);
  if (asked === null) return null;
  const base = `question-${question.id}`;

  if (question.state !== 'open' || api.readOnly || sentHere !== null) {
    const answered = answeredOf(question);
    const turn = question.turn ? api.model.turns.get(question.turn) : undefined;
    let outcome: string | null = null;
    if (question.state === 'expired') outcome = `Not answered: ${expiredWhy(turn?.end_reason)}`;
    else if (answered !== null) {
      outcome = answered.in_words ? 'Answered in your message'
        : answered.author.subject === api.me?.subject ? 'Answered by you' : `Answered by ${answered.author.name}`;
    } else if (sentHere !== null) outcome = 'Answered by you';
    return (
      <div className="chat-card question-card" id={base} data-state={question.state === 'open' && sentHere !== null ? 'answered' : question.state}>
        <Asked items={asked.items} answers={answered?.answers ?? sentHere ?? []} />
        {outcome !== null && <p className="chat-card__footnote">{outcome}</p>}
      </div>
    );
  }

  const choose = (item: number, label: string, multi: boolean, on: boolean) => {
    setChosen((held) => held.map((labels, n) => {
      if (n !== item) return labels;
      if (!multi) return [label];
      return on ? [...labels, label] : labels.filter((one) => one !== label);
    }));
    setUnanswered((held) => held.filter((n) => n !== item));
  };

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const missing = chosen.flatMap((labels, n) => (labels.length > 0 ? [] : [n]));
    if (missing.length > 0) {
      setUnanswered(missing);
      document.getElementById(`${base}-${missing[0]}-0`)?.focus();
      shell.announce(stillNeeded(missing.length));
      return;
    }
    const answers = chosen.map((selected, item) => ({ item, selected }));
    const outcome = await api.answer(question, answers);
    if (outcome.ok) {
      setSentHere(answers);
      shell.focusComposer();
      shell.announce('Answers sent.');
    } else {
      setError(outcome.words);
      shell.announce(outcome.words);
    }
  };

  return (
    <form className="chat-card question-card" id={base} data-state="open" aria-labelledby={`${base}-0`} noValidate onSubmit={(event) => void submit(event)}>
      {asked.items.map((item, n) => {
        const name = `${base}-${n}`;
        const errorId = `${name}-error`;
        return (
          <fieldset className="question-card__item" key={n} aria-describedby={unanswered.includes(n) ? errorId : undefined}>
            <legend><span className="question-card__text" id={name}>{item.text}</span></legend>
            <div className="question-card__options">
              {item.options.map((option, m) => (
                <div className="option" key={option.label}>
                  <input type={item.multi ? 'checkbox' : 'radio'} id={`${name}-${m}`} name={name} value={option.label}
                    aria-describedby={`${name}-${m}-description`} checked={chosen[n].includes(option.label)}
                    onChange={(event) => choose(n, option.label, item.multi, event.currentTarget.checked)} />{' '}
                  <span>
                    <label className="option__label" htmlFor={`${name}-${m}`}>{option.label}</label>
                    <span className="option__description" id={`${name}-${m}-description`}>{option.description}</span>
                  </span>
                </div>
              ))}
            </div>
            {unanswered.includes(n) && <p className="question-card__error" id={errorId}>Choose an answer.</p>}
          </fieldset>
        );
      })}
      <div className="chat-card__actions">
        <button className="btn btn--primary" type="submit" aria-describedby={error !== null ? `${base}-act-error` : undefined}>Send</button>
      </div>
      {error !== null && <p className="chat-card__error" id={`${base}-act-error`}>{error}</p>}
    </form>
  );
}
