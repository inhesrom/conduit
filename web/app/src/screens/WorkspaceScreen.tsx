import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { navigate } from "../router";
import { git } from "../state/git-actions";
import { deleteWorkspace, renameWorkspace } from "../state/manage";
import { repoName } from "../state/selectors";
import { settings } from "../state/settings";
import { gitCollapsed, setTermPct, termPct, toggleGitCollapsed } from "../state/split";
import { cycleGitSidebar, selectedFile, setSelectedFile } from "../state/ui";
import { store } from "../state/store";
import { StatusGlyph } from "../components/StatusGlyph";
import { TerminalRegion } from "../components/TerminalRegion";
import { DiffView } from "../components/git/DiffView";
import { GitRegion } from "../components/git/GitRegion";

export function WorkspaceScreen(props: { id: string }) {
  const ws = () => store.workspaces.find((w) => w.id === props.id);
  const [menu, setMenu] = createSignal(false);

  // Sidebar layout: a selected file swaps the main area from terminal to diff.
  const fileSel = () => selectedFile(props.id);
  const closeDiff = () => setSelectedFile(props.id, null);
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape" && fileSel()) closeDiff();
  };
  onMount(() => window.addEventListener("keydown", onKey));
  onCleanup(() => window.removeEventListener("keydown", onKey));

  // Bottom-split layout: drag the handle to resize the terminal/git ratio.
  let gridEl: HTMLDivElement | undefined;
  const onDragStart = (e: PointerEvent) => {
    e.preventDefault();
    const move = (ev: PointerEvent) => {
      if (gridEl) setTermPct((ev.clientY - gridEl.getBoundingClientRect().top) / gridEl.clientHeight);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  return (
    <div class="ws-screen">
      <Show when={ws()} fallback={<div class="empty">Workspace not found.</div>}>
        <header class="ws-screen-head">
          <button class="back" title="Back to board" onClick={() => navigate({ name: "board" })}>
            ←
          </button>
          <StatusGlyph ws={ws()!} />
          <span class="ws-screen-name">{ws()!.name}</span>
          <span class="ws-screen-repo mono">{repoName(ws()!)}</span>
          <Show when={ws()!.branch}>
            <span class="ws-screen-branch mono">{ws()!.branch}</span>
          </Show>
          <span class="ws-screen-spacer" />
          <Show when={ws()!.ready_for_review}>
            <span class="ws-screen-review" title="Ready for review">
              ◆ ready
            </span>
          </Show>
          <Show when={settings.gitLayout === "sidebar"}>
            <button class="btn xs" title="Toggle git sidebar (Ctrl+Shift+B)" onClick={cycleGitSidebar}>
              git
            </button>
          </Show>
          <div class="ws-actions">
            <button class="icon-btn" title="Workspace actions" onClick={() => setMenu((m) => !m)}>
              ⋯
            </button>
            <Show when={menu()}>
              <div class="menu-catcher" onClick={() => setMenu(false)} />
              <div class="menu">
                <button
                  class="menu-item"
                  onClick={() => {
                    setMenu(false);
                    git.setReadyForReview(ws()!.id, !ws()!.ready_for_review);
                  }}
                >
                  {ws()!.ready_for_review ? "Unmark ready for review" : "Mark ready for review"}
                </button>
                <button
                  class="menu-item"
                  onClick={() => {
                    setMenu(false);
                    void renameWorkspace(ws()!);
                  }}
                >
                  Rename
                </button>
                <button
                  class="menu-item danger"
                  onClick={() => {
                    setMenu(false);
                    void deleteWorkspace(ws()!);
                  }}
                >
                  Delete workspace
                </button>
              </div>
            </Show>
          </div>
        </header>
        <Show
          when={settings.gitLayout === "sidebar"}
          fallback={
            <div class="ws-grid" ref={gridEl}>
              <div
                class="ws-pane"
                style={{
                  "flex-basis": gitCollapsed() ? "auto" : `${termPct() * 100}%`,
                  "flex-grow": gitCollapsed() ? "1" : "0",
                  "flex-shrink": "1",
                }}
              >
                <TerminalRegion ws={ws()!} />
              </div>
              <div
                class="ws-split-handle"
                classList={{ collapsed: gitCollapsed() }}
                onPointerDown={(e) => {
                  if (!gitCollapsed()) onDragStart(e);
                }}
                title={gitCollapsed() ? "" : "Drag to resize"}
              >
                <span class="ws-split-grip">⣿⣿⣿⣿</span>
                <button
                  class="ws-split-toggle"
                  title={gitCollapsed() ? "Show git" : "Hide git"}
                  onPointerDown={(e) => e.stopPropagation()}
                  onClick={toggleGitCollapsed}
                >
                  {gitCollapsed() ? "▴ git" : "git ▾"}
                </button>
              </div>
              <Show when={!gitCollapsed()}>
                <div class="ws-pane">
                  <GitRegion ws={ws()!} />
                </div>
              </Show>
            </div>
          }
        >
          <div class="ws-main">
            {/* Terminal stays mounted (preserving xterm/PTY state) and is hidden
                when a diff is showing rather than unmounted. */}
            <div class="ws-main-term" style={{ display: fileSel() ? "none" : "flex" }}>
              <TerminalRegion ws={ws()!} />
            </div>
            <Show when={fileSel()}>
              <div class="ws-diff">
                <button class="ws-diff-close" title="Close diff (Esc)" onClick={closeDiff}>
                  ×
                </button>
                <DiffView wsId={props.id} />
              </div>
            </Show>
          </div>
        </Show>
      </Show>
    </div>
  );
}
