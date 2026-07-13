import { createEffect, createSignal, For, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { termKey, type WorkspaceSummary } from "@conduit/shared";
import { client } from "../client";
import { agentCmdFor, agentVanillaCmdFor, settings } from "../state/settings";
import { setStore, store } from "../state/store";
import { createShell, removeShell, renameShell, shellTabs } from "../state/tabs";
import { activeTab, isFreshTab, markFreshTab, setActiveTab } from "../state/ui";
import { TerminalView } from "./TerminalView";

function TabButton(props: {
  wsId: string;
  kind: "Agent" | "Shell";
  tabId: string;
  title: string;
  active: boolean;
  onActivate: () => void;
  onClose?: () => void;
  onRename?: (title: string) => void;
  onContextMenu?: (e: MouseEvent) => void;
  /** When set, shows a ▾ caret that opens the same menu as right-click. */
  onMenu?: (e: MouseEvent) => void;
}) {
  const running = () => store.terminals[termKey(props.wsId, props.kind, props.tabId)]?.running ?? false;
  return (
    <div
      class="tab"
      classList={{ active: props.active }}
      onClick={props.onActivate}
      onContextMenu={props.onContextMenu}
    >
      <span class="tab-dot" classList={{ running: running() }} />
      <span
        class="tab-title"
        onDblClick={() => {
          if (!props.onRename) return;
          const next = prompt("Rename tab", props.title);
          if (next) props.onRename(next);
        }}
      >
        {props.title}
      </span>
      <Show when={props.onMenu}>
        <button
          class="tab-caret"
          title="Switch agent type"
          onClick={(e) => {
            e.stopPropagation();
            props.onMenu!(e);
          }}
        >
          ▾
        </button>
      </Show>
      <Show when={props.onClose}>
        <button
          class="tab-close"
          title="Close tab"
          onClick={(e) => {
            e.stopPropagation();
            props.onClose!();
          }}
        >
          ×
        </button>
      </Show>
    </div>
  );
}

export function TerminalRegion(props: { ws: WorkspaceSummary; visible: () => boolean }) {
  const wsId = props.ws.id;
  // Active tab is shared state (state/ui) so the diff pane can switch to a tab
  // after sending a Diff Question; auto-start tracking lives there too.
  const active = () => activeTab(wsId);
  const setActive = (id: string) => setActiveTab(wsId, id);
  const [opened, setOpened] = createSignal<Set<string>>(new Set(["agent", active()]));

  // The agent's liveness when the screen opened decides whether we launch it.
  const agentRunningAtOpen = props.ws.agent_running;

  // "Switch agent type" menu on the agent tab (right-click or ▾ caret).
  // Rendered through a Portal at a fixed screen position: the tabbar's
  // overflow-x:auto clips absolutely-positioned descendants on both axes, so an
  // in-flow dropdown would be invisible. The agent tab is always mounted, so
  // its restart control is available as soon as it registers.
  const [agentMenu, setAgentMenu] = createSignal<{ x: number; y: number } | null>(null);
  const openAgentMenu = (x: number, y: number) =>
    setAgentMenu({ x: Math.max(0, Math.min(x, window.innerWidth - 200)), y });
  let agentControls: { restart: (cmd?: string[]) => void } | null = null;
  // `ws.agent` is null when the workspace defers to the client default.
  const currentAgent = () => props.ws.agent ?? settings.defaultAgent;

  const switchAgent = (name: string) => {
    setAgentMenu(null);
    // Optimistically reflect the new agent (so cmd()/fallbackCmd() resolve to
    // it), persist it server-side, then relaunch the agent terminal.
    setStore("workspaces", (w) => w.id === wsId, "agent", name);
    client.send({ SetWorkspaceAgent: { id: wsId, agent: name } });
    setActive("agent");
    agentControls?.restart(agentCmdFor(name));
  };

  createEffect(() => {
    const a = active();
    if (!opened().has(a)) setOpened(new Set(opened()).add(a));
  });

  const addShell = () => {
    const tab = createShell(wsId);
    markFreshTab(wsId, tab.id);
    setActive(tab.id);
  };

  const closeShell = (id: string) => {
    client.send({ StopTerminal: { id: wsId, kind: "Shell", tab_id: id } });
    removeShell(wsId, id);
    setOpened((s) => {
      const n = new Set(s);
      n.delete(id);
      return n;
    });
    if (active() === id) setActive("agent");
  };

  return (
    <section class="term-region">
      <div class="tabbar">
        <TabButton
          wsId={wsId}
          kind="Agent"
          tabId="agent"
          title="agent"
          active={active() === "agent"}
          onActivate={() => setActive("agent")}
          onContextMenu={(e) => {
            e.preventDefault();
            openAgentMenu(e.clientX, e.clientY);
          }}
          onMenu={(e) => {
            const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
            openAgentMenu(r.left, r.bottom + 4);
          }}
        />
        <Show when={agentMenu()}>
          <Portal>
            <div
              class="menu-catcher"
              onClick={() => setAgentMenu(null)}
              onContextMenu={(e) => {
                e.preventDefault();
                setAgentMenu(null);
              }}
            />
            <div
              class="menu"
              style={{
                position: "fixed",
                left: `${agentMenu()!.x}px`,
                top: `${agentMenu()!.y}px`,
                right: "auto",
              }}
            >
              <For each={settings.agents}>
                {(a) => (
                  <button
                    class="menu-item"
                    classList={{ active: currentAgent() === a.name }}
                    onClick={() => switchAgent(a.name)}
                  >
                    {a.name}
                  </button>
                )}
              </For>
              <div class="menu-sep" />
              <button
                class="menu-item"
                onClick={() => {
                  setAgentMenu(null);
                  const cmd = prompt("Custom agent command", currentAgent());
                  if (cmd && cmd.trim()) switchAgent(cmd.trim());
                }}
              >
                Custom…
              </button>
            </div>
          </Portal>
        </Show>
        <For each={shellTabs(wsId)}>
          {(shell) => (
            <TabButton
              wsId={wsId}
              kind="Shell"
              tabId={shell.id}
              title={shell.title}
              active={active() === shell.id}
              onActivate={() => setActive(shell.id)}
              onClose={() => closeShell(shell.id)}
              onRename={(t) => renameShell(wsId, shell.id, t)}
            />
          )}
        </For>
        <button class="tab-new" title="New shell" onClick={addShell}>
          +
        </button>
      </div>

      <div class="term-stack">
        <Show when={opened().has("agent")}>
          <div class="term-slot" style={{ display: active() === "agent" ? "flex" : "none" }}>
            <TerminalView
              wsId={wsId}
              kind="Agent"
              tabId="agent"
              active={() => props.visible() && active() === "agent"}
              startOnMount={!agentRunningAtOpen}
              cmd={() => agentCmdFor(props.ws.agent)}
              fallbackCmd={() => agentVanillaCmdFor(props.ws.agent)}
              externalRunning={() => props.ws.agent_running}
              registerControls={(c) => (agentControls = c)}
              initialPrompt={() => store.pendingPrompt[wsId]}
              onPromptSent={() => setStore("pendingPrompt", wsId, undefined!)}
            />
          </div>
        </Show>
        <For each={shellTabs(wsId)}>
          {(shell) => (
            <Show when={opened().has(shell.id)}>
              <div class="term-slot" style={{ display: active() === shell.id ? "flex" : "none" }}>
                <TerminalView
                  wsId={wsId}
                  kind="Shell"
                  tabId={shell.id}
                  active={() => props.visible() && active() === shell.id}
                  startOnMount={isFreshTab(wsId, shell.id)}
                  cmd={() => shell.cmd ?? []}
                  initialPrompt={() => store.pendingTabPrompt[`${wsId}/${shell.id}`]}
                  onPromptSent={() => setStore("pendingTabPrompt", `${wsId}/${shell.id}`, undefined!)}
                />
              </div>
            </Show>
          )}
        </For>
      </div>
    </section>
  );
}
