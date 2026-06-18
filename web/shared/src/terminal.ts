import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

import { bytesToB64, textToB64 } from "./b64";
import { termKey, type ConduitClient } from "./client";
import type { TerminalKind } from "./protocol";

/** xterm theme matching theme.css — keep the two in sync by hand. */
const XTERM_THEME = {
  background: "#0b0d11",
  foreground: "#d6dae2",
  cursor: "#d6dae2",
  selectionBackground: "#28304a",
  black: "#1c2026",
  red: "#e5534b",
  green: "#57ab5a",
  yellow: "#d4a72c",
  blue: "#539bf5",
  magenta: "#d75fd7",
  cyan: "#2bc6d4",
  white: "#d6dae2",
  brightBlack: "#4d5260",
  brightRed: "#f47067",
  brightGreen: "#6bc46d",
  brightYellow: "#e3b341",
  brightBlue: "#6cb6ff",
  brightMagenta: "#e58fe5",
  brightCyan: "#56d4dd",
  brightWhite: "#ffffff",
};

const RESIZE_DEBOUNCE_MS = 150;

export interface CreateTerminalOpts {
  workspaceId: string;
  kind: TerminalKind;
  tabId?: string | null;
  /** Default: 10000 for Agent, 5000 for Shell. */
  scrollback?: number;
  fontSize?: number;
  /** Called on each live output chunk (not history replay) — used to detect
   * when an agent is ready for an initial prompt. */
  onData?: (bytes: Uint8Array) => void;
}

export interface TerminalHandle {
  /** Open into a container, fit to it, seed from history, and start
   * forwarding I/O. Call once. */
  attach(el: HTMLElement): void;
  fit(): void;
  focus(): void;
  dispose(): void;
  readonly term: Terminal;
  readonly cols: number;
  readonly rows: number;
}

/**
 * Wires an xterm.js instance to one conduit terminal: sink registration for
 * output, SendTerminalInput for keystrokes, debounced ResizeTerminal for
 * size changes.
 *
 * IMPORTANT: the PTY must be spawned at the browser terminal's real size —
 * attach() (which fits) BEFORE sending StartTerminal, then use
 * `handle.cols`/`handle.rows` in the StartTerminal command.
 */
export function createTerminal(client: ConduitClient, opts: CreateTerminalOpts): TerminalHandle {
  const key = termKey(opts.workspaceId, opts.kind, opts.tabId);
  const term = new Terminal({
    allowProposedApi: true,
    scrollback: opts.scrollback ?? (opts.kind === "Agent" ? 10_000 : 5_000),
    fontSize: opts.fontSize ?? 13,
    fontFamily:
      '"JetBrains Mono", ui-monospace, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace',
    theme: XTERM_THEME,
  });
  const fitAddon = new FitAddon();
  term.loadAddon(fitAddon);

  let detachSink: (() => void) | null = null;
  let observer: ResizeObserver | null = null;
  let fitQueued = false;
  let resizeTimer: ReturnType<typeof setTimeout> | null = null;
  let sentCols = 0;
  let sentRows = 0;
  let disposed = false;

  const sendResize = (cols: number, rows: number) => {
    if (cols === sentCols && rows === sentRows) return;
    const ok = client.send({
      ResizeTerminal: {
        id: opts.workspaceId,
        kind: opts.kind,
        tab_id: opts.tabId ?? null,
        cols,
        rows,
      },
    });
    if (ok) {
      sentCols = cols;
      sentRows = rows;
    }
  };

  const sendText = (text: string) =>
    client.send({
      SendTerminalInput: {
        id: opts.workspaceId,
        kind: opts.kind,
        tab_id: opts.tabId ?? null,
        data_b64: textToB64(text),
      },
    });

  // Shift+Enter must insert a newline rather than submit. xterm sends a plain
  // CR for it (indistinguishable from Enter), so intercept it and send ESC+CR
  // — the "meta-return" sequence agent TUIs (Claude Code) read as a newline.
  term.attachCustomKeyEventHandler((e) => {
    if (
      e.type === "keydown" &&
      e.key === "Enter" &&
      e.shiftKey &&
      !e.ctrlKey &&
      !e.altKey &&
      !e.metaKey
    ) {
      sendText("\x1b\r");
      return false;
    }
    return true;
  });

  term.onData((data) => sendText(data));

  term.onBinary((data) => {
    // onBinary delivers latin1: one charCode per byte.
    const bytes = new Uint8Array(data.length);
    for (let i = 0; i < data.length; i++) bytes[i] = data.charCodeAt(i) & 0xff;
    client.send({
      SendTerminalInput: {
        id: opts.workspaceId,
        kind: opts.kind,
        tab_id: opts.tabId ?? null,
        data_b64: bytesToB64(bytes),
      },
    });
  });

  term.onResize(({ cols, rows }) => {
    if (resizeTimer !== null) clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => {
      resizeTimer = null;
      sendResize(cols, rows);
    }, RESIZE_DEBOUNCE_MS);
  });

  return {
    term,
    get cols() {
      return term.cols;
    },
    get rows() {
      return term.rows;
    },
    attach(el: HTMLElement) {
      term.open(el);

      const unicode11 = new Unicode11Addon();
      term.loadAddon(unicode11);
      term.unicode.activeVersion = "11";

      try {
        const webgl = new WebglAddon();
        webgl.onContextLoss(() => webgl.dispose());
        term.loadAddon(webgl);
      } catch {
        // WebGL unavailable — xterm falls back to the DOM renderer.
      }

      fitAddon.fit();

      // History snapshot + sink registration in one synchronous block: no
      // TerminalOutput can interleave, so ordering is exact.
      const history = client.getTerminalHistory(key);
      if (history.length > 0) term.write(history);
      detachSink = client.registerTerminalSink(key, {
        write: (bytes) => {
          term.write(bytes);
          opts.onData?.(bytes);
        },
        reset: () => term.reset(),
      });

      observer = new ResizeObserver(() => {
        if (fitQueued) return;
        fitQueued = true;
        requestAnimationFrame(() => {
          fitQueued = false;
          if (!disposed) fitAddon.fit();
        });
      });
      observer.observe(el);
    },
    fit() {
      fitAddon.fit();
    },
    focus() {
      term.focus();
    },
    dispose() {
      disposed = true;
      if (resizeTimer !== null) clearTimeout(resizeTimer);
      observer?.disconnect();
      detachSink?.();
      term.dispose();
    },
  };
}
