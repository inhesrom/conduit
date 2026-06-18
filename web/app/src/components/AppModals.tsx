import { Show } from "solid-js";
import { appModal } from "../state/modals";
import { AddRepositoryModal } from "./AddRepositoryModal";
import { CreateWorkspaceModal } from "./CreateWorkspaceModal";

/** Mounted once at the app root; renders the active app-level modal. */
export function AppModals() {
  return (
    <Show when={appModal()} keyed>
      {(m) => (m.kind === "create" ? <CreateWorkspaceModal repoId={m.repoId} /> : <AddRepositoryModal />)}
    </Show>
  );
}
