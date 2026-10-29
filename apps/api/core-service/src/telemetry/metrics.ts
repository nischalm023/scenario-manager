import { metrics, type Counter, type Histogram } from '@opentelemetry/api';

type Attributes = Record<string, string | number | boolean>;

const counters = new Map<string, Counter>();
const histograms = new Map<string, Histogram>();

// Resolved lazily so instruments bind to the MeterProvider registered by initTelemetry().
const getMeter = () => metrics.getMeter('core-service');

export function incrementCounter(name: string, value = 1, attributes?: Attributes): void {
  let counter = counters.get(name);
  if (!counter) {
    counter = getMeter().createCounter(name);
    counters.set(name, counter);
  }
  counter.add(value, attributes);
}

export function recordHistogram(name: string, value: number, attributes?: Attributes): void {
  let histogram = histograms.get(name);
  if (!histogram) {
    histogram = getMeter().createHistogram(name);
    histograms.set(name, histogram);
  }
  histogram.record(value, attributes);
}
