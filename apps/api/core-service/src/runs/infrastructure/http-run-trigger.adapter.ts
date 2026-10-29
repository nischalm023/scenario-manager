import { Injectable } from '@nestjs/common';
import { CircuitBreaker } from 'shared';
import { appConfig, requireConfig } from '../../config.js';
import type { RunTrigger, RunTriggerRequest } from '../application/ports/run-trigger.port.js';

/** Carries the HTTP status so the circuit breaker can tell client errors from engine failures. */
export class RunEngineHttpError extends Error {
  constructor(readonly status: number) {
    super(`run engine responded ${status}`);
  }
}

@Injectable()
export class HttpRunTriggerAdapter implements RunTrigger {
  @CircuitBreaker()
  async start(request: RunTriggerRequest): Promise<{ engineRef: string }> {
    const base = requireConfig(appConfig.runEngineUrl, 'RUN_ENGINE_URL').replace(/\/$/, '');
    const res = await fetch(`${base}/runs`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(request),
      signal: AbortSignal.timeout(5_000),
    });
    if (!res.ok) throw new RunEngineHttpError(res.status);
    const body = (await res.json()) as { engineRef?: string; id?: string };
    const engineRef = body.engineRef ?? body.id;
    if (!engineRef) throw new Error('run engine returned no engineRef');
    return { engineRef };
  }
}
