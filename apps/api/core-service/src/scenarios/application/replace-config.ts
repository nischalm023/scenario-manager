import { isDeepStrictEqual } from 'node:util';
import { flattenObj } from '../presentation/dto/flatten-overrides.js';

type Plain = Record<string, unknown>;

export interface ConfigChange {
  /** Dot path without the `overrides` segment, e.g. `config.agent.outage_pct`. */
  path: string;
  to: 'custom' | 'default';
  /** The new override for `custom`; the default that applies again for `default`. */
  value: unknown;
}

const isPlain = (v: unknown): v is Plain => typeof v === 'object' && v !== null && !Array.isArray(v);

const getAt = (root: unknown, path: string[]): unknown =>
  path.reduce<unknown>((node, key) => (isPlain(node) ? node[key] : undefined), root);

const overrideLeaves = (config: Plain) =>
  Object.fromEntries(Object.entries(flattenObj(config)).filter(([p]) => p.split('.').includes('overrides')));

/**
 * Replaces a stored config with `next` and reports the override leaves that changed: a new or different
 * value is `custom`, a removed one is `default` (with the default from `next`).
 */
export function replaceConfig(previous: Plain, next: Plain) {
  const before = overrideLeaves(previous);
  const after = overrideLeaves(next);
  const changes: ConfigChange[] = [];

  for (const flatPath of new Set([...Object.keys(before), ...Object.keys(after)])) {
    if (isDeepStrictEqual(before[flatPath], after[flatPath])) continue;
    const path = flatPath.split('.');
    const overrides = path.indexOf('overrides');
    const defaultsPath = [...path.slice(0, overrides), 'defaults', ...path.slice(overrides + 1)];
    if (path[0] === 'station' && path[1] === 'station_level') defaultsPath.splice(overrides + 1, 1);
    const removed = !(flatPath in after);

    changes.push({
      path: ['config', ...path.filter((_, i) => i !== overrides)].join('.'),
      to: removed ? 'default' : 'custom',
      value: removed ? (getAt(next, defaultsPath) ?? null) : after[flatPath],
    });
  }

  const modules = Object.keys({ ...previous, ...next }).filter((k) => !isDeepStrictEqual(previous[k], next[k]));
  return { config: next, changes, modules };
}
