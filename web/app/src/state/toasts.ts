import { createStore } from "solid-js/store";

export type ToastKind = "ok" | "error";
export interface Toast {
  id: number;
  kind: ToastKind;
  text: string;
}

const [toasts, setToasts] = createStore<{ items: Toast[] }>({ items: [] });
export { toasts };

let nextId = 1;

export function pushToast(kind: ToastKind, text: string, ttlMs = 4200): void {
  const id = nextId++;
  setToasts("items", (items) => [...items, { id, kind, text }]);
  setTimeout(() => dismissToast(id), ttlMs);
}

export function dismissToast(id: number): void {
  setToasts("items", (items) => items.filter((t) => t.id !== id));
}
