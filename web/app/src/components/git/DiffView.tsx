import { createEffect, createMemo, createSignal, For, on, Show } from "solid-js";
import { diffLineRange, parseUnifiedDiff, type DiffLine } from "@conduit/shared";
import { store } from "../../state/store";
import { DiffAskComposer } from "./DiffAskComposer";

const PREFIX: Record<DiffLine["kind"], string> = {
  add: "+",
  del: "-",
  context: " ",
  meta: "",
};

/** A diff selection: a contiguous run of selectable-line indices within one
 * file. `top`/`height` snapshot the focus row's geometry so the pill and
 * composer can anchor to it (offsets are relative to the scrolling body). */
type Sel = { fileIdx: number; anchor: number; focus: number; top: number; height: number };

export function DiffView(props: { wsId: string }) {
  const data = () => store.diffByWs[props.wsId];

  // Parse, then assign each non-meta line a per-file index so a click range can
  // span hunks (the hunk-header rows aren't selectable and keep index -1).
  const files = createMemo(() => {
    const d = data();
    const parsed = d ? parseUnifiedDiff(d.diff) : [];
    return parsed.map((f, fileIdx) => {
      let si = 0;
      const hunks = f.hunks.map((h) => ({
        header: h.header,
        lines: h.lines.map((l) => ({ line: l, selIndex: l.kind === "meta" ? -1 : si++ })),
      }));
      return { path: f.newPath || f.oldPath, fileIdx, hunks };
    });
  });

  const [sel, setSel] = createSignal<Sel | null>(null);
  const [composing, setComposing] = createSignal(false);
  const clear = () => {
    setSel(null);
    setComposing(false);
  };
  // Reset when the loaded diff changes (different file, or the same file
  // re-diffed) — stale indices would highlight the wrong lines.
  createEffect(on(() => data()?.diff, clear, { defer: true }));

  const bounds = () => {
    const s = sel();
    if (!s) return null;
    return { fileIdx: s.fileIdx, lo: Math.min(s.anchor, s.focus), hi: Math.max(s.anchor, s.focus) };
  };
  const isSelected = (fileIdx: number, idx: number) => {
    const b = bounds();
    return !!b && b.fileIdx === fileIdx && idx >= 0 && idx >= b.lo && idx <= b.hi;
  };

  // Click a line-number gutter to select; shift-click within the same file to
  // extend the range. The code text stays free for native text-selection.
  const onPick = (fileIdx: number, idx: number, e: MouseEvent) => {
    e.stopPropagation();
    const row = (e.currentTarget as HTMLElement).closest(".dl") as HTMLElement | null;
    const top = row?.offsetTop ?? 0;
    const height = row?.offsetHeight ?? 0;
    const cur = sel();
    if (e.shiftKey && cur && cur.fileIdx === fileIdx) setSel({ ...cur, focus: idx, top, height });
    else setSel({ fileIdx, anchor: idx, focus: idx, top, height });
    setComposing(false);
  };

  const selectedLines = (): DiffLine[] => {
    const b = bounds();
    const fa = b && files()[b.fileIdx];
    if (!b || !fa) return [];
    const out: DiffLine[] = [];
    for (const h of fa.hunks)
      for (const { line, selIndex } of h.lines)
        if (selIndex >= b.lo && selIndex <= b.hi) out.push(line);
    return out;
  };
  const selPath = () => {
    const b = bounds();
    return (b && files()[b.fileIdx]?.path) || "";
  };
  const selRange = () => diffLineRange(selectedLines()).range;

  return (
    <div class="diff">
      <Show
        when={data()}
        fallback={<div class="diff-empty">Select a file to view its diff.</div>}
      >
        <div class="diff-head mono">{data()!.file}</div>
        <div class="diff-body mono" onClick={clear}>
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
                      {({ line, selIndex }) => {
                        const pick =
                          selIndex >= 0 ? (e: MouseEvent) => onPick(f.fileIdx, selIndex, e) : undefined;
                        return (
                          <div
                            class={`dl ${line.kind}`}
                            classList={{ selected: isSelected(f.fileIdx, selIndex) }}
                          >
                            <span class="dl-gutter" classList={{ pick: selIndex >= 0 }} onClick={pick}>
                              {line.oldNo ?? ""}
                            </span>
                            <span class="dl-gutter" classList={{ pick: selIndex >= 0 }} onClick={pick}>
                              {line.newNo ?? ""}
                            </span>
                            <span class="dl-text">
                              {PREFIX[line.kind]}
                              {line.text}
                            </span>
                          </div>
                        );
                      }}
                    </For>
                  </>
                )}
              </For>
            )}
          </For>

          <Show when={sel() && !composing()}>
            <button
              class="diff-ask-pill"
              style={{ top: `${sel()!.top}px` }}
              onClick={(e) => {
                e.stopPropagation();
                setComposing(true);
              }}
            >
              ASK ⏎
            </button>
          </Show>
          <Show when={sel() && composing()}>
            <div class="diff-ask-wrap" style={{ top: `${sel()!.top + sel()!.height}px` }}>
              <DiffAskComposer
                wsId={props.wsId}
                file={selPath()}
                range={selRange()}
                lines={selectedLines()}
                onClose={clear}
              />
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
}
