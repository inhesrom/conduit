import { createEffect, createMemo, For, Show } from "solid-js";
import type { WorkspaceSummary } from "@conduit/shared";
import { makeFlip } from "../lib/flip";
import {
  BAND_LABEL,
  BAND_ORDER,
  type Band,
  bandOf,
  byRepoThenName,
  countsOf,
} from "../state/selectors";
import { store } from "../state/store";
import { WorkspaceRow } from "../components/WorkspaceRow";

type HeaderItem = { kind: "head"; band: Band };
type Item = HeaderItem | WorkspaceSummary;

// Stable references so the keyed <For> reuses DOM nodes across reorders —
// that node reuse is what lets FLIP glide a row between bands.
const HEADERS: Record<Band, HeaderItem> = {
  needs: { kind: "head", band: "needs" },
  working: { kind: "head", band: "working" },
  ready: { kind: "head", band: "ready" },
  idle: { kind: "head", band: "idle" },
};

function isHead(i: Item): i is HeaderItem {
  return (i as HeaderItem).kind === "head";
}

export function BoardScreen() {
  const flip = makeFlip();

  const grouped = createMemo(() => {
    const g: Record<Band, WorkspaceSummary[]> = { needs: [], working: [], ready: [], idle: [] };
    for (const w of store.workspaces) g[bandOf(w)].push(w);
    for (const band of BAND_ORDER) g[band].sort(byRepoThenName);
    return g;
  });

  const ordered = createMemo<Item[]>(() => {
    const g = grouped();
    const out: Item[] = [];
    for (const band of BAND_ORDER) {
      if (g[band].length === 0) continue;
      out.push(HEADERS[band]);
      out.push(...g[band]);
    }
    return out;
  });

  // After every reorder, glide moved rows from their old positions.
  createEffect(() => {
    ordered();
    flip.play();
  });

  return (
    <div class="board">
      <Show when={store.workspaces.length > 0} fallback={<EmptyBoard />}>
        <header class="board-head">
          <BoardSummary />
        </header>
        <div class="board-list">
          <For each={ordered()}>
            {(item) =>
              isHead(item) ? (
                <div class="band-head">
                  <span class="eyebrow">{BAND_LABEL[item.band]}</span>
                  <span class={`band-count ${item.band}`}>{grouped()[item.band].length}</span>
                </div>
              ) : (
                <WorkspaceRow ws={item} ref={flip.register(item.id)} />
              )
            }
          </For>
        </div>
      </Show>
    </div>
  );
}

function BoardSummary() {
  const c = createMemo(() => countsOf(store.workspaces));
  return (
    <div class="board-summary">
      <Show when={c().needs > 0} fallback={<span class="board-allclear">All clear — nothing needs you.</span>}>
        <span class="board-stat needs">
          <b>{c().needs}</b> {c().needs === 1 ? "needs you" : "need you"}
        </span>
      </Show>
      <Show when={c().working > 0}>
        <span class="board-stat working">
          <b>{c().working}</b> working
        </span>
      </Show>
      <Show when={c().ready > 0}>
        <span class="board-stat ready">
          <b>{c().ready}</b> ready
        </span>
      </Show>
    </div>
  );
}

function EmptyBoard() {
  return (
    <div class="empty">
      <Show
        when={store.repositories.length > 0}
        fallback={<p>Add a repository to start, then create a workspace.</p>}
      >
        <p>No workspaces yet — create one to put an agent to work.</p>
      </Show>
    </div>
  );
}
