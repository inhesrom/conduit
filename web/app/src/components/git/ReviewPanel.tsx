import { For, onMount, Show } from "solid-js";
import type { ChangedFile, WorkspaceSummary } from "@conduit/shared";
import { git } from "../../state/git-actions";
import { store } from "../../state/store";

function statusChar(f: ChangedFile): string {
  return f.index_status !== " " ? f.index_status : f.worktree_status;
}

export function ReviewPanel(props: {
  ws: WorkspaceSummary;
  selected: () => string | null;
  onSelect: (file: string) => void;
}) {
  const wsId = props.ws.id;
  onMount(() => git.loadBranchDiff(wsId));
  const review = () => store.reviewByWs[wsId];

  return (
    <div class="gpanel">
      <div class="gpanel-actions">
        <button
          class="btn xs"
          classList={{ primary: props.ws.ready_for_review }}
          onClick={() => git.setReadyForReview(wsId, !props.ws.ready_for_review)}
        >
          {props.ws.ready_for_review ? "◆ Ready" : "Mark ready"}
        </button>
        <span class="gpanel-spacer" />
        <button class="btn xs" onClick={() => git.push(wsId)}>
          Push
        </button>
        <button class="btn xs primary" onClick={() => git.openPullRequest(wsId)}>
          Open PR
        </button>
      </div>
      <Show when={review()} fallback={<div class="gpanel-empty">Loading branch diff…</div>}>
        <div class="branch-section eyebrow">vs {review()!.base}</div>
        <Show
          when={review()!.files.length > 0}
          fallback={<div class="gpanel-empty sm">No changes vs base branch.</div>}
        >
          <ul class="flist">
            <For each={review()!.files}>
              {(f) => (
                <li class="frow" classList={{ selected: props.selected() === f.path }}>
                  <span class={`frow-status s-${statusChar(f).toLowerCase()}`}>{statusChar(f)}</span>
                  <button
                    class="frow-path mono"
                    onClick={() => {
                      git.loadBranchFileDiff(wsId, f.path);
                      props.onSelect(f.path);
                    }}
                    title={f.path}
                  >
                    {f.path}
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </Show>
    </div>
  );
}
