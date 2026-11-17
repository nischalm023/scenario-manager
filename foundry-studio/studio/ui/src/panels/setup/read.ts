// A Setup page's read of what it shows: one GET, its answer held with the path it answers, so a page whose address
// moves on reads as loading until the new path's answer lands.

import { useEffect, useState } from 'react';
import { getJson } from '../../shell/api.ts';

export type Read<T> = { state: 'loading' } | { state: 'ready'; value: T } | { state: 'failed'; reason: string };

/** One GET, read again when its path changes; an answer for another path than the one asked is loading. */
export function useRead<T>(path: string | null): Read<T> {
  const [held, setHeld] = useState<{ key: string; read: Read<T> } | null>(null);
  const key = path ?? '';
  useEffect(() => {
    if (path === null) return;
    let current = true;
    void getJson<T>(path).then((answer) => {
      if (current) setHeld({ key, read: answer.ok ? { state: 'ready', value: answer.value } : { state: 'failed', reason: answer.reason } });
    });
    return () => {
      current = false;
    };
  }, [path, key]);
  return held !== null && held.key === key ? held.read : { state: 'loading' };
}
