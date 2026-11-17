// Spec 02 section 3.1, the chat log: `ol.chat__log#chat-log`, a focusable scrolling list that is never a live region
// (section 3.4), holding the items `drawLog` makes in journal order: "Show earlier entries", the welcome of an empty
// conversation, user entries ("You, for turn {n}"), turns (`h3.turn__header` "Agent", with no number or state words
// since D117, then the turn's items, the last "Working" while it runs), system entries and run cards. Headings walk
// the conversation: chat h2, user, turn and system h3, card h4 (section 5). F8: entries read back above the reading
// position move nothing, and the log follows what is added only while it reads its end.

import { useLayoutEffect, useRef, useState } from 'react';
import type { PageContext } from '../protocol/page.ts';
import { useShell } from '../shell/context.ts';
import { Info, TipFact } from '../shell/Info.tsx';
import { AgentText } from './AgentText.tsx';
import { ApprovalCard, ExecutionLine } from './ApprovalCard.tsx';
import { ChangesetCard } from './ChangesetCard.tsx';
import { useChatApi } from './chatContext.ts';
import { panelWord, type LogItem, type TurnItem, type TurnItemList, type UserItem } from './log.ts';
import { QuestionCard } from './QuestionCard.tsx';
import { RunCard } from './RunCard.tsx';
import { ShellLink } from '../shell/ShellLink.tsx';
import { ToolCalls } from './ToolCalls.tsx';
import { changesetWords } from './words.ts';

const SITE_WELCOME = [
  'I can read this site with you, change its flows through drafts you review, set up scenarios, run simulations and read the results with you. Every change I make to the site is shown to you as a draft before it is applied.',
  'Ask in your own words, or start with one of these.',
];
const THREAD_WELCOME = [
  'I can make a new site with you from a template, set up its flows and a scenario, run simulations and read the results with you. Every change I make to a site is shown to you as a draft before it is applied.',
  'Once you create the site, this conversation belongs to it: opening the site brings it back.',
  'Ask in your own words, or start with one of these.',
];

function Suggestions({ items }: { items: string[] }) {
  const api = useChatApi();
  const shell = useShell();
  return (
    <ul className="suggestions" aria-label="Suggested messages">
      {items.slice(0, 3).map((text) => (
        <li key={text}>
          <button className="btn btn--small" type="button" data-suggestion={text} disabled={api.readOnly}
            onClick={() => {
              shell.focusComposer();
              void api.send(text);
            }}>
            {text}
          </button>
        </li>
      ))}
    </ul>
  );
}

function Welcome({ thread }: { thread: boolean }) {
  return (
    <li className="entry entry--system" data-kind="welcome">
      <h3 className="entry__speaker">Agent Studio</h3>
      <div className="entry__body">
        {(thread ? THREAD_WELCOME : SITE_WELCOME).map((line) => <p key={line}>{line}</p>)}
        <Suggestions items={thread ? ['Create a site from a template', 'Show me the shell templates'] : ["Show me this site's flows", 'Which scenarios does this site have?']} />
      </div>
    </li>
  );
}

function UserEntry({ item }: { item: UserItem }) {
  const { me } = useChatApi();
  const mine = item.author === null || item.author.subject === me?.subject;
  return (
    <li className="entry entry--user" data-state={item.sending ? 'sending' : undefined}>
      <h3 className="entry__speaker">
        {mine ? 'You' : item.author?.name}
        {item.turnN !== null && <span className="visually-hidden">, for turn {item.turnN}</span>}
      </h3>
      <div className="entry__body">
        {item.text.split('\n').map((line, n) => <p key={n}>{line}</p>)}
      </div>
      {(item.sentAs !== '' || item.page !== null) && (
        <p className="entry__sent">
          {item.sentAs !== '' && <span className="entry__sent-as">{item.sentAs}</span>}
          {item.page !== null && <SentWith page={item.page} />}
        </p>
      )}
    </li>
  );
}

/**
 * The page context the agent was sent with a message, behind an "i" rather than in the bubble (ruling D190): the page's
 * heading, what was selected on it and the changesets it read, as the message carried them.
 */
function SentWith({ page }: { page: PageContext }) {
  return (
    <Info label="the page sent with this message">
      <TipFact term="Page">{page.title}</TipFact>
      {page.selection !== null && <TipFact term="Selected">{page.selection.label}</TipFact>}
      {page.seen.length > 0 && <TipFact term="It read">{page.seen.map((seen) => changesetWords(seen.scope, seen.seq)).join(', ')}</TipFact>}
    </Info>
  );
}

function SystemEntry({ kind, speaker, text }: { kind: string; speaker: string; text: string }) {
  return (
    <li className="entry entry--system" data-kind={kind}>
      <h3 className="entry__speaker">{speaker}</h3>
      <div className="entry__body"><p>{text}</p></div>
    </li>
  );
}

