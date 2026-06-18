import { createMemo, For, Show } from "solid-js";
import type { ChangedFile } from "@conduit/shared";
import { confirmDialog, promptDialog } from "../../state/dialogs";
import { git } from "../../state/git-actions";
import { store } from "../../state/store";

function isStaged(f: ChangedFile): boolean {
  return f.index_status !== " " && f.index_status !== "?";
}
function isUntracked(f: ChangedFile): boolean {
  return f.index_status === "?" && f.worktree_status === "?";
}
function statusChar(f: ChangedFile): string {
  if (isUntracked(f)) return "?";
  if (f.index_status !== " ") return f.index_status;
  return f.worktree_status;
}

export function ChangesPanel(props: {
  wsId: string;
  selected: () => string | null;
  onSelect: (file: string) => void;
}) {
  const changed = () => store.gitByWs[props.wsId]?.changed ?? [];
  const hasStaged = createMemo(() => changed().some(isStaged));

  const commit = async () => {
    const msg = await promptDialog({
      title: "Commit staged changes",
      placeholder: "Commit message",
      confirmLabel: "Commit",
      multiline: true,
    });
    if (msg && msg.trim()) git.commit(props.wsId, msg.trim());
  };

  const discard = async (file: string) => {
    if (await confirmDialog({ title: "Discard changes?", body: file, confirmLabel: "Discard", danger: true })) {
      git.discardFile(props.wsId, file);
    }
  };

  const discardAll = async () => {
    if (await confirmDialog({ title: "Discard all changes?", confirmLabel: "Discard all", danger: true })) {
      git.discardAll(props.wsId);
    }
  };

  return (
    <div class="gpanel">
      <div class="gpanel-actions">
        <button class="btn xs" title="Stage all" onClick={() => git.stageAll(props.wsId)}>
          Stage all
        </button>
        <button class="btn xs" title="Unstage all" onClick={() => git.unstageAll(props.wsId)}>
          Unstage all
        </button>
        <button class="btn xs" disabled={!hasStaged()} onClick={commit}>
          Commit
        </button>
        <span class="gpanel-spacer" />
        <button class="btn xs ghost" title="Discard all" onClick={discardAll}>
          Discard all
        </button>
      </div>
      <Show when={changed().length > 0} fallback={<div class="gpanel-empty">Working tree clean.</div>}>
        <ul class="flist">
          <For each={changed()}>
            {(f) => (
              <li class="frow" classList={{ selected: props.selected() === f.path }}>
                <button
                  class="frow-stage"
                  classList={{ staged: isStaged(f) }}
                  title={isStaged(f) ? "Unstage" : "Stage"}
                  onClick={() => (isStaged(f) ? git.unstageFile(props.wsId, f.path) : git.stageFile(props.wsId, f.path))}
                >
                  {isStaged(f) ? "−" : "+"}
                </button>
                <span class={`frow-status s-${statusChar(f).toLowerCase()}`}>{statusChar(f)}</span>
                <button class="frow-path mono" onClick={() => props.onSelect(f.path)} title={f.path}>
                  {f.path}
                </button>
                <button class="frow-discard" title="Discard file" onClick={() => discard(f.path)}>
                  ⌫
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}
