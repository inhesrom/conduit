import { Show } from "solid-js";
import type { WorkspaceSummary } from "@conduit/shared";
import { hrefFor, navigate, route } from "../router";
import { repoName } from "../state/selectors";
import { StatusGlyph } from "./StatusGlyph";

/** A single workspace on the Board. The root <a> takes a ref so the Board's
 * FLIP can glide it between bands. */
export function WorkspaceRow(props: {
  ws: WorkspaceSummary;
  ref?: (el: HTMLElement) => void;
}) {
  const w = () => props.ws;
  const active = () => {
    const r = route();
    return r.name === "workspace" && r.id === w().id;
  };

  const counters = () => {
    const parts: string[] = [];
    if (w().ahead) parts.push(`↑${w().ahead}`);
    if (w().behind) parts.push(`↓${w().behind}`);
    return parts.join(" ");
  };

  // The branch usually equals the workspace name — only worth showing when it
  // diverges (e.g. an existing branch checked out under a different name).
  const branch = () => (w().branch && w().branch !== w().name ? w().branch : "");

  return (
    <a
      ref={props.ref}
      class="brow"
      classList={{ active: active() }}
      href={hrefFor({ name: "workspace", id: w().id })}
      onClick={(e) => {
        e.preventDefault();
        navigate({ name: "workspace", id: w().id });
      }}
    >
      <StatusGlyph ws={w()} />
      <span class="brow-name">{w().name}</span>
      <span class="brow-repo mono">{repoName(w())}</span>
      <Show when={branch()}>
        <span class="brow-branch mono" title="branch">
          {branch()}
        </span>
      </Show>
      <span class="brow-spacer" />
      <Show when={counters()}>
        <span class="brow-counters mono">{counters()}</span>
      </Show>
      <Show when={w().dirty_files > 0}>
        <span class="brow-dirty mono" title={`${w().dirty_files} changed`}>
          ±{w().dirty_files}
        </span>
      </Show>
      <Show when={w().ready_for_review}>
        <span class="brow-review" title="Ready for review">
          ◆
        </span>
      </Show>
    </a>
  );
}
