import type { RepositorySummary, WorkspaceSummary } from "@conduit/shared";
import { store } from "./store";

/** Attention bands for the Board, in display priority. A workspace lands in
 * exactly one band; ready-for-review still shows its ◆ marker regardless. */
export type Band = "needs" | "working" | "ready" | "idle";

export const BAND_ORDER: Band[] = ["needs", "working", "ready", "idle"];

export const BAND_LABEL: Record<Band, string> = {
  needs: "Needs you",
  working: "Working",
  ready: "Ready for review",
  idle: "Idle",
};

export function bandOf(w: WorkspaceSummary): Band {
  if (w.attention === "NeedsInput" || w.attention === "Error") return "needs";
  if (w.agent_active) return "working";
  if (w.ready_for_review) return "ready";
  return "idle";
}

export function repoOf(w: WorkspaceSummary): RepositorySummary | undefined {
  return store.repositories.find((r) => r.id === w.repository_id);
}

export function repoName(w: WorkspaceSummary): string {
  return repoOf(w)?.name ?? "";
}

/** Stable ordering inside a band — by repo then name, never by activity, so
 * rows only move when their band actually changes (keeps the reflow honest). */
export function byRepoThenName(a: WorkspaceSummary, b: WorkspaceSummary): number {
  return repoName(a).localeCompare(repoName(b)) || a.name.localeCompare(b.name);
}

export interface BoardCounts {
  needs: number;
  working: number;
  ready: number;
  total: number;
}

export function countsOf(workspaces: WorkspaceSummary[]): BoardCounts {
  const c: BoardCounts = { needs: 0, working: 0, ready: 0, total: workspaces.length };
  for (const w of workspaces) {
    const band = bandOf(w);
    if (band === "needs") c.needs++;
    else if (band === "working") c.working++;
    if (w.ready_for_review) c.ready++;
  }
  return c;
}
