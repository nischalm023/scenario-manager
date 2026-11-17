// `useFocusHome`: a control its component's own state takes away while it holds focus hands focus to the home it
// names; nothing moves focus when the control stays, when focus had already gone elsewhere, when the render that took it
// away focused something else, or when the person had let go of the control before it left; and a blur that leaves
// focus on the control, as the window losing focus does, lets nothing go.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useLayoutEffect, useRef, useState } from 'react';
import { afterEach, expect, it } from 'vitest';
import { useFocusHome } from './focus.ts';

afterEach(cleanup);

/** A field that takes focus as it is drawn. */
function Field() {
  const field = useRef<HTMLInputElement>(null);
  useLayoutEffect(() => field.current?.focus(), []);
  return <input aria-label="Name" ref={field} />;
}

/** A home button, a control that "Take it away" removes, drawing `field` in its place, and a button outside what the
 * hook follows that removes it too. */
function Controls({ field = false }: { field?: boolean }) {
  const [shown, setShown] = useState(true);
  const home = useRef<HTMLButtonElement>(null);
  const handlers = useFocusHome(() => [home.current]);
  return (
    <>
      <button type="button" ref={home}>Home</button>
      <div {...handlers}>{shown && <button type="button" onClick={() => setShown(false)}>Take it away</button>}</div>
      {!shown && field && <Field />}
      <button type="button" onClick={() => setShown(false)}>Elsewhere</button>
    </>
  );
}

const control = () => screen.getByRole('button', { name: 'Take it away' });
const elsewhere = () => screen.getByRole('button', { name: 'Elsewhere' });

it('hands focus to its home as the control holding it leaves', () => {
  render(<Controls />);
  control().focus();
  fireEvent.click(control());
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Home' }));
});

it('leaves focus alone where it went before the control left', () => {
  render(<Controls />);
  control().focus();
  elsewhere().focus();
  fireEvent.click(elsewhere());
  expect(document.activeElement).toBe(elsewhere());
});

/** A form opened in the control's place, its field focused by the same render, which React tells no blur. */
it('leaves focus on what the render that took the control away focused', () => {
  render(<Controls field />);
  control().focus();
  fireEvent.click(control());
  expect(document.activeElement).toBe(screen.getByRole('textbox', { name: 'Name' }));
});

/** A click on the page's blank space blurs the control to nowhere; the control stays, then leaves for a reason of its
 * own, as a Retry does once what it asked for draws. */
it('moves no focus while the control it was blurred from stays, nor once it leaves', () => {
  const { rerender } = render(<Controls />);
  const blurred = control();
  blurred.focus();
  act(() => blurred.blur());
  rerender(<Controls />);
  expect(document.activeElement).toBe(document.body);
  fireEvent.click(elsewhere());
  expect(blurred.isConnected).toBe(false);
  expect(document.activeElement, 'the person let go of it first').toBe(document.body);
});

/** The person moved focus on, then from there to the page's blank space, before the control left. */
it('moves no focus when the control leaves after focus went from it to elsewhere, then to blank page', () => {
  render(<Controls />);
  control().focus();
  act(() => elsewhere().focus());
  act(() => elsewhere().blur());
  fireEvent.click(elsewhere());
  expect(document.activeElement).toBe(document.body);
});

/** The window losing focus blurs the control but leaves it the document's active element. */
it('still hands focus home after a blur that left focus on the control', () => {
  render(<Controls />);
  control().focus();
  fireEvent.focusOut(control());
  expect(document.activeElement).toBe(control());
  fireEvent.click(control());
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Home' }));
});
