// Ruling D121: a floor shown as a picture, the Template list's card thumbnail and one template's default evaluation,
// drawn by the one hidden viewer (`snapshots.ts`) and shown as an `img` named by what it shows in words (spec 05 section
// 6: the census, which the card also lists), with a placeholder of the same shape while it is drawn and the reason in
// words if it cannot be.

import { useEffect, useRef, useState } from 'react';
import { usePaletteVersion } from './paletteChanges.ts';
import { floorPicture } from './snapshots.ts';

export function FloorSnapshot({ floorKey, label, className }: { floorKey: string; label: string; className: string }) {
  const [drawn, setDrawn] = useState<{ key: string; url: string | null; failed: string | null } | null>(null);
  const holder = useRef<HTMLSpanElement>(null);
  const palette = usePaletteVersion();
  useEffect(() => {
    let alive = true;
    void floorPicture(floorKey, holder.current ?? document.documentElement).then(
      (url) => alive && setDrawn({ key: floorKey, url, failed: null }),
      (failure: unknown) => alive && setDrawn({ key: floorKey, url: null, failed: String(failure) }),
    );
    return () => {
      alive = false;
    };
  }, [floorKey, palette]);
  const shown = drawn?.key === floorKey ? drawn : null;
  if (shown?.url) return <img className={className} src={shown.url} alt={label} />;
  return (
    <span className={`${className} floor-snapshot--waiting`} ref={holder} role="img" aria-label={label} aria-busy={shown === null}>
      {shown?.failed !== undefined && shown.failed !== null && <span className="visually-hidden">The picture could not be drawn: {shown.failed}</span>}
    </span>
  );
}
