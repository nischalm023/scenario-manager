import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { credentials } from '@grpc/grpc-js';
import { getNodeAutoInstrumentations } from '@opentelemetry/auto-instrumentations-node';
import { OTLPMetricExporter } from '@opentelemetry/exporter-metrics-otlp-grpc';
import { OTLPTraceExporter } from '@opentelemetry/exporter-trace-otlp-grpc';
import { resourceFromAttributes } from '@opentelemetry/resources';
import { PeriodicExportingMetricReader } from '@opentelemetry/sdk-metrics';
import { NodeSDK } from '@opentelemetry/sdk-node';
import { ATTR_SERVICE_NAME, ATTR_SERVICE_VERSION } from '@opentelemetry/semantic-conventions';
import { appConfig } from '../config.js';

const SERVICE_NAME = 'core-service';

function getAppVersion(): string {
  const pkgPath = resolve(import.meta.dirname, '../../package.json');
  return (JSON.parse(readFileSync(pkgPath, 'utf8')) as { version: string }).version;
}

// The gRPC exporter wants a bare host:port; env values are often copied with a scheme.
export function normalizeOtlpEndpoint(raw: string): string {
  return raw.replace(/^https?:\/\//i, '').replace(/^grpc:\/\//i, '');
}

/** Await before the server starts listening, or the earliest spans can be dropped. */
export async function initTelemetry(): Promise<void> {
  const { otel, env } = appConfig;
  if (otel.sdkDisabled) return;

  const resource = resourceFromAttributes({
    [ATTR_SERVICE_NAME]: otel.serviceName ?? SERVICE_NAME,
    [ATTR_SERVICE_VERSION]: getAppVersion(),
    'service.namespace': 'greymatter',
    'deployment.environment': otel.deploymentEnvironment ?? env,
  });

  const endpoint = normalizeOtlpEndpoint(otel.exporterEndpoint);

  const sdk = new NodeSDK({
    resource,
    traceExporter: new OTLPTraceExporter({ url: endpoint, credentials: credentials.createInsecure() }),
    metricReaders: [new PeriodicExportingMetricReader({
      exporter: new OTLPMetricExporter({ url: endpoint, credentials: credentials.createInsecure() }),
      exportIntervalMillis: otel.metricsExportInterval,
    })],
    instrumentations: [
      getNodeAutoInstrumentations({
        '@opentelemetry/instrumentation-fs': { enabled: false },
        '@opentelemetry/instrumentation-net': { enabled: false },
        '@opentelemetry/instrumentation-http': {
          ignoreIncomingRequestHook: (req) =>
            ['/health', '/ready', '/metrics'].some((p) => req.url?.startsWith(p)),
          // Don't trace the SDK's own export traffic.
          ignoreOutgoingRequestHook: (req) => /otel-collector/.test(req.hostname ?? ''),
        },
      }),
    ],
  });

  sdk.start();
  const shutdown = () => void sdk.shutdown();
  process.once('SIGTERM', shutdown);
  process.once('SIGINT', shutdown);
}
