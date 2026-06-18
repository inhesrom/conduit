import { createEffect, createSignal, Match, onMount, Switch } from "solid-js";
import type { WorkspaceSummary } from "@conduit/shared";
import { git } from "../../state/git-actions";
import { store } from "../../state/store";
import { BranchesPanel } from "./BranchesPanel";
import { ChangesPanel } from "./ChangesPanel";
import { CommitsPanel } from "./CommitsPanel";
import { DiffView } from "./DiffView";

type Tab = "changes" | "commits" | "branches";

export function GitRegion(props: { ws: WorkspaceSummary }) {
  const wsId = props.ws.id;
  const [tab, setTab] = createSignal<Tab>("changes");
  const [selected, setSelected] = createSignal<string | null>(null);

  onMount(() => git.refresh(wsId));

  const select = (file: string) => {
    git.loadDiff(wsId, file);
    setSelected(file);
  };
  // From the Commits panel the diff is loaded by the panel itself.
  const selectLoaded = (file: string) => setSelected(file);

  // Auto-open the first change's diff so the pane isn't empty on arrival.
  createEffect(() => {
    if (selected() || tab() !== "changes") return;
    const changed = store.gitByWs[wsId]?.changed;
    if (changed && changed.length > 0) select(changed[0]!.path);
  });

  const changedCount = () => store.gitByWs[wsId]?.changed.length ?? 0;

  return (
    <section class="git-region">
      <div class="git-left">
        <div class="git-tabs">
          <button class="git-tab" classList={{ active: tab() === "changes" }} onClick={() => setTab("changes")}>
            Changes
            <span class="git-tab-count">{changedCount()}</span>
          </button>
          <button class="git-tab" classList={{ active: tab() === "commits" }} onClick={() => setTab("commits")}>
            Commits
          </button>
          <button class="git-tab" classList={{ active: tab() === "branches" }} onClick={() => setTab("branches")}>
            Branches
          </button>
        </div>
        <div class="git-left-body">
          <Switch>
            <Match when={tab() === "changes"}>
              <ChangesPanel wsId={wsId} selected={selected} onSelect={select} />
            </Match>
            <Match when={tab() === "commits"}>
              <CommitsPanel wsId={wsId} selected={selected} onSelect={selectLoaded} />
            </Match>
            <Match when={tab() === "branches"}>
              <BranchesPanel wsId={wsId} />
            </Match>
          </Switch>
        </div>
      </div>
      <div class="git-right">
        <DiffView wsId={wsId} />
      </div>
    </section>
  );
}
