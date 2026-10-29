import { describe, expect, it, vi } from 'vitest';
import { UpdateScenarioUseCase } from './update-scenario.use-case.js';

const scenario = { id: 's1', assertEditable: vi.fn() };

const setup = (
  current: { configUri: string; config?: Record<string, unknown> },
  blob: Record<string, unknown> = { agent: { defaults: { outage_pct: 1 }, overrides: {} } },
) => {
  const read = vi.fn(async () => blob);
  const update = vi.fn(
    async (_id: string, changes: { patchConfig?: (c: Record<string, unknown> | undefined) => unknown }) => {
      changes.patchConfig?.(current.config);
      return scenario;
    },
  );
  const repo = { findById: async () => scenario, findCurrentVersion: vi.fn(async () => current), update };
  const useCase = new UpdateScenarioUseCase(repo as never, { read } as never);
  return { useCase, read, update, repo };
};

const next = { agent: { defaults: { outage_pct: 1 }, overrides: { outage_pct: 8 } } };

describe('UpdateScenarioUseCase', () => {
  it('replaces the stored config with the one sent', async () => {
    const { useCase, update } = setup({ configUri: '', config: { agent: { defaults: { outage_pct: 1 }, overrides: {} } } });
    await useCase.execute({ id: 's1', config: next, updatedBy: 'u' });
    const { patchConfig } = update.mock.calls[0][1];
    expect(patchConfig?.({ anything: true })).toEqual(next);
  });

  it('reports changed overrides against the stored inline config without reading the blob', async () => {
    const { useCase, read } = setup({ configUri: 'gs://b/a.json', config: { agent: { defaults: { outage_pct: 1 }, overrides: {} } } });
    const result = await useCase.execute({ id: 's1', config: next, updatedBy: 'u' });
    expect(read).not.toHaveBeenCalled();
    expect(result.changes).toEqual([{ path: 'config.agent.outage_pct', to: 'custom', value: 8 }]);
    expect(result.modules).toEqual(['agent']);
  });

  it('diffs against the blob when no inline config is stored', async () => {
    const { useCase, read } = setup({ configUri: 'gs://b/a.json' });
    const result = await useCase.execute({ id: 's1', config: next, updatedBy: 'u' });
    expect(read).toHaveBeenCalledWith('gs://b/a.json');
    expect(result.changes).toHaveLength(1);
  });

  it('reports a removed override as reset to the default', async () => {
    const { useCase } = setup({ configUri: '', config: next });
    const result = await useCase.execute({
      id: 's1',
      config: { agent: { defaults: { outage_pct: 1 }, overrides: {} } },
      updatedBy: 'u',
    });
    expect(result.changes).toEqual([{ path: 'config.agent.outage_pct', to: 'default', value: 1 }]);
  });

  it('only renames when no config is sent', async () => {
    const { useCase, read, update, repo } = setup({ configUri: 'gs://b/a.json' });
    const result = await useCase.execute({ id: 's1', name: 'New', updatedBy: 'u' });
    expect(repo.findCurrentVersion).not.toHaveBeenCalled();
    expect(read).not.toHaveBeenCalled();
    expect(update.mock.calls[0][1]).toMatchObject({ name: 'New', patchConfig: undefined });
    expect(result.changes).toEqual([]);
  });
});
