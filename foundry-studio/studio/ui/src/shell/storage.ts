// Spec 02 section 3.8, "Browser storage refused": every read and write of `localStorage` and `sessionStorage` goes
// through here, inside `try`, so a private window that refuses storage still renders and the defaults apply.

type Area = 'local' | 'session';

function area(which: Area): Storage | null {
  try {
    return which === 'local' ? window.localStorage : window.sessionStorage;
  } catch {
    return null;
  }
}

export function readStored(which: Area, key: string): string | null {
  try {
    return area(which)?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

export function writeStored(which: Area, key: string, value: string | null): void {
  try {
    const store = area(which);
    if (value === null) store?.removeItem(key);
    else store?.setItem(key, value);
  } catch {
    // Refused storage keeps nothing; the page goes on with what it holds.
  }
}
