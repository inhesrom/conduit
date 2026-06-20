import { createSignal } from "solid-js";

/** Promise-based confirm/prompt dialogs, so call sites read linearly:
 *   if (await confirmDialog({ title: "Discard changes?" })) … */
export interface ConfirmOpts {
  title: string;
  body?: string;
  confirmLabel?: string;
  danger?: boolean;
}
export interface PromptOpts {
  title: string;
  placeholder?: string;
  initial?: string;
  confirmLabel?: string;
  multiline?: boolean;
}

export type ActiveDialog =
  | { kind: "confirm"; opts: ConfirmOpts; resolve: (ok: boolean) => void }
  | { kind: "prompt"; opts: PromptOpts; resolve: (val: string | null) => void };

const [active, setActive] = createSignal<ActiveDialog | null>(null);
export { active };

export function confirmDialog(opts: ConfirmOpts): Promise<boolean> {
  return new Promise((resolve) => setActive({ kind: "confirm", opts, resolve }));
}

export function promptDialog(opts: PromptOpts): Promise<string | null> {
  return new Promise((resolve) => setActive({ kind: "prompt", opts, resolve }));
}

export function closeDialog(value: boolean | string | null): void {
  const a = active();
  if (!a) return;
  setActive(null);
  (a.resolve as (v: boolean | string | null) => void)(value);
}
