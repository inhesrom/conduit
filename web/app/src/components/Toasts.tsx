import { For } from "solid-js";
import { Portal } from "solid-js/web";
import { dismissToast, toasts } from "../state/toasts";

/** Transient git-action / error feedback, bottom-right. */
export function Toasts() {
  return (
    <Portal>
      <div class="toasts">
        <For each={toasts.items}>
          {(t) => (
            <div class="toast" classList={{ error: t.kind === "error" }} onClick={() => dismissToast(t.id)}>
              {t.text}
            </div>
          )}
        </For>
      </div>
    </Portal>
  );
}
