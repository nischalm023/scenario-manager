// What changes the colours a floor is drawn in (spec 03 section 3.6: the tokens are read at mount and on a theme
// change, and under forced colours the system's): the system's colour scheme, forced colours turning on or off, and
// the page's own theme choice on `<html>`. Every stage and every picture of a floor follows the same three.

import { useEffect, useState } from 'react';

/** Call `changed` whenever the palette may have changed; answers how to stop. */
export function onPaletteChange(changed: () => void): () => void {
  const media = ['(prefers-color-scheme: dark)', '(forced-colors: active)'].map((query) => window.matchMedia(query));
  media.forEach((query) => query.addEventListener('change', changed));
  const themed = new MutationObserver(changed);
  themed.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'class', 'style'] });
  return () => {
    media.forEach((query) => query.removeEventListener('change', changed));
    themed.disconnect();
  };
}

/** A number that moves on every palette change, for an effect that draws in the palette to depend on. */
export function usePaletteVersion(): number {
  const [version, setVersion] = useState(0);
  useEffect(() => onPaletteChange(() => setVersion((n) => n + 1)), []);
  return version;
}
