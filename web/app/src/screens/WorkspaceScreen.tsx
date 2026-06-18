import { Show } from "solid-js";
import { navigate } from "../router";
import { repoName } from "../state/selectors";
import { store } from "../state/store";
import { StatusGlyph } from "../components/StatusGlyph";
import { TerminalRegion } from "../components/TerminalRegion";

export function WorkspaceScreen(props: { id: string }) {
  const ws = () => store.workspaces.find((w) => w.id === props.id);
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
        </header>
        <div class="ws-grid">
          <TerminalRegion ws={ws()!} />
          <section class="git-region">
            <div class="empty git-stub">Git status, log, branches, and diff arrive next.</div>
          </section>
        </div>
      </Show>
    </div>
  );
}
