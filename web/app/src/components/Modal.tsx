import { onCleanup, onMount, type JSX } from "solid-js";
import { Portal } from "solid-js/web";

/** Backdrop + centered card. Escape and backdrop clicks call onClose. */
export function Modal(props: { onClose: () => void; children: JSX.Element; width?: number }) {
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      props.onClose();
    }
  };
  onMount(() => document.addEventListener("keydown", onKey, true));
  onCleanup(() => document.removeEventListener("keydown", onKey, true));

  return (
    <Portal>
      <div
        class="modal-backdrop"
        onClick={(e) => {
          if (e.target === e.currentTarget) props.onClose();
        }}
      >
        <div class="modal-card" role="dialog" aria-modal="true" style={{ width: `${props.width ?? 440}px` }}>
          {props.children}
        </div>
      </div>
    </Portal>
  );
}
