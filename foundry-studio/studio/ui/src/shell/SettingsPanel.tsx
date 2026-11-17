// Spec 02 section 3.1, Settings as a page of its own (`/settings`) rather than the popover the gear used to open:
// the same three choices this browser keeps (how much of a reply is read, the theme, what an agent `show` does),
// each in a card with its icon, a toggle for the two-way choice and radio cards for the rest. Below them the model of
// the conversation the chat last showed and the message box's key rule, which D117 moved out of the chat, and, for a
// developer only, the provider and model the server's configuration runs the agent on (ruling D122).

import type { ReactNode } from 'react';
import type { AgentModel, ModelProvider } from '../protocol/person.ts';
import { useShell } from './context.ts';
import { setPreference, usePreferences, type ReplyReading, type Theme } from './preferences.ts';

const ICON = {
  viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', strokeWidth: 2,
  strokeLinecap: 'round', strokeLinejoin: 'round', 'aria-hidden': true,
} as const;

const SPEAKER: ReactNode = <><path d="M11 5 6 9H2v6h4l5 4z" /><path d="M15.5 8.5a5 5 0 0 1 0 7" /><path d="M19 5a9 9 0 0 1 0 14" /></>;
const PALETTE: ReactNode = <><circle cx="12" cy="12" r="9" /><circle cx="8.5" cy="10" r="1.2" /><circle cx="12" cy="7.5" r="1.2" /><circle cx="15.5" cy="10" r="1.2" /><path d="M12 21a3 3 0 0 1 0-6 2 2 0 0 0 0-4" /></>;
const PANELS: ReactNode = <><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M9 4v16" /></>;
const CHIP: ReactNode = <><rect x="7" y="7" width="10" height="10" rx="2" /><path d="M10 3v4M14 3v4M10 17v4M14 17v4M3 10h4M3 14h4M17 10h4M17 14h4" /></>;

function Icon({ children, size = 20 }: { children: ReactNode; size?: number }) {
  return <svg {...ICON} width={size} height={size}>{children}</svg>;
}

interface Choice<T extends string> {
  value: T;
  words: string;
  detail?: string;
  icon?: ReactNode;
}

const READING: Choice<ReplyReading>[] = [
  { value: 'd70', words: 'Up to three sentences, else the first', detail: 'The default: short replies in full, long ones opened by their first sentence.' },
  { value: 'full', words: 'The whole reply', detail: 'Every sentence, however long the reply runs.' },
  { value: 'first', words: 'Only the first sentence', detail: 'A headline; the rest stays in the log.' },
  { value: 'off', words: 'Nothing; I read the log myself', detail: 'The reply is never spoken.' },
];
const THEME: Choice<Theme>[] = [
  { value: 'system', words: 'Follow the system', icon: <><rect x="2" y="4" width="20" height="13" rx="2" /><path d="M8 21h8M12 17v4" /></> },
  { value: 'light', words: 'Light', icon: <><circle cx="12" cy="12" r="4.5" /><path d="M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M19.1 4.9l-1.4 1.4M6.3 17.7l-1.4 1.4" /></> },
  { value: 'dark', words: 'Dark', icon: <path d="M20 14.5A8.5 8.5 0 0 1 9.5 4a8.5 8.5 0 1 0 10.5 10.5z" /> },
];

function Card({ icon, heading, blurb, children }: { icon: ReactNode; heading: string; blurb: string; children: ReactNode }) {
  return (
    <section className="setting-card">
      <div className="setting-card__head">
        <span className="setting-card__icon" aria-hidden="true"><Icon>{icon}</Icon></span>
        <div>
          <h3 className="setting-card__heading">{heading}</h3>
          <p className="setting-card__blurb">{blurb}</p>
        </div>
      </div>
      {children}
    </section>
  );
}

