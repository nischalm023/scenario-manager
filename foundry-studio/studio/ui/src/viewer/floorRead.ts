// Spec 03 section 3.6's floor document as a mount reads it by its key (`GET /api/floors/{key}`): its bytes, or the
// sentence the stage shows with Retry instead (section 3.8). The request, its status and its body are one step, so a
// body cut off on the way is a failure in words rather than a read that never ends.

/** A floor document's bytes, or why there are none, in words. */
export type FloorRead = { ok: true; bytes: Uint8Array } | { ok: false; failed: string };

export async function readFloor(key: string, fetching: typeof fetch = fetch): Promise<FloorRead> {
  try {
    const answer = await fetching(`/api/floors/${key}`);
    if (!answer.ok) return { ok: false, failed: `The floor could not be read: the floor answered ${answer.status}.` };
    return { ok: true, bytes: new Uint8Array(await answer.arrayBuffer()) };
  } catch (failure) {
    const reason = failure instanceof TypeError ? 'the network failed' : String(failure);
    return { ok: false, failed: `The floor could not be read: ${reason}.` };
  }
}
