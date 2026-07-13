import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal, type ITheme } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

import { bytesToB64, textToB64 } from "./b64";
import { termKey, type ConduitClient } from "./client";
import type { TerminalKind } from "./protocol";

/** Encode a prompt for injection into a terminal as if pasted-and-submitted.
 * Multi-line text is wrapped in bracketed-paste escapes (matching the TUI's
 * `crates/tui/src/main.rs` paste handling) so an agent TUI treats it as one
 * atomic paste — and a shell won't execute each embedded line. The trailing CR
 * submits. Single-line text needs no wrapping. */
export function promptInput(text: string): string {
  const body = text.includes("\n") ? `\x1b[200~${text}\x1b[201~` : text;
  return `${body}\r`;
}

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

/**
 * Write text to the system clipboard. Prefers the async Clipboard API, but
 * falls back to a hidden <textarea> + execCommand("copy") because that path
 * (a) works in the web UI when served over plain HTTP from a non-loopback
 * host, where `navigator.clipboard` is undefined (insecure context), and
 * (b) is a reliable backstop inside the WebKitGTK desktop webview. Must be
 * called from within a user gesture (a keydown handler) for the fallback.
 */
function copyToClipboard(text: string): void {
  if (navigator.clipboard?.writeText) {
    navigator.clipboard.writeText(text).catch(() => execCommandCopy(text));
    return;
  }
  execCommandCopy(text);
}

function execCommandCopy(text: string): void {
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  ta.style.pointerEvents = "none";
  document.body.appendChild(ta);
  ta.focus();
  ta.select();
  try {
    document.execCommand("copy");
  } catch {
    /* nothing more we can do */
  }
  document.body.removeChild(ta);
}

export interface CreateTerminalOpts {
  workspaceId: string;
  kind: TerminalKind;
  tabId?: string | null;
  /** Default: 10000 for Agent, 5000 for Shell. */
  scrollback?: number;
  fontSize?: number;
  /** Full monospace family stack for xterm. Defaults to the JetBrains Mono
   * stack. Must be monospace — xterm measures a fixed cell width. */
  fontFamily?: string;
  /** Exact registered family name of `fontFamily`'s primary face, used to wait
   * on its woff2 (document.fonts.load) before re-measuring cell width. Null for
   * a system font (always available, no wait needed). */
  fontPrimary?: string | null;
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
  scrollToBottom(): void;
  isAtBottom(): boolean;
  /** Report transitions between the live bottom and an older viewport. */
  onViewportAtBottomChange(listener: (atBottom: boolean) => void): () => void;
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
    // Defaults to "JetBrains Mono Variable" — the family the bundled
    // @fontsource-variable web font registers (matches the app chrome's
    // --font-mono). The non-variable "JetBrains Mono" silently fell back to a
    // system/monospace font, mis-measuring cell width in the WebKitGTK webview.
    // Callers pass a different stack (Settings → Fonts) via opts.fontFamily.
    fontFamily:
      opts.fontFamily ??
      '"JetBrains Mono Variable", "JetBrains Mono", ui-monospace, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace',
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
  let viewportElement: HTMLElement | null = null;
  let viewportCheckQueued = false;

  const isAtBottom = () => {
    const buffer = term.buffer.active;
    if (buffer.viewportY < buffer.baseY) return false;
    // xterm rounds the DOM scroll position to buffer rows. Include its real
    // viewport so a small trackpad/wheel movement still counts as reading
    // history even before that rounding crosses a full terminal row.
    if (viewportElement && viewportElement.clientHeight > 0) {
      return viewportElement.scrollTop + viewportElement.clientHeight >= viewportElement.scrollHeight - 1;
    }
    return true;
  };

  const viewportListeners = new Set<(atBottom: boolean) => void>();
  let lastAtBottom = isAtBottom();
  const notifyViewport = () => {
    const next = isAtBottom();
    if (next === lastAtBottom) return;
    lastAtBottom = next;
    for (const listener of viewportListeners) listener(next);
  };
  // xterm's public onScroll catches buffer/programmatic movement; the native
  // viewport listener below fills in user-driven DOM scrolling. onWriteParsed
  // catches the bottom moving away while the user stays on an older row.
  const viewportScrollDisposable = term.onScroll(notifyViewport);
  const viewportWriteDisposable = term.onWriteParsed(notifyViewport);
  const scheduleViewportCheck = () => {
    if (viewportCheckQueued) return;
    viewportCheckQueued = true;
    queueMicrotask(() => {
      viewportCheckQueued = false;
      if (!disposed) notifyViewport();
    });
  };

  const scrollToBottom = () => {
    term.scrollToBottom();
    notifyViewport();
  };

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
    if (e.type !== "keydown") return true;

