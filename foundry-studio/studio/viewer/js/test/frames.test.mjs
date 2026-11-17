// `attachFrames` against a stand-in WebSocket and a stand-in viewer, in plain Node: the attach it sends, one frame
// per binary message pushed in order, every keyframe after the first held back, `lagged` answered by a second
// attachment from the named instant with the viewer's window cleared, `ended` (with its outcome) and a refused frame
// reported once, and nothing reported after `close()`. It needs `npm run build:ts`, which `npm test` runs first.

import assert from 'node:assert/strict';
import { beforeEach, test } from 'node:test';
import { attachFrames } from '../dist/frames.js';

/** Every socket the code under test opened, newest last; a test drives them. */
const sockets = [];

class FakeSocket {
  constructor(url) {
    this.url = url;
    this.sent = [];
    this.closed = false;
    this.listeners = new Map();
    sockets.push(this);
  }
  addEventListener(name, listener) {
    this.listeners.set(name, [...(this.listeners.get(name) ?? []), listener]);
  }
  send(text) {
    this.sent.push(JSON.parse(text));
  }
  close() {
    this.closed = true;
  }
  fire(name, data) {
    for (const listener of this.listeners.get(name) ?? []) listener({ data });
  }
  opens() {
    this.fire('open');
  }
  says(reply) {
    this.fire('message', JSON.stringify(reply));
  }
  sends(tag, ...rest) {
    this.fire('message', new Uint8Array([tag, ...rest]).buffer);
  }
}

globalThis.WebSocket = FakeSocket;

/** A viewer that remembers the first byte (the tag) of every frame pushed, and when its window was cleared. */
function sink() {
  return {
    pushed: [],
    cleared: 0,
    push_frame(bytes) {
      this.pushed.push(bytes[0]);
    },
    clear() {
      this.cleared += 1;
    },
  };
}

beforeEach(() => {
  sockets.length = 0;
});

test('attaches from the instant asked and pushes each frame once, in order', () => {
  const viewer = sink();
  const seen = [];
  attachFrames('ws://x/frames/r/1', viewer, { from: 'live', sections: ['playback'], onFrame: (tag) => seen.push(tag) });
  const [socket] = sockets;
  socket.opens();
  assert.deepEqual(socket.sent, [{ type: 'attach', id: '1', from: 'live', sections: ['playback'] }]);
  socket.sends(1);
  socket.sends(3);
  socket.sends(4);
  socket.sends(4);
  assert.deepEqual(viewer.pushed, [1, 3, 4, 4]);
  assert.deepEqual(seen, ['scenario', 'keyframe', 'delta', 'delta']);
});

test('sends the watch ticket in the attach when it has one', () => {
  attachFrames('ws://x/frames/r/1', sink(), { token: 'a-ticket' });
  const [socket] = sockets;
  socket.opens();
  assert.deepEqual(socket.sent, [{ type: 'attach', id: '1', from: 0, sections: [], token: 'a-ticket' }]);
});

test('holds back every keyframe after the first, so the window grows rather than starting again', () => {
  const viewer = sink();
  attachFrames('ws://x/frames/r/1', viewer);
  const [socket] = sockets;
  socket.opens();
  assert.deepEqual(socket.sent[0].from, 0, 'no `from` asks for the start');
  socket.sends(1);
  socket.sends(3);
  socket.sends(4);
  socket.sends(3);
  socket.sends(4);
  assert.deepEqual(viewer.pushed, [1, 3, 4, 4]);
});

test('answers lagged with a second attachment from the named instant, the window cleared first', () => {
  const viewer = sink();
  const lagged = [];
  const attached = [];
  attachFrames('ws://x/frames/r/1', viewer, {
    onLagged: (ms) => lagged.push(ms),
    onAttached: (said) => attached.push(said),
  });
  const [first] = sockets;
  first.opens();
  first.says({ type: 'attached', id: '1', run: 'r', attempt: 1, from_ms: 0, live_edge_ms: 900_000 });
  first.sends(1);
  first.sends(3);
  first.says({ type: 'lagged', id: '1', resume_from_ms: 600_000 });
  assert.equal(sockets.length, 2, 'a second socket is opened');
  assert.ok(first.closed, 'the lagging one is dropped');
  assert.equal(viewer.cleared, 1);
  assert.deepEqual(lagged, [600_000]);
  const second = sockets[1];
  second.opens();
  assert.deepEqual(second.sent, [{ type: 'attach', id: '2', from: 600_000, sections: [] }]);
  first.sends(4);
  assert.deepEqual(viewer.pushed, [1, 3], 'a frame from the dropped socket is not pushed');
  second.sends(1);
  second.sends(3);
  assert.deepEqual(viewer.pushed, [1, 3, 1, 3], 'the new attachment pushes its own keyframe');
  second.says({ type: 'attached', id: '2', run: 'r', attempt: 1, from_ms: 600_000, live_edge_ms: 1_200_000 });
  assert.deepEqual(attached.map((said) => said.from_ms), [0, 600_000]);
});

test('reports ended once, with its outcome, and nothing after it', () => {
  const viewer = sink();
  const said = [];
  attachFrames('ws://x/frames/r/1', viewer, { onEnded: (outcome) => said.push(`ended ${outcome}`), onClose: () => said.push('close') });
  const [socket] = sockets;
  socket.opens();
  socket.says({ type: 'ended', id: '1', run: 'r', outcome: 'recording-incomplete' });
  assert.ok(socket.closed);
  socket.fire('close');
  socket.sends(4);
  assert.deepEqual(said, ['ended recording-incomplete']);
  assert.deepEqual(viewer.pushed, []);
});

test('reports a frame the viewer refuses, and the server\'s own error, as errors', () => {
  const refusing = { push_frame() { throw new Error('a delta arrived before the keyframe'); }, clear() {} };
  const errors = [];
  attachFrames('ws://x/frames/r/1', refusing, { onError: (why) => errors.push(why) });
  sockets[0].opens();
  sockets[0].sends(4);
  assert.equal(errors.length, 1);
  assert.match(errors[0], /the viewer refused a frame: .*a delta arrived before the keyframe/);
  attachFrames('ws://x/frames/r/1', sink(), { onError: (why) => errors.push(why) });
  sockets[1].opens();
  sockets[1].says({ type: 'error', id: '1', message: 'no such attempt' });
  assert.deepEqual(errors.slice(1), ['no such attempt']);
});

test('close() drops the socket and reports nothing more', () => {
  const said = [];
  const attachment = attachFrames('ws://x/frames/r/1', sink(), { onClose: () => said.push('close'), onEnded: () => said.push('ended') });
  attachment.close();
  assert.ok(sockets[0].closed);
  sockets[0].fire('close');
  sockets[0].says({ type: 'ended', id: '1', run: 'r', outcome: 'finished' });
  assert.deepEqual(said, []);
});

test('an unasked close of the socket is reported as such', () => {
  const said = [];
  attachFrames('ws://x/frames/r/1', sink(), { onClose: () => said.push('close'), onError: (why) => said.push(why) });
  sockets[0].opens();
  sockets[0].fire('close');
  assert.deepEqual(said, ['close']);
});
