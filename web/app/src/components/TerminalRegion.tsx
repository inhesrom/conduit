import { createEffect, createSignal, For, Show } from "solid-js";
import { termKey, type WorkspaceSummary } from "@conduit/shared";
import { client } from "../client";
import { agentCmdFor, agentVanillaCmdFor } from "../state/settings";
import { store } from "../state/store";
import { createShell, removeShell, renameShell, shellTabs } from "../state/tabs";
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
}) {
  const running = () => store.terminals[termKey(props.wsId, props.kind, props.tabId)]?.running ?? false;
  return (
    <div class="tab" classList={{ active: props.active }} onClick={props.onActivate}>
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

export function TerminalRegion(props: { ws: WorkspaceSummary }) {
  const wsId = props.ws.id;
  const [active, setActive] = createSignal<string>("agent");
  const [opened, setOpened] = createSignal<Set<string>>(new Set(["agent"]));
  // Shells created in this session auto-start; restored ones wait for Start.
  const fresh = new Set<string>();

  // The agent's liveness when the screen opened decides whether we launch it.
  const agentRunningAtOpen = props.ws.agent_running;

  createEffect(() => {
    const a = active();
    if (!opened().has(a)) setOpened(new Set(opened()).add(a));
  });

  const addShell = () => {
    const tab = createShell(wsId);
    fresh.add(tab.id);
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
        />
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
              active={() => active() === "agent"}
              startOnMount={!agentRunningAtOpen}
              cmd={() => agentCmdFor(props.ws.agent)}
              fallbackCmd={() => agentVanillaCmdFor(props.ws.agent)}
              externalRunning={() => props.ws.agent_running}
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
                  active={() => active() === shell.id}
                  startOnMount={fresh.has(shell.id)}
                  cmd={() => []}
                />
              </div>
            </Show>
          )}
        </For>
      </div>
    </section>
  );
}
