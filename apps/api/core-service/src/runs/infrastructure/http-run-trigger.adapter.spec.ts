import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../config.js', () => ({
  appConfig: { runEngineUrl: 'http://engine.test' },
  requireConfig: (v: string | undefined) => v as string,
}));


const request = {
  runId: 'r1',
  correlationId: 'c1',
  engine: 'x',
  scenarioId: 's1',
  scenarioVersionId: 'v1',
  configUri: 'gs://cfg',
} as never;

const respond = (status: number, body: unknown = {}) =>
  vi.fn().mockResolvedValue(new Response(JSON.stringify(body), { status }));

describe('HttpRunTriggerAdapter circuit breaker', () => {
  beforeEach(() => vi.resetModules());
  afterEach(() => vi.unstubAllGlobals());

  // The breaker registry is module-global, so each test loads a fresh adapter.
  const freshAdapter = async () => {
    const mod = await import('./http-run-trigger.adapter.js');
    return new mod.HttpRunTriggerAdapter();
  };

  it.each([404, 409, 422])('does not open the circuit on repeated %i responses', async (status) => {
    const fetchMock = respond(status);
    vi.stubGlobal('fetch', fetchMock);
    const adapter = await freshAdapter();

    for (let i = 0; i < 10; i++) {
      await expect(adapter.start(request)).rejects.toMatchObject({ status });
    }

    // An open circuit would reject without calling fetch; all 10 calls reached the engine.
    expect(fetchMock).toHaveBeenCalledTimes(10);
  });

  it('opens the circuit on repeated 5xx responses', async () => {
    const fetchMock = respond(503);
    vi.stubGlobal('fetch', fetchMock);
    const adapter = await freshAdapter();

    for (let i = 0; i < 10; i++) await adapter.start(request).catch(() => undefined);

    expect(fetchMock.mock.calls.length).toBeLessThan(10);
    await expect(adapter.start(request)).rejects.toThrow(/breaker is open/i);
  });

  it('counts network/timeout errors (no status) as failures', async () => {
    const fetchMock = vi.fn().mockRejectedValue(new Error('socket hang up'));
    vi.stubGlobal('fetch', fetchMock);
    const adapter = await freshAdapter();

    for (let i = 0; i < 10; i++) await adapter.start(request).catch(() => undefined);

    expect(fetchMock.mock.calls.length).toBeLessThan(10);
  });

  it('throws a status-carrying error', async () => {
    vi.stubGlobal('fetch', respond(409));
    await expect((await freshAdapter()).start(request)).rejects.toBeInstanceOf(Error);
  });
});

