// The template panels' words (spec 05 section 6): a knob's range, its error, its "set by" line, and an evaluation's
// census as the one sentence said beside its floor, on the stage's summary line and under a card's picture, and as
// figures, each named by its place in the census, and headline tiles.

import { describe, expect, it } from 'vitest';
import type { KnobDeclaration } from '../../protocol/template.ts';
import {
  censusSentence, defaultWords, derivedWords, figures, headlineFigures, hintWords, invalidWords, rangeWords, setByWords,
} from './words.ts';

const width: KnobDeclaration = {
  key: 'width', label: 'Width', unit: 'mm', min: 4250, max: 255000, step: 850, options: [], default: 40800,
  description: 'The L’s extent along x.', group: 'Outline', order: 1,
};
const vtms: KnobDeclaration = {
  key: 'vtms_per_aisle', label: 'VTMs per aisle', unit: 'count', min: null, max: null, step: null,
  options: [{ value: 1, label: '1 VTM' }, { value: 2, label: '2 VTMs' }], default: 2, description: null, group: 'Storage', order: 3,
};

describe('a knob in words', () => {
  it('says its range, its default and what it changes', () => {
    expect(rangeWords(width)).toBe('4,250 to 255,000 mm, in steps of 850 mm');
    expect(hintWords(width)).toBe('4,250 to 255,000 mm, in steps of 850 mm; template default 40,800 mm. The L’s extent along x.');
    expect(rangeWords(vtms)).toBe('1 VTM or 2 VTMs');
  });

  it('refuses a value outside the declaration, off its step or not whole', () => {
    expect(invalidWords(width, 41650)).toBeNull();
    expect(invalidWords(width, 41000)).toBe('Width is 4,250 to 255,000 mm, in steps of 850 mm.');
    expect(invalidWords(width, 300000)).not.toBeNull();
    expect(invalidWords(width, null)).toMatch(/^Width is a whole number/);
    expect(invalidWords(vtms, 3)).toBe('VTMs per aisle is 1 VTM or 2 VTMs.');
  });

  it('carries no "set by" line at the template default, and names who set it otherwise', () => {
    expect(setByWords({ by: 'template', turn: null, author: null, revision: 1 }, 'dev')).toBeNull();
    expect(setByWords({ by: 'user', turn: null, author: { subject: 'dev', name: 'Developer' }, revision: 2 }, 'dev')).toBe('Set by you');
    expect(setByWords({ by: 'user', turn: null, author: { subject: 'kd', name: 'Kristina' }, revision: 2 }, 'dev')).toBe('Set by Kristina');
    expect(setByWords({ by: 'agent', turn: 2, author: null, revision: 3 }, 'dev')).toBe('Set by the agent in turn 2');
  });
});

describe('a knob its default expression gives (D120)', () => {
  const side: KnobDeclaration = {
    ...width, key: 'side', label: 'Side', default: 'max(aisles * 7650 + 10200, bays * 2750 + 9350)',
    description: 'The shell’s side, along x and along y. Fits {aisles} aisles and the stations on its walls.',
  };
  const aisles: KnobDeclaration = {
    key: 'aisles', label: 'Aisles', unit: 'count', min: 1, max: 30, step: 1, options: [], default: 4, description: null, group: 'Storage', order: 1,
  };

  it('says its default follows the other knobs, its hint keeping the sentences without braces', () => {
    expect(defaultWords(side)).toBe('follows the other knobs');
    expect(defaultWords(width)).toBe('40,800 mm');
    expect(hintWords(side)).toBe('4,250 to 255,000 mm, in steps of 850 mm; template default follows the other knobs. The shell’s side, along x and along y.');
  });

  it('says what its value fits, from its description, the knobs named in braces filled in', () => {
    const values = { side: 48450, aisles: 5 };
    expect(derivedWords(side, values, [side, aisles])).toBe('Fits 5 aisles and the stations on its walls: 48,450 mm');
    expect(derivedWords({ ...side, description: 'The shell’s side.' }, values, [side, aisles])).toBe('Follows the other knobs: 48,450 mm');
  });
});

describe('an evaluation in words', () => {
  it('names its census and its problems', () => {
    const census = { by_knob: { aisles: 4, pick_stations: 4 }, declared: {}, derived: { relay: 128, robot: 32 } };
    expect(censusSentence('L-Shape at its defaults', census, [], [])).toBe(
      'L-Shape at its defaults: 4 aisles, 4 pick_stations, 128 relays, 32 robots; no problems.',
    );
  });

  it('counts every headline figure among its figures, each at its place in the census, the tiles keeping their names', () => {
    const census = { by_knob: { aisles: 4, pick_stations: 4 }, declared: {}, derived: { relay: 128, robot: 32, slot: 1_152, cell: 2_100 } };
    expect(figures(census, []).map((figure) => [figure.key, figure.value])).toEqual([
      ['by_knob/aisles', 4], ['by_knob/pick_stations', 4],
      ['derived/relay', 128], ['derived/robot', 32], ['derived/slot', 1_152], ['derived/cell', 2_100],
    ]);
    expect(headlineFigures(census).map((tile) => [tile.key, tile.value])).toEqual([
      ['aisles', 4], ['pick-stations', 4], ['storage-slots', 1_152], ['robots', 32],
    ]);
  });
});
