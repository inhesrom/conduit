import { Show } from "solid-js";
import { store } from "../state/store";

/** A thin bar shown only while not connected. Speaks plainly — what's
 * happening, not an apology. */
export function ConnectionBanner() {
  return (
    <Show when={store.conn !== "open"}>
      <div class="conn-banner" classList={{ connecting: store.conn === "connecting" }}>
        <span class="conn-banner-dot" />
        {store.conn === "connecting" ? "Connecting to conduit…" : "Disconnected — retrying…"}
      </div>
    </Show>
  );
}
