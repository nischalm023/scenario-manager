// Spec 03 section 3.3, "Which browser tab acts", and its statement 4: a duplicated browser tab, which copies the
// first one's `sessionStorage`, settles an id of its own before its conversation opens; an answer that comes after
// the wait still gives it a new id, and tells it.

import { describe, expect, it } from 'vitest';
import { BrowserTab, type ChannelLike, type TabMessage } from './browserTab.ts';

/** Pages joined by one channel; a page's delivery can be held back, as a busy page answers late. */
function channels() {
  const members: { channel: ChannelLike; held: boolean; queue: TabMessage[] }[] = [];
  const join = () => {
    const member = { channel: null as unknown as ChannelLike, held: false, queue: [] as TabMessage[] };
    member.channel = {
      onmessage: null,
      postMessage: (message) => {
        for (const other of members) {
          if (other === member) continue;
          if (other.held) other.queue.push(message);
          else queueMicrotask(() => other.channel.onmessage?.({ data: message } as MessageEvent<TabMessage>));
        }
      },
    };
    members.push(member);
    return member;
  };
  const release = (member: { channel: ChannelLike; held: boolean; queue: TabMessage[] }) => {
    member.held = false;
    member.queue.splice(0).forEach((message) => member.channel.onmessage?.({ data: message } as MessageEvent<TabMessage>));
  };
  return { join, release };
}

/** A page whose `sessionStorage` holds `stored`. */
function pageOf(member: { channel: ChannelLike }, stored: { id: string | null }, waitMs = 20) {
  return new BrowserTab({ read: () => stored.id, write: (id) => { stored.id = id; }, channel: () => member.channel, waitMs });
}

describe('spec 03 section 3.3: which browser tab acts', () => {
  /**
   * Spec 03 section 3.3, which browser tab acts: a duplicate settles an id of its own before its conversation opens.
   */
  it('gives a duplicated browser tab, which copied the first one\'s id, an id of its own before it settles', async () => {
    const pages = channels();
    const first = { id: null as string | null };
    const a = pageOf(pages.join(), first);
    const idA = await a.settle();
    const copied = { id: idA };
    const b = pageOf(pages.join(), copied);
    const idB = await b.settle();
    expect(idB).not.toBe(idA);
    expect(copied.id).toBe(idB);
    expect(a.id()).toBe(idA);
  });

  /** Spec 03 section 3.3, which browser tab acts: a reload keeps its id. */
  it('keeps the id a reload finds when no other page holds it', async () => {
    const pages = channels();
    const stored = { id: '87202cd6-c458-4b48-95b6-cc86614adfd3' };
    expect(await pageOf(pages.join(), stored).settle()).toBe('87202cd6-c458-4b48-95b6-cc86614adfd3');
  });

  /** Spec 03 section 3.3, which browser tab acts: an answer after the wait still changes the id, and says so. */
  it('changes an id settled before the first page answered, and tells it', async () => {
    const pages = channels();
    const memberA = pages.join();
    const a = pageOf(memberA, { id: null });
    const idA = await a.settle();
    memberA.held = true;
    const b = pageOf(pages.join(), { id: idA });
    expect(await b.settle()).toBe(idA);
    const told: string[] = [];
    b.onChange((id) => told.push(id));
    pages.release(memberA);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(told).toHaveLength(1);
    expect(told[0]).not.toBe(idA);
    expect(b.id()).toBe(told[0]);
    expect(a.id()).toBe(idA);
  });
});
