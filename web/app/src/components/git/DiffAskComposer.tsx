import { onMount } from "solid-js";
import type { DiffLine } from "@conduit/shared";
import { askAgent, askNewAgent } from "../../state/diff-ask";

/** The Diff Question composer: a TUI-style command bar anchored to a diff
 * selection. Enter spawns a fresh agent in a new tab and sends there; Alt+Enter
 * sends to the current agent (continuing its context); Shift+Enter inserts a
 * newline; Esc dismisses. */
export function DiffAskComposer(props: {
  wsId: string;
  file: string;
  range: string;
  lines: DiffLine[];
  onClose: () => void;
}) {
  let ta!: HTMLTextAreaElement;
  onMount(() => ta.focus());

  const submit = (mode: "agent" | "new") => {
    const q = ta.value.trim();
    if (!q) return;
    if (mode === "agent") askAgent(props.wsId, props.file, props.lines, q);
    else askNewAgent(props.wsId, props.file, props.lines, q);
    props.onClose();
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      props.onClose();
    } else if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit(e.altKey ? "agent" : "new");
    }
  };

  return (
    <div class="diff-ask" onClick={(e) => e.stopPropagation()}>
      <div class="diff-ask-ref mono">
        {props.file}
        <span class="diff-ask-range"> · {props.range}</span>
      </div>
      <textarea
        ref={ta}
        class="diff-ask-input mono"
        rows={3}
        placeholder="Ask about these lines…"
        onKeyDown={onKeyDown}
      />
      <div class="diff-ask-actions">
        <button class="diff-ask-btn" onClick={() => submit("agent")}>
          <span class="diff-ask-key mono">⌥⏎</span> Current agent
        </button>
        <button class="diff-ask-btn primary" onClick={() => submit("new")}>
          <span class="diff-ask-key mono">⏎</span> New agent
        </button>
      </div>
    </div>
  );
}
