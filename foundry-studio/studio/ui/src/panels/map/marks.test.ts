// A highlight group's marks gathered from several references (spec 03 section 3.5): each entity once, by centre, x
// then y, then by key, the marks that stand nowhere after every placed one, whatever order the references came in.

import { describe, expect, it } from 'vitest';
import type { Mark, RefMarks } from '../../protocol/map.ts';
import { combinedMarks } from './marks.ts';

const mark = (key: string, centre_mm: [number, number] | null): Mark => ({ key, label: key, components: ['station'], centre_mm, on_floor: centre_mm !== null });
const ref = (key: string, marks: Mark[]): RefMarks => ({ ref: key, key, scope: null, label: key, components: [], marks });

describe('a group of several references', () => {
  it('lists its marks along the floor, each once, whatever order the references came in', () => {
    const right = mark('b-right', [30_000, 4_000]);
    const left = mark('c-left', [10_000, 4_000]);
    const lower = mark('d-lower', [10_000, 2_000]);
    const tied = mark('a-tied', [10_000, 2_000]);
    const nowhere = mark('a-nowhere', null);
    const keys = combinedMarks([ref('r1', [right, nowhere]), ref('r2', [left, lower]), ref('r3', [tied, right])]).map((one) => one.key);
    expect(keys).toEqual(['a-tied', 'd-lower', 'c-left', 'b-right', 'a-nowhere']);
  });
});
