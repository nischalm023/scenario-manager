// A view filling the window (`useFillWindow`) with an "i" inside it: while it fills it is a region named for what fills;
// Escape that closes the "i"'s tip leaves it filled, the next Escape returning; and once it returns to its page, the
// focused control is scrolled into view.

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useFillWindow } from './fillWindow.ts';
import { Info, TipLine } from './Info.tsx';

/** Each element scrolled into view, and how: jsdom lays nothing out and has no scrollIntoView of its own. */
const scrolled = vi.fn();

beforeEach(() => {
  scrolled.mockClear();
  Element.prototype.scrollIntoView = scrolled;
});

afterEach(() => {
  cleanup();
  Reflect.deleteProperty(Element.prototype, 'scrollIntoView');
});

/** A page's view as the pages draw it: the fill's toggle, and an "i" inside what fills. */
function View() {
  const fill = useFillWindow('Drawing');
  return (
    <div {...fill.region}>
      <button type="button" aria-pressed={fill.filled} onClick={fill.toggle}>Fill the window</button>
      <Info label="the drawing"><TipLine>Solid lines are material flow.</TipLine></Info>
    </div>
  );
}

it('stays filled when Escape closes an "i" tip opened in the filled view, and returns on the next Escape', () => {
  render(<View />);
  const fill = screen.getByRole('button', { name: 'Fill the window' });
  fireEvent.click(fill);
  const about = screen.getByRole('button', { name: 'About the drawing' });
  fireEvent.click(about);
  expect(about.getAttribute('aria-expanded')).toBe('true');
  fireEvent.keyDown(about, { key: 'Escape' });
  expect(about.getAttribute('aria-expanded')).toBe('false');
  expect(fill.getAttribute('aria-pressed')).toBe('true');
  fireEvent.keyDown(about, { key: 'Escape' });
  expect(fill.getAttribute('aria-pressed')).toBe('false');
});

it('is a region named for what fills while it fills, and no region once it returns', () => {
  render(<View />);
  const fill = screen.getByRole('button', { name: 'Fill the window' });
  expect(screen.queryByRole('region')).toBeNull();
  fireEvent.click(fill);
  expect(screen.getByRole('region', { name: 'Drawing, filling the window' }).getAttribute('data-filled')).toBe('true');
  fireEvent.click(fill);
  expect(screen.queryByRole('region')).toBeNull();
});

it('scrolls the focused control into view once the view returns to its page, and nothing as it fills', () => {
  render(<View />);
  const fill = screen.getByRole('button', { name: 'Fill the window' });
  fill.focus();
  fireEvent.click(fill);
  expect(scrolled).not.toHaveBeenCalled();
  fireEvent.click(fill);
  expect(scrolled.mock.contexts).toEqual([fill]);
  expect(scrolled.mock.calls).toEqual([[{ block: 'nearest' }]]);
});
