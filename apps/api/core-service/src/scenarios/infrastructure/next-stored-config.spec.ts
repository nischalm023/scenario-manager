import { describe, expect, it, vi } from 'vitest';
import { nextStoredConfig } from './drizzle-scenario.repository.js';

vi.mock('../../db/drizzle.module.js', () => ({ DRIZZLE: Symbol('DRIZZLE') }));

const stored = { configUri: 'gs://b/old.json', body: { agent: { overrides: { outage_pct: 5 } } }, version: 3 };

describe('nextStoredConfig', () => {
  it('stores the patched body, keeps the uri and bumps the version', () => {
    const patchConfig = vi.fn(() => ({ patched: true }));
    const next = nextStoredConfig(stored, { patchConfig, updatedBy: 'u' });
    expect(patchConfig).toHaveBeenCalledWith(stored.body);
    expect(next).toMatchObject({ configUri: 'gs://b/old.json', body: { patched: true }, version: 4, updatedBy: 'u' });
  });

  it('starts at version 2 when nothing was stored inline', () => {
    const next = nextStoredConfig({ configUri: 'gs://b/old.json' }, { config: { a: 1 }, updatedBy: 'u' });
    expect(next).toMatchObject({ body: { a: 1 }, version: 2 });
  });

  it('keeps the body and version on a rename', () => {
    const next = nextStoredConfig(stored, { name: 'New', updatedBy: 'u' });
    expect(next).toMatchObject({ body: stored.body, version: 3, updatedBy: 'u' });
  });
});
