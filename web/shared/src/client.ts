import { b64ToBytes } from "./b64";
import { decodeEvent, type ConduitEvent } from "./events";
import type { Command, TerminalKind } from "./protocol";

export type ConnStatus = "connecting" | "open" | "closed";

/** `${workspaceId}/${kind}/${tabId ?? ""}` — one key per terminal tab. */
export type TermKey = string;

export function termKey(id: string, kind: TerminalKind, tabId?: string | null): TermKey {
  return `${id}/${kind}/${tabId ?? ""}`;
}

/** Everything except TerminalOutput — terminal bytes never enter app state. */
export type AppEvent = Exclude<ConduitEvent, { type: "TerminalOutput" }>;

export interface TerminalSink {
  write(bytes: Uint8Array): void;
  /** A fresh replay is about to begin (reconnect or terminal restart) —
   * clear the screen so it doesn't double-print. */
  reset(): void;
}

interface TermBuffer {
  chunks: Uint8Array[];
  size: number;
  /** Connection generation that last wrote; a mismatch means the server is
   * replaying this terminal from scratch. */
  generation: number;
}

const TERM_BUFFER_CAP = 1024 * 1024; // 1 MB per terminal tab, mirrors server-side cap

export interface ConduitClientOptions {
  url: string;
  minRetryMs?: number;
  maxRetryMs?: number;
}

/**
 * WebSocket client for the conduit protocol.
 *
 * Two data paths:
 * - App state: every event except TerminalOutput goes to `onEvent`
 *   subscribers (feed these into your framework's store).
 * - Terminal bytes: TerminalOutput is base64-decoded into a per-terminal
 *   ring buffer and forwarded to a registered sink (xterm.js), bypassing
 *   reactive state entirely. The ring buffer records even with no sink
 *   mounted, so a terminal view created later seeds itself instantly via
 *   `getTerminalHistory`.
 *
 * Reconnects with jittered exponential backoff; the server replays a full
 * snapshot on every connect, and snapshot events (WorkspaceList, …) are full
 * replacements, so app state self-heals without special reconnect handling.
 */
export class ConduitClient {
  #url: string;
  #minRetryMs: number;
  #maxRetryMs: number;

  #socket: WebSocket | null = null;
  #status: ConnStatus = "closed";
  #generation = 0;
  #retryMs: number;
  #retryTimer: ReturnType<typeof setTimeout> | null = null;
  #closed = true;

  #eventHandlers = new Set<(e: AppEvent) => void>();
  #statusHandlers = new Set<(s: ConnStatus, generation: number) => void>();
  #termBuffers = new Map<TermKey, TermBuffer>();
  #termSinks = new Map<TermKey, TerminalSink>();

  constructor(opts: ConduitClientOptions) {
    this.#url = opts.url;
    this.#minRetryMs = opts.minRetryMs ?? 500;
    this.#maxRetryMs = opts.maxRetryMs ?? 15_000;
    this.#retryMs = this.#minRetryMs;
  }

  get status(): ConnStatus {
    return this.#status;
  }

  /** Increments on every successful (re)connect. */
  get generation(): number {
    return this.#generation;
  }

  connect(): void {
    this.#closed = false;
    this.#open();
  }

