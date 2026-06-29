import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show } from "solid-js";
import {
  createTerminal,
  promptInput,
  termKey,
  textToB64,
  type TerminalHandle,
  type TerminalKind,
} from "@conduit/shared";
import { client } from "../client";
import { settings } from "../state/settings";
import { fontById, fontCss } from "../state/fonts";
import { store } from "../state/store";

type Phase = "idle" | "running" | "exited";

/** One terminal tab: an xterm wired to a conduit PTY. Output and input go
 * through the shared client; this component owns the xterm lifecycle, the
 * start/stop affordances, and the agent fast-exit fallback. */
export function TerminalView(props: {
  wsId: string;
  kind: TerminalKind;
  tabId: string;
  active: () => boolean;
  startOnMount: boolean;
  cmd: () => string[];
  /** Agent only: vanilla relaunch if it dies seconds after starting. */
  fallbackCmd?: () => string[];
  /** Authoritative liveness from the workspace summary (agent_running). A
   * silent running PTY produces no output, so it isn't in the history replay;
   * the summary still knows it's alive. */
  externalRunning?: () => boolean;
  /** Agent only: an initial prompt to type once the agent produces output. */
  initialPrompt?: () => string | undefined;
  onPromptSent?: () => void;
}) {
  let host!: HTMLDivElement;
  let handle: TerminalHandle | null = null;
  let startedAt = 0;
  const key = termKey(props.wsId, props.kind, props.tabId);
  const term = () => store.terminals[key];
  const resurrect = () =>
    props.kind === "Shell" ? store.resurrection[`${props.wsId}/${props.tabId}`] : undefined;

  const clearResurrect = () =>
    client.send({ ClearShellResurrection: { id: props.wsId, tab_id: props.tabId } });
  const runResurrect = () => {
    const cmd = resurrect();
    if (cmd) {
      client.send({
        SendTerminalInput: {
          id: props.wsId,
          kind: props.kind,
          tab_id: props.tabId,
          data_b64: textToB64(`${cmd.argv.join(" ")}\r`),
        },
      });
    }
    clearResurrect();
  };
  // Optimistic until the server confirms — covers the gap between sending
  // StartTerminal and the TerminalStarted event.
  const [starting, setStarting] = createSignal(props.startOnMount);

  // Derived so it can't get stuck: the event stream is the source of truth,
  // with `starting` only filling the brief pre-confirmation gap.
  const phase = createMemo<Phase>(() => {
    const t = term();
    if (t && !t.running && t.exitCode != null) return "exited";
    if (t?.running || props.externalRunning?.()) return "running";
    return starting() ? "running" : "idle";
  });

  let promptSent = false;
  const maybeSendPrompt = () => {
    if (promptSent) return;
    const p = props.initialPrompt?.();
    if (!p) return;
    promptSent = true;
    client.send({
      SendTerminalInput: {
        id: props.wsId,
        kind: props.kind,
        tab_id: props.tabId,
        data_b64: textToB64(promptInput(p)),
      },
    });
    props.onPromptSent?.();
  };

  const start = (cmd: string[]) => {
    if (!handle) return;
    handle.fit();
    startedAt = performance.now();
    setStarting(true);
    client.send({
      StartTerminal: {
        id: props.wsId,
        kind: props.kind,
        tab_id: props.tabId,
        cmd,
        cols: handle.cols,
        rows: handle.rows,
      },
    });
    handle.focus();
    // Deliver the initial prompt after the agent's first output (below);
    // this is a fallback in case it stays silent.
    if (props.initialPrompt?.()) setTimeout(maybeSendPrompt, 2500);
  };

  onMount(() => {
    handle = createTerminal(client, {
      workspaceId: props.wsId,
      kind: props.kind,
      tabId: props.tabId,
      fontSize: settings.termFontSize,
      fontFamily: fontCss(settings.terminalFont),
      fontPrimary: fontById(settings.terminalFont)?.primary ?? null,
      onData: () => maybeSendPrompt(),
    });
    handle.attach(host);
    if (props.startOnMount) start(props.cmd());
  });
  onCleanup(() => handle?.dispose());

  // Live-apply terminal font-size changes from Settings. fit() re-derives
  // cols/rows for the new size and reports them to the PTY via ResizeTerminal.
  createEffect(() => {
    const size = settings.termFontSize;
    if (!handle) return;
    handle.term.options.fontSize = size;
    handle.fit();
  });

  // Live-apply terminal font-family changes from Settings. Toggling fontFamily
  // busts xterm's cached char dimensions so the cell width re-measures against
  // the new glyph (same trick as terminal.ts's cold-cache remeasure), then fit.
  createEffect(() => {
    const family = fontCss(settings.terminalFont);
    const primary = fontById(settings.terminalFont)?.primary;
    if (!handle) return;
    const applyFamily = () => {
      handle!.term.options.fontFamily = `${family} `;
      handle!.term.options.fontFamily = family;
      handle!.fit();
      handle!.term.refresh(0, handle!.term.rows - 1);
    };
    // Wait for the woff2 before measuring, else the cell width sticks wrong.
    if (primary && document.fonts && !document.fonts.check(`16px "${primary}"`)) {
      document.fonts.load(`16px "${primary}"`).then(applyFamily, applyFamily);
    } else {
      applyFamily();
    }
  });

  // Refit when this tab becomes visible again (xterm can't measure while hidden).
  createEffect(() => {
    if (props.active()) queueMicrotask(() => handle?.fit());
  });

  // Track liveness from the event stream; restart a crashed agent once.
  createEffect(
    on(
      () => term()?.running,
      (running, prev) => {
        if (running) {
          setStarting(false);
          startedAt = performance.now();
        } else if (prev) {
          setStarting(false);
          const code = term()?.exitCode;
          if (props.fallbackCmd && code != null && code !== 0 && performance.now() - startedAt < 3000) {
            start(props.fallbackCmd());
          }
        }
      },
      { defer: true },
    ),
  );

  return (
    <div class="termview">
      <Show when={phase() !== "running"}>
        <div class="term-overlay">
          <span class="term-overlay-msg">
            {phase() === "exited"
              ? `Exited${term()?.exitCode ? ` · code ${term()!.exitCode}` : ""}`
              : "Stopped"}
          </span>
          <button class="btn" onClick={() => start(props.cmd())}>
            {phase() === "exited" ? "Restart" : "Start"}
          </button>
        </div>
      </Show>
      <Show when={resurrect()}>
        <div class="term-resurrect">
          <span class="mono term-resurrect-cmd">Re-run: {resurrect()!.argv.join(" ")}</span>
          <span class="term-resurrect-actions">
            <button class="btn xs primary" onClick={runResurrect}>
              Run
            </button>
            <button class="btn xs" onClick={clearResurrect}>
              Dismiss
            </button>
          </span>
        </div>
      </Show>
      <div class="term-host" ref={host} />
    </div>
  );
}
