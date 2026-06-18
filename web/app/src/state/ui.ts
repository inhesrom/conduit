import { createSignal } from "solid-js";

/** Sidebar presentation, mirroring the TUI's three modes. */
export type SidebarMode = "expanded" | "rail" | "hidden";

const [sidebarMode, setSidebarMode] = createSignal<SidebarMode>("expanded");
export { sidebarMode, setSidebarMode };

export function cycleSidebar(): void {
  setSidebarMode((m) => (m === "expanded" ? "rail" : m === "rail" ? "hidden" : "expanded"));
}

/** When on, the sidebar shows only workspaces ready for review. */
const [reviewFilter, setReviewFilter] = createSignal(false);
export { reviewFilter, setReviewFilter };
