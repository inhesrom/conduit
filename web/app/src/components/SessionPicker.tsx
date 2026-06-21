import { createSignal, For, Show } from "solid-js";
import { createSession, desktop, refreshSessions, selectSession, sessions } from "../state/sessions";

/** Full-screen gate shown after login when no session is attached yet. */
export function SessionPicker() {
  const [name, setName] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const create = async () => {
    if (busy() || !name().trim()) return;
    setBusy(true);
    setError(null);
    const err = await createSession(name());
    setBusy(false);
    if (err) setError(err);
    // On success createSession attaches; this picker unmounts.
  };

  return (
    <div class="login">
      <div class="login-card">
        <span class="login-mark mono">conduit</span>
        <p class="login-prompt">Attach to a session.</p>
        <Show
          when={sessions().length > 0}
          fallback={
            <p class="session-empty">
              <Show
                when={desktop()}
                fallback={
                  <>
                    No running sessions. Start one with <code>conduit tui attach &lt;name&gt;</code>.
                  </>
                }
              >
                No running sessions yet — create one below.
              </Show>
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

        <Show when={desktop()}>
          <div class="session-create">
            <input
              class="modal-input mono"
              placeholder="New session name…"
              value={name()}
              disabled={busy()}
              onInput={(e) => setName(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void create();
              }}
            />
            <button
              class="btn primary"
              disabled={busy() || !name().trim()}
              onClick={() => void create()}
            >
              {busy() ? "Creating…" : "New session"}
            </button>
          </div>
          <Show when={error()}>
            <p class="session-empty">{error()}</p>
          </Show>
        </Show>

        <button class="btn login-submit" onClick={() => void refreshSessions()}>
          Refresh
        </button>
      </div>
    </div>
  );
}
