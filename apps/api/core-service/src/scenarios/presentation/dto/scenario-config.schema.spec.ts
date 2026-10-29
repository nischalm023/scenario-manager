import { describe, expect, it } from 'vitest';
import { validScenarioConfig } from './scenario-config.fixture.js';
import { ScenarioConfigSchema } from './scenario-config.schema.js';

const withStationOverride = (stationId: string) =>
  ScenarioConfigSchema.safeParse({
    ...validScenarioConfig,
    station: {
      ...validScenarioConfig.station,
      station_level: { ...validScenarioConfig.station.station_level, overrides: { [stationId]: {} } },
    },
  });

describe('ScenarioConfigSchema station ids', () => {
  it('accepts a uuid station id', () => {
    expect(withStationOverride('3f2b8c1e-5d4a-4e6f-9a7b-1c2d3e4f5a6b').success).toBe(true);
  });

  it.each(['PPS.101', 'PPS-101'])('rejects non-uuid station id %s', (id) => {
    expect(withStationOverride(id).success).toBe(false);
  });
});
