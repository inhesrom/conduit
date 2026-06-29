import { createSignal, For } from "solid-js";
import { promptDialog } from "../state/dialogs";
import { closeAppModal } from "../state/modals";
import { addAgent, removeAgent, settings, updateAgent, updateSettings } from "../state/settings";
import { MONO_FONTS, SANS_FONTS } from "../state/fonts";
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
        <span class="eyebrow">Display</span>
      </div>
      <label class="field">
        <span class="field-label">Terminal font size · {settings.termFontSize}px</span>
        <input
          type="range"
          min="10"
          max="28"
          step="1"
          style={{ width: "100%", "accent-color": "var(--ink)" }}
          value={settings.termFontSize}
          onInput={(e) => updateSettings({ termFontSize: parseInt(e.currentTarget.value, 10) })}
        />
      </label>
      <label class="field">
        <span class="field-label">Interface scale · {Math.round(settings.uiScale * 100)}%</span>
        <input
          type="range"
          min="70"
          max="130"
          step="5"
          style={{ width: "100%", "accent-color": "var(--ink)" }}
          value={Math.round(settings.uiScale * 100)}
          onInput={(e) => updateSettings({ uiScale: parseInt(e.currentTarget.value, 10) / 100 })}
        />
      </label>
      <label class="toggle-row">
        <input
          type="checkbox"
          checked={settings.roundedCorners}
          onChange={(e) => updateSettings({ roundedCorners: e.currentTarget.checked })}
        />
        <span>
          <span class="toggle-title">Rounded corners</span>
          <span class="toggle-hint">Soften card, button and menu corners. Off restores the 8-bit hard edges.</span>
        </span>
      </label>
      <label class="field">
        <span class="field-label">Git panel layout</span>
        <div class="seg">
          <button
            class="seg-btn"
            classList={{ active: settings.gitLayout === "sidebar" }}
            onClick={() => updateSettings({ gitLayout: "sidebar" })}
          >
            Sidebar
          </button>
          <button
            class="seg-btn"
            classList={{ active: settings.gitLayout === "bottom" }}
            onClick={() => updateSettings({ gitLayout: "bottom" })}
          >
            Bottom
          </button>
        </div>
      </label>

      <div class="settings-section">
        <span class="eyebrow">Fonts</span>
      </div>
      <label class="field">
        <span class="field-label">Interface font</span>
        <select
          class="modal-input mono"
          value={settings.uiFont}
          onChange={(e) => updateSettings({ uiFont: e.currentTarget.value })}
        >
          <For each={SANS_FONTS}>{(f) => <option value={f.id}>{f.label}</option>}</For>
        </select>
      </label>
      <label class="field">
        <span class="field-label">Terminal font</span>
        <select
          class="modal-input mono"
          value={settings.terminalFont}
          onChange={(e) => updateSettings({ terminalFont: e.currentTarget.value })}
        >
          <For each={MONO_FONTS}>{(f) => <option value={f.id}>{f.label}</option>}</For>
        </select>
      </label>
      <label class="field">
        <span class="field-label">Git diff font</span>
        <select
          class="modal-input mono"
          value={settings.diffFont}
          onChange={(e) => updateSettings({ diffFont: e.currentTarget.value })}
        >
          <For each={MONO_FONTS}>{(f) => <option value={f.id}>{f.label}</option>}</For>
        </select>
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
