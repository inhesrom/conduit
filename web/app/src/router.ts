import { createSignal } from "solid-js";

/** Client-side hash routing. The web client never sends SetRoute — that would
 * steer the TUI's screen; web navigation is purely local. */
export type Route = { name: "board" } | { name: "workspace"; id: string };

function parse(hash: string): Route {
  const m = hash.match(/^#\/w\/(.+)$/);
  if (m) return { name: "workspace", id: decodeURIComponent(m[1]!) };
  return { name: "board" };
}

const [route, setRoute] = createSignal<Route>(parse(location.hash));
window.addEventListener("hashchange", () => setRoute(parse(location.hash)));

export { route };

export function navigate(r: Route): void {
  const next = r.name === "board" ? "#/" : `#/w/${encodeURIComponent(r.id)}`;
  if (location.hash !== next) location.hash = next;
}

export function hrefFor(r: Route): string {
  return r.name === "board" ? "#/" : `#/w/${encodeURIComponent(r.id)}`;
}

export function currentWorkspaceId(): string | null {
  const r = route();
  return r.name === "workspace" ? r.id : null;
}
