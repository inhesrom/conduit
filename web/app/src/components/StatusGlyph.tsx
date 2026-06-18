import { Match, Switch } from "solid-js";
import type { WorkspaceSummary } from "@conduit/shared";

/** The single leading state glyph for a workspace, by display priority:
 * error → needs-input → agent working → notice → idle. Ready-for-review is
 * orthogonal and rendered separately as a ◆ marker. */
export function StatusGlyph(props: { ws: WorkspaceSummary }) {
  const a = () => props.ws.attention;
  return (
    <Switch fallback={<span class="glyph idle">•</span>}>
      <Match when={a() === "Error"}>
        <span class="glyph error" title="Error">
          ✕
        </span>
      </Match>
      <Match when={a() === "NeedsInput"}>
        <span class="glyph needs" title="Needs input">
          !
        </span>
      </Match>
      <Match when={props.ws.agent_active}>
        <span class="glyph active spinner" title="Agent working" />
      </Match>
      <Match when={a() === "Notice"}>
        <span class="glyph notice" title="Notice">
          •
        </span>
      </Match>
    </Switch>
  );
}
