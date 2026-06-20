import { createMemo, createSignal, For, Show } from "solid-js";
import { promptDialog } from "../state/dialogs";
import { git } from "../state/git-actions";
import { deleteWorkspace, renameWorkspace } from "../state/manage";
import { openAddRepository, openSettings } from "../state/modals";
import { closePalette } from "../state/palette";
import { repoName } from "../state/selectors";
import { store } from "../state/store";
import { navigate, route } from "../router";
import { Modal } from "./Modal";

interface Action {
  id: string;
  label: string;
  hint?: string;
  run: () => void;
}

function buildActions(): Action[] {
  const acts: Action[] = [];
  const r = route();

  if (r.name === "workspace") {
    const ws = store.workspaces.find((w) => w.id === r.id);
    if (ws) {
      acts.push({ id: "stage-all", label: "Stage all changes", run: () => git.stageAll(ws.id) });
      acts.push({
        id: "commit",
        label: "Commit…",
        run: async () => {
          const m = await promptDialog({ title: "Commit staged changes", multiline: true, confirmLabel: "Commit" });
          if (m && m.trim()) git.commit(ws.id, m.trim());
        },
      });
      acts.push({ id: "push", label: "Push", run: () => git.push(ws.id) });
      acts.push({ id: "pull", label: "Pull", run: () => git.pull(ws.id) });
      acts.push({ id: "fetch", label: "Fetch", run: () => git.fetch(ws.id) });
      acts.push({ id: "refresh", label: "Refresh git", run: () => git.refresh(ws.id) });
      acts.push({
        id: "review",
        label: ws.ready_for_review ? "Unmark ready for review" : "Mark ready for review",
        run: () => git.setReadyForReview(ws.id, !ws.ready_for_review),
      });
      acts.push({ id: "pr", label: "Open pull request", run: () => git.openPullRequest(ws.id) });
      acts.push({
        id: "run",
        label: "Run command in workspace…",
        run: async () => {
          const c = await promptDialog({ title: "Run in workspace", placeholder: "command", confirmLabel: "Run" });
          if (c && c.trim()) git.runCommand(ws.id, c.trim());
        },
      });
      acts.push({ id: "rename", label: "Rename workspace", run: () => void renameWorkspace(ws) });
      acts.push({ id: "delete", label: "Delete workspace", run: () => void deleteWorkspace(ws) });
    }
  }

  acts.push({ id: "add-repo", label: "Add repository", run: openAddRepository });
  acts.push({ id: "settings", label: "Open settings", run: openSettings });
  acts.push({ id: "board", label: "Go to board", run: () => navigate({ name: "board" }) });
  for (const w of store.workspaces) {
    acts.push({
      id: `go-${w.id}`,
      label: `Go to ${w.name}`,
      hint: repoName(w),
      run: () => navigate({ name: "workspace", id: w.id }),
    });
  }
  return acts;
}

export function CommandPalette() {
  const [q, setQ] = createSignal("");
  const [idx, setIdx] = createSignal(0);

  const filtered = createMemo(() => {
    const f = q().toLowerCase();
    const acts = buildActions();
    return f ? acts.filter((a) => a.label.toLowerCase().includes(f)) : acts;
  });

  const run = (a: Action) => {
    closePalette();
    a.run();
  };

  const onKey = (e: KeyboardEvent) => {
    const items = filtered();
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setIdx((i) => Math.min(i + 1, items.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setIdx((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const a = items[Math.min(idx(), items.length - 1)];
      if (a) run(a);
    }
  };

  return (
    <Modal onClose={closePalette} width={520}>
      <input
        class="palette-input mono"
        autofocus
        placeholder="Type a command or workspace…"
        value={q()}
        onInput={(e) => {
          setQ(e.currentTarget.value);
          setIdx(0);
        }}
        onKeyDown={onKey}
      />
      <ul class="palette-list">
        <For each={filtered()} fallback={<li class="gpanel-empty sm">No matches.</li>}>
          {(a, i) => (
            <li
              class="palette-item"
              classList={{ active: i() === idx() }}
              onClick={() => run(a)}
              onMouseEnter={() => setIdx(i())}
            >
              <span class="palette-label">{a.label}</span>
              <Show when={a.hint}>
                <span class="palette-hint mono">{a.hint}</span>
              </Show>
            </li>
          )}
        </For>
      </ul>
    </Modal>
  );
}
