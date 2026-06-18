import type { RepositorySummary, SshTarget, WorkspaceSummary } from "@conduit/shared";
import { client } from "../client";
import { navigate, route } from "../router";
import { confirmDialog, promptDialog } from "./dialogs";

export async function renameWorkspace(ws: WorkspaceSummary): Promise<void> {
  const name = await promptDialog({ title: "Rename workspace", initial: ws.name, confirmLabel: "Rename" });
  if (name && name.trim()) client.send({ RenameWorkspace: { id: ws.id, name: name.trim() } });
}

export async function deleteWorkspace(ws: WorkspaceSummary): Promise<void> {
  const ok = await confirmDialog({
    title: "Delete workspace?",
    body: `${ws.name} — its worktree is removed from disk.`,
    confirmLabel: "Delete",
    danger: true,
  });
  if (!ok) return;
  client.send({ RemoveWorkspace: { id: ws.id } });
  const r = route();
  if (r.name === "workspace" && r.id === ws.id) navigate({ name: "board" });
}

export async function removeRepository(repo: RepositorySummary): Promise<void> {
  const ok = await confirmDialog({
    title: "Remove repository?",
    body: `${repo.name} — unregisters only; files on disk are untouched.`,
    confirmLabel: "Remove",
    danger: true,
  });
  if (ok) client.send({ RemoveRepository: { repo_id: repo.id } });
}

export function registerRepository(opts: {
  name: string;
  path: string;
  ssh?: SshTarget | null;
  defaultAgent?: string | null;
}): void {
  client.send({
    RegisterRepository: {
      name: opts.name,
      path: opts.path,
      ssh: opts.ssh ?? null,
      default_agent: opts.defaultAgent ?? null,
      worktree_root: null,
    },
  });
}
