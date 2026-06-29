/** Diff Questions — compose a prompt from a selected range of diff lines and
 * deliver it to an agent. The default targets the workspace's existing agent
 * terminal (continuing its context); the secondary path spawns a fresh agent
 * in a new tab (no shared history). Both inject the prompt as keystrokes via
 * the same SendTerminalInput path the initial workspace task uses. */

import { buildDiffQuestion, promptInput, textToB64, type DiffLine } from "@conduit/shared";
import { client } from "../client";
import { agentCmdFor } from "./settings";
import { setStore, store } from "./store";
import { createShell } from "./tabs";
import { focusTerminalTab, markFreshTab } from "./ui";

const ws = (wsId: string) => store.workspaces.find((w) => w.id === wsId);

/** Send a Diff Question to the workspace's existing agent terminal. If the
 * agent isn't running, stash it as the pending prompt so it's delivered when
 * the agent (re)starts; either way switch the view to the agent tab. */
export function askAgent(wsId: string, file: string, lines: DiffLine[], question: string): void {
  const prompt = buildDiffQuestion(file, lines, question);
  const running = ws(wsId)?.agent_running ?? false;
  if (running) {
    client.send({
      SendTerminalInput: {
        id: wsId,
        kind: "Agent",
        tab_id: "agent",
        data_b64: textToB64(promptInput(prompt)),
      },
    });
  } else {
    setStore("pendingPrompt", wsId, prompt);
  }
  focusTerminalTab(wsId, "agent");
}

/** Spawn a fresh agent in a new shell tab (the workspace's configured agent
 * command) and deliver the Diff Question to it once it starts. */
export function askNewAgent(wsId: string, file: string, lines: DiffLine[], question: string): void {
  const prompt = buildDiffQuestion(file, lines, question);
  const tab = createShell(wsId, { title: "agent ↗", cmd: agentCmdFor(ws(wsId)?.agent ?? null) });
  setStore("pendingTabPrompt", `${wsId}/${tab.id}`, prompt);
  markFreshTab(wsId, tab.id);
  focusTerminalTab(wsId, tab.id);
}
