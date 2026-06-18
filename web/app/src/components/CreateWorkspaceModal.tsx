import { createMemo, createSignal, For, onMount, Show } from "solid-js";
import { client } from "../client";
import { branchSlug } from "../lib/slug";
import { closeAppModal } from "../state/modals";
import { settings } from "../state/settings";
import { setStore, store } from "../state/store";
import { Modal } from "./Modal";

type BranchChoice = { display: string; remote: boolean };

function localNameOf(remoteRef: string): string {
  const slash = remoteRef.indexOf("/");
  return slash >= 0 ? remoteRef.slice(slash + 1) : remoteRef;
}

export function CreateWorkspaceModal(props: { repoId: string }) {
  const repo = () => store.repositories.find((r) => r.id === props.repoId);
  const [name, setName] = createSignal("");
  const [mode, setMode] = createSignal<"new" | "existing">("new");
  const [base, setBase] = createSignal("");
  const [filter, setFilter] = createSignal("");
  const [picked, setPicked] = createSignal<BranchChoice | null>(null);
  const [agent, setAgent] = createSignal(repo()?.default_agent ?? settings.defaultAgent);
  const [prompt, setPrompt] = createSignal("");
  const [submitted, setSubmitted] = createSignal(false);

  onMount(() => client.send({ ListRepoBranches: { repo_id: props.repoId } }));

  const branches = createMemo<BranchChoice[]>(() => {
    const rb = store.repoBranches[props.repoId];
    if (!rb) return [];
    const localSet = new Set(rb.local);
    const all: BranchChoice[] = [
      ...rb.local.map((n) => ({ display: n, remote: false })),
      ...rb.remote.filter((r) => !localSet.has(localNameOf(r))).map((r) => ({ display: r, remote: true })),
    ];
    const f = filter().toLowerCase();
    return f ? all.filter((b) => b.display.toLowerCase().includes(f)) : all;
  });

  const slug = () => branchSlug(name());
  const canCreate = () => name().trim().length > 0 && (mode() === "new" || picked() !== null);

  const submit = () => {
    if (!canCreate() || submitted()) return;
    const choice = picked();
    const existing =
      mode() === "existing" && choice
        ? choice.remote
          ? { RemoteBranch: { remote_ref: choice.display } }
          : { LocalBranch: { name: choice.display } }
        : null;
    if (prompt().trim()) setStore("pendingCreatePrompt", prompt().trim());
    setSubmitted(true);
    client.send({
      CreateWorkspace: {
        repo_id: props.repoId,
        name: name().trim(),
        base_branch: mode() === "new" ? base().trim() || null : null,
        agent: agent().trim() || null,
        existing,
      },
    });
  };

  return (
    <Modal onClose={closeAppModal} width={520}>
      <h2 class="modal-title">New workspace in {repo()?.name}</h2>

      <Show when={submitted()} fallback={<Form />}>
        <div class="create-progress">
          <span class="spinner" /> {store.createProgress?.stage ?? "Creating worktree…"}
        </div>
      </Show>
    </Modal>
  );

  function Form() {
    return (
      <>
        <label class="field">
          <span class="field-label">Name</span>
          <input
            class="modal-input"
            autofocus
            placeholder="What is this workspace for?"
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && mode() === "new") submit();
            }}
          />
          <Show when={slug()}>
            <span class="field-hint mono">branch: {slug()}</span>
          </Show>
        </label>

        <div class="seg">
          <button class="seg-btn" classList={{ active: mode() === "new" }} onClick={() => setMode("new")}>
            New branch
          </button>
          <button class="seg-btn" classList={{ active: mode() === "existing" }} onClick={() => setMode("existing")}>
            Existing branch
          </button>
        </div>

        <Show
          when={mode() === "new"}
          fallback={
            <div class="field">
              <input
                class="modal-input mono"
                placeholder="Filter branches…"
                value={filter()}
                onInput={(e) => setFilter(e.currentTarget.value)}
              />
              <ul class="branch-picker">
                <For each={branches()} fallback={<li class="gpanel-empty sm">No branches.</li>}>
                  {(b) => (
                    <li
                      class="branch-opt mono"
                      classList={{ selected: picked()?.display === b.display }}
                      onClick={() => setPicked(b)}
                    >
                      <span class="brow-b-mark">{b.remote ? "⬡" : "○"}</span>
                      {b.display}
                    </li>
                  )}
                </For>
              </ul>
            </div>
          }
        >
          <label class="field">
            <span class="field-label">Base branch</span>
            <input
              class="modal-input mono"
              placeholder={repo()?.default_branch ?? "default branch"}
              value={base()}
              onInput={(e) => setBase(e.currentTarget.value)}
            />
          </label>
        </Show>

        <label class="field">
          <span class="field-label">Agent</span>
          <input
            class="modal-input mono"
            list="agent-profiles"
            value={agent()}
            onInput={(e) => setAgent(e.currentTarget.value)}
          />
          <datalist id="agent-profiles">
            <For each={settings.agents}>{(a) => <option value={a.name} />}</For>
          </datalist>
        </label>

        <label class="field">
          <span class="field-label">Initial prompt (optional)</span>
          <textarea
            class="modal-input mono"
            rows={3}
            placeholder="Sent to the agent once it starts"
            value={prompt()}
            onInput={(e) => setPrompt(e.currentTarget.value)}
          />
        </label>

        <div class="modal-actions">
          <button class="btn" onClick={closeAppModal}>
            Cancel
          </button>
          <button class="btn primary" disabled={!canCreate()} onClick={submit}>
            Create workspace
          </button>
        </div>
      </>
    );
  }
}
