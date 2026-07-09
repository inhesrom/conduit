import { createSignal, For, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { AppModals } from "./components/AppModals";
import { CommandPalette } from "./components/CommandPalette";
import { ConduitMark } from "./components/ConduitMark";
import { ConnectionBanner } from "./components/ConnectionBanner";
import { Dialogs } from "./components/Dialogs";
import { LoginScreen } from "./components/LoginScreen";
import { SessionPicker } from "./components/SessionPicker";
import { Sidebar } from "./components/Sidebar";
import { GitSidebar } from "./components/git/GitSidebar";
import { Toasts } from "./components/Toasts";
import { route } from "./router";
import { BoardScreen } from "./screens/BoardScreen";
import { PullRequestScreen } from "./screens/PullRequestScreen";
import { WorkspaceScreen } from "./screens/WorkspaceScreen";
import { openSettings } from "./state/modals";
import { paletteOpen, togglePalette } from "./state/palette";
import { theme, cycleTheme } from "./state/theme";
import { authState, checkSession } from "./state/session";
import { confirmDialog } from "./state/dialogs";
import {
  createSession,
  currentSession,
  deleteSession,
  desktop,
  loaded,
  pinned,
  refreshSessions,
  selectSession,
  sessions,
} from "./state/sessions";
import { settings } from "./state/settings";
import { store } from "./state/store";
import { cycleGitSidebar, cycleSidebar, gitSidebarMode, sidebarMode } from "./state/ui";

function SessionSwitcher() {
  const [open, setOpen] = createSignal(false);
  const [name, setName] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  // Create a new session and attach to it (createSession attaches on success).
  // Desktop-only — the shared web build rejects names not in the registry.
  const create = async () => {
    if (busy() || !name().trim()) return;
    setBusy(true);
    setError(null);
    const err = await createSession(name());
    setBusy(false);
    if (err) {
      setError(err);
      return;
    }
    setName("");
    setOpen(false);
  };

  // Destructive: confirm first, then stop the daemon and forget the session.
  // The currently attached session is refused in-app.
  const remove = async (target: string, e: MouseEvent) => {
    e.stopPropagation();
    if (target === currentSession()) {
      setError("cannot delete the attached session");
      return;
    }
    const ok = await confirmDialog({
      title: `Delete session "${target}"?`,
      body: "This stops its daemon and removes the session from Conduit. Worktrees on disk are left intact.",
      confirmLabel: "Delete",
      danger: true,
    });
    if (!ok) return;
    setOpen(false);
    const err = await deleteSession(target);
    if (err) setError(err);
  };

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
              <div class="menu-row">
                <button
                  class="menu-item"
                  classList={{ active: s.name === currentSession(), stale: !s.running }}
                  onClick={() => {
                    setOpen(false);
                    void selectSession(s.name);
                  }}
                >
                  {s.name}
                  <Show when={!s.running}>
                    <span class="session-stale">stale</span>
                  </Show>
                </button>
                <Show when={desktop()}>
                  <button
                    class="menu-del"
                    title={
                      s.name === currentSession()
                        ? "Cannot delete the attached session"
                        : "Delete session"
                    }
                    aria-label={`Delete session ${s.name}`}
                    disabled={s.name === currentSession()}
                    onClick={(e) => void remove(s.name, e)}
                  >
                    ×
                  </button>
                </Show>
              </div>
            )}
          </For>
          <Show when={desktop()}>
            <div class="menu-sep" />
            <div class="session-create">
              <input
                class="modal-input mono"
                placeholder="New session name"
                value={name()}
                disabled={busy()}
                onInput={(e) => setName(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void create();
                }}
              />
              <button
                class="btn primary"
                disabled={busy() || !name().trim()}
                onClick={() => void create()}
              >
                {busy() ? "…" : "New"}
              </button>
            </div>
            <Show when={error()}>
              <p class="session-empty">{error()}</p>
            </Show>
          </Show>
        </div>
      </Show>
    </div>
  );
}

function Topbar() {
  return (
    <header class="topbar">
      <span class="topbar-mark mono">
        <ConduitMark size={28} class="topbar-logo" />
        conduit
      </span>
      <span class="topbar-sep">/</span>
      <SessionSwitcher />
      <span class="topbar-sep">/</span>
      <span class="topbar-context">
        {route().name === "board" ? "board" : route().name === "pr" ? "pr viewer" : "workspace"}
      </span>
      <span class="topbar-spacer" />
      <button class="topbar-btn" title="Command palette (⌘K)" onClick={togglePalette}>
        ⌘K
      </button>
      <button
        class="topbar-btn mono"
        title={`Theme: ${theme()} — click to cycle (amber → mono → paper)`}
        onClick={cycleTheme}
      >
        {theme().charAt(0).toUpperCase()}
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
  const prRouteId = () => {
    const r = route();
    return r.name === "pr" ? r.id : null;
  };
  const workspaceRouteId = () => {
    const r = route();
    return r.name === "workspace" ? r.id : null;
  };

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
            <Match when={prRouteId()} keyed>
              {(id) => <PullRequestScreen id={id} />}
            </Match>
            <Match when={workspaceRouteId()} keyed>
              {(id) => <WorkspaceScreen id={id} />}
            </Match>
          </Switch>
        </main>
        <Show
          when={
            settings.gitLayout === "sidebar" &&
            gitSidebarMode() !== "hidden" &&
            workspaceRouteId()
          }
          keyed
        >
          {(id) => <GitSidebar wsId={id} />}
        </Show>
      </div>
      <Toasts />
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
    } else if (e.ctrlKey && !e.metaKey && e.shiftKey && (e.key === "b" || e.key === "B")) {
      e.preventDefault();
      cycleGitSidebar();
    } else if (e.ctrlKey && !e.metaKey && !e.shiftKey && (e.key === "b" || e.key === "B")) {
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
    <>
      <Show when={authState() !== "needed"} fallback={<LoginScreen />}>
        <Show when={currentSession()} fallback={<Show when={loaded()}>{<SessionPicker />}</Show>}>
          <AppShell />
        </Show>
      </Show>
      {/* Mounted at the root so promise-based confirms work on the chooser too. */}
      <Dialogs />
    </>
  );
}
