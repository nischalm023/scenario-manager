import OpossumCircuitBreaker from "opossum";

const DEFAULT_OPTIONS: OpossumCircuitBreaker.Options = {
  timeout: 3_000,
  errorThresholdPercentage: 50,
  resetTimeout: 10_000,
  volumeThreshold: 5,
  // Returning true means "don't count this error": a 4xx is the caller's mistake, not a sick dependency.
  // Errors without a status (timeouts, network failures) still count.
  errorFilter: (e: unknown) => {
    const status = (e as { status?: unknown } | null)?.status;
    return typeof status === "number" && status < 500;
  },
};

const breakers = new Map<string, OpossumCircuitBreaker>();

/** One breaker per method. Open circuit fails fast instead of calling the wrapped method. */
export function CircuitBreaker(options: OpossumCircuitBreaker.Options = {}): MethodDecorator {
  return (target, propertyKey, descriptor) => {
    const described = descriptor as PropertyDescriptor;
    const originalMethod = described.value as (this: unknown, ...args: unknown[]) => unknown;
    const name = `${target.constructor.name}.${String(propertyKey)}`;

    described.value = function (this: unknown, ...args: unknown[]) {
      let breaker = breakers.get(name);
      if (!breaker) {
        breaker = new OpossumCircuitBreaker((call: () => unknown) => Promise.resolve(call()), {
          name,
          ...DEFAULT_OPTIONS,
          ...options,
        });
        breakers.set(name, breaker);
      }

      return breaker.fire(() => originalMethod.apply(this, args));
    };

    return descriptor;
  };
}
