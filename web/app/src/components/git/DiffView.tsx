import { createMemo, For, Show } from "solid-js";
import { parseUnifiedDiff, type DiffLine } from "@conduit/shared";
import { store } from "../../state/store";

const PREFIX: Record<DiffLine["kind"], string> = {
  add: "+",
  del: "-",
  context: " ",
  meta: "",
};

export function DiffView(props: { wsId: string }) {
  const data = () => store.diffByWs[props.wsId];
  const files = createMemo(() => {
    const d = data();
    return d ? parseUnifiedDiff(d.diff) : [];
  });

  return (
    <div class="diff">
      <Show
        when={data()}
        fallback={<div class="diff-empty">Select a file to view its diff.</div>}
      >
        <div class="diff-head mono">{data()!.file}</div>
        <div class="diff-body mono">
          <For each={files()}>
            {(f) => (
              <For each={f.hunks}>
                {(h) => (
                  <>
                    <Show when={h.header}>
                      <div class="dl hunk">
                        <span class="dl-gutter" />
                        <span class="dl-gutter" />
                        <span class="dl-text">{h.header}</span>
                      </div>
                    </Show>
                    <For each={h.lines}>
                      {(l) => (
                        <div class={`dl ${l.kind}`}>
                          <span class="dl-gutter">{l.oldNo ?? ""}</span>
                          <span class="dl-gutter">{l.newNo ?? ""}</span>
                          <span class="dl-text">
                            {PREFIX[l.kind]}
                            {l.text}
                          </span>
                        </div>
                      )}
                    </For>
                  </>
                )}
              </For>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
