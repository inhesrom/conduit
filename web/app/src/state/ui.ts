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
