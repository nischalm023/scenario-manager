// Spec 03 section 3.5's marks on the Map panel, gathered from several references into one highlight group: each
// entity once, in the order the API answers every group in, so a list that says "in order along the floor" is.

import type { Mark, RefMarks } from '../../protocol/map.ts';

/** By centre, x then y, then by key; a mark that stands nowhere after every placed one (the API's `in_mark_order`). */
function markOrder(a: Mark, b: Mark): number {
  if ((a.centre_mm === null) !== (b.centre_mm === null)) return a.centre_mm === null ? 1 : -1;
  if (a.centre_mm !== null && b.centre_mm !== null) {
    const along = a.centre_mm[0] - b.centre_mm[0] || a.centre_mm[1] - b.centre_mm[1];
    if (along !== 0) return along;
  }
  return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
}

/** The marks of `refs` as one group, each entity once, in mark order. */
export function combinedMarks(refs: RefMarks[]): Mark[] {
  return [...new Map(refs.flatMap((ref) => ref.marks).map((mark) => [mark.key, mark])).values()].sort(markOrder);
}
