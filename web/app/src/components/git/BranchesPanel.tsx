import { For, Show } from "solid-js";
import { confirmDialog, promptDialog } from "../../state/dialogs";
import { git } from "../../state/git-actions";
import { store } from "../../state/store";

export function BranchesPanel(props: { wsId: string }) {
  const local = () => store.gitByWs[props.wsId]?.local_branches ?? [];
  const remote = () => store.gitByWs[props.wsId]?.remote_branches ?? [];

  const create = async () => {
    const name = await promptDialog({ title: "Create branch", placeholder: "branch-name", confirmLabel: "Create" });
    if (name && name.trim()) git.createBranch(props.wsId, name.trim());
  };

  const deleteLocal = async (branch: string) => {
    if (await confirmDialog({ title: "Delete local branch?", body: branch, confirmLabel: "Delete", danger: true })) {
      git.deleteLocal(props.wsId, branch);
    }
  };

  const deleteRemote = async (fullName: string) => {
    const slash = fullName.indexOf("/");
    const remoteName = slash >= 0 ? fullName.slice(0, slash) : "origin";
    const branch = slash >= 0 ? fullName.slice(slash + 1) : fullName;
    if (await confirmDialog({ title: "Delete remote branch?", body: fullName, confirmLabel: "Delete", danger: true })) {
      git.deleteRemote(props.wsId, remoteName, branch);
    }
  };

  const checkoutRemote = (fullName: string) => {
    const slash = fullName.indexOf("/");
    const localName = slash >= 0 ? fullName.slice(slash + 1) : fullName;
    git.checkoutRemote(props.wsId, fullName, localName);
  };

  return (
    <div class="gpanel">
      <div class="gpanel-actions">
        <button class="btn xs" onClick={() => git.fetch(props.wsId)}>
          Fetch
        </button>
        <button class="btn xs" onClick={() => git.pull(props.wsId)}>
          Pull
        </button>
        <button class="btn xs" onClick={() => git.push(props.wsId)}>
          Push
        </button>
        <span class="gpanel-spacer" />
        <button class="btn xs" onClick={create}>
          New branch
        </button>
      </div>

      <div class="branch-section eyebrow">Local</div>
      <ul class="blist">
        <For each={local()}>
          {(b) => (
            <li class="brow-b" classList={{ head: b.is_head }}>
              <button
                class="brow-b-name mono"
                disabled={b.is_head}
                title={b.is_head ? "Current branch" : "Checkout"}
                onClick={() => git.checkoutBranch(props.wsId, b.name)}
              >
                <span class="brow-b-mark">{b.is_head ? "●" : "○"}</span>
                {b.name}
              </button>
              <Show when={b.ahead || b.behind}>
                <span class="brow-b-track mono">
                  {b.ahead ? `↑${b.ahead}` : ""} {b.behind ? `↓${b.behind}` : ""}
                </span>
              </Show>
              <Show when={!b.is_head}>
                <button class="frow-discard" title="Delete branch" onClick={() => deleteLocal(b.name)}>
                  ⌫
                </button>
              </Show>
            </li>
          )}
        </For>
      </ul>

      <Show when={remote().length > 0}>
        <div class="branch-section eyebrow">Remote</div>
        <ul class="blist">
          <For each={remote()}>
            {(b) => (
              <li class="brow-b">
                <button class="brow-b-name mono" title="Check out as local branch" onClick={() => checkoutRemote(b.full_name)}>
                  <span class="brow-b-mark">○</span>
                  {b.full_name}
                </button>
                <button class="frow-discard" title="Delete remote branch" onClick={() => deleteRemote(b.full_name)}>
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
