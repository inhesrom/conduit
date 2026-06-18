import { createSignal, For, Show } from "solid-js";
import { git } from "../../state/git-actions";
import { store } from "../../state/store";

export function CommitsPanel(props: {
  wsId: string;
  selected: () => string | null;
  onSelect: (file: string) => void;
}) {
  const commits = () => store.gitByWs[props.wsId]?.recent_commits ?? [];
  const [expanded, setExpanded] = createSignal<string | null>(null);

  const toggle = (hash: string) => {
    if (expanded() === hash) {
      setExpanded(null);
      return;
    }
    setExpanded(hash);
    if (!store.commitFilesByWs[props.wsId]?.[hash]) git.loadCommitFiles(props.wsId, hash);
  };

  const files = (hash: string) => store.commitFilesByWs[props.wsId]?.[hash] ?? [];

  return (
    <div class="gpanel">
      <Show when={commits().length > 0} fallback={<div class="gpanel-empty">No commits yet.</div>}>
        <ul class="clist">
          <For each={commits()}>
            {(c) => (
              <li class="crow">
                <button class="crow-head" onClick={() => toggle(c.hash)}>
                  <span class="crow-caret">{expanded() === c.hash ? "▾" : "▸"}</span>
                  <span class="crow-hash mono">{c.hash.slice(0, 7)}</span>
                  <span class="crow-msg">{c.message}</span>
                </button>
                <Show when={expanded() === c.hash}>
                  <div class="crow-meta mono">
                    {c.author} · {c.date}
                  </div>
                  <ul class="flist nested">
                    <For each={files(c.hash)} fallback={<li class="gpanel-empty sm">Loading…</li>}>
                      {(file) => (
                        <li class="frow" classList={{ selected: props.selected() === file }}>
                          <button
                            class="frow-path mono"
                            onClick={() => {
                              git.loadCommitFileDiff(props.wsId, c.hash, file);
                              props.onSelect(file);
                            }}
                            title={file}
                          >
                            {file}
                          </button>
                        </li>
                      )}
                    </For>
                  </ul>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}
