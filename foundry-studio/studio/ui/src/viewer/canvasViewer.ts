// A canvas's viewer, one at a time: a viewer opened on a canvas whose last viewer is still open, or still opening,
// configures it with a second GPU device, and the canvas stays black while the first device's frames are refused.
// React's strict mode runs every effect twice on mount, and a player's source can change while its viewer opens, so
// each opening waits until the canvas's last viewer is freed, and a viewer let go before it opened is freed as it
// opens.

import type { Viewer } from 'crucible-viewer';

type ViewerModule = typeof import('crucible-viewer');

/** A viewer opening on a canvas: the module and the viewer once open, or null when it was let go first. */
export interface CanvasViewer {
  opened: Promise<{ module: ViewerModule; viewer: Viewer } | null>;
  /** Let the viewer go: freed now if it is open, else as soon as it opens. */
  free(): void;
}

/** Each canvas's last viewer, settled once that viewer is freed or never opened. */
const last = new WeakMap<HTMLCanvasElement, Promise<void>>();

/** Open a viewer on `gpu`, its lines stroked on `lines`, once the canvas's last viewer is freed. */
export function openCanvasViewer(gpu: HTMLCanvasElement, lines: HTMLCanvasElement): CanvasViewer {
  let wanted = true;
  let viewer: Viewer | null = null;
  let gone = () => undefined as void;
  const before = last.get(gpu) ?? Promise.resolve();
  last.set(gpu, new Promise<void>((resolve) => (gone = resolve)));
  const opened = (async () => {
    try {
      await before;
      if (!wanted) return null;
      const module = await import('crucible-viewer');
      if (!wanted) return null;
      const opening = await module.openFloorViewer(gpu, lines);
      if (!wanted) {
        opening.free();
        return null;
      }
      viewer = opening;
      return { module, viewer: opening };
    } finally {
      if (viewer === null) gone();
    }
  })();
  // A caller whose own read failed first never awaits the opening; its failure is no one's to report then.
  opened.catch(() => undefined);
  return {
    opened,
    free: () => {
      wanted = false;
      if (viewer === null) return;
      viewer.free();
      viewer = null;
      gone();
    },
  };
}
