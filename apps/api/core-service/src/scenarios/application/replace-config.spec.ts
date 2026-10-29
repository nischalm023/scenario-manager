import { describe, expect, it } from 'vitest';
import { replaceConfig } from './replace-config.js';

const station = '3f2b8c1e-5d4a-4e6f-9a7b-1c2d3e4f5a6b';

const previous = {
  agent: { defaults: { outage_pct: 5 }, overrides: { outage_pct: 3 } },
  wms_order: { defaults: { order_cutoff: '18:00' }, overrides: { order_cutoff: '20:00' } },
  station: { station_level: { defaults: { structure: { front_positions: 2 } }, overrides: {} } },
};

describe('replaceConfig', () => {
  it('returns the new config and reports set, changed and reset overrides', () => {
    const next = {
      agent: { defaults: { outage_pct: 5 }, overrides: { outage_pct: 8 } },
      wms_order: { defaults: { order_cutoff: '18:00' }, overrides: {} },
      station: {
        station_level: {
          defaults: { structure: { front_positions: 2 } },
          overrides: { [station]: { structure: { front_positions: 4 } } },
        },
      },
    };
    const result = replaceConfig(previous, next);

    expect(result.config).toBe(next);
    expect(result.modules).toEqual(['agent', 'wms_order', 'station']);
    expect(result.changes).toEqual(
      expect.arrayContaining([
        { path: 'config.agent.outage_pct', to: 'custom', value: 8 },
        { path: 'config.wms_order.order_cutoff', to: 'default', value: '18:00' },
        { path: `config.station.station_level.${station}.structure.front_positions`, to: 'custom', value: 4 },
      ]),
    );
    expect(result.changes).toHaveLength(3);
  });

  it('reports nothing when the config is unchanged', () => {
    expect(replaceConfig(previous, structuredClone(previous))).toMatchObject({ changes: [], modules: [] });
  });
});
