import { createSignal } from "solid-js";

/** App-level modals (distinct from the promise-based confirm/prompt dialogs). */
export type AppModal = { kind: "create"; repoId: string } | { kind: "addRepo" };

const [appModal, setAppModal] = createSignal<AppModal | null>(null);
export { appModal };

export function openCreateWorkspace(repoId: string): void {
  setAppModal({ kind: "create", repoId });
}
export function openAddRepository(): void {
  setAppModal({ kind: "addRepo" });
}
export function closeAppModal(): void {
  setAppModal(null);
}
