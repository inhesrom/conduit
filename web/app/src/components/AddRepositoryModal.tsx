import { createResource, createSignal, For, Show } from "solid-js";
import { registerRepository } from "../state/manage";
import { closeAppModal } from "../state/modals";
import { pushToast } from "../state/toasts";
import { Modal } from "./Modal";

interface FsEntry {
  name: string;
  path: string;
  is_repo: boolean;
}
interface Listing {
  path: string;
  parent: string | null;
  entries: FsEntry[];
}

async function fetchListing(path: string | undefined): Promise<Listing> {
  const url = "/api/fs/list" + (path ? `?path=${encodeURIComponent(path)}` : "");
  const res = await fetch(url);
  if (!res.ok) throw new Error(`Couldn't read that folder.`);
  return res.json();
}

function basename(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

export function AddRepositoryModal() {
  const [tab, setTab] = createSignal<"local" | "ssh">("local");
  const [cwd, setCwd] = createSignal<string | undefined>(undefined);
  // Wrap the source so it's always truthy — otherwise Solid skips the fetch
  // for the initial undefined path (which means "home" to the server).
  const [listing] = createResource(
    () => ({ path: cwd() }),
    (s) => fetchListing(s.path),
  );

  const [host, setHost] = createSignal("");
  const [user, setUser] = createSignal("");
  const [sshPath, setSshPath] = createSignal("");

  const addLocal = (path: string) => {
    registerRepository({ name: basename(path), path });
    pushToast("ok", `Added ${basename(path)}`);
    closeAppModal();
  };

  const addSsh = () => {
    if (!host().trim() || !sshPath().trim()) return;
    registerRepository({
      name: basename(sshPath()),
      path: sshPath().trim(),
      ssh: { host: host().trim(), user: user().trim() || null, port: null },
    });
    pushToast("ok", `Added ${basename(sshPath())} on ${host()}`);
    closeAppModal();
  };

  return (
    <Modal onClose={closeAppModal} width={560}>
      <h2 class="modal-title">Add repository</h2>
      <div class="seg">
        <button class="seg-btn" classList={{ active: tab() === "local" }} onClick={() => setTab("local")}>
          This machine
        </button>
        <button class="seg-btn" classList={{ active: tab() === "ssh" }} onClick={() => setTab("ssh")}>
          Over SSH
        </button>
      </div>

      <Show when={tab() === "local"}>
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
          </div>
          <ul class="browser-list">
            <Show when={!listing.loading} fallback={<li class="gpanel-empty sm">Loading…</li>}>
              <For each={listing()?.entries} fallback={<li class="gpanel-empty sm">No subfolders.</li>}>
                {(e) => (
                  <li class="browser-row">
                    <button class="browser-name mono" onClick={() => setCwd(e.path)}>
                      <span class="browser-icon">{e.is_repo ? "◆" : "▸"}</span>
                      {e.name}
                    </button>
                    <Show when={e.is_repo}>
                      <button class="btn xs primary" onClick={() => addLocal(e.path)}>
                        Add
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
          <div class="modal-actions">
            <button class="btn" onClick={closeAppModal}>
              Cancel
            </button>
            <button class="btn primary" disabled={!listing()?.path} onClick={() => addLocal(listing()!.path)}>
              Add this folder
            </button>
          </div>
        </div>
      </Show>

      <Show when={tab() === "ssh"}>
        <label class="field">
          <span class="field-label">Host</span>
          <input class="modal-input mono" placeholder="example.com" value={host()} onInput={(e) => setHost(e.currentTarget.value)} />
        </label>
        <label class="field">
          <span class="field-label">User (optional)</span>
          <input class="modal-input mono" value={user()} onInput={(e) => setUser(e.currentTarget.value)} />
        </label>
        <label class="field">
          <span class="field-label">Path</span>
          <input class="modal-input mono" placeholder="/home/you/project" value={sshPath()} onInput={(e) => setSshPath(e.currentTarget.value)} />
        </label>
        <div class="modal-actions">
          <button class="btn" onClick={closeAppModal}>
            Cancel
          </button>
          <button class="btn primary" disabled={!host().trim() || !sshPath().trim()} onClick={addSsh}>
            Add repository
          </button>
        </div>
      </Show>
    </Modal>
  );
}
