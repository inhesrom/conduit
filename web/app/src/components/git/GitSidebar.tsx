import { createEffect, createSignal, Match, Show, Switch } from "solid-js";
import { git } from "../../state/git-actions";
import { store } from "../../state/store";
import {
  cycleGitSidebar,
  GIT_DEFAULT_WIDTH,
  gitSidebarMode,
  gitSidebarWidth,
  selectedFile,
  setGitSidebarMode,
  setGitSidebarWidth,
  setSelectedFile,
} from "../../state/ui";
import { BranchesPanel } from "./BranchesPanel";
import { ChangesPanel } from "./ChangesPanel";
import { CommitsPanel } from "./CommitsPanel";
import { ReviewPanel } from "./ReviewPanel";

type Tab = "changes" | "commits" | "branches" | "review";

export function GitSidebar(props: { wsId: string }) {
  const ws = () => store.workspaces.find((w) => w.id === props.wsId);
  const [tab, setTab] = createSignal<Tab>("changes");

  // The sidebar lives at app level and is not remounted per workspace, so
  // refresh whenever the active workspace changes.
  createEffect(() => git.refresh(props.wsId));

  const selected = () => selectedFile(props.wsId);
  // Clicking the active file again clears it, returning the main area to the
  // terminal.
  const select = (file: string) => {
    if (selected() === file) {
      setSelectedFile(props.wsId, null);
      return;
    }
    git.loadDiff(props.wsId, file);
    setSelectedFile(props.wsId, file);
  };
  // From Commits/Review the diff is loaded by the panel itself.
  const selectLoaded = (file: string) =>
    setSelectedFile(props.wsId, selected() === file ? null : file);

  const changedCount = () => store.gitByWs[props.wsId]?.changed.length ?? 0;

  let navEl: HTMLElement | undefined;
  const onResizeStart = (e: PointerEvent) => {
    e.preventDefault();
    const move = (ev: PointerEvent) => {
      const right = navEl ? navEl.getBoundingClientRect().right : 0;
      setGitSidebarWidth(right - ev.clientX);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  return (
    <Show when={gitSidebarMode() === "expanded"} fallback={<GitRail tab={tab()} setTab={setTab} changedCount={changedCount()} ready={ws()?.ready_for_review} />}>
      <nav class="git-sidebar" ref={navEl} style={{ width: `${gitSidebarWidth()}px` }}>
        <div class="git-sidebar-head">
          <span class="eyebrow">Git</span>
          <button class="icon-btn" title="Collapse git sidebar (Ctrl+Shift+B)" onClick={cycleGitSidebar}>
            ›
          </button>
        </div>
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
          <button class="git-tab" classList={{ active: tab() === "review" }} onClick={() => setTab("review")}>
            Review
            <Show when={ws()?.ready_for_review}>
              <span class="git-tab-review">◆</span>
            </Show>
          </button>
        </div>
        <div class="git-left-body">
          <Switch>
            <Match when={tab() === "changes"}>
              <ChangesPanel wsId={props.wsId} selected={selected} onSelect={select} />
            </Match>
            <Match when={tab() === "commits"}>
              <CommitsPanel wsId={props.wsId} selected={selected} onSelect={selectLoaded} />
            </Match>
            <Match when={tab() === "branches"}>
              <BranchesPanel wsId={props.wsId} />
            </Match>
            <Match when={tab() === "review" && !!ws()}>
              <ReviewPanel ws={ws()!} selected={selected} onSelect={selectLoaded} />
            </Match>
          </Switch>
        </div>
        <div
          class="git-sidebar-resize"
          title="Drag to resize"
          onPointerDown={onResizeStart}
          onDblClick={() => setGitSidebarWidth(GIT_DEFAULT_WIDTH)}
        />
      </nav>
    </Show>
  );
}

function GitRail(props: {
  tab: Tab;
  setTab: (t: Tab) => void;
  changedCount: number;
  ready?: boolean;
}) {
  const open = (t: Tab) => {
    props.setTab(t);
    setGitSidebarMode("expanded");
  };
  return (
    <nav class="git-sidebar rail">
      <button class="icon-btn" title="Expand git sidebar (Ctrl+Shift+B)" onClick={() => setGitSidebarMode("expanded")}>
        ‹
      </button>
      <button class="rail-badge" classList={{ on: props.tab === "changes" }} title="Changes" onClick={() => open("changes")}>
        {props.changedCount > 0 ? props.changedCount : "Ch"}
      </button>
      <button class="rail-badge" classList={{ on: props.tab === "commits" }} title="Commits" onClick={() => open("commits")}>
        Co
      </button>
      <button class="rail-badge" classList={{ on: props.tab === "branches" }} title="Branches" onClick={() => open("branches")}>
        Br
      </button>
      <button class="rail-badge" classList={{ on: props.tab === "review" }} title="Review" onClick={() => open("review")}>
        {props.ready ? "◆" : "Rv"}
      </button>
    </nav>
  );
}
