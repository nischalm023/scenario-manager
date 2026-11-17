// The Agents page of Setup: the robot fleets working a site, read from its flow graph (who carries each leg) and from
// what made the site (how many robots), one card a fleet, and the kinematics Agent Studio holds no value for yet.

import type { FlowRead } from '../../protocol/flow.ts';
import type { MapSummary } from '../../protocol/map.ts';
import { printPin } from '../../shell/grammar.ts';
import { Info, TipFact, TipLine } from '../../shell/Info.tsx';
import { useRead } from './read.ts';
import { Head, Soon, Status, type WorldRoute } from './setupPage.tsx';
import { grouped } from './words.ts';

interface Fleet { name: string; count: number | null; routes: string[]; assigned: boolean }

/** The fleets the flow names: one card each, its legs read as routes without the fleet's own name repeated. */
function fleetsOf(read: FlowRead): Fleet[] {
  const fleets = new Map<string, Fleet>();
  for (const segment of read.segments) {
    const who = segment.executor;
    if (who.kind === 'external') continue;
    const key = who.ref ?? who.name;
    const name = who.kind === 'assigned' ? `Aisle ${who.name.replace(/^the /, '').split(' ')[0]}` : who.name;
    const held = fleets.get(key) ?? { name, count: who.count ?? null, routes: [], assigned: who.kind === 'assigned' };
    const route = segment.label.includes(': ') ? segment.label.slice(segment.label.indexOf(': ') + 2) : segment.label;
    if (!held.routes.includes(route)) held.routes.push(route);
    fleets.set(key, held);
  }
  return [...fleets.values()].sort((a, b) => (a.assigned ? 1 : 0) - (b.assigned ? 1 : 0));
}

const KINEMATICS: [string, string[]][] = [
  ['Speed and locomotion', ['Speed, unloaded', 'Speed, loaded', 'Reverse speed', 'Docking speed', 'Acceleration', 'Deceleration']],
  ['Lift and alignment', ['Lift up', 'Lift down', 'Buffer entry distance']],
  ['Pick and drop', ['Pick time', 'Drop time']],
  ['Turning', ['Turn 90°', 'Turn 180°']],
];

export function AgentsPanel({ site, route }: { site: string; route: WorldRoute }) {
  const pin = printPin(route.location.pin);
  const flow = useRead<FlowRead>(`/api/flow/${site}/flow@${pin}`);
  const map = useRead<MapSummary>(`/api/map/${site}/map@${pin}`);
  return (
    <div className="home setup-page">
      <Head icon={<path d="M13 2L4 14h7l-1 8 9-12h-7z" />} title="Agents" words="The robot fleets working this site, and how they move."
        info={<Info label="this page"><TipFact term="Fleets">From the site's process flow: who carries each leg.</TipFact><TipFact term="Counts">From the preset that made the site.</TipFact><TipFact term="Kinematics">Not held by Agent Studio yet; the engine runs its defaults.</TipFact></Info>} />
      <Status read={flow} what="the fleets">
        {(read) => {
          const fleets = fleetsOf(read);
          const knobs = map.state === 'ready' ? map.value.made_from?.knobs ?? [] : [];
          const robots = map.state === 'ready' ? map.value.counts.find((row) => row.label === 'robot')?.count ?? null : null;
          const perAisle = knobs.find((knob) => knob.key.includes('vtm'))?.value ?? null;
          const aisles = knobs.find((knob) => knob.key === 'aisles')?.value ?? null;
          const countOf = (fleet: Fleet) => (fleet.assigned ? (perAisle !== null && aisles !== null ? perAisle * aisles : null) : fleet.count);
          return (
            <>
              <ul className="evaluation__summary">
                <li className="figure-tile"><span className="figure-tile__value">{robots === null ? '…' : grouped(robots)}</span><span className="figure-tile__label">Robots in all</span></li>
                {fleets.map((fleet) => (
                  <li className="figure-tile" key={fleet.name}><span className="figure-tile__value">{countOf(fleet) === null ? '?' : grouped(countOf(fleet) as number)}</span><span className="figure-tile__label">{fleet.name}</span></li>
                ))}
              </ul>
              <div className="setup-grid">
                {fleets.map((fleet) => (
                  <section className="setup-card fleet-card" key={fleet.name}>
                    <div className="setup-card__head">
                      <h3 className="setup-card__title"><span className="fleet-card__glyph" aria-hidden="true">{fleet.assigned ? '⇅' : '⇄'}</span>{fleet.name}</h3>
                      <span className="pill" data-tone="info">{fleet.assigned ? `${perAisle ?? '?'} per aisle` : 'shared fleet'}</span>
                    </div>
                    <p className="muted">{fleet.assigned
                      ? 'Each aisle has its own, moving totes between the racks and the relay stands.'
                      : 'One pool for the whole floor, carrying totes between stations and relay stands.'}</p>
                    <dl className="setup-facts">
                      <div className="setup-fact"><dt>Routes</dt><dd><ul className="setup-list">{fleet.routes.map((one) => <li key={one}>{one}</li>)}</ul></dd></div>
                      <Soon label="Motion profile" note="trapezoidal by default" />
                      <Soon label="Navigation" note="the engine's pathfinder" />
                    </dl>
                  </section>
                ))}
              </div>
              <section className="setup-card">
                <div className="setup-card__head">
                  <h3 className="setup-card__title">Kinematics <Info label="kinematics"><TipLine>Speeds, lift and turn times per fleet. Setting them here is planned; until then every run uses the engine's defaults.</TipLine></Info></h3>
                  <span className="pill">Coming soon</span>
                </div>
                <div className="kin-groups">
                  {KINEMATICS.map(([group, labels]) => (
                    <details className="kin-group" key={group}>
                      <summary>{group} <span className="muted">({labels.length})</span></summary>
                      <dl className="setup-facts setup-facts--columns">{labels.map((label) => <Soon key={label} label={label} />)}</dl>
                    </details>
                  ))}
                </div>
              </section>
            </>
          );
        }}
      </Status>
    </div>
  );
}
