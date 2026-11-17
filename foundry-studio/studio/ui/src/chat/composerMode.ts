// Spec 02 section 3.1, Composer's modes: which one the conversation's open turn puts the composer in. Only `sending`
// and `working` change what the composer does (Send unavailable; Stop while working); in `answering` and `approval` a
// message answers the open question in words or denies the waiting approval (lean 6), which the server decides.

import { openTurn, type LogModel } from './journal.ts';

export type ComposerMode =
  | { mode: 'new' } | { mode: 'idle' } | { mode: 'sending' }
  | { mode: 'working'; turn: string } | { mode: 'answering'; turn: string } | { mode: 'approval'; turn: string };

export function composerMode(model: LogModel, sending: boolean): ComposerMode {
  if (sending) return { mode: 'sending' };
  const open = openTurn(model);
  if (open !== null && open.state === 'running') return { mode: 'working', turn: open.id };
  if (open !== null) return { mode: open.waiting_on === 'approval' ? 'approval' : 'answering', turn: open.id };
  return model.entries.length === 0 ? { mode: 'new' } : { mode: 'idle' };
}
