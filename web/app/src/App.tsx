import { Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { AppModals } from "./components/AppModals";
import { ConnectionBanner } from "./components/ConnectionBanner";
import { Dialogs } from "./components/Dialogs";
import { Sidebar } from "./components/Sidebar";
import { Toasts } from "./components/Toasts";
import { currentWorkspaceId, route } from "./router";
import { BoardScreen } from "./screens/BoardScreen";
import { WorkspaceScreen } from "./screens/WorkspaceScreen";
import { store } from "./state/store";
import { cycleSidebar, sidebarMode } from "./state/ui";

function Topbar() {
  return (
    <header class="topbar">
      <span class="topbar-mark mono">conduit</span>
      <span class="topbar-sep">/</span>
      <span class="topbar-context">{route().name === "board" ? "board" : "workspace"}</span>
      <span class="topbar-spacer" />
      <span
        class="conn-pill"
        classList={{
          open: store.conn === "open",
          connecting: store.conn === "connecting",
          closed: store.conn === "closed",
        }}
      >
        {store.conn}
      </span>
    </header>
  );
}

export function App() {
  const onKey = (e: KeyboardEvent) => {
    if (e.ctrlKey && !e.metaKey && (e.key === "b" || e.key === "B")) {
      e.preventDefault();
      cycleSidebar();
    }
  };
  onMount(() => window.addEventListener("keydown", onKey));
  onCleanup(() => window.removeEventListener("keydown", onKey));

  return (
    <>
      <Topbar />
      <div class="shell">
        <Show when={sidebarMode() !== "hidden"}>
          <Sidebar />
        </Show>
        <main class="main">
          <ConnectionBanner />
          <Switch fallback={<BoardScreen />}>
            <Match when={currentWorkspaceId()} keyed>
              {(id) => <WorkspaceScreen id={id} />}
            </Match>
          </Switch>
        </main>
      </div>
      <Toasts />
      <Dialogs />
      <AppModals />
    </>
  );
}