/** A radio group drawn as cards: the whole card is the label, so the click target is the choice. */
function Choices<T extends string>({ name, choices, chosen, choose, look = 'rows' }: {
  name: string; choices: Choice<T>[]; chosen: T; choose: (value: T) => void; look?: 'rows' | 'tiles';
}) {
  return (
    <div className={`choice-set choice-set--${look}`} role="radiogroup" aria-label={name}>
      {choices.map((choice) => (
        <label key={choice.value} className="choice" data-chosen={chosen === choice.value ? 'true' : undefined}>
          <input type="radio" name={name} value={choice.value} checked={chosen === choice.value}
            aria-label={choice.words} onChange={() => choose(choice.value)} />
          {choice.icon !== undefined && <span className="choice__icon" aria-hidden="true"><Icon size={18}>{choice.icon}</Icon></span>}
          <span className="choice__words">
            <span className="choice__title">{choice.words}</span>
            {choice.detail !== undefined && <span className="choice__detail">{choice.detail}</span>}
          </span>
          <span className="choice__mark" aria-hidden="true" />
        </label>
      ))}
    </div>
  );
}

/** The two-way choice as a switch: on means the agent switches the page itself. */
function Toggle({ label, detail, on, flip }: { label: string; detail: string; on: boolean; flip: (on: boolean) => void }) {
  return (
    <div className="toggle-row">
      <span className="toggle-row__words">
        <span className="toggle-row__title">{label}</span>
        <span className="toggle-row__detail">{detail}</span>
      </span>
      <button className="switch" type="button" role="switch" aria-checked={on} aria-label={label} onClick={() => flip(!on)}>
        <span className="switch__knob" aria-hidden="true" />
      </button>
    </div>
  );
}

const PROVIDER: Record<ModelProvider, string> = { gemini: 'Gemini', anthropic: 'Anthropic', vertex: 'Claude on Vertex AI' };

/** The server's model for the agent, read only: on the page for a developer, absent for anyone else. */
function AgentCard({ agent }: { agent: AgentModel }) {
  return (
    <Card icon={CHIP} heading="Agent (developers only)" blurb="Set by the server's configuration; a conversation keeps the provider it began with.">
      <dl className="settings__agent">
        <dt>Provider</dt>
        <dd>{PROVIDER[agent.provider]}</dd>
        <dt>Model</dt>
        <dd>{agent.model}</dd>
      </dl>
    </Card>
  );
}

/** `modelLabel`: the model of the conversation the chat last showed in this page's life, which the shell holds. */
export function SettingsPanel({ modelLabel }: { modelLabel: string | null }) {
  const chosen = usePreferences();
  const { me } = useShell();
  return (
    <div className="settings-page" id="settings">
      <p className="settings-page__lede">These choices are kept in this browser only.</p>
      <div className="settings-page__cards">
        <Card icon={PALETTE} heading="Theme" blurb="How Agent Studio is lit.">
          <Choices name="Theme" choices={THEME} chosen={chosen.theme} look="tiles"
            choose={(value) => setPreference('theme', value)} />
        </Card>
        <Card icon={PANELS} heading="When the agent opens a panel" blurb="What a show from the agent does to the page you are on.">
          <Toggle label="Switch to it" on={chosen.agentShow === 'switch'}
            detail="On, the page follows the agent at once; off, a link is offered in the chat and the page stays where it is."
            flip={(on) => setPreference('agentShow', on ? 'switch' : 'link')} />
        </Card>
        <Card icon={SPEAKER} heading="When the agent finishes a reply, your screen reader hears" blurb="How much of a finished reply is spoken aloud.">
          <Choices name="When the agent finishes a reply, your screen reader hears" choices={READING}
            chosen={chosen.replyReading} choose={(value) => setPreference('replyReading', value)} />
        </Card>
        <Card icon={CHIP} heading="Chat" blurb="The model of the conversation you last had open, and the message box's key rule.">
          <dl className="settings__agent">
            {modelLabel !== null && <><dt>Model</dt><dd className="settings__model">{modelLabel}</dd></>}
            <dt>Message box</dt>
            <dd className="settings__help">Enter sends; Shift and Enter starts a new line.</dd>
          </dl>
        </Card>
        {me?.developer === true && me.agent !== undefined && <AgentCard agent={me.agent} />}
      </div>
    </div>
  );
}
