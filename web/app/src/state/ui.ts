import { createSignal } from "solid-js";

/** Sidebar presentation, mirroring the TUI's three modes. Persisted so the
 * choice survives reloads. */
export type SidebarMode = "expanded" | "rail" | "hidden";

const KEY_MODE = "conduit.sidebar.mode";
const KEY_WIDTH = "conduit.sidebar.width";
const MIN_WIDTH = 180;
const MAX_WIDTH = 480;
const DEFAULT_WIDTH = 264;

function readMode(): SidebarMode {
  try {
    const v = localStorage.getItem(KEY_MODE);
    if (v === "expanded" || v === "rail" || v === "hidden") return v;
  } catch {
    // ignore
  }
  return "expanded";
}

function clampWidth(w: number): number {
  return Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, Math.round(w)));
}

function readWidth(): number {
  try {
    const v = parseFloat(localStorage.getItem(KEY_WIDTH) ?? "");
    return Number.isFinite(v) ? clampWidth(v) : DEFAULT_WIDTH;
  } catch {
    return DEFAULT_WIDTH;
  }
}

const [sidebarMode, setSidebarModeSig] = createSignal<SidebarMode>(readMode());
const [sidebarWidth, setSidebarWidthSig] = createSignal(readWidth());
export { sidebarMode, sidebarWidth };

export function setSidebarMode(m: SidebarMode): void {
  setSidebarModeSig(m);
  try {
    localStorage.setItem(KEY_MODE, m);
  } catch {
    // ignore
  }
}

export function setSidebarWidth(w: number): void {
  const c = clampWidth(w);
  setSidebarWidthSig(c);
  try {
    localStorage.setItem(KEY_WIDTH, String(c));
  } catch {
    // ignore
  }
}

export function cycleSidebar(): void {
  setSidebarMode(
    sidebarMode() === "expanded" ? "rail" : sidebarMode() === "rail" ? "hidden" : "expanded",
  );
}

/** When on, the sidebar shows only workspaces ready for review. */
const [reviewFilter, setReviewFilter] = createSignal(false);
export { reviewFilter, setReviewFilter };

/** Right-hand git sidebar, mirroring the left sidebar's three modes and
 * persistence. */
export type GitSidebarMode = "expanded" | "rail" | "hidden";

const KEY_GIT_MODE = "conduit.gitsidebar.mode";
const KEY_GIT_WIDTH = "conduit.gitsidebar.width";
const GIT_MIN_WIDTH = 240;
const GIT_MAX_WIDTH = 480;
const GIT_DEFAULT_WIDTH = 320;

function readGitMode(): GitSidebarMode {
  try {
    const v = localStorage.getItem(KEY_GIT_MODE);
    if (v === "expanded" || v === "rail" || v === "hidden") return v;
  } catch {
    // ignore
  }
  return "expanded";
}

function clampGitWidth(w: number): number {
  return Math.min(GIT_MAX_WIDTH, Math.max(GIT_MIN_WIDTH, Math.round(w)));
}

function readGitWidth(): number {
  try {
    const v = parseFloat(localStorage.getItem(KEY_GIT_WIDTH) ?? "");
    return Number.isFinite(v) ? clampGitWidth(v) : GIT_DEFAULT_WIDTH;
  } catch {
    return GIT_DEFAULT_WIDTH;
  }
}

const [gitSidebarMode, setGitSidebarModeSig] = createSignal<GitSidebarMode>(readGitMode());
const [gitSidebarWidth, setGitSidebarWidthSig] = createSignal(readGitWidth());
export { gitSidebarMode, gitSidebarWidth, GIT_DEFAULT_WIDTH };

export function setGitSidebarMode(m: GitSidebarMode): void {
  setGitSidebarModeSig(m);
  try {
    localStorage.setItem(KEY_GIT_MODE, m);
  } catch {
    // ignore
  }
}

export function setGitSidebarWidth(w: number): void {
  const c = clampGitWidth(w);
  setGitSidebarWidthSig(c);
  try {
    localStorage.setItem(KEY_GIT_WIDTH, String(c));
  } catch {
    // ignore
  }
}

export function cycleGitSidebar(): void {
  setGitSidebarMode(
    gitSidebarMode() === "expanded" ? "rail" : gitSidebarMode() === "rail" ? "hidden" : "expanded",
  );
}

/** Ephemeral per-workspace selected diff file. The single source of truth for
 * the git sidebar's row highlight and the workspace main area's terminal↔diff
 * swap. Not persisted — selection resets on reload. */
const [selectedFileByWs, setSelectedFileByWs] = createSignal<Record<string, string | null>>({});

export function selectedFile(wsId: string): string | null {
  return selectedFileByWs()[wsId] ?? null;
}

export function setSelectedFile(wsId: string, file: string | null): void {
  setSelectedFileByWs((m) => ({ ...m, [wsId]: file }));
}