  close(): void {
    this.#closed = true;
    if (this.#retryTimer !== null) clearTimeout(this.#retryTimer);
    this.#retryTimer = null;
    this.#socket?.close();
    this.#socket = null;
    this.#setStatus("closed");
  }

  /** Returns false (and drops the command) when the socket isn't open —
   * disable actions in the UI instead of queueing stale mutations. */
  send(cmd: Command): boolean {
    if (this.#socket === null || this.#socket.readyState !== WebSocket.OPEN) return false;
    this.#socket.send(JSON.stringify(cmd));
    return true;
  }

  onEvent(handler: (e: AppEvent) => void): () => void {
    this.#eventHandlers.add(handler);
    return () => this.#eventHandlers.delete(handler);
  }

  onStatus(handler: (s: ConnStatus, generation: number) => void): () => void {
    this.#statusHandlers.add(handler);
    return () => this.#statusHandlers.delete(handler);
  }

  /** One sink per terminal. Registration does not replay history — call
   * `getTerminalHistory` first (same tick) and seed the terminal yourself. */
  registerTerminalSink(key: TermKey, sink: TerminalSink): () => void {
    this.#termSinks.set(key, sink);
    return () => {
      if (this.#termSinks.get(key) === sink) this.#termSinks.delete(key);
    };
  }

  /** Concatenated ring-buffer contents for a terminal (replay + live bytes). */
  getTerminalHistory(key: TermKey): Uint8Array {
    const buf = this.#termBuffers.get(key);
    if (!buf || buf.size === 0) return new Uint8Array(0);
    const out = new Uint8Array(buf.size);
    let offset = 0;
    for (const chunk of buf.chunks) {
      out.set(chunk, offset);
      offset += chunk.length;
    }
    return out;
  }

  #open(): void {
    this.#setStatus("connecting");
    const socket = new WebSocket(this.#url);
    this.#socket = socket;

    socket.onopen = () => {
      if (socket !== this.#socket) return;
      this.#generation += 1;
      this.#retryMs = this.#minRetryMs;
      this.#setStatus("open");
    };

    socket.onmessage = (ev) => {
      if (socket !== this.#socket || typeof ev.data !== "string") return;
      const evt = decodeEvent(ev.data);
      if (evt === null) return;
      this.#dispatch(evt);
    };

    socket.onclose = () => {
      if (socket !== this.#socket) return;
      this.#socket = null;
      this.#setStatus("closed");
      this.#scheduleRetry();
    };

    socket.onerror = () => {
      // onclose follows and owns the retry.
    };
  }

  #scheduleRetry(): void {
    if (this.#closed || this.#retryTimer !== null) return;
    const jitter = 0.5 + Math.random();
    const delay = Math.min(this.#retryMs * jitter, this.#maxRetryMs);
    this.#retryMs = Math.min(this.#retryMs * 2, this.#maxRetryMs);
    this.#retryTimer = setTimeout(() => {
      this.#retryTimer = null;
      if (!this.#closed) this.#open();
    }, delay);
  }

  #dispatch(evt: ConduitEvent): void {
    if (evt.type === "TerminalOutput") {
      this.#handleTerminalOutput(termKey(evt.id, evt.kind, evt.tab_id), evt.data_b64);
      return;
    }
    if (evt.type === "TerminalStarted") {
      // A (re)started terminal replays from scratch: drop stale bytes.
      this.#resetBuffer(termKey(evt.id, evt.kind, evt.tab_id));
    }
    for (const handler of this.#eventHandlers) handler(evt);
  }

  #handleTerminalOutput(key: TermKey, dataB64: string): void {
    const bytes = b64ToBytes(dataB64);
    let buf = this.#termBuffers.get(key);
    if (!buf) {
      buf = { chunks: [], size: 0, generation: this.#generation };
      this.#termBuffers.set(key, buf);
    }
    if (buf.generation !== this.#generation) {
      // First bytes of a new connection: the server is replaying this
      // terminal's full buffer — start clean.
      buf.chunks = [];
      buf.size = 0;
      buf.generation = this.#generation;
      this.#termSinks.get(key)?.reset();
    }
    buf.chunks.push(bytes);
    buf.size += bytes.length;
    while (buf.size > TERM_BUFFER_CAP && buf.chunks.length > 1) {
      buf.size -= buf.chunks.shift()!.length;
    }
    this.#termSinks.get(key)?.write(bytes);
  }

  #resetBuffer(key: TermKey): void {
    const buf = this.#termBuffers.get(key);
    if (buf) {
      buf.chunks = [];
      buf.size = 0;
      buf.generation = this.#generation;
    }
    this.#termSinks.get(key)?.reset();
  }

  #setStatus(status: ConnStatus): void {
    if (this.#status === status) return;
    this.#status = status;
    for (const handler of this.#statusHandlers) handler(status, this.#generation);
  }
}
