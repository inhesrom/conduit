import { Show } from "solid-js";
import { navigate } from "../router";
import { store } from "../state/store";

/** Placeholder until the next milestone wires terminals + git panes. */
export function WorkspaceScreen(props: { id: string }) {
  const ws = () => store.workspaces.find((w) => w.id === props.id);
  return (
    <div class="ws-screen">
      <Show when={ws()} fallback={<div class="empty">Workspace not found.</div>}>
        <header class="ws-screen-head">
          <button class="back" onClick={() => navigate({ name: "board" })}>
            ← Board
          </button>
          <span class="ws-screen-name">{ws()!.name}</span>
          <Show when={ws()!.branch}>
            <span class="ws-screen-branch mono">{ws()!.branch}</span>
          </Show>
        </header>
        <div class="empty">Terminal and git panes arrive in the next step.</div>
      </Show>
    </div>
  );
}
