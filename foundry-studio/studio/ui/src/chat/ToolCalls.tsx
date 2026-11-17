// Spec 02 section 3.1, ToolCalls and ToolRow (ruling D81): a turn's calls behind one native disclosure, closed on
// every load and every new turn, first in the turn; its summary says the count alone (D117), so a failure shows on its
// row. Each row is updated in place in the tool's own words; a panel command's row carries "Open the {Panel} panel"
// and, when the command froze its cut, "as it was at {map changeset 1}". The turn's inputs and results are read when
// their disclosure opens.

import { useState } from 'react';
import type { ProjectedCall } from '../protocol/projection.ts';
import { getJson } from '../shell/api.ts';
import { ShellLink } from '../shell/ShellLink.tsx';
import { decidedOf } from './bodies.ts';
import { useChatApi } from './chatContext.ts';
import { panelWord, type CommandLinks } from './log.ts';
import { toolCallsCount, toolRowState } from './words.ts';

function ToolRow({ call, links }: { call: ProjectedCall; links: CommandLinks | undefined }) {
  const { me, model } = useChatApi();
  let deniedBy: string | null = null;
  if (call.state === 'denied') {
    const approval = [...model.approvals.values()].find((candidate) => candidate.call === call.call);
    const decided = approval !== undefined ? decidedOf(approval) : null;
    deniedBy = decided === null || decided.author.subject === me?.subject ? 'you' : decided.author.name;
  }
  const shown = toolRowState(call, deniedBy);
  return (
    <li className="tool-row" data-state={shown.state} data-tool={call.tool}>
      <span className="tool-row__glyph" aria-hidden="true">{shown.glyph}</span>
      <span className="tool-row__summary">{call.summary}</span>
      <span className="tool-row__state">{shown.words}</span>
      {links !== undefined && <ShellLink className="tool-row__link" to={links.route}>Open the {panelWord(links.panel)} panel</ShellLink>}
      {links?.asItWas && <ShellLink className="tool-row__link" to={links.asItWas}>as it was at {links.asItWasWords ?? 'the cut it read'}</ShellLink>}
    </li>
  );
}

type Details = { state: 'closed' } | { state: 'reading' } | { state: 'read'; text: string } | { state: 'failed'; reason: string };

function TurnDetails({ turn, count }: { turn: string; count: number }) {
  const [details, setDetails] = useState<Details>({ state: 'closed' });
  const read = async () => {
    setDetails({ state: 'reading' });
    const answer = await getJson<unknown>(`/api/turns/${turn}/calls`);
    setDetails(answer.ok ? { state: 'read', text: JSON.stringify(answer.value, null, 2) } : { state: 'failed', reason: answer.reason });
  };
  return (
    <details className="turn__details" onToggle={(event) => {
      if (event.currentTarget.open && details.state === 'closed') void read();
    }}>
      <summary>Inputs and results of {toolCallsCount(count)}</summary>
      {details.state === 'reading' && <p>Reading the tool calls</p>}
      {details.state === 'read' && <pre>{details.text}</pre>}
      {details.state === 'failed' && <p>The tool calls could not be read: {details.reason}</p>}
    </details>
  );
}

export function ToolCalls({ turn, calls, links }: { turn: string; calls: ProjectedCall[]; links: Map<string, CommandLinks> }) {
  return (
    <li>
      <details className="tool-calls" data-count={calls.length}>
        <summary className="tool-calls__summary">{toolCallsCount(calls.length)}</summary>
        <ul className="tool-calls__list">
          {calls.map((call) => <ToolRow key={call.call} call={call} links={links.get(call.call)} />)}
        </ul>
        <TurnDetails turn={turn} count={calls.length} />
      </details>
    </li>
  );
}
