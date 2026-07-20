import { createMemo, createResource, createSignal, For, Match, onMount, Show, Switch } from "solid-js";
import { client } from "../client";
import { basename, fetchListing } from "../lib/fs";
import { branchSlug } from "../lib/slug";
import { closeAppModal } from "../state/modals";
import { settings } from "../state/settings";
import { setStore, store } from "../state/store";
import { Modal } from "./Modal";

type BranchChoice = { display: string; remote: boolean };

/** "new"/"existing" create a worktree; "folder" attaches one that already
 * exists — a checkout you or another agent already has work in flight in. */
type Mode = "new" | "existing" | "folder";

function localNameOf(remoteRef: string): string {
  const slash = remoteRef.indexOf("/");
  return slash >= 0 ? remoteRef.slice(slash + 1) : remoteRef;
}

export function CreateWorkspaceModal(props: { repoId: string }) {
  const repo = () => store.repositories.find((r) => r.id === props.repoId);
  const [name, setName] = createSignal("");
  const [mode, setMode] = createSignal<Mode>("new");
  const [base, setBase] = createSignal("");
  const [filter, setFilter] = createSignal("");
  const [picked, setPicked] = createSignal<BranchChoice | null>(null);
  const [agent, setAgent] = createSignal(repo()?.default_agent ?? settings.defaultAgent);
  const [prompt, setPrompt] = createSignal("");
  const [submitted, setSubmitted] = createSignal(false);
  const [folder, setFolder] = createSignal("");
  const [resume, setResume] = createSignal(true);
  // Folder browsing is its own cursor so typing a path doesn't yank the list
  // out from under you mid-browse.
  const [cwd, setCwd] = createSignal<string | undefined>(undefined);
  const [listing] = createResource(
    () => ({ path: cwd() }),
    (s) => fetchListing(s.path),
  );

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
  const canCreate = () => {
    if (submitted()) return false;
    if (mode() === "folder") return folder().trim().length > 0;
    return name().trim().length > 0 && (mode() === "new" || picked() !== null);
  };

  /** Point at a folder and name the workspace after it, unless the name was
   * typed by hand. */
  const chooseFolder = (path: string) => {
    const previous = folder();
    setFolder(path);
    if (!name().trim() || name() === basename(previous)) setName(basename(path));
  };

  const submitAdopt = () => {
    if (prompt().trim()) setStore("pendingCreatePrompt", prompt().trim());
    setStore("pendingCreateResume", resume());
    setSubmitted(true);
    client.send({
      AddWorkspace: {
        name: name().trim(),
        path: folder().trim(),
        ssh: null,
        repository_id: props.repoId,
        base_branch: base().trim() || null,
        agent: agent().trim() || null,
        adopted: true,
      },
    });
  };

  const submit = () => {
    if (!canCreate() || submitted()) return;
    if (mode() === "folder") return submitAdopt();
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
      <h2 class="modal-title">Add workspace to {repo()?.name}</h2>

      <Show when={submitted()} fallback={<Form />}>
        <div class="create-progress">
          <span class="spinner" />{" "}
          {mode() === "folder" ? "Attaching folder…" : (store.createProgress?.stage ?? "Creating worktree…")}
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
            placeholder={mode() === "folder" ? "Defaults to the folder name" : "What is this workspace for?"}
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && mode() !== "existing") submit();
            }}
          />
          {/* Only "new" derives a branch from the name; the other modes take
              the branch the folder or picker already has. */}
          <Show when={mode() === "new" && slug()}>
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
          <button class="seg-btn" classList={{ active: mode() === "folder" }} onClick={() => setMode("folder")}>
            Existing folder
          </button>
        </div>

        <Switch>
          <Match when={mode() === "new"}>
            <BaseBranchField />
          </Match>

          <Match when={mode() === "existing"}>
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
          </Match>

          <Match when={mode() === "folder"}>
            <label class="field">
              <span class="field-label">Folder</span>
              <input
                class="modal-input mono"
                placeholder="/home/you/project"
                value={folder()}
                onInput={(e) => chooseFolder(e.currentTarget.value)}
              />
            </label>

            <div class="browser">
              <div class="browser-bar mono">
                <button
                  class="btn xs"
                  disabled={!listing()?.parent}
                  onClick={() => setCwd(listing()?.parent ?? undefined)}
                >
                  ↑
                </button>
                <span class="browser-path">{listing()?.path ?? "…"}</span>
                <button class="btn xs" disabled={!listing()?.path} onClick={() => chooseFolder(listing()!.path)}>
                  Use this
                </button>
              </div>
              <ul class="browser-list">
                <Show when={!listing.loading} fallback={<li class="gpanel-empty sm">Loading…</li>}>
                  <For each={listing()?.entries} fallback={<li class="gpanel-empty sm">No subfolders.</li>}>
                    {(e) => (
                      <li class="browser-row" classList={{ selected: folder() === e.path }}>
                        <button class="browser-name mono" onClick={() => setCwd(e.path)}>
                          <span class="browser-icon">{e.is_repo ? "◆" : "▸"}</span>
                          {e.name}
                        </button>
                        <Show when={e.is_repo}>
                          <button class="btn xs" onClick={() => chooseFolder(e.path)}>
                            Pick
                          </button>
                        </Show>
                      </li>
                    )}
                  </For>
                </Show>
              </ul>
              <Show when={listing.error}>
                <div class="gpanel-empty sm">{String(listing.error?.message ?? listing.error)}</div>
              </Show>
            </div>

            <BaseBranchField />

            <label class="toggle-row">
              <input type="checkbox" checked={resume()} onChange={(e) => setResume(e.currentTarget.checked)} />
              <span>
                <span class="toggle-title">Resume previous agent session</span>
                <span class="toggle-hint">Launches the agent with its continue flag so it picks up where it left off.</span>
              </span>
            </label>
          </Match>
        </Switch>

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
            {mode() === "folder" ? "Add workspace" : "Create workspace"}
          </button>
        </div>
      </>
    );
  }

  function BaseBranchField() {
    return (
      <label class="field">
        <span class="field-label">Base branch</span>
        <input
          class="modal-input mono"
          placeholder={repo()?.default_branch ?? "default branch"}
          value={base()}
          onInput={(e) => setBase(e.currentTarget.value)}
        />
        <Show when={mode() === "folder"}>
          <span class="field-hint">What this folder's branch is reviewed against.</span>
        </Show>
      </label>
    );
  }
}
