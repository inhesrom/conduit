import { createSignal, Show } from "solid-js";
import { active, type ConfirmOpts, type PromptOpts, closeDialog } from "../state/dialogs";
import { Modal } from "./Modal";

function ConfirmCard(props: { opts: ConfirmOpts }) {
  return (
    <Modal onClose={() => closeDialog(false)}>
      <h2 class="modal-title">{props.opts.title}</h2>
      <Show when={props.opts.body}>
        <p class="modal-body">{props.opts.body}</p>
      </Show>
      <div class="modal-actions">
        <button class="btn" onClick={() => closeDialog(false)}>
          Cancel
        </button>
        <button
          class="btn"
          classList={{ danger: props.opts.danger }}
          onClick={() => closeDialog(true)}
        >
          {props.opts.confirmLabel ?? "Confirm"}
        </button>
      </div>
    </Modal>
  );
}

function PromptCard(props: { opts: PromptOpts }) {
  const [value, setValue] = createSignal(props.opts.initial ?? "");
  const submit = () => closeDialog(value());
  return (
    <Modal onClose={() => closeDialog(null)}>
      <h2 class="modal-title">{props.opts.title}</h2>
      <Show
        when={props.opts.multiline}
        fallback={
          <input
            class="modal-input mono"
            autofocus
            placeholder={props.opts.placeholder}
            value={value()}
            onInput={(e) => setValue(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submit();
            }}
          />
        }
      >
        <textarea
          class="modal-input mono"
          rows={4}
          autofocus
          placeholder={props.opts.placeholder}
          value={value()}
          onInput={(e) => setValue(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) submit();
          }}
        />
      </Show>
      <div class="modal-actions">
        <button class="btn" onClick={() => closeDialog(null)}>
          Cancel
        </button>
        <button class="btn primary" onClick={submit}>
          {props.opts.confirmLabel ?? "OK"}
        </button>
      </div>
    </Modal>
  );
}

/** Mounted once at the app root; renders whichever dialog is active. */
export function Dialogs() {
  return (
    <Show when={active()} keyed>
      {(a) => (a.kind === "confirm" ? <ConfirmCard opts={a.opts} /> : <PromptCard opts={a.opts} />)}
    </Show>
  );
}
