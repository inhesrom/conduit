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
  /** Expose imperative controls to the parent (e.g. the agent tab's
   * right-click "switch agent type", which restarts with a new command). */
  registerControls?: (controls: { restart: (cmd?: string[]) => void }) => void;
}) {
  let host!: HTMLDivElement;
  let handle: TerminalHandle | null = null;
  let startedAt = 0;
  // Set for a caller-initiated stop (e.g. switching agent type) so the
  // fast-exit fallback doesn't treat the intentional kill as a crash.
  let intentionalStop = false;
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
  const [atBottom, setAtBottom] = createSignal(true);
  let stopViewportTracking: (() => void) | null = null;

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

  // Stop the current process and relaunch (default: props.cmd(), which reflects
  // the workspace's current agent). Marks the stop intentional so the crash
  // fallback doesn't fight the relaunch.
  const restart = (cmd?: string[]) => {
    if (!handle) return;
    intentionalStop = true;
    client.send({ StopTerminal: { id: props.wsId, kind: props.kind, tab_id: props.tabId } });
    start(cmd ?? props.cmd());
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
    stopViewportTracking = handle.onViewportAtBottomChange(setAtBottom);
    props.registerControls?.({ restart });
    if (props.startOnMount) start(props.cmd());
  });
  onCleanup(() => {
    stopViewportTracking?.();
    handle?.dispose();
  });

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

  // xterm can't measure while hidden. A transition into view (workspace
  // entry, tab selection, or a diff closing) is also the one time we
  // deliberately return to live output. Later writes never force that jump.
  createEffect(
    on(
      () => props.active(),
      (active, wasActive) => {
        if (!active || wasActive) return;
        queueMicrotask(() => {
          handle?.fit();
          handle?.scrollToBottom();
        });
      },
    ),
  );

  // xterm suppresses some user-originated scroll notifications while it
  // coordinates its DOM viewport. Event tracking updates this immediately in
  // the usual case; this active-tab reconciliation is the correctness backstop
  // so the Latest control can never stay stale after a scroll.
  createEffect(
    on(
      () => props.active(),
      (active) => {
        if (!active) return;
        let frame = 0;
        const reconcileViewport = () => {
          setAtBottom(handle?.isAtBottom() ?? true);
          frame = requestAnimationFrame(reconcileViewport);
        };
        reconcileViewport();
        onCleanup(() => cancelAnimationFrame(frame));
      },
    ),
  );

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
          // A caller-initiated stop (agent switch) already relaunched; don't let
          // the crash fallback double-start.
          if (intentionalStop) {
            intentionalStop = false;
            return;
          }
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
      <Show when={!atBottom()}>
        <button
          type="button"
          class="term-latest"
          aria-label="Scroll to latest terminal output"
          title="Scroll to latest output"
          onClick={() => {
            handle?.scrollToBottom();
            handle?.focus();
          }}
        >
          ↓ Latest
        </button>
      </Show>
    </div>
  );
}
