import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { navigate } from "../router";
import { git } from "../state/git-actions";
import { deleteWorkspace, renameWorkspace } from "../state/manage";
import { repoName } from "../state/selectors";
import { cycleGitSidebar, selectedFile, setSelectedFile } from "../state/ui";
import { store } from "../state/store";
import { StatusGlyph } from "../components/StatusGlyph";
import { TerminalRegion } from "../components/TerminalRegion";
import { DiffView } from "../components/git/DiffView";

export function WorkspaceScreen(props: { id: string }) {
  const ws = () => store.workspaces.find((w) => w.id === props.id);
  const [menu, setMenu] = createSignal(false);

  const fileSel = () => selectedFile(props.id);
  const closeDiff = () => setSelectedFile(props.id, null);

  // Esc returns the main area from the diff to the terminal.
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape" && fileSel()) closeDiff();
  };
  onMount(() => window.addEventListener("keydown", onKey));
  onCleanup(() => window.removeEventListener("keydown", onKey));

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
          <button class="btn xs" title="Toggle git sidebar (Ctrl+Shift+B)" onClick={cycleGitSidebar}>
            git
          </button>
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
    </div>
  );
}
