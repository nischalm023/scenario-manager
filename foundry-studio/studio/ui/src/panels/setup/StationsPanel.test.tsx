// The Stations page against a stub of the API (`fetch` replaced; the site holds 7 s a pick, the person's draft 8 s):
// the page reads the person's open draft with the stations and shows its values over the site's, so a return to the
// page shows the draft's value and what the site holds, and an Apply refused leaves the edit on view; a failed read of
// the stations' fields fails the page with its reason.

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DraftChecked, DraftRead } from '../../protocol/changeset_draft.ts';
import type { FlowParameters, FlowRead } from '../../protocol/flow.ts';
import { ShellContext } from '../../shell/context.ts';
import { parseRoute, type Route } from '../../shell/routes.ts';
import { shellOf } from '../../testing/render.tsx';
import { StationsPanel } from './StationsPanel.tsx';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const SITE = 'customer/demo/site/rect-1';
const ADDRESS = `/w/${SITE}@main?panel=stations`;
const DRAFT = '01M5B8ZT3QHRDJVGFSAB828HYY';
const NODE = '4b483728-2c2b-4406-80c2-a535d6269e9f';
const PICK = '/flow.node/work/pick_sim_ms';
const answer = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status });

const ALL_IN = '7.0 s operator time + 2.0 s tote swap = 9.0 s, 400 an hour per station, all-in';

const FLOW: FlowRead = {
  scope: `${SITE}/flow`, pin: 'main', seqs: { [`${SITE}/flow`]: 2 }, exact: true,
  nodes: [{
    ref: NODE, name: 'Pick stations', kind: 'stations', sectors: [], members: 4, ports: {}, process: { operation: 'pick' },
    all_in: { field: PICK, value: 400, unit: 'per hour', text: ALL_IN }, made_from_template: true,
  }],
  edges: [], segments: [], return_rules: [], plan: null, problems: [], made_from: null,
};

const PARAMETERS: FlowParameters = {
  seqs: { [`${SITE}/flow`]: 2 },
  parameters: [{
    ref: NODE, pointer: PICK, label: 'Pick stations, time per pick', short_label: 'time per pick', value: 7000, unit: 'ms',
    clock: 'sim', sweepable: true,
  }],
};

const OPEN: DraftRead = {
  draft: DRAFT, site: SITE, revision: 1, state: 'drafting', stale_reason: null, summary: 'Pick stations: Operator time per pick to 8000',
  cut: { [SITE]: 1, [`${SITE}/flow`]: 2 }, ops: [{ op: 'patch', target: NODE, pointer: PICK, value: 8000 }], parts: [], problems: [],
  approval: null, applied_seqs: null,
};

const CHECKED: DraftChecked = { draft: DRAFT, revision: 1, state: 'drafting', ops_total: 1, parts: [], problems: [], minted: {} };

/** The stub: the site's flow and fields, the person's drafts (one made by the first edit), and Apply refused. */
function stubApi(drafted: boolean, fields = () => answer(PARAMETERS)) {
  vi.stubGlobal('fetch', vi.fn(async (path: string, init?: RequestInit) => {
    const method = init?.method ?? 'GET';
    if (path === `/api/flow/${SITE}/flow@main`) return answer(FLOW);
    if (path === `/api/flow-parameters/${SITE}/flow@main`) return fields();
    if (path === `/api/drafts?site=${encodeURIComponent(SITE)}&mine=1`) return answer({ items: drafted ? [OPEN] : [] });
    if (method === 'POST' && path === '/api/drafts') {
      drafted = true;
      return answer(CHECKED, 201);
    }
    if (method === 'POST' && path === `/api/drafts/${DRAFT}/apply`) {
      return answer({ type: 'validation-failed', title: 'Not applied', status: 422, detail: 'The flow would not compile.' }, 422);
    }
    return answer({ type: 'not-found', title: 'Nothing', status: 404, detail: `No route ${method} ${path}.` }, 404);
  }));
}

function renderStations(spoken: string[] = []) {
  const route = parseRoute(ADDRESS) as Extract<Route, { view: 'world' }>;
  render(<ShellContext.Provider value={shellOf(spoken, ADDRESS)}><StationsPanel site={SITE} route={route} /></ShellContext.Provider>);
}

const pickField = () => screen.getByLabelText('Operator time per pick') as HTMLInputElement;

describe('the Stations page', () => {
  it("shows the person's open draft on a return to the page, and what the site holds", async () => {
    stubApi(true);
    renderStations();
    await waitFor(() => expect(pickField().value).toBe('8'));
    const hint = screen.getByText('Your draft changes this from 7 s.');
    expect(pickField().getAttribute('aria-describedby')).toBe(hint.id);
    expect(screen.getByText(/^Without your draft:/).textContent).toBe(`Without your draft: ${ALL_IN}`);
  });

  it('keeps an edit on view when Apply is refused, the draft still holding it', async () => {
    const spoken: string[] = [];
    stubApi(false);
    renderStations(spoken);
    await waitFor(() => expect(pickField().value).toBe('7'));
    fireEvent.change(pickField(), { target: { value: '8' } });
    fireEvent.keyDown(pickField(), { key: 'Enter' });
    await waitFor(() => expect(pickField().value).toBe('8'));
    fireEvent.click(screen.getByRole('button', { name: 'Apply changes' }));
    await waitFor(() => expect(spoken).toContain('Not applied: The flow would not compile.'));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Apply changes' }).hasAttribute('disabled')).toBe(false));
    expect(pickField().value).toBe('8');
    expect(screen.getByText('Your draft changes this from 7 s.')).not.toBeNull();
  });

  it("says why when the stations' fields cannot be read, where it once drew the stations with none", async () => {
    stubApi(false, () => answer({ type: 'unavailable', title: 'Unavailable', status: 503, detail: 'world-proto-server did not answer.' }, 503));
    renderStations();
    await waitFor(() => expect(screen.getByText('The stations could not be read: world-proto-server did not answer.')).not.toBeNull());
    expect(screen.queryByText('Pick stations')).toBeNull();
  });
});
