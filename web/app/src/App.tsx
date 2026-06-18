import { createSignal, For, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { AppModals } from "./components/AppModals";
import { CommandPalette } from "./components/CommandPalette";
import { ConnectionBanner } from "./components/ConnectionBanner";
import { Dialogs } from "./components/Dialogs";
import { LoginScreen } from "./components/LoginScreen";
import { SessionPicker } from "./components/SessionPicker";
import { Sidebar } from "./components/Sidebar";
import { Toasts } from "./components/Toasts";
import { currentWorkspaceId, route } from "./router";
import { BoardScreen } from "./screens/BoardScreen";
import { WorkspaceScreen } from "./screens/WorkspaceScreen";
import { openSettings } from "./state/modals";
import { paletteOpen, togglePalette } from "./state/palette";
import { authState, checkSession } from "./state/session";
import { currentSession, loaded, pinned, refreshSessions, selectSession, sessions } from "./state/sessions";
import { store } from "./state/store";
import { cycleSidebar, sidebarMode } from "./state/ui";

function SessionSwitcher() {
  const [open, setOpen] = createSignal(false);
  return (
    <div class="ws-actions">
      <button
        class="session-chip mono"
        disabled={pinned()}
        title={pinned() ? "Pinned session" : "Switch session"}
        onClick={() => {
          if (pinned()) return;
          void refreshSessions();
          setOpen((o) => !o);
        }}
      >
        {currentSession()}
        <Show when={!pinned()}>
          <span class="session-caret">▾</span>
        </Show>
      </button>
      <Show when={open()}>
        <div class="menu-catcher" onClick={() => setOpen(false)} />
        <div class="menu">
          <For each={sessions()}>
            {(s) => (
              <button
                class="menu-item"
                classList={{ active: s === currentSession() }}
                onClick={() => {
                  setOpen(false);
                  selectSession(s);
                }}
              >
                {s}
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

function Topbar() {
  return (
    <header class="topbar">
      <span class="topbar-mark mono">conduit</span>
      <span class="topbar-sep">/</span>
      <SessionSwitcher />
      <span class="topbar-sep">/</span>
      <span class="topbar-context">{route().name === "board" ? "board" : "workspace"}</span>
      <span class="topbar-spacer" />
      <button class="topbar-btn" title="Command palette (⌘K)" onClick={togglePalette}>
        ⌘K
      </button>
      <button class="topbar-btn" title="Settings" onClick={openSettings}>
        ⚙
      </button>
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

function AppShell() {
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
      <Show when={paletteOpen()}>
        <CommandPalette />
      </Show>
    </>
  );
}

export function App() {
  const onKey = (e: KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "K")) {
      e.preventDefault();
      togglePalette();
    } else if (e.ctrlKey && !e.metaKey && (e.key === "b" || e.key === "B")) {
      e.preventDefault();
      cycleSidebar();
    }
  };
  onMount(() => {
    window.addEventListener("keydown", onKey);
    void checkSession();
  });
  onCleanup(() => window.removeEventListener("keydown", onKey));

  return (
    <Show when={authState() !== "needed"} fallback={<LoginScreen />}>
      <Show when={currentSession()} fallback={<Show when={loaded()}>{<SessionPicker />}</Show>}>
        <AppShell />
      </Show>
    </Show>
  );
}
