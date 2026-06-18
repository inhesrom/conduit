import { createMemo, createSignal, For, Show } from "solid-js";
import type { RepositorySummary, WorkspaceSummary } from "@conduit/shared";
import { hrefFor, navigate, route } from "../router";
import { byRepoThenName } from "../state/selectors";
import { store } from "../state/store";
import {
  cycleSidebar,
  reviewFilter,
  setReviewFilter,
  setSidebarMode,
  sidebarMode,
} from "../state/ui";
import { StatusGlyph } from "./StatusGlyph";

function WsItem(props: { ws: WorkspaceSummary }) {
  const active = () => {
    const r = route();
    return r.name === "workspace" && r.id === props.ws.id;
  };
  return (
    <a
      class="ws-item"
      classList={{ active: active() }}
      href={hrefFor({ name: "workspace", id: props.ws.id })}
      onClick={(e) => {
        e.preventDefault();
        navigate({ name: "workspace", id: props.ws.id });
      }}
    >
      <StatusGlyph ws={props.ws} />
      <span class="ws-item-name">{props.ws.name}</span>
      <Show when={props.ws.ready_for_review}>
        <span class="ws-item-review" title="Ready for review">
          ◆
        </span>
      </Show>
    </a>
  );
}

function RepoGroup(props: { name: string; reviewCount: number; workspaces: WorkspaceSummary[] }) {
  const [collapsed, setCollapsed] = createSignal(false);
  return (
    <section class="repo-group">
      <button class="repo-head" onClick={() => setCollapsed((c) => !c)}>
        <span class="repo-caret">{collapsed() ? "▸" : "▾"}</span>
        <span class="repo-name">{props.name}</span>
        <span class="repo-count">{props.workspaces.length}</span>
        <Show when={props.reviewCount > 0}>
          <span class="repo-review">◆{props.reviewCount}</span>
        </Show>
      </button>
      <Show when={!collapsed()}>
        <For each={props.workspaces}>{(w) => <WsItem ws={w} />}</For>
      </Show>
    </section>
  );
}

export function Sidebar() {
  const visibleWs = createMemo(() =>
    reviewFilter() ? store.workspaces.filter((w) => w.ready_for_review) : store.workspaces,
  );

  const groups = createMemo(() =>
    store.repositories
      .map((repo: RepositorySummary) => ({
        repo,
        items: visibleWs()
          .filter((w) => w.repository_id === repo.id)
          .sort(byRepoThenName),
      }))
      .filter((g) => !reviewFilter() || g.items.length > 0),
  );

  const ungrouped = createMemo(() =>
    visibleWs()
      .filter((w) => !w.repository_id || !store.repositories.some((r) => r.id === w.repository_id))
      .sort(byRepoThenName),
  );

  return (
    <Show when={sidebarMode() === "expanded"} fallback={<RailSidebar />}>
      <nav class="sidebar">
        <div class="sidebar-head">
          <span class="eyebrow">Workspaces</span>
          <div class="sidebar-controls">
            <button
              class="icon-btn"
              classList={{ on: reviewFilter() }}
              title="Show only ready for review"
              onClick={() => setReviewFilter((v) => !v)}
            >
              ◆
            </button>
            <button class="icon-btn" title="Collapse sidebar (Ctrl+B)" onClick={cycleSidebar}>
              ‹
            </button>
          </div>
        </div>
        <div class="sidebar-scroll">
          <For each={groups()}>
            {(g) => (
              <RepoGroup
                name={g.repo.name}
                reviewCount={g.items.filter((w) => w.ready_for_review).length}
                workspaces={g.items}
              />
            )}
          </For>
          <Show when={ungrouped().length > 0}>
            <RepoGroup
              name="Ungrouped"
              reviewCount={ungrouped().filter((w) => w.ready_for_review).length}
              workspaces={ungrouped()}
            />
          </Show>
          <Show when={store.repositories.length === 0 && store.workspaces.length === 0}>
            <p class="sidebar-empty">No repositories yet.</p>
          </Show>
        </div>
      </nav>
    </Show>
  );
}

function RailSidebar() {
  return (
    <nav class="sidebar rail">
      <button class="icon-btn" title="Expand sidebar (Ctrl+B)" onClick={() => setSidebarMode("expanded")}>
        ›
      </button>
      <For each={store.repositories}>
        {(r) => (
          <button class="rail-badge" title={r.name} onClick={() => setSidebarMode("expanded")}>
            {r.name.slice(0, 2)}
          </button>
        )}
      </For>
    </nav>
  );
}
