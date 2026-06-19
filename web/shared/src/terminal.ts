import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { Terminal, type ITheme } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

import { bytesToB64, textToB64 } from "./b64";
import { termKey, type ConduitClient } from "./client";
import type { TerminalKind } from "./protocol";

// The terminal is monochrome per theme: background, text, cursor, selection
// AND the 16 ANSI colors are all derived from the active palette, so the whole
// terminal tints with it (amber CRT, mono greyscale, paper black-on-white).
// Colors come from the LIVE theme: we read the resolved background/foreground
// off <body> — a standard property always resolves, unlike a custom-property
// read (getComputedStyle().getPropertyValue("--ink")), which returned empty in
// the observer context before — and interpolate between them for the ANSI ramp.
type RGB = [number, number, number];

function parseRgb(c: string): RGB {
  const m = c.match(/rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)/i);
  if (m) return [Number(m[1]), Number(m[2]), Number(m[3])];
  const h = c.replace(/[^0-9a-f]/gi, "");
  const hex = h.length === 3 ? h.replace(/(.)/g, "$1$1") : h.padEnd(6, "0").slice(0, 6);
  const n = parseInt(hex, 16) || 0;
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** Interpolate between two colors; t=0 is `a`, t=1 is `b`. */
function mix(a: RGB, b: RGB, t: number): string {
  const ch = (i: 0 | 1 | 2) => Math.round(a[i] + (b[i] - a[i]) * t);
  return `rgb(${ch(0)}, ${ch(1)}, ${ch(2)})`;
}

// Ramp positions for the 16 ANSI slots (0 = background … 1 = foreground),
// spread so the dimmest is still legible and each "bright*" sits above its base.
function xtermTheme(): ITheme {
  const cs = getComputedStyle(document.body);
  const bg = cs.backgroundColor || "#0b0704";
  const fg = cs.color || "#ffd591";
  const p = parseRgb(bg);
  const k = parseRgb(fg);
  return {
    background: bg,
    foreground: fg,
    cursor: fg,
    cursorAccent: bg,
    selectionBackground: fg,
    selectionForeground: bg,
    black: mix(p, k, 0.16),
    red: mix(p, k, 0.42),
    green: mix(p, k, 0.5),
    yellow: mix(p, k, 0.62),
    blue: mix(p, k, 0.4),
    magenta: mix(p, k, 0.52),
    cyan: mix(p, k, 0.58),
    white: mix(p, k, 0.8),
    brightBlack: mix(p, k, 0.3),
    brightRed: mix(p, k, 0.6),
    brightGreen: mix(p, k, 0.68),
    brightYellow: mix(p, k, 0.82),
    brightBlue: mix(p, k, 0.56),
    brightMagenta: mix(p, k, 0.7),
    brightCyan: mix(p, k, 0.76),
    brightWhite: mix(p, k, 1),
  };
}

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
    fontSize: opts.fontSize ?? 15,
    fontFamily:
      '"JetBrains Mono", ui-monospace, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace',
    theme: xtermTheme(),
  });
  const fitAddon = new FitAddon();
  term.loadAddon(fitAddon);

  // Re-skin the terminal when the app's theme changes (the DOM renderer
  // repaints on theme assignment; refresh for good measure).
  const themeObserver = new MutationObserver(() => {
    term.options.theme = xtermTheme();
    term.refresh(0, term.rows - 1);
  });
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-theme"],
  });

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
  // CR for it (indistinguishable from Enter), so intercept it and send a bare
  // LF — matching the native TUI's key encoder (key_to_terminal_bytes in
  // crates/tui/src/main.rs), which maps Shift+Enter to "\n". Agent TUIs read
  // the LF as a newline; CR is what submits.
  //
  // preventDefault() is load-bearing: returning false only makes xterm's
  // _keyDown bail early, and its DOM listener discards that return value, so
  // xterm never calls preventDefault itself (see @xterm Terminal.ts — the
  // 'keydown' listener is `(ev) => this._keyDown(ev)`). Without it the browser
  // still fires the follow-up `keypress`, which xterm's _keyPress converts to a
  // CR (charCode 13) and emits via onData — submitting right after our LF. That
  // trailing CR is why earlier attempts (both "\n" and "\x1b\r") appeared to do
  // nothing but submit. Cancelling the keydown's default suppresses the
  // keypress, so only the LF is sent.
  term.attachCustomKeyEventHandler((e) => {
    if (
      e.type === "keydown" &&
      e.key === "Enter" &&
      e.shiftKey &&
      !e.ctrlKey &&
      !e.altKey &&
      !e.metaKey
    ) {
      e.preventDefault();
      sendText("\n");
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

      // DOM renderer (no WebGL addon): repaints reliably on theme changes, so
      // the terminal background follows the active theme.
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
      themeObserver.disconnect();
      detachSink?.();
      term.dispose();
    },
  };
}