    if (e.key === "Enter" && e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey) {
      e.preventDefault();
      sendText("\n");
      return false;
    }

    // Ctrl+C (or Cmd+C) copies the selection when there is one; otherwise it
    // falls through to xterm, which encodes it as 0x03 (SIGINT) as usual. This
    // matches native terminals — xterm consumes the keystroke before the
    // browser can copy, so without this Ctrl+C could never copy.
    const copyCombo =
      e.key.toLowerCase() === "c" &&
      (e.ctrlKey || e.metaKey) &&
      !e.altKey &&
      !e.shiftKey &&
      !(e.ctrlKey && e.metaKey);
    if (copyCombo && term.hasSelection()) {
      e.preventDefault();
      copyToClipboard(term.getSelection());
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
      // xterm suppresses its public `onScroll` event for user-driven DOM
      // scrolling to avoid feeding the viewport's own scroll handler back
      // into itself. Track that native event too, after xterm has updated its
      // buffer position, so the Latest affordance reflects every scroll.
      viewportElement = term.element?.querySelector<HTMLElement>(".xterm-viewport") ?? null;
      viewportElement?.addEventListener("scroll", scheduleViewportCheck, { passive: true });

      const unicode11 = new Unicode11Addon();
      term.loadAddon(unicode11);
      term.unicode.activeVersion = "11";

      // Make URLs clickable. The handler passes the real uri to window.open so
      // that in the desktop webview wry's new-window handler receives it (and
      // routes to the OS browser); in a plain browser it opens a new tab.
      term.loadAddon(
        new WebLinksAddon((_event, uri) => window.open(uri, "_blank", "noopener,noreferrer")),
      );

      // DOM renderer (no WebGL addon): repaints reliably on theme changes, so
      // the terminal background follows the active theme.
      fitAddon.fit();

      // If the terminal font isn't loaded yet, xterm measures the cell width
      // from a fallback font and glyphs then render in too-wide cells (the
      // "C l a u d e" spacing seen in the WebKitGTK webview). Once the font is
      // ready, bust xterm's cached char dimensions (toggle fontFamily), refit,
      // and repaint so the cell width matches the real glyph.
      //
      // We wait on document.fonts.load() for this specific family rather than
      // document.fonts.ready: `ready` resolves when no font loads are *pending*,
      // which on a cold cache happens before our woff2 has actually downloaded,
      // so the re-measure fired too early and the broken spacing stuck.
      const fontSize = opts.fontSize ?? 15;
      // Wait on the chosen font's primary face. opts.fontPrimary defaults to
      // JetBrains Mono Variable; a system font (null) needs no wait.
      const fontPrimary = opts.fontPrimary === undefined ? "JetBrains Mono Variable" : opts.fontPrimary;
      const fontSpec = fontPrimary ? `${fontSize}px "${fontPrimary}"` : "";
      if (fontSpec && typeof document !== "undefined" && document.fonts) {
        const remeasure = () => {
          if (disposed) return;
          const family = term.options.fontFamily ?? "";
          term.options.fontFamily = `${family} `;
          term.options.fontFamily = family;
          fitAddon.fit();
          term.refresh(0, term.rows - 1);
        };
        const needsFontWait = (() => {
          try {
            return !document.fonts.check(fontSpec);
          } catch {
            return true;
          }
        })();
        if (needsFontWait) {
          document.fonts.load(fontSpec).then(remeasure, remeasure);
        }
      }

      // History snapshot + sink registration in one synchronous block: no
      // TerminalOutput can interleave, so ordering is exact.
      const history = client.getTerminalHistory(key);
      // xterm parses writes asynchronously. Wait for the replay itself to be
      // parsed before revealing its newest rows; live writes registered below
      // remain queued after the snapshot and are never forced to follow.
      if (history.length > 0) term.write(history, scrollToBottom);
      else scrollToBottom();
      detachSink = client.registerTerminalSink(key, {
        write: (bytes) => {
          term.write(bytes);
          opts.onData?.(bytes);
        },
        reset: () => {
          term.reset();
          notifyViewport();
        },
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
    scrollToBottom,
    isAtBottom,
    onViewportAtBottomChange(listener) {
      const current = isAtBottom();
      lastAtBottom = current;
      viewportListeners.add(listener);
      listener(current);
      return () => viewportListeners.delete(listener);
    },
    dispose() {
      disposed = true;
      if (resizeTimer !== null) clearTimeout(resizeTimer);
      observer?.disconnect();
      themeObserver.disconnect();
      detachSink?.();
      viewportElement?.removeEventListener("scroll", scheduleViewportCheck);
      viewportScrollDisposable.dispose();
      viewportWriteDisposable.dispose();
      viewportListeners.clear();
      term.dispose();
    },
  };
}
