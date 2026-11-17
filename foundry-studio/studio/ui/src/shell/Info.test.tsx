// An "i"'s words, spans wherever the "i" stands, keep their structure for assistive tech: a tip's list is a list of
// items, and each fact a term and its definition, which takes no name from the term. A tip pinned open by a click
// closes once focus or a pointer press lands outside its "i".

import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { Info, TipFact, TipItem, TipLine, TipList } from './Info.tsx';

afterEach(cleanup);

it('exposes a list of items and each fact as a term and its definition, the definition labelled by nothing', () => {
  render(
    <Info label="this field">
      <TipLine>How changes apply:</TipLine>
      <TipList><TipItem>Enter redraws at once.</TipItem><TipItem>Escape puts back the value.</TipItem></TipList>
      <TipFact term="Range">1 to 30 aisles</TipFact>
    </Info>,
  );
  fireEvent.click(screen.getByRole('button', { name: 'About this field' }));
  const tip = screen.getByRole('tooltip');
  const list = within(tip).getByRole('list');
  expect(within(list).getAllByRole('listitem').map((item) => item.textContent)).toEqual(['Enter redraws at once.', 'Escape puts back the value.']);
  expect(within(tip).getByRole('term').textContent).toBe('Range');
  expect(within(tip).getByRole('definition').textContent).toBe('1 to 30 aisles');
  expect(within(tip).queryByLabelText('Range')).toBeNull();
});

/** A pinned tip once stayed open as focus or a press moved on, out of sight behind a view that then filled the window
 * or under a dialog, and took the Escape that would have returned the view or closed the dialog. */
it('closes a pinned tip once focus or a pointer press lands outside its "i", and keeps it for a press on its words', () => {
  render(
    <>
      <Info label="this field"><TipLine>Enter redraws at once.</TipLine></Info>
      <button type="button">Fill the window</button>
    </>,
  );
  const about = screen.getByRole('button', { name: 'About this field' });
  const elsewhere = screen.getByRole('button', { name: 'Fill the window' });
  fireEvent.click(about);
  fireEvent.pointerDown(screen.getByRole('tooltip'));
  expect(about.getAttribute('aria-expanded'), 'a press on the words').toBe('true');
  fireEvent.pointerDown(elsewhere);
  expect(about.getAttribute('aria-expanded'), 'a press outside').toBe('false');
  fireEvent.click(about);
  expect(about.getAttribute('aria-expanded')).toBe('true');
  act(() => elsewhere.focus());
  expect(about.getAttribute('aria-expanded'), 'focus outside').toBe('false');
});
