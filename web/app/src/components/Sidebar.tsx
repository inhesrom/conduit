import { createMemo, createSignal, For, Show } from "solid-js";
import type { RepositorySummary, WorkspaceSummary } from "@conduit/shared";
import { hrefFor, navigate, route } from "../router";
import { removeRepository } from "../state/manage";
import { openAddRepository, openCreateWorkspace } from "../state/modals";
import { byRepoThenName } from "../state/selectors";
import { store } from "../state/store";
import {
  cycleSidebar,
  reviewFilter,
  setReviewFilter,
  setSidebarMode,
  setSidebarWidth,
  sidebarMode,
  sidebarWidth,
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

function RepoGroup(props: {
  name: string;
  repo?: RepositorySummary;
  reviewCount: number;
  workspaces: WorkspaceSummary[];
}) {
  const [collapsed, setCollapsed] = createSignal(false);
  return (
    <section class="repo-group">
      <div class="repo-head">
        <button class="repo-head-main" onClick={() => setCollapsed((c) => !c)}>
          <span class="repo-caret">{collapsed() ? "▸" : "▾"}</span>
          <span class="repo-name">{props.name}</span>
          <span class="repo-count">{props.workspaces.length}</span>
          <Show when={props.reviewCount > 0}>
            <span class="repo-review">◆{props.reviewCount}</span>
          </Show>
        </button>
        <Show when={props.repo}>
          <div class="repo-actions">
            <button class="icon-btn sm" title="New workspace" onClick={() => openCreateWorkspace(props.repo!.id)}>
              +
            </button>
            <button class="icon-btn sm" title="Remove repository" onClick={() => removeRepository(props.repo!)}>
              ×
            </button>
          </div>
        </Show>
      </div>
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

  let navEl: HTMLElement | undefined;
  const onResizeStart = (e: PointerEvent) => {
    e.preventDefault();
    const move = (ev: PointerEvent) => {
      const left = navEl ? navEl.getBoundingClientRect().left : 0;
      setSidebarWidth(ev.clientX - left);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  return (
    <Show when={sidebarMode() === "expanded"} fallback={<RailSidebar />}>
      <nav class="sidebar" ref={navEl} style={{ width: `${sidebarWidth()}px` }}>
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
            <button class="icon-btn" title="Add repository" onClick={openAddRepository}>
              +
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
                repo={g.repo}
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
        <div
          class="sidebar-resize"
          title="Drag to resize"
          onPointerDown={onResizeStart}
          onDblClick={() => setSidebarWidth(264)}
        />
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
