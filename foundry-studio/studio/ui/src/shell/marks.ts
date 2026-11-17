// The performance marks a measurement reads (the evaluation-latency chore's tests/measure/eval-latency.ts): each step
// from a knob change in the MapTemplate panel to the first frame that shows its floor. They are kept only in a session
// the measurement opened, which sets MEASURING on the window before the page loads, so a person's long-lived tab never
// accumulates them; the measurement clears the marks it read.

/** The window's flag a measurement sets before the page loads (Playwright's `addInitScript`) to have the marks kept. */
export const MEASURING = 'studioMeasuring';

/** Mark `name`, with `detail`, in a measurement's session only. */
export function mark(name: string, detail: Record<string, unknown>): void {
  if ((globalThis as unknown as Record<string, unknown>)[MEASURING] === true) performance.mark(name, { detail });
}
