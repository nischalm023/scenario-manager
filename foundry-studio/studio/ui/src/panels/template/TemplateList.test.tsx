// A map preset's card on the list (`TemplateList`) against a stub of the API: a preset whose defaults do not evaluate
// says why on its card, and what a card says only when asked (the census at the defaults, each setting's range, the
// flows, what it was built from) stands behind its "i", which another card's choice survives.

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { KnobDeclaration, TemplateCard } from '../../protocol/template.ts';
import { ShellContext } from '../../shell/context.ts';
import { shellOf } from '../../testing/render.tsx';
import { TemplateList } from './TemplateList.tsx';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const AISLES: KnobDeclaration = {
  key: 'aisles', label: 'Aisles', unit: 'count', min: 1, max: 30, step: 1, options: [], default: 4,
  description: null, group: null, order: null,
};

function card(title: string, thumbnail: TemplateCard['thumbnail']): TemplateCard {
  const slug = title.toLowerCase();
  return {
    ref: `template/site/${slug}`, title, text: `The ${title} shell`, built_from: 'the GreyOrange L layouts', tag: null,
    seqs: {}, knobs: [AISLES], flows: ['Picking', 'Putaway'], thumbnail,
  };
}

const READY = card('Rectangle', { state: 'ready', floor_key: null, census: { by_knob: { aisles: 4 }, declared: {}, derived: { robot: 32 } }, problems: [] });
const FAILED = card('Square', {
  state: 'failed', floor_key: null, census: null,
  problems: [{ severity: 'error', code: 'fit', knob: 'aisles', ref: null, text: 'Four aisles do not fit a side of 4 m.' }],
});

async function listed() {
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ items: [FAILED, READY] }), { status: 200 })));
  render(<ShellContext.Provider value={shellOf()}><TemplateList /></ShellContext.Provider>);
  await waitFor(() => expect(screen.getAllByRole('radio')).toHaveLength(2));
}

describe("a map preset's card", () => {
  it('says why its defaults do not evaluate, under its line', async () => {
    await listed();
    expect(screen.getByText('Its defaults do not evaluate: Four aisles do not fit a side of 4 m.')).not.toBeNull();
    expect(screen.queryByText(/Rectangle.*do not evaluate/)).toBeNull();
  });

  it('keeps the census, each setting\'s range, the flows and its source behind its "i"', async () => {
    await listed();
    const about = screen.getByRole('button', { name: 'About Rectangle' });
    const tip = document.getElementById(about.getAttribute('aria-controls') ?? '');
    expect(tip?.hidden).toBe(true);
    expect(tip?.textContent).toBe('Rectangle at its defaults: 4 aisles, 32 robots; no problems.'
      + 'Aisles 1 to 30Flows Picking; PutawayBuilt from the GreyOrange L layouts');
  });

  it('leaves the chosen preset chosen while the person opens and reads another card\'s "i"', async () => {
    await listed();
    const rectangle = screen.getByRole('radio', { name: 'Rectangle' }) as HTMLInputElement;
    const square = screen.getByRole('radio', { name: 'Square' }) as HTMLInputElement;
    fireEvent.click(rectangle);
    const chosen = () => document.querySelector('.pinned-foot strong')?.textContent;
    expect(chosen()).toBe('Rectangle');
    const about = screen.getByRole('button', { name: 'About Square' });
    fireEvent.click(about);
    const tip = document.getElementById(about.getAttribute('aria-controls') ?? '');
    expect(tip?.hidden, 'the "i" opened its words').toBe(false);
    fireEvent.click(tip as HTMLElement);
    expect([rectangle.checked, square.checked]).toEqual([true, false]);
    expect(chosen()).toBe('Rectangle');
    fireEvent.click(screen.getByText('The Square shell'));
    expect(chosen(), 'a click elsewhere on the card still chooses it').toBe('Square');
  });
});
