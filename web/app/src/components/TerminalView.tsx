import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show } from "solid-js";
import { createTerminal, termKey, type TerminalHandle, type TerminalKind } from "@conduit/shared";
import { client } from "../client";
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
}) {
  let host!: HTMLDivElement;
  let handle: TerminalHandle | null = null;
  let startedAt = 0;
  const key = termKey(props.wsId, props.kind, props.tabId);
  const term = () => store.terminals[key];
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
  };

  onMount(() => {
    handle = createTerminal(client, {
      workspaceId: props.wsId,
      kind: props.kind,
      tabId: props.tabId,
    });
    handle.attach(host);
    if (props.startOnMount) start(props.cmd());
  });
  onCleanup(() => handle?.dispose());

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
      <div class="term-host" ref={host} />
    </div>
  );
}
