// The run page's player cursor (spec 08 section 5), shared with the metric panels that draw it on their plots: the
// player sets it as it reads the viewer, four times a second of wall clock, and each plot reads it as it draws. It is a
// store rather than the page's state, so a moving cursor redraws the plots and renders nothing.

/** An instant on a chart's x axis, read as the chart draws; `subscribe` takes what redraws the chart when the instant
 * moves, as `useSyncExternalStore` takes it, and gives back what unsubscribes it. */
export interface Cursor {
  get(): number | null;
  subscribe(moved: () => void): () => void;
}

/** The player's cursor in simtime, null until the player first reads one; `set` tells every subscriber once per new
 * instant. */
export function playerCursor(): Cursor & { set(ms: number): void } {
  let at: number | null = null;
  const subscribers = new Set<() => void>();
  return {
    get: () => at,
    subscribe: (moved) => {
      subscribers.add(moved);
      return () => subscribers.delete(moved);
    },
    set: (ms) => {
      if (ms === at) return;
      at = ms;
      for (const moved of subscribers) moved();
    },
  };
}
