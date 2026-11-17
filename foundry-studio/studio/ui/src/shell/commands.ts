// Spec 03 section 3.3, panel commands in the browser: a `panel.command` entry moves this browser tab only when all
// three checks hold, in order: (1) the entry is live, after its connection's `stream.live`; (2) this browser tab
// started the entry's turn; (3) for `show`, the person has not navigated here since the turn's `turn.started` arrived,
// focus is not inside `main.panel`, and Settings does not say "Offer a link in the chat"; for `highlight` and `clear`,
// the view shows the entry's panel and location and focus is not in its highlight list; for `select`, the same view
// and focus not in its detail pane. A `show` that fails only the third check becomes a suggestion in the chat. The
// highlights an address names are applied by `data-ref`.

import type { ChatSession } from '../chat/session.ts';
import type { StreamListener } from '../chat/stream.ts';
import { panelWord } from '../chat/log.ts';
import { bodyOf } from '../chat/bodies.ts';
import type { LogModel } from '../chat/journal.ts';
import { keyOf, parseRef } from './grammar.ts';
import { parseRoute, routeFor, viewIdentity, withParam, type Route } from './routes.ts';

let userNavigations = 0;

/** The person navigated in this browser tab (a link of theirs, Back or Forward); a later `show` becomes a link. */
export function noteUserNavigation(): void {
  userNavigations += 1;
}

export interface CommandContext {
  /** This browser tab's id as it stands now, null until it settles. */
  browserTab(): string | null;
  /** The route shown now. */
  route(): Route;
  navigate(to: string, cause: 'agent' | 'within'): void;
  say(sentence: string): void;
  offersLinkForShow(): boolean;
}

const focusWithin = (selector: string): boolean => document.activeElement?.closest(selector) != null;

/** Check 3 for a `show`: no navigation of the person's since the turn began here, no focus in the panel, no setting. */
function showMayMove(context: CommandContext, startedAt: number | undefined): boolean {
  return (startedAt === undefined || userNavigations === startedAt) && !focusWithin('main.panel') && !context.offersLinkForShow();
}

export function commandListener(context: CommandContext, session: () => ChatSession | null): StreamListener {
  /** The user-navigation count when each turn's `turn.started` arrived in this tab. */
  const startedAt = new Map<string, number>();
  return {
    loaded: (model) => {
      for (const turn of model.turns.values()) {
        if ((turn.state === 'running' || turn.state === 'waiting') && !startedAt.has(turn.id)) startedAt.set(turn.id, userNavigations);
      }
    },
    applied: (entry, model, phase) => {
      if (entry.kind === 'turn.started' && entry.turn !== null) {
        startedAt.set(entry.turn, userNavigations);
        return;
      }
      if (entry.kind !== 'panel.command' || phase !== 'live') return;
      const turn = entry.turn !== null ? model.turns.get(entry.turn) : undefined;
      if (turn === undefined || turn.browser_tab === null || turn.browser_tab !== context.browserTab()) return;
      const command = bodyOf(entry, 'panel.command');
      if (command === null) return;
      const shown = context.route();
      const target = parseRoute(command.route);
      const sameView = viewIdentity(shown) === viewIdentity(target);
      switch (command.action) {
        case 'show':
          if (showMayMove(context, startedAt.get(turn.id))) {
            context.navigate(command.route, 'agent');
            context.say(command.announce);
          } else {
            const held = session();
            held?.local.heldBack.add(entry.seq);
            held?.stream.changed();
            context.say(`The agent suggests the ${panelWord(command.panel)} panel; the link is in the chat.`);
          }
          return;
        case 'highlight':
        case 'clear':
          if (!sameView || focusWithin('.highlights')) return;
          context.navigate(routeFor(withParam(shown, 'highlights', command.action === 'highlight' ? entry.seq : undefined)), 'within');
          context.say(command.announce);
          return;
        case 'select':
          if (!sameView || focusWithin('.detail-pane') || command.select === null) return;
          context.navigate(routeFor(withParam(shown, 'select', command.select)), 'within');
          context.say(command.announce);
          return;
      }
    },
  };
}

/** The keys an address's `highlights` entry marks: each group's references, their pins dropped (rule 6). */
export function highlightedKeys(model: LogModel | null, seq: number | undefined): Set<string> {
  const keys = new Set<string>();
  if (model === null || seq === undefined) return keys;
  const entry = model.entries.find((held) => held.seq === seq && held.kind === 'panel.command');
  const command = bodyOf(entry, 'panel.command');
  for (const group of command?.highlight ?? []) {
    for (const ref of group.refs) {
      try {
        keys.add(keyOf(parseRef(ref)));
      } catch {
        // A reference off the grammar marks nothing; the API refuses such a command before it is journaled.
      }
    }
  }
  return keys;
}

/** Set `data-highlight="agent"` on every element under `root` whose `data-ref` is one of `keys`, and clear the rest. */
export function applyHighlights(root: ParentNode, keys: Set<string>): void {
  root.querySelectorAll<HTMLElement>('[data-ref]').forEach((element) => {
    const marked = keys.has(element.dataset.ref ?? '');
    if (marked) element.setAttribute('data-highlight', 'agent');
    else if (element.getAttribute('data-highlight') === 'agent') element.removeAttribute('data-highlight');
  });
}
