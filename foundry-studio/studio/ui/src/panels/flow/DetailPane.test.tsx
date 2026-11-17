// Spec 06 section 5's clock rule in the detail pane: every simtime value, a work figure and its all-in companion, is
// a `.clock[data-clock="sim"]` with its "simtime" tag, the clock read from the protocol's discriminator; a value that
// counts in no clock carries none.

import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { FlowParameter, FlowRead } from '../../protocol/flow.ts';
import { ShellContext } from '../../shell/context.ts';
import { shellOf } from '../../testing/render.tsx';
import { DetailPane } from './DetailPane.tsx';
import { cards } from './words.ts';

afterEach(cleanup);

const PICK = 'b6a9a763-687c-4067-909b-460e986e2229';
const ALL_IN = '7.0 s operator time + 2.0 s tote swap = 9.0 s, 400 an hour per station, all-in';

const read: FlowRead = {
  scope: 'customer/demo/site/lshape-6/flow', pin: 'main', seqs: {}, exact: true, edges: [], segments: [], return_rules: [],
  plan: null, problems: [], made_from: null,
  nodes: [{
    ref: PICK, name: 'Pick stations', kind: 'stations', sectors: [], members: 4, ports: {}, made_from_template: false,
    all_in: { field: '/flow.node/work/pick_sim_ms', value: 400, unit: 'per hour', text: ALL_IN },
  }],
};

const parameter = (pointer: string, value: number, clock: FlowParameter['clock']): FlowParameter => ({
  ref: PICK, pointer, label: pointer, short_label: pointer, value, unit: clock === null ? null : 'ms', clock, sweepable: false,
});

describe('spec 06 section 5: the clock tag on every simtime value', () => {
  it('tags a simtime figure and the all-in companion, and leaves a factor untagged', () => {
    const parameters = [parameter('/flow.node/work/pick_sim_ms', 7000, 'sim'), parameter('/flow.node/intake/0/admission_factor', 4, null)];
    const { container } = render(
      <ShellContext.Provider value={shellOf()}>
        <DetailPane read={read} selected={PICK} parameters={parameters} readOnly="pinned" draftOver={null} mapRoute={() => '/'} onEdit={() => undefined} onSelect={() => undefined} />
      </ShellContext.Provider>,
    );
    const row = (pointer: string) => container.querySelector(`tr[data-pointer="${pointer}"]`);
    expect(row('/flow.node/work/pick_sim_ms')?.querySelector('.clock[data-clock="sim"] .clock__tag')?.textContent).toBe('simtime');
    expect(row('/flow.node/intake/0/admission_factor')?.querySelector('.clock')).toBeNull();
    const allIn = [...container.querySelectorAll('.clock[data-clock="sim"]')].find((clock) => clock.textContent?.startsWith(ALL_IN));
    expect(allIn?.querySelector('.clock__tag')?.textContent).toBe('simtime');
  });

  it('gives a node\'s hover card its all-in figure apart, for the card to tag', () => {
    expect(cards(read).get(PICK)).toEqual({ text: 'stations.', simtime: `${ALL_IN}.` });
  });
});
