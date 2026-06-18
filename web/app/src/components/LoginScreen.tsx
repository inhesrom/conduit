import { createSignal, Show } from "solid-js";
import { login } from "../state/session";

export function LoginScreen() {
  const [password, setPassword] = createSignal("");
  const [error, setError] = createSignal(false);
  const [busy, setBusy] = createSignal(false);

  const submit = async () => {
    if (!password() || busy()) return;
    setBusy(true);
    setError(false);
    const ok = await login(password());
    setBusy(false);
    if (!ok) {
      setError(true);
      setPassword("");
    }
  };

  return (
    <div class="login">
      <form
        class="login-card"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <span class="login-mark mono">conduit</span>
        <p class="login-prompt">Enter the password to continue.</p>
        <input
          class="modal-input mono"
          type="password"
          autofocus
          placeholder="Password"
          value={password()}
          onInput={(e) => {
            setPassword(e.currentTarget.value);
            setError(false);
          }}
        />
        <Show when={error()}>
          <p class="login-error">Incorrect password.</p>
        </Show>
        <button class="btn primary login-submit" type="submit" disabled={!password() || busy()}>
          {busy() ? "Unlocking…" : "Unlock"}
        </button>
      </form>
    </div>
  );
}
