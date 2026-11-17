// Within crucible-viewer, `frames` is the browser's half of the attach protocol over one websocket to a run's frames:
// the streaming server's `/frames/<run>/<attempt>` (live frames, 26 October 2026), or crucible's `/ws`, which speak
// the same words. `attachFrames` opens the socket, sends `attach` (with the attempt's watch ticket as `token`, which
// the streaming server wants and crucible ignores), pushes every binary message into the viewer one frame at a time,
// which is what `Viewer.push_frame` takes, and tells the page what the server said in words:
// `attached`, `lagged`, `ended`, an error, and the socket closing. A `lagged` is answered here, not by the page: the
// socket is dropped, the viewer's window cleared, and a new attachment opened from the instant the server named.
//
// One frame is held back on purpose. The viewer takes ONE keyframe per attachment and the deltas after it carry the
// state forward exactly (an invariant of the format crucible's server also rests on), so a keyframe after the first,
// which a live stream has every segment, is not pushed. The page therefore sees one window that grows, never a window
// that starts again at each segment.

import type { Reply } from "./protocol.js";

/** What the frames are pushed into: the wasm `Viewer`, or a stand-in in a test. */
export interface FrameSink {
  push_frame(bytes: Uint8Array): void;
  clear(): void;
}

/** A frame's kind, read off its header's first byte, as the container names them. */
export type FrameTag = "scenario" | "declare" | "keyframe" | "delta" | "index" | "end";

const TAGS: Readonly<Record<number, FrameTag>> = { 1: "scenario", 2: "declare", 3: "keyframe", 4: "delta", 5: "index", 6: "end" };

export interface AttachOptions {
  /** The attempt's watch ticket (`POST /api/runs/{run}/watch`), which the streaming server admits the attach by. */
  token?: string;
  /** A simulated instant in milliseconds to start from, or `"live"`; absent is the start of what the server holds. */
  from?: number | "live";
  /** Which sections of each frame to send; empty means the server's default set. */
  sections?: string[];
  /** The server's answer: the instant the frames start at, and how far they reach in simtime as of the attach. */
  onAttached?(attached: { from_ms: number; live_edge_ms: number }): void;
  /** A frame pushed into the viewer, by its kind and size; a held-back keyframe is not reported. */
  onFrame?(tag: FrameTag, bytes: number): void;
  /** The server dropped this attachment as too slow and a new one is being opened from `resume_from_ms`. */
  onLagged?(resume_from_ms: number): void;
  /** Every frame is in: the run ended, or the replay reached its end; `outcome` is the server's word for how the run
   * ended, or for a copy that stops short of it. */
  onEnded?(outcome: string): void;
  /** The server refused in words, the socket failed, or the viewer refused a frame; the attachment is over. */
  onError?(message: string): void;
  /** The socket closed other than by `close()` or after `ended`. */
  onClose?(): void;
}

export interface Attachment {
  /** Drop the socket; nothing is reported after. */
  close(): void;
}

/** Open `url`, attach, and push each frame into `viewer` until `ended`, an error, or `close()`. */
export function attachFrames(url: string, viewer: FrameSink, options: AttachOptions = {}): Attachment {
  let socket: WebSocket | null = null;
  let over = false;
  let attempts = 0;

  const finish = (said: () => void) => {
    if (over) return;
    over = true;
    socket?.close();
    socket = null;
    said();
  };

  const takeText = (text: string) => {
    let reply: Reply;
    try {
      reply = JSON.parse(text) as Reply;
    } catch {
      finish(() => options.onError?.(`the server at ${url} sent something other than a reply: ${text.slice(0, 80)}`));
      return;
    }
    switch (reply.type) {
      case "attached":
        options.onAttached?.({ from_ms: reply.from_ms, live_edge_ms: reply.live_edge_ms });
        return;
      case "lagged": {
        const resume = reply.resume_from_ms;
        const dropped = socket;
        socket = null;
        dropped?.close();
        viewer.clear();
        options.onLagged?.(resume);
        open(resume);
        return;
      }
      case "ended":
        finish(() => options.onEnded?.(reply.outcome));
        return;
      case "error":
        finish(() => options.onError?.(reply.message));
        return;
      default:
        return;
    }
  };

  const open = (from: number | "live" | undefined) => {
    if (over) return;
    attempts += 1;
    const id = String(attempts);
    let keyframed = false;
    const ws = new WebSocket(url);
    ws.binaryType = "arraybuffer";
    socket = ws;
    ws.addEventListener("open", () => {
      if (socket !== ws) return;
      const attach = { type: "attach", id, from: from ?? 0, sections: options.sections ?? [] };
      ws.send(JSON.stringify(options.token === undefined ? attach : { ...attach, token: options.token }));
    });
    ws.addEventListener("message", (event: MessageEvent<string | ArrayBuffer>) => {
      if (socket !== ws || over) return;
      if (typeof event.data === "string") {
        takeText(event.data);
        return;
      }
      const bytes = new Uint8Array(event.data);
      const tag = TAGS[bytes[0] ?? 0];
      if (tag === "keyframe") {
        if (keyframed) return;
        keyframed = true;
      }
      try {
        viewer.push_frame(bytes);
      } catch (failure) {
        finish(() => options.onError?.(`the viewer refused a frame: ${String(failure)}`));
        return;
      }
      options.onFrame?.(tag ?? "delta", bytes.length);
    });
    ws.addEventListener("error", () => {
      if (socket === ws) finish(() => options.onError?.(`the socket to ${url} failed`));
    });
    ws.addEventListener("close", () => {
      if (socket === ws) finish(() => options.onClose?.());
    });
  };

  open(options.from);
  return { close: () => finish(() => undefined) };
}
