// One viewer per canvas (fwd task 1MQ_ykazVlSO2G47--GCP7O7yQgHSMDdA8dPPyV9NdA, the black canvas): an opening let go
// before it opened opens nothing, and an opening waits until the canvas's last viewer is freed. A canvas has no GPU
// here, so the viewer module is a stub that says what is opened and freed, in order.

import { afterEach, expect, it, vi } from 'vitest';
import { openCanvasViewer } from './canvasViewer.ts';

const stub = vi.hoisted(() => ({ said: [] as string[], opened: 0 }));

vi.mock('crucible-viewer', () => ({
  openFloorViewer: async () => {
    const n = (stub.opened += 1);
    stub.said.push(`open ${n}`);
    return { free: () => stub.said.push(`free ${n}`) };
  },
}));

afterEach(() => {
  stub.said = [];
  stub.opened = 0;
});

const canvases = () => [document.createElement('canvas'), document.createElement('canvas')] as const;

it('opens nothing for an opening let go at once, as strict mode lets go a mount effect it runs twice', async () => {
  const [gpu, lines] = canvases();
  const first = openCanvasViewer(gpu, lines);
  first.free();
  const second = openCanvasViewer(gpu, lines);
  expect(await first.opened).toBeNull();
  expect(await second.opened).not.toBeNull();
  expect(stub.said).toEqual(['open 1']);
});

it("opens a canvas's next viewer only once its last one is freed", async () => {
  const [gpu, lines] = canvases();
  const first = openCanvasViewer(gpu, lines);
  await first.opened;
  const second = openCanvasViewer(gpu, lines);
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(stub.said, 'the second waits').toEqual(['open 1']);
  first.free();
  await second.opened;
  expect(stub.said).toEqual(['open 1', 'free 1', 'open 2']);
});
