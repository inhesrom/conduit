import { createSignal, For } from "solid-js";
import { promptDialog } from "../state/dialogs";
import { closeAppModal } from "../state/modals";
import { addAgent, removeAgent, settings, updateAgent, updateSettings } from "../state/settings";
import { Modal } from "./Modal";

export function SettingsModal() {
  const addProfile = async () => {
    const name = await promptDialog({ title: "New agent", placeholder: "name (e.g. aider)", confirmLabel: "Add" });
    if (!name || !name.trim()) return;
    const command = await promptDialog({ title: `Launch command for ${name.trim()}`, placeholder: "command", confirmLabel: "Add", initial: name.trim() });
    if (!command || !command.trim()) return;
    addAgent({ name: name.trim(), command: command.trim(), yoloFlags: [], continueFlags: [] });
  };

  return (
    <Modal onClose={closeAppModal} width={560}>
      <h2 class="modal-title">Settings</h2>

      <label class="field">
        <span class="field-label">Default agent</span>
        <select
          class="modal-input mono"
          value={settings.defaultAgent}
          onChange={(e) => updateSettings({ defaultAgent: e.currentTarget.value })}
        >
          <For each={settings.agents}>{(a) => <option value={a.name}>{a.name}</option>}</For>
        </select>
      </label>

      <label class="toggle-row">
        <input
          type="checkbox"
          checked={settings.yoloMode}
          onChange={(e) => updateSettings({ yoloMode: e.currentTarget.checked })}
        />
        <span>
          <span class="toggle-title">Yolo mode</span>
          <span class="toggle-hint">Launch agents with their skip-permission flags.</span>
        </span>
      </label>

      <label class="toggle-row">
        <input
          type="checkbox"
          checked={settings.attentionNotifications}
          onChange={(e) => updateSettings({ attentionNotifications: e.currentTarget.checked })}
        />
        <span>
          <span class="toggle-title">Attention notifications</span>
          <span class="toggle-hint">Surface workspaces that need input.</span>
        </span>
      </label>

      <div class="settings-section">
        <span class="eyebrow">Agents</span>
        <button class="btn xs" onClick={addProfile}>
          Add agent
        </button>
      </div>
      <ul class="agent-list">
        <For each={settings.agents}>
          {(a) => (
            <li class="agent-row">
              <span class="agent-name mono">{a.name}</span>
              <input
                class="modal-input mono agent-cmd"
                value={a.command}
                onInput={(e) => updateAgent(a.name, { command: e.currentTarget.value })}
              />
              <input
                class="modal-input mono agent-flags"
                placeholder="yolo flags"
                value={a.yoloFlags.join(" ")}
                onInput={(e) => updateAgent(a.name, { yoloFlags: e.currentTarget.value.split(/\s+/).filter(Boolean) })}
              />
              <button
                class="frow-discard"
                title="Remove agent"
                disabled={settings.agents.length <= 1}
                onClick={() => removeAgent(a.name)}
              >
                ⌫
              </button>
            </li>
          )}
        </For>
      </ul>

      <div class="modal-actions">
        <button class="btn primary" onClick={closeAppModal}>
          Done
        </button>
      </div>
    </Modal>
  );
}
