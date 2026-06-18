import { For, Show } from "solid-js";
import { refreshSessions, selectSession, sessions } from "../state/sessions";

/** Full-screen gate shown after login when no session is attached yet. */
export function SessionPicker() {
  return (
    <div class="login">
      <div class="login-card">
        <span class="login-mark mono">conduit</span>
        <p class="login-prompt">Attach to a running session.</p>
        <Show
          when={sessions().length > 0}
          fallback={
            <p class="session-empty">
              No running sessions. Start one with <code>conduit -s &lt;name&gt;</code>.
            </p>
          }
        >
          <ul class="session-list">
            <For each={sessions()}>
              {(s) => (
                <li>
                  <button class="session-opt mono" onClick={() => selectSession(s)}>
                    {s}
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <button class="btn login-submit" onClick={() => void refreshSessions()}>
          Refresh
        </button>
      </div>
    </div>
  );
}
