/** Folder browsing against the daemon's read-only listing endpoint. A remote
 * web client can't touch the filesystem itself, so both the "add repository"
 * and "add existing folder" pickers go through `/api/fs/list`. */

export interface FsEntry {
  name: string;
  path: string;
  /** True when the folder contains a `.git` entry — a clone or a worktree. */
  is_repo: boolean;
}

export interface Listing {
  path: string;
  parent: string | null;
  entries: FsEntry[];
}

export async function fetchListing(path: string | undefined): Promise<Listing> {
  const url = "/api/fs/list" + (path ? `?path=${encodeURIComponent(path)}` : "");
  const res = await fetch(url);
  if (!res.ok) throw new Error(`Couldn't read that folder.`);
  return res.json();
}

export function basename(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? path;
}