function TurnItemView({ item, turn }: { item: TurnItem; turn: TurnItemList }) {
  switch (item.type) {
    case 'tools': return <ToolCalls turn={turn.turn.id} calls={item.calls} links={item.links} />;
    case 'text': return <AgentText text={item.text} streaming={item.streaming} refs={item.refs} />;
    case 'question': return <li><QuestionCard question={item.question} /></li>;
    case 'approval': return <li><ApprovalCard approval={item.approval} /></li>;
    case 'run': return <li><RunCard card={item.card} /></li>;
    case 'draft': return <li><ChangesetCard draft={item.draft} execution={item.execution} applying={item.applying} /></li>;
    case 'system': return <SystemEntry kind={item.kind} speaker={item.speaker} text={item.text} />;
    case 'suggestion':
      return (
        <li className="turn__suggestion">
          The agent suggests the {panelWord(item.links.panel)} panel:{' '}
          <ShellLink to={item.links.route}>Open the {panelWord(item.links.panel)} panel</ShellLink>
          {item.links.asItWas && <>{' '}<ShellLink to={item.links.asItWas}>as it was at {item.links.asItWasWords ?? 'the cut it read'}</ShellLink></>}
        </li>
      );
  }
}

function TurnView({ item }: { item: TurnItemList }) {
  const api = useChatApi();
  const headingId = `turn-${item.turn.n}`;
  return (
    <li className="turn" data-state={item.turn.state} aria-labelledby={headingId}>
      <h3 className="turn__header" id={headingId} tabIndex={-1}><strong>Agent</strong></h3>
      <ol className="turn__items">
        {item.items.map((turnItem) => <TurnItemView key={turnItem.key} item={turnItem} turn={item} />)}
        {item.turn.state === 'running' && <li className="turn__working">Working</li>}
      </ol>
      {item.error !== null && (
        <div className="turn__error">
          <p>{item.error.reason}</p>
          {item.error.resend !== null && !api.readOnly && (
            <button className="btn btn--small" type="button" onClick={() => void api.send(item.error?.resend ?? '')}>Send it again</button>
          )}
        </div>
      )}
    </li>
  );
}

function Earlier({ start }: { start: boolean }) {
  const api = useChatApi();
  const [reading, setReading] = useState(false);
  if (start) return <li className="chat__earlier"><p>Start of the conversation</p></li>;
  return (
    <li className="chat__earlier">
      <button className="btn btn--small" type="button" aria-disabled={reading ? 'true' : undefined}
        onClick={async () => {
          if (reading) return;
          setReading(true);
          await api.loadEarlier();
          setReading(false);
        }}>
        Show earlier entries
      </button>
    </li>
  );
}

/** How near the end, in CSS px, still reads as at the end. */
const END_SLACK_PX = 8;

/** Where the log is read: whether at its end, and where its first entry stands from the log's visible top. */
interface Reading {
  atEnd: boolean;
  first: string;
  firstTop: number;
}

/** The `li` an item draws: every item draws exactly one, in order. */
const itemElement = (log: HTMLElement, items: LogItem[], key: string): HTMLElement | null =>
  (log.children[items.findIndex((item) => item.key === key)] as HTMLElement | undefined) ?? null;

function readingOf(log: HTMLElement, items: LogItem[], first: string): Reading {
  const atEnd = log.scrollHeight - log.scrollTop - log.clientHeight <= END_SLACK_PX;
  return { atEnd, first, firstTop: (itemElement(log, items, first)?.offsetTop ?? 0) - log.scrollTop };
}

export function LogView({ items, thread }: { items: LogItem[]; thread: boolean }) {
  const log = useRef<HTMLOListElement>(null);
  const reading = useRef<Reading>({ atEnd: true, first: '', firstTop: 0 });
  const first = items.find((item) => item.type !== 'earlier')?.key ?? '';
  useLayoutEffect(() => {
    const element = log.current;
    if (element === null) return;
    const before = reading.current;
    // Entries read back above keep the entry that was first where it stood; else the log follows its end, if read there.
    const anchor = before.first !== first ? itemElement(element, items, before.first) : null;
    if (anchor !== null) element.scrollTop = anchor.offsetTop - before.firstTop;
    else if (before.atEnd) element.scrollTop = element.scrollHeight;
    reading.current = readingOf(element, items, first);
  });
  return (
    <ol className="chat__log" id="chat-log" aria-label="Conversation" tabIndex={0} ref={log}
      onScroll={(event) => {
        reading.current = readingOf(event.currentTarget, items, first);
      }}>
      {items.map((item) => {
        switch (item.type) {
          case 'earlier': return <Earlier key={item.key} start={item.start} />;
          case 'welcome': return <Welcome key={item.key} thread={thread} />;
          case 'user': return <UserEntry key={item.key} item={item} />;
          case 'turn': return <TurnView key={item.key} item={item} />;
          case 'system': return <SystemEntry key={item.key} kind={item.kind} speaker={item.speaker} text={item.text} />;
          case 'run': return <li key={item.key}><RunCard card={item.card} /></li>;
          case 'execution':
            return (
              <li key={item.key} className="entry entry--system" data-kind="approval.execution">
                <h3 className="entry__speaker">Site changes</h3>
                <div className="entry__body"><ExecutionLine approval={item.approval} /></div>
              </li>
            );
        }
      })}
    </ol>
  );
}
